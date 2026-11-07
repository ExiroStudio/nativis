use anyhow::{Context, Result};
use parking_lot::Mutex;
use tracing::{debug, warn};

use nativis_core::contract::{Frame, FrameSink, MediaError};
use nativis_core::resource::{CpuBuffer, PlanarBuffer, ResourceManager};

use crate::render::WaylandWgpuRenderer;
use crate::shell::WaylandShell;

pub struct WaylandDirectSink {
    renderer: Option<Mutex<WaylandWgpuRenderer>>,
    shell: Option<Mutex<WaylandShell>>,
    resources: ResourceManager,
}

impl WaylandDirectSink {
    pub fn new(resources: ResourceManager) -> Result<Self> {
        let shell = WaylandShell::new().context("Failed to initialize Wayland Shell")?;
        let handle = shell.surface_handle();
        let width = shell.state.width;
        let height = shell.state.height;

        let renderer = WaylandWgpuRenderer::new(handle, width, height)
            .context("Failed to initialize WGPU renderer on Wayland surface")?;

        Ok(Self {
            renderer: Some(Mutex::new(renderer)),
            shell: Some(Mutex::new(shell)),
            resources,
        })
    }
}

impl Drop for WaylandDirectSink {
    fn drop(&mut self) {
        // Drop WGPU renderer (which destroys Vulkan surface) BEFORE closing Wayland shell/surface
        drop(self.renderer.take());
        drop(self.shell.take());
    }
}

impl FrameSink for WaylandDirectSink {
    fn submit(&mut self, frame: Frame) -> Result<(), MediaError> {
        let renderer_lock = match &self.renderer {
            Some(r) => r,
            None => return Err(MediaError::GpuUpload("Renderer already shut down".into())),
        };
        let shell_lock = match &self.shell {
            Some(s) => s,
            None => return Err(MediaError::GpuUpload("Shell already shut down".into())),
        };

        let mut shell = shell_lock.lock();
        let mut renderer = renderer_lock.lock();

        // 1. Process Wayland events (resizes, outputs, etc.)
        if let Err(e) = shell.dispatch_pending() {
            warn!("Wayland event dispatch warning: {}", e);
        }

        // 2. Adjust WGPU surface configuration if screen size changed
        renderer.resize(shell.state.width, shell.state.height);

        // 3. Acquire frame pixel data and render
        let success = self.resources.acquire(frame.resource, |res| {
            // NV12 Multi-plane Video path
            if let Some(planar) = res.as_any().downcast_ref::<PlanarBuffer>() {
                if planar.planes.len() < 2 {
                    return false;
                }

                let y_plane = &planar.planes[0];
                let uv_plane = &planar.planes[1];

                let y_w = planar.width;
                let y_h = planar.height;
                let uv_w = (planar.width + 1) / 2;
                let uv_h = (planar.height + 1) / 2;

                if let Err(e) = renderer.draw_nv12(
                    &y_plane.data,
                    y_plane.stride,
                    y_w,
                    y_h,
                    &uv_plane.data,
                    uv_plane.stride,
                    uv_w,
                    uv_h,
                ) {
                    warn!("WGPU draw_nv12 error: {}", e);
                    return false;
                }
                true
            }
            // RGBA Static Image / Fallback path
            else if let Some(cpu) = res.as_any().downcast_ref::<CpuBuffer>() {
                if let Err(e) = renderer.draw_rgba(&cpu.data, cpu.width, cpu.height) {
                    warn!("WGPU draw_rgba error: {}", e);
                    return false;
                }
                true
            } else {
                false
            }
        }).unwrap_or(false);

        if !success {
            return Err(MediaError::GpuUpload(
                "Failed to render frame or unsupported buffer type".into(),
            ));
        }

        debug!("Presented frame {}x{} on Wayland Layer::Bottom", frame.width, frame.height);
        Ok(())
    }
}
