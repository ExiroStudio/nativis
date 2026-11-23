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

/// POD events that platform subsystems (throttler, DBus watcher, occlusion detector)
/// send to the runtime conductor.
///
/// Defined in `nativis-core` to avoid circular dependencies:
/// - Platforms depend on `nativis-core` (always OK).
/// - `nativis-runtime` depends on `nativis-core` (OK).
/// - Platforms must NOT depend on `nativis-runtime` (would create a cycle).
///
/// The conductor translates `PlatformEvent` → internal state changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformEvent {
    /// Platform detected that the wallpaper is not visible (fullscreen game, session lock,
    /// display off). Conductor should pause media clock to reach 0.00% CPU.
    Pause,
    /// Platform detected that the wallpaper is visible again. Conductor should resume
    /// media clock and re-submit the last frame to the sink.
    Resume,
    /// Platform requests a one-shot re-draw of the last submitted frame (e.g. after
    /// WM_DISPLAYCHANGE swapchain resize, or Explorer crash recovery).
    /// Conductor re-calls `sink.submit(last_frame)` without touching the backend.
    Redraw,
}

/// Channel sender for delivering `PlatformEvent` to the conductor.
/// Platforms clone this and send events from background threads (window message pump,
/// DBus subscriber, WinEvent hook) without coupling to `nativis-runtime`.
pub type PlatformEventSender = crossbeam_channel::Sender<PlatformEvent>;

/// A unified lifecycle trait for all Nativis platforms (KDE X11, Wayland, Win32, etc.)
pub trait Platform {
    /// Perform platform-specific initialization (e.g., install plugins, reload desktop shells).
    /// This is called once at startup. Set `force` to true to bypass cache checks and reinstall.
    fn bootstrap(&mut self, force: bool) -> Result<()>;

    /// Create the transport sink (e.g., ShmSink, DmaBufSink) for this platform.
    fn create_sink(&self, resources: &ResourceManager) -> Result<Box<dyn FrameSink>>;

    /// Enumerate all active displays/monitors on this platform (Phase 3).
    fn outputs(&self) -> Vec<OutputInfo> {
        #[cfg(target_os = "linux")]
        {
            let detected = detect_linux_drm_outputs();
            if !detected.is_empty() {
                return detected;
            }
        }
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

    /// Attach an event sender so platform components (throttler, display hooks)
    /// can send pause/resume/redraw events directly to the conductor.
    fn set_event_sender(&mut self, _sender: PlatformEventSender) {}
}

#[cfg(target_os = "linux")]
fn detect_linux_drm_outputs() -> Vec<OutputInfo> {
    use std::fs;
    let mut outputs = Vec::new();
    let drm_dir = std::path::Path::new("/sys/class/drm");
    if let Ok(entries) = fs::read_dir(drm_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !file_name.starts_with("card") || !file_name.contains('-') {
                continue;
            }

            let status_file = path.join("status");
            if let Ok(status) = fs::read_to_string(&status_file) {
                if status.trim() == "connected" {
                    let connector = file_name.split_once('-').map(|x| x.1).unwrap_or(file_name);
                    let mut width = 1920;
                    let mut height = 1080;

                    let modes_file = path.join("modes");
                    if let Ok(modes) = fs::read_to_string(&modes_file) {
                        if let Some(first_line) = modes.lines().next() {
                            if let Some((w_str, h_str)) = first_line.split_once('x') {
                                if let (Ok(w), Ok(h)) = (w_str.parse::<u32>(), h_str.parse::<u32>())
                                {
                                    width = w;
                                    height = h;
                                }
                            }
                        }
                    }

                    let is_primary = outputs.is_empty();
                    outputs.push(OutputInfo {
                        id: connector.to_string(),
                        name: connector.to_string(),
                        x: 0,
                        y: 0,
                        width,
                        height,
                        scale_factor: 1.0,
                        refresh_rate_mhz: 60000,
                        is_primary,
                    });
                }
            }
        }
    }
    outputs
}
