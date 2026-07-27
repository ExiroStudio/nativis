//! Resolution Engine driving environment evaluation into an execution AttachmentPlan.

use crate::detector::EnvironmentInfo;
use crate::rules::{
    AttachmentPlan, KdePlasmaNativeRule, MacOsWindowLevelRule, ResolutionRule,
    UniversalFallbackRule, WaylandLayerShellRule, WindowsWorkerWRule, X11EwmhRule,
};

/// Engine that executes resolution rules to build an `AttachmentPlan`.
pub struct ResolutionEngine {
    rules: Vec<Box<dyn ResolutionRule>>,
}

impl ResolutionEngine {
    /// Creates an engine initialized with all built-in platform rules.
    pub fn default_engine() -> Self {
        let mut engine = Self { rules: Vec::new() };
        engine.register(Box::new(KdePlasmaNativeRule));
        engine.register(Box::new(WaylandLayerShellRule));
        engine.register(Box::new(X11EwmhRule));
        engine.register(Box::new(WindowsWorkerWRule));
        engine.register(Box::new(MacOsWindowLevelRule));
        engine.register(Box::new(UniversalFallbackRule));
        engine
    }

    /// Registers a custom resolution rule.
    pub fn register(&mut self, rule: Box<dyn ResolutionRule>) {
        self.rules.push(rule);
    }

    /// Evaluates environment facts against all registered rules to build an `AttachmentPlan`.
    pub fn resolve(&self, env: &EnvironmentInfo) -> AttachmentPlan {
        let mut plan = AttachmentPlan::new();

        for rule in &self.rules {
            rule.evaluate(env, &mut plan);
        }

        plan
    }
}

impl Default for ResolutionEngine {
    fn default() -> Self {
        Self::default_engine()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{
        AttachmentStrategy, DesktopEnvironment, DisplayServer, OperatingSystem, WindowManager,
    };

    #[test]
    fn test_kde_plasma_wayland_resolution() {
        let engine = ResolutionEngine::default_engine();
        let env = EnvironmentInfo {
            os: OperatingSystem::Linux,
            display_server: DisplayServer::Wayland,
            desktop_environment: DesktopEnvironment::KdePlasma,
            window_manager: WindowManager::KWin,
            session_type: "wayland".into(),
            xdg_current_desktop: "KDE".into(),
            desktop_session: "plasma".into(),
        };

        let plan = engine.resolve(&env);

        assert_eq!(
            plan.strategies,
            vec![
                AttachmentStrategy::NativeAPI,
                AttachmentStrategy::LayerShell,
                AttachmentStrategy::FallbackWindow,
            ]
        );
        assert!(plan.matched_rules.contains(&"KDE Plasma Native API Rule".to_string()));
        assert!(plan.matched_rules.contains(&"Wayland Layer Shell Rule".to_string()));
    }
}
