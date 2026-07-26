//! nativis-runtime — The central conductor of the Nativis wallpaper runtime.
//!
//! Responsibility: Orchestrate the 4-stage pipeline each frame tick.
//!
//! ```text
//!                  Runtime (Conductor)
//!                        │
//!        ┌───────────────┴───────────────┐
//!        │                               │
//!        ▼                               ▼
//!  Media Backend                  Wallpaper Backend
//!  (media decode)                 (OS presentation)
//!        │                               ▲
//!        └──────────► Renderer ──────────┘
//! ```
//!
//! The runtime never names a backend type. It delegates backend selection
//! to `PluginRegistry` via `registry.create_backend(&asset_path)`.

pub mod conductor;

pub use conductor::{Runtime, RuntimeConfig};
