use anyhow::{Context, Result};
use nativis_core::contract::FrameSink;
use nativis_core::platform::Platform;
use nativis_core::resource::ResourceManager;
use tracing::info;

pub mod workerw;
pub mod sink;

pub use workerw::WorkerWContext;
pub use sink::WindowsDirectSink;

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
        let hwnd = self.workerw.as_ref().map(|w| w.wallpaper_hwnd).unwrap_or(0);
        let sink = WindowsDirectSink::new(hwnd, resources.clone());
        Ok(Box::new(sink))
    }
}
