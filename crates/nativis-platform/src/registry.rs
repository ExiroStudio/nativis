//! Driver loader executing AttachmentPlan strategy sequences.

use tracing::{debug, info, warn};

use crate::backend::{IWallpaperDriver, IWallpaperSession, WallpaperBackendError, WallpaperContext, WallpaperSurface};
use crate::detector::{EnvironmentDetector, EnvironmentInfo};
use crate::plugins::{
    fallback_borderless::BorderlessFallbackPlugin,
    kde_wallpaper_api::KdeWallpaperApiPlugin,
    macos_window_level::MacOsWindowLevelPlugin,
    wayland_layer_shell::WaylandLayerShellPlugin,
    windows_workerw::WindowsWorkerWPlugin,
    x11_ewmh_desktop::X11EwmhDesktopPlugin,
};
use crate::resolver::ResolutionEngine;

/// Loader managing and executing wallpaper presentation driver sessions.
pub struct WallpaperDriverLoader {
    drivers: Vec<Box<dyn IWallpaperDriver>>,
}

impl WallpaperDriverLoader {
    /// Creates an empty driver loader.
    pub fn new() -> Self {
        Self { drivers: Vec::new() }
    }

    /// Creates a loader populated with all built-in core wallpaper drivers.
    pub fn with_default_drivers() -> Self {
        let mut loader = Self::new();
        loader.register(Box::new(KdeWallpaperApiPlugin::new()));
        loader.register(Box::new(WaylandLayerShellPlugin::new()));
        loader.register(Box::new(X11EwmhDesktopPlugin::new()));
        loader.register(Box::new(WindowsWorkerWPlugin::new()));
        loader.register(Box::new(MacOsWindowLevelPlugin::new()));
        loader.register(Box::new(BorderlessFallbackPlugin::new()));
        loader
    }

    /// Registers a wallpaper driver.
    pub fn register(&mut self, driver: Box<dyn IWallpaperDriver>) {
        let meta = driver.metadata();
        debug!(
            "Registered Wallpaper Driver: '{}' (Strategy: {:?}, Priority: {}, Confidence: {:?})",
            meta.name, meta.strategy, meta.priority, meta.confidence
        );
        self.drivers.push(driver);
    }

    /// Detects environment, resolves `AttachmentPlan`, and attaches the highest matching driver.
    pub fn select_and_attach(
        &self,
        window: &winit::window::Window,
        output_index: usize,
    ) -> Result<Box<dyn IWallpaperSession>, WallpaperBackendError> {
        let env = EnvironmentDetector::detect();
        
        let ctx = WallpaperContext {
            surface: WallpaperSurface { window },
            output_index,
            environment: &env,
        };

        self.select_and_attach_with_ctx(&ctx)
    }

    /// Selects session using an explicit context and resolution plan.
    pub fn select_and_attach_with_ctx(
        &self,
        ctx: &WallpaperContext,
    ) -> Result<Box<dyn IWallpaperSession>, WallpaperBackendError> {
        let engine = ResolutionEngine::default_engine();
        let plan = engine.resolve(ctx.environment);

        info!("Executing AttachmentPlan strategies: {:?}", plan.strategies);

        // Iterate over strategies strictly in the order requested by AttachmentPlan
        for target_strategy in &plan.strategies {
            // Find all registered drivers matching this strategy
            let mut matching_drivers: Vec<_> = self.drivers
                .iter()
                .filter(|d| d.metadata().strategy == *target_strategy)
                .collect();

            // Sort matching drivers by priority & confidence if multiple exist
            matching_drivers.sort_by(|a, b| {
                let meta_a = a.metadata();
                let meta_b = b.metadata();
                meta_b.priority.cmp(&meta_a.priority)
                    .then_with(|| meta_b.confidence.cmp(&meta_a.confidence))
            });

            for driver in matching_drivers {
                let meta = driver.metadata();
                info!(
                    "Attempting attachment using driver '{}' (Strategy: {:?})",
                    meta.name, meta.strategy
                );

                let mut session = driver.create_session();
                match session.attach(ctx) {
                    Ok(()) => {
                        info!(
                            "Successfully attached wallpaper session '{}'!",
                            session.name()
                        );
                        return Ok(session);
                    }
                    Err(e) => {
                        warn!(
                            "Driver '{}' attachment failed: {}. Continuing plan...",
                            meta.name, e
                        );
                    }
                }
            }
        }

        Err(WallpaperBackendError::Attach(
            "All candidate drivers in AttachmentPlan failed to attach".into(),
        ))
    }

