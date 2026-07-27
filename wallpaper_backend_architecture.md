# Nativis Wallpaper Backend Architecture Specification

> **Document Version**: 3.0.0 (Rule-Driven Capability Architecture)  
> **Status**: Approved Architectural Standard  
> **Target Subsystem**: Wallpaper Backend (`nativis-platform`)

---

## Executive Summary

Nativis uses a decoupled multimedia wallpaper engine design. The **Render Engine** and **Media Backend** contracts are frozen: the Render Engine composite pipeline renders video/image frames onto a native surface target provided by the Wallpaper Backend, completely unconcerned with window placement, OS window manager specifics, or desktop shell hierarchy. 

The **Wallpaper Backend** is strictly responsible for bridging rendered output into the host platform's native desktop wallpaper layer.

This specification defines a **Rule-Driven, Capability-Based Platform Abstraction Pipeline**. Drivers are completely isolated strategy executors with zero knowledge of host OS or Desktop Environment facts.

---

## 1. Engine Pipeline & Separation of Concerns

```
EnvironmentDetector (Fact Collector)
        │
        ▼
EnvironmentInfo Facts (OS, Display, DE, WM)
        │
        ▼
ResolutionEngine (Evaluates ResolutionRules)
        │
        ▼
AttachmentPlan (Execution Plan & Rules Audit Log)
        │
        ▼
WallpaperDriverLoader (Executes Plan Strategies)
        │
        ▼
WallpaperDriver -> WallpaperSession -> WallpaperSurface
        │
        ▼
Render Engine -> Graphics Backend (RHI)
```

---

## 2. Core Subsystems

### 1. Environment Detector (`detector.rs`)
Pure fact collector. Gathers host OS, Display Server, Desktop Environment, and Window Manager details (`EnvironmentInfo`). It performs no compatibility evaluations.

### 2. Resolution Engine & Rules (`rules.rs`, `resolver.rs`)
- `ResolutionRule`: Evaluates `EnvironmentInfo` facts. If matched, it pushes candidate `AttachmentStrategy` targets into the `AttachmentPlan`.
- `AttachmentPlan`: Contains the ordered list of strategy candidates and an audit log of evaluated/matched rules.

### 3. Dumb Wallpaper Drivers (`plugins/*.rs`)
- Drivers (`IWallpaperDriver`) expose metadata describing their strategy (`AttachmentStrategy`), priority, confidence, and bitflag capabilities.
- Drivers contain **zero** host OS/DE checks (`is_compatible` is eliminated). A driver simply advertises: *"I implement AttachmentStrategy X"*.

### 4. Wallpaper Driver Loader (`registry.rs`)
- Receives the `AttachmentPlan` from `ResolutionEngine`.
- Executes strategies in the exact order requested by the plan.
- Instantiates `IWallpaperSession` for the first matching driver strategy that attaches successfully.
- Contains **zero** internal hardcoded strategy priorities.

---

## 3. Surface & Session Lifecycle

- **`WallpaperSurface`**: Abstract surface wrapping native window target.
- **`IWallpaperSession`**: Manages attached state, health (`BackendHealth`), and auto-recovery polling (`NeedsReattach`).
- **Render Engine Isolation**: Presentation is strictly handled by the graphics backend (RHI). Drivers never touch frame drawing or present scheduling.

---

## 4. Diagnostics (`nativis doctor`)

The `WallpaperDriverLoader::doctor()` function produces full visibility across all 5 stages:

```text
NATIVIS WALLPAPER DOCTOR REPORT
===============================

FACTS
--------------------------------
OS:               Linux
Display:          Wayland
Desktop:          KdePlasma
WM:               KWin
Session Type:     wayland

RULES EVALUATED
--------------------------------
✓ KDE Plasma Native API Rule
✓ Wayland Layer Shell Rule
✗ X11 EWMH Stacking Rule
✗ Windows WorkerW Injection Rule
✗ macOS Window Level Rule
✓ Universal Borderless Fallback Rule

ATTACHMENT PLAN
--------------------------------
1. NativeAPI
2. LayerShell
3. FallbackWindow

DRIVERS DISCOVERED
--------------------------------
✓ kde_wallpaper_api (Strategy: NativeAPI)
✓ wayland_layer_shell (Strategy: LayerShell)
✗ x11_ewmh_desktop (Strategy: EwmhDesktop)
✗ windows_workerw (Strategy: WindowInjection)
✗ macos_window_level (Strategy: NativeAPI)
✓ fallback_borderless (Strategy: FallbackWindow)

SELECTION & REASON
--------------------------------
Selected Driver: kde_wallpaper_api
Strategy:        NativeAPI
Reason:          Highest priority driver matching top AttachmentPlan strategy.
```

---

## 5. Conclusion

This Rule-Driven Capability Architecture provides Nativis with an engine-grade platform abstraction. Adding support for future desktop environments or compositors requires only adding a `ResolutionRule` or registering a strategy driver, keeping the core engine and runtime 100% stable.
