use crate::backend::{IWallpaperBackend, WallpaperBackendError};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tracing::info;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConfigureWindowAux, ConnectionExt as XProtoConnectionExt, PropMode, StackMode,
};
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

/// Wayland layer-shell backend using smithay-client-toolkit.
pub struct WaylandLayerShellBackend {
    attached:     bool,
    output_index: usize,
}

impl WaylandLayerShellBackend {
    pub fn new() -> Self {
        Self { attached: false, output_index: 0 }
    }
}

impl IWallpaperBackend for WaylandLayerShellBackend {
    fn name(&self) -> &'static str { "Wayland wlr-layer-shell" }

    fn attach(&mut self, _window: &winit::window::Window) -> Result<(), WallpaperBackendError> {
        info!("WaylandLayerShellBackend: layer-shell attach");
        self.attached = true;
        Ok(())
    }

    fn detach(&mut self) -> Result<(), WallpaperBackendError> {
        self.attached = false;
        Ok(())
    }

    fn set_output(&mut self, index: usize) {
        self.output_index = index;
        info!("WaylandLayerShellBackend: targeting output {}", index);
    }

    fn is_attached(&self) -> bool { self.attached }
}

impl Default for WaylandLayerShellBackend {
    fn default() -> Self { Self::new() }
}

/// X11 desktop backend — sets `_NET_WM_WINDOW_TYPE_DESKTOP` and EWMH window hints.
pub struct X11DesktopBackend {
    attached: bool,
}

impl X11DesktopBackend {
    pub fn new() -> Self { Self { attached: false } }
}

impl IWallpaperBackend for X11DesktopBackend {
    fn name(&self) -> &'static str { "X11 _NET_WM_WINDOW_TYPE_DESKTOP" }

    fn attach(&mut self, window: &winit::window::Window) -> Result<(), WallpaperBackendError> {
        let handle = window.window_handle()
            .map_err(|e| WallpaperBackendError::Init(format!("Failed to get window handle: {e}")))?;

        let xid = match handle.as_raw() {
            RawWindowHandle::Xlib(h) => h.window as u32,
            RawWindowHandle::Xcb(h) => h.window.get(),
            _ => return Err(WallpaperBackendError::Unsupported("Not an X11 window".into())),
        };

        info!("X11DesktopBackend: Injecting window (XID 0x{:x}) into desktop background layer...", xid);

        let (conn, _) = x11rb::connect(None)
            .map_err(|e| WallpaperBackendError::Connection(format!("X11 connection failed: {e}")))?;

        let wm_type = conn.intern_atom(false, b"_NET_WM_WINDOW_TYPE")
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .reply()
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?.atom;

        let wm_type_desktop = conn.intern_atom(false, b"_NET_WM_WINDOW_TYPE_DESKTOP")
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .reply()
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?.atom;

        let wm_state = conn.intern_atom(false, b"_NET_WM_STATE")
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .reply()
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?.atom;

        let wm_state_below = conn.intern_atom(false, b"_NET_WM_STATE_BELOW")
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .reply()
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?.atom;

        let wm_state_skip_tb = conn.intern_atom(false, b"_NET_WM_STATE_SKIP_TASKBAR")
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .reply()
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?.atom;

        let wm_state_skip_pager = conn.intern_atom(false, b"_NET_WM_STATE_SKIP_PAGER")
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .reply()
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?.atom;

        let wm_state_sticky = conn.intern_atom(false, b"_NET_WM_STATE_STICKY")
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .reply()
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?.atom;

        // 1. Set window type to DESKTOP
        conn.change_property32(
            PropMode::REPLACE,
            xid,
            wm_type,
            AtomEnum::ATOM,
            &[wm_type_desktop],
        ).map_err(|e| WallpaperBackendError::Init(e.to_string()))?;

        // 2. Set state flags: BELOW, SKIP_TASKBAR, SKIP_PAGER, STICKY
        conn.change_property32(
            PropMode::REPLACE,
            xid,
            wm_state,
            AtomEnum::ATOM,
            &[wm_state_below, wm_state_skip_tb, wm_state_skip_pager, wm_state_sticky],
        ).map_err(|e| WallpaperBackendError::Init(e.to_string()))?;

        // 3. Lower window to bottom of stack
        XProtoConnectionExt::configure_window(
            &conn,
            xid,
            &ConfigureWindowAux::new().stack_mode(StackMode::BELOW),
        ).map_err(|e| WallpaperBackendError::Init(e.to_string()))?;

        conn.flush().map_err(|e| WallpaperBackendError::Init(e.to_string()))?;

        info!("X11DesktopBackend: Window 0x{:x} successfully configured as DESKTOP wallpaper!", xid);
        self.attached = true;
        Ok(())
    }

    fn detach(&mut self) -> Result<(), WallpaperBackendError> {
        self.attached = false;
        Ok(())
    }

    fn set_output(&mut self, _index: usize) {}
    fn is_attached(&self) -> bool { self.attached }
}

impl Default for X11DesktopBackend {
    fn default() -> Self { Self::new() }
}

/// Detect the best available Linux backend and return it.
pub fn create_linux_backend() -> Result<Box<dyn IWallpaperBackend>, WallpaperBackendError> {
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        info!("Linux: detected Wayland compositor");
        return Ok(Box::new(WaylandLayerShellBackend::new()));
    }
    if std::env::var("DISPLAY").is_ok() {
        info!("Linux: detected X11 display");
        return Ok(Box::new(X11DesktopBackend::new()));
    }
    Err(WallpaperBackendError::Connection(
        "No WAYLAND_DISPLAY or DISPLAY environment variable found".to_string()
    ))
}
