//! nativis-ecs — Data-oriented Entity Component System.

pub mod entity;
pub mod world;
pub mod components;

pub use entity::Entity;
pub use world::World;
pub use components::{TransformComponent, MaterialComponent, ParticleComponent};

pub trait IEcsSystem: Send + Sync {
    fn update(&mut self, world: &mut World, delta_sec: f32);
}
