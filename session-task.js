// UserPromptSubmit hook: distils the latest substantive prompt into a short task
// title (via `claude -p --model haiku`, billed to the logged-in subscription) so
// the statusline (~/.claude/statusline.exe) can label each pane with its current task.
// This is the one piece still in node: it drives a `claude -p` child either way,
// so a JS runtime is not the dominant cost, and it runs once per prompt rather
// than once per render.
// Configured in ~/.claude/settings.json -> hooks.UserPromptSubmit (async).
'use strict';
const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawnSync } = require('child_process');

// The summarizer run is itself a claude session — without this guard it would
// fire this hook again, forever.
if (process.env.RTS_TASK_SUMMARIZER) process.exit(0);

const DIR = path.join(__dirname, 'session-tasks');
const MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;

let input = {};
try { input = JSON.parse(fs.readFileSync(0, 'utf8').replace(/^\uFEFF/, '')); } catch (_) { process.exit(0); }

const id = input.session_id;
if (!id) process.exit(0);

// --refresh regenerates the title from existing history without a new prompt
// (used by `statusline.exe state` when a session claims an issue mid-turn).
const refresh = process.argv[2] === '--refresh';

// Keep a per-session history so the title reflects the whole session, not just
// the latest message. Capped at the most recent 20 prompts.
let history = [];
const histFile = path.join(DIR, id + '.history');
try { history = fs.readFileSync(histFile, 'utf8').split('\n').filter(Boolean); } catch (_) {}

if (!refresh) {
  const prompt = (input.prompt || '').replace(/\s+/g, ' ').trim();
  // Skip slash commands and short continuations ("yes", "do it") — they would
  // clobber a good label without saying what the pane is working on.
  if (prompt.startsWith('/') || prompt.length < 15) process.exit(0);
  history.push(prompt.slice(0, 400));
  history = history.slice(-20);
  try {
    fs.mkdirSync(DIR, { recursive: true });
    fs.writeFileSync(histFile, history.join('\n'));
  } catch (_) {}
}

// If the session has claimed a tracker issue, anchor the title to it.
let issue = '';
try { issue = fs.readFileSync(path.join(DIR, id + '.issue'), 'utf8').trim(); } catch (_) {}
if (!history.length && !issue) process.exit(0); // nothing to summarize

const instruction =
  'Write a task title of at most 6 words for a coding session. ' +
  'Reply with the title only - no quotes, no trailing punctuation.\n' +
  (issue
    ? 'The session has claimed tracker issue ' + issue + ' - the title must describe that issue\'s work.\n'
    : '') +
  'User prompts, oldest first (recent ones matter most):\n' +
  history.map((p, i) => (i + 1) + '. ' + p).join('\n');

// The .pending marker makes the statusline show a spinner while we summarize;
// the statusline ignores markers older than 90 s in case this process dies here.
const pendingFile = path.join(DIR, id + '.pending');
try { fs.writeFileSync(pendingFile, ''); } catch (_) {}

// cwd is tmpdir so the child session does not load this project's CLAUDE.md.
// No `shell: true`: it bought nothing but an extra cmd.exe and its conhost on
// every summarize, and libuv resolves `claude` -> claude.exe from PATH anyway.
//
// `--mcp-config "" --strict-mcp-config` starts the child with no MCP servers.
// A headless one-shot summarize needs no tools, but without this it boots the
// full set from the user's config: measured 28.1 s of CPU and 158 processes per
// prompt, against 8.8 s and 116 with them off. This is by far the most expensive
// thing in the project - a render costs ~0.14 s - so it is worth the two flags.
//
// `--bare` would cut it to 1.3 s and 21 processes, but it forces auth to
// ANTHROPIC_API_KEY only (no OAuth, no keychain) and so fails outright with
// "Not logged in" on a subscription. Don't reach for it unless you have an API
// key and are willing to pay for titles separately.
const r = spawnSync('claude', ['-p', '--model', 'haiku', '--mcp-config', '', '--strict-mcp-config'], {
  input: instruction, encoding: 'utf8', timeout: 60000, cwd: os.tmpdir(),
  env: Object.assign({}, process.env, { RTS_TASK_SUMMARIZER: '1' }),
});
try { fs.unlinkSync(pendingFile); } catch (_) {}

let title = (r.stdout || '').replace(/\s+/g, ' ').trim().replace(/^["']|["'.]$/g, '');
if (!title || title.length > 200) process.exit(0); // keep the previous label on failure
// Fit the statusline's 60-char segment: trim overruns at a word boundary.
if (title.length > 60) title = title.slice(0, 60).replace(/\s+\S*$/, '');

try {
  fs.mkdirSync(DIR, { recursive: true });
  fs.writeFileSync(path.join(DIR, id + '.txt'), title);
  // Opportunistic prune so dead sessions don't accumulate files forever.
  const now = Date.now();
  for (const f of fs.readdirSync(DIR)) {
    const p = path.join(DIR, f);
    try { if (now - fs.statSync(p).mtimeMs > MAX_AGE_MS) fs.unlinkSync(p); } catch (_) {}
  }
} catch (_) {}
process.exit(0);
