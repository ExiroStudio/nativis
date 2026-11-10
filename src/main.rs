use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use anyhow::{Context, Result};
use crossbeam_channel::{unbounded, Sender};
use parking_lot::RwLock;
use tracing::{error, info, warn};

use nativis_asset::AssetPath;
use nativis_core::clock::MediaClock;
use nativis_core::resource::ResourceManager;
use nativis_ipc::{
    detect_environment, get_socket_path, is_daemon_running, send_command, EngineStatus,
    IpcCommand, IpcResponse, UserConfig,
};
use nativis_plugin::PluginManager;
use nativis_plugin_image::ImageBackend;
use nativis_runtime::{Runtime, RuntimeCommand, RuntimeConfig};

use nativis_core::platform::Platform;

#[cfg(not(target_os = "windows"))]
use nativis_platform_kde::KdePlatform;
#[cfg(not(target_os = "windows"))]
use nativis_platform_kde_wayland::KdeWaylandPlatform;
#[cfg(not(target_os = "windows"))]
use nativis_platform_wayland::WaylandPlatform;
#[cfg(any(not(target_os = "windows"), feature = "video"))]
use nativis_plugin_video::VideoBackend;

#[cfg(target_os = "windows")]
use nativis_platform_windows::WindowsPlatform;

#[cfg(not(target_os = "windows"))]
fn create_platform() -> Box<dyn Platform> {
    let env = detect_environment();
    let de_upper = env.desktop_environment.to_uppercase();
    if de_upper.contains("KDE") {
        if env.is_wayland {
            info!("Selected KDE Wayland Platform Adapter (Native Plasma 6 QML Plugin & POSIX SHM)");
            Box::new(KdeWaylandPlatform::new())
        } else {
            info!("Selected KDE X11 Platform Adapter (Native Plasma 5 QML Plugin & POSIX SHM)");
            Box::new(KdePlatform::new())
        }
    } else if env.is_wayland {
        info!("Selected Generic Wayland Platform Adapter (zwlr_layer_shell_v1, Layer::Background)");
        Box::new(WaylandPlatform::new())
    } else {
        info!("Selected Fallback Platform Adapter (KDE X11)");
        Box::new(KdePlatform::new())
    }
}

