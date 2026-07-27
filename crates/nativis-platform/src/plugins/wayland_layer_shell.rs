//! Linux Wayland Layer Shell protocol strategy (Priority 80).

use tracing::info;

use crate::backend::{BackendHealth, IWallpaperSession, IWallpaperDriver, WallpaperBackendError, WallpaperContext};
use crate::metadata::{
    AttachmentStrategy, BackendConfidence, WallpaperCapabilities,
    WallpaperPluginMetadata,
};

pub struct WaylandLayerShellSession {
    attached: bool,
}

impl WaylandLayerShellSession {
    pub fn new() -> Self {
        Self { attached: false }
    }
}

impl IWallpaperSession for WaylandLayerShellSession {
    fn name(&self) -> &'static str {
        "Wayland Layer Shell Strategy"
    }

    fn attach(&mut self, ctx: &WallpaperContext) -> Result<(), WallpaperBackendError> {
        info!("WaylandLayerShellSession: Configuring wlr-layer-shell at BACKGROUND layer on output {}...", ctx.output_index);
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
        AttachmentStrategy::LayerShell
    }
}

impl Default for WaylandLayerShellSession {
    fn default() -> Self {
        Self::new()
    }
}

pub struct WaylandLayerShellPlugin {
    meta: WallpaperPluginMetadata,
}

impl WaylandLayerShellPlugin {
    pub fn new() -> Self {
        let meta = WallpaperPluginMetadata::new(
            "wayland_layer_shell",
            "1.0.0",
            80,
            BackendConfidence::Medium,
            AttachmentStrategy::LayerShell,
        )
        .with_capabilities(
            WallpaperCapabilities::MULTI_MONITOR
                | WallpaperCapabilities::INPUT_PASSTHROUGH
                | WallpaperCapabilities::LIVE_RESIZE
                | WallpaperCapabilities::DPI_AWARE,
        );

        Self { meta }
    }
}

impl IWallpaperDriver for WaylandLayerShellPlugin {
    fn metadata(&self) -> &WallpaperPluginMetadata {
        &self.meta
    }

    fn create_session(&self) -> Box<dyn IWallpaperSession> {
        Box::new(WaylandLayerShellSession::new())
    }
}

impl Default for WaylandLayerShellPlugin {
    fn default() -> Self {
        Self::new()
    }
}
