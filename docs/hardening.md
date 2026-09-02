# Hardening notes

What the daemon and the CLI refuse, and one incident worth reading before
you change any of it.

## The frame-size limit was not enforced

**Found on 2026-08-31, on `parity`. It had been unenforced for as long as the
daemon had existed.**

A client talks to a session over `<root>/<name>.sock` in 5-byte-header
frames: one type byte, then a 32-bit big-endian length. A peer that declares
a length above `MAX_PACKET_LENGTH` (32 MiB,
`crates/pty-core/src/protocol.rs`) must have its connection dropped, or it
can make the daemon buffer without bound.

The daemon's reader thread did check the length, and on a bad one it printed
a diagnostic and then shut the socket down. **The print never returned.**

The CLI that starts a session pipes the daemon's stderr so it can report a
daemon that dies on the way up, and it stops reading when it exits. From
that moment every write to the daemon's stderr fails with `EPIPE`, and
`eprintln!` panics when a write fails. So the reader thread died at the
print, **before the line that dropped the connection**. The oversized frame
was accepted, the peer stayed connected and trusted, and nothing anywhere
said so.

**The test for it was red the whole time.** `fixtures_protocol.rs`'s
`an_oversized_declared_length_drops_the_connection` was failing in a suite
with many other failures, so it read as one more unported behaviour rather
than as a disabled protection.

Two things follow, and they are the reason this note exists.

**A red test in a red suite is invisible.** When a suite has failures for
known reasons, a failure for an unknown reason hides among them. Anything
that fails for a safety reason should be separated from anything that fails
because a feature has not been written yet.

**The daemon must never treat a diagnostic as something that can fail
loudly.** Its stderr has no reader for almost all of its life. Use
`daemon_warn!` (`crates/pty/src/daemon/mod.rs`), which ignores a write that
goes nowhere, exactly as the Node daemon does. `eprintln!` in the daemon is
a latent thread-killer, and the thread it kills is whichever one happened to
have something to report. Start-up messages are the one exception, because
the launching CLI is still reading the pipe while they are written.

## The remote tunnel deadlocked on its own stdout

**Found on 2026-08-31, while the remote work was being written. It never
shipped, and it is here because the shape is the same as the one above.**

`pty remote-serve --stdio` reads one request line from stdin, answers on
stdout, and then splices the session socket to both. The first version read
the socket on a second thread and wrote it to stdout from there.

**That thread could never write.** The request line had been read through the
stdin lock, and the main thread still held the stdout lock for the whole life
of the tunnel — which is forever. The reader blocked on a lock that would not
be released until it had finished.

**Nothing said so.** `peek --remote` timed out and `send --remote` delivered
nothing. Neither reported an error, because nothing errored: a thread was
waiting, correctly, for something that was never going to happen.

It now reads stdin on the thread and writes stdout on the main one, on the
raw descriptors rather than the locked handles.

**The lesson is the one above, from the other side.** Both failures were
silence rather than an error, and both were found by looking at the RESULT —
an oversized frame that was still accepted, a peek that returned nothing —
rather than at an exit code. **A component that reports failure by not
finishing needs a test that asserts on what it produced.**

## When a green test turns red after a fix, suspect the test first

**Three tests in this repository were passing for reasons unrelated to what
they checked, and all three were found on 2026-08-31 the same way: something
was made MORE correct and they went red.**

- Two `stats` tests matched the word `exited` inside a registry summary the
  CLI fell back to when the daemon was gone. Fixing the daemon lifecycle kept
  the daemon alive, the fallback stopped being reached, and the tests failed.
- `send --remote` to a missing session passed because `send` resolved a
  **remote** name against the **local** registry and reported "not found".
  Making it dial the peer removed the wrong answer that happened to match.
- The terminal-handle tests built their temp directory from
  `Instant::now().elapsed()`, which is not a clock reading. They shared one
  registry and raced for the same session id, and whoever arrived second
  quietly won. Making `run` take the creation lock turned the race into a
  refusal, which is correct, and they failed.

**Each looked like a regression and none was.** The improvement did not break
the test; it removed the accident the test had been relying on.

**So the rule is not "go hunting".** A deliberate hunt for a fourth on
2026-08-31 found none, while three arrived unbidden from ordinary work. The
rule is the inversion of the natural instinct:

> When a test that was green turns red after a fix, check whether the test was
> ever testing what it claims, before you assume the fix is wrong.

**A test that asserts on a short common word, shares mutable state, or derives
uniqueness from a clock is a candidate.** `docs/parity.md` §12b lists the
substring candidates in the conformance suite that nobody has yet examined.

## The conformance suite does not build the binary it tests

**`cargo test -p pty-conformance` runs whatever is sitting at
`target/debug/pty`.** The harness resolves that path (or `PTY_TEST_BIN`) at
run time, and cargo has no reason to rebuild a binary that no test target
depends on. So a source change you have not built is simply not in the run.

**It bites hardest when you are proving a guard.** On 2026-09-02 a new test
for the `pty peek` metadata race was checked by putting the defect back and
expecting red. It stayed green through two runs. The defect was in the
source and the test was running the fixed binary from before the edit. **The
verdict was about a file nobody had compiled.**

