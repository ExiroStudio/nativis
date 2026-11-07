use anyhow::{bail, Context, Result};
use nativis_core::contract::FrameSink;
use nativis_core::platform::Platform;
use nativis_core::resource::ResourceManager;

pub mod shell;
pub mod render;
pub mod sink;

pub use sink::WaylandDirectSink;

pub struct WaylandPlatform;

impl WaylandPlatform {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WaylandPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for WaylandPlatform {
    fn bootstrap(&mut self, _force: bool) -> Result<()> {
        tracing::info!("Bootstrapping WaylandPlatform (zwlr_layer_shell_v1, layer auto-detected)...");

        // Verify Wayland environment is active
        if std::env::var("WAYLAND_DISPLAY").is_err() && std::env::var("XDG_SESSION_TYPE").map(|s| s.to_lowercase() != "wayland").unwrap_or(true) {
            bail!("Wayland display not detected (WAYLAND_DISPLAY or XDG_SESSION_TYPE=wayland missing)");
        }

        tracing::info!("Wayland platform verified. Zero-install direct surface active.");
        Ok(())
    }

    fn create_sink(&self, resources: &ResourceManager) -> Result<Box<dyn FrameSink>> {
        let sink = WaylandDirectSink::new(resources.clone())
            .context("Failed to create WaylandDirectSink")?;
        Ok(Box::new(sink))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wayland_platform_creation() {
        let mut platform = WaylandPlatform::new();
        // In test environments, bootstrap verifies display check
        let _ = platform.bootstrap(false);
    }
}

