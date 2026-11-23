//! `Runtime` — the Nativis Frame Orchestrator.
//!
//! The runtime knows nothing about media files, decoders, or rendering.
//! It only knows how to drive a `MediaBackend` and submit its frames to a `FrameSink`.

use std::time::{Duration, Instant};
use tracing::{info, warn};
use crossbeam_channel::{Receiver, RecvTimeoutError};

use nativis_asset::AssetPath;
use nativis_core::clock::MediaClock;
use nativis_core::contract::{FrameStatus, MediaBackend, FrameSink};
use nativis_core::resource::ResourceManager;

/// Commands that can be dynamically sent to the running conductor.
pub enum RuntimeCommand {
    /// Hot-swaps the current media backend with a new one without restarting engine or sink.
    SetBackend(Box<dyn MediaBackend>),
    /// Atomically closes the old backend, opens the new one, and swaps — no parallel decode window.
    ///
    /// Preferred over `SetBackend` when switching wallpapers via IPC because:
    /// - The conductor holds `resources` and `clock`, so it can call `open()` itself.
    /// - The old decoder is fully stopped before the new one starts.
    OpenAndSwap {
        /// Uninitialized backend selected by the plugin manager (supports() returned true).
        backend: Box<dyn MediaBackend>,
        /// Asset path to pass to `backend.open()`.
        source: AssetPath,
        /// Optional sender for notifying caller of open success or failure (C3 rollback).
        reply: Option<crossbeam_channel::Sender<Result<(), String>>>,
    },
    /// Pauses or resumes media clock and frame rendering.
    SetPause(bool),
    /// Toggles the current pause state.
    TogglePause,
    /// Changes the target framerate on-the-fly.
    SetFps(u32),
    /// Gracefully stops the orchestrator loop.
    Stop,
    /// Re-submit the last known frame to the sink without advancing the backend.
    Redraw,
    /// Platform event (Pause, Resume, Redraw) forwarded from the platform layer.
    PlatformEvent(nativis_core::platform::PlatformEvent),
}




/// Target configuration for the runtime orchestrator.
pub struct RuntimeConfig {
    pub target_fps: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self { target_fps: 60 }
    }
}

/// The Nativis frame orchestrator.
///
/// Drives the per-frame pipeline: backend update → submit to sink.
pub struct Runtime {
    config: RuntimeConfig,
}

impl Runtime {
    pub fn new(config: RuntimeConfig) -> Self {
        Self { config }
    }

    /// Block and run the media loop with a static backend.
    pub fn run(
        &self,
        backend: Box<dyn MediaBackend>,
        sink: Box<dyn FrameSink>,
    ) -> anyhow::Result<()> {
        let (_tx, rx) = crossbeam_channel::unbounded();
        let resources = ResourceManager::new();
        let clock = MediaClock::new();
        self.run_with_commands(backend, sink, rx, resources, clock, None)
    }

