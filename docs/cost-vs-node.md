# What this pty costs, next to the Node one

Measured on 2026-08-28, on a 16-core Linux machine with 62 GB of memory, against
`pty` 0.12.0 (Node, `@xterm/headless`) and `pty-rust` 0.1.0 (libghostty).

**Read the summary as: much less memory, about the same CPU.**

## Summary

Per session, measuring only the daemon process.

| | Node | Rust | |
|---|---|---|---|
| RSS at rest | 58.8 MB | 4.3 MB | 14x |
| PSS at rest | 16.1 MB | 0.8 MB | 20x |
| RSS under load | 158.6 MB | 6.1 MB | 26x |
| PSS under load | 110.6 MB | 2.4 MB | 46x |
| CPU under load | 1.96 s | 1.44 s | **1.4x** |
| `peek` round trip | 40.8 ms | 2.6 ms | 15x |

**The CPU column is the one people misread.** Rust used 1.44 seconds where Node
used 1.96 to process the same bytes. That is 1.4 times, not 20. **Choose this
port to make sessions smaller, not to make them faster.**

**Neither daemon gives the memory back.** Ten seconds after the load drained,
both were still holding what they had grown to. A session that once printed a
lot keeps that memory for as long as it lives.

## Why both RSS and PSS

**RSS counts shared pages once per process, so adding it across many Node
daemons counts the same memory many times.** Ten Node daemons share most of the
runtime. `Pss` in `/proc/<pid>/smaps_rollup` divides each shared page among the
processes mapping it.

**PSS is the honest answer to "what does one more session cost".** RSS is here
too, because it is what `ps` and `top` show and people compare against it.

## The load

**A daemon's work is turning bytes into screen state, so the load is bytes.**

A file of 20,000 lines. Each line carries a colour escape, a zero-padded number,
a reset escape, and 55 characters of text. The file is 1,440,000 bytes.

Each session runs `cat` over that file 60 times, which is 86 MB through the
terminal emulator. Five sessions run at once. The command is sent into an
already-running shell, so the daemon under measurement is the same process
before and after.

**A marker line is echoed after the last `cat`.** The run ends when that marker
appears on every session's screen, so the measurement covers the whole load
rather than a fixed wait.

## The method

**Only the daemon is measured.** Each session also runs a shell and a `cat`, and
those are identical for both ports.

Daemons are found by reading `/proc/<pid>/environ` and matching the `PTY_ROOT`
of the test registry. **Selecting them by process name would sweep in every
daemon already running on the machine**, which is how the first attempt at this
produced numbers that were wrong by a factor of four.

- CPU is `utime` plus `stime` from `/proc/<pid>/stat`, in clock ticks, sampled
  before and after the load.
- Memory is `Rss` and `Pss` from `/proc/<pid>/smaps_rollup`.
- At rest is sampled after the session has been idle for six seconds.
- `peek` latency is 20 sequential calls, divided.

**Three runs. The spread between runs was under two percent.** The figures above
are the middle run.

## What this does not tell you

**No agent harness was measured.** The sessions ran an interactive shell. The
memory and CPU of a real coding agent are its own, not the daemon's, but a real
harness draws far more than a shell does and would grow both daemons more.

**Rendering was checked with `vim` and `htop`, not with a coding agent.** Both
draw correctly here: alternate screen, cursor addressing, colour, and redraws.
Both accept input. That covers the common terminal features. **It does not prove
any particular program is correct, and a program that draws unusually could still
find a gap.**

**Nothing here was measured over days.** Both daemons hold onto what they grow
into, so the gap between them should widen over a long-lived session. That is a
prediction from the ten-second reading, not a measurement.

## Reproducing it

Both binaries must be on the machine. Give each run its own empty `PTY_ROOT` so
the registry is isolated, start five sessions running a shell, sample, send the
`cat` loop with `pty send`, wait for the marker with `pty peek`, and sample
again.
