# Jarvis for Windows

A native Windows port of [Jarvis](https://github.com/CHITESHVARUN10/jarvuss), the
local-first macOS voice assistant. Speech, intent and command routing all run on
the machine. No Electron, no Python at runtime, no account.

The macOS project is cloned into `jarvuss/` for reference and is ignored by git.

## Status

**Phase 1 complete: first-run setup and the native app shell.**

- Rune (native GPU-rendered Rust UI) application window with navigation.
- First-run setup screen with essentials progress, status chips and per-item fixes.
- Windows microphone privacy probe and a deep link to Windows settings.
- GPU inventory probe (native DXGI) with Vulkan/DirectML capability messaging.
- Whisper and intent model presence checks against the managed data layout.
- Optional Ollama probe with a short localhost timeout.
- Persisted settings (theme, accent, setup completion) with atomic writes.
- Timestamped command log under the data directory.
- Cross-platform dev preview: the same app runs on macOS for UI work.
- GitHub Actions on Windows and macOS: formatting, clippy, tests, build.

## UI

![First-run setup](docs/screenshots/setup.png)

![Assistant](docs/screenshots/assistant.png)

![Appearance](docs/screenshots/appearance.png)

The setup screenshot uses demo statuses so all rows are visible; the real app
fills them from the Windows probes at launch.

## Stack

| Concern | Choice |
| --- | --- |
| Interface | [Rune](https://github.com/anomalyco/rune), pinned by revision |
| Speech to text | whisper.cpp via `whisper-rs`, `ggml-large-v3-turbo` |
| Intent | fine-tuned t5-small, int8 ONNX from [Hugging Face](https://huggingface.co/chitesh12545/jarvis-intent-t5-small) |
| Speech acceleration | Vulkan by default (NVIDIA, AMD, Intel iGPUs), optional CUDA build, CPU fallback |
| Intent acceleration | ONNX Runtime DirectML, optional CUDA build, CPU fallback |
| Local answers | Ollama, optional |
| Runtime | Rust only; Python stays in training tooling |

## Build

Windows prerequisites: Rust 1.97+, the MSVC build tools (the default
`x86_64-pc-windows-msvc` toolchain), and CMake. GPU builds of whisper.cpp will
also need the Vulkan SDK (`glslc`); the CPU path does not.

```powershell
cargo run -p jarvis-app
cargo test --workspace
```

On macOS the UI preview runs with the same command. Microphone and GPU probes
report that they are Windows-only, and the app uses `JarvisWindows` as its data
directory so it never touches a real macOS Jarvis install.

Headless renders for UI review (debug builds include Rune's tooling):

```powershell
cargo run -p jarvis-app -- --snap setup.png --scene setup
cargo run -p jarvis-app -- --snap ready.png --scene ready
cargo run -p jarvis-app -- --snap appearance.png --scene appearance
```

## Data layout

On Windows everything lives under `%LOCALAPPDATA%\Jarvis`:

```text
%LOCALAPPDATA%\Jarvis\
  settings.json                     theme, accent, setup completion
  logs\commands.log                 timestamped app and setup log
  models\
    ggml-large-v3-turbo.bin         speech model (about 1.5 GB)
    intent\                         managed intent releases
      current.json                  active release receipt
      last-good.json                last warmed release (phase 2)
      versions\<commit>-<nonce>\    immutable model files + manifest
```

Overrides for testing and custom installs: `JARVIS_DATA_DIR`,
`JARVIS_STT_MODEL_DIR`, `JARVIS_INTENT_MODEL_DIR`.

## Models

Speech: `ggml-large-v3-turbo.bin`, pinned to a whisper.cpp Hugging Face revision,
exactly 1,624,555,275 bytes and SHA-256
`1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69`.

Intent: `chitesh12545/jarvis-intent-t5-small`, exported as int8 ONNX
(`encoder_model_quantized.onnx` + `decoder_model_merged_quantized.onnx`),
installed through the same commit-pinned, receipt-checked layout as the macOS
app.

Phase 1 only reports presence. Phase 2 adds downloads, resume, checksum
verification, atomic activation, warm-up validation and GPU provider selection.

## Phases

| Phase | Scope | Status |
| --- | --- | --- |
| 1 | Workspace, Rune shell, first-run setup, Windows permission/GPU probes, CI | Done |
| 2 | Model manager: intent + speech download, resume, verify, activate, warm up, GPU providers | Next |
| 3 | Audio capture, VAD, Whisper transcription, dictation hotkey, text insertion | Planned |
| 4 | Intent inference, command planner/actions, Windows app/URL/volume/media/files/settings | Planned |
| 5 | Routines, insights, tray, startup, hotkey settings, UI parity | Planned |
| 6 | Packaging (MSIX/NSIS), release artifacts, performance hardening | Planned |

Architecture and porting notes: [ARCHITECTURE.md](ARCHITECTURE.md).

## Notes

- `vendor/` contains Rune's patched `winit`, `wgpu-hal`, `parley_engine` and
  `accesskit_windows` crates. Rune requires apps that embed it to apply the same
  patches; see [vendor/README.md](vendor/README.md).
- Node is installed in CI for developer tooling only. The shipped application has
  no Node or Python dependency.
