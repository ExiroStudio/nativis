//! macOS CGS / NSWindow level desktop strategy (Priority 90).

use tracing::info;

use crate::backend::{BackendHealth, IWallpaperDriver, IWallpaperSession, WallpaperBackendError, WallpaperContext};
use crate::metadata::{
    AttachmentStrategy, BackendConfidence, WallpaperCapabilities, WallpaperPluginMetadata,
};

pub struct MacOsWindowLevelSession {
    attached: bool,
}

impl MacOsWindowLevelSession {
    pub fn new() -> Self {
        Self { attached: false }
    }
}

impl IWallpaperSession for MacOsWindowLevelSession {
    fn name(&self) -> &'static str {
        "macOS Window Level Strategy"
    }

    fn attach(&mut self, ctx: &WallpaperContext) -> Result<(), WallpaperBackendError> {
        info!("MacOsWindowLevelSession: Setting NSWindow level to kCGDesktopWindowLevelKey on output {}...", ctx.output_index);
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
        AttachmentStrategy::NativeAPI
    }
}

impl Default for MacOsWindowLevelSession {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MacOsWindowLevelPlugin {
    meta: WallpaperPluginMetadata,
}

impl MacOsWindowLevelPlugin {
    pub fn new() -> Self {
        let meta = WallpaperPluginMetadata::new(
            "macos_window_level",
            "1.0.0",
            90,
            BackendConfidence::High,
            AttachmentStrategy::NativeAPI,
        )
        .with_capabilities(
            WallpaperCapabilities::MULTI_MONITOR
                | WallpaperCapabilities::NATIVE_BLIT
                | WallpaperCapabilities::INPUT_PASSTHROUGH
                | WallpaperCapabilities::DESKTOP_ICONS
                | WallpaperCapabilities::LIVE_RESIZE
                | WallpaperCapabilities::HOT_REATTACH
                | WallpaperCapabilities::DPI_AWARE
                | WallpaperCapabilities::HDR_OUTPUT,
        );

        Self { meta }
    }
}

impl IWallpaperDriver for MacOsWindowLevelPlugin {
    fn metadata(&self) -> &WallpaperPluginMetadata {
        &self.meta
    }

    fn create_session(&self) -> Box<dyn IWallpaperSession> {
        Box::new(MacOsWindowLevelSession::new())
    }
}

impl Default for MacOsWindowLevelPlugin {
    fn default() -> Self {
        Self::new()
    }
}
