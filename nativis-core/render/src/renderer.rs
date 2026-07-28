//! `Renderer` — the fixed 4-pass render engine.
//!
//! Responsibility: render a `RenderFrame` to the swapchain surface.
//!
//! Passes (fixed, not a DAG):
//!   1. Acquire  — IRhiBackend::begin_frame()
//!   2. Composite — BlitPipeline draws media texture onto surface
//!   3. Post      — Optional PostEffect chain
//!   4. Present  — IRhiBackend::present()

use nativis_core::contract::{FrameStatus, Frame};
use nativis_rhi::{IRhiBackend, TextureFormat};
use tracing::{debug, warn};

use crate::blit::BlitPipeline;
use crate::post::PostEffect;

/// The render engine. Created once. Never knows what the media source is.
pub struct Renderer {
    blit:        Option<BlitPipeline>,
    post_effects: Vec<Box<dyn PostEffect>>,
    last_frame:  Option<Frame>,
}

impl Renderer {
    pub fn new() -> Self {
        Self {
            blit:         None,
            post_effects: Vec::new(),
            last_frame:   None,
        }
    }

    /// Initialize the blit pipeline. Must be called after RHI is ready.
    pub fn init(&mut self, rhi: &dyn IRhiBackend) {
        // Build the blit pipeline for the surface format.
        let fmt = rhi.surface_format();
        let wgpu_fmt = match fmt {
            TextureFormat::Bgra8Unorm     => wgpu::TextureFormat::Bgra8Unorm,
            TextureFormat::Bgra8UnormSrgb => wgpu::TextureFormat::Bgra8UnormSrgb,
            TextureFormat::Rgba8Unorm     => wgpu::TextureFormat::Rgba8Unorm,
            TextureFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
            _ => wgpu::TextureFormat::Rgba8Unorm,
        };

        let ctx = rhi.rhi_context();
        self.blit = Some(BlitPipeline::new(ctx.device(), wgpu_fmt));
    }

    /// Register an optional post-processing effect.
    pub fn add_post_effect(&mut self, effect: Box<dyn PostEffect>) {
        self.post_effects.push(effect);
    }

    /// Execute one full render frame.
    ///
    /// Called by the runtime conductor each tick, after `MediaBackend::update()`.
    pub fn draw(&mut self, frame_status: FrameStatus, rhi: &mut dyn IRhiBackend) {
        // ── Pass 1: Acquire ──────────────────────────────────────────────────
        if let Err(e) = rhi.begin_frame() {
            warn!("begin_frame failed: {e}");
            return;
        }

        // ── Pass 2: Composite ────────────────────────────────────────────────
        let blit = match &self.blit {
            Some(b) => b,
            None => {
                warn!("Renderer not initialized — call init() before draw()");
                let _ = rhi.present();
                return;
            }
        };

        let render_frame = match frame_status {
            FrameStatus::Ready(f) => {
                self.last_frame = Some(f.clone());
                Some(f)
            }
            FrameStatus::Unchanged => {
                self.last_frame.clone()
            }
            FrameStatus::EndOfStream => {
                debug!("Media end of stream — holding last frame");
                self.last_frame.clone()
            }
        };

        if let Some(frame) = render_frame {
            let surface_view = rhi.surface_texture().wgpu_view();
            let ctx = rhi.rhi_context();

            // Post-effects: if any are registered, render to an intermediate
            // texture, then chain through each effect to the surface.
            // For now (no effects), blit directly to surface.
            
            // FIXME(V6): Renderer needs access to ResourceManager to resolve Frame::resource
            // into a TextureHandle for wgpu blitting. Disabled temporarily.
            /*
            if self.post_effects.is_empty() {
                blit.execute(
                    ctx.device(),
                    ctx.queue(),
                    frame.texture.wgpu_view(),
                    surface_view,
                );
            } else {
                // TODO: chain intermediate textures for each PostEffect.
                // For now fall back to direct blit.
                blit.execute(
                    ctx.device(),
                    ctx.queue(),
                    frame.texture.wgpu_view(),
                    surface_view,
                );
            }
            */
        }

        // ── Pass 4: Present ───────────────────────────────────────────────────
        if let Err(e) = rhi.present() {
            warn!("present failed: {e}");
        }
    }
}

impl Default for Renderer {
    fn default() -> Self { Self::new() }
}
