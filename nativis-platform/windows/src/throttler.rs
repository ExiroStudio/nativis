use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering};
use std::sync::Arc;
use tracing::info;
#[cfg(windows)]
use tracing::debug;

#[cfg(windows)]
use windows::{
    Win32::{
        Foundation::{HWND, RECT},
        Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST},
        System::{
            Power::POWERBROADCAST_SETTING,
            SystemServices::GUID_CONSOLE_DISPLAY_STATE,
            Threading::{GetCurrentProcess, SetProcessWorkingSetSize},
        },
        UI::{
            Shell::{SHQueryUserNotificationState, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN},
            WindowsAndMessaging::{
                GetForegroundWindow, GetSystemMetrics, GetWindowRect, IsIconic,
                SM_CMONITORS, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
                SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
                WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
            },
        },
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PauseReason {
    None = 0,
    D3DFullscreen = 1,
    PresentationMode = 2,
    DisplayOff = 3,
    SessionLocked = 4,
    DesktopOccluded = 5,
}

impl From<u8> for PauseReason {
    fn from(v: u8) -> Self {
        match v {
            1 => PauseReason::D3DFullscreen,
            2 => PauseReason::PresentationMode,
            3 => PauseReason::DisplayOff,
            4 => PauseReason::SessionLocked,
            5 => PauseReason::DesktopOccluded,
            _ => PauseReason::None,
        }
    }
}

pub struct ThrottlerState {
    pub paused: AtomicBool,
    pub reason: AtomicU8,
    pub display_off: AtomicBool,
    pub session_locked: AtomicBool,
    pub wallpaper_hwnd: AtomicIsize,
    pub progman_hwnd: AtomicIsize,
    pub workerw_hwnd: AtomicIsize,
}

impl ThrottlerState {
    pub fn new(wallpaper_hwnd: isize, progman_hwnd: isize, workerw_hwnd: isize) -> Self {
        Self {
            paused: AtomicBool::new(false),
            reason: AtomicU8::new(PauseReason::None as u8),
            display_off: AtomicBool::new(false),
            session_locked: AtomicBool::new(false),
            wallpaper_hwnd: AtomicIsize::new(wallpaper_hwnd),
            progman_hwnd: AtomicIsize::new(progman_hwnd),
            workerw_hwnd: AtomicIsize::new(workerw_hwnd),
        }
    }

    pub fn set_paused(&self, new_reason: PauseReason) {
        let prev_paused = self.paused.swap(true, Ordering::SeqCst);
        self.reason.store(new_reason as u8, Ordering::SeqCst);

        if !prev_paused {
            info!("Throttler: Desktop rendering PAUSED (reason: {:?})", new_reason);
            // Working set trim specifically when entering paused state:
            // Release memory pages back to OS while application/game is in focus
            #[cfg(windows)]
            unsafe {
                let process = GetCurrentProcess();
                let _ = SetProcessWorkingSetSize(process, usize::MAX, usize::MAX);
            }
        }
    }

    pub fn set_resumed(&self) {
        let prev_paused = self.paused.swap(false, Ordering::SeqCst);
        self.reason.store(PauseReason::None as u8, Ordering::SeqCst);

        if prev_paused {
            info!("Throttler: Desktop rendering RESUMED");
        }
    }

    #[cfg(windows)]
    pub fn evaluate(&self) {
        // 1. Check Display State (Screen Off / Sleep)
        if self.display_off.load(Ordering::Relaxed) {
            self.set_paused(PauseReason::DisplayOff);
            return;
        }

        // 2. Check Session Lock
        if self.session_locked.load(Ordering::Relaxed) {
            self.set_paused(PauseReason::SessionLocked);
            return;
        }

        // 3. Check Direct3D Full Screen & Presentation Mode via Shell API
        unsafe {
            if let Ok(user_state) = SHQueryUserNotificationState() {
                if user_state == QUNS_RUNNING_D3D_FULL_SCREEN {
                    self.set_paused(PauseReason::D3DFullscreen);
                    return;
                }
                if user_state == QUNS_PRESENTATION_MODE {
                    self.set_paused(PauseReason::PresentationMode);
                    return;
                }
            }
        }

        // 4. Check Foreground Window Occlusion
        unsafe {
            let fg = GetForegroundWindow();
            let fg_val = fg.0 as isize;
            let wp_val = self.wallpaper_hwnd.load(Ordering::Relaxed);
            let progman_val = self.progman_hwnd.load(Ordering::Relaxed);
            let workerw_val = self.workerw_hwnd.load(Ordering::Relaxed);

            // If foreground is desktop, workerw, or null -> desktop is visible!
            if fg_val == 0 || fg_val == wp_val || fg_val == progman_val || fg_val == workerw_val {
                self.set_resumed();
                return;
            }

            // If foreground is minimized -> desktop is visible!
            if IsIconic(fg).as_bool() {
                self.set_resumed();
                return;
            }

            // Check if foreground window covers all monitors or virtual screen
            if is_window_covering_screen(fg) {
                self.set_paused(PauseReason::DesktopOccluded);
                return;
            }
        }

        self.set_resumed();
    }

    #[cfg(not(windows))]
    pub fn evaluate(&self) {}
}

#[cfg(windows)]
unsafe fn is_window_covering_screen(hwnd: HWND) -> bool {
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_err() {
        return false;
    }

    let num_monitors = GetSystemMetrics(SM_CMONITORS);
    if num_monitors > 1 {
        // Multi-monitor: check if window covers virtual screen bounds
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vcx = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let vcy = GetSystemMetrics(SM_CYVIRTUALSCREEN);

        rect.left <= vx && rect.top <= vy && rect.right >= (vx + vcx) && rect.bottom >= (vy + vcy)
    } else {
        // Single monitor setup: check against monitor bounds with invisible sizing border margin tolerance
        let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };

        if !GetMonitorInfoW(hmon, &mut mi).as_bool() {
            return false;
        }

        rect.left <= mi.rcMonitor.left
            && rect.top <= mi.rcMonitor.top
            && rect.right >= mi.rcMonitor.right
            && rect.bottom >= mi.rcMonitor.bottom
    }
}

