use anyhow::{anyhow, Context, Result};
use nativis_core::contract::FrameSink;
use nativis_core::platform::Platform;
use nativis_core::resource::ResourceManager;
use nativis_transport_shm::ShmSink;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{info, warn};

pub struct KdePlatform {
    installed_plugin_dir: PathBuf,
    event_sender: Option<nativis_core::platform::PlatformEventSender>,
}

const METADATA_JSON: &[u8] = include_bytes!("../../../../platforms/kde-x11/metadata.json");
const MANIFEST_TOML: &[u8] = include_bytes!("../../../../platforms/kde-x11/manifest.toml");
const MAIN_QML: &[u8] = include_bytes!("../../../../platforms/kde-x11/contents/ui/main.qml");
const QMLDIR: &[u8] = include_bytes!("../../../../platforms/kde-x11/contents/ui/qmldir");
include!(concat!(env!("OUT_DIR"), "/embedded_bundle.rs"));

fn install_embedded_bundle(target_dir: &Path) -> Result<()> {
    let plugin_so = LIBNATIVISPLUGIN_SO.ok_or_else(|| {
        anyhow!(
            "Embedded KDE X11 libnativisplugin.so not compiled into binary; run 'cargo xtask bundle-kde' before building"
        )
    })?;
    let core_so = LIBNATIVIS_V1_CORE_SO.ok_or_else(|| {
        anyhow!(
            "Embedded KDE X11 libnativis_v1_core.so not compiled into binary; run 'cargo xtask bundle-kde' before building"
        )
    })?;

    let ui_dir = target_dir.join("contents/ui");
    fs::create_dir_all(&ui_dir)?;

    fs::write(target_dir.join("metadata.json"), METADATA_JSON)?;
    fs::write(target_dir.join("manifest.toml"), MANIFEST_TOML)?;

    fs::write(ui_dir.join("main.qml"), MAIN_QML)?;
    fs::write(ui_dir.join("qmldir"), QMLDIR)?;
    fs::write(ui_dir.join("libnativisplugin.so"), plugin_so)?;
    fs::write(ui_dir.join("libnativis_v1_core.so"), core_so)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(
            ui_dir.join("libnativisplugin.so"),
            fs::Permissions::from_mode(0o755),
        );
        let _ = fs::set_permissions(
            ui_dir.join("libnativis_v1_core.so"),
            fs::Permissions::from_mode(0o755),
        );
    }

    Ok(())
}

fn file_differs_from_bytes(path: &Path, expected: &[u8]) -> bool {
    match fs::read(path) {
        Ok(actual) => actual.as_slice() != expected,
        Err(_) => true,
    }
}

fn file_differs_from_path(dst: &Path, src: &Path) -> bool {
    match (fs::read(dst), fs::read(src)) {
        (Ok(a), Ok(b)) => a != b,
        _ => true,
    }
}

impl KdePlatform {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let installed_plugin_dir =
            Path::new(&home).join(".local/share/plasma/wallpapers/com.nativis.wallpaper");

        Self {
            installed_plugin_dir,
            event_sender: None,
        }
    }
}

impl Default for KdePlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl KdePlatform {
    fn is_installed_stale(&self, disk_bundle: Option<&Path>) -> bool {
        let checks = [
            "metadata.json",
            "manifest.toml",
            "contents/ui/main.qml",
            "contents/ui/qmldir",
            "contents/ui/libnativisplugin.so",
            "contents/ui/libnativis_v1_core.so",
        ];

        if let Some(bundle_dir) = disk_bundle {
            return checks.iter().any(|rel| {
                file_differs_from_path(&self.installed_plugin_dir.join(rel), &bundle_dir.join(rel))
            });
        }

        if file_differs_from_bytes(
            &self.installed_plugin_dir.join("metadata.json"),
            METADATA_JSON,
        ) || file_differs_from_bytes(
            &self.installed_plugin_dir.join("manifest.toml"),
            MANIFEST_TOML,
        ) || file_differs_from_bytes(
            &self.installed_plugin_dir.join("contents/ui/main.qml"),
            MAIN_QML,
        ) || file_differs_from_bytes(
            &self.installed_plugin_dir.join("contents/ui/qmldir"),
            QMLDIR,
        ) {
            return true;
        }

        if let Some(plugin_so) = LIBNATIVISPLUGIN_SO {
            if file_differs_from_bytes(
                &self
                    .installed_plugin_dir
                    .join("contents/ui/libnativisplugin.so"),
                plugin_so,
            ) {
                return true;
            }
        }
        if let Some(core_so) = LIBNATIVIS_V1_CORE_SO {
            if file_differs_from_bytes(
                &self
                    .installed_plugin_dir
                    .join("contents/ui/libnativis_v1_core.so"),
                core_so,
            ) {
                return true;
            }
        }

        false
    }

