//! Descendant termination without process-group signals: one snapshot of
//! the child's tree bound to process start identities, then exact TERM and
//! KILL signals to the identities that still match.
//!
//! node: src/process-tree.ts

use std::time::{Duration, Instant};

pub use pty_core::proctable::{LiveIdentity, ProcTable, Row as ProcessRow};

/// One descendant, pinned to its start identity so a reused pid is skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: i32,
    pub identity: LiveIdentity,
    pub depth: u32,
}


pub fn capture_process_identity(pid: i32) -> Option<ProcessIdentity> {
    let row = pty_core::proctable::process(pid).known()?;
    if row.is_zombie() {
        return None;
    }
    Some(ProcessIdentity {
        pid,
        identity: row.identity?,
        depth: 0,
    })
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompleteProcessTreeSnapshot {
    Complete {
        identities: Vec<ProcessIdentity>,
    },
    Unavailable {
        reason: String,
        identities: Vec<ProcessIdentity>,
    },
}

impl CompleteProcessTreeSnapshot {
    pub fn identities(&self) -> &[ProcessIdentity] {
        match self {
            Self::Complete { identities } | Self::Unavailable { identities, .. } => identities,
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete { .. })
    }
}

/// Walk a tree from `root_pid`, breadth first, recording depth.
fn walk(root_pid: i32, table: &ProcTable) -> Vec<(i32, u32)> {
    let mut children: std::collections::HashMap<i32, Vec<i32>> = Default::default();
    for r in table.rows() {
        children.entry(r.ppid).or_default().push(r.pid);
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
        out.push((pid, depth));
        for &c in children.get(&pid).map(|v| v.as_slice()).unwrap_or(&[]) {
            queue.push_back((c, depth + 1));
        }
    }
    out
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
pub fn groups_in_tree(root_pid: i32, table: &ProcTable) -> Vec<i32> {
    let pgid_of: std::collections::HashMap<i32, i32> =
        table.rows().map(|r| (r.pid, r.pgid)).collect();
    let root_group = pgid_of.get(&root_pid).copied();
    let mut groups: Vec<i32> = Vec::new();
    for (pid, _) in walk(root_pid, table) {
        if let Some(&g) = pgid_of.get(&pid)
            && Some(g) != root_group
            && g > 1
            && !groups.contains(&g)
        {
            groups.push(g);
        }
    }
    groups.sort_unstable();
    groups
}

/// The live pids that still belong to any of `groups`. A zombie is excluded:
/// `ps` still lists it with its group, and counting it would make the sweep
/// report a group it has already emptied.
pub fn members_of_groups(groups: &[i32], table: &ProcTable) -> Vec<i32> {
    let mut out: Vec<i32> = table
        .rows()
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
/// The descendants of `root_pid` from one table, deepest first, then the
/// higher pid first.
///
/// node: src/process-tree.ts:28-61
pub fn snapshot_from_table(root_pid: i32, table: &ProcTable) -> Vec<ProcessIdentity> {
    let mut out: Vec<ProcessIdentity> = walk(root_pid, table)
        .into_iter()
        .filter_map(|(pid, depth)| {
            table.identity(pid).known().map(|identity| ProcessIdentity {
                pid,
                identity,
                depth,
            })
        })
        .collect();
    out.sort_by(|a, b| b.depth.cmp(&a.depth).then(b.pid.cmp(&a.pid)));
    out
}

/// The live descendant snapshot of `root_pid`. One table read.
pub fn snapshot_descendant_processes(root_pid: i32) -> Vec<ProcessIdentity> {
    snapshot_from_table(root_pid, &ProcTable::read())
}

/// Snapshot every live descendant, keeping unreadable containment distinct
/// from a genuinely empty tree.
pub fn snapshot_complete_from_table(
    root_pid: i32,
    table: &ProcTable,
) -> CompleteProcessTreeSnapshot {
    if !table.is_readable() {
        return CompleteProcessTreeSnapshot::Unavailable {
            reason: "process-table-unreadable".to_string(),
            identities: Vec::new(),
        };
    }
    let root_available = matches!(
        table.row(root_pid),
        pty_core::proctable::Answer::Known(row)
            if !row.is_zombie() && row.identity.is_some()
    );
    let observed = snapshot_from_table(root_pid, table);
    if !root_available {
        return CompleteProcessTreeSnapshot::Unavailable {
            reason: "root-identity-unavailable".to_string(),
            identities: observed,
        };
    }
    for (pid, _) in walk(root_pid, table) {
        if let pty_core::proctable::Answer::Known(row) = table.row(pid)
            && !row.is_zombie()
            && row.identity.is_none()
        {
            return CompleteProcessTreeSnapshot::Unavailable {
                reason: format!("process-identity-unavailable:{pid}"),
                identities: observed,
            };
        }
    }
    CompleteProcessTreeSnapshot::Complete {
        identities: observed,
    }
}

pub fn snapshot_descendant_processes_complete(root_pid: i32) -> CompleteProcessTreeSnapshot {
    snapshot_complete_from_table(root_pid, &ProcTable::read())
}

/// A complete snapshot whose root must still be the process captured at
/// spawn. A reused root pid is never accepted as a new containment root.
pub fn snapshot_descendant_processes_complete_for(
    root: &ProcessIdentity,
) -> CompleteProcessTreeSnapshot {
    let table = ProcTable::read();
    if table.identity(root.pid).known().as_ref() != Some(&root.identity) {
        return CompleteProcessTreeSnapshot::Unavailable {
            reason: "root-identity-changed".to_string(),
            identities: Vec::new(),
        };
    }
    snapshot_complete_from_table(root.pid, &table)
}

/// Authenticate a control peer as the same accepted process, beneath the exact
/// launch authority, outside the exact managed child tree, and not routed
/// through the daemon. Every identity and ancestry edge must be present in one
/// complete table snapshot.
pub fn control_peer_is_authorized(
    root: &ProcessIdentity,
    daemon: &ProcessIdentity,
    authority: &ProcessIdentity,
    peer_pid: i32,
    peer_identity: &LiveIdentity,
) -> bool {
    control_peer_is_authorized_in(
        root,
        daemon,
        authority,
        peer_pid,
        peer_identity,
        &ProcTable::read(),
    )
}

fn control_peer_is_authorized_in(
    root: &ProcessIdentity,
    daemon: &ProcessIdentity,
    authority: &ProcessIdentity,
    peer_pid: i32,
    peer_identity: &LiveIdentity,
    table: &ProcTable,
) -> bool {
    if !table.is_readable()
        || root.pid <= 1
        || daemon.pid <= 1
        || authority.pid <= 1
        || peer_pid <= 1
    {
        return false;
    }
    for expected in [root, daemon] {
        let Some(row) = table.row(expected.pid).known() else {
            return false;
        };
        if row.is_zombie() || row.identity.as_ref() != Some(&expected.identity) {
            return false;
        }
    }

    let mut pid = peer_pid;
    let mut seen = std::collections::HashSet::new();
    loop {
        if !seen.insert(pid) || pid == root.pid || pid == daemon.pid {
            return false;
        }
        let Some(row) = table.row(pid).known() else {
            return false;
        };
        if row.is_zombie() || row.identity.is_none() {
            return false;
        }
        if pid == peer_pid && row.identity.as_ref() != Some(peer_identity) {
            return false;
        }
        if pid == authority.pid {
            return row.identity.as_ref() == Some(&authority.identity);
        }
        if row.ppid <= 1 {
            return false;
        }
        pid = row.ppid;
    }
}

fn merge_identities(left: &mut Vec<ProcessIdentity>, right: &[ProcessIdentity]) {
    for candidate in right {
        if !left
            .iter()
            .any(|id| id.pid == candidate.pid && id.identity == candidate.identity)
        {
            left.push(candidate.clone());
        }
    }
    left.sort_by(|a, b| b.depth.cmp(&a.depth).then(b.pid.cmp(&a.pid)));
}

fn same_identities(left: &[ProcessIdentity], right: &[ProcessIdentity]) -> bool {
    left == right
}

/// Freeze the child process group and every discovered descendant, rescanning
/// until two complete identity snapshots agree. Every raw root/group signal
/// is fenced by the root identity captured at spawn.
pub fn freeze_descendant_processes(root: &ProcessIdentity) -> CompleteProcessTreeSnapshot {
    let first = snapshot_descendant_processes_complete_for(root);
    let mut observed = first.identities().to_vec();
    let mut previous_complete = match &first {
        CompleteProcessTreeSnapshot::Complete { identities } => Some(identities.clone()),
        CompleteProcessTreeSnapshot::Unavailable { .. } => None,
    };
    let unavailable_reason = match &first {
        CompleteProcessTreeSnapshot::Unavailable { reason, .. } => reason.clone(),
        CompleteProcessTreeSnapshot::Complete { .. } => "process-tree-did-not-quiesce".to_string(),
    };
    let attempts = if first.is_complete() { 8 } else { 1 };
    // The PTY leader is its process-group leader. Freeze the group first, then
    // the exact root and any out-of-group descendants. Never signal the group
    // after the captured leader identity has disappeared.
    if !signal_group_for_root(root, libc::SIGSTOP)
        || signal_process_identities(std::slice::from_ref(root), libc::SIGSTOP).is_empty()
    {
        return CompleteProcessTreeSnapshot::Unavailable {
            reason: "root-freeze-failed".to_string(),
            identities: observed,
        };
    }
    for _ in 0..attempts {
        if signal_process_identities(&observed, libc::SIGSTOP).len() != observed.len() {
            return CompleteProcessTreeSnapshot::Unavailable {
                reason: "descendant-freeze-unverified".to_string(),
                identities: observed,
            };
        }
        let next = snapshot_descendant_processes_complete_for(root);
        merge_identities(&mut observed, next.identities());
        match next {
            CompleteProcessTreeSnapshot::Complete { identities } => {
                if previous_complete
                    .as_ref()
                    .is_some_and(|previous| same_identities(previous, &identities))
                {
                    return CompleteProcessTreeSnapshot::Complete { identities };
                }
                previous_complete = Some(identities);
            }
            CompleteProcessTreeSnapshot::Unavailable { reason, .. } => {
                return CompleteProcessTreeSnapshot::Unavailable {
                    reason,
                    identities: observed,
                };
            }
        }
    }
    CompleteProcessTreeSnapshot::Unavailable {
        reason: unavailable_reason,
        identities: observed,
    }
}


fn signal_group_for_root(root: &ProcessIdentity, signal: i32) -> bool {
    if !is_same_process(root) {
        return false;
    }
    signal_group(root.pid, signal);
    true
}

fn process_group_exists(root: &ProcessIdentity) -> Option<bool> {
    if root.pid <= 1 || !is_same_process(root) {
        return None;
    }
    // SAFETY: signal zero observes the process group without changing it.
    if unsafe { libc::kill(-root.pid, 0) } == 0 {
        return Some(true);
    }
    Some(std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
}

/// Bounded process-group fallback used only when exact containment was not
/// provable. Every observation and signal remains fenced by the root's
/// captured start identity. `false` means teardown could not be verified.
pub fn terminate_process_group(
    root: &ProcessIdentity,
    term_wait: Duration,
    kill_wait: Duration,
) -> bool {
    match process_group_exists(root) {
        Some(false) => return true,
        Some(true) => {}
        None => return false,
    }
    if !signal_group_for_root(root, libc::SIGTERM) {
        return false;
    }
    let deadline = Instant::now() + term_wait;
    while process_group_exists(root) == Some(true) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    match process_group_exists(root) {
        Some(false) => return true,
        Some(true) => {}
        None => return false,
    }
    if !signal_group_for_root(root, libc::SIGKILL) {
        return false;
    }
    let deadline = Instant::now() + kill_wait;
    while process_group_exists(root) == Some(true) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    process_group_exists(root) == Some(false)
}

/// Is this still the same process, and still running?
///
/// **A corpse is not a survivor.** On Linux an unreaped descendant keeps its
/// `/proc` row and its identity, so matching on identity alone counted it as
/// alive: the teardown would wait out its whole TERM budget for a process that
/// had already died, then report it as a descendant that survived a SIGKILL.
/// That is the kill over-claiming again, in the other direction.
///
/// macOS never had this: libproc stops listing a process the moment it exits.
/// `Silber.pty` measured that on a real Mac on 2026-09-03, and chasing why its
/// zombie test failed is what found this.
///
/// **One process, not the whole table.** A full table read costs about as much
/// as 473 single reads (measured on Linux, 2026-09-03: 4.21 ms against
/// 0.0089 ms), so a poll loop over a handful of descendants asks about each one
/// rather than re-reading the machine.
///
/// **An unreadable answer means `false`, and that is deliberate**: it says
/// "do not signal", never "it is gone". Every caller here wants the safe
/// direction for a signal; a caller that needs the difference asks
/// `proctable` and reads the three cases.
pub(crate) fn is_same_process(id: &ProcessIdentity) -> bool {
    match pty_core::proctable::process(id.pid) {
        pty_core::proctable::Answer::Known(row) if !row.is_zombie() => {
            row.identity.as_ref() == Some(&id.identity)
        }
        _ => false,
    }
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

/// **One question per surviving identity, and each question is now cheap.**
///
/// The shape of this loop is unchanged and it is now affordable.
///
/// It asks about each surviving descendant separately, which is right: a full
/// table read costs about as much as 473 single reads. What changed is the
/// cost of one question. On macOS it used to be a `ps` subprocess, so at 25 ms
/// polling inside a 1500 ms budget four descendants cost 240 spawns — 2.6
/// seconds of spawning inside a 1.5 second deadline, at the 10.9 ms a spawn was
/// measured to take. **The loop could not meet its own deadline on an idle
/// machine.** Each question is now one `proc_pidinfo` call there and one small
/// `/proc` read on Linux.
fn wait_for_identities_to_exit(ids: &[ProcessIdentity], timeout: Duration) -> Vec<ProcessIdentity> {
    let deadline = Instant::now() + timeout;
    let mut survivors: Vec<ProcessIdentity> =
        ids.iter().filter(|i| is_same_process(i)).cloned().collect();
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
        // 201 has no readable identity, so it is not in the snapshot.
        let table = pty_core::proctable::table_from_shape(
            "100 1 100 Ss linux:100\n200 100 200 S linux:200\n300 200 200 S linux:300\n201 100 200 S -\n999 1 999 S linux:999\n",
        );
        let ids = snapshot_from_table(100, &table);
        assert_eq!(
            ids,
            vec![
                ProcessIdentity {
                    pid: 300,
                    identity: LiveIdentity::new("linux:300"),
                    depth: 2
                },
                ProcessIdentity {
                    pid: 200,
                    identity: LiveIdentity::new("linux:200"),
                    depth: 1
                },
            ]
        );
    }

    /// An unreaped descendant kept its `/proc` row and its identity on Linux,
    /// so the teardown counted a corpse as a survivor: it waited out the whole
    /// TERM budget for a process that had already died, and would then report
    /// it as having survived a SIGKILL.
    ///
    /// macOS never had this, because libproc drops the process at once. The
    /// disagreement between the two platforms is what exposed it.
    #[test]
    fn a_corpse_is_not_a_survivor() {
        let mut child = std::process::Command::new("true").spawn().expect("spawn");
        let pid = child.id() as i32;
        let identity = pty_core::proctable::process(pid)
            .known()
            .and_then(|r| r.identity);
        // On macOS it may already be gone, and then there is nothing to pin.
        if let Some(identity) = identity {
            let id = ProcessIdentity {
                pid,
                identity,
                depth: 1,
            };
            let deadline = Instant::now() + Duration::from_secs(5);
            while is_same_process(&id) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                !is_same_process(&id),
                "an exited but unreaped child still reads as a live descendant"
            );
            assert!(
                signal_process_identities(&[id], 0).is_empty(),
                "and nothing should be signalled at it"
            );
        }
        let _ = child.wait();
    }

    #[test]
    fn dead_identities_are_skipped_by_signal() {
        let ids = vec![ProcessIdentity {
            pid: std::process::id() as i32,
            identity: LiveIdentity::new("not-the-real-identity"),
            depth: 1,
        }];
        assert!(signal_process_identities(&ids, 0).is_empty());
        assert!(terminate_process_identities(&ids, Duration::ZERO, Duration::ZERO).is_empty());
    }

    #[test]
    fn reused_root_identity_blocks_pid_and_group_signals() {
        let root = ProcessIdentity {
            pid: std::process::id() as i32,
            identity: LiveIdentity::new("not-the-live-root"),
            depth: 0,
        };
        assert!(!signal_group_for_root(&root, 0));
        assert!(!terminate_process_group(
            &root,
            Duration::ZERO,
            Duration::ZERO
        ));
        assert!(matches!(
            snapshot_descendant_processes_complete_for(&root),
            CompleteProcessTreeSnapshot::Unavailable { reason, .. }
                if reason == "root-identity-changed"
        ));
    }

    #[test]
    fn control_authorization_requires_exact_launch_ancestry_outside_the_child_tree() {
        let table = pty_core::proctable::table_from_shape(
            "10 1 10 S linux:authority\n\
             11 10 11 S linux:launcher\n\
             15 11 15 S linux:daemon\n\
             20 15 20 S linux:child\n\
             30 20 20 S linux:managed-descendant\n\
             40 10 40 S linux:sibling\n\
             50 20 20 S linux:fork-parent\n\
             60 1 50 S linux:reparented-child\n\
             70 40 40 S -\n\
             80 15 20 S linux:clone-parent-peer\n",
        );
        let root = ProcessIdentity {
            pid: 20,
            identity: LiveIdentity::new("linux:child"),
            depth: 0,
        };
        let daemon = ProcessIdentity {
            pid: 15,
            identity: LiveIdentity::new("linux:daemon"),
            depth: 0,
        };
        let authority = ProcessIdentity {
            pid: 10,
            identity: LiveIdentity::new("linux:authority"),
            depth: 0,
        };

        assert!(control_peer_is_authorized_in(
            &root,
            &daemon,
            &authority,
            40,
            &LiveIdentity::new("linux:sibling"),
            &table
        ));
        assert!(!control_peer_is_authorized_in(
            &root,
            &daemon,
            &authority,
            30,
            &LiveIdentity::new("linux:managed-descendant"),
            &table
        ));
        assert!(!control_peer_is_authorized_in(
            &root,
            &daemon,
            &authority,
            60,
            &LiveIdentity::new("linux:reparented-child"),
            &table
        ));
        assert!(!control_peer_is_authorized_in(
            &root,
            &daemon,
            &authority,
            40,
            &LiveIdentity::new("linux:reused-peer"),
            &table
        ));
        let reused_authority = ProcessIdentity {
            pid: 10,
            identity: LiveIdentity::new("linux:reused-authority"),
            depth: 0,
        };
        assert!(!control_peer_is_authorized_in(
            &root,
            &daemon,
            &reused_authority,
            40,
            &LiveIdentity::new("linux:sibling"),
            &table
        ));
        assert!(!control_peer_is_authorized_in(
            &root,
            &daemon,
            &authority,
            70,
            &LiveIdentity::new("tok:70"),
            &table
        ));
        assert!(!control_peer_is_authorized_in(
            &root,
            &daemon,
            &authority,
            80,
            &LiveIdentity::new("linux:clone-parent-peer"),
            &table
        ));
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
    fn rows() -> ProcTable {
        pty_core::proctable::table_from_shape(
            "100 1 100 Ss\n200 100 200 Ss\n300 200 200 S\n400 300 400 S\n900 1 900 S\n",
        )
    }

    #[test]
    fn the_daemons_own_group_is_not_a_target() {
        let groups = groups_in_tree(100, &rows());
        assert!(
            !groups.contains(&100),
            "the daemon sits alone in its group; signalling it reaches the daemon and nothing else"
        );
        assert_eq!(groups, vec![200, 400]);
    }

    #[test]
    fn an_unrelated_group_is_never_a_target() {
        assert!(!groups_in_tree(100, &rows()).contains(&900));
    }

    /// The reason to sweep groups at all: `snapshot_from_listing` drops a
    /// descendant whose start token cannot be read, so that process is never
    /// signalled. Groups are read from the raw listing, so its group is still
    /// a target.
    #[test]
    fn a_descendant_with_no_readable_token_is_still_inside_a_target_group() {
        let rows = rows();
        // 400 is the one whose identity cannot be read.
        let unnamed = pty_core::proctable::table_from_shape(
            "100 1 100 Ss\n200 100 200 Ss\n300 200 200 S\n400 300 400 S -\n900 1 900 S\n",
        );
        let snapshot = snapshot_from_table(100, &unnamed);
        assert!(
            !snapshot.iter().any(|i| i.pid == 400),
            "precondition: the token reader drops 400 from the snapshot"
        );
        assert!(
            groups_in_tree(100, &unnamed).contains(&400),
            "its group must still be a target"
        );
    }

    #[test]
    fn members_are_read_back_by_group() {
        let rows = rows();
        assert_eq!(members_of_groups(&[200], &rows), vec![200, 300]);
        assert_eq!(members_of_groups(&[400], &rows), vec![400]);
        assert_eq!(members_of_groups(&[], &rows), Vec::<i32>::new());
    }

    /// `ps` lists a zombie with its process group. Counting it would make the
    /// sweep report a group it has already emptied, and then signal it again.
    /// Measured on Linux 2026-09-03: the row reads `<pid> <ppid> <pgid> Z`.
    #[test]
    fn a_zombie_is_not_a_group_member() {
        let rows =
            pty_core::proctable::table_from_shape("100 1 100 Ss\n200 100 200 Sl\n300 200 200 Z\n");
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
        assert!(
            sent.is_empty(),
            "the command must survive to print its result"
        );
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
        members_of_groups(targets, &ProcTable::read())
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
        // A new process group of our own, so nothing else on the machine is in
        // it.
        //
        // **macOS has the `setsid` system call but no `setsid` executable.**
        // An earlier version of this test spawned the binary, so on the one
        // platform where process groups are the whole escalation story, the
        // test could not run at all. `process_group(0)` is the same thing
        // without the command. Reported from a real Mac by `Silber.pty` on
        // 2026-09-03.
        use std::os::unix::process::CommandExt;
        let mut child = std::process::Command::new("sh")
            .args(["-c", "trap '' TERM; sleep 60 & sleep 60"])
            .process_group(0)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn sh in its own group");
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
    use super::*;

    /// **A descendant whose start token cannot be read is left out of the
    /// snapshot entirely, and therefore never signalled.**
    ///
    /// This pins the current behaviour rather than endorsing it. It matters
    /// less than it did: the group sweep reaches such a process anyway,
    /// because a group needs no identity. It still means the exact per-pid
    /// teardown will not signal it.
    ///
    /// The Node tool omits it in the same way (`src/process-tree.ts`), so
    /// this is shared rather than a difference. Recorded because it is a
    /// candidate mechanism for a harness that survived a `pty kill` on a Mac
    /// on 2026-09-02.
    #[test]
    fn a_descendant_with_no_readable_token_is_not_in_the_snapshot() {
        use pty_core::proctable::table_from_shape;
        // daemon 100 -> middle 200 -> harness 300. The middle answers; the
        // harness does not.
        let unnamed = table_from_shape("100 1 100\n200 100 200\n300 200 200 S -");
        let ids = super::snapshot_from_table(100, &unnamed);
        let pids: Vec<i32> = ids.iter().map(|i| i.pid).collect();
        assert_eq!(pids, vec![200], "300 was dropped from the exact teardown");

        // And with a readable identity it is there, deepest first.
        let named = table_from_shape("100 1 100\n200 100 200\n300 200 200");
        let ids = super::snapshot_from_table(100, &named);
        let pids: Vec<i32> = ids.iter().map(|i| i.pid).collect();
        assert_eq!(pids, vec![300, 200]);

        // But its GROUP is still a target, which is how it now gets signalled.
        assert!(super::groups_in_tree(100, &unnamed).contains(&200));
    }

    /// Its children are still walked, so only the unreadable process escapes
    /// and not its subtree.
    #[test]
    fn the_subtree_below_an_unreadable_process_is_still_reached() {
        let table = pty_core::proctable::table_from_shape(
            "100 1 100\n200 100 200\n300 200 200 S -\n400 300 200",
        );
        let ids = super::snapshot_from_table(100, &table);
        let pids: Vec<i32> = ids.iter().map(|i| i.pid).collect();
        assert_eq!(pids, vec![400, 200]);
    }
}