    pub fn drivers(&self) -> &[Box<dyn IWallpaperDriver>] {
        &self.drivers
    }

    /// Generates a comprehensive, human-readable diagnostic report for debugging.
    pub fn doctor(&self, env: &EnvironmentInfo) -> String {
        let engine = ResolutionEngine::default_engine();
        let plan = engine.resolve(env);

        let mut report = String::new();
        report.push_str("NATIVIS WALLPAPER DOCTOR REPORT\n");
        report.push_str("===============================\n\n");
        
        report.push_str("FACTS\n");
        report.push_str("--------------------------------\n");
        report.push_str(&format!("OS:               {:?}\n", env.os));
        report.push_str(&format!("Display:          {:?}\n", env.display_server));
        report.push_str(&format!("Desktop:          {:?}\n", env.desktop_environment));
        report.push_str(&format!("WM:               {:?}\n", env.window_manager));
        report.push_str(&format!("Session Type:     {}\n\n", env.session_type));

        report.push_str("RULES EVALUATED\n");
        report.push_str("--------------------------------\n");
        for (rule_name, matched) in &plan.evaluated_rules {
            let check = if *matched { "✓" } else { "✗" };
            report.push_str(&format!("{} {}\n", check, rule_name));
        }
        report.push('\n');

        report.push_str("ATTACHMENT PLAN\n");
        report.push_str("--------------------------------\n");
        for (idx, strategy) in plan.strategies.iter().enumerate() {
            report.push_str(&format!("{}. {:?}\n", idx + 1, strategy));
        }
        report.push('\n');

        report.push_str("DRIVERS DISCOVERED\n");
        report.push_str("--------------------------------\n");
        for driver in &self.drivers {
            let meta = driver.metadata();
            let matches_plan = plan.strategies.contains(&meta.strategy);
            let check = if matches_plan { "✓" } else { "✗" };
            report.push_str(&format!("{} {} (Strategy: {:?})\n", check, meta.name, meta.strategy));
        }
        report.push('\n');

        report.push_str("SELECTION & REASON\n");
        report.push_str("--------------------------------\n");

        let mut selected_driver_name = None;
        for target_strategy in &plan.strategies {
            if let Some(driver) = self.drivers.iter().find(|d| d.metadata().strategy == *target_strategy) {
                selected_driver_name = Some((driver.metadata().name.clone(), target_strategy));
                break;
            }
        }

        if let Some((name, strategy)) = selected_driver_name {
            report.push_str(&format!("Selected Driver: {}\n", name));
            report.push_str(&format!("Strategy:        {:?}\n", strategy));
            report.push_str("Reason:          Highest priority driver matching top AttachmentPlan strategy.\n");
        } else {
            report.push_str("Selected Driver: None\n");
            report.push_str("Reason:          No registered driver matches the AttachmentPlan.\n");
        }

        report
    }

    pub fn len(&self) -> usize {
        self.drivers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.drivers.is_empty()
    }
}

impl Default for WallpaperDriverLoader {
    fn default() -> Self {
        Self::with_default_drivers()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{
        DesktopEnvironment, DisplayServer, OperatingSystem, WindowManager,
    };

    #[test]
    fn test_doctor_comprehensive_output() {
        let loader = WallpaperDriverLoader::with_default_drivers();
        let env = EnvironmentInfo {
            os: OperatingSystem::Windows,
            display_server: DisplayServer::Win32,
            desktop_environment: DesktopEnvironment::WindowsExplorer,
            window_manager: WindowManager::DwmWindows,
            session_type: "windows".into(),
            xdg_current_desktop: "".into(),
            desktop_session: "".into(),
        };

        let report = loader.doctor(&env);
        assert!(report.contains("FACTS"));
        assert!(report.contains("RULES EVALUATED"));
        assert!(report.contains("✓ Windows WorkerW Injection Rule"));
        assert!(report.contains("ATTACHMENT PLAN"));
        assert!(report.contains("Selected Driver: windows_workerw"));
    }
}
