use anyhow::{Context, Result};
use nativis_core::contract::FrameSink;
use nativis_core::platform::Platform;
use nativis_core::resource::ResourceManager;
use std::sync::Arc;
use tracing::info;

pub mod d3d11_renderer;
pub mod monitor;
pub mod sink;
pub mod throttler;
pub mod workerw;

pub use d3d11_renderer::D3D11Renderer;
pub use monitor::*;
pub use sink::WindowsDirectSink;
pub use throttler::{PauseReason, Throttler};
pub use workerw::WorkerWContext;

pub struct WindowsPlatform {
    workerw: Option<WorkerWContext>,
    event_sender: Option<nativis_core::platform::PlatformEventSender>,
}

impl WindowsPlatform {
    pub fn new() -> Self {
        Self {
            workerw: None,
            event_sender: None,
        }
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
        if let Some(ref sender) = self.event_sender {
            ctx.throttler.set_event_sender(sender.clone());
        }
        self.workerw = Some(ctx);
        info!("WindowsPlatform bootstrap complete.");
        Ok(())
    }

    fn set_event_sender(&mut self, sender: nativis_core::platform::PlatformEventSender) {
        if let Some(ref w) = self.workerw {
            w.throttler.set_event_sender(sender.clone());
        }
        self.event_sender = Some(sender);
    }

    fn create_sink(&self, resources: &ResourceManager) -> Result<Box<dyn FrameSink>> {
        #[cfg(windows)]
        if let Some(ref w) = self.workerw {
            if let Some(ref r) = *w.renderer.lock() {
                let dev_ptr = windows::core::Interface::as_raw(r.device()) as usize;
                let ctx_ptr = windows::core::Interface::as_raw(r.context()) as usize;
                resources.set_gpu_device(nativis_core::resource::GpuDeviceResource {
                    device_ptr: dev_ptr,
                    device_context_ptr: ctx_ptr,
                    generation: 1,
                });
            }
            w.set_resources(resources.clone());

            let sink = WindowsDirectSink::with_shared(
                Arc::clone(&w.wallpaper_hwnd),
                Arc::clone(&w.renderer),
                Arc::clone(&w.throttler),
                resources.clone(),
            );
            return Ok(Box::new(sink));
        }

        let sink = WindowsDirectSink::new(
            0,
            1920,
            1080,
            Arc::new(Throttler::new(0, 0, 0)),
            resources.clone(),
        )
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

        monitors
            .into_iter()
            .enumerate()
            .map(|(idx, m)| {
                let scale = if m.dpi_x > 0 {
                    m.dpi_x as f64 / 96.0
                } else {
                    1.0
                };
                let id = if !m.device_id.is_empty() {
                    m.device_id
                } else {
                    format!("win-monitor-{}", idx)
                };
                let name = if !m.friendly_name.is_empty() {
                    m.friendly_name
                } else {
                    format!("Display {} ({:?})", idx + 1, m.hmonitor)
                };
                nativis_core::platform::OutputInfo {
                    id,
                    name,
                    x: m.rect.0,
                    y: m.rect.1,
                    width: m.width,
                    height: m.height,
                    scale_factor: scale,
                    refresh_rate_mhz: m.refresh_rate_mhz,
                    is_primary: m.is_primary,
                }
            })
            .collect()
    }
}
