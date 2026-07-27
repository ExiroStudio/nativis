use std::env;
use std::time::Duration;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    platform::x11::{WindowAttributesExtX11, WindowType},
    window::{Window, WindowAttributes, WindowId},
};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt, StackMode, Window as XWindow};

struct RivalApp {
    window: Option<Window>,
}

impl ApplicationHandler for RivalApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attrs = WindowAttributes::default()
            .with_title("rival-timer-only")
            .with_decorations(false)
            .with_x11_window_type(vec![WindowType::Desktop]);

        let window = event_loop.create_window(attrs).unwrap();

        if let raw_window_handle::RawWindowHandle::Xlib(h) =
            raw_window_handle::HasWindowHandle::window_handle(&window)
                .unwrap()
                .as_raw()
        {
            let xid = h.window as XWindow;
            let interval_ms: u64 = env::args()
                .nth(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(50); // default 50ms — jauh lebih cepat dari debounce 150ms Nativis
            std::thread::spawn(move || {
                let _ = run_timer_only(xid, Duration::from_millis(interval_ms));
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

fn run_timer_only(xid: XWindow, interval: Duration) -> Result<(), Box<dyn std::error::Error>> {
    let (conn, _screen_num) = x11rb::connect(None)?;
    let mut shot_count: u64 = 0;
    loop {
        let aux = ConfigureWindowAux::new().stack_mode(StackMode::BELOW);
        conn.configure_window(xid, &aux)?;
        conn.flush()?;
        shot_count += 1;
        eprintln!("[rival-timer] tembakan #{shot_count} (unconditional, interval={interval:?})");
        std::thread::sleep(interval);
    }
}

fn main() {
    let event_loop = EventLoop::new().unwrap();
    let mut app = RivalApp { window: None };
    event_loop.run_app(&mut app).unwrap();
}
