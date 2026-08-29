// Implementation of the `conformance-map` bin: reads the `/// node:` doc
// comments in `tests/*.rs`, joins them with the fixed classification of
// every Node suite below, and writes `docs/conformance.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How a Node suite is covered.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Black-box CLI tests (`rig.pty`, `rig.pty_tty`, `rig.daemon`).
    Cli,
    /// Socket-level tests (`rig.daemon` + `rig.connect`).
    Protocol,
    /// Pure-logic tests already ported as crate unit tests.
    Unit,
    /// Cannot run against a binary; listed with the reason.
    NotPortable,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Cli => "cli",
            Kind::Protocol => "protocol",
            Kind::Unit => "unit",
            Kind::NotPortable => "not-portable",
        }
    }
}

const TUI: &str = "TUI widget / framework test against the Node in-process renderer";

/// Every Node suite (tests/*.test.ts in the Node checkout), its kind, and a
/// note: the reason for not-portable, or what the port covers.
const SUITES: &[(&str, Kind, &str)] = &[
    ("accordion", Kind::NotPortable, TUI),
    ("action-list-item", Kind::NotPortable, TUI),
    ("atomic-writes", Kind::Protocol, "concurrent CLI writers; later pass"),
    ("attach-no-restart", Kind::Cli, "later pass (attach*)"),
    ("attach-stream", Kind::Cli, "later pass (attach*); frame checks are protocol"),
    ("badge", Kind::NotPortable, TUI),
    ("breadcrumbs", Kind::NotPortable, TUI),
    ("buffer-palette", Kind::NotPortable, TUI),
    ("buffer-wide-char-diff", Kind::NotPortable, TUI),
    ("code-block", Kind::NotPortable, TUI),
    ("codex-integration", Kind::NotPortable, "drives the Node codex integration in-process"),
    ("completions", Kind::Cli, "per-command help and completions belong to the help work package"),
    ("connection", Kind::Protocol, "attach/screen/exit/geometry parts; later pass"),
    ("disk-layout-docs", Kind::NotPortable, "lint of the Node docs against the Node source"),
    ("display-name", Kind::Cli, ""),
    ("duration", Kind::Unit, "pty-core duration parser unit tests"),
    ("effective-geometry", Kind::Protocol, "later pass"),
    ("env-isolation", Kind::Unit, "build_spawn_env unit tests in pty-testkit; the CLI half is in spawn_options.rs"),
    ("events-emit", Kind::Cli, "CLI half; emitUserEvent / retention / EventFollower are library-only"),
    ("events", Kind::Cli, "CLI half (events --follow / --recent / --json); later pass"),
    ("exec", Kind::Cli, "later pass"),
    ("exit-event-race", Kind::Protocol, "later pass"),
    ("exit-reap", Kind::Cli, "reap-policy half; the exact-generation evidence half (pty evidence) is deferred"),
    ("exit-signal", Kind::Cli, "later pass"),
    ("filter", Kind::NotPortable, TUI),
    ("focus", Kind::NotPortable, TUI),
    ("gc-abandoned", Kind::Cli, "later pass (gc*)"),
    ("gc-flap-clear-badge-root-len", Kind::Cli, "root-length half is in pty_root.rs; the flap/badge half is a later pass (gc*)"),
    ("gc-flapping", Kind::Cli, "later pass (gc*)"),
    ("gc-generation-guard", Kind::Cli, "later pass (gc*)"),
    ("gc-parent-child", Kind::Cli, "later pass (gc*)"),
    ("gc-permanent", Kind::Cli, "later pass (gc*)"),
    ("gc", Kind::Cli, "later pass (gc*); the plist half of pty-root.test.ts belongs here too"),
    ("help", Kind::Cli, "top-level usage only; per-command help belongs to the help work package"),
    ("hit-test", Kind::NotPortable, TUI),
    ("input-parse", Kind::Unit, "pty-core input parser unit tests"),
    ("integration", Kind::Protocol, "sync, roles, geometry, malformed packets, stats; later pass"),
    ("keys", Kind::Unit, "pty-core key resolver unit tests"),
    ("kill-releases-socket-command", Kind::Cli, "later pass (process-tree)"),
    ("kill-wait", Kind::Cli, ""),
    ("list-filters", Kind::Cli, ""),
    ("list-live-session-race", Kind::Cli, "later pass"),
    ("list-liveness-budget", Kind::Cli, "later pass"),
    ("list-purity", Kind::Cli, "CLI half; the raw-candidate revalidation case is library-only"),
    ("message", Kind::NotPortable, TUI),
    ("metadata-events", Kind::Cli, "CLI half; patchMetadataById / EventFollower cases are library-only"),
    ("mouse-parse", Kind::Unit, "pty-core mouse parser unit tests"),
    ("nesting-prevention", Kind::Cli, ""),
    ("nesting", Kind::Cli, ""),
    ("panel-footer-title", Kind::NotPortable, TUI),
    ("parity-fixtures", Kind::Cli, "shared screens.json loader"),
    ("parity-node-reference", Kind::Cli, "later pass"),
    ("parity-shapes", Kind::Cli, "shared shapes.json loader"),
    ("peek-wait", Kind::Cli, ""),
    ("process-title", Kind::Cli, "later pass (reads /proc; serialized)"),
    ("process-tree", Kind::Cli, "later pass (kill on a 3-deep tree)"),
    ("progress-bars", Kind::NotPortable, TUI),
    ("protocol", Kind::Unit, "pty-core framing unit tests; the socket-level limits are in fixtures_protocol.rs"),
    ("pty-handle", Kind::NotPortable, TUI),
    ("pty-pane", Kind::NotPortable, TUI),
    ("pty-root", Kind::Cli, "root precedence, notices, --root; the gc plist half is a later pass (gc*)"),
    ("ptyfile", Kind::Unit, "pty-core manifest parser unit tests; the CLI half is up_down.rs"),
    ("ratatui-compat", Kind::Protocol, "re-expressed as SCREEN payload checks through the socket; later pass"),
    ("recovery", Kind::Protocol, "deferred (pty recover)"),
    ("remote-exec-bridge", Kind::Cli, "later pass, with a fake fabric shim on PATH"),
    ("remote-fabric", Kind::Cli, "later pass, with a fake fabric shim on PATH"),
    ("remote-reconnect", Kind::Cli, "later pass, with a fake fabric shim on PATH"),
    ("resize-tui", Kind::NotPortable, "drives the Node TUI through the Node testing Session"),
    ("restart-env-scrub", Kind::Cli, ""),
    ("restart-guardrail", Kind::Cli, ""),
    ("restart-launch-parity", Kind::Cli, ""),
    ("rm-immediate-reuse", Kind::Cli, "CLI half; the cleanupOwned* case is library-only"),
    ("rm-kill-ephemeral", Kind::Cli, ""),
    ("sanitize", Kind::Protocol, "bytes emitted by attach; later pass"),
    ("screen-replay-altscreen", Kind::Protocol, "later pass"),
    ("screenshot", Kind::NotPortable, "in-process xterm screenshot of the Node testing library"),
    ("scrollback-fidelity", Kind::Protocol, "later pass"),
    ("security-fixes", Kind::Protocol, "lock steal via <id>.lock files; later pass"),
    ("select", Kind::NotPortable, TUI),
    ("send-paste", Kind::Cli, ""),
    ("seq-delay", Kind::Cli, "end-to-end half; resolveSeqDelayMs is a pty-core unit test"),
    ("shells", Kind::NotPortable, "shell integration through the Node testing Session (pty-testkit covers it)"),
    ("shutdown-backstop", Kind::Cli, "later pass (PTY_SHUTDOWN_DEADLINE_MS)"),
    ("spawn-bundle-fallback", Kind::Cli, "later pass (only the run -d argv shape)"),
    ("spawn-options", Kind::Cli, "CLI half; spawnDaemon(...) library cases stay in Node"),
    ("spawner-pid-watchdog", Kind::Cli, "later pass (PTY_SPAWNER_PID)"),
    ("stats-cli", Kind::Cli, ""),
    ("tag-bulk", Kind::Cli, ""),
    ("tag-multi", Kind::Cli, ""),
    ("tag-mutate", Kind::Cli, ""),
    ("tags-helpers", Kind::Cli, "CLI half; the pure functions are pty-core unit tests"),
    ("tags", Kind::Cli, ""),
    ("terminal-queries", Kind::Protocol, "responses over the socket; the strip half is a pty-core unit test; later pass"),
    ("tokens", Kind::NotPortable, TUI),
    ("tui-framework", Kind::NotPortable, TUI),
    ("tui", Kind::NotPortable, "drives the Node session manager TUI"),
    ("up-down", Kind::Cli, ""),
    ("up-name-decouple", Kind::Cli, ""),
    ("version", Kind::Cli, "CLI half; formatVersion is a unit test"),
    ("widgets-bar-chart", Kind::NotPortable, TUI),
    ("widgets-command-palette", Kind::NotPortable, TUI),
    ("widgets-command-registry", Kind::NotPortable, TUI),
    ("widgets-confirm", Kind::NotPortable, TUI),
    ("widgets-date-picker", Kind::NotPortable, TUI),
    ("widgets-form", Kind::NotPortable, TUI),
    ("widgets-help-overlay", Kind::NotPortable, TUI),
    ("widgets-markdown", Kind::NotPortable, TUI),
    ("widgets-prompt-bar", Kind::NotPortable, TUI),
    ("widgets-sparkline", Kind::NotPortable, TUI),
    ("widgets-stream-view", Kind::NotPortable, TUI),
    ("widgets-table", Kind::NotPortable, TUI),
    ("widgets-tabs-mouse", Kind::NotPortable, TUI),
    ("widgets-tabs", Kind::NotPortable, TUI),
    ("widgets-text-area", Kind::NotPortable, TUI),
    ("widgets-toast", Kind::NotPortable, TUI),
    ("widgets-toolbar", Kind::NotPortable, TUI),
    ("widgets-tree", Kind::NotPortable, TUI),
    ("widgets-virtual-list-mouse", Kind::NotPortable, TUI),
    ("widgets-virtual-list", Kind::NotPortable, TUI),
    ("wrapper-signal-forwarding", Kind::Cli, "later pass"),
];

