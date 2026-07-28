//! `Runtime` — the Nativis Frame Orchestrator.
//!
//! The runtime knows nothing about media files, decoders, or rendering.
//! It only knows how to drive a `MediaBackend` and submit its frames to a `FrameSink`.

use std::time::{Duration, Instant};
use tracing::{info, warn};

use nativis_core::contract::{FrameStatus, MediaBackend, FrameSink};

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

    /// Block and run the media loop.
    pub fn run(
        &self,
        mut backend: Box<dyn MediaBackend>,
        mut sink: Box<dyn FrameSink>,
    ) -> anyhow::Result<()> {
        info!("Runtime started orchestrating: {}", backend.name());

        let target_frame_time = Duration::from_secs_f64(1.0 / self.config.target_fps as f64);
        let mut last_tick = Instant::now();

        loop {
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
