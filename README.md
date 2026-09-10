<div align="center">
  <h1>🌌 Nativis</h1>
  <p><b>Experimental native live-wallpaper engine for KDE Plasma 5 on X11</b></p>
  <p>Rust · FFmpeg · Qt 5 · POSIX Shared Memory</p>
</div>

Nativis is an experimental native live-wallpaper engine written primarily in Rust. It renders decoded media frames into a POSIX shared-memory surface which is consumed by a KDE Plasma wallpaper plugin, rather than placing a window behind the desktop.

## ✨ Current Status

The repository currently targets **KDE Plasma 5 on X11**. The implementation is still under active development; it should not be treated as a general-purpose or production-ready cross-platform wallpaper application.

| Area | Current implementation |
| --- | --- |
| Desktop integration | KDE Plasma 5 wallpaper package with a Qt 5 QML plugin |
| Display server | X11 |
| Frame transport | POSIX shared memory (`/nativis_shm`) |
| Image input | Local PNG, JPEG, WebP, BMP, GIF, and TGA files |
| Video input | Local MP4, MKV, WebM, AVI, MOV, TS, FLV, M4V, and WMV files, decoded with FFmpeg |
| Playback | Videos loop; the runtime targets 60 FPS |
| Other platforms / HTML wallpapers | Not implemented |

Image frames are converted to RGBA on the CPU. Video frames are decoded by FFmpeg on a background thread and transported as NV12 planes. The project contains a WGPU-based rendering abstraction, but the current KDE path passes CPU-accessible frames through shared memory; it is not a GPU-accelerated wallpaper pipeline.

## 🧩 Architecture

```text
local image/video file
        |
media backend (image or FFmpeg video)
        |
Nativis runtime
        |
POSIX shared memory: /nativis_shm
        |
KDE Plasma QML/OpenGL wallpaper plugin
```

The command-line binary enforces a single running instance on Unix using `/tmp/nativis.sock`.

## 🛠️ Requirements

Building the current KDE/X11 integration requires:

- Rust stable toolchain and Cargo
- CMake 3.16 or newer, a C++17 compiler, and `make`
- Qt 5 development components: Core, Gui, Qml, Quick, and OpenGL
- KDE Plasma 5 development/runtime tools, including `kpackagetool5`, `kbuildsycoca5`, and `qdbus`
- FFmpeg development libraries discoverable by `pkg-config` (for `ffmpeg-next`)

The automatic installer currently uses Debian/Ubuntu-style Plasma 5 locations and installs the system QML module under `/usr/lib/x86_64-linux-gnu/qt5/qml/org/nativis`. It may need changes on other distributions or architectures.

## 📦 Build from Source

Clone the repository and build the KDE bundle first:

```bash
git clone https://github.com/ExiroStudio/nativis.git
cd nativis
cargo xtask bundle-kde
```

`bundle-kde` builds the Rust C ABI library and the Qt/QML plugin, then assembles the Plasma wallpaper package in `platforms/kde-x11`.

Build the executable:

```bash
cargo build --release
```

For development, omit `--release` in the last command. Re-run `cargo xtask bundle-kde` whenever the KDE plugin or its Rust C ABI changes.

## 🎮 Run

Start Nativis with a local media path:

```bash
./target/release/nativis /absolute/path/to/wallpaper.mp4
```

On startup, the KDE platform bootstrap installs or refreshes the Plasma wallpaper package. Installing the Qt QML module may prompt for `sudo`, because that module is copied to a system Qt directory. Plasma is then asked to switch desktops to the `com.nativis.wallpaper` wallpaper plugin.

To force the platform bundle to be installed again:

```bash
./target/release/nativis --reinstall-platform
```

Use `Ctrl+C` to stop the engine. If a stale process still owns the single-instance socket, terminate that process before starting another instance.

## 🗂️ Repository Layout

| Path | Purpose |
| --- | --- |
| `src/main.rs` | CLI entry point and built-in backend registration |
| `nativis-core/` | Core contracts, runtime, asset handling, protocol, resources, and shared-memory transport |
| `plugins/` | Image and FFmpeg video media backends |
| `nativis-platform/kde-x11/` | KDE X11 platform bootstrap plus the Rust and Qt/QML plugin sources |
| `platforms/kde-x11/` | Assembled KDE Plasma wallpaper bundle |
| `xtask/` | `cargo xtask bundle-kde` build/packaging task |

## ⚠️ Known Limitations

- Only local file paths are supported; network URIs are not supported.
- Only the KDE Plasma 5/X11 path is implemented.
- The shared-memory sink is allocated for a maximum 3840×2160 RGBA frame plus protocol overhead.
- Platform installation is distribution- and architecture-specific at present.
- There is no supported command to select a different wallpaper or restore the previous one; KDE's wallpaper settings can be used after stopping Nativis.

## 📄 License

This project is licensed under the [Apache License 2.0](LICENSE).