#[cfg(target_os = "windows")]
fn create_platform() -> Box<dyn Platform> {
    info!("Selected Windows Platform Adapter (WorkerW Desktop Injection)");
    Box::new(WindowsPlatform::new())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = std::env::args().collect();

    // 1. Check for platform reinstallation
    if args.iter().any(|a| a == "--reinstall-platform") {
        let mut platform = create_platform();
        platform.bootstrap(true)?;
        println!("Platform bootstrapped successfully.");
        return Ok(());
    }

    // 2. CLI commands
    if args.len() > 1 {
        match args[1].as_str() {
            "status" => {
                if is_daemon_running() {
                    match send_command(&IpcCommand::GetStatus) {
                        Ok(IpcResponse::Status(status)) => {
                            println!("🌌 Nativis Engine Status:");
                            println!("  Running:      {}", status.is_running);
                            println!("  Paused:       {}", status.is_paused);
                            println!("  Wallpaper:    {}", status.active_wallpaper.as_deref().unwrap_or("None"));
                            println!("  Target FPS:   {}", status.target_fps);
                            println!("  Active Sink:  {}", status.active_sink);
                            println!("  Platform:     {}", status.platform);
                        }
                        other => println!("Daemon response: {:?}", other),
                    }
                } else {
                    let env = detect_environment();
                    let cfg = UserConfig::load();
                    println!("🌌 Nativis (Daemon Offline)");
                    println!("  OS:           {}", env.os_release);
                    println!("  DE:           {}", env.desktop_environment);
                    println!("  Display:      {}", env.display_server);
                    println!("  Active Sink:  {}", env.active_sink_name);
                    println!("  Saved Config: {}", UserConfig::config_path().display());
                    println!("  Wallpaper:    {}", cfg.active_wallpaper);
                }
                return Ok(());
            }
            "pause" => {
                match send_command(&IpcCommand::Pause) {
                    Ok(_) => println!("Wallpaper paused."),
                    Err(e) => eprintln!("Error sending pause command: {}", e),
                }
                return Ok(());
            }
            "resume" => {
                match send_command(&IpcCommand::Resume) {
                    Ok(_) => println!("Wallpaper resumed."),
                    Err(e) => eprintln!("Error sending resume command: {}", e),
                }
                return Ok(());
            }
            "stop" | "--stop" | "shutdown" => {
                if is_daemon_running() {
                    match send_command(&IpcCommand::Shutdown) {
                        Ok(_) => println!("✓ Nativis daemon stopped."),
                        Err(e) => eprintln!("Error sending shutdown command: {}", e),
                    }
                } else {
                    println!("Nativis daemon is not running.");
                }
                return Ok(());
            }
            "restart" => {
                if is_daemon_running() {
                    let _ = send_command(&IpcCommand::Shutdown);
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                let cfg = UserConfig::load();
                ensure_daemon_running(Some(&cfg.active_wallpaper))?;
                println!("✓ Nativis daemon restarted.");
                return Ok(());
            }
            "help" | "--help" | "-h" => {
                println!("🌌 Nativis Wallpaper Engine");
                println!("Usage:");
                println!("  nativis                 Launch the modern UI");
                println!("  nativis --no-gui        Start headless daemon using saved config");
                println!("  nativis <path/to/media> Set wallpaper directly");
                println!("  nativis status          Show engine status");
                println!("  nativis pause           Pause live playback");
                println!("  nativis resume          Resume live playback");
                println!("  nativis stop            Stop background daemon");
                println!("  nativis restart         Restart background daemon");
                println!("  nativis --help          Show this help message");
                return Ok(());
            }
            "--no-gui" | "no-gui" | "--daemon" | "daemon" => {
                let cfg = UserConfig::load();
                let initial_wallpaper = args.get(2).map(|s| s.as_str()).unwrap_or(&cfg.active_wallpaper);
                info!("Starting Nativis background daemon (--no-gui) with wallpaper: {}", initial_wallpaper);
                return run_daemon(Some(initial_wallpaper));
            }
            "--ui" | "ui" => {
                let cfg = UserConfig::load();
                ensure_daemon_running(Some(&cfg.active_wallpaper))?;
                return nativis_ui::run_ui();
            }
            _ => {
                let target_media = &args[1];
                if is_daemon_running() {
                    info!("Daemon is already running. Sending SetWallpaper IPC command...");
                    match send_command(&IpcCommand::SetWallpaper {
                        uri: target_media.clone(),
                        monitor: None,
                        scaling: None,
                    }) {
                        Ok(IpcResponse::Ok { message }) => {
                            println!("✓ {}", message);
                            return Ok(());
                        }
                        Ok(IpcResponse::Error { message }) => {
                            eprintln!("✗ Error setting wallpaper: {}", message);
                            return Ok(());
                        }
                        Ok(other) => {
                            println!("Daemon response: {:?}", other);
                            return Ok(());
                        }
                        Err(e) => {
                            warn!("Failed to communicate with running daemon ({}). Falling back to direct run.", e);
                        }
                    }
                } else {
                    let mut cfg = UserConfig::load();
                    cfg.active_wallpaper = target_media.clone();
                    let _ = cfg.save();
                    return run_daemon(Some(target_media));
                }
            }
        }
    }

    // Default with no arguments: Load saved user configuration and launch Modern UI!
    let cfg = UserConfig::load();
    info!("No arguments provided. Launching Nativis Modern UI with saved wallpaper: {}", cfg.active_wallpaper);
    ensure_daemon_running(Some(&cfg.active_wallpaper))?;
    nativis_ui::run_ui()
}

/// Ensures the daemon is running; if not, spawns it as a detached background process.
fn ensure_daemon_running(default_wallpaper: Option<&str>) -> Result<()> {
    if !is_daemon_running() {
        info!("Spawning background Nativis Engine daemon process...");
        let current_exe = std::env::current_exe()?;
        let mut cmd = std::process::Command::new(current_exe);
        cmd.arg("--no-gui");
        if let Some(wp) = default_wallpaper {
            cmd.arg(wp);
        }
        cmd.stdin(std::process::Stdio::null())
           .stdout(std::process::Stdio::null())
           .stderr(std::process::Stdio::inherit());

        cmd.spawn().context("Failed to spawn background daemon process")?;

        // Give the daemon a moment to bind the IPC socket
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            if is_daemon_running() {
                info!("Nativis daemon is online and responding via IPC.");
                break;
            }
        }
    }
    Ok(())
}

