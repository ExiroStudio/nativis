//! nativis-asset — Asset pipeline: import, cook, and manage engine assets.
//!
//! Raw source files (PNG, MP4, GLSL) are never parsed inside the render loop.
//! They flow through an `IAssetImporter`, are stored as `CookedAsset` binary
//! blobs, and loaded at runtime directly into GPU memory via `ResourceManager`.

pub mod importer;
pub mod manager;
pub mod types;

pub use importer::{IAssetImporter, ImportOptions, ImportError};
pub use manager::ResourceManager;
pub use types::{AssetHandle, AssetType, CookedAsset, AssetHeader};
