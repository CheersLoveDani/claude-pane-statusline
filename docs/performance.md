# Performance

*[← back to README](../README.md)*

Claude Code re-runs the statusline **as a fresh process on every update**. A process spawn
is the unit of cost, and the runtime you pay for it dominates everything else — which is
why the renderer is a native binary rather than a script.

Measured against the node implementation this replaced (Windows 11, 5 panes, which ran with
`refreshInterval: 1`). CPU figures are whole process **trees**, measured with a Windows Job
Object so the second bash, `git` and every other child is counted — per-process timing
misses them, which is how a chain looks cheap while being expensive:

| per render | node | native |
|---|---|---|
| Wall clock | 366 ms | **9.7 ms** |
| CPU (whole tree) | 309.4 ms | **135.9 ms** |
| Processes | 4 | **3** |
| Working set | 47 MB | **4.3 MB** |

**The number that matters is not in the table: `bash -c "exit 0"` costs 140.6 ms of CPU and
2 processes.** That is what Claude Code pays to run *any* command on Windows. A render now
costs less than the empty wrapper around it, so the renderer's own cost has vanished into
the noise and there is nothing further to win.

Two changes got it there. `git` is gone — the branch is read from `.git/HEAD` instead of
shelling out, which alone was ~93 ms and a whole process per render. And **the clock was
deleted**: nothing in this line changes without an event except the summarizer spinner, so
the spinner was the sole reason `refreshInterval` had to exist, and that one-second timer
re-ran the entire chain on every pane forever, busy or idle, to animate a glyph on screen
for ~15 s per prompt. The spinner now advances **one frame per redraw**. An active pane
already renders 0.25–2×/sec from events, and a pane is busy precisely when it is
summarising, so the star rides redraws that were happening anyway — and sparse redraws read
as deliberate motion rather than the random frame-skipping a wall-clock index gives you.

## What it costs to run

Against a Claude Code with no statusline configured, which spawns none of this:

| | CPU | processes | when |
|---|---|---|---|
| Render | 137.5 ms | 3 | per update (~21/min across 5 busy panes; **zero** when idle) |
| State hook | 129.7 ms | 3 | per tool call |
| Summarizer | **8.8 s** | 116 | per *substantive* prompt |
| *(reference: `bash -c "exit 0"`)* | *140.6 ms* | *2* | *what Claude Code pays for any command* |

Renders and the state hook sit at the wrapper floor, so **the summarizer is the whole cost
of this project** — it is a real model call, ~60× everything else combined, and easy to miss
because it runs async and nothing waits on it. Three things keep it in proportion:

- It fires **once per substantive prompt**, not per turn and not per tool call. Prompts
  under 15 characters and anything starting with `/` are skipped, so "yes", "do it" and
  slash commands cost nothing.
- **You pay only while it generates.** Afterwards the title is a line of text; every later
  render reads it in microseconds, so a pane displaying a label for an hour costs nothing.
  Verified: `claude` and `node` process counts return exactly to baseline after a run, with
  nothing left resident.
- It starts its child with **`--mcp-config "" --strict-mcp-config`**. Without those flags
  `claude -p` boots your full MCP server set for a one-shot summarization that uses no
  tools: 28.1 s of CPU and 158 processes, against 8.8 s and 116 with them off.

(That 116 is every process across the ~13 s call, not concurrent ones — about 18 are alive
at any instant.)

If you want this to cost *nothing*, drop the task labels; renders and hooks are already
free.

## Two traps

Both of these look like wins and are not. Recorded so the next person doesn't spend the
afternoon rediscovering them.

- **`--bare`** would cut the summarizer to 1.3 s and 21 processes, but forces auth to
  `ANTHROPIC_API_KEY` alone — no OAuth, no keychain — so on a subscription it exits 1 with
  "Not logged in". Only useful if you have an API key and want to pay for titles separately.
- **`CLAUDE_CODE_GIT_BASH_PATH`** pointed at `Git\usr\bin\bash.exe` looks like it halves the
  duplicate `bash`, and it appears equivalent when tested from inside a Git Bash shell —
  only because the environment is inherited. Launched clean it has no `MSYSTEM`, resolves
  `git` to the `/cmd` wrapper, and has neither `grep` nor `tr` on `PATH`. It breaks the Bash
  tool.

Also tried and not worth it: stripping plugins, hooks and skills from the summarizer child
via `--settings` and `--disable-slash-commands` moved 8.2 s → 7.2 s (noise) with the process
count pinned at 116 either way. MCP was the only real win.

## Measuring this yourself

Per-process CPU is misleading here because it excludes children, and wall clock is
misleading on a busy machine. Both mistakes were made while arriving at the numbers above.
Use a Job Object: create one, assign the process to it before it spawns anything, and read
`JOBOBJECT_BASIC_ACCOUNTING_INFORMATION` after exit for `TotalUserTime + TotalKernelTime`
across the whole tree.

Note that its `TotalProcesses` counts every process that *ever ran* in the job, not the
concurrent peak — which is why the summarizer reads as 116 processes but only has ~18 alive
at once.
