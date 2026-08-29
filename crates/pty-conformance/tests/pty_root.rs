//! Port of tests/pty-root.test.ts (root precedence, the one-time notices, the
//! `--root` flag, where `run -d` lands) and the `PTY_ROOT` length backstop
//! from tests/gc-flap-clear-badge-root-len.test.ts. The `gc
//! --print-launchd-plist` half of pty-root.test.ts belongs to the gc suite.

use pty_conformance::*;

fn s(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}

/// node: tests/pty-root.test.ts:37
#[test]
fn pty_root_wins_over_pty_session_dir() {
    let rig = Rig::new();
    let winner = rig.make_root();
    let loser = rig.make_root();
    let out = rig.pty_clean(
        &[
            ("PTY_ROOT", &s(&winner)),
            ("PTY_SESSION_DIR", &s(&loser)),
            ("PTY_ROOT_LEGACY_SILENT", "1"),
        ],
        &["list", "--json"],
    );
    let v = expect_json(&out);
    assert_eq!(v, serde_json::json!([]));
}

/// node: tests/pty-root.test.ts:55
#[test]
fn pty_session_dir_only_emits_deprecation_notice_once() {
    let rig = Rig::new();
    let dir = rig.make_root();
    let out = rig.pty_clean(&[("PTY_SESSION_DIR", &s(&dir))], &["list", "--json"]);
    expect_status(&out, 0);
    let err = out.stderr();
    expect_regex(&err, "PTY_SESSION_DIR is deprecated");
    assert_eq!(count_regex(&err, "PTY_SESSION_DIR is deprecated"), 1, "{err}");
}

/// node: tests/pty-root.test.ts:68
#[test]
fn pty_root_only_emits_no_notice() {
    let rig = Rig::new();
    let dir = rig.make_root();
    let out = rig.pty_clean(&[("PTY_ROOT", &s(&dir))], &["list", "--json"]);
    expect_status(&out, 0);
    expect_not_regex(&out.stderr(), "deprecated");
}

/// node: tests/pty-root.test.ts:79
#[test]
fn legacy_silent_suppresses_the_deprecation_notice() {
    let rig = Rig::new();
    let dir = rig.make_root();
    let out = rig.pty_clean(
        &[("PTY_SESSION_DIR", &s(&dir)), ("PTY_ROOT_LEGACY_SILENT", "1")],
        &["list", "--json"],
    );
    expect_status(&out, 0);
    expect_not_regex(&out.stderr(), "deprecated");
}

/// node: tests/pty-root.test.ts:93
#[test]
fn root_flag_scopes_list_to_the_given_registry() {
    let rig = Rig::new();
    let dir = rig.make_root();
    let out = rig.pty_clean(
        &[("PTY_ROOT_LEGACY_SILENT", "1")],
        &["--root", &s(&dir), "list", "--json"],
    );
    assert_eq!(expect_json(&out), serde_json::json!([]));
}

/// node: tests/pty-root.test.ts:104
#[test]
fn root_flag_overrides_pty_root_env() {
    let rig = Rig::new();
    let flag_root = rig.make_root();
    let env_root = rig.make_root();
    // A planted metadata file in the env root would leak into the list if
    // --root were ignored.
    std::fs::write(
        env_root.join("leak.json"),
        serde_json::json!({
            "command": "sh", "args": [], "displayCommand": "sh",
            "cwd": "/tmp", "rows": 24, "cols": 80, "tags": {}, "pid": 999999,
            "createdAt": "2026-01-01T00:00:00.000Z",
        })
        .to_string(),
    )
    .unwrap();
    let out = rig.pty_clean(
        &[("PTY_ROOT", &s(&env_root))],
        &["--root", &s(&flag_root), "list", "--json"],
    );
    assert_eq!(expect_json(&out), serde_json::json!([]));
}

/// node: tests/pty-root.test.ts:126
#[test]
fn root_flag_without_a_value_fails() {
    let rig = Rig::new();
    let out = rig.pty_clean(&[], &["--root"]);
    expect_failure(&out);
    expect_regex(&out.stderr(), "--root requires a path");
}

/// node: tests/pty-root.test.ts:135
#[test]
fn root_flag_does_not_swallow_a_following_flag() {
    let rig = Rig::new();
    let out = rig.pty_clean(&[], &["--root", "--json", "list"]);
    expect_failure(&out);
    expect_regex(&out.stderr(), "--root requires a path");
}

