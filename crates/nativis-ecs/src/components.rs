use nativis_core::{Mat4, Vec3};

/// 3D transform — local and world matrices. The scene graph computes
/// `world_matrix` from the hierarchy every frame (dirty-flag optimised in Phase 2).
#[derive(Debug, Clone)]
pub struct TransformComponent {
    pub local_matrix: Mat4,
    pub world_matrix: Mat4,
    pub dirty:        bool,
}

impl Default for TransformComponent {
    fn default() -> Self {
        Self {
            local_matrix: Mat4::IDENTITY,
            world_matrix: Mat4::IDENTITY,
            dirty:        true,
        }
    }
}

/// Associates an entity with a GPU material slot.
#[derive(Debug, Clone, Copy)]
pub struct MaterialComponent {
    pub material_id: u32,
    pub layer_index: i32,
}

/// Per-particle state for particle system entities.
#[derive(Debug, Clone, Copy)]
pub struct ParticleComponent {
    pub velocity:     Vec3,
    pub lifetime_sec: f32,
    pub age_sec:      f32,
    pub size:         f32,
    pub color:        [f32; 4],
}
