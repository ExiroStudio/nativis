use crate::engine::Engine;
use nativis_rhi::IRhiBackend;
use nativis_scene::BlitPass;
use tracing::{debug, warn};

/// Execute one full engine frame. Called from the winit `RedrawRequested` handler.
///
/// Phase sequence (from the architectural plan):
///  1. Event Polling   — already handled by winit before RedrawRequested
///  2. Media Update    — advance clock, poll decoders, acquire frames
///  3. Timeline Step   — evaluate keyframe tracks, apply property bindings
///  4. ECS Systems     — particle updates, transform propagation
///  5. Render Graph    — compile + execute all render passes
///  6. Present         — swap the swapchain buffer to the display
pub fn run_frame(engine: &mut Engine) {
    let rhi = match &mut engine.rhi {
        Some(r) => r,
        None    => return,
    };

    // ── Phase 1: events already dispatched by winit ────────────────────────

    // ── Phase 2: Media update ─────────────────────────────────────────────
    engine.media_clock.tick();
    let clock_ns = engine.media_clock.now_ns();

    let mut frame_texture = None;

    for source in &mut engine.media_sources {
        source.update(clock_ns);
        if let Some(frame) = source.acquire_frame() {
            frame_texture = Some(frame.texture);
            source.release_frame(frame);
        }
    }

    // ── Phase 3: Timeline & animation evaluation ──────────────────────────
    // (no tracks registered in Phase 1 demo — stub call)
    // engine.timeline.step(delta_sec, &mut engine.world);

    // ── Phase 4: ECS systems ──────────────────────────────────────────────
    // (no systems registered in Phase 1 — stub)

    // ── Phase 5: Render Graph ─────────────────────────────────────────────
    if let Err(e) = rhi.begin_frame() {
        warn!("begin_frame failed: {}", e);
        return;
    }

    engine.render_graph.reset();

    // If a media frame is available, add a BlitPass to draw it to screen.
    if let Some(tex) = frame_texture {
        engine.render_graph.add_pass(BlitPass::new(tex));
    } else {
        // No media yet — add a clear-only pass (add black BlitPass to a
        // 1×1 dummy texture would require allocating; we just skip).
        debug!("No media frame available this frame — presenting empty swapchain");
    }

    let resources = engine.render_graph.compile(rhi.as_mut());
    engine.render_graph.execute(rhi.as_mut(), &resources);

    // ── Phase 6: Present ──────────────────────────────────────────────────
    if let Err(e) = rhi.present() {
        warn!("present failed: {}", e);
    }
}
