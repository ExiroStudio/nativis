<div align="center">
  <h1>🌌 Nativis</h1>
  <p><b>High-Performance Cross-Platform Live Wallpaper Engine built with Rust</b></p>
</div>

Nativis is a blazingly fast, multi-process multimedia wallpaper engine designed for native integration across operating systems and desktop environments. Its core, media backends, and frame protocol are platform-agnostic; each platform supplies its own desktop integration. The current implementation starts with KDE Plasma, while the architecture is designed to grow beyond any single OS or DE.

## ✨ Features
- **Zero-Copy Rendering Overhead**: Uses POSIX Shared Memory (SHM) to transport high-resolution frames (e.g., 4K textures) across processes seamlessly.
- **Native Desktop Integration**: Injects directly into the host shell (e.g., `plasmashell`) rather than drawing a fake window behind your desktop icons.
- **Robust Architecture**: Built in Rust for memory safety, utilizing a modular plugin system for multimedia backends.
- **Cross-Platform Foundation**: Keeps media backends, runtime, frame protocol, and platform integration separate so native adapters can be added for other operating systems and desktop environments.
- **Single-Instance Guard**: Built-in IPC sockets prevent resource conflicts and memory tearing.

## 💎 What Makes Nativis Different?
- **Native Instead of a Fake Desktop Window**: Nativis uses the host desktop's native wallpaper layer rather than a borderless window workaround. KDE Plasma is the first implemented adapter for this approach.
- **Frame Protocol Built for More Than One Format**: The shared-memory protocol carries RGBA image frames and multi-plane NV12 video frames today, while keeping room for formats such as P010 in the future.
- **Media Backends Stay Decoupled from the Desktop**: Image and video backends only produce frames; the runtime, transport, and KDE consumer stay separate. This makes new input sources possible without coupling them to the desktop implementation.
- **4K-Friendly Data Path**: Video frames remain in NV12 through the runtime and shared-memory transport, reducing the amount of pixel data moved compared with a full RGBA video path.

## 🎞️ Media Support

| Media Type | Support Status | Notes |
|------------|----------------|-------|
| **Images** | 🟢 Stable       | Supports standard formats (JPG, PNG, etc.). |
| **Videos** | 🟢 Stable      | Supports standard resolutions and **4K video playback**. |
| **HTML5**  | 🚧 Planned      | Future support for interactive web wallpapers. |

> [!NOTE] 
> **Rendering Architecture:** Nativis now uses the **GPU** for parts of its rendering pipeline, including texture compositing and presentation through its WGPU/Qt OpenGL rendering paths. Media decoding and shared-memory transport still use CPU-side processing where required.

## 🗺️ What's Next?

| Direction | Plan |
|-----------|------|
| **External Frame Sources** | Allow other applications to provide frames to Nativis through the Nativis frame protocol. Sources that use a different API or pixel format can be supported through an adapter that converts them to the common frame contract. |
| **More Pixel Formats** | Extend the multi-plane transport beyond NV12, including P010 for 10-bit/HDR-capable workflows. |
| **More GPU Work** | Continue moving suitable rendering and color-conversion work to GPU paths while retaining CPU fallbacks where needed. |
| **Native Platform Adapters** | Add desktop integrations for more operating systems and desktop environments while keeping the shared core and frame protocol platform-independent. |
| **Simpler Linux Distribution** | Package the KDE integration so installation can move toward a user-space, no-sudo, single-binary experience. |

## 🚀 Platform & Desktop Support
Nativis is designed for multiple operating systems and desktop environments. **KDE Plasma 5 on X11 is the first available native integration**; the entries marked planned describe the intended expansion of the same core architecture.

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
