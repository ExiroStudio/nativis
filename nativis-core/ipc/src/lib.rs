use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[cfg(unix)]
use std::path::Path;
use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;

pub mod config;
pub use config::UserConfig;

/// Represents an IPC Command sent from the UI or CLI to the background Daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum IpcCommand {
    /// Apply a wallpaper from URI or local path
    SetWallpaper {
        uri: String,
        monitor: Option<String>,
        scaling: Option<String>,
    },
    /// Pause wallpaper playback
    Pause,
    /// Resume wallpaper playback
    Resume,
    /// Toggle pause state
    TogglePause,
    /// Set media playback speed
    SetSpeed { speed: f32 },
    /// Set media volume (0.0 to 1.0)
    SetVolume { volume: f32 },
    /// Request current engine status
    GetStatus,
    /// Request current user config
    GetConfig,
    /// Save user config
    SaveConfig(UserConfig),
    /// List known/cached wallpapers
    ListWallpapers,
    /// List all installed and available plugins
    ListPlugins,
    /// Request to install/enable a plugin
    InstallPlugin { name: String },
    /// Request environment diagnostics
    GetEnvironment,
    /// Gracefully terminate the daemon
    Shutdown,
}

/// Represents an IPC Response sent back from the Daemon to the UI or CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", content = "data")]
pub enum IpcResponse {
    Ok { message: String },
    Error { message: String },
    Status(EngineStatus),
    Config(UserConfig),
    Wallpapers(Vec<WallpaperItem>),
    Plugins(Vec<PluginInfo>),
    Environment(EnvironmentInfo),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineStatus {
    pub is_running: bool,
    pub is_paused: bool,
    pub active_wallpaper: Option<String>,
    pub target_fps: u32,
    pub playback_speed: f32,
    pub volume: f32,
    pub active_sink: String,
    pub platform: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WallpaperItem {
    pub name: String,
    pub path: String,
    pub media_type: String, // "video", "image", "web", "shader"
    pub size_formatted: String,
    pub resolution: String,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub category: String, // "platform_adapter", "media_source"
    pub description: String,
    pub status: String,    // "active", "installed", "available"
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentInfo {
    pub os: String,
    pub os_release: String,
    pub desktop_environment: String,
    pub display_server: String,
    pub active_sink_name: String,
    pub is_wayland: bool,
    pub recommended_adapter: String,
}

/// Detects the host operating system, desktop environment, and display server.
pub fn detect_environment() -> EnvironmentInfo {
    let os = std::env::consts::OS.to_string();
    
    #[cfg(target_os = "linux")]
    let (os_release, de, display_server, is_wayland) = {
        let release = std::fs::read_to_string("/etc/os-release")
            .unwrap_or_default()
            .lines()
            .find(|line| line.starts_with("PRETTY_NAME="))
            .map(|line| line.trim_start_matches("PRETTY_NAME=").trim_matches('"').to_string())
            .unwrap_or_else(|| "Linux".to_string());

        let de_raw = std::env::var("XDG_CURRENT_DESKTOP")
            .or_else(|_| std::env::var("DESKTOP_SESSION"))
            .unwrap_or_else(|_| "Unknown DE".to_string());

        let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok()
            || std::env::var("XDG_SESSION_TYPE")
                .map(|s| s.to_lowercase() == "wayland")
                .unwrap_or(false);

        let display_server = if is_wayland {
            "Wayland".to_string()
        } else if std::env::var("DISPLAY").is_ok() {
            "X11".to_string()
        } else {
            "Headless / Unknown".to_string()
        };

        (release, de_raw, display_server, is_wayland)
    };

    #[cfg(not(target_os = "linux"))]
    let (os_release, de, display_server, is_wayland) = {
        (
            format!("{} ({})", os, std::env::consts::ARCH),
            if cfg!(target_os = "windows") { "Windows Shell / DWM".to_string() } else { "Aqua / Cocoa".to_string() },
            if cfg!(target_os = "windows") { "DWM".to_string() } else { "Quartz".to_string() },
            false,
        )
    };

    let de_upper = de.to_uppercase();
    let is_kde = de_upper.contains("KDE");

    let (recommended_adapter, active_sink_name) = if is_kde {
        ("KDE Plasma Native QML Adapter (SHM)", "KDE Native QML (SHM)")
    } else if is_wayland {
        ("Wayland Layer-Shell Adapter (Layer::Background)", "Wayland Layer::Background (WGPU)")
    } else if de_upper.contains("GNOME") {
        ("GNOME Shell Dedicated Adapter", "GNOME Desktop Extension")
    } else if de_upper.contains("XFCE") || de_upper.contains("I3") || de_upper.contains("BSPWM") || de_upper.contains("CINNAMON") || de_upper.contains("MATE") {
        ("X11 Root Window Adapter", "X11 Root Window Overlay")
    } else if cfg!(target_os = "windows") {
        ("Windows WorkerW Adapter", "WorkerW DWM Swapchain")
    } else if cfg!(target_os = "macos") {
        ("macOS Desktop Window Adapter", "NSWindow Desktop Level")
    } else {
        ("Generic Window Adapter", "Generic Desktop Surface")
    };

    EnvironmentInfo {
        os,
        os_release,
        desktop_environment: de,
        display_server,
        active_sink_name: active_sink_name.to_string(),
        is_wayland,
        recommended_adapter: recommended_adapter.to_string(),
    }
}

/// Returns the standard socket path for Nativis IPC.
pub fn get_socket_path() -> PathBuf {
    #[cfg(unix)]
    {
        if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            let path = Path::new(&runtime_dir).join("nativis.sock");
            return path;
        }
        PathBuf::from("/tmp/nativis.sock")
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(r"\\.\pipe\nativis")
    }
}

/// Checks if a Nativis daemon is currently running and responding.
pub fn is_daemon_running() -> bool {
    #[cfg(unix)]
    {
        let socket_path = get_socket_path();
        if !socket_path.exists() {
            return false;
        }
        UnixStream::connect(socket_path).is_ok()
    }
    #[cfg(windows)]
    {
        use std::fs::OpenOptions;
        let pipe_path = get_socket_path();
        match OpenOptions::new().read(true).write(true).open(&pipe_path) {
            Ok(_) => true,
            Err(e) => {
                // In Windows, ERROR_PIPE_BUSY (231) means the pipe daemon is running!
                e.raw_os_error() == Some(231)
            }
        }
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        false
    }
}

/// Sends a command to the running Nativis daemon and waits for a response.
pub fn send_command(cmd: &IpcCommand) -> Result<IpcResponse> {
    #[cfg(unix)]
    {
        let socket_path = get_socket_path();
        let mut stream = UnixStream::connect(&socket_path)
            .with_context(|| format!("Failed to connect to daemon socket at {:?}", socket_path))?;

        let mut payload = serde_json::to_string(cmd)?;
        payload.push('\n');
        stream.write_all(payload.as_bytes())?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        reader.read_line(&mut response_line)?;

        let response: IpcResponse = serde_json::from_str(&response_line)
            .with_context(|| format!("Invalid response from daemon: {}", response_line))?;

        Ok(response)
    }
    #[cfg(windows)]
    {
        use std::fs::OpenOptions;
        use std::os::windows::ffi::OsStrExt;
        let pipe_path = get_socket_path();

        let mut attempts = 0;
        let stream = loop {
            match OpenOptions::new().read(true).write(true).open(&pipe_path) {
                Ok(file) => break file,
                Err(e) if e.raw_os_error() == Some(231) && attempts < 10 => {
                    attempts += 1;
                    let wide_path: Vec<u16> = pipe_path.as_os_str().encode_wide().chain(Some(0)).collect();
                    unsafe {
                        windows_sys::Win32::System::Pipes::WaitNamedPipeW(wide_path.as_ptr(), 500);
                    }
                }
                Err(e) => {
                    return Err(e).with_context(|| format!("Failed to connect to daemon pipe at {:?}", pipe_path));
                }
            }
        };

        let stream_reader = stream.try_clone()
            .with_context(|| "Failed to clone pipe handle for reading")?;

        let mut payload = serde_json::to_string(cmd)?;
        payload.push('\n');
        let mut writer = stream;
        writer.write_all(payload.as_bytes())?;
        writer.flush()?;

        let mut reader = BufReader::new(stream_reader);
        let mut response_line = String::new();
        reader.read_line(&mut response_line)?;

        let response: IpcResponse = serde_json::from_str(&response_line)
            .with_context(|| format!("Invalid response from daemon: {}", response_line))?;

        Ok(response)
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        anyhow::bail!("IPC not supported on this platform yet");
    }
}

/// Default list of plugins and their availability
pub fn get_default_plugins() -> Vec<PluginInfo> {
    vec![
        PluginInfo {
            id: "platform-kde".into(),
            name: "KDE Plasma Native Adapter".into(),
            category: "platform_adapter".into(),
            description: "Direct in-process QML injection ke plasmashell via POSIX Shared Memory".into(),
            status: "active".into(),
            version: "1.0.0".into(),
        },
        PluginInfo {
            id: "platform-wayland".into(),
            name: "Wayland Layer-Shell Adapter".into(),
            category: "platform_adapter".into(),
            description: "Direct in-process WGPU render via zwlr_layer_shell_v1 — auto-selects Layer::Background (KDE) or Layer::Bottom (Sway/Hyprland/COSMIC)".into(),
            status: "active".into(),
            version: "1.0.0".into(),
        },
        PluginInfo {
            id: "platform-x11".into(),
            name: "X11 Root Window Adapter".into(),
            category: "platform_adapter".into(),
            description: "Suntikkan wallpaper canvas ke X11 Root Window (XFCE, i3, bspwm, Cinnamon)".into(),
            status: "installed".into(),
            version: "0.2.0".into(),
        },
        PluginInfo {
            id: "platform-windows".into(),
            name: "Windows WorkerW Adapter".into(),
            category: "platform_adapter".into(),
            description: "Injeksi wallpaper ke window handle WorkerW di belakang icon desktop Windows 10/11".into(),
            status: "available".into(),
            version: "0.1.0".into(),
        },
        PluginInfo {
            id: "platform-macos".into(),
            name: "macOS Desktop Window Adapter".into(),
            category: "platform_adapter".into(),
            description: "NSWindow desktop-level wallpaper surface untuk macOS Sonoma/Sequoia".into(),
            status: "available".into(),
            version: "0.1.0".into(),
        },
        PluginInfo {
            id: "media-video".into(),
            name: "FFmpeg Hardware Video Backend".into(),
            category: "media_source".into(),
            description: "Hardware decoding video 4K MP4/WebM ke format planar NV12 ultra-efisien".into(),
            status: "active".into(),
            version: "1.0.0".into(),
        },
        PluginInfo {
            id: "media-image".into(),
            name: "Static Image Backend".into(),
            category: "media_source".into(),
            description: "High-speed zero-copy decoder untuk gambar JPG, PNG, WEBP, AVIF".into(),
            status: "active".into(),
            version: "1.0.0".into(),
        },
        PluginInfo {
            id: "media-web".into(),
            name: "HTML5 & Webview Wallpaper".into(),
            category: "media_source".into(),
            description: "Jalankan wallpaper interaktif berbasis HTML, CSS3, Three.js, Canvas".into(),
            status: "available".into(),
            version: "0.1.0".into(),
        },
        PluginInfo {
            id: "media-shader".into(),
            name: "GLSL / ShaderToy Live Shaders".into(),
            category: "media_source".into(),
            description: "Render live shaders real-time (ShaderToy / WGSL) via WGPU tanpa file video".into(),
            status: "available".into(),
            version: "0.1.0".into(),
        },
        PluginInfo {
            id: "media-audio-viz".into(),
            name: "PipeWire Audio Visualizer".into(),
            category: "media_source".into(),
            description: "Visualisasi spektrum audio real-time reaktif terhadap musik dari PipeWire/Pulse".into(),
            status: "available".into(),
            version: "0.1.0".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_environment_detection() {
        let env = detect_environment();
        assert!(!env.os.is_empty());
        assert!(!env.os_release.is_empty());
        assert!(!env.active_sink_name.is_empty());
        assert!(!env.recommended_adapter.is_empty());
    }

    #[test]
    fn test_ipc_command_serialization() {
        let cmd = IpcCommand::SetWallpaper {
            uri: "video4k.mp4".into(),
            monitor: Some("DP-1".into()),
            scaling: Some("fill".into()),
        };

        let json = serde_json::to_string(&cmd).expect("Serialize failed");
        let decoded: IpcCommand = serde_json::from_str(&json).expect("Deserialize failed");

        match decoded {
            IpcCommand::SetWallpaper { uri, monitor, scaling } => {
                assert_eq!(uri, "video4k.mp4");
                assert_eq!(monitor, Some("DP-1".into()));
                assert_eq!(scaling, Some("fill".into()));
            }
            _ => panic!("Variant mismatch"),
        }
    }

    #[test]
    fn test_plugin_list() {
        let plugins = get_default_plugins();
        assert!(plugins.iter().any(|p| p.category == "platform_adapter"));
        assert!(plugins.iter().any(|p| p.category == "media_source"));
        assert!(plugins.iter().any(|p| p.id == "platform-kde" && p.status == "active"));
    }

    #[test]
    fn test_user_config() {
        let mut cfg = UserConfig::default();
        assert_eq!(cfg.scaling, "fill");
        assert_eq!(cfg.playback_speed, 1.0);
        cfg.scaling = "fit".to_string();
        let json = serde_json::to_string(&cfg).unwrap();
        let loaded: UserConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.scaling, "fit");
    }
}
