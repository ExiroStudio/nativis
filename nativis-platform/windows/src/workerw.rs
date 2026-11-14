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

use std::sync::Arc;
use crate::throttler::Throttler;

pub struct WorkerWContext {
    pub progman_hwnd: isize,
    pub workerw_hwnd: isize,
    pub wallpaper_hwnd: Arc<std::sync::atomic::AtomicIsize>,
    pub width: u32,
    pub height: u32,
    pub throttler: Arc<Throttler>,
    #[cfg(windows)]
    pub renderer: Arc<parking_lot::Mutex<Option<crate::d3d11_renderer::D3D11Renderer>>>,
    #[cfg(windows)]
    thread_handle: Option<std::thread::JoinHandle<()>>,
    #[cfg(windows)]
    recovery_thread_handle: Option<std::thread::JoinHandle<()>>,
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
            BeginPaint(hwnd, &mut ps);
            EndPaint(hwnd, &ps);
            0
        }
        WM_POWERBROADCAST => {
            if wparam == PBT_POWERSETTINGCHANGE as usize {
                crate::throttler::on_power_broadcast(lparam as isize);
            }
            1
        }
        WM_WTSSESSION_CHANGE => {
            crate::throttler::on_session_change(wparam as usize);
            0
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            let (_vx, _vy, vcx, vcy) = crate::monitor::get_virtual_screen_bounds();
            debug!("Display configuration or DPI changed. Resizing wallpaper window to {}x{}", vcx, vcy);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0, 0, vcx as i32, vcy as i32,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
            );
            crate::throttler::on_display_changed();
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(windows)]
unsafe fn discover_workerw(progman: HWND) -> HWND {
    // Topology A: Windows 11 24H2+ (WorkerW as direct child of Progman)
    let child_workerw = FindWindowExW(progman, std::ptr::null_mut(), to_wide("WorkerW").as_ptr(), std::ptr::null());
    if !child_workerw.is_null() {
        return child_workerw;
    }

    // Topology B: Classic Windows 10 & Windows 11 < 24H2 (WorkerW as sibling via EnumWindows)
    let mut state = EnumState { workerw_hwnd: std::ptr::null_mut() };
    EnumWindows(Some(enum_windows_proc), &mut state as *mut _ as LPARAM);
    state.workerw_hwnd
}

impl WorkerWContext {
    #[cfg(windows)]
    pub fn new() -> Result<Self> {
        info!("Initializing Windows WorkerW desktop injection...");

        // 0. Enumerate monitors and log details at startup
        let monitors = crate::monitor::enumerate_monitors();
        info!("Found {} active display monitor(s)", monitors.len());

        unsafe {
            // 1. Locate Progman window
            let progman = FindWindowW(to_wide("Progman").as_ptr(), std::ptr::null());
            if progman.is_null() {
                bail!("Progman window not found. Windows Explorer shell is not running.");
            }
            debug!("Found Progman window handle: 0x{:X}", progman as usize);

            // 2. Trigger Explorer to spawn WorkerW behind SHELLDLL_DefView
            // Test combinations sequentially: (0xD, 1) -> (0, 0) -> (0xD, 0)
            let combinations: [(usize, isize); 3] = [
                (0x0000000D, 1), // Win 10 22H2 / Modern
                (0, 0),          // Classic Aero
                (0x0000000D, 0), // Fallback
            ];

            let mut target_workerw: HWND = std::ptr::null_mut();
            for (wparam, lparam) in combinations {
                let mut result: usize = 0;
                let _ = SendMessageTimeoutW(
                    progman,
                    0x052C, // WM_SPAWN_WORKER
                    wparam,
                    lparam,
                    SMTO_NORMAL,
                    1000,
                    &mut result,
                );
                std::thread::sleep(std::time::Duration::from_millis(50));
                target_workerw = discover_workerw(progman);
                if !target_workerw.is_null() {
                    info!(
                        "WorkerW found (0x{:X}) after 0x052C with wParam=0x{:X}, lParam={}",
                        target_workerw as usize, wparam, lparam
                    );
                    break;
                }
            }

            let is_fallback_progman = target_workerw.is_null();
            if is_fallback_progman {
                warn!("Specific WorkerW not found via dual-topology discovery; falling back to Progman (0x{:X})", progman as usize);
                target_workerw = progman;
            }

            // 3. Create and attach dedicated wallpaper window on a background window thread
            let (wallpaper_hwnd, thread_handle) = spawn_wallpaper_window_thread(target_workerw as usize, is_fallback_progman)?;

            let (_vx, _vy, width, height) = crate::monitor::get_virtual_screen_bounds();

            let throttler = Arc::new(Throttler::new(
                0,
                progman as isize,
                target_workerw as isize,
            ));
            throttler.set_wallpaper_hwnd(wallpaper_hwnd as isize);
            throttler.evaluate();

            let hwnd_d3d = windows::Win32::Foundation::HWND(wallpaper_hwnd as *mut _);
            let renderer = match crate::d3d11_renderer::D3D11Renderer::new(hwnd_d3d, width, height) {
                Ok(r) => Arc::new(parking_lot::Mutex::new(Some(r))),
                Err(e) => {
                    warn!("Failed to create initial D3D11Renderer: {:?}", e);
                    Arc::new(parking_lot::Mutex::new(None))
                }
            };

            let wallpaper_hwnd_atomic = Arc::new(std::sync::atomic::AtomicIsize::new(wallpaper_hwnd as isize));

            // 4. Setup hidden top-level window to recover on Explorer restart (C1 / TaskbarCreated)
            let recovery_ctx = Arc::new(RecoveryContext {
                wallpaper_hwnd: Arc::clone(&wallpaper_hwnd_atomic),
                renderer: Arc::clone(&renderer),
                throttler: Arc::clone(&throttler),
                width,
                height,
            });
            *RECOVERY_CTX.write() = Some(recovery_ctx);
            let recovery_thread = spawn_recovery_listener();

            info!("WorkerW desktop injection successful. Wallpaper HWND: 0x{:X} ({}x{})", wallpaper_hwnd, width, height);

            Ok(Self {
                progman_hwnd: progman as isize,
                workerw_hwnd: target_workerw as isize,
                wallpaper_hwnd: wallpaper_hwnd_atomic,
                width,
                height,
                throttler,
                renderer,
                thread_handle: Some(thread_handle),
                recovery_thread_handle: Some(recovery_thread),
            })
        }
    }