static GLOBAL_THROTTLER: parking_lot::RwLock<Option<Arc<ThrottlerState>>> = parking_lot::RwLock::new(None);

#[derive(Clone)]
pub struct Throttler {
    state: Arc<ThrottlerState>,
}

impl Throttler {
    pub fn new(wallpaper_hwnd: isize, progman_hwnd: isize, workerw_hwnd: isize) -> Self {
        let state = Arc::new(ThrottlerState::new(wallpaper_hwnd, progman_hwnd, workerw_hwnd));
        *GLOBAL_THROTTLER.write() = Some(Arc::clone(&state));
        Self { state }
    }

    pub fn is_paused(&self) -> bool {
        self.state.paused.load(Ordering::Relaxed)
    }

    pub fn pause_reason(&self) -> PauseReason {
        self.state.reason.load(Ordering::Relaxed).into()
    }

    pub fn set_wallpaper_hwnd(&self, hwnd: isize) {
        self.state.wallpaper_hwnd.store(hwnd, Ordering::SeqCst);
    }

    pub fn evaluate(&self) {
        self.state.evaluate();
    }
}

#[cfg(windows)]
pub fn on_power_broadcast(lparam: isize) {
    if lparam == 0 {
        return;
    }
    unsafe {
        let pbs = &*(lparam as *const POWERBROADCAST_SETTING);
        if pbs.PowerSetting == GUID_CONSOLE_DISPLAY_STATE {
            if let Some(state) = GLOBAL_THROTTLER.read().as_ref() {
                let display_state = *(pbs.Data.as_ptr() as *const u32);
                debug!("Throttler: Power setting display state: {}", display_state);
                if display_state == 0 {
                    // Display Off -> Pause immediately
                    state.display_off.store(true, Ordering::SeqCst);
                    state.set_paused(PauseReason::DisplayOff);
                } else {
                    // Display On or Dimmed -> Clear flag and re-evaluate
                    state.display_off.store(false, Ordering::SeqCst);
                    state.evaluate();
                }
            }
        }
    }
}

#[cfg(windows)]
pub fn on_session_change(wparam: usize) {
    if let Some(state) = GLOBAL_THROTTLER.read().as_ref() {
        if wparam == WTS_SESSION_LOCK as usize {
            debug!("Throttler: Windows session locked");
            state.session_locked.store(true, Ordering::SeqCst);
            state.set_paused(PauseReason::SessionLocked);
        } else if wparam == WTS_SESSION_UNLOCK as usize {
            debug!("Throttler: Windows session unlocked");
            state.session_locked.store(false, Ordering::SeqCst);
            state.evaluate();
        }
    }
}

#[cfg(windows)]
pub fn on_display_changed() {
    if let Some(state) = GLOBAL_THROTTLER.read().as_ref() {
        state.evaluate();
    }
}

#[cfg(windows)]
pub unsafe extern "system" fn winevent_proc(
    _hwineventhook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _idobject: i32,
    _idchild: i32,
    _ideventthread: u32,
    _dwmseventtime: u32,
) {
    if let Some(state) = GLOBAL_THROTTLER.read().as_ref() {
        state.evaluate();
    }
}
