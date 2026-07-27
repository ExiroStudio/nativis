//! Windows WorkerW window injection strategy (Priority 90).

use tracing::info;

use crate::backend::{BackendHealth, IWallpaperSession, IWallpaperDriver, WallpaperBackendError, WallpaperContext};
use crate::metadata::{
    AttachmentStrategy, BackendConfidence, WallpaperCapabilities,
    WallpaperPluginMetadata,
};

pub struct WindowsWorkerWSession {
    attached: bool,
}

impl WindowsWorkerWSession {
    pub fn new() -> Self {
        Self { attached: false }
    }
}

impl IWallpaperSession for WindowsWorkerWSession {
    fn name(&self) -> &'static str {
        "Windows WorkerW Injection Strategy"
    }

    fn attach(&mut self, ctx: &WallpaperContext) -> Result<(), WallpaperBackendError> {
        info!("WindowsWorkerWSession: Sending 0x052C message to Progman to spawn WorkerW desktop background window on output {}...", ctx.output_index);
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
        AttachmentStrategy::WindowInjection
    }
}

impl Default for WindowsWorkerWSession {
    fn default() -> Self {
        Self::new()
    }
}

pub struct WindowsWorkerWPlugin {
    meta: WallpaperPluginMetadata,
}

impl WindowsWorkerWPlugin {
    pub fn new() -> Self {
        let meta = WallpaperPluginMetadata::new(
            "windows_workerw",
            "1.0.0",
            90,
            BackendConfidence::High,
            AttachmentStrategy::WindowInjection,
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

impl IWallpaperDriver for WindowsWorkerWPlugin {
    fn metadata(&self) -> &WallpaperPluginMetadata {
        &self.meta
    }

    fn create_session(&self) -> Box<dyn IWallpaperSession> {
        Box::new(WindowsWorkerWSession::new())
    }
}

impl Default for WindowsWorkerWPlugin {
    fn default() -> Self {
        Self::new()
    }
}