/// One `#[test]` with its `/// node:` reference.
struct Mapped {
    rust_file: String,
    test_name: String,
    /// `None` for a test with no Node counterpart (the Rust-owned fixtures).
    node_suite: Option<String>,
    node_line: Option<u32>,
    gated: bool,
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Parse `tests/*.rs` for `/// node: tests/<suite>.test.ts[:<line>]`
/// comments attached to `#[test]` functions. A test is gated when its name
/// ends in `_node` or `_rust`.
fn scan_tests(tests_dir: &Path) -> Result<Vec<Mapped>, String> {
    let mut out = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(tests_dir)
        .map_err(|e| format!("{}: {e}", tests_dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "rs").unwrap_or(false))
        .collect();
    entries.sort();
    for path in entries {
        let rust_file = path.file_name().unwrap().to_string_lossy().into_owned();
        let src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut pending: Option<(String, Option<u32>)> = None;
        let mut saw_test_attr = false;
        for line in src.lines() {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("/// node:") {
                let rest = rest.trim();
                if let Some(spec) = rest.strip_prefix("tests/") {
                    let (file, line_no) = match spec.split_once(".test.ts") {
                        Some((f, tail)) => (
                            f.to_string(),
                            tail.strip_prefix(':').and_then(|n| n.trim().parse().ok()),
                        ),
                        None => continue,
                    };
                    pending = Some((file, line_no));
                }
                continue;
            }
            if t == "#[test]" {
                saw_test_attr = true;
                continue;
            }
            if saw_test_attr && let Some(name) = t.strip_prefix("fn ") {
                let name = name.split('(').next().unwrap_or("").trim().to_string();
                let gated = name.ends_with("_node") || name.ends_with("_rust");
                let (node_suite, node_line) = match pending.take() {
                    Some((suite, line_no)) => (Some(suite), line_no),
                    None => (None, None),
                };
                out.push(Mapped {
                    rust_file: rust_file.clone(),
                    test_name: name,
                    node_suite,
                    node_line,
                    gated,
                });
                saw_test_attr = false;
                continue;
            }
            if !t.starts_with("///") && !t.starts_with("#[") && !t.is_empty() {
                // Anything else between the comment and the fn resets.
                if !saw_test_attr {
                    pending = None;
                }
            }
        }
    }
    Ok(out)
}

/// Node suites present in the checkout (or the fixed list when unavailable).
fn node_suites(checkout: Option<&Path>) -> Vec<String> {
    if let Some(dir) = checkout
        && let Ok(rd) = std::fs::read_dir(dir.join("tests"))
    {
        let mut v: Vec<String> = rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter_map(|f| f.strip_suffix(".test.ts").map(|s| s.to_string()))
            .collect();
        v.sort();
        if !v.is_empty() {
            return v;
        }
    }
    let mut v: Vec<String> = SUITES.iter().map(|(n, _, _)| n.to_string()).collect();
    v.sort();
    v
}

pub fn run(args: Vec<String>) -> Result<(), String> {
    let mut checkout: Option<PathBuf> = std::env::var("PTY_NODE_CHECKOUT").ok().map(PathBuf::from);
    let mut out_path = manifest_dir().join("../../docs/conformance.md");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--node" => {
                i += 1;
                checkout = Some(PathBuf::from(args.get(i).ok_or("--node needs a path")?));
            }
            "--out" => {
                i += 1;
                out_path = PathBuf::from(args.get(i).ok_or("--out needs a path")?);
            }
            other => return Err(format!("unknown argument {other}")),
        }
        i += 1;
    }
    if checkout.is_none() {
        let default = PathBuf::from("/home/myobie/src/github.com/compoundingtech/pty");
        if default.join("tests").is_dir() {
            checkout = Some(default);
        }
    }

    let mapped = scan_tests(&manifest_dir().join("tests"))?;
    let suites = node_suites(checkout.as_deref());
    let classified: BTreeMap<&str, (Kind, &str)> =
        SUITES.iter().map(|(n, k, note)| (*n, (*k, *note))).collect();

    let mut by_suite: BTreeMap<String, Vec<&Mapped>> = BTreeMap::new();
    for m in &mapped {
        if let Some(suite) = &m.node_suite {
            by_suite.entry(suite.clone()).or_default().push(m);
        }
    }
    let unmapped = mapped.iter().filter(|m| m.node_suite.is_none()).count();
    for suite in by_suite.keys() {
        if !suites.contains(suite) {
            return Err(format!("tests reference unknown Node suite {suite}.test.ts"));
        }
    }
    for suite in &suites {
        if !classified.contains_key(suite.as_str()) {
            return Err(format!("Node suite {suite}.test.ts has no classification in conformance_map_impl.rs"));
        }
    }

    let total_tests = mapped.len();
    let gated: Vec<&Mapped> = mapped.iter().filter(|m| m.gated).collect();
    let mut counts: BTreeMap<&str, (usize, usize)> = BTreeMap::new(); // kind -> (suites, ported)
    for suite in &suites {
        let (kind, _) = classified[suite.as_str()];
        let e = counts.entry(kind.label()).or_default();
        e.0 += 1;
        if by_suite.contains_key(suite) {
            e.1 += 1;
        }
    }

    let mut md = String::new();
    md.push_str("# Conformance map\n\n");
    md.push_str("Generated by `cargo run -p pty-conformance --bin conformance-map`; do not edit by hand.\n\n");
    md.push_str("One row per Node test file (`tests/*.test.ts` in the Node checkout, 0.12.0+500eab2). ");
    md.push_str("`kind` is how the suite is covered: `cli` (black-box through the binary), `protocol` (over the session socket), ");
    md.push_str("`unit` (pure logic, ported as crate unit tests), or `not-portable` (with the reason). ");
    md.push_str("A `cli`/`protocol` suite with no Rust file yet is still to do. ");
    md.push_str("Rust tests carry a `/// node: tests/<file>.test.ts:<line>` comment; the `tests` column counts them and the last column lists the Node lines they pin.\n\n");
    md.push_str("Run: `PTY_TEST_BIN=$(which pty) cargo test -p pty-conformance` (Node) and `cargo test -p pty-conformance` after `cargo build -p pty` (Rust).\n\n");

    md.push_str("## Summary\n\n");
    md.push_str(&format!("- Node suites: {}\n", suites.len()));
    md.push_str(&format!(
        "- Rust conformance tests: {total_tests} ({} port a Node test, {unmapped} cover the Rust-owned fixtures)\n",
        total_tests - unmapped
    ));
    md.push_str(&format!(
        "- Gated (`_node`/`_rust` pairs pointing at a decision record): {} — the parity debt\n",
        gated.len()
    ));
    for (kind, (n, ported)) in &counts {
        if *kind == "not-portable" || *kind == "unit" {
            md.push_str(&format!("- {kind}: {n}\n"));
        } else {
            md.push_str(&format!("- {kind}: {n} suites, {ported} with Rust tests, {} to do\n", n - ported));
        }
    }
    md.push('\n');

    md.push_str("## Suites\n\n");
    md.push_str("| Node suite | kind | Rust test file(s) | tests | Node lines / reason |\n");
    md.push_str("|---|---|---|---|---|\n");
    for suite in &suites {
        let (kind, note) = classified[suite.as_str()];
        let tests = by_suite.get(suite);
        let mut files: Vec<String> = tests
            .map(|v| v.iter().map(|m| m.rust_file.clone()).collect())
            .unwrap_or_default();
        files.sort();
        files.dedup();
        let count = tests.map(|v| v.len()).unwrap_or(0);
        let mut lines: Vec<u32> = tests
            .map(|v| v.iter().filter_map(|m| m.node_line).collect())
            .unwrap_or_default();
        lines.sort_unstable();
        lines.dedup();
        let status = match kind {
            Kind::Cli | Kind::Protocol if count == 0 => format!("{} (to do)", kind.label()),
            _ => kind.label().to_string(),
        };
        let detail = if count > 0 {
            let l = lines.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ");
            if note.is_empty() { l } else { format!("{l} — {note}") }
        } else {
            note.to_string()
        };
        md.push_str(&format!(
            "| {suite}.test.ts | {status} | {} | {count} | {detail} |\n",
            if files.is_empty() { "—".to_string() } else { files.join(", ") }
        ));
    }
    md.push('\n');

    md.push_str("## Gated tests\n\n");
    if gated.is_empty() {
        md.push_str("None.\n\n");
    } else {
        md.push_str("| Rust test | Node suite | decision |\n|---|---|---|\n");
        for m in &gated {
            md.push_str(&format!(
                "| {}::{} | {} | see the test's doc comment |\n",
                m.rust_file,
                m.test_name,
                m.node_suite.as_ref().map(|s| format!("{s}.test.ts")).unwrap_or_else(|| "—".into())
            ));
        }
        md.push('\n');
    }

    md.push_str("## Fixtures\n\n");
    md.push_str("- `tests/fixtures/parity/{screens,shapes}.json` (Node-owned, vendored byte-identical; checked against `PTY_NODE_CHECKOUT` when set) — `parity_fixtures.rs`, `parity_shapes.rs`.\n");
    md.push_str("- `crates/pty-conformance/fixtures/*.json` (Rust-owned, v1) — `fixtures_protocol.rs`: bytes-split, escape-split, raw-bytes (decision 0001), attach-identity, late-events, frame-limits, slow-reader.\n\n");

    md.push_str("## Still to do\n\n");
    let mut todo: Vec<&String> = suites
        .iter()
        .filter(|s| {
            let (k, _) = classified[s.as_str()];
            matches!(k, Kind::Cli | Kind::Protocol) && !by_suite.contains_key(*s)
        })
        .collect();
    todo.sort();
    for s in todo {
        let (k, note) = classified[s.as_str()];
        md.push_str(&format!("- {s}.test.ts ({}): {note}\n", k.label()));
    }
    md.push_str("- Halves left out of otherwise-ported suites: the `pty evidence` half of exit-reap.test.ts (deferred), the gc plist half of pty-root.test.ts and the flap/badge half of gc-flap-clear-badge-root-len.test.ts (gc*), the retention cap and EventFollower halves of events-emit.test.ts and metadata-events.test.ts (library-only).\n");

    std::fs::write(&out_path, md).map_err(|e| format!("{}: {e}", out_path.display()))?;
    println!(
        "wrote {} ({} suites, {} tests, {} gated)",
        out_path.display(),
        suites.len(),
        total_tests,
        gated.len()
    );
    Ok(())
}
