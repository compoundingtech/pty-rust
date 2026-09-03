# 0011 — One reader for the process table

**Status:** accepted

**Node behavior.** Every caller that needs a fact about a process runs its own
`ps` and treats the output as fact.

**Rust behavior.** One module, `pty_core::proctable`, answers every such
question. It reads `/proc` on Linux and calls `proc_listpids` / `proc_pidinfo`
on macOS. **Neither spawns a subprocess.** One `ps` call remains in the whole
codebase, and it is documented in place.

**Why.** Three wrong answers this year came from the same shape, and the shape
is not a parsing bug:

- a descendant was dropped from a teardown because its start token could not be
  read;
- `registry_list` failed under load because `ps` went quiet;
- the Node tool read an empty `stat` field as "exited".

**A subprocess can be slow, truncated or silent, and all three look exactly
like "the process is gone".** Reading `ps` more carefully makes the next
instance rarer, not impossible.

**The arithmetic that settled it.** `wait_for_identities_to_exit` polls every
25 ms and asked the operating system about each surviving descendant
separately. On macOS each question was a `ps` spawn. Inside the 1500 ms TERM
budget that is up to 60 iterations, so four descendants cost 240 spawns. One
spawn was measured at 10.9 ms median, so that is **2.6 seconds of spawning
inside a 1.5 second deadline**. The teardown could not meet its own deadline
for a tree of four, on an idle machine. That is arithmetic, not load
sensitivity.

**Silence is a third answer.** Every query returns `Answer`, which separates
`Known`, `NotPresent` — the table was read and the process was not in it — and
`Unknown`. There is no `Default`, no `unwrap_or`, and no conversion to `Option`
that loses the distinction. A caller that wants silence to mean death must
write `or_absent_when_unknown()`, which is long on purpose and greps in one
command. **This does not make the mistake impossible. It makes it visible.**

**A table that does not contain the process that read it was truncated, not
empty.** `ps` always lists at least itself, so that one comparison turns a
silent or half-written listing into `Unknown` rather than "nothing exists".

**One process, not the whole table, for a single question.** A full table read
costs about as much as 473 single reads — measured on Linux on 2026-09-03,
4.21 ms against 0.0089 ms. So `proctable::process(pid)` exists alongside
`ProcTable::read()`, and the poll loops use it. Reading the whole table in
those loops would have fixed macOS by making Linux several hundred times
slower.

**Two identities, deliberately different types.** `LiveIdentity` is private to
one command's lifetime and is free to be whatever is cheapest — on macOS a
microsecond start time from `proc_bsdinfo`, which is stronger than what `ps`
prints. The registry's `recovery.processStartToken` is a different thing: it is
written into session metadata and read back by the Node tool from the same
registry, so **its exact text is a contract between two programs**. Making them
separate types is what stops the two from meeting.

**Why the last `ps` stays.** `read_process_start_token` on macOS still runs
`ps -o lstart=`, because that text is the contract above. It is one call per
session lookup, never inside a poll loop, and its failure already means "cannot
confirm" rather than "gone".

**Not sysctl.** `KERN_PROC_ALL`, `CTL_KERN`, `KERN_PROC` and `sysctl` are in the
libc crate, but **`kinfo_proc` and `extern_proc` are not**, so that route needs
a hand-declared struct. libproc needs no new dependency: `proc_listpids`,
`proc_pidinfo`, `proc_bsdinfo`, `PROC_PIDTBSDINFO` and `proc_taskinfo` are all
in libc 0.2.189, already in the tree.

**What is tested and what is not.** The three answers, the truncation guard,
the empty-column case, and a `ps` that is slow, silent or missing are all
pinned, the last three against fake programs. The real table is checked against
this very process and against a real zombie. **The macOS reader is compiled but
not run**: `cargo check --target aarch64-apple-darwin` type-checks it against
the real macOS definitions, and a control confirms that check bites — breaking
one field name fails the darwin check while the Linux check still passes. **No
macOS machine has executed this code.**
