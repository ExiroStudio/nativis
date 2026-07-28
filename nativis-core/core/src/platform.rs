use crate::contract::FrameSink;
use crate::resource::ResourceManager;
use anyhow::Result;

/// A unified lifecycle trait for all Nativis platforms (KDE X11, Wayland, Win32, etc.)
pub trait Platform {
    /// Perform platform-specific initialization (e.g., install plugins, reload desktop shells).
    /// This is called once at startup.
    fn bootstrap(&mut self) -> Result<()>;

    /// Create the transport sink (e.g., ShmSink, DmaBufSink) for this platform.
    fn create_sink(&self, resources: &ResourceManager) -> Result<Box<dyn FrameSink>>;
}
