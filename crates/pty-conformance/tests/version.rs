//! Port of tests/version.test.ts — the CLI half. `formatVersion` is a Node
//! unit test with no CLI surface.

use pty_conformance::*;

const FORMS: &[&str] = &["--version", "version", "-v", "-V"];

/// node: tests/version.test.ts:31
/// Node prints `<semver>[+<short-sha>]`. The Rust port prints
/// `<semver>-rust+<short-sha>` (docs/parity.md, "Version string"), so the
/// exact shape is gated per binary; see `version_shape_rust`.
#[test]
fn version_shape_node() {
    if !is_node() {
        return;
    }
    let rig = Rig::new();
    for form in FORMS {
        let out = rig.pty(&[form]);
        expect_status(&out, 0);
        let v = out.stdout().trim().to_string();
        expect_regex(&v, r"^\d+\.\d+\.\d+(\+[0-9a-f]{4,})?$");
    }
}

/// node: tests/version.test.ts:31
/// Rust half of `version_shape_node` (docs/parity.md, "Version string").
#[test]
fn version_shape_rust() {
    if !is_rust() {
        return;
    }
    let rig = Rig::new();
    for form in FORMS {
        let out = rig.pty(&[form]);
        expect_status(&out, 0);
        let v = out.stdout().trim().to_string();
        expect_regex(&v, r"^\d+\.\d+\.\d+-rust(\+[0-9a-f]{4,})?$");
    }
}

/// node: tests/version.test.ts:31
#[test]
fn every_version_form_prints_the_same_string() {
    let rig = Rig::new();
    let reference = rig.pty(&["--version"]);
    expect_status(&reference, 0);
    assert_eq!(reference.stdout().lines().count(), 1, "{}", reference.summary());
    for form in FORMS {
        let out = rig.pty(&[form]);
        expect_status(&out, 0);
        assert_eq!(out.stdout(), reference.stdout(), "`pty {form}` differs");
    }
}

/// node: tests/version.test.ts:42
#[test]
fn version_is_not_an_unknown_command() {
    let rig = Rig::new();
    let out = rig.pty(&["--version"]);
    expect_not_contains(&out.stderr(), "Unknown command");
}
