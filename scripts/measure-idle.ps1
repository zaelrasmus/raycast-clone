# Measure Lumen's idle footprint: CPU and resident memory with the window hidden.
#
# These two budgets cannot run in CI (docs/ARCHITECTURE.md, 7.24): a hosted
# runner has no display, no GPU and no idle session. This script is how the
# numbers in the README are produced, so they can be reproduced rather than
# taken on trust.
#
# Starts the given build, lets it settle, then samples it while nobody touches
# it. The daemon runs in a `LUMEN_HOME` of its own, with its own socket (F15),
# so it starts beside an installed one, reads and writes nothing of it, and
# only the processes this script started are stopped.
#
# The build is run once and quit before it is measured. The first time a GPU
# driver sees an executable path it compiles the renderer's shaders for it,
# and that compiler stays loaded in the process: on the AMD driver this was
# measured at +40 MB private and +200 ms CPU, for as long as that process
# lives. It happens once per install location, not once per build, so it is
# not what a user lives with -- but unwarmed, every fresh copy of a build looks
# like a 40 MB regression.
#
#   powershell -File scripts/measure-idle.ps1 -Exe target/release/lumen.exe
#
# With timers running (F8), which must cost nothing while the window is
# hidden -- the daemon sleeps until the nearest deadline and does not tick:
#
#   powershell -File scripts/measure-idle.ps1 -Exe target/release/lumen.exe -Timers 25m,1h,90m
#
# After the emoji grid (F18) has been walked end to end and hidden again, which
# is the most glyphs a summons can leave behind. Keys are sent with SendKeys,
# each only once the launcher is in front, as in measure-grid.ps1:
#
#   powershell -File scripts/measure-idle.ps1 -Exe target/release/lumen.exe -Walk

param(
    [Parameter(Mandatory)] [string] $Exe,
    # Long enough for the application index, the warm-up frame and any
    # one-shot timers to finish.
    [int] $SettleSeconds = 20,
    [int] $SampleSeconds = 60,
    # Durations to start with `lumen timer` before settling. Longer than the
    # measurement, so none of them ends inside it.
    [string[]] $Timers = @(),
    # Walk the whole emoji grid, then hide, before settling.
    [switch] $Walk,
    # 1,637 emoji are 205 rows of eight.
    [int] $Downs = 210,
    # The test daemon's config, data and cache.
    [string] $LumenHome = (Join-Path ([IO.Path]::GetTempPath()) 'lumen-idle-home')
)

$ErrorActionPreference = 'Stop'

$exe = (Resolve-Path $Exe).Path

# Set for the daemon and for `timer`, `show` and `hide`, which find it by its
# socket.
New-Item -ItemType Directory -Force $LumenHome | Out-Null
$env:LUMEN_HOME = (Resolve-Path $LumenHome).Path

# Warm the driver's shader cache for this path (see above).
$warm = Start-Process -FilePath $exe -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 5
Stop-Process -Id $warm.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2

$process = Start-Process -FilePath $exe -PassThru

# Split here as well: under `powershell -File`, `-Timers 25m,1h` arrives as
# one string, and `lumen timer "25m,1h"` is refused -- measuring a daemon
# with no timers while the heading says three.
$Timers = @($Timers | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
if ($Timers.Count -gt 0) {
    # The daemon answers its socket well within this.
    Start-Sleep -Seconds 3
    foreach ($timer in $Timers) {
        $started = Start-Process -FilePath $exe -ArgumentList 'timer', $timer -Wait -PassThru
        if ($started.ExitCode -ne 0) {
            throw "``lumen timer $timer`` failed with exit code $($started.ExitCode)"
        }
    }
    'timers       {0}' -f ($Timers -join ', ')
}

if ($Walk) {
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
    function Send-Keys($keys, $delay = 33) {
        if ([Foreground]::Pid() -ne $process.Id) {
            Stop-Process -Id $process.Id -Force
            Remove-Item Env:\LUMEN_HOME
            throw 'the launcher is not in front; stopping rather than typing into another window'
        }
        $shell.SendKeys($keys)
        Start-Sleep -Milliseconds $delay
    }

    Start-Sleep -Seconds $SettleSeconds
    & $exe show
    Start-Sleep -Milliseconds 800
    foreach ($key in 'e', 'm', 'o', 'j', 'i') { Send-Keys $key 70 }
    Start-Sleep -Milliseconds 300
    Send-Keys '{ENTER}' 700
    for ($k = 0; $k -lt $Downs; $k++) { Send-Keys '{DOWN}' }
    Start-Sleep -Milliseconds 300
    & $exe hide
    'walked       {0} rows of the emoji grid, then hid' -f $Downs
}

Start-Sleep -Seconds $SettleSeconds

if ($process.HasExited) {
    throw "$exe exited before it could be measured; run it from a terminal to see why"
}

$process.Refresh()
$cores = [Environment]::ProcessorCount
$before = $process.TotalProcessorTime.TotalMilliseconds
$threads = @{}
foreach ($thread in $process.Threads) {
    $threads[$thread.Id] = $thread.TotalProcessorTime.TotalMilliseconds
}

Start-Sleep -Seconds $SampleSeconds

$process.Refresh()
$spent = $process.TotalProcessorTime.TotalMilliseconds - $before

'idle CPU     {0:N3} % of the machine ({1:N1} ms over {2} s, {3} cores)' -f `
    ($spent / ($SampleSeconds * 1000 * $cores) * 100), $spent, $SampleSeconds, $cores
'working set  {0:N1} MB' -f ($process.WorkingSet64 / 1MB)
'private      {0:N1} MB' -f ($process.PrivateMemorySize64 / 1MB)
'threads      {0}' -f $process.Threads.Count

# The atlas textures are GPU memory, not in the working set (7.61).
$gpu = Get-Counter -ErrorAction SilentlyContinue `
    "\GPU Process Memory(pid_$($process.Id)_*)\Shared Usage",
    "\GPU Process Memory(pid_$($process.Id)_*)\Dedicated Usage"
if ($gpu) {
    $samples = $gpu.CounterSamples
    'GPU shared   {0:N1} MB' -f (($samples | Where-Object Path -like '*shared usage' | Measure-Object CookedValue -Sum).Sum / 1MB)
    'GPU dedic.   {0:N1} MB' -f (($samples | Where-Object Path -like '*dedicated usage' | Measure-Object CookedValue -Sum).Sum / 1MB)
}

# Which threads woke, so a regression points at its source.
foreach ($thread in $process.Threads) {
    $start = $threads[$thread.Id]
    if ($null -eq $start) { continue }
    $delta = $thread.TotalProcessorTime.TotalMilliseconds - $start
    if ($delta -gt 0) {
        '  thread {0,6}  +{1,7:N1} ms  {2}' -f $thread.Id, $delta, $thread.WaitReason
    }
}

Stop-Process -Id $process.Id -Force
Remove-Item Env:\LUMEN_HOME
