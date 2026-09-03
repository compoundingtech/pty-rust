//! Descendant termination without process-group signals: one snapshot of
//! the child's tree bound to process start identities, then exact TERM and
//! KILL signals to the identities that still match.
//!
//! node: src/process-tree.ts

use std::time::{Duration, Instant};

use pty_core::registry::read_process_start_token;

/// One descendant, pinned to its start token so a reused pid is skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: i32,
    pub process_start_token: String,
    pub depth: u32,
}

/// `ps -axo pid=,ppid=`.
fn list_processes() -> String {
    ps(&["-axo", "pid=,ppid="])
}

/// `ps -axo pid=,ppid=,pgid=,stat=`. One call for the whole machine, which is
/// the point: reading a start token costs a `ps` per descendant on macOS, and
/// a group sweep needs no tokens at all.
///
/// The state column is not decoration. `ps` lists a zombie with its process
/// group, so without it the sweep counts a corpse as a member and reports a
/// group it has already emptied. Measured on Linux 2026-09-03: a zombie's row
/// is `<pid> <ppid> <pgid> Z`.
pub fn list_processes_with_groups() -> String {
    ps(&["-axo", "pid=,ppid=,pgid=,stat="])
}

fn ps(args: &[&str]) -> String {
    std::process::Command::new("ps")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// One row of `ps -axo pid=,ppid=,pgid=,stat=`. The state is optional so a
/// three-column listing still parses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessRow {
    pub pid: i32,
    pub ppid: i32,
    pub pgid: i32,
    pub state: String,
}

impl ProcessRow {
    /// A zombie is a dead process that still has a row and still has a process
    /// group. It is not a member worth signalling or reporting.
    pub fn is_zombie(&self) -> bool {
        self.state.starts_with('Z')
    }
}

pub fn parse_rows(listing: &str) -> Vec<ProcessRow> {
    listing
        .lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let (Some(pid), Some(ppid), Some(pgid)) = (it.next(), it.next(), it.next()) else {
                return None;
            };
            let state = it.next().unwrap_or("").to_string();
            if it.next().is_some() {
                return None;
            }
            Some(ProcessRow {
                pid: pid.parse().ok()?,
                ppid: ppid.parse().ok()?,
                pgid: pgid.parse().ok()?,
                state,
            })
        })
        .collect()
}

/// Every distinct process group inside `root_pid`'s tree, plus the groups of
/// the root's own children.
///
/// **Deliberately not filtered by start token.** `snapshot_from_listing` drops
/// a descendant whose token cannot be read, and that process is then never
/// signalled. A group needs no identity, so collecting groups from the raw
/// listing reaches a process the token reader could not name.
///
/// The root's own group is excluded. A pty child calls `setsid`, so the daemon
/// sits alone in its group and signalling it would reach the daemon and
/// nothing else. Measured on Linux, both tools, 2026-09-03.
pub fn groups_in_tree(root_pid: i32, rows: &[ProcessRow]) -> Vec<i32> {
    let mut children: std::collections::HashMap<i32, Vec<i32>> = Default::default();
    let mut pgid_of: std::collections::HashMap<i32, i32> = Default::default();
    for r in rows {
        children.entry(r.ppid).or_default().push(r.pid);
        pgid_of.insert(r.pid, r.pgid);
    }
    let root_group = pgid_of.get(&root_pid).copied();
    let mut groups: Vec<i32> = Vec::new();
    let mut seen = std::collections::HashSet::from([root_pid]);
    let mut queue: std::collections::VecDeque<i32> =
        children.get(&root_pid).cloned().unwrap_or_default().into();
    while let Some(pid) = queue.pop_front() {
        if !seen.insert(pid) {
            continue;
        }
        if let Some(&g) = pgid_of.get(&pid)
            && Some(g) != root_group
            && g > 1
            && !groups.contains(&g)
        {
            groups.push(g);
        }
        for &c in children.get(&pid).map(|v| v.as_slice()).unwrap_or(&[]) {
            queue.push_back(c);
        }
    }
    groups.sort_unstable();
    groups
}

