# Claude Code pane statusline — task labels + attention indicator

A statusline for people running several Claude Code panes side by side. Each pane shows:

`● | ⌂ repo | ⎇ branch | ✦ task title | model | ▰▰▱▱▱ ctx% | rate limits`

- **State dot** — green `●` while Claude works, red `❯` when the pane is waiting on you
  (finished turn, permission prompt, or question).
- **Task title** — a Haiku-generated summary of what the session is working on, updated
  after every substantive prompt (billed to the logged-in `claude` subscription, one small
  call per prompt). While regenerating, the `✦` pulses `✧ ✦ ✶ ✻ ✽`.
- **Context gauge** — green → yellow (60%) → red (80%).
- **Rate limits** — 5h/7d windows appear only at ≥75% usage.

Colors are Tokyo Night truecolor; the palette is one table at the top of `statusline.js`.

## Instructions for the installing agent

You are installing three self-contained node scripts and registering them in the user's
**user-level** `~/.claude/settings.json`. Nothing here touches a project.

1. **Check requirements**: `node` and `git` on PATH, the `claude` CLI installed and logged
   in, a terminal with truecolor and powerline glyphs (any Nerd Font, or Windows Terminal
   with Cascadia). If `claude -p` is unavailable the statusline still works — task labels
   just fall back to the built-in session name.

2. **Copy** `statusline.js`, `session-task.js`, and `session-state.js` from this folder
   into `~/.claude/`. They locate each other via `__dirname` — no paths to edit inside them.

3. **Merge** the following into `~/.claude/settings.json` — read the existing file first
   and preserve everything already in it, especially existing `hooks` entries (append to an
   event's array, never replace it). Back it up before editing. Replace `<HOME>` with the
   user's real home directory, absolute, in the platform's own path style:

   ```json
   {
     "statusLine": {
       "type": "command",
       "command": "node \"<HOME>/.claude/statusline.js\"",
       "refreshInterval": 1
     },
     "hooks": {
       "UserPromptSubmit": [
         { "hooks": [
           { "type": "command", "command": "node \"<HOME>/.claude/session-task.js\"", "async": true, "timeout": 90 },
           { "type": "command", "command": "node \"<HOME>/.claude/session-state.js\" working", "async": true, "timeout": 10 }
         ] }
       ],
       "PostToolUse":  [ { "hooks": [ { "type": "command", "command": "node \"<HOME>/.claude/session-state.js\" working", "async": true, "timeout": 10 } ] } ],
       "Stop":         [ { "hooks": [ { "type": "command", "command": "node \"<HOME>/.claude/session-state.js\" waiting", "async": true, "timeout": 10 } ] } ],
       "Notification": [ { "hooks": [ { "type": "command", "command": "node \"<HOME>/.claude/session-state.js\" waiting", "async": true, "timeout": 10 } ] } ],
       "SessionEnd":   [ { "hooks": [ { "type": "command", "command": "node \"<HOME>/.claude/session-state.js\" clear", "async": true, "timeout": 10 } ] } ]
     }
   }
   ```

4. **Verify** before declaring success (all three commands must produce the stated result):
   - Pipe `{"model":{"display_name":"Test"},"workspace":{"current_dir":"<any git repo>"}}`
     into `node ~/.claude/statusline.js` → an ANSI line naming the repo folder and branch.
   - Pipe `{"session_id":"t1","prompt":"add a settings page with theme toggle"}` into
     `node ~/.claude/session-task.js` → within ~15 s, `~/.claude/session-tasks/t1.txt`
     contains a short title (this proves the `claude -p` call works). Delete the test files.
   - Pipe `{"session_id":"t1"}` into `node ~/.claude/session-state.js waiting` →
     `~/.claude/session-tasks/t1.state` contains `waiting`. Delete it.
   - Confirm `settings.json` still parses as JSON.

   Caveat when testing from PowerShell 5.1: its pipes prepend a BOM — the scripts strip it,
   so this works; do not "fix" the `﻿` replace out of the scripts.

5. **Tell the user**: already-open claude sessions need `/hooks` opened once (or a restart)
   to activate the hooks; the statusline itself appears without a restart. New sessions
   need nothing.

## Behavior details worth knowing

- Task files live in `~/.claude/session-tasks/` (`<id>.txt` label, `<id>.history` last 20
  prompts, `<id>.state` working/waiting, `<id>.pending` spinner marker). Files older than
  7 days are pruned automatically.
- Prompts starting with `/` or shorter than 15 characters never change the label.
- The summarizer child sets `RTS_TASK_SUMMARIZER` so its own hooks exit immediately —
  that guard is what prevents infinite hook recursion. Don't remove it.
- Issue chip (optional): if a tool call runs `issue.ps1 claim <n>` (a repo-specific issue
  script), the pane pins that issue — orange `#n` chip, `gh` fetches the title, and Haiku
  titles anchor to it until `issue.ps1 done <n>`. Repos without such a script are simply
  never triggered; adapt the regex in `session-state.js` to your own claim command if wanted.
- `refreshInterval: 1` re-runs the statusline every second (~100 ms of node+git per pane);
  raise it to 2 if that matters on the machine.
