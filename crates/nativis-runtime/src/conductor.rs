//! `Runtime` — the Nativis runtime conductor.
//!
//! Drives the per-frame pipeline: tick media → draw → present.
//! Never calls backends directly by name; all routing goes through contracts.

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tracing::{error, info, warn};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowAttributes, WindowId},
};

use nativis_asset::AssetPath;
use nativis_core::{
    clock::MediaClock,
    contract::{FrameStatus, MediaBackend},
};
use nativis_plugin::PluginRegistry;
use nativis_render::Renderer;
use nativis_rhi::{IRhiBackend, WgpuBackend};

// ── Configuration ─────────────────────────────────────────────────────────────

/// Runtime configuration provided by the caller before `Runtime::run()`.
pub struct RuntimeConfig {
    /// URI of the wallpaper media source (e.g. `"file:///path/cat.png"`).
    pub media_uri: String,
    /// Zero-based monitor output index to display the wallpaper on.
    pub output_index: usize,
    /// Target frame rate for the render loop.
    pub target_fps: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            media_uri:    String::new(),
            output_index: 0,
            target_fps:   60,
        }
    }
}

// ── Runtime ───────────────────────────────────────────────────────────────────

/// The Nativis runtime conductor.
///
/// Owns all pipeline stages. Calls each stage in order each frame.
/// Does not contain media-specific logic.
pub struct Runtime {
    config:   RuntimeConfig,
    registry: PluginRegistry,
}

impl Runtime {
    pub fn new(config: RuntimeConfig, registry: PluginRegistry) -> Self {
        Self { config, registry }
    }

    /// Run the runtime on the calling thread. Blocks until shutdown.
    pub fn run(self) -> Result<()> {
        let event_loop = EventLoop::new()?;
        event_loop.set_control_flow(ControlFlow::Poll);

        let mut app = RuntimeApp::new(self.config, self.registry);
        event_loop.run_app(&mut app)?;
        Ok(())
    }
}

// ── Internal winit application ────────────────────────────────────────────────

struct RuntimeApp {
    config:   RuntimeConfig,
    registry: PluginRegistry,

    // Initialized after window creation
    window:   Option<Arc<Window>>,
    rhi:      Option<WgpuBackend>,
    renderer: Renderer,
    platform: Option<Box<dyn nativis_platform::IWallpaperSession>>,
    backend:  Option<Box<dyn MediaBackend>>,
    clock:    MediaClock,

    last_tick: Instant,
}

impl RuntimeApp {
    fn new(config: RuntimeConfig, registry: PluginRegistry) -> Self {
        Self {
            config,
            registry,
            window:   None,
            rhi:      None,
            renderer: Renderer::new(),
            platform: None,
            backend:  None,
            clock:    MediaClock::new(),
            last_tick: Instant::now(),
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick);
        self.last_tick = now;

        let rhi = match self.rhi.as_mut() {
            Some(r) => r,
            None => return,
        };

        // ── Step 1: Update media backend ─────────────────────────────────────
        let frame_status = if let Some(backend) = self.backend.as_mut() {
            if let Err(e) = backend.update(dt) {
                warn!("Media backend update error: {e}");
            }
            backend.current_frame()
        } else {
            FrameStatus::Unchanged
        };

        // ── Auto-Recovery ────────────────────────────────────────────────────
        if let Some(platform) = self.platform.as_mut() {
            if platform.health() == nativis_platform::BackendHealth::NeedsReattach {
                warn!("Wallpaper backend requires re-attachment, attempting recovery...");
                let _ = platform.detach();
                if let Some(w) = self.window.as_ref() {
                    match nativis_platform::attach_platform_backend(w.as_ref(), self.config.output_index) {
                        Ok(p) => self.platform = Some(p),
                        Err(e) => warn!("Auto-recovery failed: {e}"),
                    }
                }
            }
        }

        // ── Step 2: Draw (Composite + Post) ──────────────────────────────────
        self.renderer.draw(frame_status, rhi);

        // Step 3 (Present) is called inside renderer.draw() via rhi.present().
    }
}

impl ApplicationHandler for RuntimeApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }

        // Create a borderless window (the wallpaper surface).
        let attrs = WindowAttributes::default()
            .with_title("Nativis")
            .with_decorations(false)
            .with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));

        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => { error!("Window creation failed: {e}"); event_loop.exit(); return; }
        };

        let size = window.inner_size();

        // Initialize RHI
        let rhi = match WgpuBackend::new(window.as_ref(), size.width, size.height) {
            Ok(r) => r,
            Err(e) => { error!("RHI init failed: {e}"); event_loop.exit(); return; }
        };

        // Initialize renderer (compiles blit pipeline)
        self.renderer.init(&rhi);

        // Initialize wallpaper platform backend
        match nativis_platform::attach_platform_backend(window.as_ref(), self.config.output_index) {
            Ok(p) => self.platform = Some(p),
            Err(e) => warn!("Platform backend unavailable: {e}"),
        }

        // Open media via registry — runtime never names a backend type.
        if !self.config.media_uri.is_empty() {
            match AssetPath::parse(&self.config.media_uri) {
                Ok(path) => {
                    match self.registry.create_backend(&path) {
                        Some(mut media) => {
                            let ctx = rhi.rhi_context();
                            match media.open(&path, &ctx, &self.clock) {
                                Ok(()) => {
                                    info!("Media backend '{}' opened '{}'",
                                          media.name(), path.raw_uri());
                                    self.backend = Some(media);
                                }
                                Err(e) => error!("Media open failed: {e}"),
                            }
                        }
                        None => warn!("No backend for '{}'", self.config.media_uri),
                    }
                }
                Err(e) => error!("Invalid media URI: {e}"),
            }
        }

        self.rhi    = Some(rhi);
        self.window = Some(window);

        info!("Runtime started.");
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _wid: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                if let Some(backend) = self.backend.as_mut() {
                    backend.close();
                }
                event_loop.exit();
            }

            WindowEvent::Resized(size) => {
                if let Some(rhi) = self.rhi.as_mut() {
                    let _ = rhi.resize(size.width, size.height);
                }
            }

            WindowEvent::RedrawRequested => {
                self.tick();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}
