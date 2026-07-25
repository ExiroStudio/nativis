//! nativis-timeline — Keyframe animation and property binding system.

pub mod keyframe;
pub mod track;
pub mod timeline;

pub use keyframe::{Keyframe, EasingFunction};
pub use track::PropertyTrack;
pub use timeline::Timeline;

use nativis_ecs::World;

pub trait IAnimationSystem: Send + Sync {
    fn step(&mut self, delta_sec: f32, world: &mut World);
}
