//! What a spawner settles before a daemon starts: the command path.
//!
//! node: src/spawn.ts:372-393

use std::path::{Path, PathBuf};

/// Node's `resolveCommand`: an absolute path must exist; a path with a `/`
/// is resolved against the current directory and must exist; a bare name is
/// looked up on `PATH` the way `which` does (first regular file with an
/// execute bit). Errors are `Command not found: <cmd>`.
///
/// node: src/spawn.ts:372-393
pub fn resolve_command(cmd: &str) -> Result<String, String> {
    let not_found = || format!("Command not found: {cmd}");
    let path = Path::new(cmd);
    if path.is_absolute() {
        return if path.exists() {
            Ok(cmd.to_string())
        } else {
            Err(not_found())
        };
    }
    if cmd.contains('/') {
        let resolved = std::env::current_dir()
            .map(|cwd| normalize(&cwd.join(path)))
            .map_err(|_| not_found())?;
        return if resolved.exists() {
            Ok(resolved.to_string_lossy().into_owned())
        } else {
            Err(not_found())
        };
    }
    which(cmd).ok_or_else(not_found)
}

/// `path.resolve`: collapse `.` and `..` lexically (no symlink resolution).
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The first executable regular file named `cmd` on `PATH`.
fn which(cmd: &str) -> Option<String> {
    if cmd.is_empty() {
        return None;
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = if dir.as_os_str().is_empty() {
            PathBuf::from(cmd)
        } else {
            dir.join(cmd)
        };
        if is_executable_file(&candidate) {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

fn is_executable_file(p: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(p) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).ok();
    match c {
        // SAFETY: `access` only reads the path.
        Some(c) => unsafe { libc::access(c.as_ptr(), libc::X_OK) == 0 },
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_paths_must_exist() {
        assert_eq!(resolve_command("/bin/sh").unwrap(), "/bin/sh");
        assert_eq!(
            resolve_command("/definitely/not/here").unwrap_err(),
            "Command not found: /definitely/not/here"
        );
    }

    #[test]
    fn bare_names_come_from_path() {
        let sh = resolve_command("sh").unwrap();
        assert!(sh.ends_with("/sh"), "{sh}");
        assert_eq!(
            resolve_command("no-such-command-xyz").unwrap_err(),
            "Command not found: no-such-command-xyz"
        );
    }

    #[test]
    fn relative_paths_resolve_against_cwd() {
        let cwd = std::env::current_dir().unwrap();
        let dir = cwd.join("target-rc-test-dir");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tool");
        std::fs::write(&file, "").unwrap();
        let rel = "./target-rc-test-dir/../target-rc-test-dir/tool";
        assert_eq!(
            resolve_command(rel).unwrap(),
            file.to_string_lossy().into_owned()
        );
        let _ = std::fs::remove_dir_all(&dir);
        assert!(resolve_command(rel).is_err());
    }
}
