<#
.SYNOPSIS
  Remove Pocket Isle: close it, stop it starting with Windows, and delete the program.

.DESCRIPTION
  Your settings, Pokedex, notes and downloaded sprites in %USERPROFILE%\.config\pocketisle are
  kept unless you add -RemoveSettings. Claude Code / Codex hooks you added point at
  pocketisle.exe: take them out of %USERPROFILE%\.claude\settings.json and
  %USERPROFILE%\.codex\hooks.json yourself.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\uninstall.ps1
.EXAMPLE
  .\uninstall.ps1 -RemoveSettings
#>
param(
    # Also delete settings, the Pokedex, notes, downloaded sprites and game saves.
    [switch]$RemoveSettings
)
$ErrorActionPreference = 'Stop'
$dir = Join-Path $env:LOCALAPPDATA 'PocketIsle'

Get-Process -Name pocketisle -ErrorAction SilentlyContinue | Stop-Process -Force
if (Get-ScheduledTask -TaskName 'Pocket Isle' -ErrorAction SilentlyContinue) {
    Unregister-ScheduledTask -TaskName 'Pocket Isle' -Confirm:$false
    Write-Host 'No longer starts with Windows.'
}
Start-Sleep -Milliseconds 800
if (Test-Path -LiteralPath $dir) {
    # In-game saves live in games\saves: keep them unless asked.
    Get-ChildItem -LiteralPath $dir -Force | Where-Object { $RemoveSettings -or $_.Name -ne 'games' } | Remove-Item -Recurse -Force
    if (-not (Get-ChildItem -LiteralPath $dir -Force)) { Remove-Item -LiteralPath $dir -Force }
    Write-Host "Removed Pocket Isle from $dir"
    if (Test-Path -LiteralPath (Join-Path $dir 'games')) { Write-Host "Kept your game saves in $dir\games." }
}
$settings = Join-Path $env:USERPROFILE '.config\pocketisle'
if ($RemoveSettings -and (Test-Path -LiteralPath $settings)) {
    Remove-Item -LiteralPath $settings -Recurse -Force
    Write-Host "Removed $settings"
} elseif (Test-Path -LiteralPath $settings) {
    Write-Host "Kept your settings and Pokedex in $settings (add -RemoveSettings to delete them and your game saves)."
}
Write-Host 'Pocket Isle is uninstalled.' -ForegroundColor Green
