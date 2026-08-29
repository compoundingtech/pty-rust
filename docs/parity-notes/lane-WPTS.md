# Lane WP-TS — the TypeScript testing package (packages/pty-testing)

Read lane-common.md, plan-verify-libs.md "B3", node-testing-tui.md section 1 (the Node API to mirror), and
<node-pty-checkout>/docs/testing.md (the docs to mirror).
Worktree: `wpts`. Branch: `wpts` (off `parity` after WP7b is merged — it needs `run -e --env --rows --cols`
and daemon GEOMETRY).

You own: `packages/pty-testing/**`, root `package.json` (`{"private":true,"workspaces":["packages/*"]}`),
`.gitignore` additions (node_modules, dist). Node 24 is installed (`node --version`); use `npm`.

1. Package `@compoundingtech/pty-testing`: ESM, TypeScript, `tsc` → `dist/`, no runtime dependencies, `vitest`
   as a peer/dev dependency. Engine: `PTY_BIN` env else `pty` on PATH; on first use run `<bin> --version` and
   refuse (clear error) unless it contains `-rust` or `PTY_TESTING_ALLOW_NODE=1`.
2. `src/protocol.ts`: the 5-byte frame (`[type u8][len u32 BE][payload]`), `MessageType` 0-7 and 10, encoders
   (data, attach(rows, cols), detach, resize, peek(plain, full), status), decoders (size, exit, geometry), and a
   `PacketReader` with the 32 MiB cap. Independent reimplementation; do not import the Node pty package.
3. `src/session.ts`, API mirroring Node's (node-testing-tui.md 1.1) with the engine underneath:
   `Session.spawn(command, args = [], {rows=24, cols=80, cwd, env, name})` → creates a temp `PTY_ROOT` (`/tmp/pt-XXXX`),
   `pty run -d -e --no-display-name --id <8 chars> --cwd <cwd> [--env K=V]* --rows R --cols C -- <command> <args>`
   with `PTY_ROOT` set and `PTY_SESSION*`/`PTY_SESSION_DIR`/`PTY_REAP_ON_EXIT`/`NO_COLOR` scrubbed, waits for
   `<id>.sock`, opens ONE attached socket (ATTACH rows×cols; consumes GEOMETRY → rows/cols, SCREEN/DATA drained,
   EXIT → exitCode) and uses short-lived command sockets for PEEK; `Session.connect(name, {rows, cols, root})` for an
   existing session; `Session.connectToExisting(s, {rows, cols})`; getters `name`, `root`, `rows`, `cols`,
   `hasExited`, `exitCode`; `sendKeys(s)`, `type(s)`, `press(key)` (key table ported from keys.ts incl. the
   `+ - _ C-` grammar and error texts); `async screenshot()` → `{lines, text, ansi}` (PEEK plain → split on `\n`,
   pop trailing empties; PEEK ansi → `ansi`); `waitForText/waitForAbsent/waitFor` (50 ms poll, 10 000 ms default,
   Node's exact error messages incl. the screen dump); `resize(rows, cols)` (RESIZE; rows/cols update on GEOMETRY);
   `async reconnect()`; `async attach()`; `async close()` (`pty kill` then `pty rm` when owned, then rm -rf the
   temp root).
4. `vitest`: `packages/pty-testing/vitest.config.ts`, `setup/isolate.ts` (scrubs `PTY_*`), `setup/global.ts` (kills
   leaked daemons under `/tmp/pt-*` at teardown by reading `*.pid`).
5. `docs/testing.md` inside the package, mirroring the Node doc's structure (quick start, CLI tool, colored output,
   full-screen TUI with vim, interactive shell, server-mode equivalents: reconnect, resize, two clients, tips),
   with ```ts test``` fences executed by `scripts/verify-docs.ts` (`npm run verify-docs`).
6. Tests: port tests/screenshot.test.ts (ls, ANSI 16/256/truecolor, OSC 8, vim, nano, screen replay with cursor and
   scrollback, control chars, resize + tput, CJK/emoji/wide chars, alt-screen enter/exit + replay, multi-client, high
   throughput, immediate attach) to `packages/pty-testing/test/*.test.ts`; run the shared fixtures
   `tests/fixtures/parity/screens.json` and the conformance `bytes-split`/`escape-split` fixtures through this API and
   assert equality with the Rust testkit's output (write the expected strings into the fixture files if absent).
7. `README.md` for the package: install, engine requirement (`pty` on PATH, nix package), API, publishing
   (`npm publish` from `packages/pty-testing` on a tag `pty-testing-vX.Y.Z`; do not publish now).
