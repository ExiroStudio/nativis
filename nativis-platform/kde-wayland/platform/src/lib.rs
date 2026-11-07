use std::path::{Path, PathBuf};
use std::fs;
use std::process::Command;
use anyhow::{Context, Result, anyhow};
use tracing::{info, warn};
use nativis_core::platform::Platform;
use nativis_core::contract::FrameSink;
use nativis_core::resource::ResourceManager;
use nativis_transport_shm::ShmSink;

pub struct KdeWaylandPlatform {
    installed_plugin_dir: PathBuf,
}

impl KdeWaylandPlatform {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let installed_plugin_dir = Path::new(&home)
            .join(".local/share/plasma/wallpapers/com.nativis.wallpaper");

        Self {
            installed_plugin_dir,
        }
    }

    fn check_and_install_bundle(&self, force: bool) -> Result<bool> {
        let bundle_dir = find_bundle_dir();
        if !bundle_dir.exists() {
            warn!("KDE Wayland bundle directory not found at {:?}", bundle_dir);
            return Ok(false);
        }

        let main_qml = self.installed_plugin_dir.join("contents/ui/main.qml");
        let plugin_so = self.installed_plugin_dir.join("contents/ui/libnativisplugin.so");

        let needs_install = force || !main_qml.exists() || !plugin_so.exists();

        if needs_install {
            info!("Installing KDE Plasma 6 Wayland wallpaper package into {:?}...", self.installed_plugin_dir);
            fs::create_dir_all(&self.installed_plugin_dir)?;

            // Copy recursively platforms/kde-wayland to installed_plugin_dir
            copy_dir_all(&bundle_dir, &self.installed_plugin_dir)
                .context("Failed to copy KDE Wayland wallpaper package")?;

            // Try kpackagetool6 if available to ensure Plasma recognizes package
            let _ = Command::new("kpackagetool6")
                .args(&["-t", "Plasma/Wallpaper", "-r", "com.nativis.wallpaper"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();

            let _ = Command::new("kpackagetool6")
                .args(&["-t", "Plasma/Wallpaper", "-i", bundle_dir.to_str().unwrap()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();

            // Refresh sycoca cache
            let _ = Command::new("kbuildsycoca6")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();

            info!("KDE Plasma 6 Wayland package installed successfully.");
            return Ok(true);
        }

        Ok(false)
    }

    fn reload_plasma(&self) -> Result<()> {
        info!("Applying Nativis wallpaper to Plasma 6 via qdbus-qt6...");
        let script = "var Desktops = desktops(); for (i=0;i<Desktops.length;i++) { d = Desktops[i]; d.wallpaperPlugin = 'org.kde.image'; d.wallpaperPlugin = 'com.nativis.wallpaper'; }";
        
        let output = Command::new("qdbus-qt6")
            .args(&[
                "org.kde.plasmashell",
                "/PlasmaShell",
                "org.kde.PlasmaShell.evaluateScript",
                script,
            ])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                info!("Plasma 6 wallpaper plugin reloaded successfully.");
            }
            Ok(out) => {
                warn!("Plasma 6 reload script returned non-zero: {:?}", out);
            }
            Err(e) => {
                warn!("Failed to invoke qdbus-qt6: {}", e);
            }
        }
        Ok(())
    }
}

fn find_bundle_dir() -> PathBuf {
    let local = Path::new("./platforms/kde-wayland");
    if local.exists() {
        return local.to_path_buf();
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let next_to_exe = parent.join("platforms/kde-wayland");
            if next_to_exe.exists() {
                return next_to_exe;
            }
        }
    }
    PathBuf::from("./platforms/kde-wayland")
}

fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
    fs::create_dir_all(&dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.as_ref().join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst_path)?;
        } else {
            fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}

impl Platform for KdeWaylandPlatform {
    fn bootstrap(&mut self, force: bool) -> Result<()> {
        info!("Bootstrapping KdeWaylandPlatform (force={})...", force);
        let installed = self.check_and_install_bundle(force)?;
        if installed {
            self.reload_plasma()?;
        }
        Ok(())
    }

    fn create_sink(&self, resources: &ResourceManager) -> Result<Box<dyn FrameSink>> {
        // Create SHM region big enough for a 4K frame (3840 * 2160 * 4 bytes + 4096 bytes headers)
        let shm_size = (3840 * 2160 * 4) + 4096;
        let sink = ShmSink::new("/nativis_shm", shm_size, resources.clone())
            .map_err(|e| anyhow!(e))?;
        Ok(Box::new(sink))
    }
}

impl Drop for KdeWaylandPlatform {
    fn drop(&mut self) {
        info!("KdeWaylandPlatform shutting down...");
    }
}
