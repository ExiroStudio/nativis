//! Non-invasive host environment detection strategy.

use std::env;
use tracing::debug;
use crate::metadata::{
    DesktopEnvironment, DisplayServer, OperatingSystem, WindowManager,
};

/// Host environment inspection snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentInfo {
    pub os: OperatingSystem,
    pub display_server: DisplayServer,
    pub desktop_environment: DesktopEnvironment,
    pub window_manager: WindowManager,
    pub session_type: String,
    pub xdg_current_desktop: String,
    pub desktop_session: String,
}

impl EnvironmentInfo {
    pub fn is_wayland(&self) -> bool {
        self.display_server == DisplayServer::Wayland
    }

    pub fn is_x11(&self) -> bool {
        self.display_server == DisplayServer::X11
    }
}

/// Environment detector system.
pub struct EnvironmentDetector;

impl EnvironmentDetector {
    /// Probes the host system environment and returns an `EnvironmentInfo` snapshot.
    pub fn detect() -> EnvironmentInfo {
        let os = Self::detect_os();
        let display_server = Self::detect_display_server(os);
        let xdg_current_desktop = env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        let desktop_session = env::var("DESKTOP_SESSION").unwrap_or_default();
        let session_type = env::var("XDG_SESSION_TYPE").unwrap_or_default();

        let desktop_environment = Self::detect_desktop_environment(
            os,
            &xdg_current_desktop,
            &desktop_session,
        );

        let window_manager = Self::detect_window_manager(
            &xdg_current_desktop,
            &desktop_session,
        );

        let info = EnvironmentInfo {
            os,
            display_server,
            desktop_environment,
            window_manager,
            session_type,
            xdg_current_desktop,
            desktop_session,
        };

        debug!("Host Environment Detected: {:?}", info);
        info
    }

    fn detect_os() -> OperatingSystem {
        if cfg!(target_os = "linux") {
            OperatingSystem::Linux
        } else if cfg!(target_os = "windows") {
            OperatingSystem::Windows
        } else if cfg!(target_os = "macos") {
            OperatingSystem::MacOS
        } else if cfg!(target_os = "freebsd") {
            OperatingSystem::FreeBSD
        } else {
            OperatingSystem::Unknown
        }
    }

    fn detect_display_server(os: OperatingSystem) -> DisplayServer {
        match os {
            OperatingSystem::Windows => DisplayServer::Win32,
            OperatingSystem::MacOS => DisplayServer::Quartz,
            OperatingSystem::Linux | OperatingSystem::FreeBSD => {
                if env::var("WAYLAND_DISPLAY").is_ok()
                    || env::var("XDG_SESSION_TYPE").map(|s| s == "wayland").unwrap_or(false)
                {
                    DisplayServer::Wayland
                } else if env::var("DISPLAY").is_ok() {
                    DisplayServer::X11
                } else {
                    DisplayServer::Unknown
                }
            }
            OperatingSystem::Unknown => DisplayServer::Unknown,
        }
    }

    fn detect_desktop_environment(
        os: OperatingSystem,
        xdg_desktop: &str,
        desktop_session: &str,
    ) -> DesktopEnvironment {
        if os == OperatingSystem::Windows {
            return DesktopEnvironment::WindowsExplorer;
        }
        if os == OperatingSystem::MacOS {
            return DesktopEnvironment::MacOsFinder;
        }

        let combined = format!("{}:{}", xdg_desktop, desktop_session).to_lowercase();

        if combined.contains("kde") || combined.contains("plasma") {
            DesktopEnvironment::KdePlasma
        } else if combined.contains("gnome") || combined.contains("ubuntu") {
            DesktopEnvironment::Gnome
        } else if combined.contains("xfce") {
            DesktopEnvironment::Xfce
        } else if combined.contains("cinnamon") {
            DesktopEnvironment::Cinnamon
        } else if combined.contains("cosmic") {
            DesktopEnvironment::Cosmic
        } else if combined.contains("lxqt") {
            DesktopEnvironment::LxQt
        } else if combined.contains("mate") {
            DesktopEnvironment::Mate
        } else if combined.contains("deepin") || combined.contains("dde") {
            DesktopEnvironment::Deepin
        } else if !combined.trim().is_empty() {
            DesktopEnvironment::Generic
        } else {
            DesktopEnvironment::Unknown
        }
    }

    fn detect_window_manager(xdg_desktop: &str, desktop_session: &str) -> WindowManager {
        let combined = format!("{}:{}", xdg_desktop, desktop_session).to_lowercase();

        if combined.contains("hyprland") {
            WindowManager::Hyprland
        } else if combined.contains("sway") {
            WindowManager::Sway
        } else if combined.contains("river") {
            WindowManager::River
        } else if combined.contains("niri") {
            WindowManager::Niri
        } else if combined.contains("kwin") || combined.contains("kde") {
            WindowManager::KWin
        } else if combined.contains("mutter") || combined.contains("gnome") {
            WindowManager::Mutter
        } else if combined.contains("xfwm") || combined.contains("xfce") {
            WindowManager::Xfwm4
        } else if combined.contains("openbox") {
            WindowManager::Openbox
        } else if combined.contains("bspwm") {
            WindowManager::Bspwm
        } else if combined.contains("i3") {
            WindowManager::I3
        } else if combined.contains("dwm") {
            WindowManager::Dwm
        } else {
            WindowManager::Unknown
        }
    }
}
