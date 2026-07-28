//! nativis-core — Frozen core contracts for the Nativis multimedia runtime.
//!
//! # Design Invariants
//!
//! - No crate below this layer.
//! - No GPU driver types (wgpu, Vulkan, Metal) leak out of this crate.
//! - Every type here must be explainable in one sentence.
//!
//! # Modules
//!
//! - [`contract`]  — The frozen `MediaBackend` trait and associated types.
//! - [`event`]     — `EngineEvent` enum for system-level notifications.
//! - [`clock`]     — Lightweight media presentation clock.

pub mod clock;
pub mod contract;
pub mod event;
pub mod resource;
pub mod platform;

pub use clock::MediaClock;
pub use contract::{FrameStatus, MediaBackend, MediaCapability, Frame, FrameSink, ResourceHandle};
pub use event::EngineEvent;
pub use resource::{ResourceManager, Resource, CpuBuffer};
pub use platform::Platform;
