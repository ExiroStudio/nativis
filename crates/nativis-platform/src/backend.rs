use thiserror::Error;

#[derive(Debug, Error)]
pub enum WallpaperBackendError {
    #[error("Unsupported platform: {0}")]
    Unsupported(String),
    #[error("Backend init failed: {0}")]
    Init(String),
    #[error("Display connection failed: {0}")]
    Connection(String),
}

/// Platform abstraction for desktop-background window positioning.
/// Each OS implements this to inject the engine window behind desktop icons.
pub trait IWallpaperBackend: Send + Sync {
    /// Human-readable backend name for logging.
    fn name(&self) -> &'static str;

    /// Called after the window is fully created and configured.
    /// Attaches the window to the desktop background layer.
    fn attach(&mut self, window: &winit::window::Window) -> Result<(), WallpaperBackendError>;

    /// Detach from the desktop layer and restore normal window behaviour.
    fn detach(&mut self) -> Result<(), WallpaperBackendError>;

    /// Notify the backend of a monitor output index to target.
    fn set_output(&mut self, output_index: usize);

    /// Returns `true` if the window is currently functioning as a wallpaper.
    fn is_attached(&self) -> bool;
}
