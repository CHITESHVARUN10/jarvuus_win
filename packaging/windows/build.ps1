[CmdletBinding()]
param(
    [string]$Version = "0.1.0",
    [switch]$SkipBuild,
    [switch]$SkipMsix
)

$ErrorActionPreference = "Stop"

$Version = $Version.Trim()
if ($Version.StartsWith("v") -or $Version.StartsWith("V")) {
    $Version = $Version.Substring(1)
}
if ($Version -notmatch '^\d+\.\d+\.\d+([-+][0-9A-Za-z.-]+)?$') {
    throw "Version must look like 0.1.0 or 0.1.0-beta.1 (got '$Version')."
}

$numeric = ($Version -split '[-+]')[0]
$parts = @($numeric.Split('.'))
while ($parts.Count -lt 3) { $parts += "0" }
$Version4 = "$($parts[0]).$($parts[1]).$($parts[2]).0"

$root = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$exe = Join-Path $root "target/release/jarvis.exe"
$dist = Join-Path $root "dist"
$icon = Join-Path $root "packaging/windows/jarvis.ico"

$workspaceVersion = $null
$inWorkspacePackage = $false
foreach ($line in Get-Content (Join-Path $root "Cargo.toml")) {
    if ($line -match '^\s*\[workspace\.package\]\s*$') {
        $inWorkspacePackage = $true
        continue
    }
    if ($inWorkspacePackage -and $line -match '^\s*version\s*=\s*"([^"]+)"\s*$') {
        $workspaceVersion = $Matches[1]
        break
    }
}
if ($workspaceVersion -and (($workspaceVersion -split '[-+]')[0] -ne $numeric)) {
    throw "Version $Version does not match Cargo.toml's $workspaceVersion. Bump the workspace version first."
}

if (-not (Test-Path $icon)) {
    throw "Missing $icon. Run: python packaging/windows/make_assets.py"
}

if (-not $SkipBuild) {
    Push-Location $root
    try {
        Write-Host "Building Jarvis $Version (release)..."
        cargo build --release -p jarvis-app --locked
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed." }
    } finally {
        Pop-Location
    }
}
if (-not (Test-Path $exe)) {
    throw "Missing $exe. Build the release app first or drop -SkipBuild."
}

if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }
New-Item -ItemType Directory -Force -Path $dist | Out-Null

# DirectML is loaded beside the executable when present. The intent engine falls
# back to CPU if it cannot load, so a missing DLL is a warning, not a hard fail.
$dml = $null
foreach ($candidate in @(
    (Join-Path $root "target/release/DirectML.dll"),
    (Join-Path $root "target/release/deps/DirectML.dll")
)) {
    if (Test-Path $candidate) { $dml = $candidate; break }
}
if (-not $dml -and $env:LOCALAPPDATA) {
    $cache = Join-Path $env:LOCALAPPDATA "ort.pyke.io"
    if (Test-Path $cache) {
        $found = Get-ChildItem $cache -Recurse -Filter DirectML.dll -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if ($found) { $dml = $found.FullName }
    }
}
if (-not $dml) {
    Write-Warning "DirectML.dll was not found. Packages will use CPU intent until it is available."
}

$portableDir = Join-Path $root "target/portable"
if (Test-Path $portableDir) { Remove-Item -Recurse -Force $portableDir }
New-Item -ItemType Directory -Force -Path $portableDir | Out-Null
Copy-Item $exe (Join-Path $portableDir "jarvis.exe") -Force
if ($dml) { Copy-Item $dml (Join-Path $portableDir "DirectML.dll") -Force }
$portableZip = Join-Path $dist "Jarvis-$Version-x64-portable.zip"
Compress-Archive -Path (Join-Path $portableDir "*") -DestinationPath $portableZip -Force

$makensis = (Get-Command makensis.exe -ErrorAction SilentlyContinue | Select-Object -First 1).Source
if (-not $makensis) {
    $makensis = (Get-Command makensis -ErrorAction SilentlyContinue | Select-Object -First 1).Source
}
if (-not $makensis) {
    foreach ($candidate in @(
        "C:/Program Files (x86)/NSIS/makensis.exe",
        "C:/Program Files/NSIS/makensis.exe"
    )) {
        if (Test-Path $candidate) { $makensis = $candidate; break }
    }
}
if (-not $makensis) {
    throw "NSIS not found. Install it with: choco install nsis -y"
}

$setup = Join-Path $dist "Jarvis-$Version-x64-setup.exe"
$define = if ($IsWindows -or $env:OS -eq "Windows_NT") { "/D" } else { "-D" }
$nsisArgs = @(
    "${define}VERSION=$Version",
    "${define}VERSION4=$Version4",
    "${define}EXE=$exe",
    "${define}OUT=$setup",
    "${define}ICON=$icon"
)
if ($dml) { $nsisArgs += "${define}DML=$dml" }
$nsisArgs += (Join-Path $PSScriptRoot "../nsis/jarvis.nsi")
Write-Host "Building NSIS installer..."
& $makensis @nsisArgs
if ($LASTEXITCODE -ne 0) { throw "makensis failed." }

if (-not $SkipMsix) {
    $makeappx = (Get-Command makeappx.exe -ErrorAction SilentlyContinue | Select-Object -First 1).Source
    if (-not $makeappx) {
        $kits = "${env:ProgramFiles(x86)}/Windows Kits/10/bin"
        if (${env:ProgramFiles(x86)} -and (Test-Path $kits)) {
            $makeappx = Get-ChildItem $kits -Recurse -Filter makeappx.exe -ErrorAction SilentlyContinue |
                Where-Object { $_.FullName -match '\\x64\\makeappx.exe$' } |
                Sort-Object FullName -Descending |
                Select-Object -First 1 -ExpandProperty FullName
        }
    }
    if (-not $makeappx) {
        throw "MakeAppx.exe not found. Install the Windows 10/11 SDK."
    }

    $stage = Join-Path $root "target/msix/Jarvis"
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force -Path (Join-Path $stage "Assets") | Out-Null
    Copy-Item $exe (Join-Path $stage "jarvis.exe") -Force
    if ($dml) { Copy-Item $dml (Join-Path $stage "DirectML.dll") -Force }
    Copy-Item (Join-Path $PSScriptRoot "../msix/Assets/*") (Join-Path $stage "Assets") -Force
    $manifest = Get-Content (Join-Path $PSScriptRoot "../msix/AppxManifest.xml") -Raw
    $manifest = $manifest.Replace("__VERSION4__", $Version4)
    Set-Content (Join-Path $stage "AppxManifest.xml") $manifest -Encoding UTF8

    $msix = Join-Path $dist "Jarvis-$Version-x64.msix"
    Write-Host "Building MSIX package..."
    & $makeappx pack /d $stage /p $msix /o
    if ($LASTEXITCODE -ne 0) { throw "makeappx failed." }
}

$artifacts = Get-ChildItem $dist -File | Where-Object { $_.Name -ne "SHA256SUMS.txt" } |
    Sort-Object Name
$checksums = foreach ($artifact in $artifacts) {
    $hash = (Get-FileHash $artifact.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $($artifact.Name)"
}
$checksums | Set-Content (Join-Path $dist "SHA256SUMS.txt") -Encoding ASCII

Write-Host ""
Write-Host "Artifacts in ${dist}:"
$artifacts | ForEach-Object { Write-Host "  $($_.Name)" }
Write-Host "  SHA256SUMS.txt"
