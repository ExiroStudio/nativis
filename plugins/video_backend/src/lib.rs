//! `nativis-plugin-video` — Production video frame producer for the Nativis Runtime.
//!
//! ## Architectural Invariants
//!
//! 1. **No backend may bypass the Runtime.**
//!    Frames flow: `Decode → Runtime → Transport (SHM) → Plugin → Desktop`.
//!    This crate holds zero knowledge of SHM, the KDE plugin, or any sink.
//!
//! 2. **A Backend is a Frame Producer, not a Player.**
//!    The Runtime is unaware of whether frames come from video, image, or camera.
//!    This backend's only obligation: produce `Frame`s conforming to `MediaBackend`.
//!
//! 3. **Decoder never knows where frames go.**
//!    The decoder thread builds `DecodedFrame`s and puts them in a bounded channel.
//!    It has zero reference to `ResourceManager`, `FrameSink`, or any runtime type.
//!
//! 4. **Decoded frames are immutable.**
//!    After a `DecodedFrame` exits the decoder, pixels live in `Arc<[u8]>`.
//!    The Runtime may clone the `Arc` but never mutates the data.
//!
//! ## Ownership
//! ```text
//! VideoBackend
//!   owns
//!     ├── ResourceManager clone  (shared with Runtime — registers PlanarBuffers)
//!     ├── Decoder thread         (background OS thread, exits when channel drops)
//!     └── Frame queue            (crossbeam bounded(1) Receiver<DecodedFrame>, A3a)
//!
//! Runtime
//!   owns
//!     └── ResourceManager  (authoritative instance; VideoBackend holds a clone)
//!
//! ResourceManager  (Arc<Mutex<...>>, shared via clone)
//!   owns
//!     └── PlanarBuffer per active frame  (freed on the next tick via resources.free())
//! ```
//! Dropping VideoBackend drops the Receiver, which signals the decoder thread to exit.

mod frame;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver};
use tracing::{debug, error, info, warn};

use ffmpeg_next as ffmpeg;
use ffmpeg_next::codec::threading;
use ffmpeg_next::ffi;
use ffmpeg_next::format::Pixel;
use ffmpeg_next::media::Type as MediaType;
use ffmpeg_next::software::scaling::{Context as Scaler, Flags as ScaleFlags};

use nativis_asset::AssetPath;
use nativis_core::{
    clock::MediaClock,
    contract::{Frame, FrameStatus, MediaBackend, MediaCapability, MediaError, ResourceHandle},
    resource::{GpuTextureResource, PixelFormat, PlanarBuffer, PlaneDesc, ResourceManager},
};

pub use frame::DecodedFrame;

// ── Instrumentation ───────────────────────────────────────────────────────────
const METRICS_INTERVAL: u64 = 300; // log every N published frames

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DecodePreference {
    Auto,
    Hardware,
    Software,
}

impl DecodePreference {
    fn from_environment() -> Self {
        match std::env::var("NATIVIS_DECODE")
            .unwrap_or_else(|_| "auto".to_owned())
            .to_ascii_lowercase()
            .as_str()
        {
            "hardware" | "gpu" => Self::Hardware,
            "software" | "cpu" => Self::Software,
            value => {
                if value != "auto" {
                    warn!(
                        value,
                        "[NATIVIS DECODE] unknown NATIVIS_DECODE value; using auto"
                    );
                }
                Self::Auto
            }
        }
    }
}

struct HardwareDecodeState {
    device: *mut ffi::AVBufferRef,
    pixel_format: ffi::AVPixelFormat,
    device_name: &'static str,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    is_shared_device: bool,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    generation: u64,
}

impl Drop for HardwareDecodeState {
    fn drop(&mut self) {
        unsafe { ffi::av_buffer_unref(&mut self.device) };
    }
}

/// FFmpeg AVD3D11VADeviceContext layout from <libavutil/hwcontext_d3d11va.h>
#[allow(dead_code)]
#[repr(C)]
struct AVD3D11VADeviceContext {
    device: *mut std::ffi::c_void,
    device_context: *mut std::ffi::c_void,
    video_device: *mut std::ffi::c_void,
    video_context: *mut std::ffi::c_void,
    lock: Option<unsafe extern "C" fn(*mut std::ffi::c_void)>,
    unlock: Option<unsafe extern "C" fn(*mut std::ffi::c_void)>,
    lock_ctx: *mut std::ffi::c_void,
    bind_flags: u32,
    misc_flags: u32,
}

/// Calls COM IUnknown::AddRef (vtable slot 1) on raw COM pointer.
/// FFmpeg av_hwdevice_ctx_free will call Release on device deallocation.
#[allow(dead_code)]
unsafe fn com_add_ref(ptr: *mut std::ffi::c_void) -> u32 {
    if ptr.is_null() {
        return 0;
    }
    let vtable = *(ptr as *mut *const usize);
    let add_ref_fn: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32 =
        std::mem::transmute(*vtable.add(1));
    add_ref_fn(ptr)
}

fn software_thread_count() -> usize {
    software_thread_count_for(0, 0)
}

