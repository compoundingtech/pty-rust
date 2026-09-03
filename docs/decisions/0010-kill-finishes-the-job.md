# 0010 — `pty kill` finishes the job

**Status:** accepted

**Node behavior.** The daemon tears down the child's process tree on its way
out, with exact per-pid TERM then KILL signals. `pty kill` waits for the daemon
and then returns. Whatever the daemon did not manage is nobody's work after
that.

**Rust behavior.** After the daemon has gone, `pty kill` re-reads the process
table. If anything from its pre-kill snapshot is still alive, it signals the
process groups the session left behind, waits, escalates to SIGKILL, then reads
the table again and reports what is still there. It exits non-zero unless it
verified the tree is empty.

**Why the command and not the daemon.** The daemon does its teardown inside a
process that is trying to die, so the teardown races its own exit. The command
outlives the daemon, which puts it in the only position from which the job can
be finished.

**Why process groups.** A group signal needs no per-process identity.
`snapshot_from_listing` drops a descendant whose start token cannot be read, and
that process is then never signalled — see [decision 0009]. Collecting groups
from the raw `ps` listing reaches it anyway. **The blind spot is not solved; it
is made irrelevant.** The sweep is also cheaper: reading start tokens costs one
`ps` per descendant on macOS, and a sweep costs one `ps` in total.

**The daemon's own group is not a target.** Measured on Linux for both tools on
2026-09-03: the pty child calls `setsid`, so it starts a new session and a new
group, and the daemon sits alone in its own group. Signalling the daemon's group
would reach the daemon and nothing else. The child's group held exactly the
child and its descendants, and nothing else on the machine was in it, so the
sweep is as precise here as a pid list.

**A zombie is not a member.** `ps` lists a zombie with its process group, so
counting it makes the sweep report a group it has already emptied and signal it
again. The state column is read for this reason. Found by a test against a real
process group, not by inspection.

**One TERM and hope is known not to work.** A coding agent was measured ignoring
SIGTERM for ten seconds on 2026-09-02. The sweep therefore escalates, and the
escalation is verified rather than assumed.

**Exit code.** Set by [decision 0009], which this record does not change: it is
non-zero when anything survived and when the outcome is undecided. Escalation
makes the zero mean more, because the command now clears what it found rather
than only naming it.

**No opt-out flag.** `kill` kills. An opt-out was considered and left out.

**What this does not reach.** A descendant that calls `setsid` leaves the
session and the group. Neither the tree walk nor the group sweep can see it,
and this record does not change that. A process group created after the
snapshot, by a process that had already left the tree, is likewise unreachable.

**What is tested and what is not.** The group selection, the TERM-then-KILL
ordering, the refusal to signal the running process's own group, and the zombie
exclusion are pinned by unit tests. The sweep is pinned end to end against a
**real process group that ignores SIGTERM**, which is checked against the
process table afterwards rather than against the signals sent. **The path from
`pty kill` to a survivor is not pinned end to end**, because a survivor that
`pty kill` alone produces cannot be manufactured on Linux: the daemon's exact
teardown succeeds when start tokens are readable, and the terminal hangup clears
the foreground group for free.
