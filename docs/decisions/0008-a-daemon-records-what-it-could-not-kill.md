# 0008 — A daemon records what it could not kill

**Status:** accepted

**Node behavior.** A daemon that has signalled its child's process tree with
TERM and then KILL collects the processes that are still alive, and reports
them by writing to its own standard error (`src/server.ts`). Nothing else
records it. `pty kill` waits on the daemon and cannot see a surviving child,
so it prints its success line either way.

**Rust behavior.** The same, and the daemon also appends
`session_descendants_survived` to the session's event log, carrying the pids
under `data`.

**Why.** A daemon's standard error has no reader for almost all of its life —
the command that launched it stopped listening once the session was published.
So the one moment the daemon has something worth saying is the one moment
nobody is there to hear it. The information existed and reached no one.

**How it was found.** On 2026-09-02 a coding agent survived a `pty kill` on a
Mac, its supervisor then started a second one on the same session id, and two
processes wrote to one transcript. The command had printed
`Session "…" killed.` The daemon had almost certainly noticed the survivor and
said so, into a stream with no reader.

**Client effect.** `pty events <session>` now shows it, and so does anything
following the log. Nothing else changes: `pty kill` prints and returns exactly
what it did before, and the daemon's stderr warning is unchanged. **This adds a
record; it does not add a guarantee.**

**Compatibility.** The type is new and the Node tool never writes it. Its
reader renders an unknown type through the same fallback it uses for `user.*`
events, printing the `data` object, so a Node reader on a shared root shows the
line rather than failing on it.

**What is tested and what is not.** The event's shape is pinned by a test. The
trigger is not: reaching it needs a descendant that outlives both a TERM and a
KILL, and a process that survives SIGKILL cannot be manufactured in a test.