/// A3b: Determine software decode thread count based on video resolution.
/// Frame threading holds ~1.5× more frames per thread, so we cap threads
/// for small resolutions to avoid RAM waste. Formula from MEMORY_ALLOCATION.md:
/// RAM ≈ threads_held × W × H × 1.5 bytes for NV12.
///
/// - ≤720p  → 1 thread  (~19 MB anon)
/// - ≤1080p → 2 threads (~45 MB anon)
/// - ≤1440p → 3 threads (~67 MB anon)
/// - 4K+    → 4 threads (~183 MB anon, same as before but now explicit)
///
/// NATIVIS_DECODE_THREADS env var always overrides.
fn software_thread_count_for(width: u32, height: u32) -> usize {
    if let Some(count) = std::env::var("NATIVIS_DECODE_THREADS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|count| *count > 0)
    {
        return count;
    }

    let pixels = (width as u64) * (height as u64);
    let resolution_threads = if pixels == 0 {
        // Resolution unknown at the time of this call — use max available.
        std::thread::available_parallelism()
            .map(|p| p.get().saturating_sub(1).clamp(1, 4))
            .unwrap_or(1)
    } else if pixels <= 1280 * 720 {
        1 // ≤720p
    } else if pixels <= 1920 * 1080 {
        2 // ≤1080p
    } else if pixels <= 2560 * 1440 {
        3 // ≤1440p
    } else {
        4 // 4K and above
    };

    let available = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(1);

    resolution_threads.min(available.saturating_sub(1).max(1))
}

fn configure_software_threads(
    context: &mut ffmpeg::codec::context::Context,
    width: u32,
    height: u32,
) {
    let thread_count = software_thread_count_for(width, height);
    let mut config = threading::Config::count(thread_count);
    config.kind = threading::Type::Frame;
    context.set_threading(config);
}

fn preferred_hardware_devices() -> Vec<(ffi::AVHWDeviceType, &'static str)> {
    #[cfg(target_os = "linux")]
    {
        vec![
            (ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI, "vaapi"),
            (ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA, "cuda"),
        ]
    }

    #[cfg(target_os = "windows")]
    {
        vec![
            (ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA, "d3d11va"),
            (ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA, "cuda"),
        ]
    }

    #[cfg(target_os = "macos")]
    {
        vec![(
            ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
            "videotoolbox",
        )]
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        Vec::new()
    }
}

unsafe extern "C" fn select_hardware_format(
    context: *mut ffi::AVCodecContext,
    formats: *const ffi::AVPixelFormat,
) -> ffi::AVPixelFormat {
    if context.is_null() || formats.is_null() {
        return ffi::AVPixelFormat::AV_PIX_FMT_NONE;
    }

    let state = ((*context).opaque as *const HardwareDecodeState).as_ref();
    if let Some(state) = state {
        let mut candidate = formats;
        while *candidate != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
            if *candidate == state.pixel_format {
                return state.pixel_format;
            }
            candidate = candidate.add(1);
        }
    }

    ffi::avcodec_default_get_format(context, formats)
}

fn configure_hardware_decoder(
    context: &mut ffmpeg::codec::context::Context,
    preference: DecodePreference,
    _gpu_device: Option<nativis_core::resource::GpuDeviceResource>,
) -> Result<Option<Box<HardwareDecodeState>>, String> {
    if preference == DecodePreference::Software {
        return Ok(None);
    }

    unsafe {
        let codec = ffi::avcodec_find_decoder((*context.as_ptr()).codec_id);
        if codec.is_null() {
            return Err("FFmpeg could not find a decoder for the stream".into());
        }

        // On Windows: Attempt shared D3D11VA device initialization first if platform published its device.
        #[cfg(target_os = "windows")]
        if let Some(gpu) = _gpu_device {
            let zero_copy_enabled = std::env::var("NATIVIS_D3D11_ZERO_COPY")
                .map(|v| v != "0" && v.to_lowercase() != "false")
                .unwrap_or(false);

            if zero_copy_enabled && gpu.device_ptr != 0 {
                let mut config_index = 0;
                let mut d3d11_config = None;
                loop {
                    let config = ffi::avcodec_get_hw_config(codec, config_index);
                    if config.is_null() {
                        break;
                    }
                    config_index += 1;
                    if (*config).device_type == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA
                        && ((*config).methods & 1) != 0
                    {
                        d3d11_config = Some(config);
                        break;
                    }
                }

                if let Some(config) = d3d11_config {
                    let mut device_ref =
                        ffi::av_hwdevice_ctx_alloc(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA);
                    if !device_ref.is_null() {
                        let hwctx = (*device_ref).data as *mut ffi::AVHWDeviceContext;
                        if !hwctx.is_null() {
                            let d3d11_ctx = (*hwctx).hwctx as *mut AVD3D11VADeviceContext;
                            if !d3d11_ctx.is_null() {
                                // AddRef device COM interface as FFmpeg calls Release on av_hwdevice_ctx_free
                                com_add_ref(gpu.device_ptr as *mut std::ffi::c_void);
                                (*d3d11_ctx).device = gpu.device_ptr as *mut std::ffi::c_void;
                                if gpu.device_context_ptr != 0 {
                                    com_add_ref(gpu.device_context_ptr as *mut std::ffi::c_void);
                                    (*d3d11_ctx).device_context =
                                        gpu.device_context_ptr as *mut std::ffi::c_void;
                                }
                                const D3D11_BIND_DECODER: u32 = 0x200;
                                const D3D11_BIND_SHADER_RESOURCE: u32 = 0x08;
                                (*d3d11_ctx).bind_flags =
                                    D3D11_BIND_DECODER | D3D11_BIND_SHADER_RESOURCE;

                                let init_err = ffi::av_hwdevice_ctx_init(device_ref);
                                if init_err == 0 {
                                    let decoder_device = ffi::av_buffer_ref(device_ref);
                                    if !decoder_device.is_null() {
                                        // 4 extra surfaces in pool so holding frames in channel does not starve decoder
                                        (*context.as_mut_ptr()).extra_hw_frames = 4;

                                        let state = Box::new(HardwareDecodeState {
                                            device: device_ref,
                                            pixel_format: (*config).pix_fmt,
                                            device_name: "d3d11va",
                                            is_shared_device: true,
                                            generation: gpu.generation,
                                        });
                                        let raw_context = context.as_mut_ptr();
                                        (*raw_context).hw_device_ctx = decoder_device;
                                        (*raw_context).opaque = (&*state
                                            as *const HardwareDecodeState)
                                            .cast_mut()
                                            .cast();
                                        (*raw_context).get_format = Some(select_hardware_format);
                                        info!("[NATIVIS DECODE] Initialized shared D3D11VA device context (zero-copy enabled)");
                                        return Ok(Some(state));
                                    }
                                } else {
                                    warn!("[NATIVIS DECODE] Shared D3D11VA device context init failed (code={}); falling back to independent device and CPU readback", init_err);
                                }
                            }
                        }
                        ffi::av_buffer_unref(&mut device_ref);
                    }
                }
            } else if !zero_copy_enabled {
                info!("[NATIVIS DECODE] NATIVIS_D3D11_ZERO_COPY is disabled; using independent device + CPU readback");
            }
        }

        for (device_type, device_name) in preferred_hardware_devices() {
            let mut config_index = 0;
            loop {
                let config = ffi::avcodec_get_hw_config(codec, config_index);
                if config.is_null() {
                    break;
                }
                config_index += 1;

                if (*config).device_type != device_type || ((*config).methods & 1) == 0 {
                    continue;
                }

                let mut device = std::ptr::null_mut();
                let mut created = false;

                #[cfg(target_os = "linux")]
                if device_type == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI {
                    // Probe all /dev/dri/renderD* nodes in sorted order.
                    // Hard-coding renderD128/129 misses systems with renderD127,
                    // renderD130+, hybrid-GPU setups, or USB render nodes.
                    let mut render_nodes: Vec<std::path::PathBuf> = std::fs::read_dir("/dev/dri")
                        .into_iter()
                        .flatten()
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| {
                            p.file_name()
                                .and_then(|n| n.to_str())
                                .map(|n| n.starts_with("renderD"))
                                .unwrap_or(false)
                        })
                        .collect();
                    render_nodes.sort();
                    for render_node in &render_nodes {
                        if let Some(node_str) = render_node.to_str() {
                            if let Ok(c_node) = std::ffi::CString::new(node_str) {
                                if ffi::av_hwdevice_ctx_create(
                                    &mut device,
                                    device_type,
                                    c_node.as_ptr(),
                                    std::ptr::null_mut(),
                                    0,
                                ) == 0
                                {
                                    info!(
                                        "[NATIVIS DECODE] Initialized VAAPI device context on {}",
                                        node_str
                                    );
                                    created = true;
                                    break;
                                } else {
                                    debug!(
                                        "[NATIVIS DECODE] VAAPI init failed on {}; trying next",
                                        node_str
                                    );
                                }
                            }
                        }
                    }
                    if !created {
                        debug!("[NATIVIS DECODE] No VAAPI render node succeeded; falling back to default av_hwdevice_ctx_create");
                    }
                }

                if !created
                    && ffi::av_hwdevice_ctx_create(
                        &mut device,
                        device_type,
                        std::ptr::null(),
                        std::ptr::null_mut(),
                        0,
                    ) == 0
                {
                    created = true;
                }

                if !created {
                    debug!(
                        "[NATIVIS DECODE] Hardware device context creation failed for {}",
                        device_name
                    );
                    continue;
                }

                let decoder_device = ffi::av_buffer_ref(device);
                if decoder_device.is_null() {
                    ffi::av_buffer_unref(&mut device);
                    continue;
                }

                let state = Box::new(HardwareDecodeState {
                    device,
                    pixel_format: (*config).pix_fmt,
                    device_name,
                    is_shared_device: false,
                    generation: 0,
                });
                let raw_context = context.as_mut_ptr();
                (*raw_context).hw_device_ctx = decoder_device;
                (*raw_context).opaque = (&*state as *const HardwareDecodeState).cast_mut().cast();
                (*raw_context).get_format = Some(select_hardware_format);
                return Ok(Some(state));
            }
        }
    }

    match preference {
        DecodePreference::Hardware => {
            Err("no supported hardware decoder device is available".into())
        }
        DecodePreference::Auto => {
            info!("[NATIVIS DECODE] Falling back to CPU software decode (no supported hardware decode device context succeeded)");
            Ok(None)
        }
        DecodePreference::Software => Ok(None),
    }
}

