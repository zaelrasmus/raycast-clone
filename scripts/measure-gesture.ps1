# Measure gesture-to-first-frame and cold start, from the daemon's own log.
#
# Like measure-idle.ps1, this cannot run in CI (docs/ARCHITECTURE.md, 7.24).
# The daemon times both itself (crates/lumen-app/src/timing.rs); this script
# only summons it the same way every time and reads the numbers back.
#
# `lumen show` arrives over IPC rather than as a keypress, so the few
# milliseconds between the key and WM_HOTKEY are not in the figure -- the same
# boundary timing.rs documents.
#
#   powershell -File scripts/measure-gesture.ps1 -Exe target/release/lumen.exe

param(
    [Parameter(Mandatory)] [string] $Exe,
    [int] $Summons = 15,
    [int] $SettleSeconds = 10
)

$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path $Exe).Path
$log = Join-Path ([IO.Path]::GetTempPath()) "lumen-gesture-$PID.log"

Get-Process lumen -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

$daemon = Start-Process -FilePath $exe -PassThru -RedirectStandardError $log -WindowStyle Hidden
Start-Sleep -Seconds $SettleSeconds

for ($i = 0; $i -lt $Summons; $i++) {
    & $exe show
    Start-Sleep -Milliseconds 700
    & $exe hide
    # Long enough for the vsync thread to be parked again, so every summons
    # starts from the idle state a real one does.
    Start-Sleep -Milliseconds 1500
}

Start-Sleep -Milliseconds 500
Stop-Process -Id $daemon.Id -Force
Start-Sleep -Milliseconds 300

$text = Get-Content $log -Raw
$frames = [regex]::Matches($text, 'gesture to first frame: (\d+) ms') |
    ForEach-Object { [int]$_.Groups[1].Value } | Sort-Object
$cold = [regex]::Match($text, 'cold start to usable: (\d+) ms').Groups[1].Value

function Percentile($sorted, $p) {
    $sorted[[math]::Min($sorted.Count - 1, [math]::Ceiling($p * $sorted.Count) - 1)]
}

'cold start   {0} ms' -f $cold
'summonses    {0}' -f $frames.Count
'first frame  p50 {0} ms   p95 {1} ms   max {2} ms' -f `
    (Percentile $frames 0.50), (Percentile $frames 0.95), $frames[-1]
'all          {0}' -f ($frames -join ' ')

Remove-Item $log
