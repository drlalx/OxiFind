<#
.SYNOPSIS
    Signs OxiFind release binaries and installers using signtool.exe.

.DESCRIPTION
    Applies an Authenticode digital signature with dual SHA-256 and RFC-3161
    timestamping to oxifind.exe and installer executables. Supports PFX files,
    Windows Certificate Store, or Azure Trusted Signing / Hardware Tokens.

.PARAMETER TargetFile
    Path to the executable or installer to sign.

.PARAMETER CertFile
    Path to the .pfx certificate file (optional if using cert store).

.PARAMETER CertPassword
    Password for the .pfx certificate file (env:SIGN_KEY_PASSWORD).

.PARAMETER CertThumbprint
    SHA-1 thumbprint of the certificate installed in the Windows Certificate Store.

.PARAMETER TimestampServer
    RFC-3161 timestamp authority URL (default: http://timestamp.digicert.com).
#>

[CmdletBinding()]
param (
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$TargetFile,

    [Parameter(Mandatory = $false)]
    [string]$CertFile,

    [Parameter(Mandatory = $false)]
    [string]$CertPassword = $env:SIGN_KEY_PASSWORD,

    [Parameter(Mandatory = $false)]
    [string]$CertThumbprint = $env:SIGN_CERT_THUMBPRINT,

    [Parameter(Mandatory = $false)]
    [string]$TimestampServer = "http://timestamp.digicert.com"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not (Test-Path -Path $TargetFile)) {
    Write-Error "Target file '$TargetFile' does not exist."
    exit 1
}

# Locate signtool.exe from Windows SDK if not in PATH
$SignTool = Get-Command "signtool.exe" -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source
if (-not $SignTool) {
    $SdkPaths = @(
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe",
        "${env:ProgramFiles}\Windows Kits\10\bin\*\x64\signtool.exe"
    )
    $Found = Get-Item $SdkPaths -ErrorAction SilentlyContinue | Select-Object -Last 1
    if ($Found) {
        $SignTool = $Found.FullName
    } else {
        Write-Error "signtool.exe not found in PATH or Windows Kits. Please install the Windows SDK."
        exit 1
    }
}

Write-Host "Using SignTool: $SignTool"
Write-Host "Signing target: $TargetFile"

$SignArgs = @(
    "sign",
    "/fd", "SHA256",
    "/tr", $TimestampServer,
    "/td", "SHA256",
    "/d", "OxiFind - Ultra-fast File Search",
    "/du", "https://github.com/oxifind/oxifind"
)

if ($CertFile -and (Test-Path $CertFile)) {
    Write-Host "Signing with PFX file: $CertFile"
    $SignArgs += @("/f", $CertFile)
    if ($CertPassword) {
        $SignArgs += @("/p", $CertPassword)
    }
} elseif ($CertThumbprint) {
    Write-Host "Signing with certificate thumbprint: $CertThumbprint"
    $SignArgs += @("/sha1", $CertThumbprint, "/sm")
} else {
    Write-Host "Signing with auto-selected code signing certificate from store..."
    $SignArgs += @("/a")
}

$SignArgs += @($TargetFile)

# Execute signing
& $SignTool @SignArgs
if ($LASTEXITCODE -ne 0) {
    Write-Error "Failed to sign $TargetFile. SignTool exited with code $LASTEXITCODE."
    exit $LASTEXITCODE
}

# Verify signature
Write-Host "Verifying signature of $TargetFile..."
& $SignTool verify /pa /v $TargetFile
if ($LASTEXITCODE -ne 0) {
    Write-Error "Signature verification failed for $TargetFile."
    exit $LASTEXITCODE
}

Write-Host "Successfully signed and verified: $TargetFile"