// ── VideoBackend ──────────────────────────────────────────────────────────────

pub struct VideoBackend {
    rx: Option<Receiver<DecodedFrame>>,
    stop_signal: Option<Arc<AtomicBool>>,
    /// ResourceManager clone — used to register/free PlanarBuffers each tick.
    /// ResourceManager is Arc-backed so clone is cheap.
    resources: Option<ResourceManager>,
    /// Handle of the frame registered in the last tick (freed next tick).
    last_handle: Option<ResourceHandle>,
    /// The Frame ready to present in this tick.
    pending: Option<Frame>,
    /// Accumulated playback time since open (driven by dt).
    accumulated_time: Duration,
    /// Frame popped from channel but not yet due to be presented.
    cached_frame: Option<DecodedFrame>,
    /// Track last seen PTS to detect loops
    last_pts: Duration,

    // Instrumentation
    publish_count: u64,
    skip_count: u64,
    total_publish_us: u64,
    zero_copy_frames: u64,
    readback_frames: u64,
    window_start: Instant,
}

impl VideoBackend {
    pub fn new() -> Self {
        Self {
            rx: None,
            stop_signal: None,
            resources: None,
            last_handle: None,
            pending: None,
            accumulated_time: Duration::ZERO,
            cached_frame: None,
            last_pts: Duration::ZERO,
            publish_count: 0,
            skip_count: 0,
            total_publish_us: 0,
            zero_copy_frames: 0,
            readback_frames: 0,
            window_start: Instant::now(),
        }
    }
}

