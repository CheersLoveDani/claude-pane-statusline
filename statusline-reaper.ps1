# Culls leaked helper processes that accumulate around Claude Code sessions.
# Registered as scheduled task 'StatuslineReaper' (every 5 minutes).
#
# 1) Statusline renders: Claude Code's spawner has a race where it creates the
#    node child CREATE_SUSPENDED and dies before resuming it, leaving a frozen
#    node (~30 MB, 0 CPU, one Suspended thread) every few seconds per pane.
#    1,083 of them held 33 GB on 2026-08-09. A healthy render exits in <1 s,
#    so age >60 s is unambiguous.
# 2) Git Bash console helpers: a killed bash.exe strands its hidden-console
#    cygwin-console-helper.exe, each pinning a conhost.exe (~14 MB the pair).
#    162 pairs held 2.2 GB the same day. A live helper's parent (bash) is
#    alive, so parent-dead + age >60 s is unambiguous; killing the helper
#    takes its conhost with it.
$cutoff = (Get-Date).AddSeconds(-60)
$all = Get-CimInstance Win32_Process
$livePids = @{}
$all | ForEach-Object { $livePids[$_.ProcessId] = $true }

$all | Where-Object {
    $_.CreationDate -lt $cutoff -and (
        ($_.Name -eq 'node.exe' -and $_.CommandLine -match 'statusline\.js') -or
        ($_.Name -eq 'cygwin-console-helper.exe' -and -not $livePids.ContainsKey($_.ParentProcessId))
    )
} | ForEach-Object { try { Stop-Process -Id $_.ProcessId -Force -ErrorAction Stop } catch {} }
