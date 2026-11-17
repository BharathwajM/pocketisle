<#
.SYNOPSIS
  Build Pocket Isle from this folder, install it for the current user, and start it.

.DESCRIPTION
  1. Builds pocketisle.exe with Cargo (skipped with -Exe when you already have one, for example
     one downloaded from GitHub).
  2. Copies it to %LOCALAPPDATA%\PocketIsle\pocketisle.exe (closing a running island first).
  3. Starts it with Windows: a scheduled task "Pocket Isle" that runs at sign-in, for you
     only, without admin rights (skip with -NoStartup).
  4. Starts the island.
  Settings are created on first run in %USERPROFILE%\.config\pocketisle\config.json.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\install.ps1
.EXAMPLE
  .\install.ps1 -Exe "$HOME\Downloads\pocketisle.exe"
.EXAMPLE
  .\install.ps1 -NoStartup
#>
param(
    # A pocketisle.exe to install instead of building one.
    [string]$Exe,
    # Don't start Pocket Isle with Windows.
    [switch]$NoStartup
)
$ErrorActionPreference = 'Stop'
$dir = Join-Path $env:LOCALAPPDATA 'PocketIsle'
$task = 'Pocket Isle'

New-Item -ItemType Directory -Force $dir | Out-Null

if (-not $Exe) {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-Host 'Rust is not installed. Install it and the C++ build tools, then open a new terminal and run this again:' -ForegroundColor Yellow
        Write-Host '  winget install Rustlang.Rustup'
        Write-Host '  winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"'
        Write-Host 'Or download a ready-built pocketisle.exe from GitHub and run: .\install.ps1 -Exe <path to pocketisle.exe>'
        exit 1
    }
    # A short build folder: link.exe fails on very long paths.
    $env:CARGO_TARGET_DIR = Join-Path $dir 'build'
    Write-Host 'Building Pocket Isle (the first build takes a few minutes)...'
    Push-Location $PSScriptRoot
    try {
        cargo build --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'The build failed (see above).' }
    } finally {
        Pop-Location
    }
    $Exe = Join-Path $env:CARGO_TARGET_DIR 'release\pocketisle.exe'
}
if (-not (Test-Path -LiteralPath $Exe)) { throw "Not found: $Exe" }

$target = Join-Path $dir 'pocketisle.exe'
Get-Process -Name pocketisle -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 800
Copy-Item -LiteralPath $Exe -Destination $target -Force
Unblock-File -LiteralPath $target -ErrorAction SilentlyContinue
Write-Host "Installed: $target"

if (-not $NoStartup) {
    $action = New-ScheduledTaskAction -Execute $target
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew
    $principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Limited
    Register-ScheduledTask -TaskName $task -Action $action -Trigger $trigger -Settings $settings -Principal $principal -Description 'Starts the Pocket Isle dynamic island when you sign in.' -Force | Out-Null
    Write-Host "Starts with Windows (scheduled task '$task')."
    Start-ScheduledTask -TaskName $task
} else {
    Start-Process -FilePath $target
}
Start-Sleep -Seconds 2
if (Get-Process -Name pocketisle -ErrorAction SilentlyContinue) {
    Write-Host 'Pocket Isle is running: look at the top of your screen. Right-click it for the menu.' -ForegroundColor Green
} else {
    Write-Host "Pocket Isle didn't start. Its log: $dir\pocketisle.log" -ForegroundColor Yellow
}
