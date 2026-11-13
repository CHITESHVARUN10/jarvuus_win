# Vendored Rune dependencies

These crates are copied verbatim from the pinned Rune revision
`6ba947e580f2d864bb333d913ba843284e57f427` (`anomalyco/rune`).

Rune ships modified copies of these published crates and documents that apps
building Rune must apply the same `[patch.crates-io]` entries. The `.patch`
files next to each directory are the original patch descriptions from Rune.

| Crate | Version | Purpose of the patch |
| --- | --- | --- |
| `winit` | 0.30.13 | IME helpers on macOS; Windows taskbar and window placement fixes |
| `wgpu-hal` | 30.0.1 | DXGI adapter exposure and partial presents on Windows |
| `parley_engine` | 0.12.0 | Bidi rule L1 and shaping cache improvements |
| `accesskit_windows` | 0.35.2 | Heading and landmark support in UI Automation |

Do not edit these files by hand. Re-copy them when the Rune revision changes.