    #[cfg(not(windows))]
    pub fn new() -> Result<Self> {
        bail!("Windows WorkerW is only supported on Windows targets");
    }
}

#[cfg(windows)]
unsafe fn spawn_wallpaper_window_thread(
    parent_for_thread: usize,
    is_fallback_progman: bool,
) -> Result<(usize, std::thread::JoinHandle<()>)> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<usize>>();

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

        let (x, y, width, height) = crate::monitor::get_virtual_screen_bounds();

        // Create window initially hidden (no WS_VISIBLE) to eliminate startup flicker
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            to_wide("Nativis Wallpaper").as_ptr(),
            WS_POPUP,
            x, y, width as i32, height as i32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        );

        if hwnd.is_null() {
            let _ = tx.send(Err(anyhow::anyhow!("Failed to create wallpaper window, error: {}", GetLastError())));
            return;
        }

        // Convert style to WS_CHILD before SetParent
        let current_style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let new_style = (current_style & !(WS_POPUP as isize)) | (WS_CHILD as isize);
        SetWindowLongPtrW(hwnd, GWL_STYLE, new_style);

        // Attach to WorkerW or Progman
        SetParent(hwnd, parent_for_thread as HWND);

        // Position window covering entire virtual screen bounds and make visible
        let (insert_after, insert_flags) = if is_fallback_progman {
            (HWND_BOTTOM, SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW)
        } else {
            (std::ptr::null_mut(), SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW)
        };

        SetWindowPos(
            hwnd,
            insert_after,
            0, 0, width as i32, height as i32,
            insert_flags,
        );

        // Register display power and session notifications + WinEvent hook
        let power_notify = windows::Win32::System::Power::RegisterPowerSettingNotification(
            windows::Win32::Foundation::HANDLE(hwnd as _),
            &windows::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE,
            windows::Win32::UI::WindowsAndMessaging::REGISTER_NOTIFICATION_FLAGS(0),
        );

        let _ = windows::Win32::System::RemoteDesktop::WTSRegisterSessionNotification(
            windows::Win32::Foundation::HWND(hwnd as _),
            0,
        );

        let winevent_hook = windows::Win32::UI::Accessibility::SetWinEventHook(
            windows::Win32::UI::WindowsAndMessaging::EVENT_SYSTEM_FOREGROUND,
            windows::Win32::UI::WindowsAndMessaging::EVENT_SYSTEM_FOREGROUND,
            windows::Win32::Foundation::HMODULE::default(),
            Some(crate::throttler::winevent_proc),
            0,
            0,
            windows::Win32::UI::WindowsAndMessaging::WINEVENT_OUTOFCONTEXT,
        );

        let _ = tx.send(Ok(hwnd as usize));

        // Standard message pump for the wallpaper window
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // Clean up power, session notifications and WinEvent hook
        if let Ok(pn) = power_notify {
            let _ = windows::Win32::System::Power::UnregisterPowerSettingNotification(pn);
        }
        let _ = windows::Win32::System::RemoteDesktop::WTSUnRegisterSessionNotification(
            windows::Win32::Foundation::HWND(hwnd as _),
        );
        if !winevent_hook.is_invalid() {
            let _ = windows::Win32::UI::Accessibility::UnhookWinEvent(winevent_hook);
        }
    });

    let hwnd = rx.recv().context("Failed to receive window handle from thread")??;
    Ok((hwnd, thread_handle))
}

