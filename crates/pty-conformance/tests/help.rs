//! Port of tests/help.test.ts — the top-level usage only. Per-command help
//! (`pty <cmd> --help`) belongs to the per-command help work and is not here.

use pty_conformance::*;

/// Canonical subcommands the top-level usage must list.
const COMMANDS: &[&str] = &[
    "run", "attach", "exec", "peek", "send", "events", "list", "stats", "restart", "kill", "recover",
    "rm", "gc", "tag", "tag-multi", "emit", "rename", "metadata", "up", "down", "test",
    "remote-serve", "evidence",
];

/// node: tests/help.test.ts:104
#[test]
fn top_level_help_lists_every_subcommand() {
    let rig = Rig::new();
    let out = rig.pty(&["--help"]);
    expect_status(&out, 0);
    let stdout = out.stdout();
    for cmd in COMMANDS {
        expect_contains(&stdout, &format!("pty {cmd} "));
    }
}

/// node: tests/help.test.ts:104
#[test]
fn help_verbs_print_the_same_usage() {
    let rig = Rig::new();
    let reference = rig.pty(&["--help"]);
    expect_status(&reference, 0);
    assert!(reference.stdout().starts_with("Usage:"), "{}", reference.summary());
    for form in [&["help"][..], &["-h"][..]] {
        let out = rig.pty(form);
        expect_status(&out, 0);
        assert_eq!(out.stdout(), reference.stdout(), "`pty {}` differs from `pty --help`", form.join(" "));
        assert_eq!(out.stderr(), "", "`pty {}` wrote to stderr", form.join(" "));
    }
}
