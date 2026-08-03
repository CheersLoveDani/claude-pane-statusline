// Hook: tracks whether each session is working or waiting for the user, so the
// statusline (~/.claude/statusline.js) can flag panes that need attention.
// Usage: node session-state.js working|waiting|clear  (session_id from stdin JSON)
// Registered in ~/.claude/settings.json:
//   UserPromptSubmit + PostToolUse -> working (PostToolUse is what turns a pane
//   green again after a permission prompt is approved), Stop + Notification ->
//   waiting, SessionEnd -> clear.
'use strict';
const fs = require('fs');
const path = require('path');

// Headless summarizer runs (session-task.js children) fire hooks too — ignore them.
if (process.env.RTS_TASK_SUMMARIZER) process.exit(0);

const DIR = path.join(__dirname, 'session-tasks');

let input = {};
try { input = JSON.parse(fs.readFileSync(0, 'utf8').replace(/^\uFEFF/, '')); } catch (_) { process.exit(0); }
const id = input.session_id;
if (!id) process.exit(0);

const mode = process.argv[2];
const file = path.join(DIR, id + '.state');
try {
  if (mode === 'clear') {
    try { fs.unlinkSync(file); } catch (_) {}
  } else if (mode === 'working' || mode === 'waiting') {
    fs.mkdirSync(DIR, { recursive: true });
    // Write only on transition — the file's mtime is the stint start time,
    // which the statusline shows as "how long has this pane been working".
    let prev = '';
    try { prev = fs.readFileSync(file, 'utf8').trim(); } catch (_) {}
    if (prev !== mode) fs.writeFileSync(file, mode);
  }
} catch (_) {}

// Issue tracking: a claim via the repo's issue.ps1 pins that issue to the pane
// (statusline chip + summarizer context) until issue.ps1 done clears it.
const cmdStr = (input.tool_input && input.tool_input.command) || '';
// Match only a real invocation — the script path at the start of a statement —
// not any mention of the text (a quoted test payload once pinned an issue to the
// session that was merely testing this feature).
const invoke = verb => new RegExp(
  '(?:^|[;\\r\\n]|&&|\\|\\|)\\s*' +               // start of a statement
  '(?:powershell(?:\\.exe)?\\s+(?:-\\S+\\s+)*|pwsh\\s+(?:-\\S+\\s+)*)?' +
  '(?:&\\s+)?"?(?:[.\\w:~-]*[\\\\/])*issue\\.ps1"?\\s+' + verb + '\\s+(\\d+)', 'i');
const claim = cmdStr.match(invoke('claim'));
const done = cmdStr.match(invoke('done'));
const issueFile = path.join(DIR, id + '.issue');
if (claim) {
  let label = '#' + claim[1];
  try {
    const { spawnSync } = require('child_process');
    const r = spawnSync('gh', ['issue', 'view', claim[1], '--json', 'title', '-q', '.title'],
      { encoding: 'utf8', shell: true, timeout: 15000, cwd: input.cwd || process.cwd() });
    const t = (r.stdout || '').replace(/\s+/g, ' ').trim();
    if (t) label += ' ' + t.slice(0, 120);
  } catch (_) {}
  try {
    fs.mkdirSync(DIR, { recursive: true });
    fs.writeFileSync(issueFile, label);
    fs.appendFileSync(path.join(DIR, id + '.history'), '\n(claimed issue ' + label + ')');
  } catch (_) {}
} else if (done) {
  try { fs.unlinkSync(issueFile); } catch (_) {}
}
process.exit(0);
