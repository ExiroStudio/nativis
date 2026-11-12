use std::sync::Arc;
use anyhow::Result;
use nativis_core::contract::{Frame, FrameSink, MediaError};
use nativis_core::resource::ResourceManager;
use crate::throttler::Throttler;

#[cfg(windows)]
use anyhow::Context;
#[cfg(windows)]
use nativis_core::resource::{CpuBuffer, PlanarBuffer, Resource};
#[cfg(windows)]
use parking_lot::Mutex;
#[cfg(windows)]
use tracing::warn;

#[cfg(windows)]
use crate::d3d11_renderer::D3D11Renderer;

/// Internal GPU-backed Direct3D 11 texture resource for Windows platform.
/// Kept isolated to Windows crate (D1.3, D4) rather than leaking into nativis-core.
#[cfg(windows)]
pub struct D3D11TextureResource {
    pub texture_ptr: usize,
    pub subresource_index: u32,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub keeper: Option<Arc<dyn std::any::Any + Send + Sync>>,
}

#[cfg(windows)]
impl Resource for D3D11TextureResource {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}


pub struct WindowsDirectSink {
    pub wallpaper_hwnd: isize,
    #[cfg(windows)]
    renderer: Option<Mutex<D3D11Renderer>>,
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
        #[cfg(windows)]
        let renderer = {
            if wallpaper_hwnd != 0 {
                let hwnd = windows::Win32::Foundation::HWND(wallpaper_hwnd as *mut _);
                let r = D3D11Renderer::new(hwnd, _width, _height)
                    .context("Failed to initialize Direct3D 11 renderer")?;
                Some(Mutex::new(r))
            } else {
                None
            }
        };

        Ok(Self {
            wallpaper_hwnd,
            #[cfg(windows)]
            renderer,
            #[cfg(windows)]
            throttler: _throttler,
            resources,
        })
    }
}

impl Drop for WindowsDirectSink {
    fn drop(&mut self) {
        #[cfg(windows)]
        drop(self.renderer.take());
    }
}

impl FrameSink for WindowsDirectSink {
    fn submit(&mut self, frame: Frame) -> Result<(), MediaError> {
        #[cfg(windows)]
        {
            if self.throttler.is_paused() {
                // Throttled: wait in 50ms intervals until resumed so the frame is not dropped (essential for static images)
                while self.throttler.is_paused() {
                    if self.wallpaper_hwnd != 0 {
                        let is_alive = unsafe {
                            windows::Win32::UI::WindowsAndMessaging::IsWindow(
                                windows::Win32::Foundation::HWND(self.wallpaper_hwnd as *mut _),
                            ).as_bool()
                        };
                        if !is_alive {
                            return Ok(());
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }

            let renderer_lock = match &self.renderer {
                Some(r) => r,
                None => return Err(MediaError::GpuUpload("D3D11 Renderer not initialized".into())),
            };

            let mut renderer = renderer_lock.lock();

            // 1. Adjust swapchain buffers if window dimensions changed
            if self.wallpaper_hwnd != 0 {
                let mut rc: windows::Win32::Foundation::RECT = unsafe { std::mem::zeroed() };
                let ok = unsafe {
                    windows::Win32::UI::WindowsAndMessaging::GetClientRect(
                        windows::Win32::Foundation::HWND(self.wallpaper_hwnd as *mut _),
                        &mut rc,
                    )
                };
                if ok.is_ok() {
                    let win_w = (rc.right - rc.left) as u32;
                    let win_h = (rc.bottom - rc.top) as u32;
                    if win_w > 0 && win_h > 0 && (win_w != renderer.width() || win_h != renderer.height()) {
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
                    let tex: &windows::Win32::Graphics::Direct3D11::ID3D11Texture2D = unsafe {
                        std::mem::transmute(&ptr)
                    };
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

            if success == Some(true) {
                if let Err(e) = renderer.present() {
                    warn!("D3D11 present error: {}", e);
                }
                Ok(())
            } else {
                Err(MediaError::GpuUpload("Failed to render frame to D3D11 swapchain".into()))
            }
        }
        #[cfg(not(windows))]
        {
            let _ = frame;
            let _ = &self.resources;
            Ok(())
        }
    }
}
