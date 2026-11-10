# Architecture Review — November 2026

Findings and plans from a code review of `main` at commit `4fe558f`
(Windows video support, embedded FFmpeg bootstrapper). Durable decisions are recorded in
[`DECISIONS.md`](DECISIONS.md); this document holds the analysis and the proposals that
still need a maintainer decision.

## 0. How to read this document

Every claim carries an evidence level:

| Tag | Meaning |
|---|---|
| **[verified]** | Read directly in the code at the commit above. |
| **[measured]** | Measured on a Linux harness; see [`MEMORY_ALLOCATION.md`](MEMORY_ALLOCATION.md) (Indonesian). |
| **[inferred]** | Follows from the code, but was **not reproduced** on a real Windows host. Treat as a hypothesis and verify with the steps given. |

What was **not** done: the Windows crates and `video_backend` were not compiled or run in
the review environment, and nothing was run on a real Windows host. The Windows findings
below are therefore code analysis plus the maintainer's observations.

Contents: [1 Windows findings](#1-windows-findings) ·
[2 Weaknesses vs. the plugin-first goal](#2-weaknesses-that-conflict-with-the-plugin-first-goal) ·
[3 Proposed direction](#3-proposed-direction-pending-maintainer-decision) ·
[4 CPU and RAM plan](#4-cpu-and-ram-reduction-plan) ·
[5 Open questions](#5-open-questions)

---

## 1. Windows findings

### 1.1 Video does not play on the Windows host, but images do

**Observations from the maintainer.** On a real Windows host, images display and video
does not. In a Windows VM (no GPU), both display; a 4K video used about 80% of 4 cores and
about 200 MB RAM.

**Why the VM works and the host does not [inferred].** The two environments take
different code paths:

| | VM (no GPU) | Host (real GPU) |
|---|---|---|
| Decode | D3D11VA unavailable → software fallback | D3D11VA decode succeeds |
| Frame resource | `PlanarBuffer` (NV12 in RAM) | `D3D11TextureResource` (GPU texture pointer) |
| Sink path | `draw_nv12()` (CPU → `Map` → texture) | `draw_d3d11_texture()` (GPU copy) |

Images use the `CpuBuffer` → `draw_rgba()` path, which is the same on both machines. The
only path that differs between "works" and "does not work" is the **zero-copy D3D11
texture path**, which only runs when a real GPU decoder exists. That narrows the cause to
the code below.

**Hypothesis H1 — the decoder's texture belongs to a different D3D11 device (high
confidence).**
- The decoder gets its device from `av_hwdevice_ctx_create(D3D11VA, NULL, ...)` in
  `configure_hardware_decoder` (`plugins/video_backend/src/lib.rs`). FFmpeg creates **its
  own** `ID3D11Device`. **[verified]**
- The renderer creates a **separate** device with `D3D11CreateDevice(...)`
  (`nativis-platform/windows/src/d3d11_renderer.rs`, `D3D11Renderer::new`). **[verified]**
- `draw_d3d11_texture` then calls `CopySubresourceRegion` with the decoder's texture as the
  source and a renderer-owned texture as the destination. **[verified]** A D3D11 resource
  cannot be used by another device's context. This is invalid use, and
  `CopySubresourceRegion` returns `void`, so no error is reported to the caller.
  **[inferred]** Typical symptoms are a black or green frame (an NV12 texture that was never
  written decodes to dark green with the BT.709 shader), a frozen picture, or a driver
  reset, depending on the GPU driver.
- Result: `submit()` reports success, `present()` runs, and nothing useful reaches the
  screen. There is no log line because nothing returned an error.

**Hypothesis H2 — decoder surface pool exhaustion (medium confidence).** In the zero-copy
branch each decoded frame is kept alive with `av_frame_clone`, and frames are held in the
bounded(2) channel, in `cached_frame`, and in `last_handle`. No `extra_hw_frames` or custom
`hw_frames_ctx` is configured (no match in the crate). **[verified]** FFmpeg's default D3D11VA
pool is sized to the codec's reference needs; holding several extra surfaces can leave the
decoder with no free surface, which stalls or errors decoding. **[inferred]**

**Hypothesis H3 — different GPU adapters on hybrid machines (low–medium).** Both the decoder
and the renderer ask for the default adapter independently. On laptops with an integrated
and a discrete GPU, nothing guarantees they pick the same one. **[inferred]**

**There is no fallback.** `video_backend` only restarts in software mode when
`av_hwframe_transfer_data` fails (the CPU-readback branch). It never learns that the sink
could not use a D3D11 texture, because `FrameSink::submit` has no way to report "I cannot
use this kind of surface" back to the source. **[verified]** This also violates D7.3 (silent
fallbacks are bugs).

**How to verify on the host** (run from a terminal so logs are visible):

```powershell
$env:RUST_LOG = "info"
.\nativis.exe --no-gui C:\path\to\clip.mp4
```

1. Look for `[NATIVIS DECODE] Opened: ... mode=d3d11va`. If present, the zero-copy path is
   active.
2. A/B test: `$env:NATIVIS_DECODE = "software"` and run again. If video now plays, H1/H2/H3
   are confirmed as GPU-path problems rather than a codec or file problem.
3. Report what the screen shows (black, green, frozen, previous image) and any
   `D3D11 draw_d3d11_texture error` or decoder warnings.
4. Task Manager → Details → GPU columns, to confirm which adapter is used.

**Fix options, in order of cost.**

| # | Option | Effect | Notes |
|---|---|---|---|
| A | On Windows, keep D3D11VA **decode** but read frames back with `av_hwframe_transfer_data` (the existing CPU-readback branch) instead of passing texture pointers | Works on every GPU; decode stays on the GPU; costs one GPU→RAM copy per frame | Consistent with D6; recommended immediate fix |
| B | Make the decoder use the **renderer's device**: allocate the hw device with `av_hwdevice_ctx_alloc(D3D11VA)`, set `AVD3D11VADeviceContext.device` to the renderer's `ID3D11Device`, then `av_hwdevice_ctx_init` | True zero-copy on one device | Needs the platform to hand its device to the source, which is exactly the negotiation problem in section 3. Also size the pool (`extra_hw_frames`) |
| C | Decoder copies into a **shareable** texture (`D3D11_RESOURCE_MISC_SHARED_NTHANDLE` + keyed mutex or fence); the renderer opens it with `OpenSharedResource1` | Zero-copy across devices and processes | Prerequisite for out-of-process decoding; more code |
| D | Add sink→source feedback so a rejected surface downgrades the source automatically | Removes the silent-failure class | Part of section 3 |

Recommended order: A now, D with the surface negotiation, then B or C.

### 1.2 Multi-monitor shows one big canvas instead of one wallpaper per monitor

**Cause [verified].** The engine has no concept of an *output* (monitor). The whole pipeline
is built around one window, one swapchain and one frame:

- `WorkerWContext::new` sizes the wallpaper window with `get_virtual_screen_bounds()`, the
  bounding rectangle of **all** monitors (`SM_XVIRTUALSCREEN`/`SM_CXVIRTUALSCREEN`, ...), in
  `nativis-platform/windows/src/workerw.rs` and `monitor.rs`.
- `WindowsPlatform::create_sink` creates **one** `WindowsDirectSink` with that width and
  height, hence one `D3D11Renderer` and one swapchain covering the virtual desktop.
- The vertex shader draws a single full-screen triangle with UV 0..1 and the pixel shaders
  sample the whole source. So the one frame is **stretched across the whole virtual
  desktop**, with no aspect-ratio handling (no fill/fit/center).
- `enumerate_monitors()` is called, but it only logs; its results are not used to build
  anything.
- The IPC command `SetWallpaper { uri, monitor, scaling }` carries both fields, but the
  daemon ignores them (`IpcCommand::SetWallpaper { uri, .. }` in `src/main.rs`), and the
  CLI always sends `monitor: None, scaling: None`.
- The daemon owns exactly one backend and one sink (`Conductor::run_with_commands`).

This is not a WorkerW bug. The missing piece is in the model, and it applies to every
platform, not only Windows.

**Proposed model [proposed].**

1. **Output** becomes a first-class concept in the platform contract: the platform reports
   a list of outputs, each with a stable id, pixel rectangle, scale/DPI, refresh rate, and
   change notifications (hot-plug, resolution change).
2. The engine creates **one presenter per output**; each presenter has its own swapchain
   or surface, vsync and throttle state. On Windows this means one child window per monitor
   (positioned relative to the virtual-screen origin, which can be negative) with its own
   swapchain, rather than one giant window.
3. Each output is bound to a **source instance**, or shares one source with a per-output
   viewport. A video shown on N outputs should be decoded **once** and presented N times;
   this falls out of the surface model in section 3.
4. **Scaling** is applied by the presenter per output: `fill` (cover), `fit`
   (letterbox), `stretch`, `center`, `tile`, and `span` (one source across all outputs,
   which is today's behavior and should stay available on purpose).
5. IPC `monitor` and `scaling` become real: `None` means all outputs.

Platform notes: no per-monitor handling was found in the other platform crates or the
QML/C++ bundles under `platforms/` either (a search for "monitor"/"screen" returns
nothing). **[verified]** So this is a gap in the contract, not a Windows-only bug. The fix
is a new `outputs()` capability on the `Platform` trait, with each platform implementing it
on its own (D4), not shared code.

---

## 2. Weaknesses that conflict with the plugin-first goal

These are the places where the current design cannot satisfy D1 ("any source on any
platform without engine changes").

### W1 — The sink guesses resource types instead of negotiating

`WindowsDirectSink::submit` tries `downcast_ref` for `D3D11TextureResource`, then
`PlanarBuffer`, then `CpuBuffer`, and otherwise warns "Unknown frame resource type".
**[verified]**

- A backend cannot know what a sink accepts, and a sink cannot say no in a way the backend
  can act on (see 1.1, option D).
- Every new resource kind forces edits in every sink. That directly breaks D1.4.
- `D3D11TextureResource` (a raw `texture_ptr: usize`) lives in `nativis-core`, so a
  platform type leaks into core. That breaks D1.3.
- `Resource` is `Box<dyn Resource>` with `Any` downcasting inside a global
  `Mutex<HashMap>`. `TypeId` comparisons are not reliable between separately compiled
  plugins, and `Arc`/`Box<dyn>` do not belong across an ABI boundary (D3.1).

### W2 — "Frame" is the wrong unit for sources that are not frame producers

HTML and "other programs" own their own window or surface. Forcing them through
`Frame` means capture → copy → upload every tick, which is the most expensive path
available and the opposite of the efficiency goal (D5).

### W3 — Cadence is a fixed 60 Hz poll

`Conductor::run_with_commands` loops at `target_fps` (default 60) with a `sleep`, calling
`update()` and `current_frame()` on every tick, even when nothing changes (a static image
wakes the process 60 times per second). Decoding is paced only indirectly through the
bounded(2) channel. **[verified]** The runtime itself notes that "a real engine uses
vsync/presentation feedback".

### W4 — No output model and no sink→source feedback

See 1.2 and 1.1 (option D).

### W5 — Smaller issues found on the way

- IPC `SetWallpaper` opens the **new** backend (decoder thread starts) before
  `RuntimeCommand::SetBackend` closes the old one, so two decoders briefly coexist.
  **[verified]** Peak memory effect not measured.
- `ImageBackend::close()` does not free its `CpuBuffer` handle (about 32 MB per 4K change).
  **[measured]**
- `D3D11CreateDevice` uses `D3D_DRIVER_TYPE_HARDWARE` only; there is no WARP fallback, and
  no log tells the user when the only adapter is the software "Basic Render Driver".
  **[verified]**
- Windows `sink.submit()` blocks the runtime thread while throttled and while in
  `Present(1)`. **[verified]**

---

## 3. Proposed direction (pending maintainer decision)

Nothing in this section is decided. It is the smallest design found that satisfies D1–D6.
The **frame concept stays**; what changes is *what a frame carries* and *who owns the
memory*.

### 3.1 Three source classes, one small ABI

| Class | Examples | What crosses the boundary |
|---|---|---|
| **Producer** | video, camera, capture | A reference to a surface in a negotiated buffer pool, plus a fence |
| **Declarative** | still images, shaders | A texture uploaded once, or a shader program plus uniforms. The host supplies time. No per-frame data |
| **Embedded** | HTML, other programs | A region (parent window / surface handle + rectangle) provided by the platform. The source renders itself; the engine only manages lifecycle, size, visibility and throttling |

The engine core handles only these three classes, never concrete source types.

### 3.2 Surface descriptor (sketch)

`nativis-protocol` already defines `repr(C)`, `no_std` types (`NativisFrameHeader`,
`NativisAttachment`) with formats `RGBA8888`, `NV12` and `P010`. **[verified]** The proposal
is to **extend that vocabulary** rather than invent a second one: add a surface `kind`, a
handle per plane and a fence.

```c
// Sketch only. Layout, names and versioning are open (section 5).
typedef struct {
  uint32_t struct_size;      // version by size; consumers ignore unknown tail
  uint32_t kind;             // CPU_SHM | DMABUF | D3D11_SHARED | ...  (extensible)
  uint32_t format;           // NATIVIS_FORMAT_* (RGBA8888, NV12, P010, ...)
  uint32_t width, height;
  uint64_t modifier;         // layout/tiling for GPU kinds, 0 for CPU_SHM
  struct { int64_t handle; uint32_t offset, stride; } plane[4];
  int64_t  fence;            // fd / NT handle / keyed mutex; 0 = none
  uint32_t flags;            // opaque, color range/primaries, alpha, ...
  uint32_t slot;             // index into the negotiated buffer pool
} NativisSurface;
```

Platform types such as `ID3D11Texture2D*` never appear in core; they are encoded as a
`kind` + OS handle that only the matching platform sink understands.

### 3.3 Negotiation

1. The **sink** advertises the surface kinds and formats it accepts, in preference order.
2. The **source** advertises what it can produce.
3. The core picks the best common `kind`; if none, it inserts a converter (for example
   GPU → `CPU_SHM`). `CPU_SHM` is always available (D6).
4. The sink can **reject** a surface with a reason; the source downgrades and logs it
   (D7.3). This closes 1.1 option D.
5. For same-device GPU paths, the platform exposes its device through the host API during
   negotiation (this is what option B in 1.1 needs).

### 3.4 Data plane and control plane

- **Control plane** (rare): negotiate, open, close, resize, pause, outputs changed.
  Small versioned messages.
- **Data plane** (per frame): the producer and consumer agree once on N buffers (a pool).
  After that only `(slot, fence)` moves per frame. Cost is O(1) regardless of resolution,
  there is no per-frame allocation, and the number of in-flight frames is fixed.
- Buffers are released with an explicit `release(slot)` call (D3.2).

### 3.5 Event-driven cadence

Replace the fixed 60 Hz poll with deadlines and events: a source reports "next frame at
T" or "idle" (static content = zero wakeups); a presenter reports vsync, occlusion and
pause. Throttling (fullscreen app, locked session, battery) is handled once, in the
presenter, per output.

### 3.6 Out-of-process sources (later, gated by evidence)

Heavy sources may run as a separate process (same executable in a worker mode, per D5).
The ABI then becomes a wire protocol (descriptor + OS handle passing: `SCM_RIGHTS` on
Unix, `DuplicateHandle` on Windows). Benefits: crash isolation, language independence,
RAM returned to the OS when the process exits. Costs: a few MB per process and handle
passing. **Gate:** only after the in-process zero-copy work in sections 3.2–3.4 leaves a
measured RAM problem.

### 3.7 Relation to D4 (platform isolation)

The descriptor is a **data contract in core**. Each platform still writes and owns its own
sink and its own handle handling, including duplicated code where that is clearer.

### 3.8 Migration path (no rewrite)

| Phase | Change | Exit criterion |
|---|---|---|
| 0 | Quick fixes that need no contract change: Windows option A, `ImageBackend` free, close-before-open, log the active decode mode and adapter (section 4) | Host video plays; image RAM flat across wallpaper changes |
| 1 | `NativisSurface` + negotiation, with only `CPU_SHM` and **one GPU kind per platform**. `ResourceHandle` stays internally behind an adapter | A new surface kind can be added without editing any existing sink |
| 2 | Event-driven cadence | Static image: 0 wakeups/s |
| 3 | Output model + per-output presenters + IPC `monitor`/`scaling` | One wallpaper per monitor on Windows and one Linux platform |
| 4 | Embedded source class with a first HTML or window source | Added without touching core |
| 5 | Out-of-process worker (only if measured necessary) | RAM drops when the worker exits |

---

## 4. CPU and RAM reduction plan

Measurements and the formula are in [`MEMORY_ALLOCATION.md`](MEMORY_ALLOCATION.md). Summary:
RAM ≈ `frames_held × W × H × 1.5` for 8-bit NV12 (about double for 10-bit); software
frame-threading holds about 5.5 frames with 1 thread and about 14 with 4 threads **[measured]**.

### 4.1 What the 80% CPU / 200 MB in the VM most likely is [inferred]

- Software decode of 4K with `available_parallelism − 1` = 3 threads, because there is no
  GPU decoder in the VM (`software_thread_count` in `video_backend`).
- Rendering on the Microsoft Basic Render Driver (CPU rasterizer), because the renderer
  requests `D3D_DRIVER_TYPE_HARDWARE` and the VM only exposes the basic adapter.
- 200 MB matches the measured software 4K figure (about 183 MB anonymous with 4 threads).
- Memory copies are **not** the main CPU cost: three copies of a 12 MB 4K NV12 frame at
  30 fps is roughly 1.1 GB/s, about one tenth of a core on a typical machine. Decode and
  rendering dominate. The data-layout work in section 3 mainly buys RAM, zero-copy and ABI
  stability, not a large CPU drop by itself.
- Therefore VM numbers are not representative of a machine with a GPU. Measure on the host
  or a VM with GPU passthrough.

### 4.2 Plan, highest impact first

| # | Action | Targets | Needs contract change? |
|---|---|---|---|
| 1 | Use hardware decode everywhere it exists, with CPU readback as the baseline (D6). On Windows, option A in 1.1 | CPU | No |
| 2 | Log the decode `mode` and the D3D11 adapter name at `info`; warn on software fallback and on Basic Render Driver | Visibility (D7.3) | No |
| 3 | `ImageBackend::close()` frees its handle; drop the CPU buffer after upload/copy; `malloc_trim(0)` after a wallpaper change on glibc | RAM | No |
| 4 | Close the old backend **before** opening the new one | RAM peak | No |
| 5 | Video: `bounded(2)` → `bounded(1)`, drop `cached_frame`, pick decode threads by resolution (frame threading doubles frames held) | RAM | No |
| 6 | Static content: upload to the GPU once, drop the CPU copy, **0 wakeups** | CPU, RAM | Cadence (phase 2) |
| 7 | Downscale to output resolution on the CPU path when the source is larger than the monitor | RAM | No |
| 8 | Zero-copy through a negotiated surface kind (DMA-BUF on Linux, shared D3D11 on Windows) | RAM, CPU | Phase 1 |
| 9 | Decode once, present on N outputs | CPU, RAM on multi-monitor | Phase 3 |
| 10 | Worker process for decoding | RAM returned to the OS | Phase 5, evidence-gated |

Items 1–5 and 7 are small, independent changes. Items 6, 8 and 9 follow from the
architecture work.

### 4.3 Verification checklist

```bash
# Linux: memory split of the daemon
grep -E "Rss(Anon|File|Shmem)" /proc/$(pgrep -f 'nativis.*--no-gui' | head -1)/status

# Is decode on hardware or software?
RUST_LOG=info nativis --no-gui <video> 2>&1 | grep "NATIVIS DECODE"
```

```powershell
# Windows: same check, plus an A/B test against software decode
$env:RUST_LOG = "info"; .\nativis.exe --no-gui C:\path\clip.mp4
$env:NATIVIS_DECODE = "software"   # A/B
```

Record, for each change: decode mode, adapter, `decode_fps`, process CPU %, RSS (private
bytes on Windows), and whether the source was 8-bit or 10-bit. Set numeric targets only
after a baseline exists on a machine with a GPU.

---

## 5. Open questions

1. Final layout and versioning of `NativisSurface`; extend `nativis-protocol` or add a new
   crate?
2. How does a platform hand its GPU device to a source without leaking platform types
   into core (host API interface queried by name/version)?
3. Output identity across hot-plug and reordering: use platform ids (EDID/`DeviceName`,
   `wl_output` name), or a Nativis-assigned stable id stored in config?
4. Scaling modes: is `span` a scaling mode or a separate "group outputs" setting?
5. Should the `Embedded` class be allowed on every platform, or only where the platform can
   host a foreign window (Windows child HWND, Wayland layer-shell client)?
6. When does the worker process become worthwhile (RAM threshold that justifies it)?

## 6. Provenance

- Code read at `ExiroStudio/nativis` `main`, commit `4fe558f`.
- RAM numbers come from a harness that uses the real `ImageBackend` and `ShmSink`; the
  video numbers use the FFmpeg CLI as a proxy (see `MEMORY_ALLOCATION.md` section 6 for
  limits).
- The earlier Windows startup bugs from the same review (platform dropped after bootstrap,
  so the wallpaper vanished; `0x052C` parameters that did not spawn a WorkerW on Windows 10
  22H2) are already fixed on `main` (`run_daemon` keeps the platform alive; the WorkerW
  discovery tries several parameter variants) and are not covered again here.
- `ImageBackend::close()` was re-checked at this commit and still does not free its resource
  handle.
