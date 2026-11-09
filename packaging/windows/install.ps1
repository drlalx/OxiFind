<#
.SYNOPSIS
    Installs OxiFind on Windows (Binary, Shortcuts, Context Menu & PATH).

.DESCRIPTION
    Builds (if missing) and installs OxiFind to either the user profile or system-wide.
    Sets up Start Menu shortcuts, Explorer context menu integration, and updates PATH.

.PARAMETER Scope
    'CurrentUser' (default, no admin required) or 'AllUsers' (requires Administrator privileges).

.PARAMETER DesktopShortcut
    Creates a shortcut on the Desktop.

.PARAMETER NoContextMenu
    Disables the "Search with OxiFind" Explorer context menu.

.PARAMETER NoPathUpdate
    Skips adding OxiFind to the PATH environment variable.

.EXAMPLE
    .\packaging\windows\install.ps1
    .\packaging\windows\install.ps1 -DesktopShortcut
    .\packaging\windows\install.ps1 -Scope AllUsers
#>

[CmdletBinding()]
param (
    [ValidateSet("CurrentUser", "AllUsers")]
    [string]$Scope = "CurrentUser",

    [switch]$DesktopShortcut,
    [switch]$WithService,
    [switch]$NoContextMenu,
    [switch]$NoPathUpdate
)

$ErrorActionPreference = "Stop"

# Determine repository root
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = (Resolve-Path "$ScriptDir\..\..").Path

# Interactive prompt if executed without arguments
if ($PSBoundParameters.Count -eq 0) {
    Write-Host "==========================================================" -ForegroundColor Cyan
    Write-Host "               OxiFind Windows Installer                  " -ForegroundColor Cyan
    Write-Host "==========================================================" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Select installation scope:"
    Write-Host "  [1] Current user ($env:LOCALAPPDATA\Programs\OxiFind - Recommended, no admin needed)"
    Write-Host "  [2] System-wide ($env:ProgramFiles\OxiFind - Requires Administrator)"
    $ScopeChoice = Read-Host "Enter choice [1/2] (default: 1)"
    if ($ScopeChoice -eq "2") {
        $Scope = "AllUsers"
    } else {
        $Scope = "CurrentUser"
    }

    $DeskChoice = Read-Host "Create Desktop shortcut? [y/N] (default: N)"
    if ($DeskChoice -match "^[yY]") {
        $DesktopShortcut = $true
    }

    $ServiceChoice = Read-Host "Install background indexer service (Task Scheduler)? [y/N] (default: N)"
    if ($ServiceChoice -match "^[yY]") {
        $WithService = $true
    }
    Write-Host ""
}

# Elevation check for AllUsers
$IsAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if ($Scope -eq "AllUsers" -and -not $IsAdmin) {
    Write-Error "System-wide installation (-Scope AllUsers) requires Administrator privileges. Please re-run from an elevated PowerShell prompt."
    exit 1
}

# Target directories
if ($Scope -eq "AllUsers") {
    $InstallDir = "$env:ProgramFiles\OxiFind"
    $StartMenuDir = "$env:ProgramData\Microsoft\Windows\Start Menu\Programs\OxiFind"
    $DesktopDir = [Environment]::GetFolderPath("CommonDesktopDirectory")
    $RegRoot = "Registry::HKEY_LOCAL_MACHINE\Software\Classes"
    $EnvTarget = "Machine"
    Write-Host "==> Installing OxiFind SYSTEM-WIDE ($InstallDir)..." -ForegroundColor Cyan
} else {
    $InstallDir = "$env:LOCALAPPDATA\Programs\OxiFind"
    $StartMenuDir = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\OxiFind"
    $DesktopDir = [Environment]::GetFolderPath("Desktop")
    $RegRoot = "Registry::HKEY_CURRENT_USER\Software\Classes"
    $EnvTarget = "User"
    Write-Host "==> Installing OxiFind for CURRENT USER ($InstallDir)..." -ForegroundColor Cyan
}

$ExePath = Join-Path $RepoRoot "target\release\oxifind.exe"

# Build if missing
if (-not (Test-Path $ExePath)) {
    Write-Host "==> Release binary not found at $ExePath." -ForegroundColor Yellow
    Write-Host "==> Compiling OxiFind with Cargo..." -ForegroundColor Yellow
    Push-Location $RepoRoot
    try {
        cargo build --release
    } finally {
        Pop-Location
    }
}

