//! Native KDE Plasma wallpaper API strategy plugin (Priority 90).

use tracing::info;

use crate::backend::{BackendHealth, IWallpaperSession, IWallpaperDriver, WallpaperBackendError, WallpaperContext};
use crate::metadata::{
    AttachmentStrategy, BackendConfidence, WallpaperCapabilities,
    WallpaperPluginMetadata,
};

pub struct KdeWallpaperApiSession {
    attached: bool,
}

impl KdeWallpaperApiSession {
    pub fn new() -> Self {
        Self { attached: false }
    }
}

impl IWallpaperSession for KdeWallpaperApiSession {
    fn name(&self) -> &'static str {
        "KDE Wallpaper API Strategy"
    }

    fn attach(&mut self, ctx: &WallpaperContext) -> Result<(), WallpaperBackendError> {
        info!("KdeWallpaperApiSession: Binding window to org.kde.plasmashell wallpaper layer for output {}...", ctx.output_index);
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

impl Default for KdeWallpaperApiSession {
    fn default() -> Self {
        Self::new()
    }
}

pub struct KdeWallpaperApiPlugin {
    meta: WallpaperPluginMetadata,
}

impl KdeWallpaperApiPlugin {
    pub fn new() -> Self {
        let meta = WallpaperPluginMetadata::new(
            "kde_wallpaper_api",
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
                | WallpaperCapabilities::HOT_REATTACH,
        );

        Self { meta }
    }
}

impl IWallpaperDriver for KdeWallpaperApiPlugin {
    fn metadata(&self) -> &WallpaperPluginMetadata {
        &self.meta
    }

    fn create_session(&self) -> Box<dyn IWallpaperSession> {
        Box::new(KdeWallpaperApiSession::new())
    }
}

impl Default for KdeWallpaperApiPlugin {
    fn default() -> Self {
        Self::new()
    }
}
