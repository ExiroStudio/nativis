use anyhow::{Context, Result};
use nativis_core::contract::FrameSink;
use nativis_core::platform::Platform;
use nativis_core::resource::ResourceManager;
use tracing::info;

pub mod workerw;
pub mod sink;
pub mod d3d11_renderer;

pub use workerw::WorkerWContext;
pub use sink::WindowsDirectSink;
pub use d3d11_renderer::D3D11Renderer;

pub struct WindowsPlatform {
    workerw: Option<WorkerWContext>,
}

impl WindowsPlatform {
    pub fn new() -> Self {
        Self { workerw: None }
    }
}

impl Default for WindowsPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for WindowsPlatform {
    fn bootstrap(&mut self, _force: bool) -> Result<()> {
        info!("Bootstrapping WindowsPlatform (WorkerW Desktop Injection)...");
        let ctx = WorkerWContext::new().context("Failed to initialize WorkerW context")?;
        self.workerw = Some(ctx);
        info!("WindowsPlatform bootstrap complete.");
        Ok(())
    }

    fn create_sink(&self, resources: &ResourceManager) -> Result<Box<dyn FrameSink>> {
        let (hwnd, width, height) = self.workerw.as_ref()
            .map(|w| (w.wallpaper_hwnd, w.width, w.height))
            .unwrap_or((0, 1920, 1080));
        let sink = WindowsDirectSink::new(hwnd, width, height, resources.clone())
            .context("Failed to initialize WindowsDirectSink")?;
        Ok(Box::new(sink))
    }
}
