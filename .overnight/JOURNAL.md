# Overnight Engineering Journal (2026-11-23 / 2026-11-24)

All technical statements must be labeled:
- **PROVEN** — verified by test/execution/measurement with command + concise output
- **TRACED** — verified by reading code with `file:line` and snippet
- **DOC** — cited from official docs/headers/source with version
- **INFERRED** — hypothesis/conjecture (not used as basis for fixes)
- **UNVERIFIED** — code written but cannot be compiled/run in this environment

---

## [Phase 1.1] Capability Probe & Baseline (PROVEN)

### 1. Toolchain & Environment Probe
- `cargo 1.99.1 (5f94df478 2026-08-27)`, `rustc 1.99.1 (58cf8f9c3 2026-10-13)` (PROVEN)
- Installed rustup targets: (PROVEN)
  - `x86_64-unknown-linux-gnu` (host)
  - `x86_64-pc-windows-gnu`
  - `x86_64-pc-windows-msvc`
- Cross-check `x86_64-pc-windows-msvc`:
  - `cargo check --target x86_64-pc-windows-msvc -p nativis-platform-windows`: SUCCESS (1.60s) (PROVEN)
  - `cargo check --target x86_64-pc-windows-msvc -p nativis-plugin-video`: FAILS at ffmpeg-sys-next because Windows FFmpeg sysroot is not configured in pkg-config (PROVEN)
- System libraries and headers:
  - `Qt6Quick`: 6.11.2 (PROVEN via `pkg-config --modversion Qt6Quick`)
  - `Qt5Quick`: NOT FOUND in pkg-config (PROVEN via `pkg-config --modversion Qt5Quick`)
  - `libavcodec`: 62.28.103 (PROVEN)
  - `libavformat`: 62.12.103 (PROVEN)
  - `libavutil`: 60.26.103 (PROVEN)
  - `g++`: `/usr/bin/g++` (PROVEN)
  - `cmake`: `/usr/bin/cmake` (PROVEN)
  - `plasmashell`: `plasmashell 6.7.5` (PROVEN)
  - `kwin`: `kwin 6.7.5` (PROVEN)
  - `Xvfb`, `Xorg`, `kwin_x11`, `apitrace`, `valgrind`, `perf`: not installed / not in PATH (PROVEN)
  - `/dev/shm`: present and mounted rw (`drwxrwxrwt. 2 root root 80 Nov 24 20:59 /dev/shm`) (PROVEN)
- Desktop Session & Hardware:
  - `SESSION`: `wayland`, `WAYLAND_DISPLAY`: `wayland-0`, `DISPLAY`: `:0` (PROVEN)
  - GPU 0: Intel Raptor Lake-P [UHD Graphics] (rev 04) (driver `i915`, Mesa 26.2.5, VAAPI 1.23 via Intel iHD 26.1.5 with AV1, HEVC, H264, VP9 support) (PROVEN)
  - GPU 1: NVIDIA GeForce RTX 4050 Max-Q / Mobile (rev a1) (driver `nvidia`) (PROVEN)

### 2. Test Baseline (PROVEN)
- Command: `cargo test --workspace`
  - Result: 22 passed; 0 failed; 0 ignored; across 20 crates.
  - Specifically:
    - `nativis-event`: 1 passed
    - `nativis-ipc`: 4 passed
    - `nativis-platform-wayland`: 1 passed
    - `nativis-plugin-image`: 2 passed
    - `nativis-plugin-video`: 4 passed
    - `nativis-runtime`: 5 passed
    - `nativis-transport-shm`: 3 passed
    - `nativis-v1-core`: 1 passed
    - `nativis-v2-core`: 1 passed

---