/// Runs the Nativis engine daemon with IPC listener and dynamic wallpaper orchestrator.
fn run_daemon(initial_wallpaper: Option<&str>) -> Result<()> {
    info!("Bootstrapping Nativis Daemon Engine...");

    let resources = Arc::new(ResourceManager::new());
    let current_wallpaper = Arc::new(RwLock::new(initial_wallpaper.map(|s| s.to_string())));
    let is_paused = Arc::new(AtomicBool::new(false));

    // Channel for dynamic conductor commands
    let (command_tx, command_rx) = unbounded::<RuntimeCommand>();

    // 1. Initialize Platform and Sink (keep platform alive for daemon lifetime)
    let mut platform = create_platform();
    platform.bootstrap(false)?;
    let sink = platform.create_sink(&resources)?;

    // 2. Start IPC Server Listener
    let _ipc_handle = start_ipc_server(
        command_tx.clone(),
        resources.clone(),
        current_wallpaper.clone(),
        is_paused.clone(),
    );

    // 3. Initial media backend
    let initial_uri = initial_wallpaper.unwrap_or("arlenchino.mp4");
    let asset_path = AssetPath::parse(initial_uri).unwrap_or_else(|_| AssetPath::parse("CuteBunny.jpg").unwrap());

    let mut plugin_manager = PluginManager::new();
    plugin_manager.register("image_backend", || Box::new(ImageBackend::new()));
    #[cfg(any(not(target_os = "windows"), feature = "video"))]
    plugin_manager.register("video_backend", || Box::new(VideoBackend::new()));

    let mut backend = plugin_manager
        .open(&asset_path)
        .or_else(|| {
            let fallback = AssetPath::parse("CuteBunny.jpg").ok()?;
            plugin_manager.open(&fallback)
        })
        .ok_or_else(|| anyhow::anyhow!("No plugin found for initial wallpaper: {}", initial_uri))?;

    let clock = MediaClock::new();
    if let Err(e) = backend.open(&asset_path, &clock, &resources) {
        warn!(
            "Failed to open initial wallpaper '{}': {}. Falling back to default static image.",
            initial_uri, e
        );
        let fallback = AssetPath::parse("CuteBunny.jpg")?;
        let mut fallback_backend = plugin_manager
            .open(&fallback)
            .ok_or_else(|| anyhow::anyhow!("No plugin found for fallback image"))?;
        fallback_backend.open(&fallback, &clock, &resources)?;
        backend = fallback_backend;
        *current_wallpaper.write() = Some("CuteBunny.jpg".to_string());
    }

    info!("Starting runtime conductor loop with IPC command channel...");
    let config = RuntimeConfig { target_fps: 60 };
    let runtime = Runtime::new(config);

    runtime.run_with_commands(backend, sink, command_rx)?;

    Ok(())
}

