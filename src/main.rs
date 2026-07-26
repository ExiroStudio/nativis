//! Nativis — lightweight native multimedia wallpaper runtime.
//!
//! Parses CLI args, builds the plugin registry, and hands off to the runtime.

use tracing_subscriber::EnvFilter;
use nativis_runtime::{Runtime, RuntimeConfig};
use nativis_plugin::PluginRegistry;

fn main() -> anyhow::Result<()> {
    // Initialize structured logging.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env()
            .add_directive("nativis=debug".parse()?))
        .init();

    // Parse CLI: `nativis <media_uri> [output_index]`
    let args: Vec<String> = std::env::args().collect();
    let media_uri = args.get(1).cloned().unwrap_or_default();
    let output_index = args.get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0usize);

    let config = RuntimeConfig {
        media_uri,
        output_index,
        target_fps: 60,
    };

    // Build plugin registry with built-in backends.
    // New backends are added here without touching the runtime or renderer.
    let mut registry = PluginRegistry::new();

    // Built-in image backend (static plugin).
    registry.register("image_backend", || Box::new(nativis_plugin_image::ImageBackend::new()));

    // Hand off to the runtime conductor.
    Runtime::new(config, registry).run()
}
