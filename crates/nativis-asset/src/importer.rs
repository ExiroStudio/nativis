use crate::types::{AssetType, CookedAsset};
use thiserror::Error;

#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    pub generate_mipmaps: bool,
    pub compress_gpu:     bool, // BC7/ASTC compression (Phase 2)
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Decode error: {0}")]
    Decode(String),
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),
}

/// An asset importer transforms raw source bytes into a `CookedAsset`.
/// Register importers with `ResourceManager::register_importer()`.
pub trait IAssetImporter: Send + Sync {
    fn asset_type(&self) -> AssetType;
    /// Return `true` if this importer handles the given file extension.
    fn can_import(&self, extension: &str) -> bool;
    /// Cook raw file bytes. May be called from a background thread.
    fn import(&self, source: &[u8], opts: &ImportOptions) -> Result<CookedAsset, ImportError>;
}
