use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use anyhow::Result;
use crossbeam_channel::{unbounded, Sender};
use parking_lot::RwLock;
use tracing::{error, info, warn};

use nativis_asset::AssetPath;
use nativis_core::clock::MediaClock;
use nativis_core::resource::ResourceManager;
use nativis_ipc::{
    detect_environment, get_socket_path, is_daemon_running, send_command, EngineStatus,
    IpcCommand, IpcResponse,
};
use nativis_plugin::PluginManager;
use nativis_plugin_image::ImageBackend;
use nativis_runtime::{Runtime, RuntimeCommand, RuntimeConfig};

#[cfg(not(target_os = "windows"))]
use nativis_platform_kde::KdePlatform;
#[cfg(not(target_os = "windows"))]
use nativis_plugin_video::VideoBackend;

#[cfg(not(target_os = "windows"))]
use nativis_core::platform::Platform;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = std::env::args().collect();

    // 1. Check for platform reinstallation
    if args.iter().any(|a| a == "--reinstall-platform") {
        #[cfg(not(target_os = "windows"))]
        {
            let mut platform = KdePlatform::new();
            platform.bootstrap(true)?;
            println!("Platform reinstalled successfully.");
            return Ok(());
        }
        #[cfg(target_os = "windows")]
        {
            println!("Windows platform bootstrap is not required.");
            return Ok(());
        }
    }

    // 2. CLI quick commands: status, pause, resume
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
                    println!("🌌 Nativis (Daemon Offline)");
                    println!("  OS:           {}", env.os_release);
                    println!("  DE:           {}", env.desktop_environment);
                    println!("  Display:      {}", env.display_server);
                    println!("  Active Sink:  {}", env.active_sink_name);
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
            "--daemon" | "daemon" => {
                let initial_wallpaper = args.get(2).map(|s| s.as_str());
                return run_daemon(initial_wallpaper);
            }
            "--ui" | "ui" => {
                // If daemon is not running, start it in background thread
                ensure_daemon_running(None)?;
                return nativis_ui::run_ui();
            }
            _ => {
                let target_media = &args[1];
                // If a path/URI was passed, check if daemon is running
                if is_daemon_running() {
                    info!("Daemon is already running. Sending SetWallpaper IPC command...");
                    match send_command(&IpcCommand::SetWallpaper {
                        uri: target_media.clone(),
                        monitor: None,
                        scaling: None,
                    }) {
                        Ok(_) => {
                            println!("✓ Wallpaper updated to: {}", target_media);
                            return Ok(());
                        }
                        Err(e) => {
                            warn!("Failed to communicate with running daemon ({}). Falling back to direct run.", e);
                        }
                    }
                } else {
                    // Daemon not running, run daemon with this media
                    return run_daemon(Some(target_media));
                }
            }
        }
    }

    // Default with no arguments: Launch Modern UI!
    info!("No arguments provided. Launching Nativis Modern UI...");
    ensure_daemon_running(Some("arlenchino.mp4"))?;
    nativis_ui::run_ui()
}

/// Ensures the daemon is running; if not, spawns it on a background thread.
fn ensure_daemon_running(default_wallpaper: Option<&str>) -> Result<()> {
    if !is_daemon_running() {
        info!("Starting background Nativis Engine daemon thread...");
        let wp_string = default_wallpaper.map(|s| s.to_string());
        std::thread::spawn(move || {
            if let Err(e) = run_daemon(wp_string.as_deref()) {
                error!("Daemon error: {}", e);
            }
        });

        // Give the daemon a moment to bind the IPC socket
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            if is_daemon_running() {
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

    // 1. Initialize Platform Sink
    #[cfg(not(target_os = "windows"))]
    let sink = {
        let mut platform = KdePlatform::new();
        platform.bootstrap(false)?;
        platform.create_sink(&resources)?
    };

    #[cfg(target_os = "windows")]
    let sink = {
        panic!("Windows platform is not fully implemented yet");
    };

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
    #[cfg(not(target_os = "windows"))]
    plugin_manager.register("video_backend", || Box::new(VideoBackend::new()));

    let mut backend = plugin_manager
        .open(&asset_path)
        .or_else(|| {
            // Fallback to image if video not found
            let fallback = AssetPath::parse("CuteBunny.jpg").ok()?;
            plugin_manager.open(&fallback)
        })
        .ok_or_else(|| anyhow::anyhow!("No plugin found for initial wallpaper: {}", initial_uri))?;

    let clock = MediaClock::new();
    backend.open(&asset_path, &clock, &resources)?;

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

#[cfg(not(unix))]
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
            IpcResponse::Ok { message: format!("Speed set to {}", speed) }
        }
        IpcCommand::SetVolume { volume } => {
            IpcResponse::Ok { message: format!("Volume set to {}", volume) }
        }
        IpcCommand::GetStatus => {
            let wp = current_wallpaper.read().clone();
            let env = detect_environment();
            IpcResponse::Status(EngineStatus {
                is_running: true,
                is_paused: is_paused.load(Ordering::SeqCst),
                active_wallpaper: wp,
                target_fps: 60,
                playback_speed: 1.0,
                volume: 0.0,
                active_sink: env.active_sink_name,
                platform: env.desktop_environment,
            })
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
