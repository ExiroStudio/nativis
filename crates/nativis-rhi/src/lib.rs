//! nativis-rhi — Rendering Hardware Interface.
//!
//! The RHI is the single seam between engine rendering logic and the concrete
//! GPU API.  All higher-level crates (render-graph, scene, media) operate
//! entirely on opaque handles and the `IRhiBackend` trait.  Only this crate
//! and its backends ever touch wgpu / Vulkan / Metal / D3D12 directly.

pub mod backend;
pub mod types;
pub mod wgpu_backend;

pub use backend::IRhiBackend;
pub use types::*;
pub use wgpu_backend::WgpuBackend;
