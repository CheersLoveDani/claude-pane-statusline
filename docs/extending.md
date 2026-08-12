# Extending it

*[← back to README](../README.md)*

The whole system communicates through plain files in `~/.claude/session-tasks/`, keyed by
session id — **that directory is the API**. Anything that writes these files drives the
statusline, and anything can read them:

| File | Contents | Effect |
|---|---|---|
| `<id>.txt` | one line of text | The `✦` label, shown verbatim. Write it directly to bypass Haiku entirely. |
| `<id>.issue` | e.g. `#123 fix the flaky test` | Pins context: the leading `#123` becomes the orange chip, and the full text anchors every future Haiku title until deleted. |
| `<id>.history` | one prompt/context line per row | What Haiku summarizes. Append lines to feed extra context into future titles. |
| `<id>.state` | `working` or `waiting` | The green dot / red caret. Its mtime is the stint start, so touch it only on transitions. |
| `<id>.pending` | empty marker | Shows the spinner while present (ignored after 90 s). Its **mtime is the staleness anchor** — never rewrite it to signal liveness, recreate it. |
| `<id>.spin` | a digit 0–7 | Spinner frame, advanced by the renderer each redraw. Written only while `.pending` is fresh; delete it freely. |
| `.reap` | epoch-ms timestamp (not per-session) | Last stray sweep. Delete it to force a sweep on the next render. |

Files older than 7 days are pruned automatically, so extensions need no cleanup logic.

**Getting the session id:** every Claude Code hook receives it as `session_id` in the JSON
on stdin — it is *not* in the environment of commands the agent runs. So the natural place
to extend is another hook.

## Adding your own trigger

The issue chip is just a worked example: a `PostToolUse` hook watching for
`issue.ps1 claim <n>` (see `issue_invocation` in `src/state.rs`). Because the API is plain
files, your trigger need not live in this binary or be written in Rust. In node, for
instance:

```js
// inside a PostToolUse hook script; `input` is the stdin JSON
const m = ((input.tool_input || {}).command || '').match(/^gh issue develop (\d+)/);
if (m) {
  fs.writeFileSync(path.join(DIR, input.session_id + '.issue'), '#' + m[1]);
  spawnSync('node', [path.join(__dirname, 'session-task.js'), '--refresh'],
    { input: JSON.stringify({ session_id: input.session_id }), encoding: 'utf8', timeout: 90000 });
}
```

Two hard-won rules for custom triggers:

1. **Match invocations, not mentions.** Anchor your pattern to the start of a command or a
   statement separator. An unanchored match fires on the string appearing inside a quoted
   payload, a grep, or a doc edit — ours once pinned an issue to the session that was merely
   *testing* the feature.
2. **Guard against recursion** if your extension spawns `claude`. Set
   `RTS_TASK_SUMMARIZER=1` (or your own sentinel, checked at the top of every hook) in the
   child's environment, or the child's hooks fire your hook again, forever.

**Regenerating a title on demand:** pipe `{"session_id":"<id>"}` into
`node ~/.claude/session-task.js --refresh`. It rebuilds the label from `.history` +
`.issue` without adding anything — this is what the claim trigger uses, and the right call
after your extension changes either file.

## Reskinning

Colors are Tokyo Night truecolor, defined as one block of constants at the top of
`src/render.rs`. Segment assembly is the `segs` vector in the same file: each entry is text
plus a foreground and background, and the powerline joins are worked out afterwards, so
adding or reordering a segment is a one-line change.
