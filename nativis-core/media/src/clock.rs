use std::time::Instant;

/// Monotonic nanosecond master clock used by all media sources for
/// presentation-timestamp (PTS) comparison. Drives the entire media pipeline.
pub struct MediaClock {
    start:     Instant,
    elapsed_ns: u64,
    paused:    bool,
    pause_ns:  u64,
}

impl MediaClock {
    pub fn new() -> Self {
        Self {
            start:      Instant::now(),
            elapsed_ns: 0,
            paused:     false,
            pause_ns:   0,
        }
    }

    /// Advance the clock by one real-time tick. Call once per frame.
    pub fn tick(&mut self) {
        if !self.paused {
            self.elapsed_ns = self.start.elapsed().as_nanos() as u64;
        }
    }

    /// Current master clock time in nanoseconds.
    #[inline]
    pub fn now_ns(&self) -> u64 { self.elapsed_ns }

    /// Current master clock time in seconds.
    #[inline]
    pub fn now_sec(&self) -> f64 { self.elapsed_ns as f64 / 1_000_000_000.0 }

    pub fn pause(&mut self) {
        if !self.paused {
            self.paused = true;
            self.pause_ns = self.elapsed_ns;
        }
    }

    pub fn resume(&mut self) {
        if self.paused {
            self.paused = false;
            // Rebase start so elapsed continues from where it paused.
            let already = std::time::Duration::from_nanos(self.pause_ns);
            self.start = Instant::now() - already;
        }
    }

    pub fn is_paused(&self) -> bool { self.paused }
}

impl Default for MediaClock {
    fn default() -> Self { Self::new() }
}
