// Claude Code statusline: powerline-style, Tokyo Night palette.
// Segments: dir -> branch -> session task name -> model -> context% -> rate limit (when high)
// Reads the session JSON Claude Code pipes on stdin, prints one ANSI line.
// Configured in ~/.claude/settings.json -> statusLine.command
// Note: sessions here are launched from the primary checkout and agents work in
// worktrees via absolute paths, so the dir chip is constant — the session name
// is what identifies a pane.
'use strict';
const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

// stdin is read ASYNC with a deadline, and the process exits explicitly.
// It used to be fs.readFileSync(0), which blocks forever if the parent never
// sends EOF on the pipe — a repro'd hang no timer can rescue (sync block).
// Separately, Claude Code sometimes abandons a render process it created
// SUSPENDED (never runs a line of JS) — 1,083 of those held 33 GB on
// 2026-08-09; statusline-reaper.ps1 (scheduled task StatuslineReaper) culls
// them, since a healthy render exits in well under a second.
let raw = '';
let finished = false;
function finish() {
  if (finished) return;
  finished = true;
  let input = {};
  // The BOM strip lets the script be tested from PowerShell 5.1, whose pipes prepend a BOM.
  try { input = JSON.parse(raw.replace(/^\uFEFF/, '')); } catch (_) {}
  render(input);
}
process.stdin.setEncoding('utf8');
process.stdin.on('data', d => {
  raw += d;
  // Claude Code writes the JSON in one shot — parse eagerly and skip the deadline.
  try { JSON.parse(raw.replace(/^\uFEFF/, '')); finish(); } catch (_) {}
});
process.stdin.on('end', finish);
process.stdin.on('error', finish);
setTimeout(finish, 500); // deadline: render with whatever arrived, never hang

