//! `IRhiBackend` — low-level GPU API surface.
//!
//! Only `nativis-render` and `nativis-rhi` itself use this trait.
//! Media backends use `RhiContext` instead.

use crate::types::*;

/// Core GPU abstraction. Implemented by `WgpuBackend` (and future Vulkan/Metal).
pub trait IRhiBackend: Send + Sync {
    fn backend_type(&self) -> BackendType;

    /// Acquire the next swapchain image. Must be called once per frame.
    fn begin_frame(&mut self) -> Result<(), RhiError>;

    /// Resize the swapchain surface.
    fn resize(&mut self, width: u32, height: u32) -> Result<(), RhiError>;

    /// Present the current frame to the display.
    fn present(&mut self) -> Result<(), RhiError>;

    /// Dimensions of the current swapchain surface.
    fn surface_size(&self) -> (u32, u32);

    /// Format of the swapchain surface texture.
    fn surface_format(&self) -> TextureFormat;

    /// Handle to the current swapchain render target.
    fn surface_texture(&self) -> &TextureHandle;

    /// Expose `RhiContext` for creating and uploading textures.
    fn rhi_context(&self) -> crate::RhiContext;
}
