# Nativis Platform Independence & Isolation Policy

> **Core Principle**: Setiap platform desktop environment (DE) dan display server di Nativis **WAJIB** ditangani secara mandiri dan terisolasi (*independent & decoupled*), meskipun terdapat kode yang terkesan terduplikasi (*duplicate code*).

---

## 1. Latar Belakang & Rasionalitas

Nativis beroperasi pada level integrasi sistem grafis dan desktop environment yang sangat mendalam:
- Windowing system & compositor (X11 vs Wayland protocols)
- Desktop shells (KDE Plasma 5 vs Plasma 6 vs GNOME Shell vs Sway/Hyprland)
- Rendering toolkits (Qt5 vs Qt6, OpenGL vs Vulkan RHI, WGPU layer shell)

Mencoba menyatukan penanganan dua platform yang berbeda ke dalam satu folder/crate (misalnya memaksakan kode Wayland/Qt6 ke dalam `nativis-platform/kde-x11`) terbukti menjadi **anti-pattern berbahaya**:
1. **Regresi pada Platform Stabil**: Perubahan untuk mengakomodasi platform baru dapat merusak performa atau kompatibilitas platform yang sebelumnya sudah stabil.
2. **Perbedaan Fundamental Compositor**: X11 dan Wayland memiliki paradigma rendering, z-ordering, permission model, dan threading event-loop yang berbeda secara mendasar.
3. **Ketergantungan Toolkit yang Bertolak Belakang**: Plasma 5 berbasis Qt5 / X11 standard APIs, sedangkan Plasma 6 berbasis Qt6 RHI / Wayland subsurfaces.

---

## 2. Aturan Baku Arsitektur (Strict Architectural Rules)

### Rule 1: Strict Directory & Crate Isolation
Setiap platform wajib memiliki crate dan folder terpisah di `nativis-platform/<platform-name>/`:

| Directory | Crate Name | Target Lingkungan | Display Server | Status |
|-----------|------------|-------------------|----------------|--------|
| `nativis-platform/kde-x11/` | `nativis-platform-kde` | KDE Plasma 5 / 6 (X11) | X11 | 🟢 Stable (Frozen) |
| `nativis-platform/kde-wayland/` | `nativis-platform-kde-wayland` | KDE Plasma 6 (Wayland) | Wayland | 🚧 Active Development |
| `nativis-platform/wayland/` | `nativis-platform-wayland` | Hyprland, Sway, COSMIC | Wayland (`zwlr_layer_shell_v1`) | 🟢 Stable |
| `nativis-platform/gnome/` | `nativis-platform-gnome` | GNOME Shell | Wayland / X11 | 🚧 Planned |
| `nativis-platform/windows/` | `nativis-platform-windows` | Windows Desktop | DWM / WorkerW | 🚧 Planned |

### Rule 2: Toleransi Duplikasi Kode demi Stabilitas (Copy > Wrong Abstraction)
- Jika dua platform memiliki kebutuhan serupa (misalnya registrasi bundle, IPC handshake, atau C-ABI bindings), **lebih baik menduplikasi kode tersebut ke folder platform masing-masing** daripada membuat abstraksi bersama yang rapuh (*fragile shared abstraction*).
- Menghindari *cross-contamination*: Bug fix atau optimasi performa pada platform A tidak boleh menyentuh file apa pun di platform B.

### Rule 3: Isolasi Bundle Aset & Plugin
Setiap platform yang membutuhkan plugin eksternal (seperti QML plugin untuk Plasma) wajib memiliki direktori bundelnya sendiri:
- `platforms/kde-x11/`: Bundel khusus KDE X11.
- `platforms/kde-wayland/`: Bundel khusus KDE Wayland.

### Rule 4: Runtime Orchestrator Routing
File `src/main.rs` dan `nativis-core/ipc` bertindak sebagai *router murni*:
1. Mendeteksi lingkungan host (`XDG_CURRENT_DESKTOP`, `WAYLAND_DISPLAY`, `DISPLAY`).
2. Melakukan *branching* instansiasi ke platform adapter yang spesifik tanpa membocorkan logika internal platform ke core engine:
   ```rust
   let env = detect_environment();
   if env.is_kde && env.is_wayland {
       Box::new(KdeWaylandPlatform::new())
   } else if env.is_kde {
       Box::new(KdeX11Platform::new())
   } else if env.is_wayland {
       Box::new(GenericWaylandPlatform::new())
   } ...
   ```

### Rule 5: User-Space First & Zero-Sudo Policy (Distro-Agnostic Principle)
Semua platform Nativis **WAJIB berjalan 100% di user-space tanpa membutuhkan akses `sudo` atau hak akses root**:
- **Zero Sudo**: Tidak boleh menulis ke direktori sistem seperti `/usr/lib/...` atau memanggil `sudo`.
- **Standar XDG**: Semua instalasi ekstensi/plugin menggunakan direktori standar pengguna (`$HOME/.local/share/...`).
- **Relative QML Import (`import "."`)**: Plugin C++ QML (KDE Plasma 5 & 6) diletakkan di dalam folder bundle pengguna (`contents/ui/`) dan diimpor menggunakan `import "."`. Hal ini menjamin wallpaper berjalan identik di semua distro Linux (Ubuntu, Fedora, Arch, Void, openSUSE, dll.) tanpa terpengaruh perbedaan arsitektur folder library sistem (`/usr/lib/x86_64-linux-gnu` vs `/usr/lib64` vs `/usr/lib`).
- **Protokol Standar**: Platform Wayland murni (Hyprland, Sway, dll.) menggunakan protokol `zwlr_layer_shell_v1` langsung melalui socket Wayland pengguna (`$WAYLAND_DISPLAY`) tanpa perlu menginstal file apa pun ke sistem.

---

## 3. Checklist Sebelum Melakukan Perubahan Platform

- [ ] Pastikan Anda tidak mengubah file di dalam `nativis-platform/kde-x11/` jika target pekerjaan Anda adalah Wayland.
- [ ] Pastikan platform baru memiliki `Cargo.toml` dan `CMakeLists.txt` (jika ada C++) yang sepenuhnya independen.
- [ ] Uji platform yang baru tanpa mengubah perilaku platform yang sudah berstatus `🟢 Stable`.
