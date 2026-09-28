# Measure Lumen's idle footprint: CPU and resident memory with the window hidden.
#
# These two budgets cannot run in CI (docs/ARCHITECTURE.md, 7.24): a hosted
# runner has no display, no GPU and no idle session. This script is how the
# numbers in the README are produced, so they can be reproduced rather than
# taken on trust.
#
# Starts the given build, lets it settle, then samples it while nobody touches
# it. Any Lumen daemon already running is stopped first, because a second
# `lumen` toggles the running one instead of starting its own.
#
#   powershell -File scripts/measure-idle.ps1 -Exe target/release/lumen.exe

param(
    [Parameter(Mandatory)] [string] $Exe,
    # Long enough for the application index, the warm-up frame and any
    # one-shot timers to finish.
    [int] $SettleSeconds = 20,
    [int] $SampleSeconds = 60
)

$ErrorActionPreference = 'Stop'

Get-Process lumen -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

$process = Start-Process -FilePath (Resolve-Path $Exe) -PassThru
Start-Sleep -Seconds $SettleSeconds

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
