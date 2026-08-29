//! The dispatcher: version, help interception, unknown commands and
//! git-style forwarding, the deferred verbs, and the interactive nesting
//! guard.
//!
//! node: tests/version.test.ts, tests/help.test.ts,
//! tests/nesting-prevention.test.ts:213-241, src/cli.ts:1641-1660

mod cli_common;

use std::os::unix::fs::PermissionsExt;

use cli_common::Rig;

const USAGE: &str = include_str!("fixtures/help/usage.txt");

/// node: tests/version.test.ts:31-45
#[test]
fn version_spellings_print_the_version() {
    let rig = Rig::new();
    let re = regex::Regex::new(r"^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?(\+[0-9a-f]{4,}|\+unknown)?$").unwrap();
    for spelling in ["version", "--version", "-v", "-V"] {
        let out = rig.ok(&[spelling]);
        assert!(re.is_match(out.stdout.trim()), "{spelling}: {:?}", out.stdout);
        assert!(!out.stderr.contains("Unknown command"));
    }
}

/// node: src/cli.ts:1657-1659 — stderr line, usage on stdout, exit 1.
#[test]
fn unknown_command_prints_usage_on_stdout() {
    let rig = Rig::new();
    let out = rig.run(&["definitely-not-a-real-subcommand"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, "Unknown command: definitely-not-a-real-subcommand\n");
    assert_eq!(out.stdout, USAGE);
}

/// node: src/cli.ts:1641-1655 — `which pty-<cmd>` forwarding with the
/// unfiltered args, inherited stdio and the extension's exit status.
#[test]
fn forwards_to_a_pty_extension_on_path() {
    let rig = Rig::new();
    let bin_dir = rig.scratch.join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let script = bin_dir.join("pty-hello");
    std::fs::write(
        &script,
        "#!/bin/sh\necho \"hello from extension: $* root=$PTY_ROOT\"\nexit 7\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = rig.run_env(&["hello", "--force", "world"], &[("PATH", &path)]);
    assert_eq!(out.code, 7);
    assert_eq!(
        out.stdout,
        format!(
            "hello from extension: --force world root={}\n",
            rig.root.display()
        )
    );
    assert!(!out.stderr.contains("Unknown command"));
}

/// node: src/cli.ts:756-758, tests/help.test.ts — `-h`/`--help` right after
/// the command prints that command's help; aliases resolve.
#[test]
fn per_command_help_is_intercepted_first_position_only() {
    let rig = Rig::new();
    let tag = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/help/tag.txt"
    ))
    .unwrap();
    let out = rig.ok(&["tag", "--help"]);
    assert_eq!(out.stdout, tag);
    let out = rig.ok(&["tag", "-h", "ignored"]);
    assert_eq!(out.stdout, tag);
    let list = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/help/list.txt"
    ))
    .unwrap();
    assert_eq!(rig.ok(&["ls", "--help"]).stdout, list);
    assert_eq!(rig.ok(&["a", "-h"]).stdout, rig.ok(&["attach", "--help"]).stdout);
    assert_eq!(rig.ok(&["remove", "-h"]).stdout, rig.ok(&["rm", "--help"]).stdout);
    // Only args[1] counts: `pty tag <ref> --help` reaches the command.
    let out = rig.run(&["tag", "nope", "--help"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, "Session \"nope\" not found.\n");
    // Verbs without an entry fall through to their own handling.
    assert_eq!(rig.ok(&["help", "--help"]).stdout, USAGE);
    for spelling in ["help", "--help", "-h"] {
        assert_eq!(rig.ok(&[spelling]).stdout, USAGE);
    }
}

/// docs/parity.md §12 — recover, evidence and test are documented as absent;
/// their vendored help still prints.
#[test]
fn deferred_verbs_report_their_absence() {
    let rig = Rig::new();
    for cmd in ["recover", "evidence", "test"] {
        let out = rig.run(&[cmd, "whatever"]);
        assert_eq!(out.code, 1, "{cmd}");
        assert_eq!(
            out.stderr,
            format!("pty {cmd}: not available in this build. See docs/parity.md.\n")
        );
        assert_eq!(out.stdout, "");
        let help = rig.ok(&[cmd, "--help"]);
        assert!(help.stdout.starts_with(&format!("Usage: pty {cmd}")), "{cmd}: {:?}", help.stdout);
    }
}

/// node: tests/nesting-prevention.test.ts:213-241 — the interactive picker
/// refuses to open inside a session unless `--force`.
#[test]
fn interactive_refuses_inside_a_session_unless_forced() {
    let rig = Rig::new();
    for args in [&[][..], &["i"][..], &["interactive"][..]] {
        let out = rig.run_env(args, &[("PTY_SESSION", "outer-session")]);
        assert_ne!(out.code, 0, "{args:?}");
        assert_eq!(
            out.stderr,
            "pty interactive: already inside pty session \"outer-session\".\n  The interactive picker would render inside your current session and detach would route to the outer client.\n  Detach first (Ctrl+\\) and run `pty` from outside, or pass --force to open the picker anyway.\n",
            "{args:?}"
        );
    }
    let out = rig.run_env(&["--force"], &[("PTY_SESSION", "outer-session")]);
    assert!(!out.stderr.contains("already inside pty session"), "{:?}", out.stderr);
}

/// node: src/cli.ts:96-103, 738-742 — a bad `--filter-tag` before the
/// picker is an error; `--preselect-new` alone still means the picker.
#[test]
fn interactive_flags_are_parsed_before_the_switch() {
    let rig = Rig::new();
    let out = rig.run(&["--filter-tag", "novalue"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, "--filter-tag expects \"key=value\"\n");
    let out = rig.run(&["--preselect-new"]);
    assert!(!out.stderr.contains("Unknown command"), "{:?}", out.stderr);
}

/// node: src/cli.ts:1620-1624 — completions is the only verb that exits 2.
#[test]
fn completions_exit_two_on_usage_errors() {
    let rig = Rig::new();
    assert_eq!(rig.run(&["completions"]).code, 2);
    assert_eq!(rig.run(&["completions", "powershell"]).code, 2);
    assert_eq!(rig.run(&["completions", "bash"]).code, 0);
}
