//! `--root`, the `PTY_SESSION_DIR` notices, and the root-length backstop.
//!
//! node: tests/pty-root.test.ts, tests/gc-flap-clear-badge-root-len.test.ts

mod cli_common;

use std::process::Command;

use cli_common::{Rig, pty_bin};

/// A command with an environment built from scratch (PATH + HOME only).
fn scrubbed(args: &[&str], env: &[(&str, &str)]) -> cli_common::Out {
    let mut c = Command::new(pty_bin());
    c.args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", std::env::var("HOME").unwrap_or_default());
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().unwrap();
    cli_common::Out {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code().unwrap_or(-1),
    }
}

/// node: tests/pty-root.test.ts:37-89, 233-257
#[test]
fn legacy_root_notices() {
    let rig = Rig::new();
    let a = rig.root.to_string_lossy().into_owned();
    let b = rig.scratch.to_string_lossy().into_owned();
    let out = scrubbed(
        &["list", "--json"],
        &[("PTY_ROOT", &a), ("PTY_SESSION_DIR", &b), ("PTY_ROOT_LEGACY_SILENT", "1")],
    );
    assert_eq!(out.code, 0);
    assert_eq!(out.stdout.trim(), "[]");
    assert_eq!(out.stderr, "");

    let out = scrubbed(&["list", "--json"], &[("PTY_SESSION_DIR", &b)]);
    assert_eq!(out.code, 0);
    assert_eq!(
        out.stderr,
        "pty: PTY_SESSION_DIR is deprecated; use PTY_ROOT (same shape, canonical name).\n"
    );

    let out = scrubbed(&["list", "--json"], &[("PTY_ROOT", &a)]);
    assert!(!out.stderr.contains("deprecated"));

    let out = scrubbed(
        &["list", "--json"],
        &[("PTY_SESSION_DIR", &b), ("PTY_ROOT_LEGACY_SILENT", "1")],
    );
    assert!(!out.stderr.contains("deprecated"));

    let out = scrubbed(&["list", "--json"], &[("PTY_ROOT", &a), ("PTY_SESSION_DIR", &b)]);
    assert_eq!(
        out.stderr,
        format!(
            "pty: both PTY_ROOT and PTY_SESSION_DIR are set — using PTY_ROOT ({a}); PTY_SESSION_DIR ({b}) is ignored (deprecated). For isolation, set PTY_ROOT.\n"
        )
    );
    assert_eq!(out.stderr.matches("both PTY_ROOT").count(), 1);
}

/// node: tests/pty-root.test.ts:93-142
#[test]
fn root_flag_pins_the_registry() {
    let rig = Rig::new();
    let flag_root = rig.scratch.join("flag-root");
    std::fs::create_dir_all(&flag_root).unwrap();
    // A planted record in the env root must not leak into the flag root.
    rig.write_meta("leak", serde_json::json!({"pid": 999999}));
    let out = rig.run(&["--root", flag_root.to_str().unwrap(), "list", "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout.trim(), "[]");
    // Any position works.
    let out = rig.run(&["list", "--json", "--root", flag_root.to_str().unwrap()]);
    assert_eq!(out.stdout.trim(), "[]");

    for args in [&["--root"][..], &["--root", "--json", "list"][..]] {
        let out = rig.run(args);
        assert_eq!(out.code, 1, "{args:?}");
        assert_eq!(
            out.stderr,
            "pty: --root requires a path (e.g. pty --root /var/lib/pty-eval list)\n"
        );
    }
}

/// node: tests/gc-flap-clear-badge-root-len.test.ts:163-230
#[test]
fn root_length_backstop() {
    let long_root = format!("/tmp/{}", "a".repeat(95));
    let out = scrubbed(&["list"], &[("PTY_ROOT", &long_root)]);
    assert_ne!(out.code, 0);
    assert_eq!(
        out.stderr,
        format!(
            "pty: PTY_ROOT is too long — 100 bytes; must be ≤ 90 bytes for the socket path to fit the 104-byte kernel limit.\n  root: {long_root}\n  Shorten the root (or use `pty --root <shorter-path>` for a one-off).\n"
        )
    );

    // Fires before the command switch: an unknown command is never reached.
    let root105 = format!("/tmp/{}", "b".repeat(100));
    let out = scrubbed(&["definitely-not-a-real-subcommand"], &[("PTY_ROOT", &root105)]);
    assert!(out.stderr.contains("PTY_ROOT is too long"));
    assert!(!out.stderr.contains("Unknown command"));

    // Exactly 90 bytes is fine.
    let root90 = format!("/tmp/{}", "c".repeat(85));
    std::fs::create_dir_all(&root90).unwrap();
    let out = scrubbed(&["list", "--json"], &[("PTY_ROOT", &root90), ("PTY_ROOT_LEGACY_SILENT", "1")]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout.trim(), "[]");
    let _ = std::fs::remove_dir(&root90);

    // `--root <short>` overrides an over-long env root before the check.
    let rig = Rig::new();
    let out = scrubbed(
        &["--root", rig.root.to_str().unwrap(), "list", "--json"],
        &[("PTY_ROOT", &long_root), ("PTY_ROOT_LEGACY_SILENT", "1")],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout.trim(), "[]");
}
