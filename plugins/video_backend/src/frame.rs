//! `frame.rs` — Immutable decoded frame that exits the decoder.
//!
//! ## Architectural Rule
//! After a `DecodedFrame` is created, its pixel data is immutable.
//! The Runtime may reference or release it, but never mutate it.
//! The decoder itself never holds a reference after handing the frame off.

use std::sync::Arc;
use std::time::Duration;

/// A single video frame that has been decoded to NV12 planar format.
///
/// Immutable after creation. The decoder has no further interaction with it.
/// The Runtime and Transport layer receive this purely as data.
#[derive(Clone)]
pub struct DecodedFrame {
    pub width:     u32,
    pub height:    u32,
    /// Raw PTS as returned by libavcodec. Preserved exactly to avoid
    /// precision loss for streams with unusual time bases.
    pub pts:       i64,
    /// Numerator and denominator of the stream's time base.
    /// Stored separately from `pts` so absolute time can be computed
    /// later without floating-point accumulation error.
    pub time_base_num: i32,
    pub time_base_den: i32,
    /// Y plane (luma) — full resolution, 1 byte per pixel.
    pub y_plane:       Arc<[u8]>,
    /// UV plane (chroma, interleaved) — half resolution.
    pub uv_plane:      Arc<[u8]>,
    /// Stride (bytes per row) of the Y plane — may be > width due to alignment.
    pub y_stride:      u32,
    /// Stride (bytes per row) of the UV plane — may be > width due to alignment.
    pub uv_stride:     u32,
    /// Height of the chroma plane: ceil(height / 2).
    pub chroma_height: u32,
}

impl DecodedFrame {
    /// Compute absolute presentation time in seconds.
    #[inline]
    pub fn pts_seconds(&self) -> f64 {
        if self.time_base_den == 0 {
            return 0.0;
        }
        self.pts as f64 * (self.time_base_num as f64 / self.time_base_den as f64)
    }

    /// Convert PTS to a `std::time::Duration` for the `Frame` contract.
    #[inline]
    pub fn pts_duration(&self) -> Duration {
        Duration::from_secs_f64(self.pts_seconds().max(0.0))
    }
}
