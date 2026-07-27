//! Linux X11 EWMH desktop window hint strategy (Priority 50).

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tracing::info;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConfigureWindowAux, ConnectionExt as XProtoConnectionExt, PropMode, StackMode,
};
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

use crate::backend::{BackendHealth, IWallpaperDriver, IWallpaperSession, WallpaperBackendError, WallpaperContext};
use crate::metadata::{
    AttachmentStrategy, BackendConfidence, WallpaperCapabilities, WallpaperPluginMetadata,
};

pub struct X11EwmhDesktopSession {
    attached: bool,
}

impl X11EwmhDesktopSession {
    pub fn new() -> Self {
        Self { attached: false }
    }
}

impl IWallpaperSession for X11EwmhDesktopSession {
    fn name(&self) -> &'static str {
        "X11 EWMH Desktop Strategy"
    }

    fn attach(&mut self, ctx: &WallpaperContext) -> Result<(), WallpaperBackendError> {
        let handle = ctx.surface.window.window_handle().map_err(|e| {
            WallpaperBackendError::Init(format!("Failed to get raw window handle: {e}"))
        })?;

        let xid = match handle.as_raw() {
            RawWindowHandle::Xlib(h) => h.window as u32,
            RawWindowHandle::Xcb(h) => h.window.get(),
            _ => {
                return Err(WallpaperBackendError::Unsupported(
                    "Not an X11 window handle".into(),
                ))
            }
        };

        info!(
            "X11EwmhDesktopSession: Injecting window (XID 0x{:x}) into EWMH desktop background layer...",
            xid
        );

        let (conn, _) = x11rb::connect(None).map_err(|e| {
            WallpaperBackendError::Connection(format!("X11 connection failed: {e}"))
        })?;

        let wm_type = conn
            .intern_atom(false, b"_NET_WM_WINDOW_TYPE")
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .reply()
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .atom;

        let wm_type_desktop = conn
            .intern_atom(false, b"_NET_WM_WINDOW_TYPE_DESKTOP")
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .reply()
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .atom;

        let wm_state = conn
            .intern_atom(false, b"_NET_WM_STATE")
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .reply()
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .atom;

        let wm_state_below = conn
            .intern_atom(false, b"_NET_WM_STATE_BELOW")
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .reply()
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .atom;

        let wm_state_skip_tb = conn
            .intern_atom(false, b"_NET_WM_STATE_SKIP_TASKBAR")
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .reply()
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .atom;

        let wm_state_skip_pager = conn
            .intern_atom(false, b"_NET_WM_STATE_SKIP_PAGER")
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .reply()
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .atom;

        let wm_state_sticky = conn
            .intern_atom(false, b"_NET_WM_STATE_STICKY")
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .reply()
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?
            .atom;

        // Set window type to DESKTOP
        conn.change_property32(
            PropMode::REPLACE,
            xid,
            wm_type,
            AtomEnum::ATOM,
            &[wm_type_desktop],
        )
        .map_err(|e| WallpaperBackendError::Init(e.to_string()))?;

        // Set state flags
        conn.change_property32(
            PropMode::REPLACE,
            xid,
            wm_state,
            AtomEnum::ATOM,
            &[
                wm_state_below,
                wm_state_skip_tb,
                wm_state_skip_pager,
                wm_state_sticky,
            ],
        )
        .map_err(|e| WallpaperBackendError::Init(e.to_string()))?;

        // Lower window stack mode
        XProtoConnectionExt::configure_window(
            &conn,
            xid,
            &ConfigureWindowAux::new().stack_mode(StackMode::BELOW),
        )
        .map_err(|e| WallpaperBackendError::Init(e.to_string()))?;

        conn.flush()
            .map_err(|e| WallpaperBackendError::Init(e.to_string()))?;

        info!(
            "X11EwmhDesktopSession: Window 0x{:x} successfully configured as DESKTOP wallpaper!",
            xid
        );
        self.attached = true;
        Ok(())
    }

    fn detach(&mut self) -> Result<(), WallpaperBackendError> {
        self.attached = false;
        Ok(())
    }

    fn is_attached(&self) -> bool {
        self.attached
    }

    fn health(&self) -> BackendHealth {
        if self.attached {
            BackendHealth::Healthy
        } else {
            BackendHealth::LostSurface
        }
    }

    fn strategy(&self) -> AttachmentStrategy {
        AttachmentStrategy::EwmhDesktop
    }
}

impl Default for X11EwmhDesktopSession {
    fn default() -> Self {
        Self::new()
    }
}

pub struct X11EwmhDesktopPlugin {
    meta: WallpaperPluginMetadata,
}

impl X11EwmhDesktopPlugin {
    pub fn new() -> Self {
        let meta = WallpaperPluginMetadata::new(
            "x11_ewmh_desktop",
            "1.0.0",
            50,
            BackendConfidence::Medium,
            AttachmentStrategy::EwmhDesktop,
        )
        .with_capabilities(
            WallpaperCapabilities::INPUT_PASSTHROUGH
                | WallpaperCapabilities::LIVE_RESIZE,
        );

        Self { meta }
    }
}

impl IWallpaperDriver for X11EwmhDesktopPlugin {
    fn metadata(&self) -> &WallpaperPluginMetadata {
        &self.meta
    }

    fn create_session(&self) -> Box<dyn IWallpaperSession> {
        Box::new(X11EwmhDesktopSession::new())
    }
}

impl Default for X11EwmhDesktopPlugin {
    fn default() -> Self {
        Self::new()
    }
}
