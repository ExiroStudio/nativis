//! `Runtime` — the Nativis Frame Orchestrator.
//!
//! The runtime knows nothing about media files, decoders, or rendering.
//! It only knows how to drive a `MediaBackend` and submit its frames to a `FrameSink`.

use std::time::{Duration, Instant};
use tracing::{info, warn};
use crossbeam_channel::Receiver;

use nativis_core::contract::{FrameStatus, MediaBackend, FrameSink};

/// Commands that can be dynamically sent to the running conductor.
pub enum RuntimeCommand {
    /// Hot-swaps the current media backend with a new one without restarting engine or sink.
    SetBackend(Box<dyn MediaBackend>),
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
/// Drives the per-frame pipeline: backend update -> submit to sink.
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
        self.run_with_commands(backend, sink, rx)
    }

    /// Block and run the media loop while listening for dynamic runtime commands.
    pub fn run_with_commands(
        &self,
        mut backend: Box<dyn MediaBackend>,
        mut sink: Box<dyn FrameSink>,
        command_rx: Receiver<RuntimeCommand>,
    ) -> anyhow::Result<()> {
        info!("Runtime started orchestrating: {}", backend.name());

        let mut target_frame_time = Duration::from_secs_f64(1.0 / self.config.target_fps.max(1) as f64);
        let mut last_tick = Instant::now();
        let mut is_paused = false;

        loop {
            // Process any pending commands without blocking the frame loop
            while let Ok(cmd) = command_rx.try_recv() {
                match cmd {
                    RuntimeCommand::SetBackend(new_backend) => {
                        info!("Hot-swapping media backend to: {}", new_backend.name());
                        backend.close();
                        backend = new_backend;
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
                std::thread::sleep(Duration::from_millis(50));
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
            match status {
                FrameStatus::Ready(frame) => {
                    if let Err(e) = sink.submit(frame) {
                        warn!("FrameSink submit error: {}", e);
                    }
                }
                FrameStatus::Unchanged => {
                    // Sink can decide to hold or republish, but typically we do nothing.
                }
                FrameStatus::EndOfStream => {
                    info!("Stream ended.");
                    break;
                }
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
