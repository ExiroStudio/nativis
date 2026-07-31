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
//!     ├── ResourceManager clone  (shared with Runtime — registers CpuBuffers)
//!     ├── Decoder thread         (background OS thread, exits when channel drops)
//!     └── Frame queue            (crossbeam bounded(2) Receiver<DecodedFrame>)
//!
//! Runtime
//!   owns
//!     └── ResourceManager  (authoritative instance; VideoBackend holds a clone)
//!
//! ResourceManager  (Arc<Mutex<...>>, shared via clone)
//!   owns
//!     └── CpuBuffer per active frame  (freed on the next tick via resources.free())
//! ```
//! Dropping VideoBackend drops the Receiver, which signals the decoder thread to exit.

mod frame;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver};
use tracing::{debug, error, info, warn};

use ffmpeg_next as ffmpeg;
use ffmpeg_next::format::Pixel;
use ffmpeg_next::media::Type as MediaType;
use ffmpeg_next::software::scaling::{Context as Scaler, Flags as ScaleFlags};

use nativis_asset::AssetPath;
use nativis_core::{
    clock::MediaClock,
    contract::{Frame, FrameStatus, MediaBackend, MediaCapability, MediaError, ResourceHandle},
    resource::{CpuBuffer, ResourceManager},
};

pub use frame::DecodedFrame;

// ── Instrumentation ───────────────────────────────────────────────────────────
const METRICS_INTERVAL: u64 = 300; // log every N published frames

// ── VideoBackend ──────────────────────────────────────────────────────────────

pub struct VideoBackend {
    rx:           Option<Receiver<DecodedFrame>>,
    stop_signal:  Option<Arc<AtomicBool>>,
    /// ResourceManager clone — used to register/free CpuBuffers each tick.
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
        clock: &MediaClock,
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
            // Free the CpuBuffer from the previous tick.
            if let Some(old) = self.last_handle.take() {
                resources.free(old);
            }

            let pts     = df.pts_duration();
            let width   = df.width;
            let height  = df.height;

            // Register the immutable pixel data as a CpuBuffer.
            // Arc::from() is already zero-copy from the decoder side.
            let buf = CpuBuffer {
                data:   df.pixels.to_vec(), // single copy: Arc→Vec for ResourceManager
                width,
                height,
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
    let mut thread_window         = Instant::now();

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

        let ctx = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .expect("codec context");
        let mut decoder = ctx.decoder().video().expect("video decoder");

        let (w, h)  = (decoder.width(), decoder.height());
        let src_fmt = decoder.format();

        let mut scaler = Scaler::get(
            src_fmt, w, h,
            Pixel::RGBA, w, h,
            ScaleFlags::BILINEAR,
        ).expect("scaler context");

        info!(
            "[NATIVIS DECODE] Opened: {}x{} {:?} tb={}/{}",
            w, h, src_fmt, tb.numerator(), tb.denominator()
        );

        let mut raw  = ffmpeg::util::frame::video::Video::empty();
        let mut rgba = ffmpeg::util::frame::video::Video::new(Pixel::RGBA, w, h);

        for (stream_ref, packet) in ictx.packets() {
            if stop.load(Ordering::Acquire) { break 'outer; }
            if stream_ref.index() != video_index { continue; }

            let t0 = Instant::now();
            if decoder.send_packet(&packet).is_err() { continue; }

            while decoder.receive_frame(&mut raw).is_ok() {
                if scaler.run(&raw, &mut rgba).is_err() { continue; }

                // Build immutable DecodedFrame — all FFmpeg data is copied here.
                // `rgba.data(0)` is a slice into FFmpeg's internal buffer.
                // We Arc it immediately so ownership transfers cleanly.
                let pixels: Arc<[u8]> = Arc::from(rgba.data(0));

                let df = DecodedFrame {
                    width:         w,
                    height:        h,
                    pts:           raw.pts().unwrap_or(0),
                    time_base_num: tb.numerator(),
                    time_base_den: tb.denominator(),
                    pixels,
                };

                let us = t0.elapsed().as_micros() as u64;
                total_decode_us += us;
                decode_count    += 1;

                if decode_count % METRICS_INTERVAL == 0 {
                    let elapsed_s = thread_window.elapsed().as_secs_f64();
                    let fps       = decode_count as f64 / elapsed_s.max(0.001);
                    let avg_us    = total_decode_us / decode_count.max(1);
                    info!("[NATIVIS DECODE] decode_fps={:.1} avg_decode_us={}µs", fps, avg_us);
                    decode_count    = 0;
                    total_decode_us = 0;
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

        // EOF — flush to clear any stale B-frames, then restart seamlessly.
        decoder.flush();
        debug!("[NATIVIS DECODE] EOF — restarting for loop playback");
    }

    info!("[NATIVIS DECODE] decoder thread exited cleanly");
}
