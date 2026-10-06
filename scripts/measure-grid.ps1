# Measure the per-frame cost of the emoji grid (F18) while Down is held.
#
# NEEDS THE VIEW FROM F18 TASK 5. Until Search Emoji & Symbols exists, typing
# `emoji` and Enter opens nothing and this script measures the wrong thing.
#
# The budget is the typing one (p95 < 8 ms per frame), held over the grid:
# every row the selection moves into view brings eight glyphs the atlas may
# not have yet, and a colour glyph is rasterised by DirectWrite on the frame
# that first draws it. The spike (crates/lumen-ui/examples/emoji_grid_spike.rs)
# measured that cost in isolation; this is the real view, with the launcher
# around it. Like measure-typing.ps1 it cannot run in CI (7.24): it needs a
# display, a GPU and a desktop that will give the launcher focus.
#
# gpui times every frame it draws when ZED_MEASUREMENTS is set
# (vendor/gpui/src/window.rs, `measure("frame duration", ...)`), and prints it
# to stderr. Only frames drawn while the arrows are being sent are kept: the
# appearance fade, the root search for `emoji` and the push of the view are
# not the grid.
#
# Keys are sent with SendKeys, which types into whatever window is in front.
# So every keystroke first checks that the foreground window belongs to the
# daemon, and the script stops rather than type into anything else.
#
#   powershell -File scripts/measure-grid.ps1 -Exe target/release/lumen.exe

param(
    [Parameter(Mandatory)] [string] $Exe,
    [int] $Summons = 4,
    [int] $SettleSeconds = 10,
    # About 30 presses a second: Windows' fastest key repeat.
    [int] $KeyDelayMs = 33,
    # Enough to walk the emoji groups: 1,637 emoji are 205 rows of eight.
    [int] $Downs = 200,
    [int] $Rights = 20,
    # Keep the daemon's log and print where it is, to see what was counted.
    [switch] $KeepLog
)

$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path $Exe).Path
$log = Join-Path ([IO.Path]::GetTempPath()) "lumen-grid-$PID.log"

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Foreground {
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    public static uint Pid() {
        uint pid;
        GetWindowThreadProcessId(GetForegroundWindow(), out pid);
        return pid;
    }
}
'@
$shell = New-Object -ComObject WScript.Shell

function Send-Keys($keys, $daemon, $delay = $KeyDelayMs) {
    if ([Foreground]::Pid() -ne $daemon) {
        throw 'the launcher is not in front; stopping rather than typing into another window'
    }
    $shell.SendKeys($keys)
    Start-Sleep -Milliseconds $delay
}

function Line-Count {
    @(Get-Content $log -Encoding UTF8 -ErrorAction SilentlyContinue).Count
}

# By name as well: a copied build (lumen-head.exe) runs as lumen-head.
$name = [IO.Path]::GetFileNameWithoutExtension($exe)
Get-Process lumen, $name -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

$env:ZED_MEASUREMENTS = '1'
$daemon = Start-Process -FilePath $exe -PassThru -RedirectStandardError $log -WindowStyle Hidden
Remove-Item Env:\ZED_MEASUREMENTS
Start-Sleep -Seconds $SettleSeconds

$ranges = @()
try {
    for ($i = 0; $i -lt $Summons; $i++) {
        & $exe show
        # Past the fade, so its frames are not counted.
        Start-Sleep -Milliseconds 800

        # Open the view at a person's pace, then let it settle: none of this
        # is the grid.
        foreach ($key in 'e', 'm', 'o', 'j', 'i') { Send-Keys $key $daemon.Id 70 }
        Start-Sleep -Milliseconds 300
        Send-Keys '{ENTER}' $daemon.Id 700

        $start = Line-Count
        for ($k = 0; $k -lt $Downs; $k++) { Send-Keys '{DOWN}' $daemon.Id }
        for ($k = 0; $k -lt $Rights; $k++) { Send-Keys '{RIGHT}' $daemon.Id }
        Start-Sleep -Milliseconds 300
        $ranges += , @($start, (Line-Count))

        # If the launcher reopens in the view rather than at the root, the
        # next `emoji` lands in the view's search field: compare the first
        # summons with the rest using -KeepLog.
        & $exe hide
        Start-Sleep -Milliseconds 1500
    }
} finally {
    Stop-Process -Id $daemon.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300
}

$lines = @(Get-Content $log -Encoding UTF8)
$frames = foreach ($range in $ranges) {
    for ($l = $range[0]; $l -lt $range[1] -and $l -lt $lines.Count; $l++) {
        # Rust's Duration prints microseconds with a micro sign, escaped here
        # because Windows PowerShell reads a script as ANSI.
        $m = [regex]::Match($lines[$l], 'frame duration: ([\d.]+)(ns|\u00B5s|us|ms|s)\b')
        if (-not $m.Success) { continue }
        $value = [double]::Parse($m.Groups[1].Value, [Globalization.CultureInfo]::InvariantCulture)
        switch ($m.Groups[2].Value -replace '\u00B5', 'u') {
            'ns' { $value / 1e6 }
            'us' { $value / 1e3 }
            'ms' { $value }
            's' { $value * 1e3 }
        }
    }
}
$frames = @($frames | Sort-Object)

if ($frames.Count -eq 0) {
    Remove-Item $log
    throw 'no frames were measured; was the launcher given focus?'
}

function Percentile($sorted, $p) {
    $sorted[[math]::Min($sorted.Count - 1, [math]::Ceiling($p * $sorted.Count) - 1)]
}

'frames       {0} over {1} summonses' -f $frames.Count, $Summons
'frame cost   p50 {0:N2} ms   p95 {1:N2} ms   max {2:N2} ms   (budget: p95 8 ms)' -f `
    (Percentile $frames 0.50), (Percentile $frames 0.95), $frames[-1]

if ($KeepLog) { 'log          {0}   ranges {1}' -f $log, (($ranges | ForEach-Object { $_ -join '-' }) -join ' ') } else { Remove-Item $log }
