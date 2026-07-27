//! Common traits, context, surface abstractions, and errors for the wallpaper platform subsystem.

use std::fmt;
use std::error::Error;
use winit::window::Window;

use crate::detector::EnvironmentInfo;
use crate::metadata::{AttachmentStrategy, WallpaperPluginMetadata};

/// Errors originating from the wallpaper backend platform abstraction.
#[derive(Debug)]
pub enum WallpaperBackendError {
    Init(String),
    Attach(String),
    Connection(String),
    Unsupported(String),
}

impl fmt::Display for WallpaperBackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init(msg) => write!(f, "Wallpaper Backend Init Error: {msg}"),
            Self::Attach(msg) => write!(f, "Wallpaper Backend Attach Error: {msg}"),
            Self::Connection(msg) => write!(f, "Wallpaper Backend Connection Error: {msg}"),
            Self::Unsupported(msg) => write!(f, "Wallpaper Backend Unsupported: {msg}"),
        }
    }
}

impl Error for WallpaperBackendError {}

/// Abstract surface provided by the wallpaper engine to the driver.
pub struct WallpaperSurface<'a> {
    pub window: &'a Window,
}

/// Context provided to a wallpaper driver session during attachment.
pub struct WallpaperContext<'a> {
    pub surface: WallpaperSurface<'a>,
    pub output_index: usize,
    pub environment: &'a EnvironmentInfo,
}

/// Health status of a running wallpaper session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendHealth {
    /// The session is fully operational and attached correctly.
    Healthy,
    /// The desktop environment restarted or compositor lost state; requires re-attachment.
    NeedsReattach,
    /// The window surface was destroyed or invalidated.
    LostSurface,
    /// The current environment no longer supports this session (e.g. DE changed).
    Unsupported,
}

/// A running session that manages the presentation of a wallpaper surface.
///
/// Each strategy implements this interface to present rendered window surfaces
/// behind desktop icons natively.
pub trait IWallpaperSession: Send + Sync {
    /// Human-readable session name for logging.
    fn name(&self) -> &'static str;

    /// Attaches the window surface to the native desktop background layer.
    fn attach(&mut self, ctx: &WallpaperContext) -> Result<(), WallpaperBackendError>;

    /// Detaches from the desktop layer and restores standard window behavior.
    fn detach(&mut self) -> Result<(), WallpaperBackendError>;

    /// Returns `true` if the window surface is currently active as wallpaper.
    fn is_attached(&self) -> bool;

    /// Returns the health status of the session. Allows auto-recovery on `NeedsReattach`.
    fn health(&self) -> BackendHealth {
        if self.is_attached() {
            BackendHealth::Healthy
        } else {
            BackendHealth::LostSurface
        }
    }

    /// Returns the strategy this session uses.
    fn strategy(&self) -> AttachmentStrategy;
}

/// Factory and metadata provider for wallpaper drivers.
/// Drivers are strategy-only executors and do NOT contain host environment checking logic.
pub trait IWallpaperDriver: Send + Sync {
    /// Returns the metadata describing this driver's target strategy and capabilities.
    fn metadata(&self) -> &WallpaperPluginMetadata;

    /// Instantiates a fresh session instance for active presentation.
    fn create_session(&self) -> Box<dyn IWallpaperSession>;
}