Both readings of that green are wrong in the same direction: it says the test
is weak, or it says the defect is gone. **Run `cargo build -p pty` first, and
the same green means something.** With the binary rebuilt, the test caught the
defect in 12 of 300 runs.

**This is the empty-corpus failure wearing a build system.** Nothing errors,
nothing warns, and the run that measured nothing prints what a clean run
prints.

## Stealing a stale lock is not exclusive, and this file used to say it was

**Measured 2026-09-02. Eight threads racing for one stale lock, four hundred
rounds: 386 rounds had more than one winner.** Exclusion held in 14.

The steal is three steps and nothing binds them together:

    open(O_CREAT|O_EXCL)   -> fails, a lock file is there
    read the holder pid    -> the holder is dead, so this lock is stale
    unlink, then create    -> take it

Two processes both reach step three believing the same thing:

1. A unlinks the stale file and creates its own. **A now holds the lock.**
2. B, whose decision came from the file A has already replaced, unlinks —
   **and what it unlinks is A's live lock** — and then creates its own.
3. Both hold an armed guard. Either one's drop unlinks the other's file, and
   a third process can then walk in while both still believe they own it.

**The unlink is the fault. A loser must never be able to remove a winner's
file**, and here the loser cannot tell that the file it is removing is not the
one it inspected.

**The Node tool has the identical sequence and the identical defect**
(`src/sessions.ts`, `acquireFileLock`). So this is not a difference between the
two implementations and a mixed registry is no worse than either alone. Its
comment there makes the same claim this file did: *"only one wins the wx open;
the loser returns false instead of stomping on the winner's lock."* The loser
stomps first and creates second.

**Two tests carry the old belief in their names and neither establishes it.**
`security_fixes.rs::concurrent_stealers_cannot_both_win` races two spawned
processes, and process start-up jitter is what makes it pass;
`registry_locks.rs::only_one_of_two_sequential_steals_wins` is honest about
being sequential. Reproduce the real behaviour with a barrier: N threads that
all wait, then all call `acquire_lock` on one stale lock, counted over many
rounds. It does not need process spawning and it does not need luck.

**A correct steal needs one exclusive create that only one process can win**,
which means a second lock file to funnel the steal through, and that changes a
protocol both implementations share. **It is not fixed here, on purpose. The
decision is not one implementation's to take alone.**

**When it bites:** a daemon crashes and leaves its lock behind, then two
creators for the same id arrive together. Then two daemons can own one name,
with socket rebinding and last-writer-wins metadata, or two event writers can
interleave a truncation with an append.

## A test that watches a stream must start the stream itself

**Found 2026-09-02, on a slower machine, and it read as a character-encoding
defect for most of an afternoon.**

A test had a child print one byte at a time so it could check that a
multi-byte character split across several DATA frames still arrives whole. The
child slept for a third of a second first, to let the client attach.

On a slower machine **the first byte of the sample never appeared**, and every
other byte arrived correctly, one per frame. That looks exactly like a decoder
losing its place at a character boundary. It is nothing of the kind.

**Anything the child writes before the daemon has processed an ATTACH reaches
that client in the initial SCREEN, not as a DATA frame.** The test collected
DATA only, so an early byte was not late — it was invisible.

**The sleep was the bug.** It made the race rare enough to look like something
else, and rare enough that the same test passed under load, because a busy
machine attaches at a different point in that third of a second. **A test that
passes when the machine is busy and fails when it is idle is the shape to
recognise**; the usual flake is the other way round.

The child now waits for a file the test creates after the SCREEN has arrived,
so the stream cannot begin before the watcher is watching. Two controls: never
open the gate and no bytes arrive at all; delay the attach by far longer than
the original sleep and it still passes.

**And the first assertion could not have told anybody this.** It concatenated
the frames and printed them as lossy text, so a character split across two
frames and a character mangled inside one frame produced the same message —
and those want completely different searches. It now prints the wanted bytes,
the received bytes, and each frame separately. **The rewritten message found
the cause in one run.**

## What is enforced

- **Frame size.** A declared length above 32 MiB drops the connection, on
  both sides. `PacketReader::feed` returns `InvalidData`; the daemon closes
  the socket and keeps serving everyone else.
- **Socket path length.** A `PTY_ROOT` whose session socket path would
  exceed the kernel's 104-byte `sun_path` limit is refused before anything
  is created, with the path and the limit in the message.
- **Creation locks.** `<id>.lock` held by a live process turns a second
  creator away. A lock whose holder is dead, or whose contents are garbage,
  is stolen. **Two concurrent stealers CAN both win. See below.**
- **Session names.** Validated before a spawn, so automation fails with a
  message rather than deep inside a syscall.
- **Generation tokens.** `pty exec` rewrites a session's command only while
  the generation it was given still matches, so a session that has been
  restarted underneath the caller refuses the change and runs nothing.

Tests: `crates/pty-conformance/tests/{fixtures_protocol,security_fixes,pty_root,exec}.rs`.
