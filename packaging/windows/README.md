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

For distribution, set `Publisher` in `packaging/msix/AppxManifest.xml` to the
certificate subject and sign everything with `sign.ps1` (below). The in-app
updater refuses unsigned artifacts, so a distributed release that should update
itself must be signed.

## Signing

```powershell
pwsh packaging/windows/sign.ps1 -Version 0.1.0 -Certificate C:\keys\jarvis.pfx
```

`sign.ps1` builds (unless `-SkipBuild`), signs `jarvis.exe`, every `.exe` in
`dist\` and the MSIX with `signtool`, using SHA-256 file digest, SHA-256
timestamp digest and a DigiCert RFC 3161 timestamp. It verifies each signature
with `signtool verify /pa /all` and then rewrites `SHA256SUMS.txt`, so the
published checksums describe the signed bytes.

The certificate password comes from `-Password`, else
`JARVIS_SIGNING_PASSWORD`; the path may also come from `JARVIS_SIGNING_CERT_FILE`.
Use a code-signing certificate (not a TLS certificate) from a CA such as DigiCert
or an internal enterprise CA whose root is deployed to client machines.

### Publisher pinning

The update page verifies the Authenticode signature before offering Install.
Set `JARVIS_UPDATE_PUBLISHER` on client machines to the expected certificate
subject or publisher name (comma-separated entries are accepted) to also pin the
signer. Without the variable the signature must still be trusted.

## Assets

`python packaging/windows/make_assets.py` regenerates the committed icon and
MSIX tiles from code (standard library only). The icon contains 16–256 px BMP
entries for the executable, installer and shortcuts.

## Release workflow

`.github/workflows/release.yml` runs on `v*` tags (or manually with a version
input): it builds and tests the app, runs the packaging script, and uploads the
portable zip, installer, MSIX and checksums to a GitHub release.

Signing is optional per repository. Set both secrets:

| Secret | Content |
| --- | --- |
| `WINDOWS_SIGNING_CERTIFICATE` | base64 of the `.pfx` file |
| `WINDOWS_SIGNING_PASSWORD` | the `.pfx` password |

When present, the workflow writes the certificate to a temporary file, runs
`sign.ps1 -SkipBuild`, deletes the file and verifies the installer signature
before uploading. Without the secrets it uploads unsigned artifacts and prints a
warning: those releases are manual-install only, because the updater refuses
unsigned downloads.
