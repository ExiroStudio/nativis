//! nativis-media — Media abstraction layer.
//!
//! Every media type (static image, video, procedural shader, camera, network
//! stream) implements `IMediaSource` and produces `VideoFrame`s containing
//! GPU texture handles. The renderer is completely decoupled from the source.

pub mod clock;
pub mod frame;
pub mod source;
pub mod image_source;

pub use clock::MediaClock;
pub use frame::{VideoFrame, PixelFormat, ColorSpace, MediaState};
pub use source::IMediaSource;
pub use image_source::ImageSource;

#[cfg(feature = "video")]
pub mod video_source;
#[cfg(feature = "video")]
pub use video_source::VideoFileSource;