function render(input) {
  const dir = (input.workspace && input.workspace.current_dir) || input.cwd || process.cwd();
  const model = (input.model && input.model.display_name) || '';
  const name = path.basename(dir);
  // Prefer the live task recorded by the UserPromptSubmit hook (~/.claude/session-task.js)
  // over session_name, which is generated once and goes stale as the task moves on.
  let task = '';
  try {
    task = fs.readFileSync(path.join(__dirname, 'session-tasks', input.session_id + '.txt'), 'utf8').trim();
  } catch (_) {}
  const sessionName = task || input.session_name || '';

  // Spinner while the summarizer hook is working (fresh .pending marker).
  let spinner = '';
  try {
    const pend = fs.statSync(path.join(__dirname, 'session-tasks', input.session_id + '.pending'));
    if (Date.now() - pend.mtimeMs < 90000) {
      // A star that grows and shrinks back into the resting ✦ glyph.
      // (Deliberately avoids ✳/✴ — those have emoji presentations and can render
      // as double-width colour glyphs, which breaks the segment background.)
      const frames = ['✧', '✦', '✶', '✻', '✽', '✻', '✶', '✦'];
      spinner = frames[Math.floor(Date.now() / 1000) % frames.length];
    }
  } catch (_) {}
  const ctx = input.context_window && input.context_window.used_percentage;
  const limits = input.rate_limits || {};

  let branch = '';
  try {
    branch = execFileSync(
      'git', ['-C', dir, 'rev-parse', '--abbrev-ref', 'HEAD'],
      { stdio: ['ignore', 'pipe', 'ignore'], timeout: 1500 }
    ).toString().trim();
  } catch (_) { /* not a git repo, or git unavailable */ }

  // Tokyo Night
  const C = {
    bgDark: '#1a1b26', storm: '#24283b', hi: '#3b4261',
    fg: '#c0caf5', dim: '#565f89',
    blue: '#7aa2f7', cyan: '#7dcfff', magenta: '#bb9af7',
    green: '#9ece6a', yellow: '#e0af68', red: '#f7768e', orange: '#ff9e64',
  };
  const rgb = h => [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16)];
  const FG = h => '\x1b[38;2;' + rgb(h).join(';') + 'm';
  const BG = h => '\x1b[48;2;' + rgb(h).join(';') + 'm';
  const SEP = '';      // powerline arrow
  const THIN = '';     // outline chevron, for same-bg neighbours

  // working/waiting state written by the session-state.js hooks; the file's
  // mtime is when the current stint began.
  let state = '';
  let stateSince = 0;
  try {
    const stateFile = path.join(__dirname, 'session-tasks', input.session_id + '.state');
    state = fs.readFileSync(stateFile, 'utf8').trim();
    stateSince = fs.statSync(stateFile).mtimeMs;
  } catch (_) {}

  const segs = [];
  if (state === 'waiting') {
    segs.push({ text: '❯', fg: C.red, bg: C.storm, bold: true });
  } else if (state === 'working') {
    const mins = Math.floor((Date.now() - stateSince) / 60000);
    const dur = mins < 1 ? '' : mins < 60 ? ' ' + mins + 'm'
      : ' ' + Math.floor(mins / 60) + 'h' + String(mins % 60).padStart(2, '0');
    segs.push({ text: '●' + dur, fg: C.green, bg: C.storm });
  }
  if (branch) {
    segs.push({ text: '⌂ ' + name, fg: C.bgDark, bg: C.blue, bold: true });
  } else {
    segs.push({ text: '⌂ ' + name, fg: C.fg, bg: C.hi, bold: true });
  }
  // new_agent.ps1 worktree folders embed the branch name — showing both overflows a split pane
  if (branch && !name.toLowerCase().includes(branch.toLowerCase())) {
    segs.push({ text: ' ' + branch, fg: C.cyan, bg: C.hi });
  }
  // Claimed-issue chip (written by session-state.js when issue.ps1 claim runs).
  let issueNum = '';
  try {
    const m = fs.readFileSync(path.join(__dirname, 'session-tasks', input.session_id + '.issue'), 'utf8').match(/^#\d+/);
    if (m) issueNum = m[0];
  } catch (_) {}
  if (issueNum) segs.push({ text: issueNum, fg: C.orange, bg: C.storm });

  if (sessionName || spinner) {
    const t = sessionName.length > 60 ? sessionName.slice(0, 59) + '…' : sessionName;
    segs.push({ text: (spinner || '✦') + ' ' + (t || 'summarising'), fg: C.magenta, bg: C.storm });
  }
  if (model) segs.push({ text: model, fg: C.dim, bg: C.bgDark });
  if (typeof ctx === 'number') {
    const filled = Math.min(5, Math.round(ctx / 20));
    const bar = '▰'.repeat(filled) + '▱'.repeat(5 - filled);
    const col = ctx >= 80 ? C.red : ctx >= 60 ? C.yellow : C.green;
    segs.push({ text: bar + ' ' + ctx + '%', fg: col, bg: C.bgDark });
  }
  // Rate limits are shared across every pane — surface them only once they matter.
  for (const [label, w] of [['5h', limits.five_hour], ['7d', limits.seven_day]]) {
    if (w && typeof w.used_percentage === 'number' && w.used_percentage >= 75) {
      const col = w.used_percentage >= 90 ? C.red : C.orange;
      segs.push({ text: '⧖ ' + label + ' ' + w.used_percentage + '%', fg: col, bg: C.bgDark });
    }
  }

  let out = '';
  for (let i = 0; i < segs.length; i++) {
    const s = segs[i];
    out += BG(s.bg) + (s.bold ? '\x1b[1m' : '') + FG(s.fg) + ' ' + s.text + ' \x1b[22m';
    const next = segs[i + 1];
    if (next && next.bg === s.bg) {
      out += FG(C.dim) + THIN;
    } else {
      out += (next ? BG(next.bg) : '\x1b[49m') + FG(s.bg) + SEP;
    }
  }
  out += '\x1b[0m';
  // Exit from the write callback — stdout to a pipe is async on Windows, and a
  // bare process.exit() here can truncate the line. The timer is the backstop
  // for a callback that never fires (dead pipe); either way the process ends.
  process.stdout.write(out, () => process.exit(0));
  setTimeout(() => process.exit(0), 500);
}
