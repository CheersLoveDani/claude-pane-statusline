//! The working/waiting state hook, plus claimed-issue tracking.
//!
//! Ported from session-state.js. This is the hook that fires on *every*
//! PostToolUse, so it has to be cheap: the common case is "already working",
//! which touches nothing and exits.

use crate::json::Json;
use crate::paths;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn run(mode: &str, input: &Json) {
    // Headless summarizer runs (session-task.js children) fire hooks too — ignore them.
    if std::env::var_os("RTS_TASK_SUMMARIZER").is_some() {
        return;
    }
    let Some(id) = input.get("session_id").as_str() else {
        return;
    };
    if id.is_empty() {
        return;
    }

    let dir = paths::session_tasks_dir();
    let state_file = dir.join(format!("{id}.state"));

    if mode == "clear" {
        let _ = std::fs::remove_file(&state_file);
    } else if mode == "working" || mode == "waiting" {
        let _ = std::fs::create_dir_all(&dir);
        // Write only on transition — the file's mtime is the stint start time,
        // which the statusline shows as "how long has this pane been working".
        let prev = paths::read_trimmed(&state_file);
        if prev.as_deref() != Some(mode) {
            let _ = std::fs::write(&state_file, mode);
        }
    }

    // Issue tracking: a claim via the repo's issue.ps1 pins that issue to the
    // pane (statusline chip + summarizer context) until issue.ps1 done clears it.
    let cmd = input
        .get("tool_input")
        .get("command")
        .as_str()
        .unwrap_or("");
    if cmd.is_empty() {
        return;
    }
    let issue_file = dir.join(format!("{id}.issue"));

    if let Some(num) = issue_invocation(cmd, "claim") {
        let mut label = format!("#{num}");
        let cwd = input.get("cwd").as_str().map(|s| s.to_string());
        if let Some(title) = gh_issue_title(num, cwd.as_deref()) {
            label.push(' ');
            label.push_str(&truncate_chars(&title, 120));
        }
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(&issue_file, &label);
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(format!("{id}.history")))
        {
            let _ = writeln!(f, "\n(claimed issue {label})");
        }
        // Regenerate the title now — in a "pick an issue and action it" session
        // the claim happens mid-turn and there may be no further prompt to
        // trigger it.
        refresh_session_title(id);
    } else if issue_invocation(cmd, "done").is_some() {
        let _ = std::fs::remove_file(&issue_file);
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn gh_issue_title(num: u32, cwd: Option<&str>) -> Option<String> {
    let mut c = Command::new("gh");
    c.args(["issue", "view", &num.to_string(), "--json", "title", "-q", ".title"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    let out = wait_with_timeout(c, Duration::from_secs(15))?;
    let t = String::from_utf8_lossy(&out).split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

/// session-task.js still owns summarising (it drives `claude -p`), so the claim
/// path shells out to it. This is rare — once per claimed issue, not per render.
fn refresh_session_title(id: &str) {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let Some(dir) = exe_dir else { return };
    let script = dir.join("session-task.js");
    if !script.exists() {
        return;
    }
    let Ok(mut child) = Command::new("node")
        .arg(&script)
        .arg("--refresh")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    if let Some(mut si) = child.stdin.take() {
        let _ = write!(si, r#"{{"session_id":"{id}"}}"#);
    }
    // Detached on purpose: the hook must not block a tool call for 90 s waiting
    // on a model round-trip.
}

/// Run a command, killing it if it outruns `limit`. Returns stdout on success.
fn wait_with_timeout(mut cmd: Command, limit: Duration) -> Option<Vec<u8>> {
    let mut child = cmd.spawn().ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() > limit {
                    let _ = child.kill();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
    let out = child.wait_with_output().ok()?;
    Some(out.stdout)
}

/// Match a real `issue.ps1 <verb> <n>` invocation at statement position.
///
/// Not any mention of the text: a quoted test payload once pinned an issue to
/// the session that was merely testing this feature. Statement boundaries are
/// `;`, newlines, `&&` and `||`, matching the JS regex this replaces.
pub fn issue_invocation(cmd: &str, verb: &str) -> Option<u32> {
    for stmt in split_statements(cmd) {
        if let Some(n) = match_invocation(stmt, verb) {
            return Some(n);
        }
    }
    None
}

fn split_statements(cmd: &str) -> Vec<&str> {
    let b = cmd.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        let width = match b[i] {
            b';' | b'\r' | b'\n' => 1,
            b'&' if b.get(i + 1) == Some(&b'&') => 2,
            b'|' if b.get(i + 1) == Some(&b'|') => 2,
            _ => 0,
        };
        if width > 0 {
            out.push(&cmd[start..i]);
            i += width;
            start = i;
        } else {
            i += 1;
        }
    }
    out.push(&cmd[start..]);
    out
}

fn match_invocation(stmt: &str, verb: &str) -> Option<u32> {
    let s = stmt.trim_start();

    // Optional shell prefix: powershell[.exe] / pwsh, then any -flags.
    let s = strip_shell_prefix(s);
    // Optional PowerShell call operator: `& path\to\issue.ps1`.
    let s = match s.strip_prefix('&') {
        Some(rest) if rest.starts_with(char::is_whitespace) => rest.trim_start(),
        _ => s,
    };
    let s = s.strip_prefix('"').unwrap_or(s);
    // Optional path prefix: (?:[.\w:~-]*[\\/])*
    let s = strip_path_prefix(s);

    let s = strip_ci_prefix(s, "issue.ps1")?;
    let s = s.strip_prefix('"').unwrap_or(s);

    if !s.starts_with(char::is_whitespace) {
        return None;
    }
    let s = s.trim_start();
    let s = strip_ci_prefix(s, verb)?;
    if !s.starts_with(char::is_whitespace) {
        return None;
    }
    let s = s.trim_start();
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

fn strip_ci_prefix<'a>(s: &'a str, lit: &str) -> Option<&'a str> {
    let n = lit.len();
    // Byte-wise compare with a boundary check: commands are arbitrary user text
    // and may hold multi-byte characters right where the literal would end.
    if s.len() >= n && s.is_char_boundary(n) && s.as_bytes()[..n].eq_ignore_ascii_case(lit.as_bytes())
    {
        Some(&s[n..])
    } else {
        None
    }
}

fn strip_shell_prefix(s: &str) -> &str {
    let rest = strip_ci_prefix(s, "powershell.exe")
        .or_else(|| strip_ci_prefix(s, "powershell"))
        .or_else(|| strip_ci_prefix(s, "pwsh"));
    let Some(rest) = rest else { return s };
    if !rest.starts_with(char::is_whitespace) {
        return s;
    }
    let mut cur = rest.trim_start();
    // (?:-\S+\s+)* — flags before the script path.
    while cur.starts_with('-') {
        let end = cur.find(char::is_whitespace).unwrap_or(cur.len());
        let after = &cur[end..];
        if after.is_empty() {
            return s; // a trailing flag with no script after it isn't an invocation
        }
        cur = after.trim_start();
    }
    cur
}

fn strip_path_prefix(s: &str) -> &str {
    let b = s.as_bytes();
    // Last separator that is preceded only by path-ish characters.
    let mut last_sep = None;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'\\' | b'/' => last_sep = Some(i),
            c if is_path_char(c) => {}
            _ => break,
        }
    }
    match last_sep {
        Some(i) => &s[i + 1..],
        None => s,
    }
}

fn is_path_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b':' | b'~' | b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_a_bare_invocation() {
        assert_eq!(issue_invocation("issue.ps1 claim 42", "claim"), Some(42));
    }

    #[test]
    fn matches_common_real_world_spellings() {
        for cmd in [
            r"tools\agent\issue.ps1 claim 7",
            r".\tools\agent\issue.ps1 claim 7",
            r"powershell -File tools\agent\issue.ps1 claim 7",
            r"powershell.exe -NoProfile -File .\issue.ps1 claim 7",
            r"pwsh -File C:\repo\tools\issue.ps1 claim 7",
            r"& .\tools\agent\issue.ps1 claim 7",
            r#"& "C:\repo\tools\agent\issue.ps1" claim 7"#,
            r"cd foo; issue.ps1 claim 7",
            "git status && issue.ps1 claim 7",
            "git status || issue.ps1 claim 7",
            "echo hi\nissue.ps1 claim 7",
            r"ISSUE.PS1 CLAIM 7",
        ] {
            assert_eq!(issue_invocation(cmd, "claim"), Some(7), "should match: {cmd}");
        }
    }

    #[test]
    fn ignores_mentions_inside_a_quoted_payload() {
        // The regression this replaces: a quoted test payload pinned an issue to
        // the session that was merely testing the feature.
        for cmd in [
            r#"gh issue create --body "run issue.ps1 claim 42 to take it""#,
            r#"echo "issue.ps1 claim 42""#,
            r#"git commit -m "document issue.ps1 claim 42""#,
        ] {
            assert_eq!(issue_invocation(cmd, "claim"), None, "should not match: {cmd}");
        }
    }

    #[test]
    fn requires_the_verb_and_a_number() {
        assert_eq!(issue_invocation("issue.ps1 claim", "claim"), None);
        assert_eq!(issue_invocation("issue.ps1 claim abc", "claim"), None);
        assert_eq!(issue_invocation("issue.ps1 claiming 42", "claim"), None);
        assert_eq!(issue_invocation("issue.ps1claim 42", "claim"), None);
    }

    #[test]
    fn claim_and_done_do_not_cross_match() {
        assert_eq!(issue_invocation("issue.ps1 done 9", "claim"), None);
        assert_eq!(issue_invocation("issue.ps1 done 9", "done"), Some(9));
        assert_eq!(issue_invocation("issue.ps1 claim 9", "done"), None);
    }

    #[test]
    fn a_similarly_named_script_does_not_match() {
        assert_eq!(issue_invocation("my-issue.ps1x claim 3", "claim"), None);
        // The script name has to start the statement (or follow a separator);
        // `issue.ps1` buried inside a longer word is not an invocation.
        assert_eq!(issue_invocation("reissue.ps1 claim 3", "claim"), None);
        assert_eq!(issue_invocation("notes-about-issue.ps1 claim 3", "claim"), None);
    }

    #[test]
    fn multibyte_commands_do_not_panic() {
        assert_eq!(issue_invocation("échoing something", "claim"), None);
        assert_eq!(issue_invocation("issue.ps1 claim 5 # café ✦", "claim"), Some(5));
    }

    #[test]
    fn statement_splitting_finds_the_boundaries() {
        assert_eq!(split_statements("a;b"), vec!["a", "b"]);
        assert_eq!(split_statements("a&&b"), vec!["a", "b"]);
        assert_eq!(split_statements("a||b"), vec!["a", "b"]);
        assert_eq!(split_statements("a\r\nb"), vec!["a", "", "b"]);
        // A single & is the call operator, not a boundary.
        assert_eq!(split_statements("a&b"), vec!["a&b"]);
    }
}
