//! The statusline itself: powerline-style, Tokyo Night palette.
//! Segments: state -> dir -> branch -> issue -> session task -> model -> context% -> rate limit.

use crate::json::Json;
use crate::paths;
use std::path::Path;

// Tokyo Night
const BG_DARK: &str = "#1a1b26";
const STORM: &str = "#24283b";
const HI: &str = "#3b4261";
const FG: &str = "#c0caf5";
const DIM: &str = "#565f89";
const BLUE: &str = "#7aa2f7";
const CYAN: &str = "#7dcfff";
const MAGENTA: &str = "#bb9af7";
const GREEN: &str = "#9ece6a";
const YELLOW: &str = "#e0af68";
const RED: &str = "#f7768e";
const ORANGE: &str = "#ff9e64";

const SEP: &str = "\u{e0b0}"; // powerline arrow
const THIN: &str = "\u{e0b1}"; // outline chevron, for same-bg neighbours

struct Seg {
    text: String,
    fg: &'static str,
    bg: &'static str,
    bold: bool,
}

fn rgb(hex: &str) -> (u8, u8, u8) {
    let b = hex.as_bytes();
    let h = |i: usize| {
        u8::from_str_radix(std::str::from_utf8(&b[i..i + 2]).unwrap_or("0"), 16).unwrap_or(0)
    };
    (h(1), h(3), h(5))
}

fn fg(hex: &str) -> String {
    let (r, g, b) = rgb(hex);
    format!("\x1b[38;2;{r};{g};{b}m")
}

fn bg(hex: &str) -> String {
    let (r, g, b) = rgb(hex);
    format!("\x1b[48;2;{r};{g};{b}m")
}

