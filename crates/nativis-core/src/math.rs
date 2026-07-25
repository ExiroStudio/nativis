//! Math primitives — thin re-exports of `glam` with engine-friendly aliases.

pub use glam::{
    Mat2, Mat3, Mat4,
    Quat,
    Vec2, Vec3, Vec4,
    IVec2, IVec3, IVec4,
    UVec2, UVec3, UVec4,
    BVec2, BVec3, BVec4,
};

/// Axis-aligned 2-D bounding rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    #[inline]
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { min: Vec2::new(x, y), max: Vec2::new(x + w, y + h) }
    }

    #[inline]
    pub fn size(&self) -> Vec2 { self.max - self.min }

    #[inline]
    pub fn width(&self) -> f32 { self.max.x - self.min.x }

    #[inline]
    pub fn height(&self) -> f32 { self.max.y - self.min.y }

    #[inline]
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.min.x && p.y >= self.min.y
            && p.x <= self.max.x && p.y <= self.max.y
    }
}
