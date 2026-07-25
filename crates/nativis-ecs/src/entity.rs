/// Generational entity identifier.
/// `index` is the slot in the entity array; `generation` invalidates stale references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity {
    pub index:      u32,
    pub generation: u32,
}

impl Entity {
    pub const INVALID: Self = Self { index: u32::MAX, generation: 0 };

    pub fn new(index: u32, generation: u32) -> Self { Self { index, generation } }
    pub fn is_valid(&self) -> bool { self.index != u32::MAX }
}
