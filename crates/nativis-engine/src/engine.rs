use nativis_audio::NullAudioCapture;
use nativis_ecs::World;
use nativis_event::EventBus;
use nativis_media::{IMediaSource, MediaClock};
use nativis_platform::IWallpaperBackend;
use nativis_plugin::PermissiveSecurityManager;
use nativis_render_graph::RenderGraph;
use nativis_rhi::{IRhiBackend, WgpuBackend};
use nativis_scene::SceneGraph;
use nativis_timeline::Timeline;
use std::sync::Arc;
use tracing::{error, info, warn};
use winit::{
    application::ApplicationHandler,
    event::{WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId, WindowAttributes},
};

/// Engine configuration set by the caller before `Engine::run()`.
pub struct EngineConfig {
    pub wallpaper_path: String,
    pub output_index:   usize,
    pub target_fps:     u32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            wallpaper_path: String::new(),
            output_index:   0,
            target_fps:     60,
        }
    }
}

/// The top-level engine struct. Owns all subsystems and drives the 6-phase
/// frame scheduler. Created once per process lifetime.
pub struct Engine {
    pub config:      EngineConfig,
    pub event_bus:   EventBus,

    // Subsystems (Option so they can be initialised after window creation)
    pub rhi:         Option<Box<WgpuBackend>>,
    pub world:       World,
    pub scene:       SceneGraph,
    pub render_graph: RenderGraph,
    pub media_clock: MediaClock,
    pub timeline:    Timeline,
    pub audio:       NullAudioCapture,
    pub security:    PermissiveSecurityManager,

    pub media_sources: Vec<Box<dyn IMediaSource>>,
    pub platform:      Option<Box<dyn IWallpaperBackend>>,

    // Window
    pub window:      Option<Arc<Window>>,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            config,
            event_bus: EventBus::new(),
            rhi:       None,
            world:     World::new(),
            scene:     SceneGraph::new(),
            render_graph: RenderGraph::new(),
            media_clock: MediaClock::new(),
            timeline:  Timeline::new(),
            audio:     NullAudioCapture::new(),
            security:  PermissiveSecurityManager,
            media_sources: Vec::new(),
            platform:  None,
            window:    None,
        }
    }

    /// Run the engine on the calling thread. This blocks until the window closes.
    pub fn run(self) -> anyhow::Result<()> {
        let event_loop = EventLoop::new()?;
        event_loop.set_control_flow(ControlFlow::Poll);

        let mut app = NativisApp { engine: self };
        event_loop.run_app(&mut app)?;
        Ok(())
    }
}

// ── winit ApplicationHandler ──────────────────────────────────────────────────

struct NativisApp {
    engine: Engine,
}

impl ApplicationHandler for NativisApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.engine.window.is_some() { return; }

        // Create the window
        let attrs = WindowAttributes::default()
            .with_title("Nativis Engine")
            .with_inner_size(winit::dpi::LogicalSize::new(1280u32, 720u32))
            .with_decorations(true);

        let window = match event_loop.create_window(attrs) {
            Ok(w)  => Arc::new(w),
            Err(e) => { error!("Window creation failed: {}", e); event_loop.exit(); return; }
        };

        // Initialise RHI
        let size = window.inner_size();
        let rhi = match WgpuBackend::new(window.as_ref(), size.width, size.height) {
            Ok(r)  => r,
            Err(e) => { error!("RHI init failed: {}", e); event_loop.exit(); return; }
        };

        self.engine.rhi    = Some(Box::new(rhi));
        self.engine.window = Some(window);

        // Initialise platform backend
        match nativis_platform::create_platform_backend() {
            Ok(mut backend) => {
                backend.set_output(self.engine.config.output_index);
                let _ = backend.attach();
                self.engine.platform = Some(backend);
            }
            Err(e) => warn!("Platform backend unavailable: {}", e),
        }

        // Initialise media sources
        let path = self.engine.config.wallpaper_path.clone();
        if !path.is_empty() {
            let mut source: Box<dyn IMediaSource> =
                Box::new(nativis_media::ImageSource::new(&path));

            if let Some(rhi) = &mut self.engine.rhi {
                match source.initialize(rhi.as_mut()) {
                    Ok(_)  => info!("Media source '{}' initialised", source.name()),
                    Err(e) => error!("Media source init failed: {}", e),
                }
            }
            self.engine.media_sources.push(source);
        }

        info!("Engine initialised. Starting render loop.");
        self.engine.event_bus.publish(nativis_event::EngineEvent::EngineStarted);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _wid: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.engine.event_bus.publish(nativis_event::EngineEvent::EngineShutdown);
                event_loop.exit();
            }

            WindowEvent::Resized(size) => {
                self.engine.event_bus.publish(nativis_event::EngineEvent::WindowResized {
                    width: size.width, height: size.height,
                });
                if let Some(rhi) = &mut self.engine.rhi {
                    let _ = rhi.resize(size.width, size.height);
                }
            }

            WindowEvent::RedrawRequested => {
                crate::scheduler::run_frame(&mut self.engine);
                if let Some(w) = &self.engine.window {
                    w.request_redraw();
                }
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(w) = &self.engine.window {
            w.request_redraw();
        }
    }
}