/// Percentages arrive as JSON numbers; print 42 rather than 42.0, but keep a
/// real fraction if the payload has one.
fn pct(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

pub fn render(input: &Json) -> String {
    let dir = input
        .get("workspace")
        .get("current_dir")
        .as_str()
        .or_else(|| input.get("cwd").as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        });
    let dir_path = Path::new(&dir);
    let name = dir_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.clone());
    let model = input.get("model").get("display_name").as_str().unwrap_or("");
    let session_id = input.get("session_id").as_str().unwrap_or("");

    let tasks = paths::session_tasks_dir();
    let per_session = |ext: &str| tasks.join(format!("{session_id}{ext}"));

    // Prefer the live task recorded by the UserPromptSubmit hook over
    // session_name, which is generated once and goes stale as the task moves on.
    let task = if session_id.is_empty() {
        None
    } else {
        paths::read_trimmed(&per_session(".txt"))
    };
    let session_name = task
        .or_else(|| input.get("session_name").as_str().map(|s| s.to_string()))
        .unwrap_or_default();

    let now = paths::now_ms();

    // Spinner while the summarizer hook is working (fresh .pending marker).
    //
    // The frame is indexed by *render count*, not by wall-clock time. Indexing by
    // time is what forced `refreshInterval` into settings.json: a clock-driven
    // glyph only animates if something re-runs the command every second, so every
    // pane re-spawned the whole render chain once a second, forever, idle or not,
    // to move a star that is on screen for ~15 s per prompt.
    //
    // Renders already happen 0.25-2x/sec from events alone while a pane is busy —
    // and a pane is busy exactly when it is summarising. Stepping one frame per
    // render rides those for free: the star always advances, so sparse renders
    // read as deliberate motion instead of the random frame-skipping a clock index
    // gives you when redraws are irregular.
    let mut spinner = "";
    let summarising = !session_id.is_empty()
        && paths::mtime_ms(&per_session(".pending")).is_some_and(|m| now - m < 90_000);
    if summarising {
        // A star that grows and shrinks back into the resting ✦ glyph.
        // (Deliberately avoids ✳/✴ — those have emoji presentations and can render
        // as double-width colour glyphs, which breaks the segment background.)
        const FRAMES: [&str; 8] = ["✧", "✦", "✶", "✻", "✽", "✻", "✶", "✦"];
        // The counter lives beside .pending, never in it: .pending's mtime is the
        // staleness anchor, and rewriting it every render would keep it forever
        // fresh and strand the spinner if the summarizer died.
        let spin_file = per_session(".spin");
        let frame = paths::read_trimmed(&spin_file)
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0)
            % FRAMES.len();
        spinner = FRAMES[frame];
        let _ = std::fs::write(&spin_file, ((frame + 1) % FRAMES.len()).to_string());
    }

    let branch = paths::git_branch(dir_path);

    // working/waiting state written by the state hook; the file's mtime is when
    // the current stint began.
    let (state, state_since) = if session_id.is_empty() {
        (None, 0)
    } else {
        let f = per_session(".state");
        (paths::read_trimmed(&f), paths::mtime_ms(&f).unwrap_or(0))
    };

    let mut segs: Vec<Seg> = Vec::new();

    match state.as_deref() {
        Some("waiting") => segs.push(Seg {
            text: "❯".into(),
            fg: RED,
            bg: STORM,
            bold: true,
        }),
        Some("working") => {
            let mins = (now - state_since) / 60_000;
            let dur = if mins < 1 {
                String::new()
            } else if mins < 60 {
                format!(" {mins}m")
            } else {
                format!(" {}h{:02}", mins / 60, mins % 60)
            };
            segs.push(Seg {
                text: format!("●{dur}"),
                fg: GREEN,
                bg: STORM,
                bold: false,
            });
        }
        _ => {}
    }

    segs.push(Seg {
        text: format!("⌂ {name}"),
        fg: if branch.is_some() { BG_DARK } else { FG },
        bg: if branch.is_some() { BLUE } else { HI },
        bold: true,
    });

    // new_agent.ps1 worktree folders embed the branch name — showing both
    // overflows a split pane.
    if let Some(b) = &branch {
        if !name.to_lowercase().contains(&b.to_lowercase()) {
            segs.push(Seg {
                text: format!(" {b}"),
                fg: CYAN,
                bg: HI,
                bold: false,
            });
        }
    }

    // Claimed-issue chip (written by the state hook when issue.ps1 claim runs).
    if !session_id.is_empty() {
        if let Some(raw) = paths::read_trimmed(&per_session(".issue")) {
            if let Some(num) = leading_issue_number(&raw) {
                segs.push(Seg {
                    text: num,
                    fg: ORANGE,
                    bg: STORM,
                    bold: false,
                });
            }
        }
    }

    if !session_name.is_empty() || summarising {
        let t = truncate_chars(&session_name, 60);
        let glyph = if spinner.is_empty() { "✦" } else { spinner };
        let label = if t.is_empty() { "summarising".to_string() } else { t };
        segs.push(Seg {
            text: format!("{glyph} {label}"),
            fg: MAGENTA,
            bg: STORM,
            bold: false,
        });
    }

    if !model.is_empty() {
        segs.push(Seg {
            text: model.to_string(),
            fg: DIM,
            bg: BG_DARK,
            bold: false,
        });
    }

    if let Some(ctx) = input.get("context_window").get("used_percentage").as_f64() {
        let filled = ((ctx / 20.0).round() as i64).clamp(0, 5) as usize;
        let bar = format!("{}{}", "▰".repeat(filled), "▱".repeat(5 - filled));
        let col = if ctx >= 80.0 {
            RED
        } else if ctx >= 60.0 {
            YELLOW
        } else {
            GREEN
        };
        segs.push(Seg {
            text: format!("{bar} {}%", pct(ctx)),
            fg: col,
            bg: BG_DARK,
            bold: false,
        });
    }

    // Rate limits are shared across every pane — surface them only once they matter.
    let limits = input.get("rate_limits");
    for (label, key) in [("5h", "five_hour"), ("7d", "seven_day")] {
        if let Some(used) = limits.get(key).get("used_percentage").as_f64() {
            if used >= 75.0 {
                segs.push(Seg {
                    text: format!("⧖ {label} {}%", pct(used)),
                    fg: if used >= 90.0 { RED } else { ORANGE },
                    bg: BG_DARK,
                    bold: false,
                });
            }
        }
    }

    let mut out = String::new();
    for i in 0..segs.len() {
        let s = &segs[i];
        out.push_str(&bg(s.bg));
        if s.bold {
            out.push_str("\x1b[1m");
        }
        out.push_str(&fg(s.fg));
        out.push(' ');
        out.push_str(&s.text);
        out.push_str(" \x1b[22m");
        match segs.get(i + 1) {
            Some(next) if next.bg == s.bg => {
                out.push_str(&fg(DIM));
                out.push_str(THIN);
            }
            Some(next) => {
                out.push_str(&bg(next.bg));
                out.push_str(&fg(s.bg));
                out.push_str(SEP);
            }
            None => {
                out.push_str("\x1b[49m");
                out.push_str(&fg(s.bg));
                out.push_str(SEP);
            }
        }
    }
    out.push_str("\x1b[0m");
    out
}

