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
    std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid="])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
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
