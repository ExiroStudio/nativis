use crate::throttler::Throttler;
use anyhow::Result;
use nativis_core::contract::{Frame, FrameSink, MediaError};
use nativis_core::resource::ResourceManager;
use std::sync::Arc;

#[cfg(windows)]
use anyhow::Context;
#[cfg(windows)]
use nativis_core::resource::{CpuBuffer, PlanarBuffer};
#[cfg(windows)]
use parking_lot::Mutex;
#[cfg(windows)]
use tracing::warn;

#[cfg(windows)]
use crate::d3d11_renderer::D3D11Renderer;

/// GPU-backed Direct3D 11 texture resource for Windows platform.
/// Aliased to neutral GpuTextureResource in core (D1.3, D4).
pub type D3D11TextureResource = nativis_core::resource::GpuTextureResource;

pub struct WindowsDirectSink {
    pub wallpaper_hwnd: Arc<std::sync::atomic::AtomicIsize>,
    #[cfg(windows)]
    renderer: Arc<Mutex<Option<D3D11Renderer>>>,
    #[cfg(windows)]
    throttler: Arc<Throttler>,
    resources: ResourceManager,
}

impl WindowsDirectSink {
    pub fn new(
        wallpaper_hwnd: isize,
        _width: u32,
        _height: u32,
        _throttler: Arc<Throttler>,
        resources: ResourceManager,
    ) -> Result<Self> {
        let hwnd_atomic = Arc::new(std::sync::atomic::AtomicIsize::new(wallpaper_hwnd));
        #[cfg(windows)]
        let renderer = {
            if wallpaper_hwnd != 0 {
                let hwnd = windows::Win32::Foundation::HWND(wallpaper_hwnd as *mut _);
                let r = D3D11Renderer::new(hwnd, _width, _height)
                    .context("Failed to initialize Direct3D 11 renderer")?;
                Arc::new(Mutex::new(Some(r)))
            } else {
                Arc::new(Mutex::new(None))
            }
        };

        Ok(Self {
            wallpaper_hwnd: hwnd_atomic,
            #[cfg(windows)]
            renderer,
            #[cfg(windows)]
            throttler: _throttler,
            resources,
        })
    }

    pub fn with_shared(
        wallpaper_hwnd: Arc<std::sync::atomic::AtomicIsize>,
        #[cfg(windows)] renderer: Arc<Mutex<Option<D3D11Renderer>>>,
        _throttler: Arc<Throttler>,
        resources: ResourceManager,
    ) -> Self {
        Self {
            wallpaper_hwnd,
            #[cfg(windows)]
            renderer,
            #[cfg(windows)]
            throttler: _throttler,
            resources,
        }
    }
}

impl Drop for WindowsDirectSink {
    fn drop(&mut self) {
        #[cfg(windows)]
        let _ = self.renderer.lock().take();
    }
}

impl FrameSink for WindowsDirectSink {
    fn submit(&mut self, frame: Frame) -> Result<(), MediaError> {
        #[cfg(windows)]
        {
            if self.throttler.is_paused() {
                // Batch 2 / C2 fix: do NOT block conductor thread in a sleep loop.
                // Return Ok(()) immediately so conductor stays responsive to IPC commands (SetWallpaper/Stop).
                // Conductor is notified via PlatformEvent::Pause and suspends at 0.00% CPU.
                // Upon resume, PlatformEvent::Resume / Redraw triggers re-presentation of last_frame.
                return Ok(());
            }

            let mut renderer_guard = self.renderer.lock();
            let renderer = match renderer_guard.as_mut() {
                Some(r) => r,
                None => {
                    return Err(MediaError::GpuUpload(
                        "D3D11 Renderer not initialized".into(),
                    ))
                }
            };

            // 1. Adjust swapchain buffers if window dimensions changed
            let current_hwnd = self
                .wallpaper_hwnd
                .load(std::sync::atomic::Ordering::Relaxed);
            if current_hwnd != 0 {
                let mut rc: windows::Win32::Foundation::RECT = unsafe { std::mem::zeroed() };
                let ok = unsafe {
                    windows::Win32::UI::WindowsAndMessaging::GetClientRect(
                        windows::Win32::Foundation::HWND(current_hwnd as *mut _),
                        &mut rc,
                    )
                };
                if ok.is_ok() {
                    let win_w = (rc.right - rc.left) as u32;
                    let win_h = (rc.bottom - rc.top) as u32;
                    if win_w > 0
                        && win_h > 0
                        && (win_w != renderer.width() || win_h != renderer.height())
                    {
                        let _ = renderer.resize(win_w, win_h);
                    }
                }
            }

            // 2. Acquire frame resource and render
            let success = self.resources.acquire(frame.resource, |res| {
                // Zero-Copy Direct3D 11 VRAM Texture Path
                if let Some(gpu) = res.as_any().downcast_ref::<D3D11TextureResource>() {
                    let ptr = gpu.texture_ptr as *mut std::ffi::c_void;
                    if ptr.is_null() {
                        return false;
                    }
                    let tex: &windows::Win32::Graphics::Direct3D11::ID3D11Texture2D =
                        unsafe { std::mem::transmute(&ptr) };
                    if let Err(e) = renderer.draw_d3d11_texture(
                        tex,
                        gpu.subresource_index,
                        gpu.width,
                        gpu.height,
                    ) {
                        warn!("D3D11 draw_d3d11_texture error: {}", e);
                        return false;
                    }
                    true
                }
                // NV12 Multi-plane Video path
                else if let Some(planar) = res.as_any().downcast_ref::<PlanarBuffer>() {
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
                        warn!("D3D11 draw_nv12 error: {}", e);
                        return false;
                    }
                    true
                }
                // RGBA Static Image / Fallback path
                else if let Some(cpu) = res.as_any().downcast_ref::<CpuBuffer>() {
                    if let Err(e) = renderer.draw_rgba(&cpu.data, cpu.width, cpu.height) {
                        warn!("D3D11 draw_rgba error: {}", e);
                        return false;
                    }
                    true
                } else {
                    warn!("Unknown frame resource type in WindowsDirectSink");
                    false
                }
            });

            match success {
                Some(true) => {
                    if let Err(e) = renderer.present() {
                        warn!("D3D11 present error: {}", e);
                    }
                    Ok(())
                }
                // Resource known but drawing failed: keep reporting the real error.
                Some(false) => Err(MediaError::GpuUpload(
                    "Failed to render frame to D3D11 swapchain".into(),
                )),
                // Resource already freed (e.g. image CpuBuffer released after the first frame):
                // redraw from the renderer's persistent texture instead of failing silently.
                None => renderer.re_present_last_frame().map_err(|e| {
                    MediaError::GpuUpload(format!(
                        "Re-present from persistent texture failed: {}",
                        e
                    ))
                }),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = frame;
            let _ = &self.resources;
            Ok(())
        }
    }

    fn capabilities(&self) -> nativis_core::contract::SinkCapabilities {
        nativis_core::contract::SinkCapabilities {
            accepted_kinds: vec![
                nativis_protocol::NATIVIS_SURFACE_KIND_D3D11_SHARED,
                nativis_protocol::NATIVIS_SURFACE_KIND_CPU_PLANAR,
                nativis_protocol::NATIVIS_SURFACE_KIND_CPU_BUFFER,
            ],
            accepted_formats: vec![
                nativis_protocol::NATIVIS_FORMAT_NV12,
                nativis_protocol::NATIVIS_FORMAT_RGBA8888,
            ],
        }
    }
}