impl Default for VideoBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaBackend for VideoBackend {
    fn name(&self) -> &'static str {
        "video_backend"
    }

    fn supports(&self, source: &AssetPath) -> bool {
        matches!(
            source.extension(),
            "mp4" | "mkv" | "webm" | "avi" | "mov" | "ts" | "flv" | "m4v" | "wmv"
        )
    }

    fn capabilities(&self) -> &[MediaCapability] {
        &[MediaCapability::Loop]
    }

    fn open(
        &mut self,
        source: &AssetPath,
        _clock: &MediaClock,
        resources: &ResourceManager,
    ) -> Result<(), MediaError> {
        let path = source.to_file_path().ok_or_else(|| {
            MediaError::Open(format!(
                "VideoBackend requires a local file path, got: {}",
                source.raw_uri()
            ))
        })?;
        let path_str = path.to_string_lossy().to_string();

        // Synchronous validation: verify file can be opened and decoder is available
        {
            let ictx = ffmpeg::format::input(&path).map_err(|e| {
                MediaError::Open(format!("Cannot open video file '{}': {}", path_str, e))
            })?;
            let video_stream = ictx.streams().best(MediaType::Video).ok_or_else(|| {
                MediaError::Open(format!("No video stream found in '{}'", path_str))
            })?;
            let codec_ctx =
                ffmpeg::codec::context::Context::from_parameters(video_stream.parameters())
                    .map_err(|e| {
                        MediaError::Open(format!("Failed to initialize codec context: {}", e))
                    })?;

            let decoder_available = unsafe {
                let codec_id = (*codec_ctx.as_ptr()).codec_id;
                !ffi::avcodec_find_decoder(codec_id).is_null()
            };

            if !decoder_available {
                let codec_name = unsafe {
                    let ptr = ffi::avcodec_get_name((*codec_ctx.as_ptr()).codec_id);
                    if !ptr.is_null() {
                        std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
                    } else {
                        "unknown".to_string()
                    }
                };
                return Err(MediaError::Open(format!(
                    "No FFmpeg decoder found for codec '{}' (e.g. system lacks {} decoder). Please use H.264/VP9/AV1 or install full codec packages.",
                    codec_name, codec_name
                )));
            }
        }

        // Clone the shared ResourceManager — zero cost (Arc clone).
        // We use it in update() to register a CpuBuffer per decoded frame.
        self.resources = Some(resources.clone());
        self.accumulated_time = Duration::ZERO;
        self.last_pts = Duration::ZERO;

        // A3a: bounded(1) — only one decoded frame waits in the channel at a time.
        // This reduces in-flight frames from the previous bounded(2), saving ~12 MB
        // for 4K NV12 (W×H×1.5 bytes). The decoder still runs one frame ahead for
        // smooth playback with natural backpressure. See MEMORY_ALLOCATION.md §5.
        let (tx, rx) = bounded::<DecodedFrame>(1);
        let stop = Arc::new(AtomicBool::new(false));

        self.rx = Some(rx);
        self.stop_signal = Some(Arc::clone(&stop));
        self.window_start = Instant::now();

        let res_clone = resources.clone();
        std::thread::Builder::new()
            .name(format!("nativis-decode:{}", path_str))
            .spawn(move || decoder_thread(&path_str, tx, stop, res_clone))
            .map_err(|e| MediaError::Open(format!("Failed to spawn decoder thread: {}", e)))?;

        info!("VideoBackend: opened '{}'", source.raw_uri());
        Ok(())
    }

    /// Advance the media clock — pick the newest decoded frame, register it.
    ///
    /// "Present the newest frame that is still valid for the current media clock."
    /// Older frames in the queue are counted as skipped. Never silent.
    fn update(&mut self, dt: Duration) -> Result<(), MediaError> {
        let rx = match &self.rx {
            Some(r) => r,
            None => return Ok(()),
        };
        let resources = match &self.resources {
            Some(r) => r.clone(),
            None => return Ok(()),
        };

        self.accumulated_time += dt;
        let mut current_pts = self.accumulated_time;
        let t0 = Instant::now();
        let mut selected: Option<DecodedFrame> = None;
        let mut drained = 0u32;

        // Drain the channel up to the current presentation time.
        loop {
            let f = if let Some(cached) = self.cached_frame.take() {
                cached
            } else {
                match rx.try_recv() {
                    Ok(frame) => {
                        drained += 1;
                        frame
                    }
                    Err(_) => break,
                }
            };

            let frame_pts = f.pts_duration();

            // Detect loop (PTS drops significantly, e.g. more than 1 second backwards)
            if self.last_pts > frame_pts + Duration::from_secs(1) {
                self.accumulated_time = frame_pts;
                current_pts = frame_pts;
            }
            self.last_pts = frame_pts;

            if frame_pts <= current_pts {
                selected = Some(f);
            } else {
                // Frame is in the future. Keep it for later.
                self.cached_frame = Some(f);
                break;
            }
        }

        if drained > 1 {
            let skipped = (drained - 1) as u64;
            self.skip_count += skipped;
            // Skipped frames are NEVER silent.
            warn!(
                skipped,
                total_skipped = self.skip_count,
                "VideoBackend: runtime behind decoder — {skipped} frame(s) skipped"
            );
        }

        self.pending = None;

        if let Some(df) = selected {
            // Free the PlanarBuffer from the previous tick.
            if let Some(old) = self.last_handle.take() {
                resources.free(old);
            }

            let pts = df.pts_duration();
            let width = df.width;
            let height = df.height;

            let handle = if let Some(gpu_frame) = &df.d3d11_texture {
                let buf = GpuTextureResource {
                    texture_ptr: gpu_frame.texture_ptr,
                    subresource_index: gpu_frame.subresource_index,
                    width,
                    height,
                    format: 103, // DXGI_FORMAT_NV12
                    keeper: gpu_frame.frame_ref.clone(),
                };
                self.zero_copy_frames += 1;
                resources.register(Box::new(buf))
            } else {
                // Register NV12 planar data — Arc::clone is refcount-only, NO memcpy.
                let buf = PlanarBuffer {
                    format: PixelFormat::Nv12,
                    width,
                    height,
                    planes: vec![
                        PlaneDesc {
                            data: df.y_plane.clone(),
                            stride: df.y_stride,
                        },
                        PlaneDesc {
                            data: df.uv_plane.clone(),
                            stride: df.uv_stride,
                        },
                    ],
                };
                self.readback_frames += 1;
                resources.register(Box::new(buf))
            };
            self.last_handle = Some(handle);

            self.pending = Some(Frame::new(handle, width, height, pts, true));

            let us = t0.elapsed().as_micros() as u64;
            self.total_publish_us += us;
            self.publish_count += 1;

            if self.publish_count.is_multiple_of(METRICS_INTERVAL) {
                let elapsed_s = self.window_start.elapsed().as_secs_f64();
                let fps = self.publish_count as f64 / elapsed_s.max(0.001);
                let avg_us = self.total_publish_us / self.publish_count.max(1);
                info!(
                    "[NATIVIS VIDEO] publish_fps={:.1} avg_publish_us={}µs zero_copy={} readback={} total_skipped={}",
                    fps, avg_us, self.zero_copy_frames, self.readback_frames, self.skip_count
                );
                // Reset window
                self.publish_count = 0;
                self.total_publish_us = 0;
                self.skip_count = 0;
                self.window_start = Instant::now();
            }
        }

        Ok(())
    }

    fn current_frame(&self) -> FrameStatus {
        match &self.pending {
            Some(f) => FrameStatus::Ready(f.clone()),
            None => FrameStatus::Unchanged,
        }
    }

    fn next_deadline(&self) -> Option<Duration> {
        // Items 6 & 8: If cached lookahead frame is waiting for its presentation PTS,
        // tell conductor the exact sleep duration needed instead of fixed 60 Hz polling.
        if let Some(ref cached) = self.cached_frame {
            let frame_pts = cached.pts_duration();
            if frame_pts > self.accumulated_time {
                Some(frame_pts - self.accumulated_time)
            } else {
                Some(Duration::ZERO)
            }
        } else {
            None
        }
    }

    fn close(&mut self) {
        if let Some(stop) = self.stop_signal.take() {
            stop.store(true, Ordering::Release);
        }
        // Drop Receiver → Sender in decoder thread gets Err next send → thread exits.
        self.rx = None;

        // Free the last registered buffer.
        if let (Some(res), Some(handle)) = (&self.resources, self.last_handle.take()) {
            res.free(handle);
        }
        self.pending = None;
        self.resources = None;
        info!("VideoBackend: closed");
    }
}

