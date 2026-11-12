use std::sync::Arc;
use anyhow::{Context, Result};
use nativis_core::contract::FrameSink;
use nativis_core::platform::Platform;
use nativis_core::resource::ResourceManager;
use tracing::info;

pub mod workerw;
pub mod sink;
pub mod d3d11_renderer;
pub mod throttler;
pub mod monitor;

pub use workerw::WorkerWContext;
pub use sink::WindowsDirectSink;
pub use d3d11_renderer::D3D11Renderer;
pub use throttler::{Throttler, PauseReason};
pub use monitor::*;

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
        crate::monitor::enable_dpi_awareness_v2();
        let ctx = WorkerWContext::new().context("Failed to initialize WorkerW context")?;
        self.workerw = Some(ctx);
        info!("WindowsPlatform bootstrap complete.");
        Ok(())
    }

    fn create_sink(&self, resources: &ResourceManager) -> Result<Box<dyn FrameSink>> {
        let (hwnd, width, height, throttler) = self.workerw.as_ref()
            .map(|w| (w.wallpaper_hwnd, w.width, w.height, Arc::clone(&w.throttler)))
            .unwrap_or_else(|| (0, 1920, 1080, Arc::new(Throttler::new(0, 0, 0))));
        let sink = WindowsDirectSink::new(hwnd, width, height, throttler, resources.clone())
            .context("Failed to initialize WindowsDirectSink")?;
        Ok(Box::new(sink))
    }

    fn outputs(&self) -> Vec<nativis_core::platform::OutputInfo> {
        let monitors = crate::monitor::enumerate_monitors();
        if monitors.is_empty() {
            return vec![nativis_core::platform::OutputInfo {
                id: "win-virtual".to_string(),
                name: "Virtual Desktop".to_string(),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale_factor: 1.0,
                refresh_rate_mhz: 60000,
                is_primary: true,
            }];
        }

        monitors.into_iter().enumerate().map(|(idx, m)| {
            let scale = if m.dpi_x > 0 { m.dpi_x as f64 / 96.0 } else { 1.0 };
            nativis_core::platform::OutputInfo {
                id: format!("win-monitor-{}", idx),
                name: format!("Display {} ({:?})", idx + 1, m.hmonitor),
                x: m.rect.0,
                y: m.rect.1,
                width: m.width,
                height: m.height,
                scale_factor: scale,
                refresh_rate_mhz: 60000,
                is_primary: m.is_primary,
            }
        }).collect()
    }
}

