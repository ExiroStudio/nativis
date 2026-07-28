use nativis_asset::AssetPath;
use nativis_core::clock::MediaClock;
use nativis_core::resource::ResourceManager;
use nativis_plugin::PluginManager;
use nativis_runtime::{Runtime, RuntimeConfig};
use nativis_plugin_image::ImageBackend;
use nativis_platform_kde::KdePlatform;
use nativis_core::platform::Platform;

fn main() -> anyhow::Result<()> {
    // 0. Single Instance Guard
    let socket_path = "/tmp/nativis.sock";
    use std::os::unix::net::{UnixStream, UnixListener};
    if UnixStream::connect(socket_path).is_ok() {
        eprintln!("Another instance of Nativis is already running. Please terminate it first.");
        std::process::exit(1);
    }
    let _ = std::fs::remove_file(socket_path);
    let _listener = UnixListener::bind(socket_path).expect("Failed to bind lock socket");

    tracing_subscriber::fmt::init();

    // 1. Simple CLI parsing (Nativis V1 just takes the URI as the first arg)
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: nativis <media_uri>");
        std::process::exit(1);
    }
    let uri = &args[1];
    let asset_path = AssetPath::parse(uri)?;

    // 2. Initialize the resource manager and plugin manager
    let resources = ResourceManager::new();
    let mut plugin_manager = PluginManager::new();

    // Register built-in plugins
    plugin_manager.register("image_backend", || Box::new(ImageBackend::new()));

    // 3. Find and instantiate the correct backend
    let mut backend = plugin_manager.open(&asset_path)
        .ok_or_else(|| anyhow::anyhow!("No plugin found for {}", uri))?;

    // 4. Open the media
    let clock = MediaClock::new();
    backend.open(&asset_path, &clock, &resources)?;

    // 5. Initialize the Platform
    let mut platform = KdePlatform::new();
    platform.bootstrap()?;
    
    // Create the frame sink via the platform
    let sink = platform.create_sink(&resources)?;

    // 6. Run the Orchestrator
    let config = RuntimeConfig { target_fps: 60 };
    let runtime = Runtime::new(config);

    runtime.run(backend, sink)?;

    // platform's Drop trait handles cleanup implicitly!
    Ok(())
}
