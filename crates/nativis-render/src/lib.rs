//! nativis-render — Fixed 4-pass render engine.
//!
//! Responsibility: Draw a `RenderFrame` GPU texture onto the swapchain surface.
//!
//! The renderer never decodes media. It never knows whether the source
//! is an image, video, shader, or webview. It only operates on GPU textures.
//!
//! # Pass sequence (hardcoded, not a DAG)
//!
//! 1. **Acquire**     — Get the current swapchain render target from the RHI.
//! 2. **Composite**   — Blit the media `RenderFrame` texture onto the surface.
//! 3. **Post-Effect** — Optional chain of stateless post-processing passes.
//! 4. **Present**     — Hand the surface back to the RHI for presentation.

pub mod blit;
pub mod renderer;
pub mod post;

pub use renderer::Renderer;
pub use post::PostEffect;
