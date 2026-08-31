# pty-testkit

Write tests against a terminal program the way a person uses it: start it,
type at it, and look at the screen.

The screen is a real terminal — [libghostty][], the emulator from
[Ghostty][] — so what a test sees is what the program actually drew, escape
sequences, wide characters and all.

[libghostty]: https://libghostty.tip.ghostty.org/
[Ghostty]: https://ghostty.org

## Two ways to get a session

**Spawn** puts a process on the end of a pty this library opens. Use it for a
command-line tool or a full-screen program.

```rust,no_run
use pty_testkit::{Session, SpawnOptions};

let mut s = Session::spawn("bash", &["--norc"], SpawnOptions::default())?;
s.wait_for_text_default("$")?;
s.type_str("echo hello\r");
s.wait_for_text_default("hello")?;
s.close();
# Ok::<(), Box<dyn std::error::Error>>(())
```

**Server** asks the `pty` binary for a session and talks to its daemon over
the session socket. Use it to test what a real client sees: the screen
arrives as frames, a reconnect replays it, and two clients can watch at once.

```rust,no_run
use pty_testkit::{ServerOptions, Session};

let mut s = Session::server("bash", &["--norc"], ServerOptions::default())?;
s.wait_for_text_default("$")?;

// A client that loses its connection and comes back sees the same screen.
s.reconnect()?;
s.wait_for_text_default("$")?;

// Closing a session this handle created stops it and cleans up after it.
s.close();
# Ok::<(), Box<dyn std::error::Error>>(())
```

The binary is `PTY_BIN`, else `pty` on PATH.

## Looking at the screen

- `screenshot()` — the text, the lines, and the ANSI.
- `wait_for_text` / `wait_for_absent` / `wait_for`, each with a `_default`
  form that allows ten seconds.
- `actor()` — the terminal itself, for typed reads: the cell grid, the
  cursor, the modes, the scrollback.

## Typing

- `type_str` and `send_keys` for literal text.
- `press("ctrl+c")` for a named key. `ctrl+u`, `ctrl-u`, `ctrl_u` and `C-u`
  all mean the same thing.

## Size

`resize(rows, cols)` in spawn mode sets the size. In server mode it asks: the
daemon gives every client the smallest size any of them requested, so read
`rows()` and `cols()` afterwards to see what you got.
