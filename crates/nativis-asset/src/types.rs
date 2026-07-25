use nativis_core::Handle;
use serde::{Deserialize, Serialize};

// ── Asset type tags ───────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssetType {
    Texture2D,
    VideoStream,
    AudioClip,
    Shader,
    Scene,
    Material,
    Font,
    Raw,
}

// ── Generational handle ───────────────────────────────────────────────────────
pub struct AssetMarker<T>(std::marker::PhantomData<fn() -> T>);
pub type AssetHandle<T> = Handle<AssetMarker<T>>;

// ── Cooked asset format ───────────────────────────────────────────────────────

/// 32-byte fixed header stored at the start of every `.nva` (Nativis Asset) file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetHeader {
    pub magic:      [u8; 4],   // b"NVA\x01"
    pub asset_type: AssetType,
    pub uuid:       u128,
    pub source_hash: u64,
    pub width:      u32,       // for textures
    pub height:     u32,       // for textures
    pub mip_levels: u32,
}

impl AssetHeader {
    pub const MAGIC: [u8; 4] = *b"NVA\x01";
}

/// A fully cooked asset ready for GPU upload. Produced by `IAssetImporter`
/// and consumed by `ResourceManager`.
pub struct CookedAsset {
    pub header: AssetHeader,
    /// Raw GPU-ready pixel data (RGBA8, BC7, ASTC, etc.)
    pub data:   Vec<u8>,
}
