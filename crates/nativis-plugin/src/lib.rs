//! nativis-plugin — Capability declaration and security permission system.
//!
//! Every plugin must declare its required capabilities in its `PluginManifest`.
//! The `ICapabilitySecurityManager` decides whether to grant them at load time.
//! In Phase 1 this ships as a fully-defined contract with a permissive stub
//! implementation. Enforcement is added in Phase 2 (Wasm sandbox).

use serde::{Deserialize, Serialize};

// ── Capability enum ───────────────────────────────────────────────────────────

/// Every privileged operation a plugin may request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Capability {
    /// Capture system audio loopback for spectrum analysis.
    AudioLoopbackCapture,
    /// Open camera / webcam device streams.
    CameraStreamAccess,
    /// Open TCP/UDP network sockets.
    NetworkSocketAccess,
    /// Read arbitrary files from the host filesystem.
    FileSystemRead,
    /// Write arbitrary files to the host filesystem.
    FileSystemWrite,
    /// Dispatch GPU compute shaders.
    GpuComputeDispatch,
    /// Access the screen capture API.
    ScreenCapture,
}

// ── Manifest ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Unique reverse-DNS identifier, e.g. `"io.nativis.lottie-source"`.
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub requested_capabilities: Vec<Capability>,
}

// ── Security contract ─────────────────────────────────────────────────────────

pub trait ICapabilitySecurityManager: Send + Sync {
    /// Returns `true` if the plugin is allowed to use this capability.
    fn request_permission(&self, plugin_id: &str, capability: Capability) -> bool;
    fn revoke_permission(&self, plugin_id: &str, capability: Capability);
    fn has_permission(&self, plugin_id: &str, capability: Capability) -> bool;
}

// ── Phase 1 permissive stub ───────────────────────────────────────────────────

/// Development stub: grants every capability unconditionally.
/// Replace with policy-enforced implementation before marketplace launch.
pub struct PermissiveSecurityManager;

impl ICapabilitySecurityManager for PermissiveSecurityManager {
    fn request_permission(&self, _plugin_id: &str, _capability: Capability) -> bool { true }
    fn revoke_permission(&self, _plugin_id: &str, _capability: Capability) {}
    fn has_permission(&self, _plugin_id: &str, _capability: Capability) -> bool { true }
}