#[cfg(unix)]
fn start_ipc_server(
    command_tx: Sender<RuntimeCommand>,
    resources: Arc<ResourceManager>,
    current_wallpaper: Arc<RwLock<Option<String>>>,
    is_paused: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let socket_path = get_socket_path();
        let _ = std::fs::remove_file(&socket_path);

        use std::os::unix::net::UnixListener;
        let listener = match UnixListener::bind(&socket_path) {
            Ok(l) => l,
            Err(e) => {
                error!("Failed to bind IPC socket at {:?}: {}", socket_path, e);
                return;
            }
        };

        info!("Nativis IPC server listening at {:?}", socket_path);

        for stream in listener.incoming() {
            if let Ok(mut stream) = stream {
                let cmd_tx = command_tx.clone();
                let res = resources.clone();
                let cur_wp = current_wallpaper.clone();
                let paused = is_paused.clone();

                std::thread::spawn(move || {
                    use std::io::{BufRead, BufReader, Write};
                    let mut reader = BufReader::new(&stream);
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_ok() {
                        if let Ok(cmd) = serde_json::from_str::<IpcCommand>(&line) {
                            let resp = handle_ipc_command(cmd, &cmd_tx, &res, &cur_wp, &paused);
                            if let Ok(resp_json) = serde_json::to_string(&resp) {
                                let _ = stream.write_all(resp_json.as_bytes());
                                let _ = stream.write_all(b"\n");
                                let _ = stream.flush();
                            }
                        }
                    }
                });
            }
        }
    })
}

#[cfg(windows)]
fn start_ipc_server(
    command_tx: Sender<RuntimeCommand>,
    resources: Arc<ResourceManager>,
    current_wallpaper: Arc<RwLock<Option<String>>>,
    is_paused: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        use std::os::windows::ffi::OsStrExt;
        use std::os::windows::io::FromRawHandle;
        use std::io::{BufRead, BufReader, Write};
        use windows_sys::Win32::System::Pipes::*;
        use windows_sys::Win32::Storage::FileSystem::*;
        use windows_sys::Win32::Foundation::*;

        let pipe_path = get_socket_path();
        let wide_path: Vec<u16> = pipe_path.as_os_str().encode_wide().chain(Some(0)).collect();

        info!("Nativis Windows IPC Named Pipe server listening at {:?}", pipe_path);

        loop {
            let handle = unsafe {
                CreateNamedPipeW(
                    wide_path.as_ptr(),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    PIPE_UNLIMITED_INSTANCES,
                    4096,
                    4096,
                    0,
                    std::ptr::null(),
                )
            };

            if handle == INVALID_HANDLE_VALUE {
                error!("Failed to create Windows Named Pipe at {:?}: error code {}", pipe_path, unsafe { GetLastError() });
                std::thread::sleep(std::time::Duration::from_millis(500));
                continue;
            }

            let connect_res = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
            let connected = connect_res != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;

            if !connected {
                unsafe { CloseHandle(handle); }
                continue;
            }

            let mut stream = unsafe { std::fs::File::from_raw_handle(handle as *mut _) };
            let cmd_tx = command_tx.clone();
            let res = resources.clone();
            let cur_wp = current_wallpaper.clone();
            let paused = is_paused.clone();

            std::thread::spawn(move || {
                let stream_reader = match stream.try_clone() {
                    Ok(f) => f,
                    Err(_) => return,
                };
                let mut reader = BufReader::new(stream_reader);
                let mut line = String::new();
                if reader.read_line(&mut line).is_ok() {
                    if let Ok(cmd) = serde_json::from_str::<IpcCommand>(&line) {
                        let resp = handle_ipc_command(cmd, &cmd_tx, &res, &cur_wp, &paused);
                        if let Ok(resp_json) = serde_json::to_string(&resp) {
                            let _ = stream.write_all(resp_json.as_bytes());
                            let _ = stream.write_all(b"\n");
                            let _ = stream.flush();
                        }
                    }
                }
            });
        }
    })
}

#[cfg(all(not(unix), not(windows)))]
fn start_ipc_server(
    _command_tx: Sender<RuntimeCommand>,
    _resources: Arc<ResourceManager>,
    _current_wallpaper: Arc<RwLock<Option<String>>>,
    _is_paused: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(|| {})
}

