use crate::{
    importer::{IAssetImporter, ImportOptions},
    types::CookedAsset,
};
use parking_lot::Mutex;
use std::{collections::HashMap, path::Path, sync::Arc};
use tracing::debug;

/// Central registry of importers and loaded cooked assets.
/// In Phase 2 this gains an async background loading queue backed by tokio.
pub struct ResourceManager {
    importers: Vec<Box<dyn IAssetImporter>>,
    cache:     Mutex<HashMap<String, Arc<CookedAsset>>>,
}

impl ResourceManager {
    pub fn new() -> Self {
        Self {
            importers: Vec::new(),
            cache:     Mutex::new(HashMap::new()),
        }
    }

    pub fn register_importer<I: IAssetImporter + 'static>(&mut self, importer: I) {
        self.importers.push(Box::new(importer));
    }

    /// Load a source file, cook it with the matching importer, and cache the
    /// result. Returns the cached asset on subsequent calls.
    pub fn load(&self, path: &str) -> Result<Arc<CookedAsset>, String> {
        if let Some(cached) = self.cache.lock().get(path) {
            return Ok(cached.clone());
        }

        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let importer = self.importers.iter()
            .find(|i| i.can_import(&ext))
            .ok_or_else(|| format!("No importer for extension: {}", ext))?;

        debug!("Importing asset: {}", path);
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let cooked = importer.import(&bytes, &ImportOptions::default())
            .map_err(|e| e.to_string())?;

        let arc = Arc::new(cooked);
        self.cache.lock().insert(path.to_string(), arc.clone());
        Ok(arc)
    }
}

impl Default for ResourceManager {
    fn default() -> Self { Self::new() }
}
