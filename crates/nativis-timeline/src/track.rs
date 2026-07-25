use crate::keyframe::{Keyframe, Interpolate};

/// A sequence of keyframes that drives one named property on one entity.
/// `property_path` is a dot-separated identifier understood by the scene system,
/// e.g. `"material.uniforms.u_opacity"` or `"transform.scale.x"`.
pub struct PropertyTrack<T: Clone + Copy + Interpolate> {
    pub target_entity_id: u32,
    pub property_path:    String,
    pub keyframes:        Vec<Keyframe<T>>,
}

impl<T: Clone + Copy + Interpolate + 'static> PropertyTrack<T> {
    pub fn new(entity_id: u32, path: impl Into<String>) -> Self {
        Self {
            target_entity_id: entity_id,
            property_path:    path.into(),
            keyframes:        Vec::new(),
        }
    }

    pub fn add_key(&mut self, time_sec: f32, value: T, easing: crate::keyframe::EasingFunction) {
        self.keyframes.push(Keyframe { time_sec, value, easing });
        // Keep sorted by time
        self.keyframes.sort_by(|a, b| a.time_sec.partial_cmp(&b.time_sec).unwrap());
    }

    /// Evaluate the interpolated value at `time_sec`.
    pub fn evaluate(&self, time_sec: f32) -> Option<T> {
        if self.keyframes.is_empty() { return None; }

        // Before first key
        if time_sec <= self.keyframes[0].time_sec {
            return Some(self.keyframes[0].value);
        }
        // After last key
        let last = self.keyframes.last().unwrap();
        if time_sec >= last.time_sec {
            return Some(last.value);
        }

        // Find surrounding keyframe pair
        for i in 0..self.keyframes.len() - 1 {
            let a = &self.keyframes[i];
            let b = &self.keyframes[i + 1];
            if time_sec >= a.time_sec && time_sec < b.time_sec {
                let span = b.time_sec - a.time_sec;
                let t_norm = (time_sec - a.time_sec) / span;
                let t_eased = a.easing.apply(t_norm);
                return Some(T::lerp(a.value, b.value, t_eased));
            }
        }
        None
    }
}