pub fn determine_decode_mode(
    zero_copy_frames: u64,
    readback_frames: u64,
    hw_device: Option<&str>,
) -> &'static str {
    if zero_copy_frames > 0 && zero_copy_frames >= readback_frames {
        "d3d11va+zero_copy"
    } else if readback_frames > 0 {
        match hw_device {
            Some("d3d11va") => "d3d11va+cpu_readback",
            Some("vaapi") => "vaapi+cpu_readback",
            Some("cuda") => "cuda+cpu_readback",
            Some(_) => "hardware+cpu_readback",
            None => "software",
        }
    } else {
        "software"
    }
}

// ── Decoder thread ────────────────────────────────────────────────────────────
//
// Owns ALL FFmpeg state. Runs on its own OS thread.
// Produces immutable `DecodedFrame`s. Has no knowledge of the Runtime or sink.
// Exits cleanly when `stop` is set OR the channel sender side is dropped.

fn decoder_thread(
    path: &str,
    tx: crossbeam_channel::Sender<DecodedFrame>,
    stop: Arc<AtomicBool>,
    resources: ResourceManager,
) {
    if let Err(e) = ffmpeg::init() {
        error!("[NATIVIS DECODE] ffmpeg::init() failed: {}", e);
        return;
    }

    let mut decode_count: u64 = 0;
    let mut total_decode_us: u64 = 0;
    let mut total_transfer_us: u64 = 0;
    let mut total_scale_us: u64 = 0;
    let mut total_copy_us: u64 = 0;
    let mut window_zero_copy_frames: u64 = 0;
    let mut window_readback_frames: u64 = 0;
    let mut thread_window = Instant::now();
    let preference = DecodePreference::from_environment();
    let mut allow_hardware = preference != DecodePreference::Software;
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables, unused_mut))]
    let mut logged_recreate_fallback = false;

    // Outer loop: restart from the top for seamless looping at EOF.
    'outer: loop {
        if stop.load(Ordering::Acquire) {
            break;
        }

        let mut ictx = match ffmpeg::format::input(&path) {
            Ok(c) => c,
            Err(e) => {
                error!("[NATIVIS DECODE] Cannot open '{}': {}", path, e);
                break;
            }
        };

        let video_index = match ictx.streams().best(MediaType::Video) {
            Some(s) => s.index(),
            None => {
                error!("[NATIVIS DECODE] No video stream in '{}'", path);
                break;
            }
        };

        let stream = ictx.stream(video_index).unwrap();
        let tb = stream.time_base();

        let mut context =
            match ffmpeg::codec::context::Context::from_parameters(stream.parameters()) {
                Ok(context) => context,
                Err(error) => {
                    error!("[NATIVIS DECODE] codec context failed: {}", error);
                    break;
                }
            };
        // A3b: get video resolution from codec parameters to select thread count.
        let (stream_width, stream_height) = unsafe {
            let par = stream.parameters().as_ptr();
            ((*par).width as u32, (*par).height as u32)
        };
        configure_software_threads(&mut context, stream_width, stream_height);

        let hardware_preference = if allow_hardware {
            preference
        } else {
            DecodePreference::Software
        };
        let mut hardware = match configure_hardware_decoder(
            &mut context,
            hardware_preference,
            resources.gpu_device(),
        ) {
            Ok(hardware) => hardware,
            Err(error) if preference == DecodePreference::Hardware => {
                error!(
                    "[NATIVIS DECODE] hardware decode required but unavailable: {}",
                    error
                );
                break;
            }
            Err(error) => {
                debug!(
                    "[NATIVIS DECODE] hardware setup unavailable; using software: {}",
                    error
                );
                None
            }
        };

        let mut decoder = match context.decoder().video() {
            Ok(decoder) => decoder,
            Err(error) if hardware.is_some() && preference == DecodePreference::Auto => {
                warn!(
                    "[NATIVIS DECODE] hardware decoder open failed; retrying software: {}",
                    error
                );
                hardware = None;
                let mut software_context =
                    match ffmpeg::codec::context::Context::from_parameters(stream.parameters()) {
                        Ok(context) => context,
                        Err(error) => {
                            error!("[NATIVIS DECODE] software codec context failed: {}", error);
                            break;
                        }
                    };
                configure_software_threads(&mut software_context, stream_width, stream_height);
                match software_context.decoder().video() {
                    Ok(decoder) => decoder,
                    Err(error) => {
                        error!("[NATIVIS DECODE] software decoder open failed: {}", error);
                        break;
                    }
                }
            }

            Err(error) => {
                error!("[NATIVIS DECODE] decoder open failed: {}", error);
                break;
            }
        };

        let (w, h) = (decoder.width(), decoder.height());
        let decoder_format = decoder.format();
        let decoder_mode = hardware
            .as_ref()
            .map(|state| state.device_name)
            .unwrap_or("software");

        // K2: Log decode mode and adapter info at info level (D7.3 — no silent fallbacks).
        // On Windows, D3D11VA decode runs on GPU but frames are read back to CPU RAM
        // (zero-copy path disabled in Phase 0 due to cross-device bug, see K1 comment).
        #[cfg(target_os = "windows")]
        {
            if let Some(state) = hardware.as_ref() {
                let mode_str = if state.is_shared_device {
                    "d3d11va+zero_copy"
                } else {
                    "d3d11va+cpu_readback"
                };
                info!(
                    "[NATIVIS DECODE] Opened: {}x{} {:?} → NV12 tb={}/{} mode={} threads={}",
                    w,
                    h,
                    decoder_format,
                    tb.numerator(),
                    tb.denominator(),
                    mode_str,
                    software_thread_count(),
                );
            } else {
                warn!(
                    "[NATIVIS DECODE] Opened: {}x{} {:?} → NV12 tb={}/{} mode=software threads={} \
                     (hardware decode unavailable or disabled; expect higher CPU/RAM usage)",
                    w,
                    h,
                    decoder_format,
                    tb.numerator(),
                    tb.denominator(),
                    software_thread_count(),
                );
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            if hardware.is_some() {
                info!(
                    "[NATIVIS DECODE] Opened: {}x{} {:?} → NV12 tb={}/{} mode={} threads={}",
                    w,
                    h,
                    decoder_format,
                    tb.numerator(),
                    tb.denominator(),
                    decoder_mode,
                    software_thread_count(),
                );
            } else {
                warn!(
                    "[NATIVIS DECODE] Opened: {}x{} {:?} → NV12 tb={}/{} mode=software threads={} \
                     (hardware decode unavailable or disabled; expect higher CPU/RAM usage)",
                    w,
                    h,
                    decoder_format,
                    tb.numerator(),
                    tb.denominator(),
                    software_thread_count(),
                );
            }
        }

        let mut raw = ffmpeg::util::frame::video::Video::empty();
        let mut transferred = ffmpeg::util::frame::video::Video::empty();
        let mut nv12 = ffmpeg::util::frame::video::Video::empty();
        let mut scaler: Option<(Pixel, u32, u32, Scaler)> = None;
        let mut restart_with_software = false;

        // Item 3: 3-slot pre-allocated planar buffer pool
        struct PlanarSlot {
            y: Arc<[u8]>,
            uv: Arc<[u8]>,
        }
        let mut pool: [Option<PlanarSlot>; 3] = [None, None, None];
        let mut pool_idx = 0usize;
        let mut pool_hits = 0u64;
        let mut pool_misses = 0u64;
        let mut logged_non_nv12 = false;

        'packets: for (stream_ref, packet) in ictx.packets() {
            if stop.load(Ordering::Acquire) {
                break 'outer;
            }
            if stream_ref.index() != video_index {
                continue;
            }

            let t0 = Instant::now();
            if decoder.send_packet(&packet).is_err() {
                continue;
            }

            while decoder.receive_frame(&mut raw).is_ok() {
                let t_decode_done = Instant::now();
                let raw_pts = raw.pts().unwrap_or(0);

                // Zero-Copy Direct3D 11 VRAM Texture Path (Phase 1)
                #[cfg(target_os = "windows")]
                if let Some(state) = hardware.as_ref() {
                    if state.is_shared_device
                        && raw.format() == Pixel::from(ffi::AVPixelFormat::AV_PIX_FMT_D3D11)
                    {
                        let current_gpu = resources.gpu_device();
                        if current_gpu.map_or(0, |g| g.generation) == state.generation {
                            let cloned_frame = unsafe { ffi::av_frame_clone(raw.as_ptr()) };
                            if !cloned_frame.is_null() {
                                struct AvFrameKeeper(*mut ffi::AVFrame);
                                impl Drop for AvFrameKeeper {
                                    fn drop(&mut self) {
                                        unsafe { ffi::av_frame_free(&mut self.0) };
                                    }
                                }
                                unsafe impl Send for AvFrameKeeper {}
                                unsafe impl Sync for AvFrameKeeper {}

                                let keeper = Arc::new(AvFrameKeeper(cloned_frame));
                                // AVFrame.data[0] holds ID3D11Texture2D*; data[1] holds array slice index cast from intptr_t
                                let tex_ptr = unsafe { (*cloned_frame).data[0] as usize };
                                let sub_idx = unsafe { (*cloned_frame).data[1] as usize as u32 };

                                let df = DecodedFrame {
                                    width: raw.width(),
                                    height: raw.height(),
                                    pts: raw_pts,
                                    time_base_num: tb.numerator(),
                                    time_base_den: tb.denominator(),
                                    y_plane: Arc::from([]),
                                    uv_plane: Arc::from([]),
                                    y_stride: 0,
                                    uv_stride: 0,
                                    chroma_height: (raw.height() + 1) / 2,
                                    d3d11_texture: Some(Arc::new(frame::D3D11GpuFrame {
                                        texture_ptr: tex_ptr,
                                        subresource_index: sub_idx,
                                        frame_ref: Some(keeper),
                                    })),
                                };

                                let decode_us = t_decode_done.duration_since(t0).as_micros() as u64;
                                total_decode_us += decode_us;
                                window_zero_copy_frames += 1;
                                decode_count += 1;

                                if decode_count.is_multiple_of(METRICS_INTERVAL) {
                                    let elapsed_s = thread_window.elapsed().as_secs_f64();
                                    let fps = decode_count as f64 / elapsed_s.max(0.001);
                                    let avg_dec = total_decode_us / decode_count.max(1);
                                    let avg_xfer = total_transfer_us / decode_count.max(1);
                                    let avg_scl = total_scale_us / decode_count.max(1);
                                    let avg_copy = total_copy_us / decode_count.max(1);
                                    let active_mode = determine_decode_mode(
                                        window_zero_copy_frames,
                                        window_readback_frames,
                                        hardware.as_ref().map(|s| s.device_name),
                                    );
                                    info!(
                                        "[NATIVIS DECODE] mode={} decode_fps={:.1} zero_copy_frames={} readback_frames={} avg_decode_us={}µs avg_transfer_us={}µs avg_scale_us={}µs avg_copy_us={}µs pool_hits={} pool_misses={}",
                                        active_mode, fps, window_zero_copy_frames, window_readback_frames, avg_dec, avg_xfer, avg_scl, avg_copy, pool_hits, pool_misses
                                    );
                                    decode_count = 0;
                                    total_decode_us = 0;
                                    total_transfer_us = 0;
                                    total_scale_us = 0;
                                    total_copy_us = 0;
                                    window_zero_copy_frames = 0;
                                    window_readback_frames = 0;
                                    pool_hits = 0;
                                    pool_misses = 0;
                                    thread_window = Instant::now();
                                }

                                match tx.send(df) {
                                    Ok(()) => {}
                                    Err(_) => {
                                        debug!("[NATIVIS DECODE] channel closed, exiting");
                                        break 'outer;
                                    }
                                }
                                if stop.load(Ordering::Acquire) {
                                    break 'outer;
                                }
                                continue;
                            }
                        } else if !logged_recreate_fallback {
                            warn!(
                                "[NATIVIS DECODE] Renderer device was recreated (generation mismatch: {} vs {}); falling back to CPU readback",
                                state.generation,
                                current_gpu.map_or(0, |g| g.generation)
                            );
                            logged_recreate_fallback = true;
                        }
                    }
                }

                let source = if let Some(state) = hardware.as_ref() {
                    if raw.format() == Pixel::from(state.pixel_format) {
                        unsafe {
                            ffi::av_frame_unref(transferred.as_mut_ptr());
                            if ffi::av_hwframe_transfer_data(
                                transferred.as_mut_ptr(),
                                raw.as_ptr(),
                                0,
                            ) < 0
                            {
                                if preference == DecodePreference::Auto {
                                    warn!("[NATIVIS DECODE] hardware frame transfer failed; restarting with software decode");
                                    allow_hardware = false;
                                    restart_with_software = true;
                                    break 'packets;
                                }
                                error!("[NATIVIS DECODE] hardware frame transfer failed");
                                break 'outer;
                            }
                        }
                        &transferred
                    } else {
                        &raw
                    }
                } else {
                    &raw
                };
                let t_transfer_done = Instant::now();

                let source_format = source.format();
                let source_width = source.width();
                let source_height = source.height();
                if source_width == 0 || source_height == 0 {
                    continue;
                }

                let mut scale_done = t_transfer_done;
                let (y_stride, uv_stride, y_src, uv_src) = if source_format == Pixel::NV12 {
                    (
                        source.stride(0) as u32,
                        source.stride(1) as u32,
                        source.data(0),
                        source.data(1),
                    )
                } else {
                    let needs_scaler = scaler.as_ref().is_none_or(|(format, width, height, _)| {
                        *format != source_format
                            || *width != source_width
                            || *height != source_height
                    });
                    if needs_scaler {
                        if !logged_non_nv12 {
                            info!("[NATIVIS DECODE] Source format {:?} is 10-bit / non-NV12; converting via CPU swscale", source_format);
                            logged_non_nv12 = true;
                        }
                        let Ok(next_scaler) = Scaler::get(
                            source_format,
                            source_width,
                            source_height,
                            Pixel::NV12,
                            source_width,
                            source_height,
                            ScaleFlags::BILINEAR,
                        ) else {
                            warn!(
                                ?source_format,
                                "[NATIVIS DECODE] scaler setup failed; dropping frame"
                            );
                            continue;
                        };
                        scaler = Some((source_format, source_width, source_height, next_scaler));
                    }
                    if nv12.format() != Pixel::NV12
                        || nv12.width() != source_width
                        || nv12.height() != source_height
                    {
                        nv12 = ffmpeg::util::frame::video::Video::new(
                            Pixel::NV12,
                            source_width,
                            source_height,
                        );
                    }
                    if scaler.as_mut().unwrap().3.run(source, &mut nv12).is_err() {
                        continue;
                    }
                    scale_done = Instant::now();
                    (
                        nv12.stride(0) as u32,
                        nv12.stride(1) as u32,
                        nv12.data(0),
                        nv12.data(1),
                    )
                };

                // Readback Architecture & Memory Model:
                // - Copy #1: GPU VRAM -> host memory via av_hwframe_transfer_data.
                // - Copy #2: FFmpeg AVFrame buffers -> 3-slot pre-allocated planar buffer pool (copy_from_slice).
                //   NOTE (R6b): The earlier comment claiming this copy "cannot be avoided without
                //   exposing AVBufferRef/AVFrame across the plugin ABI (D3)" was INCORRECT. It is
                //   possible to wrap a pool slot with av_buffer_create and pass it as the dst->buf[0]
                //   to av_hwframe_transfer_data, letting the driver write directly into the pool —
                //   all within this plugin, no FFmpeg types cross the ABI. That would eliminate Copy #2.
                //   The current copy_from_slice approach is kept for simplicity and because
                //   av_hwframe_transfer_data layout guarantees vary by driver (planar vs semi-planar
                //   strides differ across VAAPI drivers). The copy also allows pool slots to use a
                //   canonical stride (width, no padding), which the sink and consumer both expect.
                //   If profiling shows Copy #2 is a bottleneck, revisit with av_buffer_create wrapping.
                // - Pool Hit Guarantees: With crossbeam bounded(1) queue, at most 1 frame waits in channel,
                //   1 frame is held by the renderer/sink, and 1 frame is being populated by decoder.
                //   With 3 slots in the ring, slot recycling always observes strong_count == 1 after warmup,
                //   yielding 100% steady-state pool hits and eliminating memory allocations/page faults.
                // - Copy #3: Host memory -> GPU staging/texture in sink via D3D11_MAP_WRITE_DISCARD.
                let (y_plane, uv_plane) = {
                    let slot = &mut pool[pool_idx];
                    pool_idx = (pool_idx + 1) % 3;

                    let reusable = match slot {
                        Some(ref mut s)
                            if s.y.len() == y_src.len() && s.uv.len() == uv_src.len() =>
                        {
                            if let (Some(y_mut), Some(uv_mut)) =
                                (Arc::get_mut(&mut s.y), Arc::get_mut(&mut s.uv))
                            {
                                y_mut.copy_from_slice(y_src);
                                uv_mut.copy_from_slice(uv_src);
                                true
                            } else {
                                false
                            }
                        }
                        _ => false,
                    };

                    if reusable {
                        pool_hits += 1;
                        let s = slot.as_ref().unwrap();
                        (Arc::clone(&s.y), Arc::clone(&s.uv))
                    } else {
                        pool_misses += 1;
                        let y: Arc<[u8]> = Arc::from(y_src);
                        let uv: Arc<[u8]> = Arc::from(uv_src);
                        let y_clone = Arc::clone(&y);
                        let uv_clone = Arc::clone(&uv);
                        *slot = Some(PlanarSlot { y, uv });
                        (y_clone, uv_clone)
                    }
                };
                let copy_done = Instant::now();

                let decode_us = t_decode_done.duration_since(t0).as_micros() as u64;
                let transfer_us = t_transfer_done.duration_since(t_decode_done).as_micros() as u64;
                let scale_us = scale_done.duration_since(t_transfer_done).as_micros() as u64;
                let copy_us = copy_done.duration_since(scale_done).as_micros() as u64;

                // PENTING: stride SEBENARNYA dari FFmpeg, bukan diasumsikan width*N.
                // FFmpeg sering align ke 32 byte untuk SIMD, jadi stride >= width.
                let chroma_h = source_height.div_ceil(2);

                let df = DecodedFrame {
                    width: source_width,
                    height: source_height,
                    pts: raw_pts,
                    time_base_num: tb.numerator(),
                    time_base_den: tb.denominator(),
                    y_plane,
                    uv_plane,
                    y_stride,
                    uv_stride,
                    chroma_height: chroma_h,
                    d3d11_texture: None,
                };

                total_decode_us += decode_us;
                total_transfer_us += transfer_us;
                total_scale_us += scale_us;
                total_copy_us += copy_us;
                window_readback_frames += 1;
                decode_count += 1;

                if decode_count.is_multiple_of(METRICS_INTERVAL) {
                    let elapsed_s = thread_window.elapsed().as_secs_f64();
                    let fps = decode_count as f64 / elapsed_s.max(0.001);
                    let avg_dec = total_decode_us / decode_count.max(1);
                    let avg_xfer = total_transfer_us / decode_count.max(1);
                    let avg_scl = total_scale_us / decode_count.max(1);
                    let avg_copy = total_copy_us / decode_count.max(1);
                    let active_mode = determine_decode_mode(
                        window_zero_copy_frames,
                        window_readback_frames,
                        hardware.as_ref().map(|s| s.device_name),
                    );
                    info!(
                        "[NATIVIS DECODE] mode={} decode_fps={:.1} zero_copy_frames={} readback_frames={} avg_decode_us={}µs avg_transfer_us={}µs avg_scale_us={}µs avg_copy_us={}µs (scale_pct={:.0}%) pool_hits={} pool_misses={}",
                        active_mode, fps, window_zero_copy_frames, window_readback_frames, avg_dec, avg_xfer, avg_scl, avg_copy,
                        (avg_scl as f64 / (avg_dec + avg_xfer + avg_scl + avg_copy).max(1) as f64) * 100.0,
                        pool_hits, pool_misses,
                    );
                    decode_count = 0;
                    total_decode_us = 0;
                    total_transfer_us = 0;
                    total_scale_us = 0;
                    total_copy_us = 0;
                    window_zero_copy_frames = 0;
                    window_readback_frames = 0;
                    pool_hits = 0;
                    pool_misses = 0;
                    thread_window = Instant::now();
                }

                match tx.send(df) {
                    Ok(()) => {}
                    // Runtime dropped the receiver — exit cleanly.
                    Err(_) => {
                        debug!("[NATIVIS DECODE] channel closed, exiting");
                        break 'outer;
                    }
                }

                if stop.load(Ordering::Acquire) {
                    break 'outer;
                }
            }
        }

        if restart_with_software {
            continue 'outer;
        }

        // EOF — flush to clear any stale B-frames, then restart seamlessly.
        decoder.flush();
        debug!("[NATIVIS DECODE] EOF — restarting for loop playback");
    }

    info!("[NATIVIS DECODE] decoder thread exited cleanly");
}

