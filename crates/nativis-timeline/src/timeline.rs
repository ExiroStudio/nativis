use crate::track::PropertyTrack;
use crate::IAnimationSystem;
use nativis_ecs::World;

type EvaluatorFn = Box<dyn Fn(f32, &mut World) + Send + Sync>;

/// Master animation timeline. Holds typed tracks and evaluates them each frame.
///
/// Usage:
/// ```ignore
/// let mut timeline = Timeline::new();
/// let mut track = PropertyTrack::<f32>::new(entity.index, "material.uniforms.u_opacity");
/// track.add_key(0.0, 0.0, EasingFunction::EaseInOut);
/// track.add_key(2.0, 1.0, EasingFunction::EaseInOut);
/// timeline.add_float_track(track);
/// ```
pub struct Timeline {
    elapsed_sec: f32,
    looping:     bool,
    duration_sec: f32,
    // Type-erased evaluators — avoids making Timeline generic over T.
    evaluators:  Vec<EvaluatorFn>,
}

impl Timeline {
    pub fn new() -> Self {
        Self {
            elapsed_sec:  0.0,
            looping:      true,
            duration_sec: 0.0,
            evaluators:   Vec::new(),
        }
    }

    pub fn set_duration(&mut self, sec: f32) { self.duration_sec = sec; }
    pub fn set_looping(&mut self, looping: bool) { self.looping = looping; }

    /// Register a float-valued keyframe track. The closure applies the
    /// evaluated value to the entity/world on each animation step.
    pub fn add_float_track<F>(&mut self, track: PropertyTrack<f32>, apply: F)
    where
        F: Fn(f32, &mut World) + Send + Sync + 'static,
    {
        self.evaluators.push(Box::new(move |time_sec, world| {
            if let Some(v) = track.evaluate(time_sec) {
                apply(v, world);
            }
        }));
    }
}

impl IAnimationSystem for Timeline {
    fn step(&mut self, delta_sec: f32, world: &mut World) {
        self.elapsed_sec += delta_sec;
        if self.looping && self.duration_sec > 0.0 {
            self.elapsed_sec %= self.duration_sec;
        }
        let t = self.elapsed_sec;
        for eval in &self.evaluators {
            eval(t, world);
        }
    }
}

impl Default for Timeline { fn default() -> Self { Self::new() } }
