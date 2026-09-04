//! `pty help` / `--help` / `-h` print the Node usage text byte for byte, and
//! `pty version` prints the Rust version shape. The texts are the fixtures
//! under `tests/fixtures/help/`, captured from the Node `pty` at `500eab2`.
//!
//! node: tests/help.test.ts, tests/version.test.ts.

use std::process::{Command, Output};

const USAGE: &str = include_str!("fixtures/help/usage.txt");

/// node: tests/help.test.ts:13-18 (`COMMANDS`).
const COMMANDS: [&str; 23] = [
    "run",
    "attach",
    "exec",
    "peek",
    "send",
    "events",
    "list",
    "stats",
    "restart",
    "kill",
    "recover",
    "rm",
    "gc",
    "tag",
    "tag-multi",
    "emit",
    "rename",
    "metadata",
    "up",
    "down",
    "test",
    "remote-serve",
    "evidence",
];

/// node: tests/help.test.ts:20 (`ALIASES`).
const ALIASES: [(&str, &str); 3] = [("a", "attach"), ("ls", "list"), ("remove", "rm")];

fn pty(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pty"))
        .args(args)
        .env("PTY_ROOT_LEGACY_SILENT", "1")
        .env_remove("PTY_SESSION")
        .output()
        .expect("run pty")
}

fn fixture(cmd: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/help/{cmd}.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// node: tests/help.test.ts:100-112 (`pty --help` exits 0 and lists every
/// command); src/cli.ts:1634-1638 (`help`, `--help`, `-h` share one path).
#[test]
fn help_verbs_print_the_usage_text() {
    for verb in ["help", "--help", "-h"] {
        let out = pty(&[verb]);
        assert_eq!(out.status.code(), Some(0), "pty {verb}");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            USAGE,
            "pty {verb}: stdout is not the usage fixture"
        );
        assert!(out.stderr.is_empty(), "pty {verb}: stderr {:?}", out.stderr);
    }
    for cmd in COMMANDS {
        assert!(USAGE.contains(&format!("pty {cmd} ")), "usage lacks {cmd}");
    }
}

/// node: src/cli.ts:745-748 — `pty` with no arguments opens the interactive
/// session manager, not the usage text (the picker itself is supplied by
/// the TUI crate; until then the dispatcher reports its absence).
#[test]
fn no_arguments_opens_the_picker_not_the_usage() {
    let out = pty(&[]);
    assert_ne!(String::from_utf8_lossy(&out.stdout), USAGE);
    assert!(!String::from_utf8_lossy(&out.stderr).contains("Unknown command"));
}

/// node: tests/version.test.ts:25-47 — every version form prints one line
/// and exits 0, and is not an unknown command. The Rust shape is
/// `0.13.<n>-rust+<short-sha>` (docs/parity.md §14).
#[test]
fn version_forms_print_the_rust_version() {
    let re = regex::Regex::new(r"^0\.13\.\d+-rust\+[0-9a-f]{4,}$").unwrap();
    for form in ["version", "--version", "-v", "-V"] {
        let out = pty(&[form]);
        assert_eq!(out.status.code(), Some(0), "pty {form}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let line = stdout.trim_end_matches('\n');
        assert!(re.is_match(line), "pty {form}: {line:?}");
        assert!(!line.contains('\n'), "pty {form}: more than one line");
        assert!(
            !String::from_utf8_lossy(&out.stderr).contains("Unknown command"),
            "pty {form}: treated as unknown"
        );
    }
}

/// node: tests/help.test.ts:38-50 — `pty <cmd> --help` prints the command's
/// focused help and exits 0 without running the command.
///
/// Ignored until the dispatcher intercepts `-h`/`--help` as the token after
/// the command (node: src/cli.ts:756-758); the text itself is pinned by the
/// unit tests in `src/cli/help.rs`.
#[test]
#[ignore = "per-command --help interception lands with the dispatcher rewrite"]
fn per_command_help_prints_the_fixture() {
    for cmd in COMMANDS {
        for flag in ["--help", "-h"] {
            let out = pty(&[cmd, flag]);
            assert_eq!(out.status.code(), Some(0), "pty {cmd} {flag}");
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                fixture(cmd),
                "pty {cmd} {flag}"
            );
            assert!(out.stderr.is_empty(), "pty {cmd} {flag}: stderr");
        }
    }
    for (alias, canonical) in ALIASES {
        let out = pty(&[alias, "--help"]);
        assert_eq!(out.status.code(), Some(0), "pty {alias} --help");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            fixture(canonical),
            "pty {alias} --help"
        );
    }
}

/// node: tests/help.test.ts:53-70 — the evidence leaves print their own help.
///
/// Ignored for the same reason as [`per_command_help_prints_the_fixture`].
#[test]
#[ignore = "evidence's argument parser lands with the dispatcher rewrite"]
fn evidence_leaf_help_prints_the_fixture() {
    for leaf in ["snapshot", "remove"] {
        let out = pty(&["evidence", leaf, "--help"]);
        assert_eq!(out.status.code(), Some(0), "pty evidence {leaf} --help");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            fixture(&format!("evidence-{leaf}"))
        );
        assert!(out.stderr.is_empty());
    }
}

/// node: src/cli.ts:3489-3511 — `-h`/`--help` after other arguments reaches
/// tag-multi's own parser, which prints a different text.
///
/// Ignored for the same reason as [`per_command_help_prints_the_fixture`].
#[test]
#[ignore = "tag-multi's argument parser lands with the dispatcher rewrite"]
fn tag_multi_parser_help_prints_the_fixture() {
    let out = pty(&["tag-multi", "--all", "--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        fixture("tag-multi-parser")
    );
}
