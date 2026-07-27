//! Rule abstractions and implementations for platform resolution.

use crate::detector::EnvironmentInfo;
use crate::metadata::{
    AttachmentStrategy, DesktopEnvironment, DisplayServer, OperatingSystem,
};

/// An execution plan produced by the ResolutionEngine after inspecting host environment facts.
#[derive(Debug, Clone, Default)]
pub struct AttachmentPlan {
    /// Ordered sequence of strategy candidates to attempt.
    pub strategies: Vec<AttachmentStrategy>,
    /// Audit log of rules evaluated and whether they matched (`(rule_name, matched)`).
    pub evaluated_rules: Vec<(String, bool)>,
    /// List of rule names that successfully matched.
    pub matched_rules: Vec<String>,
}

impl AttachmentPlan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_strategy(&mut self, strategy: AttachmentStrategy) {
        if !self.strategies.contains(&strategy) {
            self.strategies.push(strategy);
        }
    }
}

/// A rule that evaluates environment facts and appends compatible strategies to an `AttachmentPlan`.
pub trait ResolutionRule: Send + Sync {
    /// Name of the rule for diagnostics.
    fn name(&self) -> &'static str;

    /// Evaluates environment facts. If matched, modifies the plan and returns `true`.
    fn evaluate(&self, env: &EnvironmentInfo, plan: &mut AttachmentPlan) -> bool;
}

// ── Built-in Resolution Rules ────────────────────────────────────────────────

pub struct KdePlasmaNativeRule;

impl ResolutionRule for KdePlasmaNativeRule {
    fn name(&self) -> &'static str {
        "KDE Plasma Native API Rule"
    }

    fn evaluate(&self, env: &EnvironmentInfo, plan: &mut AttachmentPlan) -> bool {
        let matched = env.os == OperatingSystem::Linux
            && env.desktop_environment == DesktopEnvironment::KdePlasma;

        if matched {
            plan.push_strategy(AttachmentStrategy::NativeAPI);
            plan.matched_rules.push(self.name().to_string());
        }
        plan.evaluated_rules.push((self.name().to_string(), matched));
        matched
    }
}

pub struct WaylandLayerShellRule;

impl ResolutionRule for WaylandLayerShellRule {
    fn name(&self) -> &'static str {
        "Wayland Layer Shell Rule"
    }

    fn evaluate(&self, env: &EnvironmentInfo, plan: &mut AttachmentPlan) -> bool {
        let matched = env.os == OperatingSystem::Linux && env.is_wayland();

        if matched {
            plan.push_strategy(AttachmentStrategy::LayerShell);
            plan.matched_rules.push(self.name().to_string());
        }
        plan.evaluated_rules.push((self.name().to_string(), matched));
        matched
    }
}

pub struct X11EwmhRule;

impl ResolutionRule for X11EwmhRule {
    fn name(&self) -> &'static str {
        "X11 EWMH Stacking Rule"
    }

    fn evaluate(&self, env: &EnvironmentInfo, plan: &mut AttachmentPlan) -> bool {
        let matched = env.os == OperatingSystem::Linux && env.is_x11();

        if matched {
            plan.push_strategy(AttachmentStrategy::EwmhDesktop);
            plan.push_strategy(AttachmentStrategy::RootBlit);
            plan.matched_rules.push(self.name().to_string());
        }
        plan.evaluated_rules.push((self.name().to_string(), matched));
        matched
    }
}

pub struct WindowsWorkerWRule;

impl ResolutionRule for WindowsWorkerWRule {
    fn name(&self) -> &'static str {
        "Windows WorkerW Injection Rule"
    }

    fn evaluate(&self, env: &EnvironmentInfo, plan: &mut AttachmentPlan) -> bool {
        let matched = env.os == OperatingSystem::Windows
            || env.display_server == DisplayServer::Win32;

        if matched {
            plan.push_strategy(AttachmentStrategy::WindowInjection);
            plan.matched_rules.push(self.name().to_string());
        }
        plan.evaluated_rules.push((self.name().to_string(), matched));
        matched
    }
}

pub struct MacOsWindowLevelRule;

impl ResolutionRule for MacOsWindowLevelRule {
    fn name(&self) -> &'static str {
        "macOS Window Level Rule"
    }

    fn evaluate(&self, env: &EnvironmentInfo, plan: &mut AttachmentPlan) -> bool {
        let matched = env.os == OperatingSystem::MacOS
            || env.display_server == DisplayServer::Quartz;

        if matched {
            plan.push_strategy(AttachmentStrategy::NativeAPI);
            plan.matched_rules.push(self.name().to_string());
        }
        plan.evaluated_rules.push((self.name().to_string(), matched));
        matched
    }
}

pub struct UniversalFallbackRule;

impl ResolutionRule for UniversalFallbackRule {
    fn name(&self) -> &'static str {
        "Universal Borderless Fallback Rule"
    }

    fn evaluate(&self, _env: &EnvironmentInfo, plan: &mut AttachmentPlan) -> bool {
        // Universal fallback rule always matches as safety net
        plan.push_strategy(AttachmentStrategy::FallbackWindow);
        plan.matched_rules.push(self.name().to_string());
        plan.evaluated_rules.push((self.name().to_string(), true));
        true
    }
}
