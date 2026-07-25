use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

/// Generational handle used to reference GPU and engine resources without
/// holding raw pointers. `T` is a zero-sized marker type that makes handles
/// for different resource types incompatible at compile time.
///
/// - `index`      — slot in the resource pool array
/// - `generation` — monotonically increasing counter that invalidates
///                  stale handles after a slot is recycled
pub struct Handle<T> {
    pub(crate) index: u32,
    pub(crate) generation: u32,
    _phantom: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    pub const INVALID: Self = Self {
        index: u32::MAX,
        generation: 0,
        _phantom: PhantomData,
    };

    /// Construct a handle directly. Prefer the resource pool's `allocate()`
    /// method in most situations.
    #[inline]
    pub fn new(index: u32, generation: u32) -> Self {
        Self { index, generation, _phantom: PhantomData }
    }

    #[inline]
    pub fn index(&self) -> u32 { self.index }

    #[inline]
    pub fn generation(&self) -> u32 { self.generation }

    #[inline]
    pub fn is_valid(&self) -> bool { self.index != u32::MAX }
}

// ── Derived trait implementations ────────────────────────────────────────────
impl<T> Copy  for Handle<T> {}
impl<T> Clone for Handle<T> { fn clone(&self) -> Self { *self } }

impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Handle({}/{})", self.index, self.generation)
    }
}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for Handle<T> {}

impl<T> Hash for Handle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}
