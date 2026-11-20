# Windows packaging

Phase 6 builds three artifacts from one release executable:

| Artifact | Purpose |
| --- | --- |
| `Jarvis-<version>-x64-setup.exe` | Per-user NSIS installer, no administrator required |
| `Jarvis-<version>-x64-portable.zip` | Portable folder with `jarvis.exe` and `DirectML.dll` |
| `Jarvis-<version>-x64.msix` | MSIX package for Start menu/tile installation and execution alias |
| `SHA256SUMS.txt` | SHA-256 for every artifact |

## Prerequisites

- Windows 10 2004+ or Windows 11, x64.
- Rust 1.97+, MSVC build tools, CMake and LLVM/libclang (the app's native model
  dependencies).
- NSIS 3: `choco install nsis -y`.
- Windows 10/11 SDK for `MakeAppx.exe`.
- For the target machine: Microsoft Visual C++ 2015–2022 Redistributable (x64).
  The installer warns when `vcruntime140.dll` is missing; download it from
  <https://aka.ms/vs/17/release/vc_redist.x64.exe>. It is present on most Windows
  10/11 installs and on the CI runner.

The release workflow installs NSIS and uses the SDK already present on
`windows-latest`.

## Build

From the repository root:

```powershell
pwsh packaging/windows/build.ps1 -Version 0.1.0
```

The script builds `target\release\jarvis.exe` unless `-SkipBuild` is passed,
stages the MSIX tree, builds the installer and writes `dist\`.

```powershell
pwsh packaging/windows/build.ps1 -Version 0.1.0 -SkipBuild
pwsh packaging/windows/build.ps1 -Version 0.1.0 -SkipMsix
```

`-Version` accepts `v0.1.0` or `0.1.0-beta.1`. The MSIX and installer version
fields use the first three numeric components plus `.0`; the display version
keeps the suffix. The script refuses a version whose numeric base does not match
the workspace version in `Cargo.toml`, so bump that before tagging a release.

## DirectML

The script looks for `DirectML.dll` in `target\release`, then in ort's
`%LOCALAPPDATA%\ort.pyke.io` cache. When found it is placed beside the executable
in all three artifacts. Without it the app still starts and the intent engine
falls back to CPU, but DirectML acceleration is unavailable.

## Installer behaviour

- Installs to `%LOCALAPPDATA%\Programs\Jarvis` for the current user.
- Adds Start menu shortcuts and an optional desktop shortcut.
- Registers a normal uninstall entry under
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Jarvis`.
- Stops a running `jarvis.exe` before replacing files.
- On uninstall, asks whether to remove `%LOCALAPPDATA%\Jarvis` (settings,
  routines, insights and downloaded models). The default is to keep it.
- Removes the launch-at-login `Run` value on uninstall.

## MSIX notes

`AppxManifest.xml` uses the `runFullTrust` capability and a
`Windows.FullTrustApplication` entry point. The package is unsigned by default.
For local testing, enable Developer Mode and install with:

```powershell
Add-AppxPackage -AllowUnsigned .\dist\Jarvis-0.1.0-x64.msix
```

For distribution, sign the package and set `Publisher` in
`packaging/msix/AppxManifest.xml` to the certificate subject:

```powershell
signtool sign /fd SHA256 /a /f certificate.pfx /p <password> `
  dist\Jarvis-0.1.0-x64.msix
```

The same certificate should sign the installer and the executable to avoid
SmartScreen warnings.

## Assets

`python packaging/windows/make_assets.py` regenerates the committed icon and
MSIX tiles from code (standard library only). The icon contains 16–256 px BMP
entries for the executable, installer and shortcuts.

## Release workflow

`.github/workflows/release.yml` runs on `v*` tags: it builds the release app,
runs the packaging script, and uploads the portable zip, installer, MSIX and
checksums to a GitHub release. The workflow produces unsigned artifacts unless
signing is added to the release environment.