    /// Block and run the media loop while listening for dynamic runtime commands.
    ///
    /// `resources` and `clock` are stored by the conductor so that it can call
    /// `backend.open()` on behalf of `OpenAndSwap` commands, eliminating the
    /// close-before-open race that would otherwise exist in the IPC thread.
    pub fn run_with_commands(
        &self,
        mut backend: Box<dyn MediaBackend>,
        mut sink: Box<dyn FrameSink>,
        command_rx: Receiver<RuntimeCommand>,
        resources: ResourceManager,
        clock: MediaClock,
        initial_source: Option<AssetPath>,
    ) -> anyhow::Result<()> {
        info!("Runtime started orchestrating: {}", backend.name());

        let mut target_frame_time = Duration::from_secs_f64(1.0 / self.config.target_fps.max(1) as f64);
        let mut last_tick = Instant::now();
        // User pause (IPC) and platform pause (fullscreen/lock/display-off) are tracked
        // separately so a platform Resume can never override an explicit user pause.
        let mut user_paused = false;
        let mut platform_paused = false;
        let mut is_paused = false;
        let mut pending_cmd: Option<RuntimeCommand> = None;
        // Last successfully submitted frame — used for Redraw (throttle resume, display change,
        // Explorer crash recovery) so the platform doesn't need the original CpuBuffer to be alive.
        let mut last_frame: Option<nativis_core::contract::Frame> = None;
        let mut current_source: Option<AssetPath> = initial_source;


        loop {
            // Process any pending commands without blocking the frame loop
            while let Some(cmd) = pending_cmd.take().or_else(|| command_rx.try_recv().ok()) {
                match cmd {
                    RuntimeCommand::SetBackend(new_backend) => {
                        info!("Hot-swapping media backend to: {}", new_backend.name());
                        backend.close();
                        // The old backend's handles are gone; never redraw them.
                        last_frame = None;
                        backend = new_backend;
                    }
                    RuntimeCommand::OpenAndSwap { backend: mut new_backend, source, reply } => {
                        // Close old backend first so only one decoder runs at a time (K4).
                        info!(
                            "OpenAndSwap: closing '{}', opening '{}' for '{}'",
                            backend.name(),
                            new_backend.name(),
                            source.raw_uri(),
                        );
                        backend.close();
                        // The old backend's handles are gone; never redraw them.
                        last_frame = None;
                        match new_backend.open(&source, &clock, &resources) {
                            Ok(()) => {
                                backend = new_backend;
                                current_source = Some(source);
                                if let Some(tx) = reply {
                                    let _ = tx.send(Ok(()));
                                }
                            }
                            Err(e) => {
                                warn!(
                                    "OpenAndSwap: failed to open '{}' for '{}': {}. Attempting rollback to previous wallpaper.",
                                    new_backend.name(),
                                    source.raw_uri(),
                                    e,
                                );
                                if let Some(ref old_source) = current_source {
                                    if let Err(re_err) = backend.open(old_source, &clock, &resources) {
                                        warn!("OpenAndSwap rollback failed: {}", re_err);
                                    } else {
                                        info!("OpenAndSwap: successfully rolled back to '{}'", old_source.raw_uri());
                                    }
                                }
                                if let Some(tx) = reply {
                                    let _ = tx.send(Err(e.to_string()));
                                }
                            }
                        }
                    }
                    RuntimeCommand::SetPause(paused) => {
                        user_paused = paused;
                        is_paused = user_paused || platform_paused;
                        info!("Playback paused: {} (user={}, platform={})", is_paused, user_paused, platform_paused);
                    }
                    RuntimeCommand::TogglePause => {
                        user_paused = !user_paused;
                        is_paused = user_paused || platform_paused;
                        info!("Playback paused: {} (user={}, platform={})", is_paused, user_paused, platform_paused);
                    }
                    RuntimeCommand::SetFps(fps) => {
                        target_frame_time = Duration::from_secs_f64(1.0 / fps.max(1) as f64);
                        info!("Target framerate updated to: {} FPS", fps);
                    }
                    RuntimeCommand::Stop => {
                        info!("Stop command received, terminating runtime orchestrator.");
                        backend.close();
                        return Ok(());
                    }
                    RuntimeCommand::Redraw => {
                        // Platform requested a re-draw (throttle resume, WM_DISPLAYCHANGE,
                        // Explorer restart). Re-submit the last frame without touching the backend.
                        // D3D11Renderer keeps rgba_texture / NV12 textures alive in VRAM — no RAM needed.
                        if let Some(ref frame) = last_frame {
                            if let Err(e) = sink.submit(frame.clone()) {
                                warn!("Redraw: sink.submit error: {}", e);
                            }
                        }
                    }
                    RuntimeCommand::PlatformEvent(ev) => match ev {
                        nativis_core::platform::PlatformEvent::Pause => {
                            platform_paused = true;
                            is_paused = true;
                            info!("Platform event: Playback PAUSED (e.g. fullscreen/display off)");
                        }
                        nativis_core::platform::PlatformEvent::Resume => {
                            platform_paused = false;
                            is_paused = user_paused || platform_paused;
                            info!("Platform event: platform pause cleared (playback paused={})", is_paused);
                            if let Some(ref frame) = last_frame {
                                if let Err(e) = sink.submit(frame.clone()) {
                                    warn!("PlatformEvent::Resume redraw error: {}", e);
                                }
                            }
                        }
                        nativis_core::platform::PlatformEvent::Redraw => {
                            if let Some(ref frame) = last_frame {
                                if let Err(e) = sink.submit(frame.clone()) {
                                    warn!("PlatformEvent::Redraw error: {}", e);
                                }
                            }
                        }
                    }
                }
            }



            if is_paused {
                // True 0.00% CPU usage when paused (game running, display sleep, or user paused):
                // Block unconditionally on command channel until a command (Resume, SetWallpaper, Stop) arrives.
                //
                // Item 7 fix: `recv()` returns Err(RecvError) when ALL senders have dropped.
                // Previously this fell through `if let Ok` silently and re-entered the loop
                // at ~100% CPU (measured 99.4%). Now we break and shut down gracefully.
                match command_rx.recv() {
                    Ok(cmd) => pending_cmd = Some(cmd),
                    Err(_)  => {
                        info!("Conductor: command channel disconnected while paused — shutting down.");
                        break;
                    }
                }
                last_tick = Instant::now();
                continue;
            }



            let now = Instant::now();
            let dt = now.duration_since(last_tick);
            last_tick = now;

            // 1. Advance media clock
            if let Err(e) = backend.update(dt) {
                warn!("Media backend update error: {}", e);
            }

            // 2. Fetch the latest frame
            let status = backend.current_frame();
            
            // 3. Submit to transport sink
            let mut is_idle = false;
            match status {
                FrameStatus::Ready(frame) => {
                    // Remember last submitted frame for Redraw (throttle resume, display change).
                    last_frame = Some(frame.clone());
                    if let Err(e) = sink.submit(frame) {
                        warn!("FrameSink submit error: {}", e);
                    }
                }
                FrameStatus::Unchanged => {
                    // Sink can decide to hold or republish, but typically we do nothing.
                }
                FrameStatus::Idle => {
                    // Phase 2: Static content is idle — zero active polling wakeups required.
                    is_idle = true;
                }
                FrameStatus::EndOfStream => {
                    info!("Stream ended.");
                    break;
                }
            }


            if is_idle {
                // Phase 2 Event-Driven Cadence:
                // Block until a new runtime command arrives (e.g. SetWallpaper, Pause, Stop)
                // or 1-second watchdog timeout. This replaces the 60 Hz busy loop for static
                // content, reducing CPU wakeups to 0/sec while idle.
                //
                // Item 7 fix: `recv_timeout` returns Err(Disconnected) instantly when all senders
                // have dropped, causing the loop to spin at ~100% CPU (measured 99.8%).
                // Break on disconnect to shut down gracefully instead.
                match command_rx.recv_timeout(Duration::from_secs(1)) {
                    Ok(cmd)                             => pending_cmd = Some(cmd),
                    Err(RecvTimeoutError::Timeout)      => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        info!("Conductor: command channel disconnected while idle — shutting down.");
                        break;
                    }
                }
                last_tick = Instant::now();
                continue;
            }


            // Dynamic frame pacing via backend.next_deadline() (Items 6 & 8)
            // Allows 30 fps video to sleep ~33ms, waking conductor ~30 times/s instead of 60.
            // Using recv_timeout ensures immediate responsiveness if a command arrives mid-sleep.
            let sleep_duration = match backend.next_deadline() {
                Some(deadline) => deadline,
                None => {
                    let elapsed = now.elapsed();
                    if elapsed < target_frame_time {
                        target_frame_time - elapsed
                    } else {
                        Duration::ZERO
                    }
                }
            };

            if sleep_duration > Duration::ZERO {
                let capped = sleep_duration.min(target_frame_time * 2);
                match command_rx.recv_timeout(capped) {
                    Ok(cmd)                             => pending_cmd = Some(cmd),
                    Err(RecvTimeoutError::Timeout)      => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        info!("Conductor: command channel disconnected — shutting down.");
                        break;
                    }
                }
            }
        }



        info!("Runtime orchestrator finished.");
        backend.close();
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use nativis_core::contract::{Frame, MediaCapability, MediaError, ResourceHandle};
    use nativis_core::platform::PlatformEvent;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct MockBackend {
        updates: Arc<AtomicUsize>,
        status: FrameStatus,
    }

    impl MediaBackend for MockBackend {
        fn name(&self) -> &'static str { "mock" }
        fn open(&mut self, _s: &AssetPath, _c: &MediaClock, _r: &ResourceManager) -> Result<(), MediaError> { Ok(()) }
        fn close(&mut self) {}
        fn update(&mut self, _dt: Duration) -> Result<(), MediaError> {
            self.updates.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn current_frame(&self) -> FrameStatus { self.status.clone() }
        fn capabilities(&self) -> &[MediaCapability] { &[] }
        fn supports(&self, _s: &AssetPath) -> bool { true }
    }

    struct MockSink { submits: Arc<Mutex<Vec<u64>>> }

    impl FrameSink for MockSink {
        fn submit(&mut self, frame: Frame) -> Result<(), MediaError> {
            self.submits.lock().unwrap().push(frame.resource.0);
            Ok(())
        }
    }

    fn spawn_runtime(
        backend: MockBackend,
        sink: MockSink,
        fps: u32,
    ) -> (crossbeam_channel::Sender<RuntimeCommand>, std::thread::JoinHandle<()>) {
        let (tx, rx) = unbounded();
        let h = std::thread::spawn(move || {
            Runtime::new(RuntimeConfig { target_fps: fps })
                .run_with_commands(Box::new(backend), Box::new(sink), rx, ResourceManager::new(), MediaClock::new(), None)
                .unwrap();
        });
        (tx, h)
    }

    #[test]
    fn user_pause_survives_platform_resume() {
        let updates = Arc::new(AtomicUsize::new(0));
        let (tx, h) = spawn_runtime(
            MockBackend { updates: updates.clone(), status: FrameStatus::Unchanged },
            MockSink { submits: Arc::new(Mutex::new(vec![])) },
            120,
        );
        std::thread::sleep(Duration::from_millis(30));
        tx.send(RuntimeCommand::SetPause(true)).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let paused_at = updates.load(Ordering::SeqCst);

        tx.send(RuntimeCommand::PlatformEvent(PlatformEvent::Pause)).unwrap();
        tx.send(RuntimeCommand::PlatformEvent(PlatformEvent::Resume)).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(updates.load(Ordering::SeqCst), paused_at, "platform Resume overrode the user pause");

        tx.send(RuntimeCommand::SetPause(false)).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        assert!(updates.load(Ordering::SeqCst) > paused_at, "did not resume after the user unpaused");

        tx.send(RuntimeCommand::Stop).unwrap();
        h.join().unwrap();
    }

    #[test]
    fn platform_pause_blocks_and_resume_restarts() {
        let updates = Arc::new(AtomicUsize::new(0));
        let (tx, h) = spawn_runtime(
            MockBackend { updates: updates.clone(), status: FrameStatus::Unchanged },
            MockSink { submits: Arc::new(Mutex::new(vec![])) },
            120,
        );
        tx.send(RuntimeCommand::PlatformEvent(PlatformEvent::Pause)).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        let a = updates.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(updates.load(Ordering::SeqCst), a, "conductor kept updating while platform-paused");

        tx.send(RuntimeCommand::PlatformEvent(PlatformEvent::Resume)).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        assert!(updates.load(Ordering::SeqCst) > a);
        tx.send(RuntimeCommand::Stop).unwrap();
        h.join().unwrap();
    }

    #[test]
    fn open_and_swap_clears_redraw_frame() {
        let submits = Arc::new(Mutex::new(Vec::new()));
        let frame = Frame::new(ResourceHandle(42), 16, 16, Duration::ZERO, true);
        let (tx, h) = spawn_runtime(
            MockBackend { updates: Arc::new(AtomicUsize::new(0)), status: FrameStatus::Ready(frame) },
            MockSink { submits: submits.clone() },
            60,
        );
        std::thread::sleep(Duration::from_millis(40));
        assert!(!submits.lock().unwrap().is_empty(), "first frame was never submitted");

        let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
        tx.send(RuntimeCommand::OpenAndSwap {
            backend: Box::new(MockBackend { updates: Arc::new(AtomicUsize::new(0)), status: FrameStatus::Unchanged }),
            source: AssetPath::parse("memory://x.png").unwrap(),
            reply: Some(reply_tx),
        }).unwrap();
        reply_rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
        submits.lock().unwrap().clear();

        tx.send(RuntimeCommand::Redraw).unwrap();
        tx.send(RuntimeCommand::PlatformEvent(PlatformEvent::Resume)).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        assert!(submits.lock().unwrap().is_empty(), "stale frame from the closed backend was re-submitted");

        tx.send(RuntimeCommand::Stop).unwrap();
        h.join().unwrap();
    }

    #[test]
    fn exits_when_command_channel_disconnects_while_idle() {
        let (tx, h) = spawn_runtime(
            MockBackend { updates: Arc::new(AtomicUsize::new(0)), status: FrameStatus::Idle },
            MockSink { submits: Arc::new(Mutex::new(vec![])) },
            60,
        );
        std::thread::sleep(Duration::from_millis(30));
        drop(tx);
        h.join().unwrap(); // would hang (or spin) if disconnect were not handled
    }

    #[test]
    fn exits_when_command_channel_disconnects_while_paused() {
        let (tx, h) = spawn_runtime(
            MockBackend { updates: Arc::new(AtomicUsize::new(0)), status: FrameStatus::Unchanged },
            MockSink { submits: Arc::new(Mutex::new(vec![])) },
            60,
        );
        tx.send(RuntimeCommand::SetPause(true)).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        drop(tx);
        h.join().unwrap();
    }
}
