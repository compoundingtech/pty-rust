# Node `pty` package: testing library, TUI framework, interactive session manager, pty-layout

Inventory of `<node-pty-checkout>` (v0.12.0, `package.json:3`) for the Rust rewrite.
All paths below are relative to that repo unless prefixed with `/home/...`. Line numbers are as of the working tree read on 2026-08-29.

Package exports (`package.json:38-63`): `./testing` -> `dist/testing/index.js`, `./tui` -> `dist/tui/index.js`, plus `./client`, `./server`, `./protocol`, `./keys`. Runtime deps (`package.json:76-82`): `@preact/signals-core`, `@xterm/addon-serialize`, `@xterm/headless`, `node-pty`, `smol-toml`.

---

## 1. TESTING LIBRARY (`@compoundingtech/pty/testing`)

Source: `src/testing/index.ts` (2 lines), `src/testing/types.ts` (33), `src/testing/screenshot.ts` (25), `src/testing/session.ts` (458). Docs: `docs/testing.md` (408 lines, tagline "like Playwright, but for the terminal", `docs/testing.md:3`).

### 1.1 Public surface

`src/testing/index.ts:1-2` exports exactly: `Session` (class) and types `Screenshot`, `SpawnOptions`, `ServerOptions`. (`buildSpawnEnv` is `export`ed from `session.ts:47` "for unit testing" but is not re-exported from the index.)

#### Types (`src/testing/types.ts`)

```ts
interface Screenshot {            // types.ts:2-9
  lines: string[];   // plain text lines; trailing whitespace trimmed per line; trailing empty lines removed
  text: string;      // lines.join("\n") — for .toContain()
  ansi: string;      // full ANSI-serialized terminal state incl. escape codes (colors, bold...)
}
interface SpawnOptions {          // types.ts:12-21
  rows?: number;     // default 24
  cols?: number;     // default 80
  cwd?: string;      // working dir of spawned process
  env?: Record<string,string>;  // merged over process.env
}
interface ServerOptions {         // types.ts:24-33
  name?: string;     // session name; auto-generated if omitted
  rows?: number; cols?: number; cwd?: string;
}
```

#### `class Session` (`src/testing/session.ts:62`)

Private fields: `terminal: xterm.Terminal`, `serialize: SerializeAddon`, `backend: Backend`, `_rows/_cols` (effective), `_requestedRows/_requestedCols` (`session.ts:63-69`). `Backend` is a tagged union (`session.ts:21-23`):
- `{ kind: "spawn"; ptyProcess: pty.IPty }`
- `{ kind: "server"; server: PtyServer; ownsServer: boolean; socket: net.Socket; reader: PacketReader; screenCallbacks: Array<() => void>; exitCode: number | null; name: string }`

Factories:
- `static spawn(command: string, args: string[] = [], opts: SpawnOptions = {}): Session` (`session.ts:100-133`). Creates `new xterm.Terminal({ rows, cols, scrollback: 10000, allowProposedApi: true })` + `SerializeAddon` (`108-114`), builds env via `buildSpawnEnv(process.env, opts.env)` (`117`), then `pty.spawn(command, args, { name: "xterm-256color", cols, rows, cwd: opts.cwd ?? process.cwd(), env })` (`119-125`); `proc.onData -> terminal.write` (`127-129`). Synchronous; no readiness wait.
- `static async server(command, args = [], opts: ServerOptions = {}): Promise<Session>` (`session.ts:140-183`). Constructs an in-process `new PtyServer({ name, command, args, displayCommand: command, cwd, rows, cols })` (`149-157`), `await server.ready` (`158`), creates xterm + serialize, builds a server backend with `ownsServer: true`, then `await session.connectSocket()` (`181`). Does NOT attach; caller must `await session.attach()`.
- `static async connectToExisting(existing: Session, opts: { rows?, cols? } = {}): Promise<Session>` (`session.ts:190-224`). Throws unless `existing` is server-mode (`194`); reuses `existing.backend.server` with `ownsServer: false` (`210-219`), defaults rows/cols to the existing session's requested size (`198-199`), connects a second socket. Second client for multi-client tests.

Properties:
- `get rows(): number` / `get cols(): number` — current effective size (`session.ts:229-236`), updated from daemon GEOMETRY packets in server mode.
- `get hasExited(): boolean` — server-mode only (`exitCode !== null`); always `false` in spawn mode (`239-244`).
- `get server(): PtyServer` — throws in spawn mode (`247-252`).
- `get name(): string` — throws in spawn mode (`255-260`).

Input:
- `sendKeys(keys: string): void` — spawn: `ptyProcess.write(keys)`; server: `socket.write(encodeData(keys))` (`265-271`).
- `press(keyName: string): void` — `sendKeys(resolveKey(keyName))` (`277-279`).
- `type(text: string): void` — alias of `sendKeys` (`282-284`).

Screen:
- `screenshot(): Screenshot` — `captureScreenshot(terminal, serialize)` (`289-291`).

