use winit::{
    application::ApplicationHandler,
    dpi::{PhysicalPosition, PhysicalSize},
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    platform::x11::{WindowAttributesExtX11, WindowType},
    window::{Window, WindowAttributes, WindowId},
};

struct NativisApp {
    window: Option<Window>,
}

impl ApplicationHandler for NativisApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }
        
        let monitor = event_loop
            .primary_monitor()
            .expect("tidak ada monitor primer terdeteksi");
        let size = monitor.size();
        let position = monitor.position();

        let attrs = WindowAttributes::default()
            .with_title("nativis-surface")
            .with_decorations(false)
            .with_resizable(false)
            // KRITIS (Fact 6.3): harus di-set SEBELUM window dibuat.
            .with_x11_window_type(vec![WindowType::Desktop])
            // Phase 9: JANGAN with_fullscreen() — samakan geometri manual.
            .with_inner_size(PhysicalSize::new(size.width, size.height))
            .with_position(PhysicalPosition::new(position.x, position.y));

        let window = event_loop
            .create_window(attrs)
            .expect("gagal membuat window ber-type Desktop");

        // Phase 8: paksa ke dasar stacking order segera setelah map.
        #[cfg(target_os = "linux")]
        {
            // force_below_desktop_containment(&window); // Disabled to test the guard!
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = window.window_handle() {
                if let RawWindowHandle::Xlib(h) = handle.as_raw() {
                    spawn_stacking_guard(h.window as u32);
                }
            }
        }

        self.window = Some(window);
        println!("Window created and mapped.");
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let WindowEvent::CloseRequested = event {
            event_loop.exit();
        }
    }
}

#[cfg(target_os = "linux")]
fn force_below_desktop_containment(window: &winit::window::Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt, StackMode};

    let RawWindowHandle::Xlib(handle) = window.window_handle().unwrap().as_raw() else {
        return;
    };
    let xid = handle.window as u32;
    println!("XID: {}", xid);

    let (conn, _screen_num) =
        x11rb::connect(None).expect("tidak bisa konek ulang ke X server");

    // StackMode::BELOW tanpa sibling eksplisit mendorong window ke dasar
    // stacking order root — setara XLowerWindow.
    let aux = ConfigureWindowAux::new().stack_mode(StackMode::BELOW);
    conn.configure_window(xid, &aux).expect("gagal me-lower window");
    conn.flush().expect("flush gagal");
    println!("Force BELOW applied via x11rb.");
}

pub fn spawn_stacking_guard(nativis_xid: u32) {
    std::thread::spawn(move || {
        if let Err(e) = run_guard(nativis_xid) {
            eprintln!("[stacking-guard] berhenti karena error: {}", e);
        }
    });
}

fn run_guard(nativis_xid: u32) -> Result<(), Box<dyn std::error::Error>> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        AtomEnum, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt, EventMask,
        StackMode, Window,
    };
    use x11rb::protocol::Event;
    use std::time::{Duration, Instant};

    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;

    conn.change_window_attributes(
        root,
        &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )?;
    conn.flush()?;

    let stacking_atom = conn
        .intern_atom(false, b"_NET_CLIENT_LIST_STACKING")?
        .reply()?
        .atom;

    let debounce = Duration::from_millis(150);
    let mut last_action: Option<Instant> = None;

    loop {
        let event = conn.wait_for_event()?;
        let detect_time = Instant::now();
        let Event::PropertyNotify(ev) = event else { continue };
        if ev.window != root || ev.atom != stacking_atom {
            continue;
        }

        if let Some(t) = last_action {
            if t.elapsed() < debounce {
                continue;
            }
        }

        if !is_at_bottom(&conn, root, stacking_atom, nativis_xid)? {
            let aux = ConfigureWindowAux::new().stack_mode(StackMode::BELOW);
            conn.configure_window(nativis_xid, &aux)?;
            conn.flush()?;
            let action_time = Instant::now();
            last_action = Some(action_time);
            eprintln!("[stacking-guard] window lain merebut posisi bawah, Nativis di-re-lower (Latency: {:?})", action_time.duration_since(detect_time));
        }
    }
}

fn is_at_bottom(
    conn: &impl x11rb::connection::Connection,
    root: u32,
    stacking_atom: u32,
    xid: u32,
) -> Result<bool, Box<dyn std::error::Error>> {
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    let reply = conn
        .get_property(false, root, stacking_atom, AtomEnum::WINDOW, 0, u32::MAX)?
        .reply()?;
    let windows: Vec<u32> = reply.value32().map(|it| it.collect()).unwrap_or_default();
    Ok(windows.first() == Some(&xid))
}

fn main() {
    let event_loop = EventLoop::new().unwrap();
    let mut app = NativisApp { window: None };
    event_loop.run_app(&mut app).unwrap();
}
