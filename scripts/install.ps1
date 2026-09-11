<#
.SYNOPSIS
    Install Lumen for daily use.

.DESCRIPTION
    Builds a release binary and installs it somewhere `cargo clean` cannot
    reach, which is the whole difference between "it runs" and "it is
    installed": target/release is a build directory, not a home.

    What this does, all of it reversible by scripts/uninstall.ps1:

      * copies lumen.exe to %LOCALAPPDATA%\Programs\Lumen
      * puts that folder on the user PATH, so `lumen` works in a terminal
      * adds a Start Menu entry
      * adds a startup entry, so the daemon is running when you log in

    Nothing is written outside the user's own profile and nothing needs
    administrator rights.

.PARAMETER SkipBuild
    Install whatever is already in target/release instead of rebuilding.

.PARAMETER NoStartup
    Do not start Lumen at login. The daemon then has to be started by hand,
    and the global hotkey only works while it is running.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\install.ps1
#>

[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$NoStartup
)

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$target = Join-Path $repo 'target\release\lumen.exe'
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\Lumen'
$installed = Join-Path $installDir 'lumen.exe'

function Step($text) { Write-Host "  $text" -ForegroundColor Cyan }
function Done($text) { Write-Host "  $text" -ForegroundColor Green }
function Note($text) { Write-Host "  $text" -ForegroundColor DarkGray }

Write-Host ''
Write-Host 'Installing Lumen' -ForegroundColor White
Write-Host ''

# -- Build ------------------------------------------------------------------

if (-not $SkipBuild) {
    Step 'Building (this takes a few minutes the first time)...'
    Push-Location $repo
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw 'the build failed; nothing was installed' }
    } finally {
        Pop-Location
    }
}

if (-not (Test-Path $target)) {
    throw "no binary at $target. Run without -SkipBuild."
}

# -- Stop anything already running ------------------------------------------
#
# An installed copy that is running holds its own file open, and Windows will
# not let it be overwritten. Asking it to quit is politer than killing it: the
# daemon closes its windows and releases the hotkey on the way out.

$running = Get-Process lumen -ErrorAction SilentlyContinue
if ($running) {
    Step 'Stopping the running copy...'
    if (Test-Path $installed) { & $installed quit 2>$null }
    Start-Sleep -Milliseconds 800
    Get-Process lumen -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 400
}

# -- Copy -------------------------------------------------------------------

Step "Installing to $installDir"
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
Copy-Item $target $installed -Force

$size = [math]::Round((Get-Item $installed).Length / 1MB, 1)
Note "lumen.exe, $size MB"

# -- PATH -------------------------------------------------------------------

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -notlike "*$installDir*") {
    Step 'Adding it to your PATH'
    $updated = if ([string]::IsNullOrEmpty($userPath)) { $installDir } else { "$userPath;$installDir" }
    [Environment]::SetEnvironmentVariable('Path', $updated, 'User')
    Note 'open a new terminal before `lumen` is on PATH there'
}

# -- Shortcuts --------------------------------------------------------------

function New-Shortcut($path, $target, $description) {
    $shell = New-Object -ComObject WScript.Shell
    $link = $shell.CreateShortcut($path)
    $link.TargetPath = $target
    $link.WorkingDirectory = Split-Path -Parent $target
    $link.Description = $description
    $link.Save()
}

$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
Step 'Adding a Start Menu entry'
New-Shortcut (Join-Path $startMenu 'Lumen.lnk') $installed 'Lumen launcher'

$startup = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Startup'
$startupLink = Join-Path $startup 'Lumen.lnk'

if ($NoStartup) {
    if (Test-Path $startupLink) { Remove-Item $startupLink -Force }
    Note 'not starting at login (-NoStartup)'
} else {
    Step 'Starting it at login'
    New-Shortcut $startupLink $installed 'Lumen launcher (daemon)'
}

# -- Start it now -----------------------------------------------------------

Step 'Starting the daemon'
Start-Process $installed
Start-Sleep -Seconds 3

$now = Get-Process lumen -ErrorAction SilentlyContinue
if (-not $now) {
    throw 'the daemon did not stay running. Run it from a terminal to see why.'
}

Write-Host ''
Done 'Lumen is installed and running.'
Write-Host ''
Write-Host '  Press Alt+Space to open it.' -ForegroundColor White
Write-Host ''
Note 'If nothing happens, something else already owns Alt+Space (Raycast,'
Note 'PowerToys Run, Spotlight-alikes). Quit it and restart Lumen with:'
Note '    lumen quit ; lumen'
Note ''
Note 'Config lives in %APPDATA%\Lumen\config'
Note 'Uninstall with scripts\uninstall.ps1'
Write-Host ''
