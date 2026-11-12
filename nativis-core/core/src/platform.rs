use crate::contract::FrameSink;
use crate::resource::ResourceManager;
use anyhow::Result;

/// Descriptor of a physical or virtual display monitor (Phase 3 Output Model).
#[derive(Debug, Clone, PartialEq)]
pub struct OutputInfo {
    pub id: String,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
    pub refresh_rate_mhz: u32,
    pub is_primary: bool,
}

/// A unified lifecycle trait for all Nativis platforms (KDE X11, Wayland, Win32, etc.)
pub trait Platform {
    /// Perform platform-specific initialization (e.g., install plugins, reload desktop shells).
    /// This is called once at startup. Set `force` to true to bypass cache checks and reinstall.
    fn bootstrap(&mut self, force: bool) -> Result<()>;

    /// Create the transport sink (e.g., ShmSink, DmaBufSink) for this platform.
    fn create_sink(&self, resources: &ResourceManager) -> Result<Box<dyn FrameSink>>;

    /// Enumerate all active displays/monitors on this platform (Phase 3).
    fn outputs(&self) -> Vec<OutputInfo> {
        vec![OutputInfo {
            id: "default".to_string(),
            name: "Primary Display".to_string(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale_factor: 1.0,
            refresh_rate_mhz: 60000,
            is_primary: true,
        }]
    }
}

