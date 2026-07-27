//! Wallpaper plugin metadata specifications and capability definitions.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

/// Operating system targets (used by detector and rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OperatingSystem {
    Linux,
    Windows,
    MacOS,
    FreeBSD,
    Unknown,
}

/// Display server protocol targets (used by detector and rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DisplayServer {
    Wayland,
    X11,
    Win32,
    Quartz,
    Unknown,
}

/// Desktop environment targets (used by detector and rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DesktopEnvironment {
    KdePlasma,
    Gnome,
    Xfce,
    Cinnamon,
    Cosmic,
    LxQt,
    Mate,
    Deepin,
    WindowsExplorer,
    MacOsFinder,
    Generic,
    Unknown,
}

/// Window manager targets (used by detector and rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WindowManager {
    Hyprland,
    Sway,
    River,
    Niri,
    KWin,
    Mutter,
    Xfwm4,
    Marco,
    Openbox,
    Bspwm,
    I3,
    Dwm,
    DwmWindows,
    QuartzMacOS,
    Generic,
    Unknown,
}

/// Strategy used by the driver to attach the wallpaper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash)]
pub enum AttachmentStrategy {
    /// Universal borderless fullscreen window fallback.
    FallbackWindow,
    /// EWMH desktop hints / Stacking.
    EwmhDesktop,
    /// Direct blitting to the root window (e.g. X11 Root Window).
    RootBlit,
    /// Wayland Layer Shell protocol.
    LayerShell,
    /// Injecting window into desktop shell hierarchy (e.g. Windows WorkerW).
    WindowInjection,
    /// Native API provided by the OS or DE (e.g. KDE Wallpaper API, macOS CGS).
    NativeAPI,
}

/// Confidence level that the driver will work optimally in the target strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BackendConfidence {
    Low,
    Medium,
    High,
}

bitflags! {
    /// Capabilities exposed by a wallpaper driver.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct WallpaperCapabilities: u32 {
        /// Supports per-monitor display targeting.
        const MULTI_MONITOR     = 0b000000001;
        /// Mouse & input events pass through window to desktop icons.
        const INPUT_PASSTHROUGH = 0b000000010;
        /// Alpha blending with underlying desktop shell.
        const TRANSPARENCY      = 0b000000100;
        /// Direct compositor layer blit attachment.
        const NATIVE_BLIT       = 0b000001000;
        /// Desktop icons remain visible on top of the wallpaper.
        const DESKTOP_ICONS     = 0b000010000;
        /// Can survive and adapt to live output/resolution resizing.
        const LIVE_RESIZE       = 0b000100000;
        /// Can recover cleanly if the compositor/DE restarts.
        const HOT_REATTACH      = 0b001000000;
        /// Correctly scales with dynamic OS DPI changes.
        const DPI_AWARE         = 0b010000000;
        /// Supports HDR color space output.
        const HDR_OUTPUT        = 0b100000000;
    }
}

impl serde::Serialize for WallpaperCapabilities {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u32(self.bits())
    }
}

impl<'de> serde::Deserialize<'de> for WallpaperCapabilities {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bits = u32::deserialize(deserializer)?;
        Ok(WallpaperCapabilities::from_bits_truncate(bits))
    }
}

/// Metadata exposed by every wallpaper driver.
/// Purely describes the driver's capabilities and strategy without host coupling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WallpaperPluginMetadata {
    pub name: String,
    pub version: String,
    pub min_engine_version: Option<String>,
    pub max_engine_version: Option<String>,
    pub priority: u32,
    pub confidence: BackendConfidence,
    pub strategy: AttachmentStrategy,
    pub capabilities: WallpaperCapabilities,
}

impl WallpaperPluginMetadata {
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        priority: u32,
        confidence: BackendConfidence,
        strategy: AttachmentStrategy,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            min_engine_version: Some("2.0.0".into()),
            max_engine_version: None,
            priority,
            confidence,
            strategy,
            capabilities: WallpaperCapabilities::empty(),
        }
    }

    pub fn with_capabilities(mut self, caps: WallpaperCapabilities) -> Self {
        self.capabilities |= caps;
        self
    }
}
