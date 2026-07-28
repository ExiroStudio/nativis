<div align="center">
  <h1>🌌 Nativis</h1>
  <p><b>High-Performance Native Live Wallpaper Engine built with Rust</b></p>
</div>

Nativis is a blazingly fast, multi-process multimedia wallpaper engine designed to integrate deeply with your desktop environment. It bypasses traditional overlay windows by pushing pixel data directly into the compositor's native rendering pipeline using POSIX Shared Memory (SHM).

## ✨ Features
- **Zero-Copy Rendering Overhead**: Uses POSIX Shared Memory (SHM) to transport high-resolution frames (e.g., 4K textures) across processes seamlessly.
- **Native Desktop Integration**: Injects directly into the host shell (e.g., `plasmashell`) rather than drawing a fake window behind your desktop icons.
- **Robust Architecture**: Built in Rust for memory safety, utilizing a modular plugin system for multimedia backends (Images, Videos, HTML5).
- **Single-Instance Guard**: Built-in IPC sockets prevent resource conflicts and memory tearing.

## 🚀 Supported Environments
Nativis is built to be cross-platform, but currently focuses on deep integration with Linux Desktop Environments.

| Platform / Desktop | Display Server | Support Status | Method |
|--------------------|----------------|----------------|--------|
| **KDE Plasma 5**   | X11            | 🟢 **Stable**  | Native System QML Plugin (C++ `NativisItem`) |
| **KDE Plasma 6**   | Wayland        | 🚧 Planned     | Layer Shell / KWin Ext |
| **GNOME**          | Wayland / X11  | 🚧 Planned     | TBD |
| **Windows**        | DWM            | 🚧 Planned     | WorkerW Injection |

## 🛠️ Prerequisites
To compile Nativis, you will need the following dependencies installed on your system:
- **Rust Toolchain** (latest stable)
- **CMake** & **Make**
- **KDE & Qt5 Development Headers**: `qtdeclarative5-dev`, `plasma-workspace-dev`, `kpackagetool5`

## 📦 Building from Source
Nativis uses a custom Cargo `xtask` to orchestrate the build process across Rust and C++ components.

1. **Clone the repository**
   ```bash
   git clone https://github.com/ExiroStudio/nativis.git
   cd nativis
   ```

2. **Build the Desktop Integration Bundle**
   ```bash
   cargo xtask bundle-kde
   ```
   *This command compiles the Rust C-ABI core and the KDE Plasma C++ QML Plugin.*

3. **Build the Engine**
   ```bash
   cargo build --release
   ```

## 🎮 Usage
Running Nativis is extremely simple. Just pass the path to your media file:

```bash
./target/release/nativis /path/to/your/wallpaper.jpg
```

**Note on System Installation (KDE Plasma):** 
The very first time you run Nativis, it will detect if its native renderer plugin is installed in your system Qt directory (`/usr/lib/x86_64-linux-gnu/qt5/qml/org/nativis`). If it's missing, Nativis will prompt you for your `sudo` password to copy the plugin to the system path. This only happens **once**.

To exit Nativis and pause the engine, simply press `Ctrl+C` in the terminal.

## 📄 License
This project is licensed under the MIT OR Apache-2.0 License.
