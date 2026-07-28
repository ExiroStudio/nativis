//! nativis-rhi — Rendering Hardware Interface.
//!
//! Provides opaque GPU handles (`TextureHandle`, `BufferHandle`) and the
//! `RhiContext` passed to media backends at `open()` time. Engine code above
//! this layer never touches wgpu / Vulkan / Metal / D3D12 directly.

pub mod backend;
pub mod context;
pub mod types;
pub mod wgpu_backend;

pub use backend::IRhiBackend;
pub use context::RhiContext;
pub use types::*;
pub use wgpu_backend::WgpuBackend;