Waiting (all poll every 50 ms, default timeout 10000 ms, throw `Error` whose message includes the current screen text):
- `async waitForText(text: string, timeoutMs = 10000): Promise<Screenshot>` (`305-316`): loop `while (Date.now()-start < timeoutMs) { await sleep(50); ss = screenshot(); if (ss.text.includes(text)) return ss; }` then throws `Timed out after Nms waiting for "text".\nScreen:\n<text>`.
- `async waitForAbsent(text, timeoutMs = 10000)` (`319-330`): same loop, returns when `!ss.text.includes(text)`; error `... waiting for "text" to disappear`.
- `async waitFor(predicate: (ss: Screenshot) => boolean, timeoutMs = 10000, description = "predicate")` (`333-348`): same loop; error `... waiting for ${description}`.
- Note: the first check happens only after the first 50 ms sleep (sleep precedes screenshot). Comment at `session.ts:295-303` explains the 10000 ms budget (raised from 5000 for CI flakiness; must stay under vitest's 15000 ms testTimeout in `tests/screenshot.test.ts:26`). `docs/testing.md:144` still says "Default timeout: 5000ms" — docs are stale on this point.

Server-mode only (each throws `... is only available in server mode` otherwise):
- `async attach(): Promise<void>` (`353-367`): pushes a callback onto `screenCallbacks`, writes `encodeAttach(requestedRows, requestedCols)`, then awaits either the SCREEN packet being fully written into xterm or a 5000 ms fallback timer (`358-366`).
- `async reconnect(): Promise<void>` (`370-379`): `socket.destroy()`, sleep 100 ms, `terminal.reset()`, `connectSocket()`, `attach()` — simulates detach + reattach with screen replay.
- `resize(rows: number, cols: number): void` (`382-389`): records requested size, writes `encodeResize(rows, cols)`. Effective `rows/cols` update only when the daemon reports GEOMETRY (min-wins across writable clients; `docs/testing.md:339-342`).

Lifecycle:
- `async close(): Promise<void>` (`394-408`): spawn -> `ptyProcess.kill()` (errors swallowed) + `terminal.dispose()`; server -> `socket.destroy()`, `terminal.dispose()`, and `await server.close()` only if `ownsServer`. Docs say "Always call this in afterEach" (`session.ts:393`).

Private `connectSocket()` (`411-457`): `net.createConnection(getSocketPath(name))`, feeds bytes through `PacketReader` (protocol framing errors destroy the socket, `427-430`), and dispatches:
- `MessageType.GEOMETRY` -> `decodeGeometry`, sets `_rows/_cols`, `terminal.resize(cols, rows)` (`433-438`) — geometry is applied before subsequent screen/data bytes are parsed.
- `MessageType.SCREEN` -> `terminal.reset()` then `terminal.write(payload, cb)`; cb fires all pending `screenCallbacks` (`440-446`).
- `MessageType.DATA` -> `terminal.write(payload)` (`448-449`).
- `MessageType.EXIT` -> `exitCode = payload.readInt32BE(0)` (`451-452`).

#### `buildSpawnEnv(base, optsEnv?)` (`session.ts:47-60`)
Merges `base` + `optsEnv`; always deletes `PTY_SERVER_CONFIG` and `PTY_SESSION` (nesting guard / bogus config); deletes `PTY_ROOT` and `PTY_SESSION_DIR` only if the caller did not set them explicitly in `optsEnv` (`55-58`). Rationale in comment `session.ts:31-46`: the harness may itself run inside a pty session.

#### `captureScreenshot(terminal, serialize)` (`src/testing/screenshot.ts:5-24`)
Iterates `terminal.buffer.active` from line 0 to `buffer.length` (so scrollback lines are included, `docs/testing.md:137`), `line.translateToString(true)` (trim right), pops trailing blank lines, returns `{ lines, text: lines.join("\n"), ansi: serialize.serialize() }`.

### 1.2 Key names (`src/keys.ts`, shared with `pty send --seq key:` and the `./keys` export)

`resolveKey(spec: string): string` (`keys.ts:61-163`): case-insensitive; `KEY_MAP` (`keys.ts:1-18`): `return|enter`->`\r`, `tab`->`\t`, `escape|esc`->`\x1b`, `space`->` `, `backspace`->`\x7f`, `delete`->`\x1b[3~`, `up/down/right/left`->`\x1b[A/B/C/D`, `home`->`\x1b[H`, `end`->`\x1b[F`, `pageup`->`\x1b[5~`, `pagedown`->`\x1b[6~`. Modifiers `ctrl|alt|shift` (`keys.ts:20`), separators `+ - _` (`21`), leading `C-` means ctrl (`48-54`). Letters a-z: shift uppercases, ctrl -> code-96, alt prefixes ESC (`114-131`). Named keys with modifiers: `shift+tab` -> `\x1b[Z` (`141-143`); `CSI N ~` keys -> `\x1b[N;mod~`; `CSI X` keys -> `\x1b[1;modX` (`146-154`); return/tab/escape/space/backspace with modifiers -> kitty CSI-u `\x1b[code;modu` (`27-36`, `156-160`). Errors for unknown modifier/key, incomplete spec, and ambiguous specs (`73-111`). `parseSeqValue(value)` handles `key:` prefix (`166-171`). Docs table: `docs/testing.md:187-210`.

### 1.3 Attach to a real daemon vs spawn directly

- `Session.spawn` = direct `node-pty` child, no daemon, no socket. Used to drive arbitrary programs, including the `pty` CLI itself (tests/tui.test.ts spawns `node cli.ts` with `PTY_SESSION_DIR` set, `tests/tui.test.ts:56-67`).
- `Session.server` = constructs a `PtyServer` **in the test process** (`src/server.ts` import, `session.ts:7`) and talks to it over its Unix socket using the real wire protocol (`encodeAttach/encodeData/encodeResize`, `PacketReader`, `decodeGeometry`, `session.ts:8-15`). It does not spawn the `pty` binary. `getSocketPath(name)` (`src/sessions.ts`) resolves under `PTY_ROOT`/`PTY_SESSION_DIR`; tests set `process.env.PTY_SESSION_DIR` to a temp dir (`tests/screenshot.test.ts:31-32`).
- There is no factory for attaching to an already-running external daemon by name; `connectToExisting` requires a `Session` object created via `server()`. (The TUI's `attachPty(name)` in section 2 does attach by name.)

### 1.4 Expected usage patterns (Playwright-style)

From `docs/testing.md`:
- Quick start `docs/testing.md:23-30`: `Session.spawn("echo", ["hello world"])`, `await waitForText`, `expect(ss.text).toContain`, `await close()`.
- CLI tool (`214-223`), colored output via `ss.ansi` regex (`227-237`), full-screen TUI (vim: waitForText welcome, `sendKeys("i")`, waitForText("INSERT"), `press("escape")`, `sendKeys(":q!\n")`, `239-267`), interactive shell (waitFor regex prompt, send command, `press("ctrl+c")`, `269-297`).
- Server mode: `attach()` required (`303-311`), `reconnect()` shows screen replay (`313-329`), `resize()` (`331-342`), `connectToExisting` for two clients sharing output (`344-358`).
- Tips (`384-408`): raise timeouts for slow apps, `console.log(session.screenshot().text)` for debugging, temp-dir isolation, server mode for detach/reattach.
- Runner: `pty test` (thin vitest wrapper) or `npx vitest` (`367-382`). The `typescript test` fenced blocks in docs are executed by `scripts/verify-docs.ts` (`package.json:70`, `npm run verify-docs`).
- In-repo tests using it: `tests/screenshot.test.ts` (ls, ANSI 16/256/truecolor, OSC 8 hyperlinks, vim, nano, screen replay fidelity incl. cursor + scrollback, control chars, resize + `tput`, CJK/emoji/wide chars, alt-screen enter/exit + replay, multi-client, high-throughput, daemon spawning, immediate attach — `tests/screenshot.test.ts:88-1170`), `tests/tui.test.ts` (interactive manager, section 3), `tests/ratatui-compat.test.ts` (section 2.9), `tests/resize-tui.test.ts`, `tests/pty-handle.test.ts` (uses `createPty` from tui).

### 1.5 What depends on `@xterm/headless`

Everything that produces a `Screenshot`: the `Terminal` instance per Session (`session.ts:2-5, 108-114, 160-166, 201-207`), `terminal.write/reset/resize/dispose`, `buffer.active.getLine(i).translateToString(true)` (`screenshot.ts:10-14`), and `SerializeAddon.serialize()` for `ansi` (`screenshot.ts:23`). Behaviours implicitly guaranteed by xterm: alternate screen tracking, cursor state, 10000-line scrollback, wide-char/CJK/emoji cell handling, OSC 8 hyperlink underline styling, SGR fidelity in `ansi`. A Rust implementation needs an equivalent VT emulator (e.g. `vt100`/`alacritty_terminal`-class) with (a) plain-text row extraction with right-trim, (b) scrollback included in `lines`, (c) an ANSI re-serializer for `ansi`. Nothing in the testing library depends on the TUI framework.

---

## 2. TUI FRAMEWORK (`@compoundingtech/pty/tui`)

Source: `src/tui/*.ts` (24 files) + `src/tui/widgets/*.ts` (28 files). Public surface is `src/tui/index.ts:1-124` plus `export *` of `src/tui/widgets/index.ts:1-159`.

### 2.1 Architecture overview

Declarative, immediate-mode-ish: each frame the screen's `render(ctx)` returns a fresh `UINode[]` tree; a two-pass layout assigns `_rect`s; the tree is painted into a `CellBuffer`; the buffer is diffed against the previous frame and minimal ANSI is written to stdout. Re-render is driven reactively by preact signals (any signal read during render subscribes the frame effect).

**App lifecycle** — `app(config: AppConfig): App` (`src/tui/app.ts:53-306`).
- `AppConfig` (`app.ts:18-39`): `screen: Screen | (() => Screen)`, `overlay?: () => Screen | null`, `onKey?(key) => boolean` (global interceptor, runs before screen), `onMouse?(event) => boolean`, `theme?: () => Theme` (default `themes.coolBlue`, `74`), `boxStyle?: () => BoxStyle` (default `"rounded"`, `78`), `mouse?: boolean` (enables SGR mouse reporting on start), `focus?: FocusManager`.
- `App` (`app.ts:42-51`): `start()`, `stop()`, `pause()` (hand terminal to another process — used for attach), `resume()`.
- Terminal setup (`enterTerminal`, `247-252`): `\x1b[?1049h` alt screen + hide cursor, optional `MOUSE_ENABLE_SGR`, `stdin.setRawMode(true)`, `stdin.resume()`. Teardown (`leaveTerminal`, `254-263`): disable mouse, show cursor (+ SGR reset on full stop), leave alt screen, raw mode off, `stdin.pause()`.
- Render loop (`renderFrame`, `105-182`): `recordFrame()` for FPS; size from `stdout.rows/columns` (fallback 35x120, `70`); builds a `ScreenContext` (`85-103`) with `rows, cols, theme, boxStyle, navigate/back/openOverlay/closeOverlay` no-ops, `isTextInputActive/setTextInputActive` stubs, `quit()` (stop + `process.exit(0)`), and `focus`; detects screen transitions and calls `onLeave/onEnter` (`114-118`); `buf = scr.renderToBuffer(ctx)`; composites overlay by bounding box of non-blank cells (`123-153`); paints an FPS badge top-right when visible (`156-170`); emits `diff(prev, buf)` if same size else `fullRender(buf)` (`172-178`); writes `hideCursor() + output`.
- The frame is wrapped in a preact `effect(() => renderFrame())` (`271, 301`), so any signal `.get()` during render re-schedules a render. Resize handler nulls `prevBuffer` and re-renders (`225-229`). SIGINT/SIGTERM/exit stop the app (`231-236`).
- Input (`registerListeners`, `184-223`): `parseInput(buf)`; mouse events go `onMouse` -> `screen.handleMouse`; keys go `onKey` -> default **ctrl+c quits with exit 130** (`208-211`) -> `screen.handleKey`. Return values from `handleKey` are ignored (`213-220`).
- Screen abstraction (`src/tui/types.ts:80-90`): `Screen { id; render(ctx): string; renderToBuffer(ctx): CellBuffer; handleKey(key, ctx): boolean; handleMouse?(event, ctx): boolean; onEnter?; onLeave? }`. `ScreenContext` (`types.ts:55-77`) is extensible (`[key: string]: any`).

**Buffer / cell model** — `src/tui/types.ts`, `src/tui/buffer.ts`.
- `Cell` (`types.ts:7-24`): `char: string` (empty string = right half of a wide char), `fg/bg: [r,g,b] | null` (flattened RGB; null = terminal default), `fgIndex/bgIndex: number | null` (0-255 palette index when the source was indexed SGR; null for truecolor/default), `bold, dim, italic, underline`. `emptyCell()` (`26`), `cellsEqual()` (`33-45`, compares indices too).
- `class CellBuffer(rows, cols)` (`buffer.ts:6-216`): `cells: Cell[][]`, `clear()`, `getCell(row, col)`, `setCell(row, col, cell)` (bounds-checked), `writeAnsi(ansi: string)` (`45-204`) — an ANSI mini-parser: CSI with private prefixes `? > < = space` (`67-70`), SGR 0/1/2/3/4/22/23/24/27/39/49/38;2/48;2/38;5/48;5/30-37/40-47/90-97/100-107 (`74-117`), `CUP H` (`118-122`), `ED J` (only "" or "2" clears, `123-129`), `EL K` (cursor to EOL, `130-135`), OSC skipped to BEL/ST (`137-142`), charset designations skipped (`143-145`), `\n` / `\r` (`149-155`), printable chars with surrogate-pair combining and wide-char placeholder cell (`157-202`). `clone()` (`206-215`).
- `diff(prev, next): string` (`buffer.ts:314-388`): wrapped in DEC synchronized output `\x1b[?2026h ... \x1b[?2026l`; walks all cells, skips placeholder cells (`375`), skips `cellsEqual` cells, emits cursor moves only when not adjacent (`326-328`, tracking `lastCol += charWidth`), attribute resets on attr drop, and index-first SGR emission via `emitFg/emitBg` (`249-286`): palette cells re-emit as SGR 30-37/90-97/38;5;N so the outer terminal's theme wins; truecolor as 38;2; default as 39/49. Handles wide-char "fossil" cases (comment `293-313`; tests `tests/buffer-wide-char-diff.test.ts`).
- `fullRender(buf): string` (`buffer.ts:391-440`): `\x1b[?2026h\x1b[H\x1b[0m` + every row, same SGR state machine.
- 16-color and 256-color flatten tables (`buffer.ts:449-471`).

**Layout** — `src/tui/layout.ts` (two-pass: measure bottom-up, position top-down, `layout.ts:1`).
- `Rect { x, y, width, height }` 0-based (`nodes.ts:13-18`). Every node gets `_rect`.
- `measureHeight(node, maxWidth): number | "flex"` (`layout.ts:38-101`): text = 1 or wrapped line count; gap = size or "flex" for `"center"`; separator/dot/checkbox/progressBar/spinner/icon/row/statusBar/footer/textInput/fpsCounter = 1; askBar = 3; column/hstack/panel = sum/max of children or "flex" if any child is flex (panel adds 2 for borders); scrollable/selectable/ptyView = "flex"; canvas = `height ?? "flex"`.
- `measureWidth(node): number | "flex"` (`106-130`): text = `textWidth` unless `truncate`/`wrap` (then flex); spacer/separator/textInput/containers/ptyView = flex; indent = depth*2; icon = `charWidth`; fpsCounter = 8; progressBar = `width ?? flex`; canvas = `widthHint ?? flex`.
- `layoutRoot(nodes, viewport)` (`134-158`): `statusBar` nodes pinned to top rows, `footer` nodes to bottom rows (reverse order), the rest flow vertically in the middle.
- `layoutVertical(nodes, rect)` (`162-195`): fixed heights first, remaining split equally among flex nodes (remainder distributed 1 each), rects clipped to parent via `clipRect` (`12-23`).
- `layoutRow(children, rect)` (`199-232`): same for widths.
- `layoutHStack` (`260-297`): columns with fixed `width` or flex, `gap` between.
- `layoutPanel(node, rect)` (`299-344`): content inset 2 cols left/right, 1 row top/bottom; `separator` children span the full panel width to join the borders (`331-334`).
- `layoutScrollable` (`346-364`): one row per visible item starting at `offset`, each item laid out as a row.
- `textWidth(str)` (`27-33`) sums `charWidth`.

**Rendering** — two backends over the same tree:
- `renderToAnsi(nodes, theme, boxStyle, opts: RenderOpts, clip?)` (`src/tui/renderer.ts:61-73`) emits positioned ANSI strings per node (`renderNode` dispatch `77-120`). `RenderOpts { spinnerChar, fps, showFPS }` (`22-26`). Used by `Screen.render()`.
- `renderTreeToBuffer` / `renderNodeToBuffer` (`src/tui/screen.ts:465-741`) paints directly into a `CellBuffer`; this is the path `app()` uses (`renderToBuffer`). Notable: text `inverse` swaps fg/bg with fallbacks (`495-500`), text `background`, wrap + highlight spans (`509-523`, `writeSpannedBuf` `332-379`), truncation with ellipsis (`525-540`), panel bg fill + border + top title + optional `footerTitle` on the bottom border (`594-620`), statusBar (accent bg, `636-647`), footer (muted, left `hints` + optional `right`, `648-656`), askBar (3-row boxed input, `657-673`), textInput (block cursor, `674-683`), canvas (`690-704`), ptyView (`705-739`: resizes the handle to the rect, `handle.rev.get()` to subscribe, blits `readCells()` forwarding `fgIndex/bgIndex`).
- `resolveColor(color, theme)` (`renderer.ts:30-34`) delegates to `resolveSemantic`.
- `executeCanvasDraw(node, rect, theme)` (`renderer.ts:462-502`) builds the `DrawContext { width, height, set(), write(), fill() }`.

**Screen wrappers** — `src/tui/screen.ts`.
- `screen(config: DeclarativeScreenConfig): Screen` (`screen.ts:65-148`); config (`42-59`): `id`, `render(ctx) => UINode[]`, `handleKey?`, `handleMouse?`, `onEnter?`, `onLeave?`, `tick?: { ms, update }` (setInterval game loop while active, `129-142`). Runs `layoutRoot` with viewport = full terminal, manages the spinner timer refcount when the tree contains a spinner node (`21-40`, `80-84`), fills bg with `theme.bg1` (`116`).
- `overlay(config: OverlayConfig): Screen` (`screen.ts:168-275`); config (`152-166`): `id, title, width: number | (cols) => number, height: number | (rows) => number, render, handleKey?, handleMouse?, onEnter?, onLeave?`. Centers a panel, draws a shadow offset (1 down, 2 right, bg `[8,10,16]`, `224-225`), lays out content via `layoutPanel`.
- Overlay compositing is done by `app()` (bounding box, `app.ts:133-151`).

**Focus** — `src/tui/focus.ts` (stack-based router, rationale `focus.ts:1-48`).
- `FocusScope { id; active?: () => boolean; onKey?(key, ctx) => boolean; onMouse?(event, ctx) => boolean }` (`53-66`).
- `FocusManager { push(scope) => dispose; current(); stack(); dispatchKey(key, ctx); dispatchMouse(event, ctx) }` (`68-80`), `createFocusManager()` (`82-134`): dispatch walks innermost -> outermost over a snapshot, skips inactive scopes, first `true` consumes. Every `ScreenContext` carries `ctx.focus` (`types.ts:76`); screens opt in by calling `ctx.focus.dispatchKey` from `handleKey`. Tests: `tests/focus.test.ts`.

**Input parsing** — `src/tui/input.ts`.
- `KeyEvent { kind?: "key"; name: string; char?: string; ctrl; alt; shift }` (`input.ts:3-12`). `MouseButton = "left"|"middle"|"right"|"none"`, `MouseAction = "press"|"release"|"drag"|"move"|"scrollUp"|"scrollDown"` (`14-15`), `MouseEvent { kind: "mouse"; action; button; x; y (0-based); ctrl; alt; shift }` (`17-28`), `InputEvent = KeyEvent | MouseEvent` (`30`), `isMouseEvent()` (`47`).
- `MOUSE_ENABLE_SGR = "\x1b[?1002h\x1b[?1006h"`, `MOUSE_DISABLE_SGR` (`54-55`).
- `parseInput(data: Buffer): InputEvent[]` (`104-249`): SGR mouse `ESC[<b;x;y(M|m)` decoded via `decodeMouse` (`57-95`: low 2 bits button, 0x04 shift, 0x08 alt, 0x10 ctrl, 0x20 motion (drag/move), 0x40 wheel); plain arrows/home/end (`135-140`); modified arrows `ESC[1;modsX` (`146-158`); `ESC[Z` -> `backtab` (`161`); delete/pageup/pagedown `ESC[3~/5~/6~` (`164-168`); **kitty CSI-u** `ESC[code[;mods]u` with optional mods (`175-198`): shift+tab -> `backtab`, codepoints 27/13/9/127 -> named `escape/return/tab/backspace` (`39-44`), else `{ name: ch, char: ch }`; unknown CSI skipped to final byte (`201-204`); `ESC`+printable -> alt+char (`208-213`); bare ESC -> `escape` (`216-218`); `\r`->return, `\t`->tab, `\x7f`->backspace, `0x1c`->ctrl+`\` (`224-227`); `0x01-0x1a` -> ctrl+letter (`230-235`); printable (`238-242`). No bracketed-paste parsing (pty-layout strips/forwards markers itself). `parseKey(data): KeyEvent[]` filters mouse out (`99-101`). Tests: `tests/input-parse.test.ts`, `tests/mouse-parse.test.ts`.

**Hit testing** — `src/tui/hit-test.ts`: `HitResult { node; path: UINode[] }` (`19-24`); `hitTest(roots, x, y): HitResult | null` (`50-59`) descends into row/column/hstack/panel/scrollable/selectable children (`33-46`) returning the deepest node whose `_rect` contains the point; `findInPath(hit, type)` nearest ancestor of a node type (`74-82`). Tests: `tests/hit-test.test.ts`.

**Tokens / theme / palettes**.
- `Theme` (`src/tui/colors.ts:273-287`): 13 slots, each `[r,g,b] | null`: `bg1, bg2, bgHi, bgAc, fg1, fg2, fgAc, fgMu, ok, warn, err, info, border`. `BoxStyle = "rounded" | "sharp" | "double" | "heavy"` (`213`), glyph tables (`220-225`), `boxChars(style)` (`227`).
- `themes: Record<string, Theme>` (`colors.ts:289-357`): `coolBlue, warmAmber, mono, dracula, forest, coolBlueLight, warmAmberLight, monoLight, draculaLight, forestLight, terminal` (the last is all-null = use terminal defaults, `351-356`).
- Semantic colors `SemanticColor = "ok"|"muted"|"error"|"accent"|"primary"|"secondary"|"warn"|"info"|"border"`, `Color = SemanticColor | [r,g,b]` (`nodes.ts:6-10`). `SEMANTIC_SLOTS` map (`src/tui/tokens.ts:20-30`: primary->fg1, secondary->fg2, accent->fgAc, muted->fgMu, ok, warn, error->err, info, border), `resolveSemantic(color, theme): Rgb | null` (`39-44`), `semanticColorNames()` (`47`), `themeTokens(theme): Record<SemanticColor, Rgb|null>` (`55-61`, framework-neutral serializer "the foundation for the same palette on web", `index.ts:85-86`). Tests: `tests/tokens.test.ts`.
- `themeToXterm(theme)` (`builders.ts:35-61`) converts a Theme into an xterm ITheme (16 ANSI colors derived from theme slots + brightened variants) so programs inside embedded ptys look coherent.
- Low-level ANSI helpers exported from `colors.ts` (`index.ts:24-36`): `charWidth` (wcwidth-style table, `colors.ts:35-79`), `visibleLength`, `stripAnsi`, `truncate` (ellipsis), `wrapText(text, maxWidth) -> { lines, offsets }` (word-wrap, char-break fallback, CJK break-after, `121-183`), `pad`, `moveTo`, `fg`, `bg`, `reset`, `bold`, `dim`, `italic`, `underline`, `inverse`, `BOLD`, `DIM`, `RESET`, `clearScreen`, `hideCursor`, `showCursor`, `writeAt`, `fillRect`, `fillLine`, `drawBox(row, col, w, h, { style?, title?, fill? })` (1-based, `229-257`), `hSep`, `boxChars`, `progressBar` (as `progressBarString`), `themes`, `c(theme)` (pre-rendered SGR strings), `initScreen`, `titleBar`, `footerBar`, `panel` (as `drawPanel`), `panelLine`, `askBar` (as `drawAskBar`), `askBarCompact`, `agentActivity`.

**Signals** — `src/tui/signals.ts` wraps `@preact/signals-core`: `signal<T>(initial) -> { get(); set(v); peek() }` (`11-24`), `computed(fn) -> { get(); peek() }` (`26-40`), `effect`, `batch` re-exported (`42`), `debouncedSignal() -> { get(); peek(); bump(); flush() }` (one notification per `setImmediate` tick for firehose producers, `64-96`).

**Scroll region** — `src/tui/scrollable.ts`: `ScrollRegion { offset, selectedIndex, totalItems, viewportHeight }` (`3-8`), `createScrollRegion`, `updateScrollRegion(region, total, viewportHeight?)`, `scrollUp/scrollDown/pageUp/pageDown/scrollToTop/scrollToBottom` (pure, keep selection visible via `ensureVisible`, `48-58`), `visibleSlice(items, region)`.

**Text input (legacy ask bar)** — `src/tui/text-input.ts`: `TextInputState { text, cursor, active, processing }`, `createTextInput`, `activateTextInput`, `deactivateTextInput`, `handleTextInputKey(state, key, onSubmit?)` (escape clears, return submits when non-empty, backspace/delete/left/right/home/end/ctrl+a/e/u, printable insert; `23-88`), `finishProcessing`.

**Fuzzy** — `src/tui/fuzzy.ts`: `fuzzyMatch(query, target): { match, score }` (fzf-style in-order chars; bonuses for consecutive runs, word boundaries `- _ / space .`, prefix; shorter-target bonus; `19-67`). Not exported from `index.ts` (imported by relative path by interactive.ts and command-palette).

**Animation / FPS** — `src/tui/animation.ts`: braille spinner frames every 80 ms via a refcounted global timer (`spinnerChar` computed, `startSpinnerTimer/stopSpinnerTimer/isSpinnerRunning`). `src/tui/fps.ts`: `recordFrame`, `getCurrentFPS` (60-frame rolling window), `isFPSVisible`, `toggleFPS`.

**Legacy modules**: `src/tui/render.ts` (older ANSI primitives with a single rounded box style, `renderHeader/renderFooter/clearLine`, not exported) and `src/tui/screen-list.ts` (older imperative session list: `ListState`, `handleListKey`, `renderList`, `sortSessions`, `shortPath`, `timeAgo`; only `sortSessions/shortPath/timeAgo` are still used by `interactive.ts:23`).

### 2.2 Node types and builders (`src/tui/nodes.ts`, `src/tui/builders.ts`)

Union `UINode` (`nodes.ts:353-376`). Builders (`index.ts:69-77`):

| Builder (`builders.ts`) | Node | Notes |
|---|---|---|
| `text(str)`, `text(str, color, opts?)`, `text(str, { fg?, ...opts })` (`82-98`) | `TextNode` (`nodes.ts:36-55`) | opts: `bold, dim, italic, inverse, background, truncate, wrap, highlight(text) => Span[]`; `Span { start, end, color?, bold?, dim?, italic? }` code-point indices (`nodes.ts:23-30`) |
| `spacer()` (`100`) | `SpacerNode` | flex width filler in rows |
| `gap(size \| "center")` (`104`) | `GapNode` | vertical gap; `"center"` = flex |
| `separator()` (`108`) | `SeparatorNode` | horizontal rule joining panel borders |
| `indent(depth)` (`112`) | `IndentNode` | depth*2 columns |
| `dot(filled, color?)`, `checkbox(checked, color?)`, `progressBar(percent, { width?, color? })`, `spinner(color?)`, `icon(char, color?)` (`116-137`) | leaf nodes | 1-cell glyphs; progress bar `█`/`░` |
| `row(...children)` (`139`) | `RowNode` | horizontal, 1 row high |
| `column({ width?, flex? }, children)` (`143`) | `ColumnNode` | vertical flow |
| `hstack({ gap? }, columns)` (`150`) | `HStackNode` | side-by-side columns |
| `panel(title, children, style? \| { style?, footerTitle? })` (`157-166`) | `PanelNode` (`nodes.ts:136-143`) | bordered box, bg2 fill, title, optional bottom caption (`tests/panel-footer-title.test.ts`) |
| `scrollable(items, renderFn)` (`168`) | `ScrollableNode` | rows of nodes, offset |
| `selectable(region, items, renderFn(item, i, selected))` (`181`) | `SelectableNode` | driven by `ScrollRegion` |
| `groupedSelectable(region, groups: SelectableGroup<T>[], renderItem, renderHeader?)` (`207-245`) | `SelectableNode` | section headers + blank spacer rows; selectedIndex counts items only; visual offset auto-computed (`231-236`) |
| `statusBar(left, right)` (`247`) | `StatusBarNode` | pinned top |
| `footer(hints, right?)` (`251`) | `FooterNode` | pinned bottom |
| `askBar(state, { placeholder?, rightLabel?, style? })` (`255`) | `AskBarNode` | 3-row boxed prompt |
| `textInput(state, { placeholder? })` (`269`) | `TextInputNode` | 1-row input |
| `fpsCounter()` (`282`) | `FPSCounterNode` | |
| `canvas(draw, { height?, width? })` (`299-310`) | `CanvasNode` (`nodes.ts:231-240`) | free-form cell drawing via `DrawContext` |
| `ptyView(handle)` (`792-799`) | `PtyViewNode` (`nodes.ts:343-349`) | embedded terminal, flex |

### 2.3 Widgets (`src/tui/widgets/`, all "state-first: you own the state, widgets are pure render + pure key dispatch", `widgets/index.ts:1-3`)

| Widget | File | One-liner and notable features |
|---|---|---|
| tree | `tree.ts` | Keyboard-navigable expand/collapse tree. `TreeNode<T>{id,label,data,children?}`, `TreeState{expanded:Set,selectedId}`, `flattenTree(roots, expanded) -> TreeRow[]` (`41-53`), `toggleExpanded`, `selectById`, `moveSelection` (clamped; first arrow selects row 0, `68-83`), `handleTreeKey` (up/down/left/right/return -> `{ state, action: moved/expanded/collapsed/activated/none, row }`, `98-139`), `treeGlyph` (▸/▾). |
| date-picker | `date-picker.ts` | Calendar grid + time. `DatePickerState{year,month,day,hour,minute}`, `MONTH_NAMES`, `daysInMonth`, `datePickerFromDate`, `clampDay`, `shiftDay/shiftMonth/shiftTime`, `toDate`, `handleDatePickerKey` (arrows = ±1/±7 days, `[`/`]` month, h/H hour, m/M ±5 min; `84-101`), `calendarCanvas` (canvas node, 7 rows), `datePickerBody`, `datePickerPanel`. |
| form | `form.ts` | Single-line text field editing + multi-field focus ring. `TextFieldState{text,cursor}`, `applyTextKey(state, key)` (backspace, delete, left/right incl. alt-word motion, alt+b/f, home/end, ctrl+a/e, ctrl+u clear-to-start, ctrl+w delete-word, ctrl+k kill-to-end, printable insert; returns null if unhandled; `50-118`), `prevWordBoundary/nextWordBoundary`, `renderFieldText` (legacy block cursor), `renderFieldNodes(text, cursor, active, {color?,bold?})` -> 3 nodes with an `inverse` cursor cell (`136-155`), `FormState<Id>{values,focused,order}`, `createFormState`, `focusField`, `setFieldText`, `handleFormKey` (tab/backtab cycle, enter = activate/submit, escape = cancel; `225-256`). |
| markdown | `markdown.ts` | CommonMark subset -> nodes: headings 1-4, paragraphs (optional wrap width), bold/italic/inline code/links, fenced code, bullet/ordered lists, task lists, blockquotes, hr (`1-24`). `parseMarkdown`, `parseInline`, `renderMarkdown(source, { width? })`. |
| text-area | `text-area.ts` | Multi-line composer. `TextAreaState{lines,row,col}`, `createTextArea`, `textAreaToString`, `applyTextAreaKey` (newline, backspace/delete with line merge, arrows incl. alt-word, alt+b/f, home/end/ctrl+a/e, printable; tab/backtab/escape/ctrl+return return null for outer handling; `42-178`), `renderTextArea(state, active)` (inverse cursor). |
| virtual-list | `virtual-list.ts` | Windowed list for large datasets. `VirtualListState{total,selectedIndex,offset,viewport}`, `createVirtualListState`, `clampVirtual`, `virtualWindow`, `moveVirtualSelection`, `pageVirtual`, `jumpVirtualToStart/End`, `handleVirtualKey` (up/down/pageup/pagedown/home/end/return->activate), `handleVirtualMouse(state, event, rect)` (wheel ±3, left click selects+activates; `101-125`), `renderVirtualList(state, renderItem(index, selected))` (empty state "(empty)"). |
| stream-view | `stream-view.ts` | Chat/log tail pinned to bottom; scrolling up unpins. `StreamViewState{scrollback}`, `createStreamView`, `streamIsPinned`, `streamPin`, `streamScrollUp/Down`, `streamWindow`, `handleStreamKey` (up/down/pageup/pagedown/end pin/home top), `handleStreamMouse` (wheel), `renderStreamView` ("N more below" hint). |
| tabs | `tabs.ts` | Horizontal tab strip. `TabDef{id,label,data?}`, `TabsState{activeId}`, `createTabsState`, `selectTab`, `nextTab/prevTab`, `handleTabsKey` (ctrl+tab / ctrl+backtab cycle, 1-9 jump), `handleTabsMouse(state, tabs, event, rect)` (click hit-test by label width), `renderTabs` (`[ Active ]`). |
| confirm | `confirm.ts` | Yes/no modal body for `overlay()`. `ConfirmState`, `createConfirm({title,message,yesLabel?,noLabel?,defaultFocus?})` (default focus "no"), `handleConfirmKey` (left/right/tab/backtab toggle, return commit, escape=no, y/n), `confirmPanel`. |
| toast | `toast.ts` | Ephemeral notification queue. `Toast{id,kind,text,expiresAt}`, `ToastKind = info/success/warn/error`, `createToastQueue`, `pushToast(queue, msg, {kind?,durationMs?=3000,now?})`, `pruneExpired`, `dismissToast`, `renderToasts` (glyph + color per kind). |
| command-palette | `command-palette.ts` | Fuzzy action runner (ctrl+p style). `Command{id,label,hint?,keywords?,run()}`, `CommandPaletteState{query: TextFieldState, selectedIndex}`, `createCommandPaletteState`, `filterCommands` (fuzzy-ranked), `handleCommandPaletteKey` -> `{state, action: run/cancel/edited/moved/none, command?}`, `renderCommandPalette(state, commands, {title?, limit?=10})` (panel). |
| command-registry | `command-registry.ts` | Global signal-backed registry: `registerGlobalCommand(cmd) -> dispose`, `useCommandScope(scopeId, commands) -> dispose` (replaces batch per scope), `clearCommandScope`, `findCommand`, `runCommand(id)`, `allCommands` (computed), `_resetCommandRegistry`. |
| table | `table.ts` | Sortable table. `TableColumn<Row>{id,header,render,getSortValue?,align?,width?}`, `TableState{sortColumnId,sortDirection,selectedIndex}`, `createTableState`, `sortRows` (stable), `handleTableKey` (up/down/pageup(10)/pagedown/home/end/return activate, digits 1-9 toggle sort column/direction; `77-114`), `renderTable` (header with ▲/▼, rule, rows; auto column widths). |
| help-overlay | `help-overlay.ts` | Keybinding cheat sheet: `HelpSection{title,bindings:HelpBinding{key,desc}[]}`, `helpPanel(sections, title="keybindings")` with aligned key column and separators. |
| prompt-bar | `prompt-bar.ts` | Claude-Code-style full-width prompt: top/bottom rules, `❯` glyph, single-line (`TextFieldState`) or multi-line (`TextAreaState`) value, optional title on the rule (left/center/right) and status strip (left/right). `promptBar(value: PromptBarValue, opts)`. |
| toolbar | `toolbar.ts` | Hotkey legend/buttons row: `ToolbarItem{key,label,hint?,active?,disabled?}`, `toolbar(items, {separator?, format: "bracket"\|"inline", activeColor?})`, `toolbarItemFor(items, char)`. |
| sparkline | `sparkline.ts` | Unicode block sparkline: `sparklineString(series, {width?,min?,max?})` (tail-sampled, left-padded), `sparkline(series, {..., color?})` -> text node. |
| bar-chart | `bar-chart.ts` | Vertical histogram as a canvas: `barChart(items: {label?,value,color?}[], {height?=6,barWidth?=2,gap?=1,min?,max?,color?,showLabels?,labelColor?})`, 1/8-block vertical resolution. |
| pty-pane | `pty-pane.ts` | First-class live pty pane (section 2.5). |
| badge | `badge.ts` | Uppercase padded status chip: `badge(label, {variant: neutral/ok/warn/error/accent/info, solid?, uppercase?=true, bold?})` -> TextNode with `background`. |
| breadcrumbs | `breadcrumbs.ts` | `breadCrumbs(items, {separator?=" ❯ ", emphasizeLast?=true, chips?})` -> row of text nodes. |
| progress-bars | `progress-bars.ts` | SRCL-style bars as single text nodes: `barProgress(percent, {width?=20,color?,background?})` (`░` fill), `barLoader` (`█` fill); percent clamped 0-100 (`tests/progress-bars.test.ts`). |
| accordion | `accordion.ts` | Disclosure section: `accordion(title, expanded, children, {focused?, collapsedIcon?, expandedIcon?, indent?=2})` -> column. |
| action-list-item | `action-list-item.ts` | `[icon] label ... right` row with 3-cell icon chip highlighted when focused: `actionListItem(label, {icon?, focused?, right?})`. |
| code-block | `code-block.ts` | Numbered code/log block: `codeBlock(code, {startLine?, gutterColor?, showLineNumbers?, highlight?(line, i) => Span[]})`. |
| message | `message.ts` | Chat bubble: `message(content, {outgoing?, from?})` (incoming = border fill left-aligned; outgoing = accent fill right-aligned). |
| select | `select.ts` | Dropdown: `SelectState{open,index}`, `createSelectState`, `renderSelect(options, selectedIndex, state, {placeholder?, focused?, openCaret?, closedCaret?})`, `handleSelectKey(state, optionsLength, key)` -> `{state, selectedIndex?}` (Enter/Down opens, Up/Down move, Enter commits, Escape closes; `tests/select.test.ts`). |

Widget tests: `tests/widgets-*.test.ts`, `tests/accordion.test.ts`, `tests/action-list-item.test.ts`, `tests/badge.test.ts`, `tests/breadcrumbs.test.ts`, `tests/code-block.test.ts`, `tests/select.test.ts`, `tests/progress-bars.test.ts`, `tests/panel-footer-title.test.ts`.

### 2.4 PtyHandle — embedding a live pty (`src/tui/nodes.ts:271-341`, `src/tui/builders.ts:432-779`)

```ts
interface PtyCell { char; fg; bg; fgIndex; bgIndex; bold; dim; italic; underline }   // nodes.ts:249-268 (structurally a Cell)
interface PtyHandle {                                                                // nodes.ts:271-341
  write(data: string): void;                    // raw input to child / DATA packet
  resize(cols: number, rows: number): void;     // child winsize (createPty) or RESIZE request (attachPty; effective size arrives via GEOMETRY)
  readCells(scrollOffset?: number): PtyCell[][];  // typed cell grid, rows x cols; scrollOffset lines back into history (0 = live)
  readWrappedFlags(scrollOffset?: number): boolean[];  // per-row xterm isWrapped, aligned with readCells
  cols: number; rows: number;                   // current effective size
  kill(): void;                                 // createPty: kill child + dispose; attachPty: DETACH + destroy socket (daemon keeps running)
  readonly exited: boolean;
  dirty: boolean;                               // set on data/exit/resize/theme; consumer clears after reading
  onActivity: (() => void) | null;              // escape-hatch callback on data/exit/geometry/screen
  rev: Signal<number>;                          // reactive revision; ptyView() reads it during render
  setTheme(theme: Theme): void;                 // updates xterm theme (palette flattening)
  readonly cursorRow: number; readonly cursorCol: number;   // 0-based, viewport-relative
  readonly mouseMode: boolean;                  // child enabled ?1000/?1002/?1003
  readonly alternateScreen: boolean;            // ?1049 active
  readonly kittyKeyboardFlags: number[];        // stack of CSI > N u pushes (copy)
  readonly bracketedPasteMode: boolean;         // ?2004
  readonly scrollback: number;                  // configured lines
  readonly bufferLength: number;                // viewport + history lines
  readonly baseY: number;                       // index of top of live viewport
}
```

- `createPty(command, args = [], opts?: { cols?=80, rows?=24, scrollback?=0, cwd?, env?, theme? }): PtyHandle` (`builders.ts:432-584`). Lazily `require`s `node-pty` and `@xterm/headless` (`439-442`). Registers xterm CSI handlers to track mouse modes (`454-473`) and the kitty keyboard stack (`476-491`). Spawns with `TERM=xterm-256color` and node-pty `name: "xterm-256color"` (`493-505`). `proc.onData -> terminal.write(data, cb)` where cb sets `dirty`, bumps `rev`, fires `onActivity` (`515-521`); `onExit` sets `exited` (`522-527`). `resize` no-ops when unchanged or exited (`532-541`). `kill` kills the child and disposes the terminal (`551-557`).
- `attachPty(name, opts?: { cols?, rows?, scrollback?, theme? }): Promise<PtyHandle>` (`builders.ts:600-779`). Connects to `getSocketPath(name)` (`662-669`), same mode tracking, packet loop handles GEOMETRY (sets `handle.rows/cols`, `terminal.resize`, `740-748`), SCREEN (`terminal.reset()` + write, `750-756`), DATA (`757-763`), EXIT (`exited = true`, `764-770`); `write` sends `encodeData` (`682`); `resize` sends `encodeResize(rows, cols)` only when the requested size changed (`684-690`); `kill` sends `encodeDetach()` and destroys the socket (`700-706`). Sends `encodeAttach(rows, cols)` then waits a fixed 100 ms before returning (`775-776`).
- Cell reading (`readXtermCells`, `builders.ts:341-408`): `startLine = max(0, baseY - scrollOffset)`; per cell uses `isFgRGB/isFgPalette/getFgColor` etc. to fill RGB + index (`366-391`); `getChars() || " "`; attributes bold/dim/italic/underline. `readXtermWrappedFlags` (`415-430`) reads `line.isWrapped`.
- `ptyView(handle)` (`792-799`) renders it as a flex node; the buffer renderer resizes the pty to the rect and blits (`screen.ts:705-739`), the ANSI renderer likewise (`renderer.ts:505-538`).
- Tests: `tests/pty-handle.test.ts` (cursor, mouseMode 1000/1002/1003, alternateScreen, kitty stack push/pop/copy, readWrappedFlags, scrollback/bufferLength/baseY, readCells offsets and clamping, palette index preservation for SGR 34/94/38;5;N/48;5;N, truecolor -> null index). `tests/pty-root.test.ts` is about the `PTY_ROOT` registry root, not the TUI.

### 2.5 PtyPane widget (`src/tui/widgets/pty-pane.ts`)

"A first-class, reusable widget that renders a live pty session really well: a bordered/titled, focus-aware, scrollback-capable pane with selection highlighting and cursor-with-scroll reporting" (`pty-pane.ts:1-3`); generalizes the render path pty-layout grew (`5-10`; CHANGELOG `279-282`).
- `PtyPaneSelection { startRow, startCol, endRow, endCol, scrollOffset }` (`33-41`) — content-anchored selection.
- `PtyPaneOptions { theme; title?; focused?; chrome?=true; boxStyle?="rounded"; scrollOffset?=0; selection?; borderColor?; mutedBorderColor?; cache?=true }` (`43-67`).
- `PtyPaneResult { cursor: {row, col} | null (1-based, for moveTo); inner: Rect }` (`69-76`).
- `renderPtyPane(buf: CellBuffer, rect: Rect, handle: PtyHandle, opts): PtyPaneResult` (`154-255`): draws border via `drawBox` with focus/muted color (`169-182`), `handle.resize(inner)` (`188`), per-handle `WeakMap` cell cache reused when `!handle.dirty` and same size/scroll (`191-212`), blits cells preserving `fgIndex/bgIndex`, inverts fg/bg (and indices) for selected cells (`225-233`), reports cursor only when focused and on-screen (`243-251`).
- Helpers: `ptyPaneInnerRect(rect, chrome)` (`98-106`), `ptyPaneCursorRow` = `effectiveCursorRow(cursorRow, scrollOffset, innerHeight)` (`112-120`), `isSelectedInPane(row, col, sel, currentScrollOffset)` (`125-143`), `clearPtyPaneCache(handle?)` (`91-94`). Tests: `tests/pty-pane.test.ts`.

### 2.6 Also exported from `./tui`

Session helpers re-exported for TUI consumers (`index.ts:117-124`): `listSessions`, `getSession`, `SessionInfo`, `SessionMetadata` (from `src/sessions.ts`), `spawnDaemon`, `SpawnDaemonOptions` (from `src/spawn.ts`). pty-layout's `pane.ts` relies on these (`<pty-layout-checkout>/src/pane.ts:1`).

### 2.7 In-repo consumers other than the session manager

`demos/playground` (widget catalog with sidebar tree, focus scopes, mouse routing, `demos/playground/main.ts:1-40`), `demos/reminders`, `demos/file-browser`, `demos/agent-teams` (`demos/` listing). These are not shipped in `files` (`package.json:23-31`) but exercise the same API.

### 2.8 Framework-level tests

`tests/tui-framework.test.ts` (builders, textWidth, layout incl. statusBar/footer pinning, gap center, spacer distribution, hstack fixed+flex, panel auto-height, clipping, scrollable viewport, canvas sizing, renderer positions/panel/leaf glyphs, `screen()`/`overlay()` render and buffer, panel background preservation, canvas draw API, CellBuffer private CSI prefixes, wrapText, highlight spans, `app()` shape, fuzzyMatch), `tests/buffer-palette.test.ts` (index round-trips through `writeAnsi`/`fullRender`/`diff`), `tests/buffer-wide-char-diff.test.ts`, `tests/focus.test.ts`, `tests/hit-test.test.ts`, `tests/tokens.test.ts`, `tests/input-parse.test.ts`, `tests/mouse-parse.test.ts`.

### 2.9 ratatui-compat (`tests/ratatui-compat.test.ts`)

Not a library module — a 1215-line integration suite named for the Rust `ratatui` TUI crate. It drives `Session.server(...)` + `attach()` (`tests/ratatui-compat.test.ts:43-58`) and inspects raw SCREEN packets (`getRawScreenPayload`, `101-133`) to prove that ratatui/codex-style full-screen apps survive the daemon's serialize/replay path. Sections: (1) "ECH/CUF round-trip with background colors" — full-width RGB background fills survive serialize/replay (`137-257`); (2) "full-screen ratatui-style rendering" — alt screen + per-row EL background erase survives replay (`259-449`); (3) "kitty keyboard protocol stack" — pushed flags are replayed in the server's mode prefix on reattach, push/pop nesting (`451-699`); (4) "resize timing with full-screen redraw" — a SIGWINCH-redrawing app with 0..N ms delay shows the correct size after resize + reconnect (`701-894`); (5) "mixed content layout (codex-style UI)" — box-drawing with styled content survives reconnect (`896-1215`). Why it exists: consumers run Rust ratatui apps (codex) inside pty sessions, and screen replay must reproduce their output faithfully; it is a contract the Rust daemon must also honour, and the Node testing library is the harness that proves it.

---

## 3. INTERACTIVE SESSION MANAGER (`pty` with no args)

Entry: `src/cli.ts:84-93` `runInteractive(options)` — nesting guard `ensureNotNested("interactive", { force, hint })` (`cli.ts:626-637`; refuses when `PTY_SESSION` is set unless `--force`, printing "Detach first (Ctrl+\) and run `pty` from outside, or pass --force"), then lazy `import("./tui/interactive.ts")` and `mod.runInteractive(options)`. Dispatch (`cli.ts:719-765`): if the subcommand is empty, `i`, or `interactive`, `--preselect-new`, `--force`, and repeatable `--filter-tag k=v` are consumed (`738-742`, `extractFilterTags` `96-103`). Usage text (`cli.ts:481-485`) and README (`README.md:24-50`).

Implementation: `src/tui/interactive.ts` (767 lines) built on `app()`, `screen()`, signals, `panel/footer/row/text`, `groupedSelectable`, `updateScrollRegion`, `themes`, `applyTextKey`, `renderFieldNodes` (`interactive.ts:14-22`).

### 3.1 State

Signals: `sessions: SessionInfo[]` (`53`), `filterField: TextFieldState` (readline-style text + cursor, `56`), `selectedIndex` (`57`), `filterTags: Record<string,string>` (`61`), `themeIndex` (`82`, persisted to `<sessionDir>/theme`, `67-80`), `relayHosts: RelayHost[]` (`119`). Computed: `sortedSessions` (running first then name, then tag-filtered, `151-156`), `filteredGroups` (`246-262`), `totalItems` (`265-267`).

### 3.2 List rendering (`listScreen`, `interactive.ts:344-490`)

- Layout: one `panel("pty", [...])` filling the screen, containing the filter line, a blank line, and a `groupedSelectable` list; plus a `footer` (`385-394`). Viewport height for the list = `rows - 6` (`350`). Section headers ("Local", each relay host label with item count) are rendered only when relay hosts exist (`383-391`); otherwise the header renderer returns `[]` so the local list is flat.
- Filter line (`365-381`): `"  Filter: "` + `renderFieldNodes(text, cursor, true)` (inverse-cell cursor) + optional tag filter `#k=v ...`; when empty shows dim `"(type to filter)"`.
- Item rows (`renderListItem`, `282-342`):
  - selection marker `▸ ` vs two spaces (`283`);
  - live marker `●` for `status === "running"`, `○` otherwise (`312`, remote `294`);
  - label: `displayName (name)` when a displayName exists else `name` (`328-329`); ` [permanent]` when `tags.strategy === "permanent"` (`322-323`); inline user tags ` #key=value` excluding reserved keys / `:`-prefixed tool tags (`renderTagsInline`, `276-280`, `isReservedTagKey`);
  - detail: `  <cwd shortened with ~>  <displayCommand>`; exited sessions show `(exited Xs/m/h/d ago)` from `exitedAt` (`317-320`, `timeAgo`);
  - selected row = one accent bold truncated text node; unselected = bold name (primary if running, muted if not) + dim muted detail (`335-341`). There are no fixed columns; it's name-then-detail with truncation (tests: box fills width at 80/120/200 cols, path not truncated at 120, works at 60, `tests/tui.test.ts:132-230`).
  - `+ Create new session...` item (accent when selected, muted otherwise) for Local and for each `spawn_enabled` relay host (`284-290`).
- Footer: `↑↓ select  ⏎ attach  ctrl+g theme (<name>)  q quit` (`393`).
- Remote grouping (`buildFilteredGroups`, `198-244`): groups = `Local` + one per relay host (`host.error` hosts skipped). Filter syntax `host/session` splits on the first `/`: host part fuzzy-matches host labels (and the literal "local"), session part filters sessions (`203-210`, `216-221`, `226-229`). Create items are hidden unless the filter is empty or a prefix of "new" (`212`). Tests: `tests/filter.test.ts`.

### 3.3 Filter (`filterAndSort`, `158-194`)

Fuzzy (`fuzzyMatch`) against `name`, `displayName`, `cwd`, `displayCommand` (remote: name/command/cwd); running sessions get +100000, name/displayName matches +10000 over cwd/cmd matches; sorted by score desc. Editing is delegated to `applyTextKey` (`477-488`): backspace, delete, arrows, alt+b/f, home/end, ctrl+a/e, ctrl+u, ctrl+w, ctrl+k, printable. Any text change resets `selectedIndex` to 0; cursor-only motion keeps it (`482-485`).

### 3.4 Keyboard map (`handleKey`, `397-490`, plus `app()` config `750-757`)

| Key | Action |
|---|---|
| `up` / `down` | move selection, clamped (`414-421`) |
| `return` | on Local session: attach if running, else restart (`doRestart`) then attach (`443-452`); on `create`: one-keystroke local create (`425-431`); on `remote-create`: `pty-relay connect <url> --spawn <id> [--tag k=v]` (`432-438`); on `remote`: `pty-relay connect ...` attach (`439-442`) |
| `escape` | clear filter if non-empty, else quit (`454-461`) |
| `q` (no ctrl/alt, filter empty) | quit (`462-465`) |
| `ctrl+c` | quit (global default in `app()`, `app.ts:208-211`) |
| `ctrl+g` | cycle theme through `themes` (persisted) (`754`) |
| everything else | text-field editing via `applyTextKey`; unhandled keys swallowed (`477-489`) |
| kitty CSI-u encodings | handled by `parseInput` (test "kitty keyboard protocol: CSI-u escape clears filter, then quits", `tests/tui.test.ts:707`) |

Mouse is not enabled (`app({ screen, theme, onKey })` only, `750-757`).

### 3.5 Attach-and-return-to-list

`doAttach(name)` (`591-620`): `pauseApp()` (stops the 1 s poll and `app.pause()` leaves alt screen/raw mode, `713-716`), then `attach({ name, onDetach, onExit })` from `src/client.ts:444` runs the real attach client in the same process. `Ctrl+\` (`0x1c`, or kitty `\x1b[92;5u`, `client.ts:20-21`) detaches (double-tap within 300 ms forwards a literal Ctrl+\ to the child, `client.ts:540-566`). `onDetach`: re-list, clamp selection, `resumeApp()` (`596-608`) — filter and selection are preserved (#27). `onExit`: wait 200 ms for exit metadata, re-list, resume (`609-618`). Tests: "Ctrl+\\ detaches and returns to session list", "multiple attach/detach cycles work without breaking input", "keystrokes are not doubled" (`tests/tui.test.ts:581, 747, 956`).

### 3.6 Create-new-session flow

There is **no directory picker or name/command prompt in the current code**. CHANGELOG `541`: "Create new session..." is a one-keystroke action: Enter spawns `$SHELL` (fallback `bash`) in `$HOME` (`defaultCwd/defaultShell`, `interactive.ts:40-47`) with a random 8-char base32 id (`randomSessionName`, `29-35`) and no `displayName`; users `pty rename` / `pty exec` from inside to promote it. `doCreate(dir, name, shell)` (`625-658`): `pauseApp()`, `spawnDaemonWithCreationLock({ name, command: shell, args: [], displayCommand: shell, cwd, tags? })` (`631-637`), on lock conflict or error prints to stderr, re-lists, resumes; on success `doAttach(name)`. Restarting an exited/vanished session (`doRestart`, `498-521`) reuses stored `command/args/displayCommand/cwd/tags/displayName` after `cleanupAll(name)`.

### 3.7 `--preselect-new`

`runInteractive({ preselectNew })` (`725-747`): after the initial `listSessions()`, walks `filteredGroups` and sets `selectedIndex` to the global index of the first `create` item (`733-745`). Test `tests/tui.test.ts:273`. Documented as useful for pty-layout panes that should land on the create prompt (CHANGELOG `542`).

### 3.8 `--filter-tag` inheritance

`filterTags` signal set from options (`726-728`). Effects: local list filtered by `matchesAllTags(session.metadata.tags, required)` (`151-156`); remote sessions filtered by their relay-reported `tags` (`249-256`); filter line shows `#k=v` (`358-360`); new local sessions get `tags` (`doCreate`, `629-637`); remote spawns forward `--tag k=v` (`buildSpawnRemoteArgs`, `560-566`). Tests `tests/tui.test.ts:335, 383`.

### 3.9 Remote sessions (pty-relay)

`relayBin = which pty-relay` at module load (`114-117`). `refreshRelayHosts()` runs `pty-relay ls --json` asynchronously (10 s timeout) and sets `relayHosts` (`123-138`); called at startup and after each remote attach/spawn returns. `RelayHost { label, url, sessions: RemoteSession[], spawn_enabled, error }`, `RemoteSession { name, status, command?, cwd?, tags? }` (`96-112`). Attach argv (`buildAttachRemoteArgs`, `527-534`): `ssh://` peers -> `connect <label> --session <name>`; token URLs -> session name inserted before any `#fragment`. Both run `pty-relay` via `spawnSync(..., { stdio: "inherit" })` with the app paused (`536-556`, `568-589`).

### 3.10 Other behaviours

- Auto-refresh: 1 s `setInterval` polling `listSessions()` while the list is visible (`685-701`, unref'd), paused during attach (`713-722`); fs.watch deliberately avoided (`672-684`). Tests `tests/tui.test.ts:786-834` (created/exited/tags changed externally).
- Theme persistence file `<sessionDir>/theme` (`67-80`); default theme is `terminal` (all-null colors) when no file (`73`).
- Status semantics: `running`, `exited`, `vanished` (killed daemon without exit metadata; both non-running kinds are restartable, `443-448`).
- Exit: `ctx.quit()` -> `app.stop()` + `process.exit(0)` (`app.ts:97-100`).
- Empty state shows only "Create new session..." (`tests/tui.test.ts:647`).

---

## 4. PTY-LAYOUT (`<pty-layout-checkout>`, checkout exists)

`@myobie/pty-layout` 0.1.0, "Terminal multiplexer for @myobie/pty sessions. Tag-driven panes, tmux compatibility shim, agent-team aware" (`package.json:2-4`), depends on `@myobie/pty ^0.10.0` (`package.json:36`; an older, pre-rename version of the same package — so it predates `renderPtyPane` and re-implements that path itself). Entry `src/main.ts` (1319 lines); `README.md` describes tag model, keybindings (`^]` prefix, `^\` detach, 1-9 focus, `,`/`.` prev/next, `l` layout cycle grid->stacked->single(->zoom), `m` move, `n` session picker, `w` close, `q` quit; mouse drag selection copied via OSC 52 with wrapped-line reconstruction; shift-click extends selection), `--tmux` shim env, `pty layout new`.

What it imports from the TUI library:
- `src/main.ts:2,22`: `hideCursor, showCursor, reset, CellBuffer, fullRender, moveTo, fg, bg, visibleLength, RESET, spawnDaemon`.
- `src/render.ts:1-13`: `CellBuffer, diff, fullRender, drawBox, fg, bg, reset, moveTo, showCursor, RESET, visibleLength`.
- `src/pane.ts:1`: `createPty, attachPty, spawnDaemon, getSession, type PtyHandle`.
- `src/session-picker.ts:1`: `listSessions`. `src/tag-subscription.ts:1`: `EventFollower, listSessions` from `@myobie/pty/client`; `src/main.ts:23`: `updateTags` from `/client`.

What it needs, concretely:
1. A `CellBuffer` with `writeAnsi()` (it composes chrome as ANSI strings via `drawBox/moveTo/fg/bg` and writes them into the buffer, `render.ts:99-105, 121-127, 355-360`), `setCell()` for blitting pty cells including palette indices (`render.ts:163-175`), and `diff(prev, next)` / `fullRender(buf)` (`render.ts:207-211`); frames are then written to stdout with a trailing `moveTo + showCursor` for the focused pane's cursor (`render.ts:214-219`).
2. `PtyHandle` from both `createPty(cmd, args, { cols, rows, scrollback: 10000, env })` (local panes, `pane.ts:71-76`) and `attachPty(name, { cols, rows, scrollback: 10000 })` (session panes, `pane.ts:29, 47`), using: `resize` (`render.ts:132`), `dirty` read/write for its own cell cache (`render.ts:137, 195`), `readCells(scrollOffset)` (`render.ts:144`, `main.ts:908`), `readWrappedFlags(scrollOffset)` (`main.ts:909-910`), `cursorRow/cursorCol` (`render.ts:187-190`), `baseY` for scroll anchoring (`main.ts:201, 258, 678, 963, 968`), `onActivity` to schedule renders and detect exit (`main.ts:247-253`), `exited`, `kittyKeyboardFlags` proxied to the outer terminal (`main.ts:159-180`, `syncModesToTerminal`), `bracketedPasteMode` + kitty flags to translate input (`main.ts:665-668`), `mouseMode` to forward synthesized SGR mouse sequences to the child (`main.ts:794-802, 847-855, 880-888, 941-951`), `alternateScreen` to turn wheel into arrow keys (`main.ts:954-958`), `write`, `kill` (detach semantics for attached sessions, `main.ts:394-397`), `cols/rows` (`main.ts:1304`).
3. Session/daemon helpers: `spawnDaemon({ name, command, args, displayCommand, tags, env })` (`pane.ts:28`), `getSession(name)` for displayName (`pane.ts:50-52`), `listSessions()` for the picker, `updateTags(name, set, remove)` and `EventFollower` (tag subscription drives pane add/remove; `README.md` "How it works").
4. Its own: layout math (`layout.ts`), prefix-key state machine and CSI-u->legacy translation (`keys.ts`), selection model (`selection.ts`), fuzzy picker (`session-picker.ts`, mirrors the manager's grouping/filter/relay logic), render scheduling (immediate for focused echo, 8 ms otherwise, `main.ts:120-151`), resize debounce (`main.ts:1185-1192`), raw terminal setup (alt screen, `?1002h ?1006h` mouse, `?2004h` bracketed paste, `main.ts:54-65`).

---

## 5. FEATURE LIST for a replacement Rust TUI library

Capabilities the session manager (section 3) and a pty-layout-like multiplexer (section 4) need. Widget internals omitted; these are capabilities.

**Terminal session and render loop**
- [ ] Enter/leave alternate screen, raw mode, hide/show cursor, SGR reset; clean teardown on SIGINT/SIGTERM/exit (`app.ts:247-263, 231-236`).
- [ ] `pause()` / `resume()` that fully releases the terminal and stdin to another in-process client (the attach client) and re-enters with a forced full redraw (`app.ts:288-302`; `interactive.ts:591-620`).
- [ ] Optional SGR mouse reporting (`?1002h ?1006h`) and bracketed paste (`?2004h`) enable/disable (`input.ts:54-55`; pty-layout `main.ts:56-65`).
- [ ] Resize handling (SIGWINCH) with full redraw, and the ability to debounce (`app.ts:225-229`; pty-layout `main.ts:1185-1192`).
- [ ] Reactive re-render: state changes (signals or equivalent) schedule a frame; consumers can also request urgent vs. coalesced renders (`app.ts:271`; pty-layout `main.ts:127-151`).
- [ ] Global key interceptor before screen dispatch; default ctrl+c -> exit 130 that apps can override (`app.ts:200-211`).
- [ ] Explicit `quit()` hook exposed to screens (`types.ts:66-70`).

**Cell buffer and output**
- [ ] Cell grid with char (wide-char placeholder convention), fg/bg RGB or default, **palette index preservation** (0-255) for indexed SGR, bold/dim/italic/underline (`types.ts:7-24`).
- [ ] ANSI-string ingestion into the buffer (`writeAnsi`: CUP, ED, EL, SGR incl. 256/truecolor, private-prefix CSI tolerance, OSC skipping, surrogate pairs/wide chars) so hosts can compose chrome as strings (`buffer.ts:45-204`; pty-layout relies on it heavily).
- [ ] Direct `setCell/getCell` for blitting external cell grids (`buffer.ts:37-43`).
- [ ] Frame diffing with minimal cursor moves and index-first SGR emission (indexed colors round-trip so the outer terminal's theme applies), wrapped in DEC synchronized output 2026; full-render fallback on size change; correct wide-char fossil handling (`buffer.ts:314-440`).
- [ ] Append a final cursor placement + show-cursor after the diff for the focused pty pane (`render.ts:214-219` in pty-layout; `pty-pane.ts:243-251`).

**Layout and drawing**
- [ ] Declarative node tree with two-pass flex layout: vertical flow, rows with flex spacers, side-by-side columns with fixed/flex widths and gaps, panels with borders/insets, top-pinned status bar and bottom-pinned footer, clipping (`layout.ts`).
- [ ] Panel chrome: 4 box styles, title on top border, optional caption on bottom border, separators that join the borders, background fill (`colors.ts:213-262`; `screen.ts:594-620`).
- [ ] Text: semantic or RGB color, background, bold/dim/italic/inverse, truncation with ellipsis, soft wrap with code-point offsets, per-span highlight callback (`nodes.ts:36-55`; `colors.ts:121-183`).
- [ ] wcwidth-style character width for CJK/emoji/box drawing (`colors.ts:35-79`) and width-aware truncate/pad.
- [ ] Free-form canvas drawing region with set/write/fill (`renderer.ts:462-502`).
- [ ] Centered overlay/modal with shadow composited over the base screen by bounding box (`screen.ts:168-275`; `app.ts:123-153`); pty-layout needs centered overlays for the prefix help and session picker (`render.ts:330-546`).
- [ ] Scrollable and selectable lists with a scroll-region model (offset/selected/viewport), grouped lists with section headers whose selection index counts items only (`builders.ts:181-245`; `scrollable.ts`).
- [ ] Spinner/animation timer and optional per-screen tick loop (`animation.ts`; `screen.ts:42-59`).

**Theme and tokens**
- [ ] Theme struct with the 13 slots (`colors.ts:273-287`), a built-in set including light variants and an all-default "terminal" theme, runtime theme cycling (`interactive.ts:82-90`).
- [ ] Nine semantic color tokens resolved through a single slot map; serializable name->RGB (`tokens.ts`).
- [ ] Theme -> 16-color palette mapping for embedded terminals so child programs match the UI (`builders.ts:35-61`).

**Input**
- [ ] Key parsing: printable, ctrl+letter, alt+char, named keys (arrows/home/end/pageup/pagedown/delete/tab/backtab/return/escape/backspace), modified arrows `CSI 1;mods X`, **kitty CSI-u** with optional modifier param and named decoding of 27/13/9/127 and shift+tab (`input.ts:104-249`).
- [ ] SGR mouse decoding: press/release/drag/move/wheel, buttons, modifiers, 0-based coords (`input.ts:57-95`).
- [ ] Hit-testing of the laid-out tree for mouse routing (`hit-test.ts`).
- [ ] Stack-based focus scopes with conditional `active()` predicates, innermost-first bubbling for keys and mouse (`focus.ts`).
- [ ] Readline-style single-line editing primitive (cursor motion, word motion, ctrl+a/e/u/w/k, inverse-cell cursor rendering) usable for the filter field (`form.ts:50-155`).
- [ ] Fuzzy matcher with boundary/consecutive/prefix scoring (`fuzzy.ts`) for filter and pickers.
- [ ] Pass-through / translation hooks for bracketed paste markers and CSI-u sequences when forwarding to a child that lacks the mode (pty-layout `keys.ts:27-42`).

**Embedded live pty (PtyHandle)**
- [ ] Spawn a child in a local pty with cols/rows/cwd/env/scrollback and a VT emulator behind it (`builders.ts:432-584`).
- [ ] Attach to a named daemon session over the wire protocol (ATTACH/DATA/RESIZE/DETACH out; GEOMETRY/SCREEN/DATA/EXIT in), with SCREEN replay = reset + write, geometry applied before dependent bytes, detach-on-kill semantics (`builders.ts:600-779`).
- [ ] Input `write`, `resize` (effective size may differ from requested for shared sessions), `kill`, `exited` (`nodes.ts:273-296`).
- [ ] Typed cell-grid read with scrollback offset and per-row wrapped flags (`nodes.ts:288-309`; `builders.ts:341-430`).
- [ ] Cursor row/col, mouse-tracking mode, alternate-screen flag, kitty keyboard flag stack, bracketed-paste flag, scrollback size, buffer length, baseY (`nodes.ts:312-340`).
- [ ] Activity signalling: dirty flag, revision signal/subscription, activity callback fired on data/exit/geometry/screen/theme (`nodes.ts:297-311`; `builders.ts:515-527, 740-770`).
- [ ] Runtime theme update for the embedded emulator (`nodes.ts:312`).
- [ ] A pane renderer that draws focus-colored border+title, resizes the child to the inner rect, caches cells until dirty, blits preserving palette indices, highlights a content-anchored selection (scroll-translated), and reports the on-screen cursor or none (`pty-pane.ts:154-255`); plus a simpler flex `ptyView` node that participates in layout (`screen.ts:705-739`).
- [ ] Support for a host to proxy the focused pane's kitty flags to the outer terminal and to synthesize SGR mouse sequences for children in mouse mode (pty-layout `main.ts:159-180, 794-802`).

**Session-manager-specific integrations (library or host)**
- [ ] Access to `listSessions/getSession/spawnDaemon` (+ creation lock), `cleanupAll`, `updateTags`, tag matching (`matchesAllTags`) and reserved-tag filtering (`isReservedTagKey`), event following (`EventFollower`) from the Rust daemon crate (`index.ts:117-124`; `interactive.ts:8-13`; pty-layout `tag-subscription.ts:1`).
- [ ] In-process attach client with detach (Ctrl+\ incl. kitty encoding, double-tap forwarding), `onDetach`/`onExit` callbacks, and reconnect loop (`client.ts:20-23, 444-566`).
- [ ] Ability to shell out to `pty-relay ls --json` / `pty-relay connect ...` with inherited stdio while the TUI is paused (`interactive.ts:123-138, 536-589`).
- [ ] Periodic background refresh (polling) that pauses while the app is paused (`interactive.ts:685-722`).
- [ ] Persisted small preferences (theme name) under the session dir (`interactive.ts:67-80`).

**Testing hooks**
- [ ] The whole TUI must be drivable by the testing library: spawned as a child in a pty, rendered into a VT emulator, asserted by text (`tests/tui.test.ts:56-67`), including kitty CSI-u input and multi-attach/detach cycles.