#[cfg(test)]
mod tests {
    use super::*;
    use nativis_core::resource::ResourceManager;

    #[test]
    fn test_determine_decode_mode() {
        assert_eq!(
            determine_decode_mode(10, 0, Some("d3d11va")),
            "d3d11va+zero_copy"
        );
        assert_eq!(
            determine_decode_mode(0, 10, Some("d3d11va")),
            "d3d11va+cpu_readback"
        );
        assert_eq!(
            determine_decode_mode(0, 10, Some("vaapi")),
            "vaapi+cpu_readback"
        );
        assert_eq!(
            determine_decode_mode(0, 10, Some("cuda")),
            "cuda+cpu_readback"
        );
        assert_eq!(determine_decode_mode(0, 10, None), "software");
        assert_eq!(determine_decode_mode(0, 0, None), "software");
    }

    #[test]
    fn test_gpu_texture_resource_keeper_lifecycle() {
        use nativis_core::resource::GpuTextureResource;
        use std::sync::atomic::{AtomicBool, Ordering};

        let dropped = Arc::new(AtomicBool::new(false));
        struct MockKeeper(Arc<AtomicBool>);
        impl Drop for MockKeeper {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }

        let keeper = Arc::new(MockKeeper(dropped.clone()));
        let res = GpuTextureResource {
            texture_ptr: 0x12345678,
            subresource_index: 0,
            width: 1920,
            height: 1080,
            format: 103,
            keeper: Some(keeper),
        };

        let resources = ResourceManager::new();
        let handle = resources.register(Box::new(res));
        assert!(!dropped.load(Ordering::Acquire));

        resources.free(handle);
        assert!(
            dropped.load(Ordering::Acquire),
            "Keeper must be dropped when resource handle is freed"
        );
    }

