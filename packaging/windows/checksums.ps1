[CmdletBinding()]
param([string]$Directory = "")

$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
if ([string]::IsNullOrWhiteSpace($Directory)) {
    $Directory = Join-Path $root "dist"
}
if (-not (Test-Path $Directory)) {
    throw "No dist folder at $Directory. Build the packages first."
}

$artifacts = Get-ChildItem $Directory -File | Where-Object {
    $_.Name -ne "SHA256SUMS.txt" -and $_.Name -notlike "*.sha256"
} | Sort-Object Name

$lines = foreach ($artifact in $artifacts) {
    $hash = (Get-FileHash $artifact.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $($artifact.Name)"
}
$lines | Set-Content (Join-Path $Directory "SHA256SUMS.txt") -Encoding ASCII
$lines | ForEach-Object { Write-Host $_ }
