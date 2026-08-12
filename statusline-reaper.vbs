' Windowless launcher for statusline-reaper.ps1 — scheduled task 'StatuslineReaper'
' runs this via wscript.exe so no console window flashes every 5 minutes.
' Third arg True = wait for PowerShell, so the task's 2-min execution limit still applies.
CreateObject("WScript.Shell").Run "powershell.exe -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File ""C:\Users\kingm\.claude\statusline-reaper.ps1""", 0, True
