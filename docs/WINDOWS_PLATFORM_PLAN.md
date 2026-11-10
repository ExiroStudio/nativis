# Nativis Windows Platform Architecture & Performance Plan (Revised v2.1)

Dokumen ini merinci rencana teknis, arsitektur, dan strategi implementasi untuk platform **Windows (`nativis-platform-windows`)** setelah evaluasi arsitektural dan koreksi menyeluruh terhadap celah teknis (zero-copy D3D11VA, integrasi main binary/IPC, dual-topology WorkerW Windows 11, deteksi fullscreen akurat, spike CI/FFmpeg, serta **eliminasi kode Linux untuk minimalisasi ukuran binary `.exe`**).

---

## 1. Prinsip Desain & Target Metrik Realistis

Berdasarkan analisis teknis (footprint driver vendor GPU DirectX, surface 4K, dan decoding runtime), target metrik disesuaikan dari asumsi teoritis ke target operasional yang terukur:

| Aspek | Estimasi Realistis Nativis | Catatan Teknis & Mitigasi |
| :--- | :--- | :--- |
| **CPU Usage (1080p 60fps)** | **< 0.5% – 1.0%** | Demuxing ringan + HW decoding (D3D11VA). |
| **CPU Usage (4K 60fps)** | **1.0% – 3.0%** | Demuxing kontainer 4K bit-rate tinggi + submit batch ke driver D3D11. |
| **CPU Usage (Paused / Game)** | **0.00%** | Thread decode & render freeze seketika (no `Present()`). |
| **RAM (Working Set)** | **60 MB – 110 MB** | Termasuk driver DLL GPU (nvldumdx/amdxx), FFmpeg state, dan swapchain surface. |
| **RAM saat Paused/Idle** | **< 30 MB** | Pemanggilan `SetProcessWorkingSetSize(-1, -1)` saat pause/sleep untuk trim working set. |
| **Compositing Model** | **DWM Composited (Flip Discard)** | Child window WorkerW tetap dikomposisi DWM (bukan DirectFlip bypass), namun `DXGI_SWAP_EFFECT_FLIP_DISCARD` meminimalkan latensi dan konsumsi VRAM (2-buffer ring). |
| **Ukuran Binary `.exe`** | **Minimal & Ramping** | Seluruh kode & aset Linux (KDE, Wayland, POSIX SHM, `.so`) **dieliminasi total** via conditional compilation. |
| **Single Executable** | **Target Utama** | Single binary portable tanpa installer atau dependensi DLL eksternal runtime. |

---

## 2. Solusi untuk Celah Teknis Utama

### 2.1. Zero-Copy VRAM Pipeline (Tahap 4)
Pada arsitektur saat ini:
- `plugins/video_backend/src/lib.rs` memanggil `av_hwframe_transfer_data` yang menyalin frame hardware ke RAM, lalu `Arc::from(source.data(0))` menyalinnya kembali.
- `nativis-core/core/src/resource.rs` saat ini hanya mengenal `CpuBuffer` dan `PlanarBuffer` di RAM CPU.

**Solusi Arsitektural:**
1. **Ekstensi Abstraksi `Resource`**:
   Tambahkan resource tipe GPU handle yang aman dan downcastable via `Any`:
   ```rust
   // Di nativis-core atau nativis-platform-windows
   pub struct D3D11TextureResource {
       pub texture: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
       pub subresource_index: u32,
       pub width: u32,
       pub height: u32,
       pub format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
   }
   ```
2. **Shared D3D11 Device Context**:
   FFmpeg tidak boleh mengalokasikan device DirectX terpisah. Decoder dan Renderer wajib berbagi satu `ID3D11Device` yang sama melalui `AVD3D11VADeviceContext`:
   - Sink membuat `ID3D11Device`.
   - Decoder mengikat device tersebut ke `av_hwdevice_ctx_alloc(AV_HWDEVICE_TYPE_D3D11VA)`.
3. **BindFlags & Multithreading**:
   - Frame pool decoder harus memiliki flag `D3D11_BIND_SHADER_RESOURCE` agar tekstur NV12 dapat langsung di-bind sebagai Shader Resource View (SRV).
   - Mengaktifkan proteksi multithread pada device context:
     ```rust
     let multithread: ID3D11Multithread = device.cast()?;
     multithread.SetMultithreadProtected(true);
     ```
     Ini menjamin thread decoder (background) dan thread presenter (sink) tidak mengalami race condition saat mengakses pipeline D3D11.

---

