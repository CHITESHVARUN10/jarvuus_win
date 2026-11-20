# Architecture

Jarvis for Windows is a native Rust application that ports the macOS Jarvis
pipeline instead of wrapping it. The macOS project remains the behavioural
reference; its portable Rust policy code and its Python-free runtime choices are
the starting point.

## Crate graph

```text
crates/jarvis-app
  ├── crates/jarvis-core        OS-neutral paths, settings, setup model, logging
  ├── crates/jarvis-platform    Windows probes, hotkey, clipboard insertion
  ├── crates/jarvis-speech      capture, VAD, chunking, dictation controller
  └── crates/jarvis-models      downloads, verified releases, warm-up engines
              └── jarvis-core
```

- **jarvis-core** has no windowing or OS calls. It owns `AppPaths`,
  `AppSettings`, the setup report model, model file status, logging and the
  Ollama probe. All of it is unit-tested off Windows.
- **jarvis-platform** owns Windows-specific behaviour behind `cfg(windows)` and
  exposes a small preview stub elsewhere so the UI can be developed on macOS.
- **jarvis-models** owns model storage, ranged downloads, Hugging Face release
  resolution, checksum verification, atomic activation, the whisper-rs and ONNX
  engines, and the thread-based `ModelManager`. It depends only on core.
- **jarvis-speech** owns the cpal capture thread, resampling, VAD, chunking, the
  persistent Whisper worker, session epochs and the dictation lifecycle. It
  depends on core and the model crate's `WhisperEngine`.
- **jarvis-app** owns the Rune view, navigation, first-run gating, the setup probe
  and the model manager channel. The binary is a thin `main` that claims a
  single-instance name and runs the app.

The app runs on macOS as a UI preview; Windows-only probes report a preview
notice and the data directory is `JarvisWindows` to avoid touching a real macOS
Jarvis install.

## Startup and first-run flow

```text
main
  └─ rune::single_instance("com.chiteshvarun.jarvis")
       └─ Jarvis::new()
            ├─ AppPaths::resolve()          (JARVIS_DATA_DIR or %LOCALAPPDATA%\Jarvis)
            ├─ create data/models/logs
            ├─ load settings.json           (tolerant defaults)
            ├─ first run? Settings → Setup : Assistant
            └─ write launch line to logs/commands.log
       └─ first frame (setup rows show "Checking")
       ├─ model manager, speech, hotkey and insertion channels
       └─ AppSpec::setup
            ├─ store the Rune waker; attach model and dictation notifiers
            ├─ register Ctrl+Shift+D (Windows)
            └─ spawn probe thread → SetupReport → wake
       └─ AppSpec::poll
            ├─ drain ModelUpdate events → setup rows, essentials, progress
            ├─ drain SpeechEvent, HotkeyEvent and insertion results
            └─ drain SetupReport → baseline setup status
```

The probe is threaded because the Ollama check performs a localhost HTTP request
with a 900 ms deadline. Registry and file checks are cheap, but keeping all
probes on one worker means the UI never blocks on I/O.

`Check again` starts another single-flight probe. The thread wakes Rune through
the `Waker` supplied to `AppSpec::setup`; no polling loop is needed.

Demo scenes (`setup`, `downloading`, `ready`, `appearance`, `about`) install a
fixed report and model states before the setup hook, so headless snapshots are
deterministic.

## Setup checks

| Kind | Source | Ready when | Fix |
| --- | --- | --- | --- |
| Microphone | `HKCU\...\CapabilityAccessManager\ConsentStore\microphone` (device and `NonPackaged`) | neither scope is `Deny` | Opens `ms-settings:privacy-microphone` |
| Speech model | `<models>\ggml-large-v3-turbo.bin` metadata | exact size and ggml magic | Download / Resume / Load |
| Intent model | `current.json` receipt + required files in `versions\<dir>` | receipt valid, required files present | Download / Retry / Load |
| GPU | DXGI adapter enumeration (`CreateDXGIFactory1`) | at least one non-software adapter | Informational |
| Ollama | `GET 127.0.0.1:11434/api/tags` | server responds | Opens ollama.com |

