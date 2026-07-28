//! nativis-plugin — Plugin registry and loader for media backends.
//!
//! Responsibility: Find the right `MediaBackend` for a given `AssetPath`.
//!
//! The runtime calls `registry.create_backend(&asset_path)` and receives
//! a ready-to-open backend. It never names a backend type directly.
//!
//! # Plugin registration
//!
//! Static (built-in) backends call `registry.register(factory_fn)`.
//! Dynamic (.so/.dll) backends will call the same function from their
//! `nativis_plugin_init()` C entry point.

use nativis_asset::AssetPath;
use nativis_core::contract::MediaBackend;
use tracing::{debug, warn};

/// A factory function that constructs a `MediaBackend` instance.
pub type BackendFactory = Box<dyn Fn() -> Box<dyn MediaBackend> + Send + Sync>;

/// A registered backend entry: factory + supported-uri predicate.
struct BackendEntry {
    name:    &'static str,
    factory: BackendFactory,
}

/// Central registry of all available media backends.
///
/// The runtime creates one `PluginRegistry` at startup, registers built-in
/// backends, then calls `create_backend()` for each media open request.
pub struct PluginRegistry {
    entries: Vec<BackendEntry>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Register a media backend factory.
    ///
    /// The `factory` must return a fresh backend instance each call.
    /// `name` is used only for logging.
    pub fn register<F>(&mut self, name: &'static str, factory: F)
    where
        F: Fn() -> Box<dyn MediaBackend> + Send + Sync + 'static,
    {
        debug!("Plugin registered: {name}");
        self.entries.push(BackendEntry { name, factory: Box::new(factory) });
    }

    /// Find and instantiate a backend that supports the given `AssetPath`.
    ///
    /// Returns `None` if no registered backend supports the URI.
    /// The runtime never decides which backend to use — the registry does.
    pub fn create_backend(&self, source: &AssetPath) -> Option<Box<dyn MediaBackend>> {
        for entry in &self.entries {
            // Probe using a temporary instance — supports() is cheap.
            let probe = (entry.factory)();
            if probe.supports(source) {
                debug!("Backend '{}' selected for '{}'", entry.name, source.raw_uri());
                // Return a fresh instance (not the probe).
                return Some((entry.factory)());
            }
        }
        warn!("No backend found for '{}'", source.raw_uri());
        None
    }

    /// Number of registered backends.
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
}

impl Default for PluginRegistry {
    fn default() -> Self { Self::new() }
}
