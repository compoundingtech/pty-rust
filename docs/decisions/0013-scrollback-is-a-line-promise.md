# 0013 — scrollback is a line promise, and libghostty counts bytes

**Status:** accepted

**Node behavior.** Node's daemon passes `scrollback: 10000` to
`xterm-headless` (`src/server.ts:333-338`), where it is a line count: the
buffer holds 10 000 lines of history and evicts the 10 001st line. `pty stats`
reports `scrollbackUsed` (`buffer.active.length`, `src/server.ts:1128`) against
`scrollbackCapacity` (`rows + scrollback`), and that capacity is a ceiling the
buffer never exceeds.

**Rust behavior.** `TerminalActor::new(rows, cols, scrollback)` still takes a
line count, and now converts it to the byte budget libghostty actually wants.

libghostty's `Options::max_scrollback` is documented as "maximum number of
lines to keep in scrollback history" and is not: it is a byte budget for the
history page list, and the list evicts whole pages. Measured against
libghostty-vt 0.2.1, a terminal given `max_scrollback: 10_000` and fed 10 008
short lines keeps:

| columns | rows retained | oldest line surviving |
| --- | --- | --- |
| 20 | 3 310 | `L6698` |
| 80 | 745 | `L9263` |
| 200 | 456 | `L9552` |
| 400 | 149 | `L9859` |

The retained count scales inversely with the width, which a line count cannot
do, and doubling the number to 20 000 changes nothing at 80 columns — both are
smaller than one page, and one page is the floor. Passing a line count straight
through therefore delivered 7% of the promised history at 80 columns, and the
number `scrollback_capacity()` reported (`24 + 10_000`) described nothing that
existed.

The conversion is `SCROLLBACK_ROW_OVERHEAD + SCROLLBACK_BYTES_PER_COL * cols`
per line (256 + 16), against a measured cost of ~838 bytes per row at 80
columns and ~1 804 at 200 — roughly 1.8x headroom, because the page list rounds
up to whole pages and because a row of styled or multi-codepoint cells costs
more than a plain one. The total is capped at `MAX_SCROLLBACK_BYTES` (64 MiB,
which is 10 000 lines of a 400-column terminal).

Three reads describe the result instead of the request:

- `scrollback()` — lines actually retainable at the current width.
- `scrollback_request()` — what the owner asked for, met or not.
- `scrollback_bytes()` — the budget libghostty holds the history in.

**Why.** The alternative was to leave the number as libghostty takes it and
weaken the promise to "up to N lines", which is what the first Fractal test did
when it asserted `used <= capacity` — an assertion that passes when 6 398 of
10 008 lines have been thrown away. A replay window is a product promise: a
consumer that shows history decides what to keep on the basis of that number,
and the honest options were to meet it or to publish a smaller one. Meeting it
costs memory that is bounded, documented, and only touched when the history
actually fills; publishing a smaller one would have meant every consumer
carrying its own conversion from lines to whatever libghostty's number means
this release.

libghostty exposes the budget only in `Options`, with no setter, so it is fixed
at construction. That is the source of the one remaining gap below.

**Client effect.** A session asked for 10 000 lines retains 10 000 lines at the
width it was created with, and `stats` reports numbers that are true. Memory
per session with the default 10 000 lines: 14.6 MiB of budget at 80 columns
(33 MiB at 200, 63.5 MiB at 400), against ~1 MiB before — the budget is address
space the page list fills only as history accumulates, so an idle session pays
nothing.

Residual differences a consumer can observe:

1. `scrollback_capacity()` is a guaranteed minimum, where Node's is a ceiling.
   libghostty never holds less than one page, so a terminal asked for a small
   scrollback keeps more than it promised: a 100-line request at 80 columns
   retains about 1 000 rows. The promise is met and then some; code that
   treated the number as an upper bound on `scrollback_used` has to stop.
2. Widening a terminal reduces the line count it can retain, because the byte
   budget is fixed at construction and a wider row costs more. `scrollback()`
   and `scrollback_capacity()` follow the width down; `scrollback_request()`
   keeps saying what was asked for. A terminal created at 80 columns and
   widened to 400 holds about a fifth of the lines. Fixing this needs either a
   libghostty setter for the budget (upstream) or budgeting for the widest
   plausible width at construction (10 000 lines at 1 000 columns is 154 MiB
   per session, which is not worth it).
3. A request whose budget exceeds `MAX_SCROLLBACK_BYTES` is clamped, and
   `scrollback()` reports the clamped line count rather than the request.

**Test.** `crates/pty-terminal/tests/scrollback.rs` — six cases, all of which
fail on the pass-through:
`ten_thousand_lines_of_history_are_all_retained` (the oldest of 10 008 lines is
still `L0`, at 24x80 with the default scrollback),
`the_promise_holds_at_every_width` (the same at 20, 80, 200 and 400 columns,
where the per-row cost differs by 5x),
`a_small_scrollback_keeps_at_least_what_it_promised` (the promised window is
present and the far past is gone),
`used_and_capacity_are_honest`,
`a_request_beyond_the_memory_bound_reports_what_fits`, and
`a_widened_terminal_reports_the_history_it_can_still_hold`.

Every line written by these tests is short enough that it cannot wrap, so no
result here depends on reflow: a row written is a row of history.

No gated `_node` / `_rust` conformance pair exists for the byte conversion
itself — it is an implementation detail of reaching Node's behaviour, not a
deviation from it. The observable deviations are the three above.

**Migration / negotiation.** None. `TerminalActor::new` and
`SpawnOptions`/`AttachOptions` still take a line count and now honour it; a
consumer reading `scrollback_capacity()` as a ceiling should read it as a floor
(residual 1), and one that resizes should re-read `scrollback()` rather than
assume the original number (residual 2).
