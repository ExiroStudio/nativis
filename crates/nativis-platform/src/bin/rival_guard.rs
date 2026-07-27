use std::time::{Duration, Instant};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    platform::x11::{WindowAttributesExtX11, WindowType},
    window::{Window, WindowAttributes, WindowId},
};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt, EventMask,
    StackMode, Window as XWindow,
};
use x11rb::protocol::Event;

struct RivalApp {
    window: Option<Window>,
}

impl ApplicationHandler for RivalApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attrs = WindowAttributes::default()
            .with_title("rival-guard") // dipakai sebagai penanda visual di xwininfo
            .with_decorations(false)
            .with_x11_window_type(vec![WindowType::Desktop]);

        let window = event_loop.create_window(attrs).unwrap();

        if let raw_window_handle::RawWindowHandle::Xlib(h) =
            raw_window_handle::HasWindowHandle::window_handle(&window)
                .unwrap()
                .as_raw()
        {
            let xid = h.window as XWindow;
            std::thread::spawn(move || {
                let _ = run_naive_guard(xid); // logika lama yang sengaja dipertahankan buggy
            });
        }
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let WindowEvent::CloseRequested = event {
            event_loop.exit();
        }
    }
}

/// Logika PERSIS Phase 11/12: tidak peduli siapa displacer-nya, selalu
/// menembak BELOW begitu terdeteksi bukan index 0. Ini yang dites di sini.
fn run_naive_guard(xid: XWindow) -> Result<(), Box<dyn std::error::Error>> {
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
        let Event::PropertyNotify(ev) = event else { continue };
        if ev.window != root || ev.atom != stacking_atom { continue; }
        if let Some(t) = last_action {
            if t.elapsed() < debounce { continue; }
        }
        let reply = conn
            .get_property(false, root, stacking_atom, AtomEnum::WINDOW, 0, u32::MAX)?
            .reply()?;
        let windows: Vec<XWindow> = reply.value32().map(|it| it.collect()).unwrap_or_default();
        if windows.first() != Some(&xid) {
            let aux = ConfigureWindowAux::new().stack_mode(StackMode::BELOW);
            conn.configure_window(xid, &aux)?;
            conn.flush()?;
            last_action = Some(Instant::now());
            eprintln!("[rival-guard] {:#x} bukan index 0, menembak BELOW", xid);
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().unwrap();
    let mut app = RivalApp { window: None };
    event_loop.run_app(&mut app).unwrap();
}