/// node: tests/pty-root.test.ts:201
#[test]
fn detached_session_lands_only_under_pty_root() {
    let rig = Rig::new();
    let root = rig.make_root();
    let scratch = rig.make_root();
    let name = unique_id("rd");
    let out = rig.pty_clean(
        &[
            ("PTY_ROOT", &s(&root)),
            ("PTY_SESSION_DIR", &s(&scratch)),
            ("PTY_ROOT_LEGACY_SILENT", "1"),
        ],
        &["run", "-d", "--id", &name, "--", "cat"],
    );
    expect_status(&out, 0);
    let json = root.join(format!("{name}.json"));
    let sock = root.join(format!("{name}.sock"));
    wait_until("session files under PTY_ROOT", || json.exists() && sock.exists());
    assert!(!scratch.join(format!("{name}.json")).exists(), "leaked into PTY_SESSION_DIR");
    assert!(
        !rig.home().join(".local/state/pty").join(format!("{name}.json")).exists(),
        "leaked into the default registry"
    );
    let kill = rig.pty_clean(
        &[("PTY_ROOT", &s(&root)), ("PTY_ROOT_LEGACY_SILENT", "1")],
        &["kill", &name],
    );
    expect_status(&kill, 0);
}

/// node: tests/pty-root.test.ts:233
#[test]
fn warns_once_when_both_roots_are_set() {
    let rig = Rig::new();
    let a = rig.make_root();
    let b = rig.make_root();
    let out = rig.pty_clean(
        &[("PTY_ROOT", &s(&a)), ("PTY_SESSION_DIR", &s(&b))],
        &["list", "--json"],
    );
    expect_status(&out, 0);
    let err = out.stderr();
    expect_regex(&err, "both PTY_ROOT and PTY_SESSION_DIR are set");
    assert_eq!(count_regex(&err, "both PTY_ROOT and PTY_SESSION_DIR are set"), 1, "{err}");
}

/// node: tests/pty-root.test.ts:247
#[test]
fn legacy_silent_suppresses_the_masking_warning() {
    let rig = Rig::new();
    let a = rig.make_root();
    let b = rig.make_root();
    let out = rig.pty_clean(
        &[
            ("PTY_ROOT", &s(&a)),
            ("PTY_SESSION_DIR", &s(&b)),
            ("PTY_ROOT_LEGACY_SILENT", "1"),
        ],
        &["list", "--json"],
    );
    expect_status(&out, 0);
    expect_not_regex(&out.stderr(), "both PTY_ROOT and PTY_SESSION_DIR");
}

// ── PTY_ROOT length backstop ──

/// node: tests/gc-flap-clear-badge-root-len.test.ts:163
#[test]
fn too_long_root_fails_at_startup() {
    let rig = Rig::new();
    let too_long = format!("/tmp/{}", "a".repeat(95));
    let out = rig.pty_env(&[("PTY_ROOT", &too_long)], &["list"]);
    expect_failure(&out);
    let err = out.stderr();
    expect_regex(&err, "PTY_ROOT is too long");
    expect_regex(&err, "104-byte kernel limit");
    expect_regex(&err, "Shorten the root");
}

/// node: tests/gc-flap-clear-badge-root-len.test.ts:182
#[test]
fn too_long_root_fails_before_dispatch() {
    let rig = Rig::new();
    let too_long = format!("/tmp/{}", "b".repeat(100));
    let out = rig.pty_env(&[("PTY_ROOT", &too_long)], &["definitely-not-a-real-subcommand"]);
    expect_failure(&out);
    let err = out.stderr();
    expect_regex(&err, "PTY_ROOT is too long");
    expect_not_regex(&err, "Unknown command");
}

/// node: tests/gc-flap-clear-badge-root-len.test.ts:197
#[test]
fn root_at_the_usable_threshold_is_allowed() {
    let rig = Rig::new();
    let usable = 104 - (1 + 8 + 5);
    let ok_root = format!("/tmp/{}", "c".repeat(usable - 5));
    assert_eq!(ok_root.len(), usable);
    std::fs::create_dir_all(&ok_root).unwrap();
    let out = rig.pty_env(&[("PTY_ROOT", &ok_root)], &["list", "--json"]);
    let _ = std::fs::remove_dir_all(&ok_root);
    assert_eq!(expect_json(&out), serde_json::json!([]));
}

/// node: tests/gc-flap-clear-badge-root-len.test.ts:217
#[test]
fn root_flag_overrides_a_too_long_env_root() {
    let rig = Rig::new();
    let too_long = format!("/tmp/{}", "d".repeat(95));
    let short = rig.make_root();
    let out = rig.pty_env(
        &[("PTY_ROOT", &too_long)],
        &["--root", &s(&short), "list", "--json"],
    );
    assert_eq!(expect_json(&out), serde_json::json!([]));
}
