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
            force_below_desktop_containment(&window);
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

use std::collections::VecDeque;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt, EventMask,
    StackMode, Window as XWindow,
};
use x11rb::protocol::Event;

const PLASMASHELL_CLASS_HINT: &str = "plasmashell"; // BELUM diverifikasi via xprop, lihat Section 6
const OSCILLATION_WINDOW: Duration = Duration::from_secs(2);
const OSCILLATION_THRESHOLD: usize = 3;
const BACKOFF_BASE: Duration = Duration::from_secs(1);
const BACKOFF_CAP: Duration = Duration::from_secs(30);
const DEBOUNCE: Duration = Duration::from_millis(150);

struct DisplacerEvent {
    at: Instant,
    xid: XWindow,
}

pub fn spawn_stacking_guard(nativis_xid: u32) {
    std::thread::spawn(move || {
        if let Err(e) = run_guard_v2(nativis_xid) {
            eprintln!("[stacking-guard] berhenti karena error: {e}");
        }
    });
}

fn run_guard_v2(nativis_xid: XWindow) -> Result<(), Box<dyn std::error::Error>> {
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

    let mut history: VecDeque<DisplacerEvent> = VecDeque::new();
    let mut last_action: Option<Instant> = None;
    let mut backoff_until: Option<Instant> = None;
    let mut backoff_step: u32 = 0;

    loop {
        let event = conn.wait_for_event()?;
        let detect_time = Instant::now();
        let Event::PropertyNotify(ev) = event else { continue };
        if ev.window != root || ev.atom != stacking_atom { continue; }

        // Lapis 1: debounce cepat (sama seperti Phase 11).
        if let Some(t) = last_action {
            if t.elapsed() < DEBOUNCE { continue; }
        }

        let windows = get_stacking_list(&conn, root, stacking_atom)?;
        if windows.first() == Some(&nativis_xid) {
            continue; // sudah di dasar, tidak ada yang perlu dilakukan
        }
        let Some(&displacer) = windows.first() else { continue };

        // Lapis 2: identitas & backoff cooldown (Desain Opsi A: Pasif Event-Driven).
        // Keputusan Desain Opsi A: Guard bersifat murni pasif terhadap PropertyNotify.
        // Jika rival terus menembak, PropertyNotify akan terus masuk secara alami.
        // Setelah backoff_until habis, PropertyNotify berikutnya akan otomatis membangunkan guard
        // untuk mencoba reassert kembali, tanpa membutuhkan thread heartbeat terpisah.
        let class = get_wm_class(&conn, displacer).unwrap_or_default();

        if let Some(until) = backoff_until {
            if Instant::now() < until && class.as_deref() != Some(PLASMASHELL_CLASS_HINT) {
                continue; // masih cooldown vs lawan tak dikenal
            }
        }

        if class.as_deref() == Some(PLASMASHELL_CLASS_HINT) {
            // Kasus dikenal & jinak (Fact Phase 11) — selalu tindak segera.
            reassert_below(&conn, nativis_xid)?;
            let action_time = Instant::now();
            last_action = Some(action_time);
            backoff_step = 0;
            backoff_until = None;
            history.clear(); // Clear oscillation history on confirmed plasmashell
            eprintln!("[stacking-guard] displacer=plasmashell, re-lower segera (Latency: {:?})", action_time.duration_since(detect_time));
            continue;
        }

        if class.is_none() || class.as_deref() == Some("") {
            // WM_CLASS belum siap dibaca (race startup) — reassert seperti biasa,
            // TAPI jangan masukkan ke history sampai identitas jelas.
            reassert_below(&conn, nativis_xid)?;
            let action_time = Instant::now();
            last_action = Some(action_time);
            eprintln!("[stacking-guard] displacer=unresolved, re-lower sementara (Latency: {:?})", action_time.duration_since(detect_time));
            continue;
        }

        history.push_back(DisplacerEvent { at: Instant::now(), xid: displacer });
        while let Some(front) = history.front() {
            if front.at.elapsed() > OSCILLATION_WINDOW { history.pop_front(); } else { break; }
        }
        let repeat_count = history.iter().filter(|e| e.xid == displacer).count();

        if repeat_count >= OSCILLATION_THRESHOLD {
            // Lapis 3: displacer sama berulang dalam waktu singkat = stalemate.
            let wait = BACKOFF_BASE.saturating_mul(2u32.saturating_pow(backoff_step)).min(BACKOFF_CAP);
            eprintln!(
                "[stacking-guard] STALEMATE vs xid={displacer:#x} class={class:?} — backoff {wait:?}"
            );
            backoff_until = Some(Instant::now() + wait);
            backoff_step += 1;
            continue;
        }

        // Displacement baru/jarang, kemungkinan lifecycle event wajar.
        reassert_below(&conn, nativis_xid)?;
        let action_time = Instant::now();
        last_action = Some(action_time);
        eprintln!("[stacking-guard] displacer=unknown ({class:?}), re-lower (Latency: {:?})", action_time.duration_since(detect_time));
    }
}

fn get_stacking_list(
    conn: &impl Connection,
    root: XWindow,
    stacking_atom: u32,
) -> Result<Vec<XWindow>, Box<dyn std::error::Error>> {
    let reply = conn
        .get_property(false, root, stacking_atom, AtomEnum::WINDOW, 0, u32::MAX)?
        .reply()?;
    Ok(reply.value32().map(|it| it.collect()).unwrap_or_default())
}

fn get_wm_class(
    conn: &impl Connection,
    xid: XWindow,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let reply = conn
        .get_property(false, xid, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)?
        .reply()?;
    let parts: Vec<&[u8]> = reply.value.split(|&b| b == 0).filter(|s| !s.is_empty()).collect();
    Ok(parts.last().map(|s| String::from_utf8_lossy(s).to_string()))
}

fn reassert_below(conn: &impl Connection, xid: XWindow) -> Result<(), Box<dyn std::error::Error>> {
    let aux = ConfigureWindowAux::new().stack_mode(StackMode::BELOW);
    conn.configure_window(xid, &aux)?;
    conn.flush()?;
    Ok(())
}

fn main() {
    let event_loop = EventLoop::new().unwrap();
    let mut app = NativisApp { window: None };
    event_loop.run_app(&mut app).unwrap();
}
