//! nativis-platform — Desktop wallpaper backend and window management.
//!
//! `IWallpaperBackend` abstracts OS-specific desktop injection:
//!   - Linux / Wayland: `wlr-layer-shell` at BACKGROUND layer
//!   - Linux / X11:    `_NET_WM_WINDOW_TYPE_DESKTOP` (feature-gated)
//!   - Windows:        WorkerW injection (Phase 4)
//!   - macOS:          CGS desktop window level (Phase 4)

pub mod backend;

pub use backend::{IWallpaperBackend, WallpaperBackendError};

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "linux")]
pub use linux::create_linux_backend;

/// Create the best available wallpaper backend for the current platform.
pub fn create_platform_backend() -> Result<Box<dyn IWallpaperBackend>, WallpaperBackendError> {
    #[cfg(target_os = "linux")]
    return linux::create_linux_backend();

    #[cfg(not(target_os = "linux"))]
    Err(WallpaperBackendError::Unsupported("Platform not yet implemented".to_string()))
}
