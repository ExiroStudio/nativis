/// Contract for optional post-processing effects.
///
/// Post-effects are stateless and receive an input texture + output texture.
/// They are applied in the order registered on the `Renderer`.
pub trait PostEffect: Send + Sync {
    /// Human-readable name (e.g. `"blur"`, `"tone_map"`).
    fn name(&self) -> &'static str;

    /// Execute the effect.
    ///
    /// - `device` / `queue` are borrowed from the RHI for this call only.
    /// - `input`  is the texture produced by the previous pass.
    /// - `output` is the texture to write the result into.
    fn apply(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        input: &wgpu::TextureView,
        output: &wgpu::TextureView,
    );
}
