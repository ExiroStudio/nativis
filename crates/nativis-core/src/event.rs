//! `EngineEvent` — explicit system-level notification enum.
//!
//! Replaces a generic pub/sub event bus. All runtime lifecycle signals
//! are variants of this enum. No anonymous topic strings.

/// System-level events dispatched by the runtime conductor.
///
/// Components receive these by value; there is no subscription mechanism.
/// The runtime delivers events to whichever subsystem is relevant.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum EngineEvent {
    /// A monitor's resolution or scale factor changed.
    MonitorChanged { width: u32, height: u32, scale: f64 },
    /// The system entered or left a low-power mode.
    PowerStateChanged { low_power_mode: bool },
    /// A media backend plugin was successfully registered.
    PluginRegistered { name: String },
    /// The active media source changed state.
    MediaStateChanged { state: MediaState },
    /// The runtime is about to shut down.
    Shutdown,
}

/// Lifecycle state of the active media backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaState {
    Opening,
    Playing,
    Paused,
    EndOfStream,
    Error,
}
