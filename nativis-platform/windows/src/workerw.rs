use anyhow::{Result, bail};
#[cfg(windows)]
use anyhow::Context;
#[cfg(windows)]
use tracing::{debug, info, warn};

#[cfg(windows)]
use std::ffi::OsStr;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use windows_sys::Win32::Foundation::*;
#[cfg(windows)]
use windows_sys::Win32::Graphics::Gdi::*;
#[cfg(windows)]
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[cfg(windows)]
fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

pub struct WorkerWContext {
    pub progman_hwnd: isize,
    pub workerw_hwnd: isize,
    pub wallpaper_hwnd: isize,
    pub width: u32,
    pub height: u32,
    #[cfg(windows)]
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

#[cfg(windows)]
struct EnumState {
    workerw_hwnd: HWND,
}

#[cfg(windows)]
unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let state = &mut *(lparam as *mut EnumState);
    let shelldll = FindWindowExW(
        hwnd,
        std::ptr::null_mut(),
        to_wide("SHELLDLL_DefView").as_ptr(),
        std::ptr::null(),
    );
    if !shelldll.is_null() {
        // Found WorkerW that contains the desktop icons!
        // The next WorkerW directly behind it in z-order is the target desktop background.
        let next_workerw = FindWindowExW(
            std::ptr::null_mut(),
            hwnd,
            to_wide("WorkerW").as_ptr(),
            std::ptr::null(),
        );
        if !next_workerw.is_null() {
            state.workerw_hwnd = next_workerw;
            return 0; // Stop enumeration
        }
    }
    1 // Continue enumeration
}

#[cfg(windows)]
unsafe extern "system" fn wallpaper_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1, // Suppress default background erase (avoids flickering)
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            // Solid dark brush fill for initial visual verification behind desktop icons
            let brush = CreateSolidBrush(0x00151515);
            FillRect(hdc, &ps.rcPaint, brush);
            DeleteObject(brush as _);
            EndPaint(hwnd, &ps);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

impl WorkerWContext {
    #[cfg(windows)]
    pub fn new() -> Result<Self> {
        info!("Initializing Windows WorkerW desktop injection...");

        unsafe {
            // 1. Locate Progman window
            let progman = FindWindowW(to_wide("Progman").as_ptr(), std::ptr::null());
            if progman.is_null() {
                bail!("Progman window not found. Windows Explorer shell is not running.");
            }
            debug!("Found Progman window handle: 0x{:X}", progman as usize);

            // 2. Trigger Explorer to spawn WorkerW behind SHELLDLL_DefView
            let mut result: usize = 0;
            let send_res = SendMessageTimeoutW(
                progman,
                0x052C, // WM_SPAWN_WORKER
                0x0000000D,
                0,
                SMTO_NORMAL,
                1000,
                &mut result,
            );
            if send_res == 0 {
                warn!("SendMessageTimeoutW(0x052C) returned 0; continuing with discovery");
            }

            // 3. Dual-topology WorkerW discovery
            let mut target_workerw: HWND = std::ptr::null_mut();

            // Topology A: Windows 11 24H2+ (WorkerW as direct child of Progman)
            let child_workerw = FindWindowExW(progman, std::ptr::null_mut(), to_wide("WorkerW").as_ptr(), std::ptr::null());
            if !child_workerw.is_null() {
                info!("Found WorkerW as direct child of Progman (Windows 11 24H2+ topology: 0x{:X})", child_workerw as usize);
                target_workerw = child_workerw;
            } else {
                // Topology B: Classic Windows 10 & Windows 11 < 24H2 (WorkerW as sibling via EnumWindows)
                let mut state = EnumState { workerw_hwnd: std::ptr::null_mut() };
                EnumWindows(Some(enum_windows_proc), &mut state as *mut _ as LPARAM);
                if !state.workerw_hwnd.is_null() {
                    info!("Found WorkerW as sibling behind SHELLDLL_DefView (Classic topology: 0x{:X})", state.workerw_hwnd as usize);
                    target_workerw = state.workerw_hwnd;
                }
            }

            if target_workerw.is_null() {
                warn!("Specific WorkerW not found via dual-topology discovery; falling back to Progman (0x{:X})", progman as usize);
                target_workerw = progman;
            }

            // 4. Create and attach dedicated wallpaper window on a background window thread
            let (tx, rx) = std::sync::mpsc::channel::<Result<usize>>();
            let parent_for_thread = target_workerw as usize;

            let thread_handle = std::thread::spawn(move || {
                let hinstance = GetModuleHandleW(std::ptr::null());
                let class_name = to_wide("NativisWallpaperWindowClass");

                let wnd_class = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(wallpaper_wndproc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: hinstance,
                    hIcon: std::ptr::null_mut(),
                    hCursor: std::ptr::null_mut(),
                    hbrBackground: std::ptr::null_mut(),
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: class_name.as_ptr(),
                    hIconSm: std::ptr::null_mut(),
                };
                RegisterClassExW(&wnd_class);

                let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
                let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
                let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
                let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);

                let hwnd = CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    class_name.as_ptr(),
                    to_wide("Nativis Wallpaper").as_ptr(),
                    WS_POPUP | WS_VISIBLE,
                    x, y, width, height,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    hinstance,
                    std::ptr::null(),
                );

                if hwnd.is_null() {
                    let _ = tx.send(Err(anyhow::anyhow!("Failed to create wallpaper window, error: {}", GetLastError())));
                    return;
                }

                // Attach to WorkerW
                SetParent(hwnd, parent_for_thread as HWND);

                // Convert style to WS_CHILD and remove WS_POPUP
                let current_style = GetWindowLongPtrW(hwnd, GWL_STYLE);
                let new_style = (current_style & !(WS_POPUP as isize)) | (WS_CHILD as isize) | (WS_VISIBLE as isize);
                SetWindowLongPtrW(hwnd, GWL_STYLE, new_style);

                // Position window covering entire virtual screen bounds
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0, 0, width, height,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
                );

                let _ = tx.send(Ok(hwnd as usize));

                // Standard message pump for the wallpaper window
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            });

            let screen_cx = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let screen_cy = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            let width = if screen_cx > 0 { screen_cx as u32 } else { 1920 };
            let height = if screen_cy > 0 { screen_cy as u32 } else { 1080 };

            let wallpaper_hwnd = rx.recv()
                .context("Failed to receive window handle from thread")??;

            info!("WorkerW desktop injection successful. Wallpaper HWND: 0x{:X} ({}x{})", wallpaper_hwnd, width, height);

            Ok(Self {
                progman_hwnd: progman as isize,
                workerw_hwnd: target_workerw as isize,
                wallpaper_hwnd: wallpaper_hwnd as isize,
                width,
                height,
                thread_handle: Some(thread_handle),
            })
        }
    }

    #[cfg(not(windows))]
    pub fn new() -> Result<Self> {
        bail!("Windows WorkerW is only supported on Windows targets");
    }
}

impl Drop for WorkerWContext {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            if self.wallpaper_hwnd != 0 {
                info!("Closing Nativis Wallpaper window (HWND: 0x{:X})...", self.wallpaper_hwnd as usize);
                PostMessageW(self.wallpaper_hwnd as HWND, WM_CLOSE, 0, 0);
            }
        }
        #[cfg(windows)]
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}
