# Measure a global command hotkey to the command having run, from the daemon's
# own log.
#
# Like measure-gesture.ps1, this cannot run in CI (docs/ARCHITECTURE.md, 7.24).
# The daemon times itself: it takes an Instant when WM_HOTKEY arrives and logs
# `hotkey to action done: N ms` once the bound command has run. This script
# only binds a chord, presses it the same way every time and reads the numbers
# back.
#
# The boundary is WM_HOTKEY's arrival, so the few milliseconds between the key
# and WM_HOTKEY are not in the figure -- the daemon cannot see the key before
# then, and the summon budget excludes them the same way.
#
# The measured command is `window:center`, against a window this script opens
# and moves off-centre before every press, so each press really moves it. The
# daemon runs under a temporary LUMEN_HOME, beside any installed Lumen, which is
# left alone; only the processes started here are stopped.
#
#   powershell -File scripts/measure-hotkey.ps1 -Exe target/release/lumen.exe

param(
    [Parameter(Mandatory)] [string] $Exe,
    [int] $Presses = 15,
    [int] $SettleSeconds = 10
)

$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path $Exe).Path
$home_ = Join-Path ([IO.Path]::GetTempPath()) "lumen-hotkey-$PID"
$log = Join-Path ([IO.Path]::GetTempPath()) "lumen-hotkey-$PID.log"
$title = "lumen-hotkey-target-$PID"

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class HotkeyUser32 {
    [DllImport("user32.dll")] public static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int cmd);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(
        IntPtr hwnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern void keybd_event(
        byte vk, byte scan, uint flags, UIntPtr extra);
}
'@

$VK_SHIFT = 0x10; $VK_CONTROL = 0x11; $VK_MENU = 0x12; $VK_F11 = 0x7A
$KEYUP = 0x2

function Key($vk, $flags) {
    [HotkeyUser32]::keybd_event([byte]$vk, 0, [uint32]$flags, [UIntPtr]::Zero)
}

function Press-Chord {
    Key $VK_CONTROL 0; Key $VK_MENU 0; Key $VK_SHIFT 0
    Key $VK_F11 0; Key $VK_F11 $KEYUP
    Key $VK_SHIFT $KEYUP; Key $VK_MENU $KEYUP; Key $VK_CONTROL $KEYUP
}

function Percentile($sorted, $p) {
    $sorted[[math]::Min($sorted.Count - 1, [math]::Ceiling($p * $sorted.Count) - 1)]
}

New-Item -ItemType Directory -Force (Join-Path $home_ 'config') | Out-Null
@'
{ "commands": { "window:center": { "hotkey": "Ctrl+Alt+Shift+F11" } } }
'@ | Set-Content -Encoding ascii (Join-Path $home_ 'config\shortcuts.json')

$previous = $env:LUMEN_HOME
$daemon = $null
$target = $null
try {
    # Start-Process inherits this process's environment.
    $env:LUMEN_HOME = $home_
    $daemon = Start-Process -FilePath $exe -PassThru -RedirectStandardError $log -WindowStyle Hidden
    if ($null -eq $previous) { Remove-Item Env:LUMEN_HOME } else { $env:LUMEN_HOME = $previous }

    # A window this script owns, rather than Notepad: on Windows 11 starting
    # Notepad can open a tab in a Notepad window the user already has.
    $form = "Add-Type -AssemblyName System.Windows.Forms; " +
        "`$f = New-Object Windows.Forms.Form; `$f.Text = '$title'; " +
        "`$f.StartPosition = 'Manual'; `$f.Width = 480; `$f.Height = 320; " +
        "[Windows.Forms.Application]::Run(`$f)"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($form))
    $target = Start-Process powershell -PassThru -WindowStyle Hidden `
        -ArgumentList '-NoProfile', '-EncodedCommand', $encoded

    Start-Sleep -Seconds $SettleSeconds

    # [NullString]: PowerShell would pass $null to a string parameter as "".
    $hwnd = [HotkeyUser32]::FindWindow([NullString]::Value, $title)
    if ($hwnd -eq [IntPtr]::Zero) { throw "target window '$title' did not appear" }
    [HotkeyUser32]::ShowWindow($hwnd, 9) | Out-Null # SW_RESTORE

    for ($i = 0; $i -lt $Presses; $i++) {
        # Top-left, off-centre, so centring it is real work every time.
        [HotkeyUser32]::SetWindowPos($hwnd, [IntPtr]::Zero, 40 + $i, 40, 0, 0, 0x0001 -bor 0x0004) |
            Out-Null # SWP_NOSIZE | SWP_NOZORDER
        [HotkeyUser32]::SetForegroundWindow($hwnd) | Out-Null
        Start-Sleep -Milliseconds 200
        if ([HotkeyUser32]::GetForegroundWindow() -ne $hwnd) {
            Write-Warning "press $($i + 1): the target window is not in the foreground"
        }
        Press-Chord
        Start-Sleep -Milliseconds 600
    }
    Start-Sleep -Milliseconds 500
}
finally {
    if ($null -eq $previous) { Remove-Item Env:LUMEN_HOME -ErrorAction SilentlyContinue }
    else { $env:LUMEN_HOME = $previous }
    if ($daemon) { Stop-Process -Id $daemon.Id -Force -ErrorAction SilentlyContinue }
    if ($target) { Stop-Process -Id $target.Id -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 300
    $text = if (Test-Path $log) { Get-Content $log -Raw } else { '' }
    Remove-Item $log -ErrorAction SilentlyContinue
    Remove-Item $home_ -Recurse -Force -ErrorAction SilentlyContinue
}

$samples = @([regex]::Matches("$text", 'hotkey to action done: (\d+) ms') |
    ForEach-Object { [int]$_.Groups[1].Value } | Sort-Object)

'presses      {0}' -f $Presses
'samples      {0}' -f $samples.Count
if ($samples.Count -eq 0) {
    'no `hotkey to action done` lines in the log: command hotkeys are not wired yet'
    exit 1
}

$p50 = Percentile $samples 0.50
$p95 = Percentile $samples 0.95
'action done  p50 {0} ms   p95 {1} ms   max {2} ms' -f $p50, $p95, $samples[-1]
'all          {0}' -f ($samples -join ' ')

if ($p50 -ge 20 -or $p95 -ge 40) {
    'over budget (p50 < 20 ms, p95 < 40 ms)'
    exit 1
}