/// The live pids that still belong to any of `groups`. A zombie is excluded:
/// `ps` still lists it with its group, and counting it would make the sweep
/// report a group it has already emptied.
pub fn members_of_groups(groups: &[i32], rows: &[ProcessRow]) -> Vec<i32> {
    let mut out: Vec<i32> = rows
        .iter()
        .filter(|r| !r.is_zombie() && groups.contains(&r.pgid))
        .map(|r| r.pid)
        .collect();
    out.sort_unstable();
    out
}

/// TERM every group, wait, KILL what is left, wait, then say what is STILL
/// there. The caller reports the return value; it never reports the sending.
///
/// `own_group` is skipped so the command survives to print its own result.
pub fn sweep_groups(
    groups: &[i32],
    own_group: i32,
    term_wait: Duration,
    kill_wait: Duration,
    mut live: impl FnMut(&[i32]) -> Vec<i32>,
    mut signal: impl FnMut(i32, i32),
) -> Vec<i32> {
    let targets: Vec<i32> = groups
        .iter()
        .copied()
        .filter(|&g| g > 1 && g != own_group)
        .collect();
    if targets.is_empty() {
        return live(groups);
    }
    for &g in &targets {
        signal(g, libc::SIGTERM);
    }
    let deadline = Instant::now() + term_wait;
    let mut remaining = live(&targets);
    while !remaining.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
        remaining = live(&targets);
    }
    if remaining.is_empty() {
        return Vec::new();
    }
    for &g in &targets {
        signal(g, libc::SIGKILL);
    }
    let deadline = Instant::now() + kill_wait;
    remaining = live(&targets);
    while !remaining.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
        remaining = live(&targets);
    }
    remaining
}

/// `kill(2)` on a whole process group.
pub fn signal_group(pgid: i32, signal: i32) {
    if pgid <= 1 {
        return;
    }
    // SAFETY: a negative pid is the documented way to signal a process group.
    unsafe { libc::kill(-pgid, signal) };
}

/// The caller's own process group.
pub fn own_process_group() -> i32 {
    // SAFETY: getpgrp(2) takes no arguments and cannot fail.
    unsafe { libc::getpgrp() }
}

/// BFS from `root_pid` over a `pid ppid` listing; deepest first, then the
/// higher pid first.
///
/// node: src/process-tree.ts:28-61
pub fn snapshot_from_listing(
    root_pid: i32,
    listing: &str,
    read_token: impl Fn(i32) -> Option<String>,
) -> Vec<ProcessIdentity> {
    let mut children: std::collections::HashMap<i32, Vec<i32>> = Default::default();
    for line in listing.lines() {
        let mut it = line.split_whitespace();
        let (Some(pid), Some(ppid), None) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let (Ok(pid), Ok(ppid)) = (pid.parse::<i32>(), ppid.parse::<i32>()) else {
            continue;
        };
        children.entry(ppid).or_default().push(pid);
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::from([root_pid]);
    let mut queue: std::collections::VecDeque<(i32, u32)> = children
        .get(&root_pid)
        .map(|c| c.iter().map(|&p| (p, 1)).collect())
        .unwrap_or_default();
    while let Some((pid, depth)) = queue.pop_front() {
        if !seen.insert(pid) {
            continue;
        }
        if let Some(token) = read_token(pid) {
            out.push(ProcessIdentity {
                pid,
                process_start_token: token,
                depth,
            });
        }
        for &c in children.get(&pid).map(|v| v.as_slice()).unwrap_or(&[]) {
            queue.push_back((c, depth + 1));
        }
    }
    out.sort_by(|a, b| b.depth.cmp(&a.depth).then(b.pid.cmp(&a.pid)));
    out
}

/// The live descendant snapshot of `root_pid`.
pub fn snapshot_descendant_processes(root_pid: i32) -> Vec<ProcessIdentity> {
    snapshot_from_listing(root_pid, &list_processes(), read_process_start_token)
}

fn is_same_process(id: &ProcessIdentity) -> bool {
    read_process_start_token(id.pid).as_deref() == Some(id.process_start_token.as_str())
}

/// Signal the identities whose start token still matches. Returns the pids
/// signalled.
///
/// node: src/process-tree.ts:69-86
pub fn signal_process_identities(ids: &[ProcessIdentity], signal: i32) -> Vec<i32> {
    let mut signalled = Vec::new();
    for id in ids {
        if !is_same_process(id) {
            continue;
        }
        // SAFETY: plain kill(2) on a pid we just re-verified.
        if unsafe { libc::kill(id.pid, signal) } == 0 {
            signalled.push(id.pid);
        }
    }
    signalled
}

fn wait_for_identities_to_exit(ids: &[ProcessIdentity], timeout: Duration) -> Vec<ProcessIdentity> {
    let deadline = Instant::now() + timeout;
    let mut survivors: Vec<ProcessIdentity> = ids.iter().filter(|i| is_same_process(i)).cloned().collect();
    while !survivors.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
        survivors.retain(is_same_process);
    }
    survivors
}