# 1. Copy files
Write-Host "==> Copying binaries and resources to $InstallDir..."
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

Copy-Item -Path $ExePath -Destination "$InstallDir\oxifind.exe" -Force
if (Test-Path "$RepoRoot\resources\oxifind.png") {
    Copy-Item -Path "$RepoRoot\resources\oxifind.png" -Destination "$InstallDir\oxifind.png" -Force
}

# 2. Create Start Menu Shortcut
Write-Host "==> Creating Start Menu shortcut..."
if (-not (Test-Path $StartMenuDir)) {
    New-Item -ItemType Directory -Path $StartMenuDir -Force | Out-Null
}

$WshShell = New-Object -ComObject WScript.Shell
$StartShortcut = $WshShell.CreateShortcut("$StartMenuDir\OxiFind.lnk")
$StartShortcut.TargetPath = "$InstallDir\oxifind.exe"
$StartShortcut.WorkingDirectory = $InstallDir
$StartShortcut.Description = "Ultra-fast file search tool"
$StartShortcut.IconLocation = "$InstallDir\oxifind.exe,0"
$StartShortcut.Save()

# 3. Create Desktop Shortcut (if requested)
if ($DesktopShortcut) {
    Write-Host "==> Creating Desktop shortcut..."
    $DeskShortcut = $WshShell.CreateShortcut("$DesktopDir\OxiFind.lnk")
    $DeskShortcut.TargetPath = "$InstallDir\oxifind.exe"
    $DeskShortcut.WorkingDirectory = $InstallDir
    $DeskShortcut.Description = "Ultra-fast file search tool"
    $DeskShortcut.IconLocation = "$InstallDir\oxifind.exe,0"
    $DeskShortcut.Save()
}

# 4. Add Explorer Context Menu
if (-not $NoContextMenu) {
    Write-Host "==> Registering Explorer context menu ('Search with OxiFind')..."
    $ContextTargets = @("Directory\shell\OxiFind", "Drive\shell\OxiFind")
    foreach ($SubKey in $ContextTargets) {
        $KeyPath = "$RegRoot\$SubKey"
        New-Item -Path $KeyPath -Force | Out-Null
        Set-ItemProperty -Path $KeyPath -Name "(Default)" -Value "Search with OxiFind"
        Set-ItemProperty -Path $KeyPath -Name "Icon" -Value "`"$InstallDir\oxifind.exe`""

        $CmdPath = "$KeyPath\command"
        New-Item -Path $CmdPath -Force | Out-Null
        Set-ItemProperty -Path $CmdPath -Name "(Default)" -Value "`"$InstallDir\oxifind.exe`" `"%1`""
    }
}

# 5. Add to PATH
if (-not $NoPathUpdate) {
    Write-Host "==> Updating PATH environment variable ($EnvTarget)..."
    $CurrentPath = [Environment]::GetEnvironmentVariable("Path", $EnvTarget)
    $PathParts = $CurrentPath -split ';' | Where-Object { $_ -ne "" }
    if ($PathParts -notcontains $InstallDir) {
        $NewPath = ($PathParts + $InstallDir) -join ';'
        [Environment]::SetEnvironmentVariable("Path", $NewPath, $EnvTarget)
        $env:Path += ";$InstallDir"
        Write-Host "    Added $InstallDir to $EnvTarget PATH." -ForegroundColor Green
    } else {
        Write-Host "    $InstallDir is already in $EnvTarget PATH." -ForegroundColor Gray
    }
}

# 6. Service setup if requested
if ($WithService) {
    Write-Host "==> Installing background indexer service (Task Scheduler)..."
    & "$InstallDir\oxifind.exe" service install --delay 15
}

Write-Host "`n==> Installation complete!" -ForegroundColor Green
Write-Host "    Executable: $InstallDir\oxifind.exe"
Write-Host "    Start Menu: $StartMenuDir\OxiFind.lnk"
if ($WithService) {
    Write-Host "    Service:    OxiFind-Service (active with 15s delayed start)"
}
Write-Host "You can now run 'oxifind' from PowerShell/CMD, or start it from the Windows Start menu."