/// The `.issue` file holds `#123 Some title`; only the number goes in the chip.
fn leading_issue_number(raw: &str) -> Option<String> {
    let rest = raw.strip_prefix('#')?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        None
    } else {
        Some(format!("#{digits}"))
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let mut t: String = s.chars().take(max - 1).collect();
        t.push('…');
        t
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c2 in chars.by_ref() {
                    if c2 == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    fn render_str(payload: &str) -> String {
        render(&json::parse(payload).unwrap())
    }

    #[test]
    fn shows_dir_model_and_context_bar() {
        let out = render_str(
            r#"{"workspace":{"current_dir":"E:\\dev\\widget"},
                "model":{"display_name":"Opus 5"},
                "context_window":{"used_percentage":42}}"#,
        );
        let plain = strip_ansi(&out);
        assert!(plain.contains("⌂ widget"), "{plain}");
        assert!(plain.contains("Opus 5"), "{plain}");
        assert!(plain.contains("▰▰▱▱▱ 42%"), "{plain}");
    }

    #[test]
    fn context_bar_fills_and_recolours_at_thresholds() {
        assert!(strip_ansi(&render_str(r#"{"context_window":{"used_percentage":100}}"#))
            .contains("▰▰▰▰▰ 100%"));
        assert!(strip_ansi(&render_str(r#"{"context_window":{"used_percentage":0}}"#))
            .contains("▱▱▱▱▱ 0%"));
        // 80%+ is the red band; check the colour actually changes.
        let hot = render_str(r#"{"context_window":{"used_percentage":85}}"#);
        assert!(hot.contains(&fg(RED)));
        let warm = render_str(r#"{"context_window":{"used_percentage":65}}"#);
        assert!(warm.contains(&fg(YELLOW)));
    }

    #[test]
    fn rate_limits_appear_only_above_the_threshold() {
        let quiet = strip_ansi(&render_str(
            r#"{"rate_limits":{"five_hour":{"used_percentage":50}}}"#,
        ));
        assert!(!quiet.contains("5h"), "{quiet}");
        let loud = strip_ansi(&render_str(
            r#"{"rate_limits":{"five_hour":{"used_percentage":80},"seven_day":{"used_percentage":91}}}"#,
        ));
        assert!(loud.contains("⧖ 5h 80%"), "{loud}");
        assert!(loud.contains("⧖ 7d 91%"), "{loud}");
    }

    #[test]
    fn session_name_is_used_when_no_task_file_exists() {
        let out = strip_ansi(&render_str(
            r#"{"session_id":"no-such-session-id","session_name":"Refactor the parser"}"#,
        ));
        assert!(out.contains("✦ Refactor the parser"), "{out}");
    }

    #[test]
    fn long_titles_are_ellipsised_to_the_segment_width() {
        let long = "x".repeat(80);
        let out = strip_ansi(&render_str(&format!(
            r#"{{"session_id":"none","session_name":"{long}"}}"#
        )));
        assert!(out.contains(&format!("{}…", "x".repeat(59))), "{out}");
    }

    #[test]
    fn empty_input_still_renders_a_line() {
        let out = render(&json::parse("{}").unwrap());
        assert!(out.contains("⌂ "), "{out}");
        assert!(out.ends_with("\x1b[0m"));
    }

    #[test]
    fn issue_chip_takes_only_the_number() {
        assert_eq!(leading_issue_number("#123 Fix the thing").as_deref(), Some("#123"));
        assert_eq!(leading_issue_number("no number here"), None);
        assert_eq!(leading_issue_number("#"), None);
    }

    #[test]
    fn percentages_print_without_a_trailing_zero() {
        assert_eq!(pct(42.0), "42");
        assert_eq!(pct(42.5), "42.5");
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        // Multi-byte characters must not be split, and must not count double.
        let s = "é".repeat(70);
        let t = truncate_chars(&s, 60);
        assert_eq!(t.chars().count(), 60);
        assert!(t.ends_with('…'));
        assert_eq!(truncate_chars("short", 60), "short");
    }
}
