use crate::{
    components::{TransformComponent, MaterialComponent, ParticleComponent},
    entity::Entity,
};
use std::collections::HashMap;

/// Flat, column-storage ECS world. Phase 1 uses HashMap-of-components for
/// correctness and simplicity. Phase 2 migrates to archetype column arrays
/// for SIMD-friendly cache access on large particle counts.
pub struct World {
    next_index: u32,
    generations: Vec<u32>,
    alive:       Vec<bool>,
    free_list:   Vec<u32>,

    // Component storage (sparse maps — Phase 2 converts to dense archetypes)
    pub transforms: HashMap<u32, TransformComponent>,
    pub materials:  HashMap<u32, MaterialComponent>,
    pub particles:  HashMap<u32, ParticleComponent>,
}

impl World {
    pub fn new() -> Self {
        Self {
            next_index: 0,
            generations: Vec::new(),
            alive:       Vec::new(),
            free_list:   Vec::new(),
            transforms:  HashMap::new(),
            materials:   HashMap::new(),
            particles:   HashMap::new(),
        }
    }

    pub fn spawn(&mut self) -> Entity {
        if let Some(idx) = self.free_list.pop() {
            self.alive[idx as usize] = true;
            Entity::new(idx, self.generations[idx as usize])
        } else {
            let idx = self.next_index;
            self.next_index += 1;
            self.generations.push(0);
            self.alive.push(true);
            Entity::new(idx, 0)
        }
    }

    pub fn despawn(&mut self, e: Entity) {
        if !self.is_alive(e) { return; }
        let idx = e.index as usize;
        self.alive[idx] = false;
        self.generations[idx] += 1;
        self.free_list.push(e.index);
        self.transforms.remove(&e.index);
        self.materials.remove(&e.index);
        self.particles.remove(&e.index);
    }

    pub fn is_alive(&self, e: Entity) -> bool {
        self.alive.get(e.index as usize).copied().unwrap_or(false)
            && self.generations.get(e.index as usize).copied().unwrap_or(u32::MAX) == e.generation
    }
}

impl Default for World {
    fn default() -> Self { Self::new() }
}
