# Measure the per-frame cost while typing, from gpui's own frame measurement.
#
# The budget (p95 < 8 ms per frame while typing, with the full list loaded)
# had only its ranking half measured, by `cargo bench --bench search`. This is
# the drawing half. Like measure-gesture.ps1 it cannot run in CI (7.24): it
# needs a display, a GPU and a desktop that will give the launcher focus.
#
# gpui times every frame it draws and presents when ZED_MEASUREMENTS is set
# (vendor/gpui/src/window.rs, `measure("frame duration", ...)`), and prints it
# to stderr. Frames that only present an unchanged scene are not drawn and are
# not counted. The figure is draw plus present: layout, paint and the GPU
# submission, without the wait for the next vsync, which is latency and not
# cost.
#
# Only frames drawn while keys are being typed are kept: the appearance fade
# and whatever the summons draws are not typing.
#
# Keys are sent with SendKeys, which types into whatever window is in front.
# So every keystroke first checks that the foreground window belongs to the
# daemon, and the script stops rather than type into anything else.
#
#   powershell -File scripts/measure-typing.ps1 -Exe target/release/lumen.exe

param(
    [Parameter(Mandatory)] [string] $Exe,
    [int] $Summons = 6,
    [int] $SettleSeconds = 10,
    [int] $KeyDelayMs = 70,
    # Keep the daemon's log and print where it is, to see what was counted.
    [switch] $KeepLog
)

$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path $Exe).Path
$log = Join-Path ([IO.Path]::GetTempPath()) "lumen-typing-$PID.log"

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

# What is typed: names, a setting, a word that matches little, the calculator
# (which draws the card), each cleared before the next. `+` is SendKeys'
# Shift, so it is braced.
$queries = @('visual studio', 'settings', 'qzx', 'calc', '12{+}12', 'terminal')

function Send-Keys($keys, $daemon) {
    if ([Foreground]::Pid() -ne $daemon) {
        throw 'the launcher is not in front; stopping rather than typing into another window'
    }
    $shell.SendKeys($keys)
    Start-Sleep -Milliseconds $KeyDelayMs
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
        # Past the fade, so its frames are not counted as typing.
        Start-Sleep -Milliseconds 800

        $start = Line-Count
        foreach ($query in $queries) {
            # One key at a time, as a person types: each is its own frame.
            $keys = [regex]::Matches($query, '\{[^}]+\}|.') | ForEach-Object { $_.Value }
            foreach ($key in $keys) { Send-Keys $key $daemon.Id }
            foreach ($key in $keys) { Send-Keys '{BACKSPACE}' $daemon.Id }
        }
        Start-Sleep -Milliseconds 300
        $ranges += , @($start, (Line-Count))

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