/// Default TERM grace (`termWaitMs`).
pub const TERM_WAIT: Duration = Duration::from_millis(1_500);
/// Default KILL grace (`killWaitMs`).
pub const KILL_WAIT: Duration = Duration::from_millis(500);

/// TERM, wait ≤ `term_wait`; KILL the survivors, wait ≤ `kill_wait`; return
/// whatever is still alive.
///
/// node: src/process-tree.ts:109-124
pub fn terminate_process_identities(
    ids: &[ProcessIdentity],
    term_wait: Duration,
    kill_wait: Duration,
) -> Vec<ProcessIdentity> {
    if ids.is_empty() {
        return Vec::new();
    }
    signal_process_identities(ids, libc::SIGTERM);
    let after_term = wait_for_identities_to_exit(ids, term_wait);
    if after_term.is_empty() {
        return Vec::new();
    }
    signal_process_identities(&after_term, libc::SIGKILL);
    wait_for_identities_to_exit(&after_term, kill_wait)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node: tests/process-tree.test.ts:10-79
    #[test]
    fn snapshot_is_deepest_first_with_tokens() {
        let listing = "100 1\n200 100\n300 200\n201 100\n999 1\n";
        let ids = snapshot_from_listing(100, listing, |pid| {
            (pid != 201).then(|| format!("linux:{pid}"))
        });
        assert_eq!(
            ids,
            vec![
                ProcessIdentity {
                    pid: 300,
                    process_start_token: "linux:300".into(),
                    depth: 2
                },
                ProcessIdentity {
                    pid: 200,
                    process_start_token: "linux:200".into(),
                    depth: 1
                },
            ]
        );
    }

    #[test]
    fn dead_identities_are_skipped_by_signal() {
        let ids = vec![ProcessIdentity {
            pid: std::process::id() as i32,
            process_start_token: "not-the-real-token".into(),
            depth: 1,
        }];
        assert!(signal_process_identities(&ids, 0).is_empty());
        assert!(terminate_process_identities(&ids, Duration::ZERO, Duration::ZERO).is_empty());
    }
}

#[cfg(test)]
mod group_tests {
    use super::*;
    use std::cell::RefCell;

    // daemon 100 (its own group), pty child 200 (setsid: its own group and
    // session), 300 under the child, and 400 in a background group of its own.
    // 900 is an unrelated process. This is the shape measured on Linux for
    // both tools on 2026-09-03.
    const ROWS: &str = "\
100 1 100
200 100 200
300 200 200
400 300 400
900 1 900
";

    #[test]
    fn the_daemons_own_group_is_not_a_target() {
        let groups = groups_in_tree(100, &parse_rows(ROWS));
        assert!(
            !groups.contains(&100),
            "the daemon sits alone in its group; signalling it reaches the daemon and nothing else"
        );
        assert_eq!(groups, vec![200, 400]);
    }

    #[test]
    fn an_unrelated_group_is_never_a_target() {
        assert!(!groups_in_tree(100, &parse_rows(ROWS)).contains(&900));
    }

    /// The reason to sweep groups at all: `snapshot_from_listing` drops a
    /// descendant whose start token cannot be read, so that process is never
    /// signalled. Groups are read from the raw listing, so its group is still
    /// a target.
    #[test]
    fn a_descendant_with_no_readable_token_is_still_inside_a_target_group() {
        let rows = parse_rows(ROWS);
        let snapshot = snapshot_from_listing(100, ROWS, |pid| {
            // 400 is the one whose token cannot be read.
            (pid != 400).then(|| format!("tok:{pid}"))
        });
        assert!(
            !snapshot.iter().any(|i| i.pid == 400),
            "precondition: the token reader drops 400 from the snapshot"
        );
        assert!(
            groups_in_tree(100, &rows).contains(&400),
            "its group must still be a target"
        );
    }

