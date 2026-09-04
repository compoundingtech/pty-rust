# 0007 — A detaching client leaves the books at once

**Status:** accepted

**Node behavior.** A `DETACH` closes the client's socket
(`src/server.ts`, `socket.end()`), and the client is removed from the
daemon's map in the socket's `close` handler. So there is a window between a
client observing its own socket close and the daemon recording that it has
gone. `pty stats` inside that window counts a client that has already left.

**Rust behavior.** The client is removed when the `DETACH` is handled, before
the close comes back. `pty stats` never counts a client that has asked to
leave.

**Why.** There is nothing to wait for. The client has said it is going, its
socket is being closed in the same breath, and the size negotiation should
stop counting it for the same reason. Waiting for the close to come back
around adds nothing except a period in which the answer is wrong.

**How it was found.** A conformance test detaches a client, waits for its
socket to close, reattaches and asks how many clients are attached. It expects
one and got two, on Apple silicon, three times out of three. It has never
failed on Linux.

**Measured on 2026-09-02:** 60 detach-and-immediately-reattach cycles on
Linux gave 0 stale readings, for **both** implementations. The window exists
in both and is too narrow to observe there. It is wide enough to observe on
Apple silicon, which fits `docs/parity.md` §12c: a daemon there learns of a
departure from an ordinary end of stream rather than from a reset.

**Client effect.** Anything that detaches and then asks the daemon a question
gets an answer that is right sooner. Nothing that was true before stops being
true: a client that dies without detaching is still noticed by its socket
closing, and `on_closed` still runs and finds nothing left to remove.

**Not reproducible on the machine that made this change**, and that is stated
rather than hidden. Linux cannot demonstrate the defect or the fix; the port
is structurally correct where the Node tool is structurally racy, and the
machine that showed the difference is the one that can confirm it.
