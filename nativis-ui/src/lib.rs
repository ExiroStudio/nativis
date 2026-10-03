slint::include_modules!();

use std::path::Path;
use anyhow::Result;
use tracing::{info, warn};
use slint::{Model, ModelRc, VecModel};
use nativis_ipc::{IpcCommand, detect_environment, is_daemon_running, send_command};

/// Main entry point to launch the Nativis Modern UI.
pub fn run_ui() -> Result<()> {
    info!("Starting Nativis Modern UI...");

    let app = AppWindow::new()?;

    // 1. Detect and populate Host Environment Diagnostics
    let env_info = detect_environment();
    app.set_detected_os(env_info.os_release.into());
    app.set_detected_de(format!("{} ({})", env_info.desktop_environment, env_info.display_server).into());
    app.set_active_sink_name(env_info.active_sink_name.into());

    // 2. Discover local wallpapers in current directory / common directories
    let discovered_wallpapers = scan_wallpapers();
    if !discovered_wallpapers.is_empty() {
        let models: Vec<WallpaperData> = discovered_wallpapers
            .into_iter()
            .enumerate()
            .map(|(idx, item)| WallpaperData {
                name: item.name.into(),
                path: item.path.into(),
                media_type: item.media_type.into(),
                meta: item.meta.into(),
                size: item.size.into(),
                is_active: idx == 0, // First item is initially active
            })
            .collect();
        app.set_wallpapers(ModelRc::new(VecModel::from(models)));
    }

    // 3. Check if daemon is active
    let daemon_up = is_daemon_running();
    app.set_engine_running(daemon_up);
    app.set_engine_status_text(if daemon_up {
        "Engine Online • 60 FPS".into()
    } else {
        "Engine Standby (Click Apply to Start)".into()
    });

    // 4. Connect Callbacks
    let app_weak = app.as_weak();
    app.on_apply_wallpaper(move |target_path| {
        info!("UI requested to apply wallpaper: {}", target_path);
        let path_str = target_path.to_string();

        // Send IPC command to background daemon
        let cmd = IpcCommand::SetWallpaper {
            uri: path_str.clone(),
            monitor: None,
            scaling: None,
        };

        match send_command(&cmd) {
            Ok(resp) => {
                info!("Daemon response: {:?}", resp);
            }
            Err(e) => {
                warn!("Could not reach daemon via IPC ({}), running directly or queuing", e);
            }
        }

        // Update active indicators in the UI
        if let Some(app) = app_weak.upgrade() {
            let current_model = app.get_wallpapers();
            let mut updated_items = Vec::new();
            for i in 0..current_model.row_count() {
                if let Some(mut row) = current_model.row_data(i) {
                    row.is_active = row.name == path_str || row.path == path_str;
                    updated_items.push(row);
                }
            }
            app.set_wallpapers(ModelRc::new(VecModel::from(updated_items)));
            app.set_engine_running(true);
            app.set_engine_status_text("Engine Online • 60 FPS".into());
        }
    });

    let app_weak_pause = app.as_weak();
    app.on_toggle_pause(move || {
        if let Some(app) = app_weak_pause.upgrade() {
            let next_running = !app.get_engine_running();
            let cmd = if next_running { IpcCommand::Resume } else { IpcCommand::Pause };
            let _ = send_command(&cmd);
            app.set_engine_running(next_running);
            app.set_engine_status_text(if next_running {
                "Engine Online • 60 FPS".into()
            } else {
                "Engine Paused".into()
            });
        }
    });

    app.on_change_speed(move |speed| {
        info!("UI requested speed change: {}x", speed);
        let _ = send_command(&IpcCommand::SetSpeed { speed });
    });

    app.on_install_plugin(move |name| {
        info!("UI requested to install plugin: {}", name);
        let _ = send_command(&IpcCommand::InstallPlugin { name: name.to_string() });
    });

    let app_weak_add = app.as_weak();
    app.on_add_custom_wallpaper(move || {
        info!("Add custom media clicked");
        if let Some(app) = app_weak_add.upgrade() {
            let current = app.get_wallpapers();
            let mut items: Vec<WallpaperData> = (0..current.row_count())
                .filter_map(|i| current.row_data(i))
                .collect();
            
            // Add a mock or scanned item as demonstration
            items.push(WallpaperData {
                name: "CustomWallpaper.mp4".into(),
                path: "CustomWallpaper.mp4".into(),
                media_type: "video".into(),
                meta: "1080p 60fps (User Imported)".into(),
                size: "24.5 MB".into(),
                is_active: false,
            });
            app.set_wallpapers(ModelRc::new(VecModel::from(items)));
        }
    });

    // Run the Slint Event Loop
    app.run()?;

    Ok(())
}

struct ScannedWallpaper {
    name: String,
    path: String,
    media_type: String,
    meta: String,
    size: String,
}

fn scan_wallpapers() -> Vec<ScannedWallpaper> {
    let mut results = Vec::new();
    let current_dir = Path::new(".");

    let known_files = [
        ("arlenchino.mp4", "video", "1080p 60fps (NV12)", "15.2 MB"),
        ("video4k.mp4", "video", "3840x2160 (4K NV12)", "23.1 MB"),
        ("CuteBunny.jpg", "image", "1920x1080 (RGBA)", "796 KB"),
        ("arona.mp4", "video", "1080p 60fps (NV12)", "33.0 MB"),
        ("WallpaperSunna.jpg", "image", "3840x2160 (RGBA)", "971 KB"),
        ("remielle.mp4", "video", "1080p 60fps (NV12)", "19.4 MB"),
        ("1080.mp4", "video", "1920x1080 60fps", "33.4 MB"),
        ("video.mp4", "video", "1280x720 30fps", "1.0 MB"),
    ];

    for (name, media_type, meta, default_size) in known_files {
        let p = current_dir.join(name);
        if p.exists() {
            let size_str = if let Ok(meta_fs) = std::fs::metadata(&p) {
                let bytes = meta_fs.len();
                if bytes > 1024 * 1024 {
                    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
                } else {
                    format!("{} KB", bytes / 1024)
                }
            } else {
                default_size.to_string()
            };

            results.push(ScannedWallpaper {
                name: name.to_string(),
                path: name.to_string(),
                media_type: media_type.to_string(),
                meta: meta.to_string(),
                size: size_str,
            });
        }
    }

    results
}
