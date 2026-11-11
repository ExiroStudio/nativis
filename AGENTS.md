# Instructions for AI coding agents

Read [`docs/DECISIONS.md`](docs/DECISIONS.md) **before** changing `nativis-core`, any contract
(`MediaBackend`, `FrameSink`, `Platform`, `Resource`), or adding a plugin or platform.

The short version:

1. **Plugin-first on both axes.** Sources (image, video, HTML, programs, ...) and platforms
   (Windows, KDE, Wayland, ...) are plugins. Core never names a concrete backend or platform
   and never contains platform/graphics-API types. A new source or platform must be addable
   without editing core, the runtime, or any other plugin/platform.
2. **Not a media player.** The engine abstraction is a surface producer.
3. **C-compatible, POD, versioned ABI** across plugin boundaries: no `Arc`/`Vec`/`dyn`/`String`,
   no `TypeId` downcasting between plugins, buffers freed by their allocator via a release call.
4. **Platforms stay isolated** (`docs/PLATFORM_INDEPENDENCE.md`): duplication beats a fragile
   shared abstraction.
5. **Lower CPU/RAM beats single-binary purity**; a self-relaunching single executable is fine.
6. **Hardware decode with a CPU-readback baseline.** GPU zero-copy is a negotiated optimization,
   never the only path.
7. **Evidence before fixes, no overengineering, no silent fallbacks** (log the reason).

If a requested change contradicts one of these, stop and ask the maintainer. Analysis and
proposals that are not yet decisions are in `docs/ARCHITECTURE_REVIEW_2026-11.md`.