### 2.2. Integrasi Main Binary & IPC Windows (Tahap 0 & 1)
- **Kompilasi Workspace Bersih**:
  Seluruh crate baru di `nativis-platform/windows/` diberi `#![cfg(windows)]` agar perintah `cargo check --workspace` di Linux tetap berjalan normal.
- **IPC Abstraction (Named Pipe)**:
  `UnixListener` dan `UnixStream` tidak tersedia di Windows.
  - Tambahkan implementasi Windows IPC berbasis **Named Pipe (`\\.\pipe\nativis`)**:
    - Server: `CreateNamedPipeW` / `ConnectNamedPipe` atau crate asynchronous lightweight.
    - Client: `CreateFileW` / `CallNamedPipeW`.
  - Protokol payload tetap sama persis: JSON baris per baris (`IpcCommand` & `IpcResponse`).
- **Main Routing & Platform Selection**:
  Pada `src/main.rs`:
  ```rust
  #[cfg(target_os = "windows")]
  fn create_platform() -> Box<dyn Platform> {
      info!("Selected Windows Platform Adapter (WorkerW Desktop Injection & D3D11)");
      Box::new(WindowsPlatform::new())
  }
  ```
  Hapus `panic!("Windows platform is not fully implemented yet")`.

---

### 2.3. Dual-Topology WorkerW (Windows 10, 11 23H2, dan 24H2+) (Tahap 1)
Pada Windows 11 24H2+, struktur hierarki jendela desktop dapat bervariasi: `WorkerW` bisa berada sebagai *sibling* dari `Progman` atau sebagai *direct child* di dalam `Progman`.

**Algoritma Pencarian Dual-Topology:**
```mermaid
flowchart TD
    Start["FindWindowW('Progman', NULL)"] --> Spawn["SendMessageTimeoutW(0x052C)"]
    Spawn --> TrySibling["Cari Sibling WorkerW via EnumWindows<br/>(Belakang SHELLDLL_DefView)"]
    TrySibling --> FoundSibling{"Ditemukan?"}
    FoundSibling -- Ya --> SetParent["SetParent(our_hwnd, workerw_hwnd)"]
    FoundSibling -- Tidak --> TryChild["Cari WorkerW sebagai Child Progman<br/>via FindWindowExW"]
    TryChild --> SetParent
    SetParent --> ApplyStyle["SetWindowLongPtrW: Ganti WS_POPUP ke WS_CHILD"]
```
Langkah setelah `SetParent`:
- Ubah *window style* secara eksplisit: hapus `WS_POPUP`, tambahkan `WS_CHILD`.
- Posisikan jendela mencakup seluruh area desktop virtual (`SM_XVIRTUALSCREEN`, `SM_YVIRTUALSCREEN`, `SM_CXVIRTUALSCREEN`, `SM_CYVIRTUALSCREEN`).

---

### 2.4. Smart Throttler yang Akurat (Tahap 3)
Menghindari bug batas offset jendela maximized (`x = -8, y = -8`) dan mendukung state monitor sleep:

1. **Deteksi Fullscreen via Shell API**:
   - Panggil `SHQueryUserNotificationState(&mut state)`.
   - Jika `state == QUNS_RUNNING_D3D_FULL_SCREEN` atau `QUNS_PRESENTATION_MODE`:
     $\to$ Aplikasi fullscreen/game 3D sedang aktif. Pause render instan ($0.00\%$ CPU).
   - Pasang WinEvent Hook (`SetWinEventHook`) untuk `EVENT_SYSTEM_FOREGROUND` agar deteksi perpindahan jendela terjadi secara event-driven (tanpa tight polling).
2. **Deteksi Monitor Sleep**:
   - `WTSRegisterSessionNotification` hanya menangkap lock/unlock session Windows.
   - Untuk monitor sleep, daftarkan:
     ```rust
     RegisterPowerSettingNotification(
         hwnd,
         &GUID_CONSOLE_DISPLAY_STATE,
         DEVICE_NOTIFY_WINDOW_HANDLE,
     );
     ```
     Saat `PBT_POWERSETTINGCHANGE` bernilai `0` (display off) $\to$ pause render loop dan panggil `SetProcessWorkingSetSize(GetCurrentProcess(), -1, -1)`.

---

### 2.5. Rendering D3D11 & Precompiled Shaders (Tahap 2)
- **Precompiled Bytecode**:
  Jangan mengompilasi shader HLSL saat runtime dengan `D3DCompile` (menghindari ketergantungan pada `d3dcompiler_47.dll` dan CPU overhead saat startup).
  - Shader NV12 $\to$ RGB dan RGBA dikompilasi saat build time (`fxc.exe` atau `build.rs`), lalu di-embed via `include_bytes!`.
