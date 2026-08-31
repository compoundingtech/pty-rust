# @compoundingtech/pty-testing

Write tests against a terminal program the way a person uses it: start it,
type at it, and look at the screen.

The session is hosted by the `pty` binary, so a test sees exactly what any
client sees — the screen arrives as frames over the session socket, a
reconnect replays it, and two clients can watch at once.

## Getting started

```ts
import { Session } from "@compoundingtech/pty-testing";

const s = await Session.spawn("bash", ["--norc"]);
await s.waitForText("$");
s.type("echo hello\r");
await s.waitForText("hello");
await s.close();
```

`Session.spawn` makes a session in a temporary registry and attaches to it.
`close()` stops it and removes the registry.

## The engine

The binary is `PTY_BIN`, else `pty` on PATH. The first call checks it is the
Rust one and refuses otherwise, because this package is written against its
behaviour. `PTY_TESTING_ALLOW_NODE=1` says you meant the other one.

## Looking at the screen

- `screenshot()` — `{ lines, text, ansi }`.
- `waitForText(text)`, `waitForAbsent(text)`, `waitFor(predicate)`. Each
  allows ten seconds unless you say otherwise, and the timeout message
  carries the screen it gave up on.

## Typing

- `type(text)` and `sendKeys(text)` for literal text.
- `press("ctrl+c")` for a named key. `ctrl+u`, `ctrl-u`, `ctrl_u` and `C-u`
  all mean the same thing; the error tells you what a spec may contain.

## More than one client

```ts
const first = await Session.spawn("bash", ["--norc"]);
const second = await Session.connectToExisting(first, { rows: 10, cols: 40 });
```

The daemon gives every client the smallest size any of them asked for, so
read `rows` and `cols` after a `resize` to see what you got. Closing a client
that did not create the session leaves the session running.

`Session.connect(name, { root })` attaches to a session by id.

## Losing the connection

`reconnect()` drops the socket and opens a new one. The screen is rebuilt
from the daemon's replay, which is what a client that dropped off the network
sees when it comes back.

## Publishing

`npm publish` from this directory on a tag `pty-testing-vX.Y.Z`.
