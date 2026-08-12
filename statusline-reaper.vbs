' Windowless launcher for statusline-reaper.ps1 — scheduled task 'StatuslineReaper'
' runs this via wscript.exe so no console window flashes every 5 minutes.
' Finds the .ps1 next to itself, so neither file needs editing after install.
' Third arg True = wait for PowerShell, so the task's 2-min execution limit still applies.
Dim here
here = CreateObject("Scripting.FileSystemObject").GetParentFolderName(WScript.ScriptFullName)
CreateObject("WScript.Shell").Run _
  "powershell.exe -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File """ & here & "\statusline-reaper.ps1""", 0, True