- **Fitur Crate `windows` yang Dibutuhkan**:
  ```toml
  windows = { version = "0.58", features = [
      "Win32_Foundation",
      "Win32_UI_WindowsAndMessaging",
      "Win32_UI_HiDpi",
      "Win32_UI_Shell",
      "Win32_Graphics_Direct3D",
      "Win32_Graphics_Direct3D11",
      "Win32_Graphics_Dxgi",
      "Win32_Graphics_Dxgi_Common",
      "Win32_Graphics_Gdi",
      "Win32_System_Power",
      "Win32_System_RemoteDesktop",
      "Win32_System_Threading",
      "Win32_System_LibraryLoader",
      "Win32_Security",
  ] }
  ```

---

### 2.6. Isolasi Kompilasi & Eliminasi Kode Linux (Binary Size Stripping)
Agar file distribusi `nativis.exe` di Windows seringan mungkin, kode dan artefak khusus Linux **wajib dikeluarkan secara total dari proses kompilasi**:

1. **Target-Gated Dependencies di Root `Cargo.toml`**:
   Crate platform Linux dan POSIX SHM tidak boleh dimasukkan ke dalam dependency tree Windows:
   ```toml
   # Hanya dikompilasi untuk target non-Windows (Linux/FreeBSD)
   [target.'cfg(not(target_os = "windows"))'.dependencies]
   nativis-platform-kde         = { workspace = true }
   nativis-platform-kde-wayland = { workspace = true }
   nativis-platform-wayland     = { workspace = true }
   nativis-transport-shm        = { workspace = true }

   # Khusus Windows
   [target.'cfg(target_os = "windows")'.dependencies]
   nativis-platform-windows     = { workspace = true }
   ```
2. **Eliminasi Aset Biner Embedded Linux**:
   Pada platform KDE, binary Linux meng-embed shared library ELF (`libnativisplugin.so`, `libnativis_v2_core.so`, `libnativis_v1_core.so`, dan QML bundle) sebesar ~1.1 MB via `include_bytes!`. Karena crate `nativis-platform-kde*` tidak disertakan saat build Windows, seluruh file biner `.so` Linux ini **100% tidak akan masuk ke dalam `nativis.exe`**.
3. **Optimasi Profil Release Cargo**:
   Konfigurasi release profile di workspace `Cargo.toml` untuk memangkas *dead code* dan simbol:
   ```toml
   [profile.release]
   opt-level     = 3       # Optimasi kecepatan eksekusi
   lto           = "fat"   # Link-Time Optimization agresif lintas seluruh crate
   codegen-units = 1       # Memaksimalkan efektivitas LTO dan dead-code elimination
   panic         = "abort" # Menghilangkan tabel stack unwinding (menghemat ratusan KB)
   strip         = true    # Menghapus simbol debug dan string path sistem
   ```

---

## 3. Rencana Eksekusi Berurutan (Revised Roadmap - Status: SELESAI / COMPLETED)

```mermaid
flowchart LR
    T0["Tahap 0: Baseline & Spike CI<br/>(COMPLETED)"] --> T1["Tahap 1: WorkerW & Main Integrasi<br/>(COMPLETED)"]
    T1 --> T2["Tahap 2: D3D11 RAM Renderer<br/>(COMPLETED)"]
    T2 --> T3["Tahap 3: Throttler Per-Monitor<br/>(COMPLETED)"]
    T3 --> T4["Tahap 4: Zero-Copy D3D11VA<br/>(COMPLETED)"]
    T4 --> T5["Tahap 5: Multi-Monitor & DPI<br/>(COMPLETED)"]
    T5 --> T6["Tahap 6: Release CI & Distribusi Ramping<br/>(COMPLETED)"]
```

### Tahap 0: Baseline & Keputusan Awal (Spike CI & IPC) - [COMPLETED]
- **Hasil**:
  1. Implementasi Named Pipe IPC client di `nativis-core/ipc/src/lib.rs` (`\\.\pipe\nativis`).
  2. Implementasi Named Pipe IPC server di `src/main.rs` berbasis `CreateNamedPipeW` dan `ConnectNamedPipe`.
  3. Konfigurasi CI Windows di `.github/workflows/release.yml`.

