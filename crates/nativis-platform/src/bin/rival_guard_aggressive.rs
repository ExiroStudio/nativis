use std::time::Duration;
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
            .with_title("rival-guard-aggressive")
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
                let _ = run_aggressive_guard(xid);
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

/// Rival Agresif: TANPA DEBOUNCE.
/// Begitu menerima PropertyNotify bahwa ia bukan index 0, atau via timer periodik 200ms,
/// ia SELALU menembak BELOW tanpa jeda untuk memaksa livelock osilasi.
fn run_aggressive_guard(xid: XWindow) -> Result<(), Box<dyn std::error::Error>> {
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

    let conn_thread = conn;
    
    loop {
        let event = conn_thread.wait_for_event()?;
        let Event::PropertyNotify(ev) = event else { continue };
        if ev.window != root || ev.atom != stacking_atom { continue; }

        let reply = conn_thread
            .get_property(false, root, stacking_atom, AtomEnum::WINDOW, 0, u32::MAX)?
            .reply()?;
        let windows: Vec<XWindow> = reply.value32().map(|it| it.collect()).unwrap_or_default();
        if windows.first() != Some(&xid) {
            let aux = ConfigureWindowAux::new().stack_mode(StackMode::BELOW);
            conn_thread.configure_window(xid, &aux)?;
            conn_thread.flush()?;
            eprintln!("[rival-agresif] {:#x} direbut, menembak BELOW TANPA DEBOUNCE!", xid);
            std::thread::sleep(Duration::from_millis(200)); // sleep sebentar agar tidak overflow X buffer, tapi pasti menembak ulang
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().unwrap();
    let mut app = RivalApp { window: None };
    event_loop.run_app(&mut app).unwrap();
}
