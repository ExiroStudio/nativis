#[cfg(windows)]
use tracing::{info, warn};

#[cfg(windows)]
use windows::Win32::{
    Devices::Display::{
        DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
        DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
        DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_TARGET_DEVICE_NAME,
        QDC_ONLY_ACTIVE_PATHS,
    },
    Foundation::{BOOL, LPARAM, RECT},
    Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO},
    UI::{
        HiDpi::{
            GetDpiForMonitor, SetProcessDpiAwarenessContext,
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI,
        },
        WindowsAndMessaging::{
            GetSystemMetrics, MONITORINFOF_PRIMARY, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
            SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
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
    pub device_id: String,
    pub friendly_name: String,
    pub refresh_rate_mhz: u32,
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
                    device_id: String::new(),
                    friendly_name: String::new(),
                    refresh_rate_mhz: 60000,
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

        let mut monitors: Vec<MonitorDescriptor> = Vec::new();
        unsafe {
            let _ = EnumDisplayMonitors(
                HDC::default(),
                None,
                Some(enum_proc),
                LPARAM(&mut monitors as *mut _ as isize),
            );
        }

        let paths = query_display_paths();
        for (i, m) in monitors.iter_mut().enumerate() {
            if let Some(p) = paths.get(i) {
                if !p.device_path.is_empty() {
                    m.device_id = p.device_path.clone();
                } else {
                    m.device_id = format!("win-monitor-{}", i);
                }
                if !p.friendly_name.is_empty() {
                    m.friendly_name = p.friendly_name.clone();
                } else {
                    m.friendly_name = format!("Display {}", i + 1);
                }
                m.refresh_rate_mhz = p.refresh_rate_mhz;
            } else {
                m.device_id = format!("win-monitor-{}", i);
                m.friendly_name = format!("Display {}", i + 1);
                m.refresh_rate_mhz = 60000;
            }
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

/// Query active display paths via Windows QueryDisplayConfig API.
/// Retrieves accurate refresh rates (mHz) and device paths (EDID/target device path).
#[cfg(windows)]
#[derive(Debug, Clone, Default)]
pub struct DisplayPathDetails {
    pub device_path: String,
    pub friendly_name: String,
    pub refresh_rate_mhz: u32,
}

#[cfg(windows)]
pub fn query_display_paths() -> Vec<DisplayPathDetails> {
    unsafe {
        let mut path_count = 0u32;
        let mut mode_count = 0u32;
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
            .is_err()
        {
            return Vec::new();
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
        .is_err()
        {
            return Vec::new();
        }
        paths.truncate(path_count as usize);

        let mut results = Vec::new();
        for path in paths {
            let mut target_name = DISPLAYCONFIG_TARGET_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                    size: std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };

            let (friendly_name, device_path) =
                if DisplayConfigGetDeviceInfo(&mut target_name.header) == 0 {
                    let f_name = String::from_utf16_lossy(&target_name.monitorFriendlyDeviceName)
                        .trim_matches('\0')
                        .to_string();
                    let d_path = String::from_utf16_lossy(&target_name.monitorDevicePath)
                        .trim_matches('\0')
                        .to_string();
                    (f_name, d_path)
                } else {
                    (String::new(), String::new())
                };

            let refresh_rate_mhz = if path.targetInfo.refreshRate.Denominator > 0 {
                ((path.targetInfo.refreshRate.Numerator as u64 * 1000)
                    / path.targetInfo.refreshRate.Denominator as u64) as u32
            } else {
                60000
            };

            results.push(DisplayPathDetails {
                device_path,
                friendly_name,
                refresh_rate_mhz,
            });
        }
        results
    }
}
