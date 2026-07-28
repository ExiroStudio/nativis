use nativis_asset::AssetPath;
use nativis_core::clock::MediaClock;
use nativis_core::resource::ResourceManager;
use nativis_plugin::PluginManager;
use nativis_runtime::{Runtime, RuntimeConfig};
use nativis_transport_shm::ShmSink;
use nativis_plugin_image::ImageBackend;

fn main() -> anyhow::Result<()> {
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

    // 5. Initialize the frame sink (Transport layer)
    // Create an SHM region big enough for a 4K frame (3840 * 2160 * 4 bytes)
    // In a real system, the sink size might be dynamic based on the first frame.
    let shm_size = 3840 * 2160 * 4;
    let sink = Box::new(ShmSink::new("/nativis_shm", shm_size, resources).map_err(|e| anyhow::anyhow!(e))?);

    // 6. Run the Orchestrator
    let config = RuntimeConfig { target_fps: 60 };
    let runtime = Runtime::new(config);

    runtime.run(backend, sink)?;

    Ok(())
}
