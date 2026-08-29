# 0004 — `gc --print-launchd-plist` lists the binary itself as the program

**Status:** accepted

**Node behavior.** `ProgramArguments` is three strings: `process.execPath`
(the `node` interpreter), `process.argv[1]` (the invoked launcher, e.g.
`.../bin/pty` or `.../dist/cli.js`), and `gc` (`src/cli.ts:3245-3255`).
`tests/gc.test.ts:303-312` pins that the invoked launcher path appears as a
`<string>` element.

**Rust behavior.** `ProgramArguments` is two strings: the absolute path of
the running `pty` executable (`std::env::current_exe()`), and `gc`. Every
other element of the plist — the `Label` rules, `StartInterval`,
`RunAtLoad`, the `<root>/gc.log` paths, `PATH` and `PTY_ROOT` — is
byte-for-byte Node's.

**Why.** There is no interpreter to name: the Rust binary is its own
program. Emitting a fake first element would produce a plist launchd cannot
run.

**Client effect.** A script that parses the plist and expects three
`ProgramArguments` sees two. `launchctl load` of either plist runs `pty gc`
against the same root.

**Test.** `crates/pty/tests/cli_gc.rs::launchd_plist` pins the Rust plist
byte for byte. Node's own assertion (`<string><binPath></string>`) holds on
both binaries, so the conformance suite needs no gated pair.

**Migration / negotiation.** None; a plist installed from the Node binary
keeps working until it is regenerated from the Rust one.
