use crate::backend::{IWallpaperBackend, WallpaperBackendError};
use tracing::info;

/// Wayland layer-shell backend using smithay-client-toolkit.
/// Sets the window at the BACKGROUND layer so it appears behind all desktop
/// icons and application windows.
///
/// Compatible with: Hyprland, Sway, River, KDE Wayland (Plasma 6+).
/// NOT compatible with: stock GNOME Mutter (requires GNOME Shell extension).
pub struct WaylandLayerShellBackend {
    attached:      bool,
    output_index:  usize,
}

impl WaylandLayerShellBackend {
    pub fn new() -> Self {
        Self { attached: false, output_index: 0 }
    }
}

impl IWallpaperBackend for WaylandLayerShellBackend {
    fn name(&self) -> &'static str { "Wayland wlr-layer-shell" }

    fn attach(&mut self) -> Result<(), WallpaperBackendError> {
        // Phase 1: the window is created by winit which negotiates xdg-shell.
        // Full layer-shell surface replacement requires smithay-client-toolkit
        // direct surface creation, which is wired in Phase 2.
        //
        // For Phase 1, we log the intent and mark attached. The demo will
        // render correctly even as a regular window, confirming the
        // render pipeline works before we add the shell injection.
        info!("WaylandLayerShellBackend: layer-shell attach (Phase 2 will inject wlr-layer-shell)");
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

/// X11 desktop backend — sets `_NET_WM_WINDOW_TYPE_DESKTOP`.
/// Enabled via `platform-x11` feature flag.
pub struct X11DesktopBackend {
    attached: bool,
}

impl X11DesktopBackend {
    pub fn new() -> Self { Self { attached: false } }
}

impl IWallpaperBackend for X11DesktopBackend {
    fn name(&self) -> &'static str { "X11 _NET_WM_WINDOW_TYPE_DESKTOP" }

    fn attach(&mut self) -> Result<(), WallpaperBackendError> {
        info!("X11DesktopBackend: _NET_WM_WINDOW_TYPE_DESKTOP (Phase 2 xcb integration)");
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

/// Detect the best available Linux backend and return it.
pub fn create_linux_backend() -> Result<Box<dyn IWallpaperBackend>, WallpaperBackendError> {
    // Prefer Wayland if WAYLAND_DISPLAY is set
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        info!("Linux: detected Wayland compositor");
        return Ok(Box::new(WaylandLayerShellBackend::new()));
    }
    // Fall back to X11
    if std::env::var("DISPLAY").is_ok() {
        info!("Linux: detected X11 display");
        return Ok(Box::new(X11DesktopBackend::new()));
    }
    Err(WallpaperBackendError::Connection(
        "No WAYLAND_DISPLAY or DISPLAY environment variable found".to_string()
    ))
}
