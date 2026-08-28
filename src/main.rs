use nativis_asset::AssetPath;
use nativis_core::clock::MediaClock;
use nativis_core::resource::ResourceManager;
use nativis_plugin::PluginManager;
use nativis_runtime::{Runtime, RuntimeConfig};
use nativis_plugin_image::ImageBackend;

#[cfg(not(target_os = "windows"))]
use nativis_plugin_video::VideoBackend;

#[cfg(not(target_os = "windows"))]
use nativis_platform_kde::KdePlatform;

#[cfg(not(target_os = "windows"))]
use nativis_core::platform::Platform;

fn main() -> anyhow::Result<()> {
    // 0. Single Instance Guard (Unix only)
    #[cfg(unix)]
    {
        let socket_path = "/tmp/nativis.sock";
        use std::os::unix::net::{UnixStream, UnixListener};
        if UnixStream::connect(socket_path).is_ok() {
            eprintln!("Another instance of Nativis is already running. Please terminate it first.");
            std::process::exit(1);
        }
        let _ = std::fs::remove_file(socket_path);
        let _listener = UnixListener::bind(socket_path).expect("Failed to bind lock socket");
        // Keep listener alive by leaking or storing it?
        // Actually, leaking it is fine for the single instance guard in this simple app.
        Box::leak(Box::new(_listener));
    }

    tracing_subscriber::fmt::init();

    // 1. Simple CLI parsing
    let args: Vec<String> = std::env::args().collect();
    let force_reinstall = args.iter().any(|a| a == "--reinstall-platform");

    #[cfg(not(target_os = "windows"))]
    if force_reinstall {
        let mut platform = KdePlatform::new();
        platform.bootstrap(true)?;
        println!("Platform reinstalled successfully.");
        return Ok(());
    }

    if args.len() < 2 {
        eprintln!("Usage: nativis <media_uri> or nativis --reinstall-platform");
        std::process::exit(1);
    }
    let uri = &args[1];
    let asset_path = AssetPath::parse(uri)?;

    // 2. Initialize the resource manager and plugin manager
    let resources = ResourceManager::new();
    let mut plugin_manager = PluginManager::new();

    // Register built-in plugins
    plugin_manager.register("image_backend", || Box::new(ImageBackend::new()));
    
    #[cfg(not(target_os = "windows"))]
    plugin_manager.register("video_backend", || Box::new(VideoBackend::new()));

    // 3. Find and instantiate the correct backend
    let mut backend = plugin_manager.open(&asset_path)
        .ok_or_else(|| anyhow::anyhow!("No plugin found for {}", uri))?;

    // 4. Open the media
    let clock = MediaClock::new();
    backend.open(&asset_path, &clock, &resources)?;

    // 5. Initialize the Platform
    #[cfg(not(target_os = "windows"))]
    let sink = {
        let mut platform = KdePlatform::new();
        platform.bootstrap(false)?;
        platform.create_sink(&resources)?
    };

    #[cfg(target_os = "windows")]
    let sink = {
        // Dummy sink for Windows for now, since native windowing isn't built yet
        panic!("Windows platform is not fully implemented yet");
    };

    // 6. Run the Orchestrator
    let config = RuntimeConfig { target_fps: 60 };
    let runtime = Runtime::new(config);

    runtime.run(backend, sink)?;

    Ok(())
}
