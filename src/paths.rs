//! Filesystem lookups shared by the render and the state hook.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// `session-tasks/` sits next to the executable, matching the `__dirname`
/// resolution the JS used — session-task.js still writes into the same
/// directory, so the two must agree.
pub fn session_tasks_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("session-tasks")
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn mtime_ms(p: &Path) -> Option<i64> {
    std::fs::metadata(p)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

pub fn read_trimmed(p: &Path) -> Option<String> {
    let s = std::fs::read_to_string(p).ok()?;
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// Current branch, read straight off the refs instead of spawning `git`.
///
/// This replaces `git -C <dir> rev-parse --abbrev-ref HEAD`, which cost ~93 ms
/// and a whole process (plus a conhost, and on Git for Windows a stray
/// cygwin-console-helper) on every single render. Walking up for `.git`
/// reproduces git's own repo discovery.
pub fn git_branch(dir: &Path) -> Option<String> {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        let dot_git = d.join(".git");
        if dot_git.is_dir() {
            return head_branch(&dot_git);
        }
        if dot_git.is_file() {
            // Worktrees and submodules use a `.git` file pointing elsewhere —
            // agents here work in worktrees, so this path is the common one.
            let s = std::fs::read_to_string(&dot_git).ok()?;
            let target = s.trim().strip_prefix("gitdir:")?.trim();
            let target = PathBuf::from(target);
            let resolved = if target.is_absolute() { target } else { d.join(target) };
            return head_branch(&resolved);
        }
        cur = d.parent();
    }
    None
}

fn head_branch(git_dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref:") {
        // Keep the full ref tail: `refs/heads/feature/foo` is branch
        // `feature/foo`, which is what --abbrev-ref reports.
        Some(r) => {
            let r = r.trim();
            Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_string())
        }
        // Detached HEAD — `git rev-parse --abbrev-ref HEAD` prints "HEAD".
        None if !head.is_empty() => Some("HEAD".to_string()),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cps-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reads_branch_from_a_plain_repo() {
        let d = tmpdir("plain");
        fs::create_dir_all(d.join(".git")).unwrap();
        fs::write(d.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        assert_eq!(git_branch(&d).as_deref(), Some("main"));
    }

    #[test]
    fn keeps_slashes_in_branch_names() {
        let d = tmpdir("slashes");
        fs::create_dir_all(d.join(".git")).unwrap();
        fs::write(d.join(".git/HEAD"), "ref: refs/heads/feature/nested/thing\n").unwrap();
        assert_eq!(git_branch(&d).as_deref(), Some("feature/nested/thing"));
    }

    #[test]
    fn discovers_the_repo_from_a_subdirectory() {
        let d = tmpdir("subdir");
        fs::create_dir_all(d.join(".git")).unwrap();
        fs::write(d.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let sub = d.join("a/b/c");
        fs::create_dir_all(&sub).unwrap();
        assert_eq!(git_branch(&sub).as_deref(), Some("main"));
    }

    #[test]
    fn follows_a_worktree_gitdir_file() {
        let d = tmpdir("worktree");
        let real = d.join("realgit");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("HEAD"), "ref: refs/heads/wt-branch\n").unwrap();
        let wt = d.join("wt");
        fs::create_dir_all(&wt).unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}\n", real.display())).unwrap();
        assert_eq!(git_branch(&wt).as_deref(), Some("wt-branch"));
    }

    #[test]
    fn detached_head_reports_head_like_git_does() {
        let d = tmpdir("detached");
        fs::create_dir_all(d.join(".git")).unwrap();
        fs::write(d.join(".git/HEAD"), "9fceb02aaf1e4b0b0c0d0e0f1a2b3c4d5e6f7a8b\n").unwrap();
        assert_eq!(git_branch(&d).as_deref(), Some("HEAD"));
    }

    #[test]
    fn no_repo_yields_nothing() {
        let d = tmpdir("norepo");
        assert_eq!(git_branch(&d), None);
    }
}
