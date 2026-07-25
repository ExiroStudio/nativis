use nativis_core::Vec2;

/// Every OS-level or engine-level signal that crosses a module boundary must
/// be expressed as an `EngineEvent` variant. Add new variants here — never
/// add direct callbacks between subsystems.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum EngineEvent {
    // ── Window / display ────────────────────────────────────────────────────
    WindowResized      { width: u32, height: u32 },
    WindowFocusGained,
    WindowFocusLost,
    WindowCloseRequested,

    /// A monitor was plugged in, removed, or its resolution/refresh changed.
    DisplayTopologyChanged { monitor_count: u32 },

    // ── Input ───────────────────────────────────────────────────────────────
    PointerMoved   { position: Vec2 },
    PointerButton  { button: MouseButton, pressed: bool },
    KeyboardKey    { key: KeyCode, pressed: bool },
    ScrollWheel    { delta: Vec2 },

    // ── Power management ────────────────────────────────────────────────────
    BatteryStateChanged { on_battery: bool, low_power_mode: bool },

    // ── Audio device ────────────────────────────────────────────────────────
    AudioDeviceChanged  { device_id: String },

    // ── Engine lifecycle ────────────────────────────────────────────────────
    EngineStarted,
    EngineShutdown,

    // ── Media ───────────────────────────────────────────────────────────────
    MediaSourceEnded    { source_name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton { Left, Right, Middle, Other(u8) }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Escape, Space, Enter, Tab,
    ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
    Char(char),
    Other(u32),
}
