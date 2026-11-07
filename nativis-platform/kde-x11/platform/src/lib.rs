use std::path::{Path, PathBuf};
use std::fs;
use std::process::Command;
use anyhow::{Context, Result, anyhow};
use tracing::{info, warn};
use nativis_core::platform::Platform;
use nativis_core::contract::FrameSink;
use nativis_core::resource::ResourceManager;
use nativis_transport_shm::ShmSink;

pub struct KdePlatform {
    installed_plugin_dir: PathBuf,
}

impl KdePlatform {
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
            warn!("KDE Plasma 5 bundle directory not found at {:?}", bundle_dir);
            return Ok(false);
        }

        let main_qml = self.installed_plugin_dir.join("contents/ui/main.qml");
        let plugin_so = self.installed_plugin_dir.join("contents/ui/libnativisplugin.so");

        let needs_install = force || !main_qml.exists() || !plugin_so.exists();

        if needs_install {
            info!("Installing KDE Plasma 5 wallpaper package into {:?}...", self.installed_plugin_dir);
            fs::create_dir_all(&self.installed_plugin_dir)?;

            // Copy recursively platforms/kde-x11 to installed_plugin_dir (User Space, zero sudo)
            copy_dir_all(&bundle_dir, &self.installed_plugin_dir)
                .context("Failed to copy KDE Plasma 5 wallpaper package")?;

            // Try kpackagetool5 if available to ensure Plasma recognizes package
            let _ = Command::new("kpackagetool5")
                .args(&["-t", "Plasma/Wallpaper", "-r", "com.nativis.wallpaper"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();

            let _ = Command::new("kpackagetool5")
                .args(&["-t", "Plasma/Wallpaper", "-i", bundle_dir.to_str().unwrap()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();

            // Refresh sycoca cache
            let _ = Command::new("kbuildsycoca5")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();

            info!("KDE Plasma 5 wallpaper package installed successfully.");
            return Ok(true);
        }

        Ok(false)
    }

    fn reload_plasma(&self) -> Result<()> {
        info!("Reloading Plasma desktop...");
        let output = Command::new("qdbus")
            .args(&[
                "org.kde.plasmashell",
                "/PlasmaShell",
                "org.kde.PlasmaShell.evaluateScript",
                "var Desktops = desktops(); for (i=0;i<Desktops.length;i++) { d = Desktops[i]; d.wallpaperPlugin = 'org.kde.image'; d.wallpaperPlugin = 'com.nativis.wallpaper'; }",
            ])
            .output()?;

        if !output.status.success() {
            warn!("Failed to reload plasma: {:?}", output);
        }
        Ok(())
    }
}

impl Platform for KdePlatform {
    fn bootstrap(&mut self, force: bool) -> Result<()> {
        info!("Bootstrapping KdePlatform (force={})...", force);
        
        // 1. Check and install bundle if necessary or forced
        let installed = self.check_and_install_bundle(force)?;
        
        // 2. Reload plasmashell if we installed or updated
        if installed {
            self.reload_plasma()?;
        }

        Ok(())
    }

    fn create_sink(&self, resources: &ResourceManager) -> Result<Box<dyn FrameSink>> {
        // Create an SHM region big enough for a 4K frame (3840 * 2160 * 4 bytes + some padding for headers)
        let shm_size = (3840 * 2160 * 4) + 4096;
        let sink = ShmSink::new("/nativis_shm", shm_size, resources.clone())
            .map_err(|e| anyhow!(e))?;
        Ok(Box::new(sink))
    }
}

impl Drop for KdePlatform {
    fn drop(&mut self) {
        info!("KdePlatform cleaning up...");
        // Here we could unmap SHM or do DBus teardown if needed.
        // For PoC, the OS cleans up our SHM because we don't unlink it if plasma is still using it,
        // or we do unlink it if we want it to die with us.
    }
}

fn find_bundle_dir() -> PathBuf {
    let local = Path::new("./platforms/kde-x11");
    if local.exists() {
        return local.to_path_buf();
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let next_to_exe = parent.join("platforms/kde-x11");
            if next_to_exe.exists() {
                return next_to_exe;
            }
        }
    }
    PathBuf::from("./platforms/kde-x11")
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
