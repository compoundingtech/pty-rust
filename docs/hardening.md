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

## What is enforced

- **Frame size.** A declared length above 32 MiB drops the connection, on
  both sides. `PacketReader::feed` returns `InvalidData`; the daemon closes
  the socket and keeps serving everyone else.
- **Socket path length.** A `PTY_ROOT` whose session socket path would
  exceed the kernel's 104-byte `sun_path` limit is refused before anything
  is created, with the path and the limit in the message.
- **Creation locks.** `<id>.lock` held by a live process turns a second
  creator away. A lock whose holder is dead, or whose contents are garbage,
  is stolen — and two concurrent stealers cannot both win.
- **Session names.** Validated before a spawn, so automation fails with a
  message rather than deep inside a syscall.
- **Generation tokens.** `pty exec` rewrites a session's command only while
  the generation it was given still matches, so a session that has been
  restarted underneath the caller refuses the change and runs nothing.

Tests: `crates/pty-conformance/tests/{fixtures_protocol,security_fixes,pty_root,exec}.rs`.