Essentials are microphone, speech model and intent model (0–3). GPU, Ollama and
future checks are optional. Finishing setup persists `setup_completed` and opens
the assistant; it does not require all essentials to pass, matching the macOS
behaviour.

The probe is the on-disk baseline and stays cheap. Downloads and warm-ups report
live phases through the model manager; those updates also drive the row status,
the essentials count and the progress bar. Full checksum verification happens on
install (and on the intent engine's load selection), not on every startup probe.

## Windows permission model

Windows has no Accessibility or Automation grant equivalent to macOS.

- **Microphone.** Desktop apps normally have access unless the user turns off
  microphone access or desktop-app access in Windows privacy settings. The
  registry probe reads the device-wide and `NonPackaged` values and treats
  missing values as allowed. There is no request dialog to show; the setup row
  deep-links to `ms-settings:privacy-microphone` when access is off.
- **Text insertion.** `SendInput` sends Ctrl+V; it needs no grant. A non-elevated
  process cannot inject into an elevated foreground window (UIPI). `SendInput`
  returning zero leaves the transcript on the clipboard and reports
  `ClipboardOnly` instead of pretending the paste worked.
- **Global hotkeys.** `RegisterHotKey` needs no permission. `Ctrl+Shift+D`
  (dictate) is registered on a dedicated message-loop thread with
  `MOD_NOREPEAT`; `Ctrl+Shift+A` (command) arrives with Phase 4.
- **Launch at login.** Planned `HKCU\...\Run` or MSIX startup task; the system
  remains the source of truth and is re-read after toggling.

The macOS accessibility FFI, permission prompts, AppleScript automation errors
and `-1743` handling have no Windows equivalent and are not ported.

## Storage

`AppPaths` is the single source of truth:

| Path | Purpose |
| --- | --- |
| `settings.json` | Theme, accent, setup completion, hotkey choices |
| `logs/commands.log` | Append-only app/setup log |
| `models/ggml-large-v3-turbo.bin` | Whisper model |
| `models/ggml-large-v3-turbo.bin.part` | Resumable speech download |
| `models/intent/current.json` | Active managed intent release |
| `models/intent/last-good.json` | Last successfully warmed release |
| `models/intent/download-<commit>/` | Resumable intent staging |
| `models/intent/versions/<commit>-<nonce>/` | Immutable release directory |

`JARVIS_DATA_DIR`, `JARVIS_STT_MODEL_DIR` and `JARVIS_INTENT_MODEL_DIR` mirror
the macOS test seams. Windows uses `%LOCALAPPDATA%` rather than `%APPDATA%`
because models must not roam.

## Model lifecycle (implemented)

`jarvis-models::ModelManager` runs one single-flight job per model kind on a
worker thread and sends `ModelUpdate` events to the app. `Jarvis::poll` drains
them, so phase, progress, speed, ETA and errors drive the Setup rows and the
essentials count without blocking the window. The manager wakes Rune through the
same `Waker` used by the setup probe.

The first frame starts from cheap on-disk states (`Missing`, `Installed`). A
successful download warms the engine automatically; otherwise `Load` starts the
warm-up. Warm-up engines are created and dropped on the worker thread, so the UI
thread never owns a `WhisperContext` or an ONNX session. A persistent worker for
live transcription arrives with Phase 3.

### Intent model

Source: `chitesh12545/jarvis-intent-t5-small` on Hugging Face.

Flow, ported from the macOS `IntentModelRemote`/`IntentModelStorage` pair:

1. Resolve `main` once through the Hugging Face API and pin every request to that
   commit.
2. Download `model-release.json`, validate format version, runtime tag
   (`jarvis-t5-onnx-v1`), schema version, token limits, version syntax, file
   count, unique names, allowed names and Apache license notices.
3. Download files into `download-<commit>` with resumable range requests,
   rejecting over-size bodies and redirects outside `huggingface.co`/`hf.co`.
4. Verify each file's size and SHA-256, then `config.json` is a T5 export.
5. Rename the complete staging directory to `versions/<commit>-<nonce>` and write
   `current.json` atomically only after every check passes.
6. If the current receipt's manifest hash matches the server and the install
   verifies, report up to date. If a locally installed version is numerically
   newer, keep it instead of downgrading.
7. On warm-up, generate `"pause"` and require a non-empty JSON array of objects
   with a `name`; then record `last-good.json`.
8. If a downloaded release fails activation or warm-up, the previous working
   release remains selectable and the UI reports the error. There is no silent
   fallback to unrelated files.

Inference uses ONNX Runtime through `ort`. On Windows the target-specific
dependency downloads ort's pinned, DirectML-enabled ONNX Runtime 1.31
distribution at build time and links it into the binary; the engine offers the
DirectML execution provider first and always appends CPU as the fallback. This
covers NVIDIA, AMD and Intel iGPUs without a Python environment or a manual DLL
copy. On macOS the dev preview loads ONNX Runtime dynamically only when the
intent engine is actually used (`JARVIS_ORT_DYLIB`).

### Speech model

Source: pinned whisper.cpp release from Hugging Face.

- `ggml-large-v3-turbo.bin`, 1,624,555,275 bytes, SHA-256
  `1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69`.
- 8 MiB ranged requests, `Accept-Encoding: identity`, 300 s request timeout.
- `.part` resume; an invalid completed file is quarantined, not deleted.
- HTTP 200 on a ranged request restarts from zero; `Content-Range` and
  `Content-Length` are validated. An interrupted body keeps the partial for the
  next attempt.
- Full install verifies size, ggml magic and SHA-256 before renaming into place.
  Startup/load uses the cheap size + magic check so launch is not delayed by a
  1.6 GB hash.
- Cancelling keeps the partial. Resuming continues from the real byte count.

`whisper-rs` is CPU-only in the default build. The `gpu-vulkan` feature compiles
whisper.cpp with Vulkan for discrete and integrated GPUs and requires the Vulkan
SDK (`glslc`) at build time. A GPU build first loads with `use_gpu = true` and
retries with the CPU context if initialization fails; the backend actually used
is reported in the UI.

## Speech pipeline (implemented)

**Capture.** `jarvis-speech::audio` spawns a dedicated thread that owns cpal's
`!Send` stream. The stream opens at the device's native sample rate and channel
count; frames are averaged to mono and passed through a stateful linear
resampler to the 16 kHz contract. The capture buffer is bounded at 300 s and
keeps the beginning rather than rolling, because chunk offsets depend on it.
Level RMS is published at most every 50 ms. A watchdog reports a dead stream if
no buffers arrive for 3 s.

**Gate and chunks.** Silence is measured over 20 ms frames with an RMS floor of
0.004. A speech window needs at least six voiced frames (120 ms) and keeps 160 ms
of padding, so a click or breath never reaches Whisper. While recording, a
chunker snapshots the last 7 s every 6 s (1 s overlap) and sends only windows
that pass the gate. On stop, the final slice starts 1 s before the last sent
window; if a chunk failed or none were sent, it starts at zero. The worker
dedupes overlapping words and stitches the session transcript. A silent final
tail finalizes the partial text instead of discarding it.

**Worker.** One long-lived thread owns `WhisperEngine`, loaded lazily on the
first session. Each chunk gets a fresh decode state with English, greedy
sampling, `no_context`, `single_segment`, blank/non-speech suppression and four
threads. The worker only accepts commands for the current session id, so cancel,
restart and stale queued chunks cannot publish a late result. It unloads the
model after 300 s with no session activity.

**Lifecycle.** `DictationController` allows one active capture. `start` checks
the cheap on-disk model state, starts the stream, then sends `StartSession`.
`stop` joins the chunker, drops the stream, and sends the final buffered audio.
`cancel` advances the session, stops capture/chunker and sends `Cancel` without
emitting a final. Every app event carries the session id; the app drops stale
events. `abort` is the error path: it tears down capture without overwriting the
error state with an idle event.

**Hotkey and insertion.** A dedicated Windows thread registers
`Ctrl+Shift+D` with `MOD_NOREPEAT`, pumps `WM_HOTKEY`, and wakes Rune through the
app's waker. Registration failure is surfaced with a retry. Automatic insertion
snapshots the text clipboard, writes the transcript, sends Ctrl+V with
`SendInput`, and restores the snapshot only if the clipboard still holds the
transcript. If `SendInput` returns zero (elevated target/UIPI), the transcript is
left on the clipboard and the UI says so. The clipboard snapshot is text-only.

The macOS Apple `SpeechAnalyzer` path and Porcupine wake word are not part of the
Windows plan. Live listening is Whisper-only. A floating dictation pill, tray
state and configurable hotkeys arrive with Phase 5.

## Command plan (Phase 4)

The macOS `rust-core` already contains most of the portable policy layer. Phase 4
ports it and replaces the macOS effect layer:

| Portable | Needs a Windows implementation |
| --- | --- |
| `actions`, `validator`, `planner`, `intent_mapper`, `command_policy` | App launch/close (`ShellExecute`, process/window close) |
| `safety` (replace protected paths with `C:\Windows`, Program Files, ProgramData) | Volume (`IAudioEndpointVolume`) |
| `workspace`, `local_search`, `shuffle`, `system_volume` parsing | Media (`GlobalSystemMediaTransportControlsSession`) |
| Routine compilation and policy | Display brightness/contrast (WMI + DDC/CI) |
| Ollama parsing and prompts | Settings deep links (`ms-settings:`), Explorer, Windows Search |
| Cross-language fixture tests | Hotkeys and clipboard insertion |

Order of routing stays: native command → workspace/search → safety → rules →
learned intent model → Ollama. The model is only asked for ambiguous or compound
commands, and every failure path ends in a spoken/printed answer rather than
silence.

## CI

`.github/workflows/ci.yml` runs on Windows and macOS:

1. Node 22 is installed for developer tooling (Command Code, repo scripts).
2. Rust 1.97.1 with rustfmt and clippy.
3. `cargo fmt --all -- --check`
4. `cargo clippy --workspace --all-targets -- -D warnings`
5. `cargo build --workspace`
6. On Windows, copy `DirectML.dll` beside the test binaries.
7. `cargo test --workspace --all-targets`

Hardware tests (microphone capture, live model downloads) are `#[ignore]` and
run on a developer machine, not CI. CI has no microphone and never downloads the
1.6 GB model.

The headless Rune tests exercise the real view, click targets and state
transitions, so the setup flow is covered without a display.

The model crate adds native build work: whisper.cpp (CMake), SentencePiece
(static) and ONNX Runtime. Windows has no Vulkan SDK on the runner, so CI builds
the CPU speech path; `gpu-vulkan` is a release/developer build. Windows downloads
ort's pinned DirectML distribution and links it, so the Windows CI build also
proves the DirectML link path. Model weights are never downloaded in CI: the
network, resume, manifest and activation tests use a local HTTP server with tiny
fixtures.

## Vendored Rune crates

Rune pins patched copies of `winit`, `wgpu-hal`, `parley_engine` and
`accesskit_windows`. Cargo patches in a dependency do not apply to the workspace
that consumes it, so `vendor/` carries those crates and the root `Cargo.toml`
applies the same `[patch.crates-io]` entries. See [vendor/README.md](vendor/README.md).

## Out of scope

- Apple accessibility/text-insertion APIs, AppleScript automation, Spotlight,
  CoreAudio, DisplayServices, security-scoped bookmarks and the Apple speech
  framework.
- The Python voice-auth service: Windows phases drop speaker verification from
  the critical path and can add a native equivalent later.
- Training. Python training/export tooling remains in the macOS repository.
