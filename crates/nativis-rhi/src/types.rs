//! RHI type definitions: opaque handles, formats, descriptors, errors.
//!
//! No wgpu types are re-exported. Engine code above this crate uses only
//! the types defined here.

use std::sync::Arc;

// ── TextureHandle ─────────────────────────────────────────────────────────────

/// Opaque handle to a GPU-resident texture.
///
/// Lifetime managed by Arc. Clone is cheap. Dropping the last clone frees
/// the GPU texture and view.
#[derive(Clone)]
pub struct TextureHandle {
    view:    Arc<wgpu::TextureView>,
    texture: Arc<wgpu::Texture>,
    width:   u32,
    height:  u32,
}

impl TextureHandle {
    pub(crate) fn from_arc_view(
        view: Arc<wgpu::TextureView>,
        texture: Arc<wgpu::Texture>,
        width: u32,
        height: u32,
    ) -> Self {
        Self { view, texture, width, height }
    }

    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }

    /// Access the raw wgpu TextureView and Texture for internal RHI use only.
    /// Returns `None` if the handle is invalid (should not happen in practice).
    pub(crate) fn raw_refs(&self) -> Option<(&wgpu::TextureView, &wgpu::Texture)> {
        Some((&self.view, &self.texture))
    }

    /// Expose the inner `wgpu::TextureView` for the render engine only.
    /// This must not be called from media plugin code.
    pub fn wgpu_view(&self) -> &wgpu::TextureView {
        &self.view
    }
}

impl std::fmt::Debug for TextureHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextureHandle({}x{})", self.width, self.height)
    }
}

// ── Pixel / texture formats ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    Rgba8Unorm,
    Rgba8UnormSrgb,
    Rgba16Float,
    R8Unorm,
    Rg8Unorm,
    Bgra8Unorm,
    Bgra8UnormSrgb,
    /// NV12: Y plane (R8) + interleaved UV plane (RG8), for hardware video.
    Nv12,
}

impl TextureFormat {
    pub fn to_wgpu(self) -> Option<wgpu::TextureFormat> {
        match self {
            Self::Rgba8Unorm     => Some(wgpu::TextureFormat::Rgba8Unorm),
            Self::Rgba8UnormSrgb => Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            Self::Rgba16Float    => Some(wgpu::TextureFormat::Rgba16Float),
            Self::R8Unorm        => Some(wgpu::TextureFormat::R8Unorm),
            Self::Rg8Unorm       => Some(wgpu::TextureFormat::Rg8Unorm),
            Self::Bgra8Unorm     => Some(wgpu::TextureFormat::Bgra8Unorm),
            Self::Bgra8UnormSrgb => Some(wgpu::TextureFormat::Bgra8UnormSrgb),
            Self::Nv12           => None,
        }
    }

    pub fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::R8Unorm => 1,
            Self::Rg8Unorm => 2,
            Self::Rgba8Unorm | Self::Rgba8UnormSrgb
            | Self::Bgra8Unorm | Self::Bgra8UnormSrgb => 4,
            Self::Rgba16Float => 8,
            Self::Nv12 => 1, // Y plane only
        }
    }
}

// ── TextureUsage ──────────────────────────────────────────────────────────────

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct TextureUsage: u32 {
        const COPY_SRC      = 0b0000_0001;
        const COPY_DST      = 0b0000_0010;
        const SAMPLED       = 0b0000_0100;
        const RENDER_TARGET = 0b0000_1000;
        const STORAGE       = 0b0001_0000;
    }
}

// ── Descriptors ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TextureDescriptor {
    pub label:      Option<String>,
    pub width:      u32,
    pub height:     u32,
    pub format:     TextureFormat,
    pub usage:      TextureUsage,
    pub mip_levels: u32,
}

impl TextureDescriptor {
    /// Convenience constructor for a 2D sampled texture (media frame upload).
    pub fn media_frame(width: u32, height: u32, format: TextureFormat) -> Self {
        Self {
            label: Some("media_frame".into()),
            width,
            height,
            format,
            usage: TextureUsage::COPY_DST | TextureUsage::SAMPLED,
            mip_levels: 1,
        }
    }

    /// Convenience constructor for a render target (offscreen post-processing).
    pub fn render_target(width: u32, height: u32, format: TextureFormat) -> Self {
        Self {
            label: Some("render_target".into()),
            width,
            height,
            format,
            usage: TextureUsage::SAMPLED | TextureUsage::RENDER_TARGET | TextureUsage::COPY_SRC,
            mip_levels: 1,
        }
    }
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum RhiError {
    #[error("No suitable GPU adapter found")]
    NoAdapter,
    #[error("Device creation failed: {0}")]
    DeviceCreation(String),
    #[error("Surface creation failed: {0}")]
    SurfaceCreation(String),
    #[error("Surface format not supported")]
    SurfaceFormat,
    #[error("Invalid texture handle")]
    InvalidHandle,
    #[error("GPU upload failed: {0}")]
    UploadFailed(String),
    #[error("Backend error: {0}")]
    Backend(String),
}

// ── Backend type tag ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendType { Vulkan, Metal, DirectX12, OpenGL, WebGpu }

// ── Texture upload helper (for IRhiBackend::upload_texture_data) ─────────────

pub struct TextureUpload<'a> {
    pub handle:        &'a TextureHandle,
    pub data:          &'a [u8],
    pub bytes_per_row: u32,
}
