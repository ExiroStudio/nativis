use nativis_core::Handle;

// ── Marker types for type-safe handles ───────────────────────────────────────
pub struct GpuTexture;
pub struct GpuBuffer;
pub struct GpuShader;
pub struct GpuPipeline;
pub struct GpuSwapchain;

pub type TextureHandle   = Handle<GpuTexture>;
pub type BufferHandle    = Handle<GpuBuffer>;
pub type ShaderHandle    = Handle<GpuShader>;
pub type PipelineHandle  = Handle<GpuPipeline>;
pub type SwapchainHandle = Handle<GpuSwapchain>;

// ── Pixel / texture formats ───────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    Rgba8Unorm,
    Rgba8UnormSrgb,
    Rgba16Float,
    R8Unorm,
    Rg8Unorm,
    /// NV12: Y plane (R8) + interleaved UV plane (RG8), hardware video surfaces.
    Nv12,
    Bgra8Unorm,
    Bgra8UnormSrgb,
}

impl TextureFormat {
    /// Returns the corresponding `wgpu::TextureFormat` for formats supported
    /// by wgpu natively. `Nv12` must be handled via YUV conversion pass.
    pub fn to_wgpu(self) -> Option<wgpu::TextureFormat> {
        match self {
            Self::Rgba8Unorm       => Some(wgpu::TextureFormat::Rgba8Unorm),
            Self::Rgba8UnormSrgb   => Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            Self::Rgba16Float      => Some(wgpu::TextureFormat::Rgba16Float),
            Self::R8Unorm          => Some(wgpu::TextureFormat::R8Unorm),
            Self::Rg8Unorm         => Some(wgpu::TextureFormat::Rg8Unorm),
            Self::Bgra8Unorm       => Some(wgpu::TextureFormat::Bgra8Unorm),
            Self::Bgra8UnormSrgb   => Some(wgpu::TextureFormat::Bgra8UnormSrgb),
            Self::Nv12             => None,
        }
    }
}

// ── Texture descriptor ────────────────────────────────────────────────────────
bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct TextureUsage: u32 {
        const COPY_SRC        = 0b0000_0001;
        const COPY_DST        = 0b0000_0010;
        const SAMPLED         = 0b0000_0100;
        const RENDER_TARGET   = 0b0000_1000;
        const STORAGE         = 0b0001_0000;
    }
}

#[derive(Debug, Clone)]
pub struct TextureDescriptor {
    pub label:   Option<String>,
    pub width:   u32,
    pub height:  u32,
    pub format:  TextureFormat,
    pub usage:   TextureUsage,
    pub mip_levels: u32,
}

impl TextureDescriptor {
    pub fn render_target(width: u32, height: u32, format: TextureFormat) -> Self {
        Self {
            label:      None,
            width, height, format,
            usage:      TextureUsage::SAMPLED | TextureUsage::RENDER_TARGET | TextureUsage::COPY_SRC,
            mip_levels: 1,
        }
    }

    pub fn sampled_2d(width: u32, height: u32, format: TextureFormat) -> Self {
        Self {
            label:      None,
            width, height, format,
            usage:      TextureUsage::COPY_DST | TextureUsage::SAMPLED,
            mip_levels: 1,
        }
    }
}

// ── Buffer descriptor ─────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferUsage { Vertex, Index, Uniform, Storage, Staging }

#[derive(Debug, Clone)]
pub struct BufferDescriptor {
    pub label: Option<String>,
    pub size:  u64,
    pub usage: BufferUsage,
}

// ── Shader descriptor ─────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderStage { Vertex, Fragment, Compute }

#[derive(Debug, Clone)]
pub struct ShaderDescriptor {
    pub label:  Option<String>,
    pub stage:  ShaderStage,
    /// WGSL source code (wgpu backend) or SPIR-V (future Vulkan backend).
    pub source: ShaderSource,
}

#[derive(Debug, Clone)]
pub enum ShaderSource {
    Wgsl(String),
    SpirV(Vec<u32>),
}

// ── Pipeline descriptor ───────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    Opaque,
    AlphaBlend,
    Additive,
    Multiply,
    Screen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CullMode { None, Front, Back }

#[derive(Debug, Clone)]
pub struct PipelineDescriptor {
    pub label:        Option<String>,
    pub vertex_shader:   ShaderHandle,
    pub fragment_shader: ShaderHandle,
    pub blend_mode:   BlendMode,
    pub cull_mode:    CullMode,
    pub depth_write:  bool,
    /// Output texture format (must match the render target).
    pub output_format: TextureFormat,
}

// ── Upload helper ─────────────────────────────────────────────────────────────
/// Raw pixel data to be uploaded to a GPU texture.
pub struct TextureUpload<'a> {
    pub handle: TextureHandle,
    pub data:   &'a [u8],
    pub bytes_per_row: u32,
}

// ── Error ─────────────────────────────────────────────────────────────────────
#[derive(Debug, thiserror::Error)]
pub enum RhiError {
    #[error("No suitable GPU adapter found")]
    NoAdapter,
    #[error("Device creation failed: {0}")]
    DeviceCreation(String),
    #[error("Surface creation failed: {0}")]
    SurfaceCreation(String),
    #[error("Surface configuration failed: no supported format")]
    SurfaceFormat,
    #[error("Invalid handle")]
    InvalidHandle,
    #[error("GPU texture upload failed: {0}")]
    UploadFailed(String),
    #[error("Backend error: {0}")]
    Backend(String),
}

// ── Backend type ──────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendType { Vulkan, Metal, DirectX12, OpenGL, WebGpu }
