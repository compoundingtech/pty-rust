//! `pty completions <shell>` prints the vendored completion scripts byte for
//! byte, with Node's usage text and exit codes.
//!
//! node: tests/completions.test.ts.

use std::process::{Command, Output};

const USAGE: &str = include_str!("fixtures/help/completions.txt");

/// The checked-in scripts at the repo root, the same files the binary embeds.
const SCRIPTS: [(&str, &str); 3] = [
    ("fish", include_str!("../../../completions/pty.fish")),
    ("bash", include_str!("../../../completions/pty.bash")),
    ("zsh", include_str!("../../../completions/pty.zsh")),
];

fn pty(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pty"))
        .args(args)
        .env("PTY_ROOT_LEGACY_SILENT", "1")
        .output()
        .expect("run pty")
}

/// node: tests/completions.test.ts:81-89 (every checked-in artifact is what
/// the binary prints), :181-187 (fish, bash, zsh go to stdout).
#[test]
fn prints_the_vendored_script_for_each_shell() {
    for (shell, file) in SCRIPTS {
        let out = pty(&["completions", shell]);
        assert_eq!(out.status.code(), Some(0), "pty completions {shell}");
        assert!(out.stderr.is_empty(), "pty completions {shell}: stderr");
        assert!(
            out.stdout == file.as_bytes(),
            "pty completions {shell}: stdout differs from completions/pty.{shell}"
        );
    }
}

/// node: tests/completions.test.ts:197-203.
#[test]
fn help_flag_prints_usage_to_stdout() {
    for flag in ["--help", "-h"] {
        let out = pty(&["completions", flag]);
        assert_eq!(out.status.code(), Some(0), "pty completions {flag}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), USAGE);
        assert!(out.stderr.is_empty());
    }
}

/// node: src/completions.ts:734-737 — no shell: usage on stderr, exit 2.
#[test]
fn missing_shell_prints_usage_to_stderr_and_exits_2() {
    let out = pty(&["completions"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert_eq!(String::from_utf8_lossy(&out.stderr), USAGE);
}

/// node: tests/completions.test.ts:189-195; src/completions.ts:738-743 — an
/// unknown shell names itself, then a blank line, then the usage, exit 2.
#[test]
fn unknown_shell_prints_usage_to_stderr_and_exits_2() {
    let out = pty(&["completions", "tcsh"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!("pty completions: unknown shell: tcsh\n\n{USAGE}")
    );
}

/// node: tests/completions.test.ts:91-96.
#[test]
fn every_script_offers_run_env() {
    let markers = [("fish", "-l env"), ("bash", "--env"), ("zsh", "--env")];
    for (shell, marker) in markers {
        let out = pty(&["completions", shell]);
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(marker),
            "{shell} script lacks {marker:?}"
        );
    }
}

/// node: tests/completions.test.ts:205-227 — each script parses in its shell
/// (`<shell> -n -c <script>`), for the shells installed on this machine.
#[test]
fn scripts_are_syntactically_valid() {
    for (shell, _) in SCRIPTS {
        let script = String::from_utf8(pty(&["completions", shell]).stdout).unwrap();
        let check = Command::new(shell).args(["-n", "-c", &script]).output();
        let out = match check {
            Ok(out) => out,
            Err(_) => {
                // Not installed here; bash is the one shell every test host has.
                assert_ne!(shell, "bash", "bash is required for this test");
                continue;
            }
        };
        assert!(
            out.status.success(),
            "{shell} -n failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
