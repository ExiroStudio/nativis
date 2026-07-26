//! `MediaClock` — lightweight presentation clock for media synchronization.
//!
//! Tracks elapsed wall-clock time for media backends to synchronize
//! decode and frame presentation. Intentionally minimal.

use std::time::{Duration, Instant};

/// A simple monotonic clock used by media backends to drive playback timing.
///
/// The runtime creates one `MediaClock` and passes it to `MediaBackend::open()`.
/// The backend stores a reference and uses `elapsed()` to schedule frames.
pub struct MediaClock {
    started_at: Instant,
    paused_at: Option<Instant>,
    accumulated: Duration,
}

impl MediaClock {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            paused_at: None,
            accumulated: Duration::ZERO,
        }
    }

    /// Wall-clock time elapsed since the clock was last reset or started.
    pub fn elapsed(&self) -> Duration {
        match self.paused_at {
            Some(p) => self.accumulated + (p - self.started_at),
            None => self.accumulated + self.started_at.elapsed(),
        }
    }

    /// Pause the clock. Subsequent `elapsed()` calls return the same value.
    pub fn pause(&mut self) {
        if self.paused_at.is_none() {
            self.paused_at = Some(Instant::now());
        }
    }

    /// Resume the clock from where it was paused.
    pub fn resume(&mut self) {
        if let Some(p) = self.paused_at.take() {
            self.accumulated += p.elapsed();
            self.started_at = Instant::now();
        }
    }

    /// Reset the clock to zero.
    pub fn reset(&mut self) {
        self.started_at = Instant::now();
        self.paused_at = None;
        self.accumulated = Duration::ZERO;
    }

    pub fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }
}

impl Default for MediaClock {
    fn default() -> Self {
        Self::new()
    }
}
