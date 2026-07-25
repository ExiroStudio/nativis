//! nativis-audio — Audio capture and real-time spectrum analysis.
//!
//! Phase 1: contracts + no-op stub implementation.
//! Phase 2: WASAPI / PipeWire loopback capture + RustFFT spectrum pipeline.

use thiserror::Error;


pub const SPECTRUM_BANDS: usize = 128;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("No audio device found")]
    NoDevice,
    #[error("Capture error: {0}")]
    Capture(String),
}

/// 128-band frequency spectrum, double-buffered for lock-free access between
/// the audio processing thread and the render thread.
pub struct SpectrumBuffer {
    /// Current band amplitudes (0.0 – 1.0 normalised).
    pub bands: [f32; SPECTRUM_BANDS],
    /// Beat detection output — value spikes on transients.
    pub beat_energy: f32,
}

impl SpectrumBuffer {
    pub fn new() -> Self {
        Self { bands: [0.0; SPECTRUM_BANDS], beat_energy: 0.0 }
    }

    pub fn band(&self, index: usize) -> f32 {
        self.bands.get(index).copied().unwrap_or(0.0)
    }
}

impl Default for SpectrumBuffer { fn default() -> Self { Self::new() } }

/// Audio capture contract. Concrete implementations open system loopback
/// streams and feed PCM samples into an FFT engine.
pub trait IAudioCapture: Send + Sync {
    fn start(&mut self) -> Result<(), AudioError>;
    fn stop(&mut self);
    fn is_active(&self) -> bool;
    /// Latest computed spectrum snapshot — safe to call from render thread.
    fn spectrum(&self) -> &SpectrumBuffer;
}

/// No-op stub: returns a silent spectrum. Replace with OS loopback impl.
pub struct NullAudioCapture {
    spectrum: SpectrumBuffer,
}

impl NullAudioCapture {
    pub fn new() -> Self { Self { spectrum: SpectrumBuffer::new() } }
}

impl IAudioCapture for NullAudioCapture {
    fn start(&mut self) -> Result<(), AudioError> { Ok(()) }
    fn stop(&mut self) {}
    fn is_active(&self) -> bool { false }
    fn spectrum(&self) -> &SpectrumBuffer { &self.spectrum }
}

impl Default for NullAudioCapture { fn default() -> Self { Self::new() } }
