#![no_std]

pub const NATIVIS_MAGIC: u32 = 0x4954414E; // 'NATI' in little-endian ASCII (N=0x4E, A=0x41, T=0x54, I=0x49) Wait, N is 0x4E, A is 0x41, T is 0x54, I is 0x49. So 0x4954414E.

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativisFrameHeader {
    pub magic: u32,
    pub version: u32,
    pub frame_id: u64,
    pub timestamp: u64,
    pub attachment_count: u32,
    pub attachment_offset: u32, // Offset in bytes from the start of the header to the array of NativisAttachment
}

pub const NATIVIS_ATTACHMENT_USAGE_COLOR: u32 = 1;
pub const NATIVIS_ATTACHMENT_USAGE_DEPTH: u32 = 2;
pub const NATIVIS_ATTACHMENT_USAGE_METADATA: u32 = 3;

pub const NATIVIS_FORMAT_RGBA8888: u32 = 1;
pub const NATIVIS_FORMAT_NV12: u32 = 2;
pub const NATIVIS_FORMAT_P010: u32 = 3;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativisAttachment {
    pub usage: u32,
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub planes: u32,
    pub surface_index: u32,
    pub data_offset: u32, // Offset in bytes from the start of the SHM region to the pixel data
}

// ── Surface Negotiation & Multi-Kind Descriptors (Phase 1) ──────────────────

pub const NATIVIS_SURFACE_KIND_CPU_SHM: u32 = 1;
pub const NATIVIS_SURFACE_KIND_CPU_PLANAR: u32 = 2;
pub const NATIVIS_SURFACE_KIND_DMABUF: u32 = 3;
pub const NATIVIS_SURFACE_KIND_D3D11_SHARED: u32 = 4;
pub const NATIVIS_SURFACE_KIND_CPU_BUFFER: u32 = 5;

pub const NATIVIS_SURFACE_FLAG_OPAQUE: u32 = 1 << 0;
pub const NATIVIS_SURFACE_FLAG_HDR: u32 = 1 << 1;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NativisPlaneDesc {
    pub handle: i64,
    pub offset: u32,
    pub stride: u32,
}

/// Extensible, C-compatible, POD surface descriptor across plugin and platform boundaries (D3).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativisSurface {
    pub struct_size: u32, // Versioning by size
    pub kind: u32,        // NATIVIS_SURFACE_KIND_*
    pub format: u32,      // NATIVIS_FORMAT_*
    pub width: u32,
    pub height: u32,
    pub modifier: u64,    // Layout / tiling for GPU kinds, 0 for CPU
    pub planes: [NativisPlaneDesc; 4],
    pub plane_count: u32,
    pub fence: i64,       // fd / NT handle / keyed mutex; 0 = none
    pub flags: u32,       // NATIVIS_SURFACE_FLAG_*
    pub slot: u32,        // Buffer pool slot index
}

impl Default for NativisSurface {
    fn default() -> Self {
        Self {
            struct_size: core::mem::size_of::<Self>() as u32,
            kind: NATIVIS_SURFACE_KIND_CPU_BUFFER,
            format: NATIVIS_FORMAT_RGBA8888,
            width: 0,
            height: 0,
            modifier: 0,
            planes: [NativisPlaneDesc { handle: 0, offset: 0, stride: 0 }; 4],
            plane_count: 0,
            fence: 0,
            flags: 0,
            slot: 0,
        }
    }
}

