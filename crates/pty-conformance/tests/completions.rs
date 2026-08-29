//! Port of tests/completions.test.ts: `pty completions <shell>` prints the
//! generated script (byte-identical to the checked-in `completions/pty.*`
//! in the Node checkout), models `run --env` and
//! `attach --attach-stream-fd-v1 <fd>`, and completes session names from
//! the registry. Shell syntax checks run only where the shell is installed,
//! as in Node. Left out: the spec-vs-COMMAND_HELP parity check (reads the
//! Node source) and the `evidence` leaves (deferred in docs/parity.md §12).

use pty_conformance::*;
use std::path::PathBuf;
use std::process::Command;

fn generate(rig: &Rig, shell: &str) -> String {
    let out = rig.pty(&["completions", shell]);
    expect_status(&out, 0);
    out.stdout()
}

fn which(bin: &str) -> Option<PathBuf> {
    let out = Command::new("sh").arg("-c").arg(format!("command -v {bin}")).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && !s.is_empty() { Some(PathBuf::from(s)) } else { None }
}

fn node_checkout_dir() -> Option<PathBuf> {
    node_checkout()
        .or_else(|| Some(PathBuf::from("/home/myobie/src/github.com/compoundingtech/pty")))
        .filter(|p| p.join("completions").is_dir())
}

/// node: tests/completions.test.ts:81
#[test]
fn matches_every_checked_in_completion_artifact() {
    let Some(checkout) = node_checkout_dir() else {
        eprintln!("skipping: no Node checkout with completions/ (set PTY_NODE_CHECKOUT)");
        return;
    };
    let rig = Rig::new();
    for shell in ["fish", "bash", "zsh"] {
        let checked_in = std::fs::read_to_string(checkout.join("completions").join(format!("pty.{shell}"))).unwrap();
        assert_eq!(generate(&rig, shell), checked_in, "completions/pty.{shell} differs");
    }
}

/// node: tests/completions.test.ts:91
#[test]
fn offers_run_env_in_every_shell() {
    let rig = Rig::new();
    for (shell, marker) in [("fish", "-l env"), ("bash", "--env"), ("zsh", "--env")] {
        expect_contains(&generate(&rig, shell), marker);
    }
}

/// node: tests/completions.test.ts:169
#[test]
fn attach_stream_fd_consumes_a_free_form_value() {
    let rig = Rig::new();
    expect_contains(&generate(&rig, "fish"), "-l attach-stream-fd-v1 -x ");
    expect_contains(&generate(&rig, "bash"), "\"${prev}\" == \"--attach-stream-fd-v1\"");
    expect_regex(&generate(&rig, "zsh"), r"--attach-stream-fd-v1\[[^\]]+\]:fd:");
    let Some(bash) = which("bash") else { return };
    let root = rig.make_dir("fd-root");
    std::fs::write(root.join("target.json"), "{}").unwrap();
    let script = format!(
        "{}\nCOMP_WORDS=(pty attach --attach-stream-fd-v1 3 \"\")\nCOMP_CWORD=4\n_pty\nprintf '%s\\n' \"${{COMPREPLY[@]}}\"",
        generate(&rig, "bash")
    );
    let out = Command::new(bash)
        .arg("-c")
        .arg(&script)
        .env("PTY_ROOT", &root)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "target");
}

/// node: tests/completions.test.ts:196
#[test]
fn prints_fish_bash_zsh_to_stdout() {
    let rig = Rig::new();
    for shell in ["fish", "bash", "zsh"] {
        let out = generate(&rig, shell);
        assert!(out.len() > 50, "{shell}: {out:?}");
        assert!(out.ends_with('\n'), "{shell} output should end with a newline");
    }
}

/// node: tests/completions.test.ts:205
#[test]
fn unknown_shell_prints_usage_and_exits_2() {
    let rig = Rig::new();
    let out = rig.pty(&["completions", "tcsh"]);
    expect_status(&out, 2);
    expect_regex(&out.stderr(), "(?i)unknown shell");
}

/// node: tests/completions.test.ts:213
#[test]
fn help_prints_usage() {
    let rig = Rig::new();
    let out = rig.pty(&["completions", "--help"]);
    expect_status(&out, 0);
    expect_regex(&out.stdout(), "usage: pty completions");
}

fn syntax_check(shell: &str) {
    let Some(bin) = which(shell) else {
        eprintln!("skipping: {shell} is not installed");
        return;
    };
    let rig = Rig::new();
    let script = generate(&rig, shell);
    let out = Command::new(bin).arg("-n").arg("-c").arg(&script).output().unwrap();
    assert!(out.status.success(), "{shell} -n failed:\n{}", String::from_utf8_lossy(&out.stderr));
}

/// node: tests/completions.test.ts:221
#[test]
fn fish_output_is_syntactically_valid() {
    syntax_check("fish");
}

/// node: tests/completions.test.ts:229
#[test]
fn bash_output_is_syntactically_valid() {
    syntax_check("bash");
}

/// node: tests/completions.test.ts:237
#[test]
fn zsh_output_is_syntactically_valid() {
    syntax_check("zsh");
}
