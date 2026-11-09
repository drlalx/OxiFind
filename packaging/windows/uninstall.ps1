<#
.SYNOPSIS
    Uninstalls OxiFind on Windows.

.PARAMETER Scope
    'CurrentUser' (default) or 'AllUsers'.

.EXAMPLE
    .\packaging\windows\uninstall.ps1
    .\packaging\windows\uninstall.ps1 -Scope AllUsers
#>

[CmdletBinding()]
param (
    [ValidateSet("CurrentUser", "AllUsers")]
    [string]$Scope = "CurrentUser"
)

$ErrorActionPreference = "Stop"

$IsAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if ($Scope -eq "AllUsers" -and -not $IsAdmin) {
    Write-Error "System-wide uninstallation (-Scope AllUsers) requires Administrator privileges. Please re-run from an elevated PowerShell prompt."
    exit 1
}

if ($Scope -eq "AllUsers") {
    $InstallDir = "$env:ProgramFiles\OxiFind"
    $StartMenuDir = "$env:ProgramData\Microsoft\Windows\Start Menu\Programs\OxiFind"
    $DesktopDir = [Environment]::GetFolderPath("CommonDesktopDirectory")
    $RegRoot = "Registry::HKEY_LOCAL_MACHINE\Software\Classes"
    $EnvTarget = "Machine"
    Write-Host "==> Uninstalling OxiFind (SYSTEM-WIDE)..." -ForegroundColor Cyan
} else {
    $InstallDir = "$env:LOCALAPPDATA\Programs\OxiFind"
    $StartMenuDir = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\OxiFind"
    $DesktopDir = [Environment]::GetFolderPath("Desktop")
    $RegRoot = "Registry::HKEY_CURRENT_USER\Software\Classes"
    $EnvTarget = "User"
    Write-Host "==> Uninstalling OxiFind (CURRENT USER)..." -ForegroundColor Cyan
}

# 1. Remove Context Menu
$ContextTargets = @("Directory\shell\OxiFind", "Drive\shell\OxiFind")
foreach ($SubKey in $ContextTargets) {
    $KeyPath = "$RegRoot\$SubKey"
    if (Test-Path $KeyPath) {
        Write-Host "==> Removing Explorer context menu ($SubKey)..."
        Remove-Item -Path $KeyPath -Recurse -Force
    }
}

# 2. Remove Shortcuts
if (Test-Path $StartMenuDir) {
    Write-Host "==> Removing Start Menu shortcuts..."
    Remove-Item -Path $StartMenuDir -Recurse -Force
}

$DesktopLnk = Join-Path $DesktopDir "OxiFind.lnk"
if (Test-Path $DesktopLnk) {
    Write-Host "==> Removing Desktop shortcut..."
    Remove-Item -Path $DesktopLnk -Force
}

# 3. Remove from PATH
$CurrentPath = [Environment]::GetEnvironmentVariable("Path", $EnvTarget)
if ($CurrentPath) {
    $PathParts = $CurrentPath -split ';' | Where-Object { $_ -ne "" -and $_ -ne $InstallDir }
    $NewPath = $PathParts -join ';'
    [Environment]::SetEnvironmentVariable("Path", $NewPath, $EnvTarget)
    Write-Host "==> Removed from $EnvTarget PATH."
}

# 4. Remove Files
if (Test-Path $InstallDir) {
    Write-Host "==> Removing program files in $InstallDir..."
    Remove-Item -Path $InstallDir -Recurse -Force
}

Write-Host "`n==> OxiFind has been uninstalled successfully." -ForegroundColor Green
