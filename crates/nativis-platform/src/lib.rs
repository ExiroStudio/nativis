//! Platform-specific native wallpaper attachment subsystem.
//!
//! Exposes a rule-driven capability resolution engine and wallpaper driver loader.

pub mod backend;
pub mod detector;
pub mod metadata;
pub mod plugins;
pub mod registry;
pub mod resolver;
pub mod rules;

pub use backend::{
    BackendHealth, IWallpaperDriver, IWallpaperSession, WallpaperBackendError, WallpaperContext,
    WallpaperSurface,
};
pub use detector::{EnvironmentDetector, EnvironmentInfo};
pub use metadata::{
    AttachmentStrategy, BackendConfidence, DesktopEnvironment, DisplayServer, OperatingSystem,
    WallpaperCapabilities, WallpaperPluginMetadata, WindowManager,
};
pub use registry::WallpaperDriverLoader;
pub use resolver::ResolutionEngine;
pub use rules::{AttachmentPlan, ResolutionRule};

/// Universal entrypoint: creates a default `WallpaperDriverLoader`, probes the host environment,
/// resolves an `AttachmentPlan`, and attaches the matching driver session to the provided window.
pub fn attach_platform_backend(
    window: &winit::window::Window,
    output_index: usize,
) -> Result<Box<dyn IWallpaperSession>, WallpaperBackendError> {
    let loader = WallpaperDriverLoader::with_default_drivers();
    loader.select_and_attach(window, output_index)
}
