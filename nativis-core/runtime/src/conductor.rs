//! `Runtime` — the Nativis Frame Orchestrator.
//!
//! The runtime knows nothing about media files, decoders, or rendering.
//! It only knows how to drive a `MediaBackend` and submit its frames to a `FrameSink`.

use std::time::{Duration, Instant};
use tracing::{info, warn};
use crossbeam_channel::Receiver;

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
    },
    /// Pauses or resumes media clock and frame rendering.
    SetPause(bool),
    /// Toggles the current pause state.
    TogglePause,
    /// Changes the target framerate on-the-fly.
    SetFps(u32),
    /// Gracefully stops the orchestrator loop.
    Stop,
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
        self.run_with_commands(backend, sink, rx, resources, clock)
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
    ) -> anyhow::Result<()> {
        info!("Runtime started orchestrating: {}", backend.name());

        let mut target_frame_time = Duration::from_secs_f64(1.0 / self.config.target_fps.max(1) as f64);
        let mut last_tick = Instant::now();
        let mut is_paused = false;
        let mut pending_cmd: Option<RuntimeCommand> = None;

        loop {
            // Process any pending commands without blocking the frame loop
            while let Some(cmd) = pending_cmd.take().or_else(|| command_rx.try_recv().ok()) {
                match cmd {
                    RuntimeCommand::SetBackend(new_backend) => {
                        info!("Hot-swapping media backend to: {}", new_backend.name());
                        backend.close();
                        backend = new_backend;
                    }
                    RuntimeCommand::OpenAndSwap { backend: mut new_backend, source } => {
                        // Close old backend first so only one decoder runs at a time (K4).
                        info!(
                            "OpenAndSwap: closing '{}', opening '{}' for '{}'",
                            backend.name(),
                            new_backend.name(),
                            source.raw_uri(),
                        );
                        backend.close();
                        if let Err(e) = new_backend.open(&source, &clock, &resources) {
                            warn!(
                                "OpenAndSwap: failed to open '{}' for '{}': {}. Keeping previous backend closed.",
                                new_backend.name(),
                                source.raw_uri(),
                                e,
                            );
                            // Keep old backend reference alive (now closed) rather than
                            // replacing with a half-initialized new one.
                        } else {
                            backend = new_backend;
                        }
                    }
                    RuntimeCommand::SetPause(paused) => {
                        is_paused = paused;
                        info!("Playback paused state changed to: {}", is_paused);
                    }
                    RuntimeCommand::TogglePause => {
                        is_paused = !is_paused;
                        info!("Playback pause toggled to: {}", is_paused);
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
                }
            }

            if is_paused {
                // True 0.00% CPU usage when paused (game running, display sleep, or user paused):
                // Block unconditionally on command channel until a command (Resume, SetWallpaper, Stop) arrives.
                if let Ok(cmd) = command_rx.recv() {
                    pending_cmd = Some(cmd);
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
                if let Ok(cmd) = command_rx.recv_timeout(Duration::from_secs(1)) {
                    pending_cmd = Some(cmd);
                }
                last_tick = Instant::now();
                continue;
            }

            // Simple sleep-based rate limiting (a real engine uses vsync/presentation feedback)
            let elapsed = now.elapsed();
            if elapsed < target_frame_time {
                std::thread::sleep(target_frame_time - elapsed);
            }
        }


        info!("Runtime orchestrator finished.");
        backend.close();
        
        Ok(())
    }
}
