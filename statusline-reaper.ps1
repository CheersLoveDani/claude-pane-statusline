# Culls leaked statusline render processes. Claude Code's statusline spawner
# has a race: it can create the node child CREATE_SUSPENDED and die before
# resuming it, leaving a frozen node (~30 MB each, 0 CPU, single Suspended
# thread) roughly every few seconds per pane. 1,083 of them held 33 GB on
# 2026-08-09. A healthy render exits in <1 s, so age >60 s is unambiguous.
# Registered as scheduled task 'StatuslineReaper' (every 5 minutes).
$cutoff = (Get-Date).AddSeconds(-60)
Get-CimInstance Win32_Process -Filter "Name='node.exe'" |
    Where-Object { $_.CommandLine -match 'statusline\.js' -and $_.CreationDate -lt $cutoff } |
    ForEach-Object { try { Stop-Process -Id $_.ProcessId -Force -ErrorAction Stop } catch {} }
