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
        let bundle_dir = Path::new("./platforms/kde-x11");
        if !bundle_dir.exists() {
            warn!("Bundle directory not found at {:?}", bundle_dir);
            return Ok(false);
        }

        let mut installed_something = false;

        // 1. Install System QML Plugin (Requires sudo)
        let system_qml_target = Path::new("/usr/lib/x86_64-linux-gnu/qt5/qml/org/nativis");
        let system_plugin_so = system_qml_target.join("libnativisplugin.so");
        let bundle_system_qml = bundle_dir.join("system-qml/org/nativis");
        
        let needs_install = !system_plugin_so.exists() || {
            let qmldir_path = system_qml_target.join("qmldir");
            qmldir_path.exists() && std::fs::read_to_string(qmldir_path).unwrap_or_default().contains(".")
        };

        if bundle_system_qml.exists() && needs_install {
            println!("============================================================");
            println!("Nativis needs to install the C++ QML plugin into the system");
            println!("Qt directory to integrate with KDE Plasma Settings.");
            println!("The following commands will be executed with sudo:");
            
            let parent_dir = "/usr/lib/x86_64-linux-gnu/qt5/qml/org";
            println!("    sudo mkdir -p {}", parent_dir);
            println!("    sudo cp -r {} {}", bundle_system_qml.display(), parent_dir);
            println!("============================================================");
            
            let status1 = Command::new("sudo")
                .args(&["mkdir", "-p", parent_dir])
                .status()?;
                
            let status2 = Command::new("sudo")
                .args(&["cp", "-r", bundle_system_qml.to_str().unwrap(), parent_dir])
                .status()?;

            if !status1.success() || !status2.success() {
                warn!("Failed to install system QML plugin.");
            } else {
                info!("Successfully installed system QML plugin.");
                installed_something = true;
            }
        }

        // 2. Install KDE Plasma Wallpaper Package (User Space)
        info!("Installing KDE Plasma wallpaper package using kpackagetool5...");
        let _ = Command::new("kpackagetool5")
            .args(&["-t", "Plasma/Wallpaper", "-r", "com.nativis.wallpaper"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        let status = Command::new("kpackagetool5")
            .args(&["-t", "Plasma/Wallpaper", "-i", bundle_dir.to_str().unwrap()])
            .status()?;
            
        if status.success() {
            info!("KDE Plasma wallpaper package installed.");
            installed_something = true;
        } else {
            warn!("Failed to install KDE Plasma wallpaper package.");
        }

        // Update KDE System Configuration Cache (sycoca) so Plasma recognizes the new wallpaper plugin
        if installed_something {
            let _ = Command::new("kbuildsycoca5")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }

        Ok(installed_something)
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
