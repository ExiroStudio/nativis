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
//!     └── Frame queue            (crossbeam bounded(2) Receiver<DecodedFrame>)
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
    resource::{PlanarBuffer, PlaneDesc, PixelFormat, ResourceManager},
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
                    warn!(value, "[NATIVIS DECODE] unknown NATIVIS_DECODE value; using auto");
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
}

impl Drop for HardwareDecodeState {
    fn drop(&mut self) {
        unsafe { ffi::av_buffer_unref(&mut self.device) };
    }
}

fn software_thread_count() -> usize {
    if let Some(count) = std::env::var("NATIVIS_DECODE_THREADS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|count| *count > 0)
    {
        return count;
    }

    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get().saturating_sub(1).clamp(1, 4))
        .unwrap_or(1)
}

fn configure_software_threads(context: &mut ffmpeg::codec::context::Context) {
    let mut config = threading::Config::count(software_thread_count());
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
        vec![(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX, "videotoolbox")]
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
) -> Result<Option<Box<HardwareDecodeState>>, String> {
    if preference == DecodePreference::Software {
        return Ok(None);
    }

    unsafe {
        let codec = ffi::avcodec_find_decoder((*context.as_ptr()).codec_id);
        if codec.is_null() {
            return Err("FFmpeg could not find a decoder for the stream".into());
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
                if ffi::av_hwdevice_ctx_create(
                    &mut device,
                    device_type,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    0,
                ) < 0
                {
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
        DecodePreference::Hardware => Err("no supported hardware decoder device is available".into()),
        DecodePreference::Auto | DecodePreference::Software => Ok(None),
    }
}

// ── VideoBackend ──────────────────────────────────────────────────────────────

pub struct VideoBackend {
    rx:           Option<Receiver<DecodedFrame>>,
    stop_signal:  Option<Arc<AtomicBool>>,
    /// ResourceManager clone — used to register/free PlanarBuffers each tick.
    /// ResourceManager is Arc-backed so clone is cheap.
    resources:    Option<ResourceManager>,
    /// Handle of the frame registered in the last tick (freed next tick).
    last_handle:  Option<ResourceHandle>,
    /// The Frame ready to present in this tick.
    pending:      Option<Frame>,
    /// Accumulated playback time since open (driven by dt).
    accumulated_time: Duration,
    /// Frame popped from channel but not yet due to be presented.
    cached_frame: Option<DecodedFrame>,
    /// Track last seen PTS to detect loops
    last_pts:     Duration,

    // Instrumentation
    publish_count:    u64,
    skip_count:       u64,
    total_publish_us: u64,
    window_start:     Instant,
}

impl VideoBackend {
    pub fn new() -> Self {
        Self {
            rx:               None,
            stop_signal:      None,
            resources:        None,
            last_handle:      None,
            pending:          None,
            accumulated_time: Duration::ZERO,
            cached_frame:     None,
            last_pts:         Duration::ZERO,
            publish_count:    0,
            skip_count:       0,
            total_publish_us: 0,
            window_start:     Instant::now(),
        }
    }
}

impl Default for VideoBackend {
    fn default() -> Self { Self::new() }
}

impl MediaBackend for VideoBackend {
    fn name(&self) -> &'static str { "video_backend" }

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
        let path = source
            .to_file_path()
            .ok_or_else(|| MediaError::Open(format!(
                "VideoBackend requires a local file path, got: {}",
                source.raw_uri()
            )))?;
        let path_str = path.to_string_lossy().to_string();

        // Clone the shared ResourceManager — zero cost (Arc clone).
        // We use it in update() to register a CpuBuffer per decoded frame.
        self.resources = Some(resources.clone());
        self.accumulated_time = Duration::ZERO;
        self.last_pts = Duration::ZERO;

        // bounded(2): natural backpressure — decoder pauses if Runtime is slow.
        let (tx, rx) = bounded::<DecodedFrame>(2);
        let stop     = Arc::new(AtomicBool::new(false));

        self.rx          = Some(rx);
        self.stop_signal = Some(Arc::clone(&stop));
        self.window_start = Instant::now();

        std::thread::Builder::new()
            .name(format!("nativis-decode:{}", path_str))
            .spawn(move || decoder_thread(&path_str, tx, stop))
            .map_err(|e| MediaError::Open(format!("Failed to spawn decoder thread: {}", e)))?;

        info!("VideoBackend: opened '{}'", source.raw_uri());
        Ok(())
    }

    /// Advance the media clock — pick the newest decoded frame, register it.
    ///
    /// "Present the newest frame that is still valid for the current media clock."
    /// Older frames in the queue are counted as skipped. Never silent.
    fn update(&mut self, dt: Duration) -> Result<(), MediaError> {
        let rx        = match &self.rx        { Some(r) => r,    None => return Ok(()) };
        let resources = match &self.resources { Some(r) => r.clone(), None => return Ok(()) };

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
                    Ok(frame) => { drained += 1; frame }
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

            let pts     = df.pts_duration();
            let width   = df.width;
            let height  = df.height;

            // Register NV12 planar data — Arc::clone is refcount-only, NO memcpy.
            let buf = PlanarBuffer {
                format: PixelFormat::Nv12,
                width,
                height,
                planes: vec![
                    PlaneDesc { data: df.y_plane.clone(),  stride: df.y_stride },
                    PlaneDesc { data: df.uv_plane.clone(), stride: df.uv_stride },
                ],
            };
            let handle = resources.register(Box::new(buf));
            self.last_handle = Some(handle);

            self.pending = Some(Frame {
                resource:  handle,
                width,
                height,
                pts,
                is_opaque: true,
            });

            let us = t0.elapsed().as_micros() as u64;
            self.total_publish_us += us;
            self.publish_count    += 1;

            if self.publish_count % METRICS_INTERVAL == 0 {
                let elapsed_s = self.window_start.elapsed().as_secs_f64();
                let fps       = self.publish_count as f64 / elapsed_s.max(0.001);
                let avg_us    = self.total_publish_us / self.publish_count.max(1);
                info!(
                    "[NATIVIS VIDEO] publish_fps={:.1} avg_publish_us={}µs total_skipped={}",
                    fps, avg_us, self.skip_count
                );
                // Reset window
                self.publish_count    = 0;
                self.total_publish_us = 0;
                self.skip_count       = 0;
                self.window_start     = Instant::now();
            }
        }

        Ok(())
    }

    fn current_frame(&self) -> FrameStatus {
        match &self.pending {
            Some(f) => FrameStatus::Ready(f.clone()),
            None    => FrameStatus::Unchanged,
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
        self.pending   = None;
        self.resources = None;
        info!("VideoBackend: closed");
    }
}

// ── Decoder thread ────────────────────────────────────────────────────────────
//
// Owns ALL FFmpeg state. Runs on its own OS thread.
// Produces immutable `DecodedFrame`s. Has no knowledge of the Runtime or sink.
// Exits cleanly when `stop` is set OR the channel sender side is dropped.

fn decoder_thread(
    path: &str,
    tx:   crossbeam_channel::Sender<DecodedFrame>,
    stop: Arc<AtomicBool>,
) {
    if let Err(e) = ffmpeg::init() {
        error!("[NATIVIS DECODE] ffmpeg::init() failed: {}", e);
        return;
    }

    let mut decode_count:    u64 = 0;
    let mut total_decode_us: u64 = 0;
    let mut total_transfer_us: u64 = 0;
    let mut total_scale_us:  u64 = 0;
    let mut total_copy_us:   u64 = 0;
    let mut thread_window         = Instant::now();
    let preference = DecodePreference::from_environment();
    let mut allow_hardware = preference != DecodePreference::Software;

    // Outer loop: restart from the top for seamless looping at EOF.
    'outer: loop {
        if stop.load(Ordering::Acquire) { break; }

        let mut ictx = match ffmpeg::format::input(&path) {
            Ok(c)  => c,
            Err(e) => { error!("[NATIVIS DECODE] Cannot open '{}': {}", path, e); break; }
        };

        let video_index = match ictx.streams().best(MediaType::Video) {
            Some(s) => s.index(),
            None    => { error!("[NATIVIS DECODE] No video stream in '{}'", path); break; }
        };

        let stream = ictx.stream(video_index).unwrap();
        let tb     = stream.time_base();

        let mut context = match ffmpeg::codec::context::Context::from_parameters(stream.parameters()) {
            Ok(context) => context,
            Err(error) => {
                error!("[NATIVIS DECODE] codec context failed: {}", error);
                break;
            }
        };
        configure_software_threads(&mut context);

        let hardware_preference = if allow_hardware {
            preference
        } else {
            DecodePreference::Software
        };
        let mut hardware = match configure_hardware_decoder(&mut context, hardware_preference) {
            Ok(hardware) => hardware,
            Err(error) if preference == DecodePreference::Hardware => {
                error!("[NATIVIS DECODE] hardware decode required but unavailable: {}", error);
                break;
            }
            Err(error) => {
                debug!("[NATIVIS DECODE] hardware setup unavailable; using software: {}", error);
                None
            }
        };

        let mut decoder = match context.decoder().video() {
            Ok(decoder) => decoder,
            Err(error) if hardware.is_some() && preference == DecodePreference::Auto => {
                warn!("[NATIVIS DECODE] hardware decoder open failed; retrying software: {}", error);
                hardware = None;
                let mut software_context = match ffmpeg::codec::context::Context::from_parameters(stream.parameters()) {
                    Ok(context) => context,
                    Err(error) => {
                        error!("[NATIVIS DECODE] software codec context failed: {}", error);
                        break;
                    }
                };
                configure_software_threads(&mut software_context);
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

        info!(
            "[NATIVIS DECODE] Opened: {}x{} {:?} → NV12 tb={}/{} mode={} threads={}",
            w, h, decoder_format, tb.numerator(), tb.denominator(), decoder_mode,
            software_thread_count(),
        );

        let mut raw = ffmpeg::util::frame::video::Video::empty();
        let mut transferred = ffmpeg::util::frame::video::Video::empty();
        let mut nv12 = ffmpeg::util::frame::video::Video::empty();
        let mut scaler: Option<(Pixel, u32, u32, Scaler)> = None;
        let mut restart_with_software = false;

        'packets: for (stream_ref, packet) in ictx.packets() {
            if stop.load(Ordering::Acquire) { break 'outer; }
            if stream_ref.index() != video_index { continue; }

            let t0 = Instant::now();
            if decoder.send_packet(&packet).is_err() { continue; }

            while decoder.receive_frame(&mut raw).is_ok() {
                let t_decode_done = Instant::now();
                let raw_pts = raw.pts().unwrap_or(0);

                let source = if let Some(state) = hardware.as_ref() {
                    if raw.format() == Pixel::from(state.pixel_format) {
                        unsafe {
                            ffi::av_frame_unref(transferred.as_mut_ptr());
                            if ffi::av_hwframe_transfer_data(
                                transferred.as_mut_ptr(),
                                raw.as_ptr(),
                                0,
                            ) < 0 {
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
                let (y_stride, uv_stride, y_plane, uv_plane) = if source_format == Pixel::NV12 {
                    let y_plane: Arc<[u8]> = Arc::from(source.data(0));
                    let uv_plane: Arc<[u8]> = Arc::from(source.data(1));
                    (
                        source.stride(0) as u32,
                        source.stride(1) as u32,
                        y_plane,
                        uv_plane,
                    )
                } else {
                    let needs_scaler = scaler.as_ref().map_or(true, |(format, width, height, _)| {
                        *format != source_format || *width != source_width || *height != source_height
                    });
                    if needs_scaler {
                        let Ok(next_scaler) = Scaler::get(
                            source_format,
                            source_width,
                            source_height,
                            Pixel::NV12,
                            source_width,
                            source_height,
                            ScaleFlags::BILINEAR,
                        ) else {
                            warn!(?source_format, "[NATIVIS DECODE] scaler setup failed; dropping frame");
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
                    let y_plane: Arc<[u8]> = Arc::from(nv12.data(0));
                    let uv_plane: Arc<[u8]> = Arc::from(nv12.data(1));
                    (
                        nv12.stride(0) as u32,
                        nv12.stride(1) as u32,
                        y_plane,
                        uv_plane,
                    )
                };
                let copy_done = Instant::now();

                let decode_us = t_decode_done.duration_since(t0).as_micros() as u64;
                let transfer_us = t_transfer_done.duration_since(t_decode_done).as_micros() as u64;
                let scale_us = scale_done.duration_since(t_transfer_done).as_micros() as u64;
                let copy_us = copy_done.duration_since(scale_done).as_micros() as u64;

                // PENTING: stride SEBENARNYA dari FFmpeg, bukan diasumsikan width*N.
                // FFmpeg sering align ke 32 byte untuk SIMD, jadi stride >= width.
                let chroma_h = (source_height + 1) / 2;

                let df = DecodedFrame {
                    width:         source_width,
                    height:        source_height,
                    pts:           raw_pts,
                    time_base_num: tb.numerator(),
                    time_base_den: tb.denominator(),
                    y_plane,
                    uv_plane,
                    y_stride,
                    uv_stride,
                    chroma_height: chroma_h,
                };

                total_decode_us += decode_us;
                total_transfer_us += transfer_us;
                total_scale_us  += scale_us;
                total_copy_us += copy_us;
                decode_count    += 1;

                if decode_count % METRICS_INTERVAL == 0 {
                    let elapsed_s = thread_window.elapsed().as_secs_f64();
                    let fps       = decode_count as f64 / elapsed_s.max(0.001);
                    let avg_dec   = total_decode_us / decode_count.max(1);
                    let avg_xfer  = total_transfer_us / decode_count.max(1);
                    let avg_scl   = total_scale_us  / decode_count.max(1);
                    let avg_copy  = total_copy_us / decode_count.max(1);
                    info!(
                        "[NATIVIS DECODE] decode_fps={:.1} avg_decode_us={}µs avg_transfer_us={}µs avg_scale_us={}µs avg_copy_us={}µs (scale_pct={:.0}%)",
                        fps, avg_dec, avg_xfer, avg_scl, avg_copy,
                        (avg_scl as f64 / (avg_dec + avg_xfer + avg_scl + avg_copy).max(1) as f64) * 100.0
                    );
                    decode_count    = 0;
                    total_decode_us = 0;
                    total_transfer_us = 0;
                    total_scale_us  = 0;
                    total_copy_us   = 0;
                    thread_window   = Instant::now();
                }

                match tx.send(df) {
                    Ok(())  => {}
                    // Runtime dropped the receiver — exit cleanly.
                    Err(_)  => { debug!("[NATIVIS DECODE] channel closed, exiting"); break 'outer; }
                }

                if stop.load(Ordering::Acquire) { break 'outer; }
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
