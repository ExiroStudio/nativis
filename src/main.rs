use nativis_engine::{Engine, EngineConfig};
use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    // ── Logging ───────────────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,nativis=debug")),
        )
        .with_target(true)
        .init();

    // ── CLI args ──────────────────────────────────────────────────────────
    let mut config = EngineConfig::default();

    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--wallpaper" | "-w" => {
                i += 1;
                if let Some(path) = args.get(i) {
                    config.wallpaper_path = path.clone();
                }
            }
            "--output" | "-o" => {
                i += 1;
                if let Some(idx) = args.get(i) {
                    config.output_index = idx.parse().unwrap_or(0);
                }
            }
            "--fps" => {
                i += 1;
                if let Some(fps) = args.get(i) {
                    config.target_fps = fps.parse().unwrap_or(60);
                }
            }
            "--help" | "-h" => {
                eprintln!(
                    "Nativis Engine v{}\n\
                     Usage: nativis [OPTIONS]\n\n\
                     Options:\n\
                       -w, --wallpaper <path>   Path to image or video wallpaper\n\
                       -o, --output   <index>   Monitor output index (default: 0)\n\
                       --fps          <fps>     Target frame rate (default: 60)\n\
                       -h, --help               Print this help\n",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(());
            }
            _ => {}
        }
        i += 1;
    }

    tracing::info!(
        wallpaper = %config.wallpaper_path,
        output    = config.output_index,
        fps       = config.target_fps,
        "Starting Nativis Engine"
    );

    // ── Boot ──────────────────────────────────────────────────────────────
    Engine::new(config).run()
}
