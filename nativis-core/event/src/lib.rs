//! nativis-event — Decoupled publish-subscribe engine event bus.
//!
//! All subsystems communicate via `EngineEvent` rather than direct callbacks.
//! The `EventBus` dispatches to all registered listeners synchronously within
//! the *Event Polling* phase of the frame scheduler.

pub mod bus;
pub mod events;

pub use bus::EventBus;
pub use events::EngineEvent;
