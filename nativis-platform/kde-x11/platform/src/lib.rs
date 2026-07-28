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

    fn check_and_install_bundle(&self) -> Result<bool> {
        // Find the bundle in ./platforms/kde-x11
        let bundle_dir = Path::new("./platforms/kde-x11");
        if !bundle_dir.exists() {
            warn!("Bundle directory not found at {:?}", bundle_dir);
            return Ok(false);
        }

        // For PoC V1, we assume the bundle is always newer/needed if it exists,
        // or we just unconditionally copy it for simplicity in development.
        // A real implementation would parse manifest.toml and compare versions.
        info!("Installing KDE Plasma plugin from bundle...");

        if self.installed_plugin_dir.exists() {
            fs::remove_dir_all(&self.installed_plugin_dir)?;
        }
        
        fs::create_dir_all(&self.installed_plugin_dir)?;

        // Recursive copy
        Self::copy_dir_recursive(&bundle_dir, &self.installed_plugin_dir)?;

        info!("KDE Plasma plugin installed to {:?}", self.installed_plugin_dir);
        Ok(true) // indicates we installed something, so plasma might need reload
    }

    fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
        if !src.is_dir() {
            return Err(anyhow!("Source is not a directory"));
        }
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            let new_dst = dst.join(entry.file_name());
            if ty.is_dir() {
                fs::create_dir_all(&new_dst)?;
                Self::copy_dir_recursive(&entry.path(), &new_dst)?;
            } else {
                fs::copy(entry.path(), &new_dst)?;
            }
        }
        Ok(())
    }

    fn reload_plasma(&self) -> Result<()> {
        info!("Reloading Plasma desktop...");
        let output = Command::new("qdbus")
            .args(&[
                "org.kde.plasmashell",
                "/PlasmaShell",
                "org.kde.PlasmaShell.evaluateScript",
                "var Desktops = desktops(); for (i=0;i<Desktops.length;i++) { d = Desktops[i]; d.wallpaperPlugin = 'com.nativis.wallpaper'; }",
            ])
            .output()?;

        if !output.status.success() {
            warn!("Failed to reload plasma: {:?}", output);
        }
        Ok(())
    }
}

impl Platform for KdePlatform {
    fn bootstrap(&mut self) -> Result<()> {
        info!("Bootstrapping KdePlatform...");
        
        // 1. Check and install bundle if necessary
        let installed = self.check_and_install_bundle()?;
        
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