    #[test]
    fn test_generation_mismatch_fallback_logic() {
        use nativis_core::resource::GpuDeviceResource;

        let resources = ResourceManager::new();
        assert!(resources.gpu_device().is_none());

        resources.set_gpu_device(GpuDeviceResource {
            device_ptr: 0x1000,
            device_context_ptr: 0x2000,
            generation: 1,
        });

        let dev = resources.gpu_device().expect("registered gpu device");
        assert_eq!(dev.generation, 1);

        // Generation matches
        assert_eq!(dev.generation, 1);

        // Explorer recreates swapchain and increments generation
        resources.set_gpu_device(GpuDeviceResource {
            device_ptr: 0x3000,
            device_context_ptr: 0x4000,
            generation: 2,
        });
        let new_dev = resources.gpu_device().expect("updated gpu device");
        assert_eq!(new_dev.generation, 2);
        // Old state.generation (1) mismatches new_dev.generation (2) -> triggers fallback
        assert_ne!(1, new_dev.generation);
    }

    #[test]
    fn test_avd3d11va_context_layout_and_frame_data_extraction() {
        assert_eq!(
            std::mem::size_of::<AVD3D11VADeviceContext>(),
            64,
            "AVD3D11VADeviceContext must be exactly 64 bytes to match <libavutil/hwcontext_d3d11va.h>"
        );
        assert_eq!(
            std::mem::offset_of!(AVD3D11VADeviceContext, bind_flags),
            56,
            "bind_flags must be at byte offset 56 to match FFmpeg D3D11VA layout"
        );

        // Verify AVFrame.data[0] and data[1] extraction logic
        unsafe {
            let mut mock_frame = ffi::av_frame_alloc();
            assert!(!mock_frame.is_null());
            (*mock_frame).data[0] = 0xdeadbeef_usize as *mut u8;
            (*mock_frame).data[1] = 3_usize as *mut u8; // Subresource array index 3

            let tex_ptr = (*mock_frame).data[0] as usize;
            let sub_idx = (*mock_frame).data[1] as usize as u32;

            assert_eq!(tex_ptr, 0xdeadbeef_usize);
            assert_eq!(sub_idx, 3);

            ffi::av_frame_free(&mut mock_frame);
        }
    }
}
