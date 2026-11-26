//! nativis-media — Media abstraction layer.
//!
//! Every media type (static image, video, procedural shader, camera, network
//! stream) implements `IMediaSource` and produces `VideoFrame`s containing
//! GPU texture handles. The renderer is completely decoupled from the source.

pub mod clock;
pub mod frame;
pub mod image_source;
pub mod source;

pub use clock::MediaClock;
pub use frame::{ColorSpace, MediaState, PixelFormat, VideoFrame};
pub use image_source::ImageSource;
pub use source::IMediaSource;
