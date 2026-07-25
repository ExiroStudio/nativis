//! nativis-core — Foundation primitives shared by all engine crates.
//!
//! Provides:
//!  - Generational `Handle<T>` for GPU resource references
//!  - `glam` math re-exports
//!  - Lock-free SPSC `RingBuffer<T, N>`

pub mod handle;
pub mod math;
pub mod ring_buffer;

pub use handle::Handle;
pub use math::*;
pub use ring_buffer::RingBuffer;
