//! Universal borderless window fallback plugin (Priority 10).

use tracing::info;

use crate::backend::{BackendHealth, IWallpaperSession, IWallpaperDriver, WallpaperBackendError, WallpaperContext};
use crate::metadata::{AttachmentStrategy, BackendConfidence, WallpaperPluginMetadata};

pub struct BorderlessFallbackSession {
    attached: bool,
}

impl BorderlessFallbackSession {
    pub fn new() -> Self {
        Self { attached: false }
    }
}

impl IWallpaperSession for BorderlessFallbackSession {
    fn name(&self) -> &'static str {
        "Borderless Window Fallback Strategy"
    }

    fn attach(&mut self, ctx: &WallpaperContext) -> Result<(), WallpaperBackendError> {
        info!("BorderlessFallbackSession: Attaching borderless window target surface on output {}...", ctx.output_index);
        ctx.surface.window.set_decorations(false);
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
        AttachmentStrategy::FallbackWindow
    }
}

impl Default for BorderlessFallbackSession {
    fn default() -> Self {
        Self::new()
    }
}

pub struct BorderlessFallbackPlugin {
    meta: WallpaperPluginMetadata,
}

impl BorderlessFallbackPlugin {
    pub fn new() -> Self {
        let meta = WallpaperPluginMetadata::new(
            "fallback_borderless",
            "1.0.0",
            10, // Lowest priority universal fallback
            BackendConfidence::Low,
            AttachmentStrategy::FallbackWindow,
        );
        Self { meta }
    }
}

impl IWallpaperDriver for BorderlessFallbackPlugin {
    fn metadata(&self) -> &WallpaperPluginMetadata {
        &self.meta
    }

    fn create_session(&self) -> Box<dyn IWallpaperSession> {
        Box::new(BorderlessFallbackSession::new())
    }
}

impl Default for BorderlessFallbackPlugin {
    fn default() -> Self {
        Self::new()
    }
}
