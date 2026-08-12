# Integration proof for the stray collector in src/reap.rs.
#
# The unit tests cover the rate limit; they cannot cover the part that matters,
# which is behaviour against a REAL process created CREATE_SUSPENDED and never
# resumed - the exact condition observed in the wild (0 CPU, one suspended
# thread, never executed an instruction). This creates those processes for real.
#
#   .\tests\reaper-integration.ps1 [-Exe <path to statusline.exe>]
#
# Takes ~90s: the never-resumed threshold is 15s and several cases must age past it.
param(
    [string]$Exe = "$PSScriptRoot\..\target\release\statusline.exe"
)

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public class SuspendedLauncher {
  [StructLayout(LayoutKind.Sequential)] public struct PROCESS_INFORMATION {
    public IntPtr hProcess; public IntPtr hThread; public uint dwProcessId; public uint dwThreadId; }
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct STARTUPINFO {
    public uint cb; public string lpReserved; public string lpDesktop; public string lpTitle;
    public uint dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
    public ushort wShowWindow; public ushort cbReserved2; public IntPtr lpReserved2;
    public IntPtr hStdInput, hStdOutput, hStdError; }
  [DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
  public static extern bool CreateProcess(string app, string cmd, IntPtr pa, IntPtr ta,
    bool inherit, uint flags, IntPtr env, string cwd, ref STARTUPINFO si, out PROCESS_INFORMATION pi);
  // Deliberately never calls ResumeThread. That IS the upstream bug.
  public static uint StartSuspended(string exe) {
    var si = new STARTUPINFO(); si.cb = (uint)Marshal.SizeOf(si);
    PROCESS_INFORMATION pi;
    bool ok = CreateProcess(exe, null, IntPtr.Zero, IntPtr.Zero, false,
                            0x4 /*CREATE_SUSPENDED*/ | 0x08000000 /*CREATE_NO_WINDOW*/,
                            IntPtr.Zero, null, ref si, out pi);
    if (!ok) throw new Exception("CreateProcess failed: " + Marshal.GetLastWin32Error());
    return pi.dwProcessId;
  }
}
'@

$Exe = (Resolve-Path $Exe).Path
$stamp = Join-Path (Split-Path $Exe) 'session-tasks\.reap'
$pass = 0; $fail = 0

function Check($name, $want, $got) {
    if ($want -eq $got) { Write-Host "  PASS  $name"; $script:pass++ }
    else { Write-Host "  FAIL  $name (want $want, got $got)"; $script:fail++ }
}
function Alive($p) { $null -ne (Get-Process -Id $p -ErrorAction SilentlyContinue) }
function CpuOf($p) {
    $ci = Get-CimInstance Win32_Process -Filter "ProcessId=$p" -ErrorAction SilentlyContinue
    if ($ci) { ($ci.KernelModeTime + $ci.UserModeTime) / 1e7 } else { $null }
}
function Sweep {
    # clear the stamp so the sweep is not rate limited, then let a render do it
    if (Test-Path $stamp) { [System.IO.File]::Delete($stamp) }
    '{}' | & $Exe | Out-Null
    Start-Sleep -Milliseconds 800
}
# NB: not named Kill - that is a built-in alias for Stop-Process, and aliases win
# over functions, so the name would silently bypass this wrapper's error handling.
function StopIfAlive($p) { try { Stop-Process -Id $p -Force -ErrorAction Stop } catch {} }

Write-Host "testing: $Exe`n"

Write-Host "=== an abandoned CREATE_SUSPENDED render ==="
$victim = [SuspendedLauncher]::StartSuspended($Exe)
Start-Sleep -Milliseconds 500
Check "it exists"                 $true (Alive $victim)
Check "it has 0 CPU (never ran)"  0     (CpuOf $victim)
Sweep
Check "younger than 15s: spared"  $true (Alive $victim)
Start-Sleep -Seconds 16
Sweep
Check "older than 15s: reaped"    $false (Alive $victim)
StopIfAlive $victim

Write-Host "=== rate limiting ==="
$v2 = [SuspendedLauncher]::StartSuspended($Exe)
Start-Sleep -Seconds 16
'{}' | & $Exe | Out-Null          # stamp is fresh from the sweep above
Start-Sleep -Milliseconds 500
Check "fresh stamp suppresses the sweep" $true (Alive $v2)
Sweep
Check "expired stamp collects it"        $false (Alive $v2)
StopIfAlive $v2

Write-Host "=== safety: only abandoned copies of THIS image are touched ==="
# NB: do not use notepad.exe as the bystander. Modern Notepad hands a new launch
# to an existing instance and the launcher exits on its own, which reads as a
# false failure. Use a process that certainly stays alive.
$sleeper = Start-Process powershell.exe -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 40' -PassThru -WindowStyle Hidden
Start-Sleep -Milliseconds 800
Sweep
Check "unrelated live process survives" $true (Alive $sleeper.Id)
StopIfAlive $sleeper.Id

$altDir = Join-Path $env:TEMP 'statusline-reaper-imposter'
if (-not (Test-Path $altDir)) { New-Item -ItemType Directory -Force $altDir | Out-Null }
Copy-Item $Exe (Join-Path $altDir 'statusline.exe') -Force
$imposter = [SuspendedLauncher]::StartSuspended((Join-Path $altDir 'statusline.exe'))
Start-Sleep -Seconds 17
Sweep
Check "same name, different image path: spared" $true (Alive $imposter)
StopIfAlive $imposter
Remove-Item $altDir -Recurse -Force -ErrorAction SilentlyContinue

Write-Host "=== the sweeping render still does its actual job ==="
if (Test-Path $stamp) { [System.IO.File]::Delete($stamp) }
$out = '{"model":{"display_name":"Opus 5"}}' | & $Exe
Check "output intact while sweeping" $true ($out -match 'Opus 5')

Write-Host "`npass=$pass fail=$fail"
if ($fail -gt 0) { exit 1 }
