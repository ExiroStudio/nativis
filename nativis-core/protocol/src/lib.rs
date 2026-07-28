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