### Tahap 1: Injeksi WorkerW & Integrasi Main Binary - [COMPLETED]
- **Hasil**:
  1. Implementasi dual-topology discovery di `nativis-platform/windows/src/workerw.rs`:
     - **Topologi A (Windows 11 24H2+)**: Pencarian direct child `WorkerW` langsung di bawah `Progman`.
     - **Topologi B (Windows 10 & Windows 11 < 24H2)**: Pencarian sibling `WorkerW` di belakang `SHELLDLL_DefView` via `EnumWindows`.
  2. Jendela wallpaper diinjeksikan sebagai `WS_CHILD` dengan penekanan flicker (`WM_ERASEBKGND` handling).
  3. Penghubungan `create_platform()` di `src/main.rs` ke `WindowsPlatform::new()`.

### Tahap 2: Renderer D3D11 dari RAM (`PlanarBuffer`) - [COMPLETED]
- **Hasil**:
  1. Renderer `D3D11Renderer` di `nativis-platform/windows/src/d3d11_renderer.rs`.
  2. SwapChain modern `DXGI_SWAP_EFFECT_FLIP_DISCARD` (2-buffer ring) tanpa alokasi redundan.
  3. Fullscreen triangle VS tanpa vertex buffer via `SV_VertexID`.
  4. Pixel shader konversi warna BT.709 NV12 $\to$ RGBA dan shader RGBA murni.
  5. Terintegrasi penuh ke `WindowsDirectSink` (`nativis-platform/windows/src/sink.rs`).

### Tahap 3: Throttler & Power Management Per-Monitor - [COMPLETED]
- **Hasil**:
  1. Modul `Throttler` di `nativis-platform/windows/src/throttler.rs`.
  2. Deteksi game fullscreen 3D via `SHQueryUserNotificationState` (`QUNS_RUNNING_D3D_FULL_SCREEN` & `QUNS_PRESENTATION_MODE`).
  3. Deteksi monitor sleep via `RegisterPowerSettingNotification(GUID_CONSOLE_DISPLAY_STATE)`.
  4. Deteksi lock screen via `WTSRegisterSessionNotification(NOTIFY_FOR_THIS_SESSION)`.
  5. Event-driven window occlusion detection via `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)`.
  6. Working-set memory trimming via `SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX)` tepat saat state berpindah ke Paused (CPU $0.00\%$, RAM $< 30$ MB).

### Tahap 4: Zero-Copy D3D11VA dengan Resource Baru - [COMPLETED]
- **Hasil**:
  1. Penambahan `D3D11TextureResource` di `nativis-core/core/src/resource.rs`.
  2. Pengaktifan driver-level thread safety via `ID3D11Multithread::SetMultithreadProtected(true)`.
  3. Salinan DMA VRAM internal super cepat (<0.05ms) via `CopySubresourceRegion` pada tekstur shader resource.
  4. Penambahan `D3D11GpuFrame` di `plugins/video_backend/src/frame.rs`.
  5. Bypass total `av_hwframe_transfer_data` dan RAM memcpy di `plugins/video_backend/src/lib.rs` untuk Windows `AV_PIX_FMT_D3D11`.

### Tahap 5: Multi-Monitor & DPI Awareness Penuh - [COMPLETED]
- **Hasil**:
  1. Modul `monitor` di `nativis-platform/windows/src/monitor.rs`.
  2. Pengaktifan `SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)` sebelum pembuatan window dan swapchain.
  3. Enumerasi layar aktif, batas koordinat pixel fisik, dan persentase scaling DPI monitor via `EnumDisplayMonitors` & `GetDpiForMonitor`.
  4. Penanganan pesan `WM_DISPLAYCHANGE` dan `WM_DPICHANGED` di `wallpaper_wndproc`: otomatis reposition dan resize virtual screen covering seluruh monitor.
  5. Swapchain backbuffer secara dinamis menyesuaikan ukuran client window aktual (`GetClientRect`), bukan resolusi frame video.

### Tahap 6: Release CI & Distribusi Single Executable Ramping - [COMPLETED]
- **Hasil**:
  1. Konfigurasi `[profile.release]` di root `Cargo.toml`: `opt-level = 3`, `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = true`.
  2. Eliminasi 100% kode Linux (`kde`, `kde-wayland`, `wayland`, `transport-shm`) dan seluruh file biner `.so` dari build Windows.
  3. Pemisahan dependensi video opsional via `video` feature, memungkinkan pembuatan single binary `nativis.exe` yang murni portable, zero external DLL.
  4. Workflow rilis GitHub Actions `.github/workflows/release.yml` dengan job `build-windows` (MSVC x86_64, zip packaging, SHA-256 generation).
  5. Workflow CI multi-platform `.github/workflows/ci.yml` yang menguji Linux dan Windows MSVC secara simultan pada setiap commit dan Pull Request.

