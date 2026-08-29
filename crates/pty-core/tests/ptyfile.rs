//! Port of the pty project's `tests/ptyfile.test.ts`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pty_core::ptyfile::{command_with_env_exports, read_pty_file, PtySessionDef};

fn make_dir(name: &str, content: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pty-ptyfile-{}-{}-{}", name, std::process::id(), n));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("pty.toml"), content).unwrap();
    dir
}

fn def(command: &str, env: Option<&[(&str, &str)]>) -> PtySessionDef {
    PtySessionDef {
        display_name: "x".into(),
        short_name: "x".into(),
        id: None,
        command: command.into(),
        cwd: None,
        tags: None,
        env: env.map(|pairs| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>()
        }),
    }
}

// ── env ──

#[test]
fn parses_env_as_string_map() {
    let dir = make_dir(
        "envok",
        "[sessions.worker]\ncommand = \"cat\"\n\n[sessions.worker.env]\nFOO = \"bar\"\nSECOND = \"two\"\n",
    );
    let file = read_pty_file(Some(&dir)).unwrap();
    assert_eq!(file.sessions.len(), 1);
    let env = file.sessions[0].env.clone().unwrap();
    assert_eq!(env.get("FOO").map(String::as_str), Some("bar"));
    assert_eq!(env.get("SECOND").map(String::as_str), Some("two"));
    assert_eq!(env.len(), 2);
}

#[test]
fn omits_env_when_absent() {
    let dir = make_dir("noenv", "[sessions.plain]\ncommand = \"cat\"\n");
    assert!(read_pty_file(Some(&dir)).unwrap().sessions[0].env.is_none());
}

#[test]
fn accepts_empty_env_table() {
    let dir = make_dir(
        "emptyenv",
        "[sessions.empty]\ncommand = \"cat\"\n\n[sessions.empty.env]\n",
    );
    let env = read_pty_file(Some(&dir)).unwrap().sessions[0].env.clone().unwrap();
    assert!(env.is_empty());
}

#[test]
fn rejects_non_string_env_values() {
    let dir = make_dir(
        "badenv",
        "[sessions.bad]\ncommand = \"cat\"\n\n[sessions.bad.env]\nNOT_A_STRING = 42\n",
    );
    let err = read_pty_file(Some(&dir)).unwrap_err();
    assert!(err.contains("env.NOT_A_STRING must be a string"), "{err}");
}

// ── cwd ──

#[test]
fn omits_cwd_when_absent() {
    let dir = make_dir("nocwd", "[sessions.plain]\ncommand = \"cat\"\n");
    assert!(read_pty_file(Some(&dir)).unwrap().sessions[0].cwd.is_none());
}

#[test]
fn keeps_absolute_cwd() {
    let dir = make_dir("abscwd", "[sessions.svc]\ncommand = \"cat\"\ncwd = \"/opt/app\"\n");
    assert_eq!(
        read_pty_file(Some(&dir)).unwrap().sessions[0].cwd.as_deref(),
        Some("/opt/app")
    );
}

#[test]
fn resolves_relative_cwd_against_manifest_dir() {
    let sub = make_dir("relcwd", "[sessions.svc]\ncommand = \"cat\"\ncwd = \"..\"\n");
    let expected = Path::new(&sub).parent().unwrap().to_string_lossy().into_owned();
    assert_eq!(
        read_pty_file(Some(&sub)).unwrap().sessions[0].cwd.as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn resolves_dot_to_manifest_dir() {
    let dir = make_dir("dotcwd", "[sessions.svc]\ncommand = \"cat\"\ncwd = \".\"\n");
    let expected = dir.to_string_lossy().into_owned();
    assert_eq!(
        read_pty_file(Some(&dir)).unwrap().sessions[0].cwd.as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn rejects_non_string_cwd() {
    let dir = make_dir("badcwd", "[sessions.bad]\ncommand = \"cat\"\ncwd = 42\n");
    let err = read_pty_file(Some(&dir)).unwrap_err();
    assert!(err.contains("\"cwd\" must be a non-empty string"), "{err}");
}

#[test]
fn rejects_empty_cwd() {
    let dir = make_dir("emptycwd", "[sessions.bad]\ncommand = \"cat\"\ncwd = \"\"\n");
    let err = read_pty_file(Some(&dir)).unwrap_err();
    assert!(err.contains("\"cwd\" must be a non-empty string"), "{err}");
}

// ── command_with_env_exports ──

#[test]
fn bare_command_when_env_absent() {
    assert_eq!(command_with_env_exports(&def("echo hi", None)), "echo hi");
}

#[test]
fn bare_command_when_env_empty() {
    assert_eq!(command_with_env_exports(&def("echo hi", Some(&[]))), "echo hi");
}

#[test]
fn prepends_export_statements() {
    assert_eq!(
        command_with_env_exports(&def("echo $FOO", Some(&[("FOO", "bar")]))),
        "export FOO='bar'; echo $FOO"
    );
}

#[test]
fn one_export_per_entry() {
    let out = command_with_env_exports(&def("do-thing", Some(&[("A", "1"), ("B", "two")])));
    assert!(out.contains("export A='1'"));
    assert!(out.contains("export B='two'"));
    assert!(out.ends_with("; do-thing"));
}

#[test]
fn escapes_single_quotes() {
    assert_eq!(
        command_with_env_exports(&def("echo $MSG", Some(&[("MSG", "it's a value")]))),
        "export MSG='it'\\''s a value'; echo $MSG"
    );
}

#[test]
fn handles_shell_metacharacters_safely() {
    assert_eq!(
        command_with_env_exports(&def("go", Some(&[("PATH_LIKE", "$HOME/bin:/usr/bin; echo pwned")]))),
        "export PATH_LIKE='$HOME/bin:/usr/bin; echo pwned'; go"
    );
}
