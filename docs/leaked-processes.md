# Leaked render processes (Windows)

*[← back to README](../README.md)*

**An upstream bug. Nothing here can prevent it — the binary collects after it instead, with
no scheduled task, no PowerShell and no extra process.**

Claude Code's spawner sometimes creates a render child `CREATE_SUSPENDED` and abandons it
before resuming. Caught in the act, a stray has **0 CPU time, one thread, `ThreadState 5`
(Wait) / `ThreadWaitReason 5` (Suspended)** — it never executed a single instruction. No
code inside the process can rescue it, in any language; a native binary is stranded exactly
as a node script was. 1,083 of them accumulated holding 33 GB on 2026-08-09.

I could not reproduce it deliberately. 52 attempts at killing the parent shell mid-spawn
produced zero strays, so the trigger remains unidentified — only the signature is certain.

## How collection works

Something else must kill them, and the cheapest available something else is the next
render. `src/reap.rs` sweeps for abandoned copies of itself:

- **No extra processes** — it runs inside one Claude Code already spawned. The scheduled
  task it replaced cost `wscript` + `powershell` + a conhost (~0.5 s) every 5 minutes
  forever, to usually find nothing.
- **It fires when strays are created**, because strays *are* abandoned renders: no renders,
  no strays, nothing to collect. Trigger and cause are the same event.
- **Collection within a minute** rather than up to five.

Sweeps are rate-limited by a `.reap` stamp file, so the common render pays one small file
read (~10 µs) and only one render a minute pays the ~11 ms process-table walk — about 0.02%
of a core.

## What it will and won't kill

It terminates a process only when **all** of these hold:

- the image name matches
- the full image path on disk is identical to its own, so a same-named binary elsewhere is
  never touched
- it is not itself
- it either has 0 CPU time more than 15 s after starting (never resumed — conclusive) or is
  older than 60 s (ran, but hung)

A healthy render exits in ~10 ms, so neither threshold is close.

`tests/reaper-integration.ps1` proves this against processes created with
`CreateProcess(CREATE_SUSPENDED)` and never resumed — reproducing the bug rather than
simulating it. It checks that an abandoned copy is reaped, one younger than 15 s is spared,
a same-named binary at a different path survives, an unrelated live process survives, and
the sweeping render still emits correct output.

## Two bugs found while building this

- **The rate limit tracked the last sweep by the stamp file's mtime.** `fs::write(&stamp,
  "")` on an already-empty file performs no actual write, so Windows never moves
  `LastWriteTime`. The stamp froze at creation and every render after the first minute
  swept. The timestamp now lives in the file's *contents*.
- **A stamp from the future would have stalled collection indefinitely.** The first version
  honoured it, so a clock moving backwards would disable reaping until wall-clock caught up.
  It now re-stamps and self-heals.

## A separate leak, which was fixable

The original script read stdin with `fs.readFileSync(0)`, which blocks forever if the pipe
never delivers EOF — and no timer can rescue a blocked synchronous call. The renderer now
reads stdin on a thread with a 500 ms deadline and exits explicitly.

That one was ours. The suspended-spawn leak is not.