    #[test]
    fn members_are_read_back_by_group() {
        let rows = parse_rows(ROWS);
        assert_eq!(members_of_groups(&[200], &rows), vec![200, 300]);
        assert_eq!(members_of_groups(&[400], &rows), vec![400]);
        assert_eq!(members_of_groups(&[], &rows), Vec::<i32>::new());
    }

    #[test]
    fn a_short_listing_row_is_ignored_rather_than_guessed() {
        let rows = parse_rows("1 2\nx y z\n7 8 9\n");
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].pid, rows[0].ppid, rows[0].pgid), (7, 8, 9));
    }

    /// `ps` lists a zombie with its process group. Counting it would make the
    /// sweep report a group it has already emptied, and then signal it again.
    /// Measured on Linux 2026-09-03: the row reads `<pid> <ppid> <pgid> Z`.
    #[test]
    fn a_zombie_is_not_a_group_member() {
        let rows = parse_rows("100 1 100 Ss\n200 100 200 Sl\n300 200 200 Z\n");
        assert_eq!(
            members_of_groups(&[200], &rows),
            vec![200],
            "300 is a corpse and must not count as a member"
        );
    }

    fn sweep(groups: &[i32], own: i32, alive: Vec<Vec<i32>>) -> (Vec<i32>, Vec<(i32, i32)>) {
        let sent = RefCell::new(Vec::new());
        let step = RefCell::new(0usize);
        let left = sweep_groups(
            groups,
            own,
            Duration::ZERO,
            Duration::ZERO,
            |_| {
                let mut i = step.borrow_mut();
                let v = alive.get(*i).cloned().unwrap_or_default();
                *i += 1;
                v
            },
            |g, sig| sent.borrow_mut().push((g, sig)),
        );
        (left, sent.into_inner())
    }

    #[test]
    fn a_group_that_answers_term_is_never_killed() {
        let (left, sent) = sweep(&[200], 5, vec![vec![]]);
        assert!(left.is_empty());
        assert_eq!(sent, vec![(200, libc::SIGTERM)], "SIGKILL must not be sent");
    }

    /// A coding agent was measured ignoring SIGTERM for ten seconds. A design
    /// that sends one TERM and hopes is already known not to work here.
    #[test]
    fn a_group_that_ignores_term_is_killed() {
        let (left, sent) = sweep(&[200], 5, vec![vec![300], vec![]]);
        assert!(left.is_empty());
        assert_eq!(sent, vec![(200, libc::SIGTERM), (200, libc::SIGKILL)]);
    }

    /// The command reports the machine, not the signals. A group that outlives
    /// SIGKILL comes back as still there.
    #[test]
    fn what_outlives_sigkill_is_returned_not_swallowed() {
        let (left, _) = sweep(&[200], 5, vec![vec![300], vec![300]]);
        assert_eq!(left, vec![300]);
    }

    #[test]
    fn my_own_group_is_never_signalled() {
        let (_, sent) = sweep(&[200], 200, vec![vec![]]);
        assert!(sent.is_empty(), "the command must survive to print its result");
    }

    #[test]
    fn group_one_and_below_are_never_signalled() {
        let (_, sent) = sweep(&[0, 1, -1], 999, vec![vec![]]);
        assert!(sent.is_empty());
    }
}

#[cfg(test)]
mod real_group_tests {
    use super::*;

    fn live(targets: &[i32]) -> Vec<i32> {
        members_of_groups(targets, &parse_rows(&list_processes_with_groups()))
    }

