#[cfg(windows)]
use tracing::{info, warn};

#[cfg(windows)]
use windows::{
    Win32::{
        Foundation::{BOOL, LPARAM, RECT},
        Graphics::Gdi::{
            EnumDisplayMonitors, GetMonitorInfoW, MONITORINFO, HDC, HMONITOR,
        },
        UI::{
            HiDpi::{
                GetDpiForMonitor, SetProcessDpiAwarenessContext,
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI,
            },
            WindowsAndMessaging::{
                GetSystemMetrics, MONITORINFOF_PRIMARY,
                SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
                SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
            },
        },
    },
};

/// High-level descriptor of an attached display monitor.
#[derive(Debug, Clone)]
pub struct MonitorDescriptor {
    pub hmonitor: isize,
    pub rect: (i32, i32, i32, i32), // left, top, right, bottom
    pub width: u32,
    pub height: u32,
    pub is_primary: bool,
    pub dpi_x: u32,
    pub dpi_y: u32,
}

/// Enable Per-Monitor v2 DPI awareness for pixel-perfect multi-monitor rendering.
/// Eliminates DWM bitmap blurring and ensures virtual screen coordinates match physical pixels.
pub fn enable_dpi_awareness_v2() {
    #[cfg(windows)]
    unsafe {
        match SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) {
            Ok(()) => info!("Per-Monitor Aware V2 DPI awareness enabled successfully"),
            Err(e) => warn!("SetProcessDpiAwarenessContext(V2) returned error: {:?}", e),
        }
    }
}

/// Enumerate all active display monitors, their pixel bounds, and their per-monitor DPI scaling.
pub fn enumerate_monitors() -> Vec<MonitorDescriptor> {
    #[cfg(windows)]
    {
        unsafe extern "system" fn enum_proc(
            hmon: HMONITOR,
            _hdc: HDC,
            _rect: *mut RECT,
            lparam: LPARAM,
        ) -> BOOL {
            let list = &mut *(lparam.0 as *mut Vec<MonitorDescriptor>);

            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };

            if GetMonitorInfoW(hmon, &mut mi).as_bool() {
                let left = mi.rcMonitor.left;
                let top = mi.rcMonitor.top;
                let right = mi.rcMonitor.right;
                let bottom = mi.rcMonitor.bottom;
                let width = (right - left).unsigned_abs();
                let height = (bottom - top).unsigned_abs();
                let is_primary = (mi.dwFlags & MONITORINFOF_PRIMARY) != 0;

                let mut dpi_x = 96u32;
                let mut dpi_y = 96u32;
                let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);

                let desc = MonitorDescriptor {
                    hmonitor: hmon.0 as isize,
                    rect: (left, top, right, bottom),
                    width,
                    height,
                    is_primary,
                    dpi_x,
                    dpi_y,
                };

                info!(
                    "Detected Monitor {}: {}x{} @ ({}, {}) [Primary: {}, DPI: {}x{} ({:.0}% scaling)]",
                    list.len(),
                    width,
                    height,
                    left,
                    top,
                    is_primary,
                    dpi_x,
                    dpi_y,
                    (dpi_x as f64 / 96.0) * 100.0,
                );

                list.push(desc);
            }

            BOOL(1) // Continue enumeration
        }

        let mut monitors = Vec::new();
        unsafe {
            let _ = EnumDisplayMonitors(
                HDC::default(),
                None,
                Some(enum_proc),
                LPARAM(&mut monitors as *mut _ as isize),
            );
        }
        monitors
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Retrieve bounding rectangle of entire virtual desktop across all connected monitors.
pub fn get_virtual_screen_bounds() -> (i32, i32, u32, u32) {
    #[cfg(windows)]
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let cx = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let cy = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        (
            x,
            y,
            if cx > 0 { cx as u32 } else { 1920 },
            if cy > 0 { cy as u32 } else { 1080 },
        )
    }
    #[cfg(not(windows))]
    {
        (0, 0, 1920, 1080)
    }
}
