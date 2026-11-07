use anyhow::{Context, Result};
use tracing::info;
use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::{wl_compositor, wl_output, wl_region, wl_registry, wl_surface};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::render::WaylandSurfaceHandle;

/// Detects the best layer for a wallpaper surface based on what the compositor exposes.
///
/// # Layer selection rationale
///
/// | Layer              | Z-order in KWin                                         |
/// |--------------------|----------------------------------------------------------|
/// | `Layer::Background`| Below plasma desktop icons — correct for KDE Plasma     |
/// | `Layer::Bottom`    | Above plasma desktop icons — covers them (wrong for KDE)|
///
/// KDE Plasma desktop icons are rendered by `plasmashell` using `org_kde_plasma_shell`
/// with role "Desktop". KWin stacks this ABOVE `Layer::Background` but BELOW `Layer::Bottom`.
///
/// When `org_kde_plasma_shell` is present in the registry we know this is KDE (or a
/// KWin-compatible compositor). We use `Layer::Background` so nativis sits:
///   [above]  KDE's own wallpaper plugin (also at Background, but created earlier)
///   [below]  KDE desktop icons / folder view
///
/// For every other compositor (Sway, Hyprland, River, Wayfire…) we use `Layer::Bottom`
/// which is the conventional layer for interactive wallpaper apps on wlroots-based stacks.
fn detect_wallpaper_layer(globals: &GlobalList) -> zwlr_layer_shell_v1::Layer {
    // `org_kde_plasma_shell` is advertised exclusively by KDE's KWin compositor.
    let is_kde = globals
        .contents()
        .clone_list()
        .iter()
        .any(|g| g.interface == "org_kde_plasma_shell");

    if is_kde {
        info!(
            "KDE compositor detected (org_kde_plasma_shell present) — \
             using Layer::Background so desktop icons remain visible above nativis"
        );
        zwlr_layer_shell_v1::Layer::Background
    } else {
        info!(
            "Non-KDE compositor detected — \
             using Layer::Bottom (standard wlroots wallpaper layer)"
        );
        zwlr_layer_shell_v1::Layer::Bottom
    }
}

pub struct WaylandShellState {
    pub configured: bool,
    pub width: u32,
    pub height: u32,
    pub running: bool,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandShellState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(WaylandShellState: ignore wl_compositor::WlCompositor);
delegate_noop!(WaylandShellState: ignore wl_surface::WlSurface);
delegate_noop!(WaylandShellState: ignore wl_region::WlRegion);
delegate_noop!(WaylandShellState: ignore wl_output::WlOutput);
delegate_noop!(WaylandShellState: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for WaylandShellState {
    fn event(
        state: &mut Self,
        surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                // ack_configure must be sent before the next wl_surface.commit()
                surface.ack_configure(serial);
                state.configured = true;
                if width > 0 {
                    state.width = width;
                }
                if height > 0 {
                    state.height = height;
                }
            }
            zwlr_layer_surface_v1::Event::Closed => {
                state.running = false;
            }
            _ => {}
        }
    }
}

pub struct WaylandShell {
    pub conn: Connection,
    pub event_queue: EventQueue<WaylandShellState>,
    pub state: WaylandShellState,
    pub surface: wl_surface::WlSurface,
    pub layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    /// Which layer was selected at init time (for diagnostics / status reporting)
    pub active_layer: zwlr_layer_shell_v1::Layer,
}

impl WaylandShell {
    pub fn new() -> Result<Self> {
        let conn = Connection::connect_to_env()
            .context("Failed to connect to Wayland display")?;

        let (globals, mut event_queue) = registry_queue_init::<WaylandShellState>(&conn)
            .context("Failed to initialize Wayland registry")?;

        let qh = event_queue.handle();

        let compositor: wl_compositor::WlCompositor = globals
            .bind(&qh, 1..=6, ())
            .context("Compositor does not support wl_compositor")?;

        let layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1 = globals
            .bind(&qh, 1..=4, ())
            .context("Compositor does not support zwlr_layer_shell_v1")?;

        let surface = compositor.create_surface(&qh, ());

        // Empty input region = full click-through passthrough (nativis never steals input)
        let empty_region = compositor.create_region(&qh, ());
        surface.set_input_region(Some(&empty_region));
        // Destroy region immediately — compositor already captured the state
        empty_region.destroy();

        // Auto-select the correct layer for this compositor
        let active_layer = detect_wallpaper_layer(&globals);

        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            None, // None = apply to all outputs
            active_layer,
            "nativis-wallpaper".to_string(),
            &qh,
            (),
        );

        // Fill the entire output (anchor to all four edges, no exclusive zone)
        let all_anchors = zwlr_layer_surface_v1::Anchor::Top
            | zwlr_layer_surface_v1::Anchor::Bottom
            | zwlr_layer_surface_v1::Anchor::Left
            | zwlr_layer_surface_v1::Anchor::Right;

        layer_surface.set_anchor(all_anchors);
        // -1 = don't push panels/docks; we render behind everything that matters
        layer_surface.set_exclusive_zone(-1);
        layer_surface.set_keyboard_interactivity(
            zwlr_layer_surface_v1::KeyboardInteractivity::None,
        );

        // --- Layer shell two-commit handshake ---
        // 1. First commit: tells the compositor to send us a `configure` event with
        //    the actual output dimensions and a serial we must acknowledge.
        surface.commit();

        let mut state = WaylandShellState {
            configured: false,
            width: 1920,
            height: 1080,
            running: true,
        };

        // First roundtrip: blocks until configure is received and ack_configure is sent
        // (ack_configure happens inside the Dispatch impl above)
        event_queue.roundtrip(&mut state)
            .context("Initial Wayland roundtrip failed")?;

        // 2. Second commit: after ack_configure the compositor waits for this commit
        //    to actually map (activate) the layer surface. Without it the surface stays
        //    in "pending" state and is never rendered — it would be invisible.
        surface.commit();

        // Second roundtrip: flush the second commit and confirm compositor accepted it
        event_queue.roundtrip(&mut state)
            .context("Post-configure Wayland roundtrip failed")?;

        info!(
            "Layer surface mapped: {}x{} on {:?}",
            state.width, state.height, active_layer
        );

        Ok(Self {
            conn,
            event_queue,
            state,
            surface,
            layer_surface,
            active_layer,
        })
    }

    pub fn surface_handle(&self) -> WaylandSurfaceHandle {
        WaylandSurfaceHandle {
            display_ptr: self.conn.backend().display_ptr() as *mut _,
            surface_ptr: self.surface.id().as_ptr() as *mut _,
        }
    }

    pub fn dispatch_pending(&mut self) -> Result<()> {
        self.event_queue.dispatch_pending(&mut self.state)?;
        // Flush any pending outgoing Wayland messages to the compositor socket.
        // This is required because WGPU Vulkan (mesa) may queue surface events on a
        // separate internal queue; flushing ensures compositor sees all frame commits.
        self.conn.flush()?;
        Ok(())
    }
}
