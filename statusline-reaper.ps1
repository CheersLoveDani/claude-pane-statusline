# Culls leaked helper processes that accumulate around Claude Code sessions.
# Registered as scheduled task 'StatuslineReaper' (every 5 minutes) via
# statusline-reaper.vbs, so no console window flashes.
#
# This is a backstop, not a fix. Claude Code sometimes creates a render process
# CREATE_SUSPENDED and abandons it before resuming — the process never executes a
# single instruction, so no code inside it (JS, Rust, anything) can rescue it.
# Confirmed signature on a caught stray: 0 CPU time, one thread, ThreadState 5
# (Wait) / ThreadWaitReason 5 (Suspended). 1,083 of them held 33 GB on
# 2026-08-09, before this script existed.
#
# Since the port to a native binary each stray costs ~4 MB rather than ~47 MB,
# and there are far fewer of them, but they still need collecting.
#
# 1) Statusline renders: a healthy render exits in ~10 ms, so *either* signal is
#    conclusive — never-ran (0 CPU) past a short grace period, or simply old.
# 2) Git Bash console helpers: a killed bash.exe strands its hidden-console
#    cygwin-console-helper.exe, each pinning a conhost.exe (~14 MB the pair).
#    162 pairs held 2.2 GB the same day. A live helper's parent (bash) is alive,
#    so parent-dead + age >60 s is unambiguous; killing the helper takes its
#    conhost with it.

$now = Get-Date
$neverRanCutoff = $now.AddSeconds(-15)   # 0 CPU this long after start = never resumed
$staleCutoff    = $now.AddSeconds(-60)   # ran, but outlived any plausible render

$all = Get-CimInstance Win32_Process
$livePids = @{}
$all | ForEach-Object { $livePids[$_.ProcessId] = $true }

$all | Where-Object {
    # statusline.exe is the current renderer; statusline.js covers node-era
    # strays still lingering from before the port.
    $isRender = ($_.Name -eq 'statusline.exe') -or
                ($_.Name -eq 'node.exe' -and $_.CommandLine -match 'statusline\.js')
    $isHelper = ($_.Name -eq 'cygwin-console-helper.exe' -and -not $livePids.ContainsKey($_.ParentProcessId))

    ($isRender -and (
        ($_.CreationDate -lt $neverRanCutoff -and ($_.KernelModeTime + $_.UserModeTime) -eq 0) -or
        ($_.CreationDate -lt $staleCutoff)
    )) -or
    ($isHelper -and $_.CreationDate -lt $staleCutoff)
} | ForEach-Object { try { Stop-Process -Id $_.ProcessId -Force -ErrorAction Stop } catch {} }