fn handle_ipc_command(
    cmd: IpcCommand,
    command_tx: &Sender<RuntimeCommand>,
    resources: &ResourceManager,
    current_wallpaper: &RwLock<Option<String>>,
    is_paused: &AtomicBool,
) -> IpcResponse {
    match cmd {
        IpcCommand::SetWallpaper { uri, .. } => {
            info!("IPC request: SetWallpaper -> {}", uri);
            let asset_path = match AssetPath::parse(&uri) {
                Ok(p) => p,
                Err(e) => return IpcResponse::Error { message: format!("Invalid URI {}: {}", uri, e) },
            };

            let mut plugin_manager = PluginManager::new();
            plugin_manager.register("image_backend", || Box::new(ImageBackend::new()));
            #[cfg(not(target_os = "windows"))]
            plugin_manager.register("video_backend", || Box::new(VideoBackend::new()));

            let mut backend = match plugin_manager.open(&asset_path) {
                Some(b) => b,
                None => return IpcResponse::Error { message: format!("No plugin found for {}", uri) },
            };

            let clock = MediaClock::new();
            if let Err(e) = backend.open(&asset_path, &clock, resources) {
                return IpcResponse::Error { message: format!("Failed to open backend: {}", e) };
            }

            *current_wallpaper.write() = Some(uri.clone());
            let _ = command_tx.send(RuntimeCommand::SetBackend(backend));

            // Persist wallpaper choice in user config
            let mut cfg = UserConfig::load();
            cfg.active_wallpaper = uri.clone();
            let _ = cfg.save();

            IpcResponse::Ok { message: format!("Wallpaper set to {}", uri) }
        }
        IpcCommand::Pause => {
            is_paused.store(true, Ordering::SeqCst);
            let _ = command_tx.send(RuntimeCommand::SetPause(true));
            IpcResponse::Ok { message: "Paused".into() }
        }
        IpcCommand::Resume => {
            is_paused.store(false, Ordering::SeqCst);
            let _ = command_tx.send(RuntimeCommand::SetPause(false));
            IpcResponse::Ok { message: "Resumed".into() }
        }
        IpcCommand::TogglePause => {
            let next = !is_paused.load(Ordering::SeqCst);
            is_paused.store(next, Ordering::SeqCst);
            let _ = command_tx.send(RuntimeCommand::SetPause(next));
            IpcResponse::Ok { message: if next { "Paused".into() } else { "Resumed".into() } }
        }
        IpcCommand::SetSpeed { speed } => {
            let mut cfg = UserConfig::load();
            cfg.playback_speed = speed;
            let _ = cfg.save();
            IpcResponse::Ok { message: format!("Speed set to {}", speed) }
        }
        IpcCommand::SetVolume { volume } => {
            let mut cfg = UserConfig::load();
            cfg.volume = volume;
            let _ = cfg.save();
            IpcResponse::Ok { message: format!("Volume set to {}", volume) }
        }
        IpcCommand::GetStatus => {
            let wp = current_wallpaper.read().clone();
            let env = detect_environment();
            let cfg = UserConfig::load();
            IpcResponse::Status(EngineStatus {
                is_running: true,
                is_paused: is_paused.load(Ordering::SeqCst),
                active_wallpaper: wp,
                target_fps: cfg.target_fps,
                playback_speed: cfg.playback_speed,
                volume: cfg.volume,
                active_sink: env.active_sink_name,
                platform: env.desktop_environment,
            })
        }
        IpcCommand::GetConfig => {
            IpcResponse::Config(UserConfig::load())
        }
        IpcCommand::SaveConfig(new_cfg) => {
            let _ = new_cfg.save();
            IpcResponse::Ok { message: "Config saved".into() }
        }
        IpcCommand::ListWallpapers => {
            IpcResponse::Wallpapers(vec![])
        }
        IpcCommand::ListPlugins => {
            IpcResponse::Plugins(nativis_ipc::get_default_plugins())
        }
        IpcCommand::InstallPlugin { name } => {
            IpcResponse::Ok { message: format!("Plugin '{}' installation requested", name) }
        }
        IpcCommand::GetEnvironment => {
            IpcResponse::Environment(detect_environment())
        }
        IpcCommand::Shutdown => {
            let _ = command_tx.send(RuntimeCommand::Stop);
            IpcResponse::Ok { message: "Shutdown initiated".into() }
        }
    }
}
