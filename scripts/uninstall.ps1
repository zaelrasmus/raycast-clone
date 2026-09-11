<#
.SYNOPSIS
    Remove Lumen.

.DESCRIPTION
    Undoes everything scripts\install.ps1 did: the binary, the PATH entry, the
    Start Menu entry and the startup entry.

    Your configuration -- themes, sheets, scripts, snippets -- is left alone
    unless you pass -Purge. Reinstalling and finding your shortcut sheets gone
    is a worse surprise than a folder left behind.

.PARAMETER Purge
    Also delete %APPDATA%\Lumen, which is the configuration and the clipboard
    history. This cannot be undone.
#>

[CmdletBinding()]
param([switch]$Purge)

$ErrorActionPreference = 'Stop'

$installDir = Join-Path $env:LOCALAPPDATA 'Programs\Lumen'
$installed = Join-Path $installDir 'lumen.exe'

function Step($text) { Write-Host "  $text" -ForegroundColor Cyan }
function Note($text) { Write-Host "  $text" -ForegroundColor DarkGray }

Write-Host ''
Write-Host 'Removing Lumen' -ForegroundColor White
Write-Host ''

if (Get-Process lumen -ErrorAction SilentlyContinue) {
    Step 'Stopping the daemon'
    if (Test-Path $installed) { & $installed quit 2>$null }
    Start-Sleep -Milliseconds 800
    Get-Process lumen -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 400
}

$shortcuts = @(
    (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Lumen.lnk'),
    (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Startup\Lumen.lnk')
)
foreach ($shortcut in $shortcuts) {
    if (Test-Path $shortcut) {
        Step "Removing $(Split-Path -Leaf $shortcut)"
        Remove-Item $shortcut -Force
    }
}

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -like "*$installDir*") {
    Step 'Taking it off your PATH'
    $kept = ($userPath -split ';' | Where-Object { $_ -and $_ -ne $installDir }) -join ';'
    [Environment]::SetEnvironmentVariable('Path', $kept, 'User')
}

if (Test-Path $installDir) {
    Step 'Removing the binary'
    Remove-Item $installDir -Recurse -Force
}

$config = Join-Path $env:APPDATA 'Lumen'
if ($Purge -and (Test-Path $config)) {
    Step 'Removing your configuration (-Purge)'
    Remove-Item $config -Recurse -Force
} elseif (Test-Path $config) {
    Note "Configuration left at $config (pass -Purge to remove it too)"
}

Write-Host ''
Write-Host '  Lumen removed.' -ForegroundColor Green
Write-Host ''