#[cfg(windows)]
static TASKBAR_CREATED_MSG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[cfg(windows)]
struct RecoveryContext {
    wallpaper_hwnd: Arc<std::sync::atomic::AtomicIsize>,
    renderer: Arc<parking_lot::Mutex<Option<crate::d3d11_renderer::D3D11Renderer>>>,
    throttler: Arc<Throttler>,
    width: u32,
    height: u32,
}

#[cfg(windows)]
static RECOVERY_CTX: parking_lot::RwLock<Option<Arc<RecoveryContext>>> = parking_lot::RwLock::new(None);

#[cfg(windows)]
unsafe extern "system" fn recovery_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let taskbar_msg = TASKBAR_CREATED_MSG.load(std::sync::atomic::Ordering::Relaxed);
    if taskbar_msg != 0 && msg == taskbar_msg {
        info!("Explorer restart broadcast received (TaskbarCreated)! Initiating automatic recovery...");
        if let Some(ctx) = RECOVERY_CTX.read().clone() {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(300));
                unsafe {
                    for attempt in 1..=5 {
                        let progman = FindWindowW(to_wide("Progman").as_ptr(), std::ptr::null());
                        if progman.is_null() {
                            std::thread::sleep(std::time::Duration::from_millis(150));
                            continue;
                        }

                        let combinations: [(usize, isize); 3] = [
                            (0x0000000D, 1),
                            (0, 0),
                            (0x0000000D, 0),
                        ];

                        let mut target_workerw: HWND = std::ptr::null_mut();
                        for (wparam, lparam) in combinations {
                            let mut result: usize = 0;
                            let _ = SendMessageTimeoutW(
                                progman,
                                0x052C,
                                wparam,
                                lparam,
                                SMTO_NORMAL,
                                1000,
                                &mut result,
                            );
                            std::thread::sleep(std::time::Duration::from_millis(50));
                            target_workerw = discover_workerw(progman);
                            if !target_workerw.is_null() {
                                break;
                            }
                        }

                        let is_fallback = target_workerw.is_null();
                        let parent = if is_fallback { progman } else { target_workerw };

                        match spawn_wallpaper_window_thread(parent as usize, is_fallback) {
                            Ok((new_hwnd, _th)) => {
                                info!("Explorer recovery: new wallpaper HWND 0x{:X}", new_hwnd);
                                ctx.wallpaper_hwnd.store(new_hwnd as isize, std::sync::atomic::Ordering::SeqCst);
                                ctx.throttler.set_wallpaper_hwnd(new_hwnd as isize);

                                let hwnd_win = windows::Win32::Foundation::HWND(new_hwnd as *mut _);
                                match crate::d3d11_renderer::D3D11Renderer::new(hwnd_win, ctx.width, ctx.height) {
                                    Ok(new_r) => {
                                        *ctx.renderer.lock() = Some(new_r);
                                        info!("Explorer recovery: swapchain recreated successfully. Triggering Redraw.");
                                        crate::throttler::on_display_changed();
                                        return;
                                    }
                                    Err(e) => {
                                        warn!("Explorer recovery: D3D11Renderer::new failed: {:?}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("Explorer recovery: attempt {} failed to spawn window: {:?}", attempt, e);
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                }
            });
        }
        return 0;
    }

    match msg {
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(windows)]
fn spawn_recovery_listener() -> std::thread::JoinHandle<()> {
    std::thread::spawn(|| {
        unsafe {
            let taskbar_msg = RegisterWindowMessageW(to_wide("TaskbarCreated").as_ptr());
            TASKBAR_CREATED_MSG.store(taskbar_msg, std::sync::atomic::Ordering::SeqCst);
            debug!("Registered TaskbarCreated message ID: 0x{:X}", taskbar_msg);

            let hinstance = GetModuleHandleW(std::ptr::null());
            let class_name = to_wide("NativisRecoveryListenerClass");
            let wnd_class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: 0,
                lpfnWndProc: Some(recovery_wndproc),
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

            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                to_wide("Nativis Recovery Listener").as_ptr(),
                0,
                0, 0, 0, 0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinstance,
                std::ptr::null(),
            );

            if hwnd.is_null() {
                warn!("Failed to create TaskbarCreated recovery listener window");
                return;
            }

            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    })
}

impl Drop for WorkerWContext {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            let hwnd = self.wallpaper_hwnd.load(std::sync::atomic::Ordering::Relaxed);
            if hwnd != 0 {
                info!("Closing Nativis Wallpaper window (HWND: 0x{:X})...", hwnd as usize);
                PostMessageW(hwnd as HWND, WM_CLOSE, 0, 0);
            }
        }
        #[cfg(windows)]
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
        #[cfg(windows)]
        drop(self.recovery_thread_handle.take());
    }
}
