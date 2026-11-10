use anyhow::Result;
use nativis_core::contract::{Frame, FrameSink, MediaError};
use nativis_core::resource::ResourceManager;

pub struct WindowsDirectSink {
    pub wallpaper_hwnd: isize,
    _resources: ResourceManager,
}

impl WindowsDirectSink {
    pub fn new(wallpaper_hwnd: isize, resources: ResourceManager) -> Self {
        Self {
            wallpaper_hwnd,
            _resources: resources,
        }
    }
}

impl FrameSink for WindowsDirectSink {
    fn submit(&mut self, _frame: Frame) -> Result<(), MediaError> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
            InvalidateRect(self.wallpaper_hwnd as _, std::ptr::null(), 0);
        }
        Ok(())
    }
}
