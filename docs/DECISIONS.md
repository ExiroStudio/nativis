# Nativis Decision Record

> **Audience: maintainers and AI coding agents.** Read this file before changing
> `nativis-core`, any contract (`MediaBackend`, `FrameSink`, `Platform`, `Resource`),
> or before adding a plugin or platform. These decisions come from the maintainer.
> If a change you are about to make contradicts one of them, **stop and ask the
> maintainer** instead of working around it.

- Last updated: 2026-11-10
- Scope: durable decisions only. Analysis, evidence and proposals live in
  [`ARCHITECTURE_REVIEW_2026-11.md`](ARCHITECTURE_REVIEW_2026-11.md).

---

## D1 — Nativis is plugin-first on both axes

**Decision.** Wallpaper *sources* (image, video, shader, HTML, other programs, anything
future) and *platforms* (Windows, KDE X11, KDE Wayland, wlroots, GNOME, macOS, ...)
are both plugins. The engine core must keep working, **unchanged**, for any
combination of source and platform.

**Rules that follow.**

1. `nativis-core`, `nativis-runtime` and the conductor never name a concrete backend or
   platform. No `if backend == "video"`, no platform `cfg` branches in core logic.
2. Backends are routed only through `supports()` / the plugin registry.
3. Core must not contain types that belong to one platform or graphics API
   (D3D11, Wayland, Qt, VAAPI, ...). A platform-specific type in `nativis-core` is a bug.
4. Adding a source = adding a plugin crate. Adding a platform = adding a crate under
   `nativis-platform/`. Neither may require edits to core, the runtime, or any *other*
   plugin or platform.

**Acceptance test for any design or PR.** *Can a brand-new source type and a brand-new
platform be added without editing core, the runtime, or an existing plugin/platform?*
If the answer is no, the design is wrong, even if it works today.

## D2 — The engine is not a media player

**Decision.** The abstraction at the engine level is a **surface producer**, not a player.
Video decoding (FFmpeg, libmpv, a platform decoder) is an internal concern of the video
plugin. The core must not grow player concepts (playlists, transport UI, audio device
management). Capabilities such as `Seek` or `Loop` stay optional flags that a source
may or may not advertise.

## D3 — Plugin ABI is C-compatible, POD, versioned

**Decision.** Anything that crosses a plugin or platform boundary is `repr(C)`,
plain-old-data and versioned (vtables with a size/version field, as drafted in the ABI
spec: `NativisPluginVTable`, `NativisPlatformVTable`, `NativisFrameSinkVTable`,
`NativisHostApi`).

**Rules that follow.**

1. No Rust-only types across the boundary: no `Arc`, `Vec`, `Box<dyn ...>`, `String`,
   and no `TypeId`-based downcasting between separately built plugins.
2. A buffer is freed by the side that allocated it, through an explicit release
   callback. Plugins never free memory they did not allocate.
3. New capabilities are added by extension (new `kind` values, new interfaces queried by
   name/version), never by changing the meaning or layout of an existing field.

## D4 — Platforms stay isolated

**Decision.** The rules in [`PLATFORM_INDEPENDENCE.md`](PLATFORM_INDEPENDENCE.md) remain in
force: one crate per platform, duplication is preferred over a fragile shared
abstraction, a fix for platform A must not touch platform B.

*Clarification.* A shared **data contract** in core (for example a surface descriptor) is
not shared implementation. Each platform still implements its own sink.

## D5 — CPU and RAM efficiency outrank single-binary purity

**Decision.** When they conflict, lowering CPU/RAM usage wins over keeping everything in
one self-contained binary. A single executable that re-launches itself in a worker mode
(for example a decoder process) satisfies both and is preferred over multiple files.

## D6 — Hardware decode with a portable CPU baseline

**Decision.** Hardware decode (VAAPI, CUDA/NVDEC, D3D11VA, VideoToolbox) is preferred,
with **CPU readback as the cross-platform baseline** that must always work. Zero-copy GPU
paths are optimizations that are *negotiated* per platform (see the review document) and
must degrade to the baseline automatically; they may never be the only path.

## D7 — Working method

1. **Evidence before fixes.** Measure or reproduce first. Label claims as *verified*,
   *measured* or *inferred*.
2. **Avoid overengineering.** Start with the smallest design that satisfies D1; add
   mechanisms (pools, fences, worker processes) only when a measurement justifies them.
3. **Silent fallbacks are bugs.** If hardware decode, a GPU path or a plugin falls back to
   a slower path, it must log it at `info`/`warn` level with the reason.

---

## Not decided yet

These are proposals in the review document and need an explicit maintainer decision
before anyone implements them:

- The exact surface descriptor layout and the negotiation API.
- Whether and when decoding moves to a separate worker process.
- The per-monitor ("output") model and the meaning of scaling modes.
- Event-driven cadence replacing the fixed 60 Hz poll in the conductor.

## How to change this file

Add a new numbered decision or mark an old one *Superseded by Dn* with a date and the
reason. Do not silently edit a decision to match new code.
