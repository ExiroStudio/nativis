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

const METADATA_JSON: &[u8] = include_bytes!("../../../../platforms/kde-wayland/metadata.json");
const MANIFEST_TOML: &[u8] = include_bytes!("../../../../platforms/kde-wayland/manifest.toml");
const MAIN_QML: &[u8] = include_bytes!("../../../../platforms/kde-wayland/contents/ui/main.qml");
const CONFIG_QML: &[u8] = include_bytes!("../../../../platforms/kde-wayland/contents/ui/config.qml");
const QMLDIR: &[u8] = include_bytes!("../../../../platforms/kde-wayland/contents/ui/qmldir");
const MAIN_XML: &[u8] = include_bytes!("../../../../platforms/kde-wayland/contents/config/main.xml");
const LIBNATIVISPLUGIN_SO: &[u8] = include_bytes!("../../../../platforms/kde-wayland/contents/ui/libnativisplugin.so");
const LIBNATIVIS_V2_CORE_SO: &[u8] = include_bytes!("../../../../platforms/kde-wayland/contents/ui/libnativis_v2_core.so");

fn install_embedded_bundle(target_dir: &Path) -> Result<()> {
    let ui_dir = target_dir.join("contents/ui");
    let config_dir = target_dir.join("contents/config");
    fs::create_dir_all(&ui_dir)?;
    fs::create_dir_all(&config_dir)?;

    fs::write(target_dir.join("metadata.json"), METADATA_JSON)?;
    fs::write(target_dir.join("manifest.toml"), MANIFEST_TOML)?;
    fs::write(config_dir.join("main.xml"), MAIN_XML)?;

    fs::write(ui_dir.join("main.qml"), MAIN_QML)?;
    fs::write(ui_dir.join("config.qml"), CONFIG_QML)?;
    fs::write(ui_dir.join("qmldir"), QMLDIR)?;
    fs::write(ui_dir.join("libnativisplugin.so"), LIBNATIVISPLUGIN_SO)?;
    fs::write(ui_dir.join("libnativis_v2_core.so"), LIBNATIVIS_V2_CORE_SO)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(ui_dir.join("libnativisplugin.so"), fs::Permissions::from_mode(0o755));
        let _ = fs::set_permissions(ui_dir.join("libnativis_v2_core.so"), fs::Permissions::from_mode(0o755));
    }

    Ok(())
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

        let main_qml = self.installed_plugin_dir.join("contents/ui/main.qml");
        let plugin_so = self.installed_plugin_dir.join("contents/ui/libnativisplugin.so");

        let needs_install = force || !main_qml.exists() || !plugin_so.exists();

        if needs_install {
            info!("Installing KDE Plasma 6 Wayland wallpaper package into {:?}...", self.installed_plugin_dir);
            fs::create_dir_all(&self.installed_plugin_dir)?;

            if bundle_dir.exists() {
                info!("Installing from disk bundle directory: {:?}", bundle_dir);
                copy_dir_all(&bundle_dir, &self.installed_plugin_dir)
                    .context("Failed to copy KDE Wayland wallpaper package from disk")?;
            } else {
                info!("Disk bundle not found. Extracting embedded KDE Wayland wallpaper package into {:?}", self.installed_plugin_dir);
                install_embedded_bundle(&self.installed_plugin_dir)
                    .context("Failed to unpack embedded KDE Wayland wallpaper package")?;
            }

            // Refresh sycoca cache if kbuildsycoca is available
            for sycoca_cmd in &["kbuildsycoca6", "kbuildsycoca5"] {
                if let Ok(status) = Command::new(sycoca_cmd)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                {
                    if status.success() {
                        info!("Refreshed KDE sycoca cache using {}", sycoca_cmd);
                        break;
                    }
                }
            }

            info!("KDE Plasma 6 Wayland package installed successfully.");
            return Ok(true);
        }

        Ok(false)
    }

    fn reload_plasma(&self) -> Result<()> {
        info!("Applying Nativis wallpaper to Plasma 6 via DBus...");
        let script = "var Desktops = desktops(); for (i=0;i<Desktops.length;i++) { d = Desktops[i]; d.wallpaperPlugin = 'org.kde.image'; d.wallpaperPlugin = 'com.nativis.wallpaper'; }";
        
        for qdbus_cmd in &["qdbus-qt6", "qdbus6", "qdbus"] {
            let output = Command::new(qdbus_cmd)
                .args(&[
                    "org.kde.plasmashell",
                    "/PlasmaShell",
                    "org.kde.PlasmaShell.evaluateScript",
                    script,
                ])
                .output();

            match output {
                Ok(out) if out.status.success() => {
                    info!("Plasma 6 wallpaper plugin reloaded successfully using {}.", qdbus_cmd);
                    return Ok(());
                }
                Ok(out) => {
                    warn!("{} reload script returned non-zero: {:?}", qdbus_cmd, out);
                }
                Err(_) => {
                    // Try next command
                }
            }
        }

        warn!("Could not find working qdbus executable (tried qdbus-qt6, qdbus6, qdbus).");
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
