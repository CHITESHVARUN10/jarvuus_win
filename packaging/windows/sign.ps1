[CmdletBinding()]
param(
    [string]$Version = "0.1.0",
    [string]$Certificate = "",
    [string]$Password = "",
    [string]$TimestampUrl = "http://timestamp.digicert.com",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($Certificate)) {
    throw "Pass -Certificate (a .pfx path), or set JARVIS_SIGNING_CERT_FILE."
}
if (-not (Test-Path $Certificate)) {
    $fromEnv = $env:JARVIS_SIGNING_CERT_FILE
    if ($fromEnv -and (Test-Path $fromEnv)) {
        $Certificate = $fromEnv
    } else {
        throw "Signing certificate not found: $Certificate"
    }
}
if ([string]::IsNullOrWhiteSpace($Password)) {
    $Password = $env:JARVIS_SIGNING_PASSWORD
}
if ([string]::IsNullOrWhiteSpace($Password)) {
    throw "Pass -Password or set JARVIS_SIGNING_PASSWORD."
}

$root = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$dist = Join-Path $root "dist"
$exe = Join-Path $root "target/release/jarvis.exe"

if (-not $SkipBuild) {
    Push-Location $root
    try {
        cargo build --release -p jarvis-app --locked
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed." }
    } finally {
        Pop-Location
    }
}
if (-not (Test-Path $exe)) {
    throw "Missing $exe. Build the release app first or drop -SkipBuild."
}

$signtool = (Get-Command signtool.exe -ErrorAction SilentlyContinue | Select-Object -First 1).Source
if (-not $signtool) {
    $kits = "${env:ProgramFiles(x86)}/Windows Kits/10/bin"
    if (${env:ProgramFiles(x86)} -and (Test-Path $kits)) {
        $signtool = Get-ChildItem $kits -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match '\\x64\\signtool.exe$' } |
            Sort-Object FullName -Descending |
            Select-Object -First 1 -ExpandProperty FullName
    }
}
if (-not $signtool) {
    throw "signtool.exe not found. Install the Windows 10/11 SDK."
}

function Sign-And-Verify([string]$Target) {
    Write-Host "Signing $Target"
    & $signtool sign /fd SHA256 /td SHA256 /tr $TimestampUrl /f $Certificate /p $Password $Target
    if ($LASTEXITCODE -ne 0) { throw "signtool sign failed for $Target" }
    & $signtool verify /pa /all $Target
    if ($LASTEXITCODE -ne 0) { throw "signature verification failed for $Target" }
}

# Sign the executable before packaging so the portable zip, the installer payload
# and the MSIX all carry the signed binary rather than an unsigned copy.
Sign-And-Verify $exe

& (Join-Path $PSScriptRoot "build.ps1") -Version $Version -SkipBuild
if ($LASTEXITCODE -ne 0) { throw "packaging failed." }
if (-not (Test-Path $dist)) {
    throw "No dist folder at $dist after packaging."
}

Get-ChildItem $dist -Filter "*.exe" | ForEach-Object { Sign-And-Verify $_.FullName }

# The MSIX signature lives on the archive, not on the payload executable.
$msix = Get-ChildItem $dist -Filter "*.msix" -ErrorAction SilentlyContinue | Select-Object -First 1
if ($msix) {
    Sign-And-Verify $msix.FullName
}

# Checksums must describe the signed bytes, so refresh after every signature.
& (Join-Path $PSScriptRoot "checksums.ps1") -Directory $dist
if ($LASTEXITCODE -ne 0) { throw "checksum refresh failed." }

Write-Host ""
Write-Host "Signed artifacts:"
Get-ChildItem $dist -File | Sort-Object Name | ForEach-Object { Write-Host "  $($_.Name)" }
Write-Host ""
Write-Host "Set JARVIS_UPDATE_PUBLISHER on client machines to this certificate's subject"
Write-Host "or publisher name so updates are pinned to this signer."