    fn check_and_install_bundle(&self, force: bool) -> Result<bool> {
        let bundle_dir = find_bundle_dir();
        let disk_bundle_ready = bundle_dir.join("contents/ui/libnativisplugin.so").exists()
            && bundle_dir
                .join("contents/ui/libnativis_v1_core.so")
                .exists();

        let main_qml = self.installed_plugin_dir.join("contents/ui/main.qml");
        let plugin_so = self
            .installed_plugin_dir
            .join("contents/ui/libnativisplugin.so");

        let stale = if disk_bundle_ready {
            self.is_installed_stale(Some(&bundle_dir))
        } else {
            self.is_installed_stale(None)
        };
        let needs_install = force || !main_qml.exists() || !plugin_so.exists() || stale;

        if needs_install {
            info!(
                "Installing KDE Plasma 5 wallpaper package into {:?} (force={}, stale={})...",
                self.installed_plugin_dir, force, stale
            );
            fs::create_dir_all(&self.installed_plugin_dir)?;

            if disk_bundle_ready {
                info!("Installing from disk bundle directory: {:?}", bundle_dir);
                copy_dir_all(&bundle_dir, &self.installed_plugin_dir)
                    .context("Failed to copy KDE Plasma 5 wallpaper package from disk")?;
            } else {
                info!("Complete disk bundle not found. Extracting embedded KDE Plasma 5 wallpaper package into {:?}", self.installed_plugin_dir);
                install_embedded_bundle(&self.installed_plugin_dir)
                    .context("Failed to unpack embedded KDE Plasma 5 wallpaper package")?;
            }

            // Refresh sycoca cache if kbuildsycoca is available
            for sycoca_cmd in &["kbuildsycoca5", "kbuildsycoca6"] {
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

            info!("KDE Plasma 5 wallpaper package installed successfully.");
            return Ok(true);
        }

        Ok(false)
    }

    fn reload_plasma(&self) -> Result<()> {
        info!("Applying Nativis wallpaper to Plasma 5 via DBus...");
        let script = "var Desktops = desktops(); for (i=0;i<Desktops.length;i++) { d = Desktops[i]; d.wallpaperPlugin = 'org.kde.image'; d.wallpaperPlugin = 'com.nativis.wallpaper'; }";

        for qdbus_cmd in &["qdbus", "qdbus-qt5", "qdbus5"] {
            let output = Command::new(qdbus_cmd)
                .args([
                    "org.kde.plasmashell",
                    "/PlasmaShell",
                    "org.kde.PlasmaShell.evaluateScript",
                    script,
                ])
                .output();

            match output {
                Ok(out) if out.status.success() => {
                    info!(
                        "Plasma 5 wallpaper plugin reloaded successfully using {}.",
                        qdbus_cmd
                    );
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

        warn!("Could not find working qdbus executable (tried qdbus, qdbus-qt5, qdbus5).");
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

    fn set_event_sender(&mut self, sender: nativis_core::platform::PlatformEventSender) {
        let sender_clone = sender.clone();
        std::thread::Builder::new()
            .name("nativis-dbus-screensaver".into())
            .spawn(move || {
                run_screensaver_monitor(sender_clone);
            })
            .ok();

        #[cfg(feature = "x11-fullscreen")]
        {
            let sender_fs = sender.clone();
            std::thread::Builder::new()
                .name("nativis-x11-fullscreen".into())
                .spawn(move || {
                    run_x11_fullscreen_monitor(sender_fs);
                })
                .ok();
        }

        self.event_sender = Some(sender);
    }

    fn create_sink(&self, resources: &ResourceManager) -> Result<Box<dyn FrameSink>> {
        let shm_size = detect_initial_shm_size();
        let sink =
            ShmSink::new("/nativis_shm", shm_size, resources.clone()).map_err(|e| anyhow!(e))?;
        Ok(Box::new(sink))
    }
}

fn detect_initial_shm_size() -> usize {
    // Try to detect primary display resolution via kscreen-doctor
    if let Ok(output) = Command::new("kscreen-doctor").arg("-o").output() {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if let Some(geom_idx) = line.find("Geometry:") {
                    let part = line[geom_idx + "Geometry:".len()..].trim();
                    if let Some(res) = part.split_whitespace().nth(1) {
                        let mut dims = res.split('x');
                        if let (Some(w_str), Some(h_str)) = (dims.next(), dims.next()) {
                            if let (Ok(w), Ok(h)) = (w_str.parse::<usize>(), h_str.parse::<usize>())
                            {
                                if w > 0 && h > 0 {
                                    let size = (w * h * 4) + 4096;
                                    info!(
                                        "Detected display resolution {}x{}; initialized SHM to {} bytes (~{:.1} MB)",
                                        w, h, size, size as f64 / (1024.0 * 1024.0)
                                    );
                                    return size;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // Baseline fallback: 1080p RGBA + header (~8.3 MB)
    let default_size = (1920 * 1080 * 4) + 4096;
    info!(
        "Display resolution auto-detect unavailable; using baseline 1080p SHM size ({} bytes)",
        default_size
    );
    default_size
}

/// Listens for freedesktop ScreenSaver ActiveChanged signals to pause/resume playback
/// when the screen locks or display enters sleep mode (0.00% CPU when occluded).
fn run_screensaver_monitor(sender: nativis_core::platform::PlatformEventSender) {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let child = Command::new("dbus-monitor")
        .arg("--session")
        .arg("type='signal',interface='org.freedesktop.ScreenSaver',member='ActiveChanged'")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();

    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                "Failed to spawn dbus-monitor ({}); lock/screen-off pausing is disabled",
                e
            );
            return;
        }
    };
    {
        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                let trimmed = line.trim();
                if trimmed.contains("boolean true") {
                    info!("Linux ScreenSaver active: pausing desktop wallpaper (0.00% CPU)");
                    let _ = sender.send(nativis_core::platform::PlatformEvent::Pause);
                } else if trimmed.contains("boolean false") {
                    info!("Linux ScreenSaver inactive: resuming desktop wallpaper");
                    let _ = sender.send(nativis_core::platform::PlatformEvent::Resume);
                }
            }
        }
        tracing::warn!("dbus-monitor exited; lock/screen-off pausing has stopped");
        let _ = child.kill();
    }
}

#[cfg(feature = "x11-fullscreen")]
fn run_x11_fullscreen_monitor(sender: nativis_core::platform::PlatformEventSender) {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, EventMask};

    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        warn!("Failed to connect to X11 server for fullscreen detection");
        return;
    };

    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;

    if conn
        .change_window_attributes(
            root,
            &x11rb::protocol::xproto::ChangeWindowAttributesAux::new()
                .event_mask(EventMask::PROPERTY_CHANGE),
        )
        .is_err()
    {
        return;
    }
    let _ = conn.flush();

    let Ok(net_active_cookie) = conn.intern_atom(false, b"_NET_ACTIVE_WINDOW") else {
        return;
    };
    let Ok(net_state_cookie) = conn.intern_atom(false, b"_NET_WM_STATE") else {
        return;
    };
    let Ok(net_fullscreen_cookie) = conn.intern_atom(false, b"_NET_WM_STATE_FULLSCREEN") else {
        return;
    };

    let Ok(net_active) = net_active_cookie.reply().map(|r| r.atom) else {
        return;
    };
    let Ok(net_state) = net_state_cookie.reply().map(|r| r.atom) else {
        return;
    };
    let Ok(net_fullscreen) = net_fullscreen_cookie.reply().map(|r| r.atom) else {
        return;
    };

    let mut is_fullscreen = false;

    loop {
        let Ok(event) = conn.wait_for_event() else {
            break;
        };
        if let x11rb::protocol::Event::PropertyNotify(ev) = event {
            if ev.atom == net_active || ev.atom == net_state {
                let active_win = conn
                    .get_property(false, root, net_active, AtomEnum::WINDOW, 0, 1)
                    .ok()
                    .and_then(|c| c.reply().ok())
                    .and_then(|r| r.value32().and_then(|mut v| v.next()));

                let mut current_fs = false;
                if let Some(win) = active_win {
                    if let Ok(cookie) =
                        conn.get_property(false, win, net_state, AtomEnum::ATOM, 0, 32)
                    {
                        if let Ok(reply) = cookie.reply() {
                            if let Some(atoms) = reply.value32() {
                                current_fs = atoms.into_iter().any(|a| a == net_fullscreen);
                            }
                        }
                    }
                }

                if current_fs != is_fullscreen {
                    is_fullscreen = current_fs;
                    if is_fullscreen {
                        info!(
                            "X11 active window entered fullscreen: pausing wallpaper (0.00% CPU)"
                        );
                        let _ = sender.send(nativis_core::platform::PlatformEvent::Pause);
                    } else {
                        info!("X11 active window exited fullscreen: resuming wallpaper");
                        let _ = sender.send(nativis_core::platform::PlatformEvent::Resume);
                    }
                }
            }
        }
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