    /// The mocked sweep tests prove the ordering. This one proves the ordering
    /// is about real processes: it builds a real process group whose members
    /// ignore SIGTERM, runs the real sweep against it, and checks the machine
    /// afterwards rather than the signals sent at it.
    ///
    /// A coding agent was measured ignoring SIGTERM for ten seconds on
    /// 2026-09-02. This is that shape, in a test.
    #[test]
    fn a_real_group_that_ignores_term_is_killed_and_verified_gone() {
        // `setsid` gives the shell its own session and process group, so this
        // group is ours alone and nothing else on the machine is in it.
        let mut child = std::process::Command::new("setsid")
            .args(["sh", "-c", "trap '' TERM; sleep 60 & sleep 60"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn setsid sh");
        let leader = child.id() as i32;

        // Wait for the group to actually exist before testing anything.
        let deadline = Instant::now() + Duration::from_secs(5);
        while live(&[leader]).len() < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        let members = live(&[leader]);
        assert!(
            members.len() >= 2,
            "precondition: expected a group with the shell and its child, got {members:?}"
        );

        // SIGTERM alone must not be enough, or the test proves nothing about
        // escalation.
        signal_group(leader, libc::SIGTERM);
        std::thread::sleep(Duration::from_millis(400));
        assert!(
            !live(&[leader]).is_empty(),
            "precondition: this group is supposed to ignore SIGTERM"
        );

        let still_there = sweep_groups(
            &[leader],
            own_process_group(),
            Duration::from_millis(500),
            Duration::from_secs(2),
            |t| live(t),
            signal_group,
        );

        // Clean up the whole group before asserting. The leader alone is not
        // the cleanup: on a failure its group would leak into the next run.
        let verdict = (still_there.clone(), live(&[leader]));
        signal_group(leader, libc::SIGKILL);
        let _ = child.wait();

        assert!(
            verdict.0.is_empty(),
            "the sweep reported success while these were alive: {:?}",
            verdict.0
        );
        assert!(
            verdict.1.is_empty(),
            "the process table disagrees with the sweep"
        );
    }

    /// The command must not signal the group it is running in, or it dies
    /// before it can report. Proven against this test process's own group.
    #[test]
    fn the_running_process_group_is_never_signalled() {
        let own = own_process_group();
        let me = std::process::id() as i32;
        let still_there = sweep_groups(
            &[own],
            own,
            Duration::ZERO,
            Duration::ZERO,
            |t| live(t),
            signal_group,
        );
        assert!(
            still_there.contains(&me) || !still_there.is_empty(),
            "the sweep should report our own group as still there, not signal it"
        );
        assert!(registry_alive(me), "we must still be running");
    }

    fn registry_alive(pid: i32) -> bool {
        // SAFETY: signal 0 only checks for existence.
        unsafe { libc::kill(pid, 0) == 0 }
    }
}

#[cfg(test)]
mod unreadable_token_tests {
    use super::snapshot_from_listing;

    /// **A descendant whose start token cannot be read is left out of the
    /// snapshot entirely, and therefore never signalled.**
    ///
    /// This pins the current behaviour rather than endorsing it. Reading a
    /// token is a `/proc` read on Linux and a `ps` call per descendant on
    /// macOS, and a `ps` that answers slowly, or not at all, silently drops
    /// that process from the teardown.
    ///
    /// The Node tool omits it in the same way (`src/process-tree.ts`), so
    /// this is shared rather than a difference. Recorded because it is a
    /// candidate mechanism for a harness that survived a `pty kill` on a Mac
    /// on 2026-09-02.
    #[test]
    fn a_descendant_with_no_readable_token_is_not_in_the_snapshot() {
        // daemon 100 -> middle 200 -> harness 300
        let listing = "100 1\n200 100\n300 200\n";
        // The middle answers; the harness does not.
        let ids = snapshot_from_listing(100, listing, |pid| {
            (pid != 300).then(|| format!("tok:{pid}"))
        });
        let pids: Vec<i32> = ids.iter().map(|i| i.pid).collect();
        assert_eq!(pids, vec![200], "300 was dropped, so nothing will signal it");

        // And with a readable token it is there, deepest first.
        let ids = snapshot_from_listing(100, listing, |pid| Some(format!("tok:{pid}")));
        let pids: Vec<i32> = ids.iter().map(|i| i.pid).collect();
        assert_eq!(pids, vec![300, 200]);
    }

    /// Its children are still walked, so only the unreadable process escapes
    /// and not its subtree.
    #[test]
    fn the_subtree_below_an_unreadable_process_is_still_reached() {
        let listing = "100 1\n200 100\n300 200\n400 300\n";
        let ids = snapshot_from_listing(100, listing, |pid| {
            (pid != 300).then(|| format!("tok:{pid}"))
        });
        let pids: Vec<i32> = ids.iter().map(|i| i.pid).collect();
        assert_eq!(pids, vec![400, 200]);
    }
}
