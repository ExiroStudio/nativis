/// Interpolation curve for a keyframe transition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EasingFunction {
    Linear,
    Step,
    /// CSS cubic-bezier(x1, y1, x2, y2) — standard easing curves.
    CubicBezier { x1: f32, y1: f32, x2: f32, y2: f32 },
    EaseIn,
    EaseOut,
    EaseInOut,
}

impl EasingFunction {
    /// Evaluate the easing curve at normalised time `t` ∈ [0, 1].
    pub fn apply(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear      => t,
            Self::Step        => if t < 1.0 { 0.0 } else { 1.0 },
            Self::EaseIn      => t * t,
            Self::EaseOut     => t * (2.0 - t),
            Self::EaseInOut   => {
                if t < 0.5 { 2.0 * t * t }
                else       { -1.0 + (4.0 - 2.0 * t) * t }
            }
            // Simple cubic bezier approximation (full implementation in Phase 2)
            Self::CubicBezier { x1: _, y1, x2: _, y2 } => {
                let p1y = *y1;
                let p2y = *y2;
                // Approximate with 3rd-order Bezier y(t)
                let t2 = t * t;
                let t3 = t2 * t;
                let u  = 1.0 - t;
                let u2 = u * u;
                3.0 * u2 * t * p1y + 3.0 * u * t2 * p2y + t3
            }
        }
    }
}

/// A single keyframe at a point in time.
#[derive(Debug, Clone)]
pub struct Keyframe<T: Clone + Copy + Interpolate> {
    /// Time in seconds from timeline start.
    pub time_sec: f32,
    pub value:    T,
    pub easing:   EasingFunction,
}

/// Value types that can be interpolated between keyframes.
pub trait Interpolate: Clone + Copy {
    fn lerp(a: Self, b: Self, t: f32) -> Self;
}

impl Interpolate for f32 {
    fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }
}

impl Interpolate for [f32; 4] {
    fn lerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
        [
            f32::lerp(a[0], b[0], t),
            f32::lerp(a[1], b[1], t),
            f32::lerp(a[2], b[2], t),
            f32::lerp(a[3], b[3], t),
        ]
    }
}
