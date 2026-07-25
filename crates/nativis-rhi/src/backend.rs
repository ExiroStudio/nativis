use crate::types::*;

/// The core GPU abstraction contract. Every concrete graphics API backend
/// (wgpu/Vulkan/Metal/D3D12) implements this trait. Engine code above this
/// layer interacts exclusively with these methods and opaque handles.
pub trait IRhiBackend: Send + Sync {
    // ── Identity ─────────────────────────────────────────────────────────────
    fn backend_type(&self) -> BackendType;

    // ── Lifecycle ────────────────────────────────────────────────────────────
    /// Called once per frame before any rendering work is encoded.
    fn begin_frame(&mut self) -> Result<(), RhiError>;
    /// Resize the swapchain surface (called on `WindowResized` events).
    fn resize(&mut self, width: u32, height: u32) -> Result<(), RhiError>;
    /// Present the current frame to the display.
    fn present(&mut self) -> Result<(), RhiError>;

    // ── Resource creation ─────────────────────────────────────────────────────
    fn create_texture(&mut self, desc: &TextureDescriptor)
        -> Result<TextureHandle, RhiError>;
    fn destroy_texture(&mut self, handle: TextureHandle);

    fn create_buffer(&mut self, desc: &BufferDescriptor)
        -> Result<BufferHandle, RhiError>;
    fn destroy_buffer(&mut self, handle: BufferHandle);

    fn create_shader(&mut self, desc: ShaderDescriptor)
        -> Result<ShaderHandle, RhiError>;
    fn create_pipeline(&mut self, desc: &PipelineDescriptor)
        -> Result<PipelineHandle, RhiError>;

    // ── Data upload ──────────────────────────────────────────────────────────
    /// Upload raw pixel bytes into an existing texture (COPY_DST must be set).
    fn upload_texture_data(&mut self, upload: TextureUpload<'_>) -> Result<(), RhiError>;

    /// Upload bytes into a buffer.
    fn write_buffer(&mut self, handle: BufferHandle, offset: u64, data: &[u8]);

    // ── Query ────────────────────────────────────────────────────────────────
    fn surface_format(&self) -> TextureFormat;
    fn surface_size(&self) -> (u32, u32);

    /// Borrow the current swapchain texture handle so passes can render into it.
    fn current_surface_texture(&self) -> TextureHandle;
}
