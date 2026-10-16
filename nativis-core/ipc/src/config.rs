use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    /// Active wallpaper URI or local file path
    pub active_wallpaper: String,
    /// Playback speed multiplier (0.25 to 2.0)
    pub playback_speed: f32,
    /// Audio volume (0.0 to 1.0)
    pub volume: f32,
    /// Display scaling mode: "fill", "fit", "stretch", "center"
    pub scaling: String,
    /// Target engine framerate
    pub target_fps: u32,
    /// Pause playback when a fullscreen application is focused
    pub pause_on_fullscreen: bool,
    /// Pause playback when device is on battery saver mode
    pub pause_on_battery: bool,
    /// Enable hardware decoding
    pub hw_acceleration: bool,
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            active_wallpaper: "arlenchino.mp4".to_string(),
            playback_speed: 1.0,
            volume: 0.0,
            scaling: "fill".to_string(),
            target_fps: 60,
            pause_on_fullscreen: true,
            pause_on_battery: true,
            hw_acceleration: true,
        }
    }
}

impl UserConfig {
    /// Resolves the user configuration directory ($XDG_CONFIG_HOME/nativis or ~/.config/nativis).
    pub fn config_dir() -> PathBuf {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            if !xdg.is_empty() {
                return Path::new(&xdg).join("nativis");
            }
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        Path::new(&home).join(".config").join("nativis")
    }

    /// Resolves the configuration file path.
    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.json")
    }

    /// Loads the configuration from disk, or returns default if not found or corrupted.
    pub fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(cfg) = serde_json::from_str::<UserConfig>(&content) {
                    return cfg;
                } else {
                    warn!("Failed to parse config at {:?}, using defaults", path);
                }
            }
        }

        // Default configuration
        let mut cfg = Self::default();

        // If default wallpaper doesn't exist, search for any known media in current directory
        if !Path::new(&cfg.active_wallpaper).exists() {
            let candidates = ["arlenchino.mp4", "video4k.mp4", "CuteBunny.jpg", "arona.mp4", "remielle.mp4", "WallpaperSunna.jpg", "1080.mp4", "video.mp4"];
            for candidate in candidates {
                if Path::new(candidate).exists() {
                    cfg.active_wallpaper = candidate.to_string();
                    break;
                }
            }
        }

        let _ = cfg.save();
        cfg
    }

    /// Saves the current configuration to disk.
    pub fn save(&self) -> Result<()> {
        let dir = Self::config_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create config directory at {:?}", dir))?;

        let path = Self::config_path();
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)
            .with_context(|| format!("Failed to write config file to {:?}", path))?;

        info!("Saved user config to {:?}", path);
        Ok(())
    }
}
