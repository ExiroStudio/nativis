use nativis_core::Handle;

/// Opaque marker type for render graph virtual resources (textures/buffers
/// that may or may not have been physically allocated yet).
pub struct VirtualResource;
pub type ResourceId = Handle<VirtualResource>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceAccess { Read, Write, ReadWrite }

/// Resolved resource table available during pass execution.
/// Maps virtual resource IDs to concrete `TextureHandle`s.
pub struct RenderGraphResources {
    pub(crate) texture_map: std::collections::HashMap<u32, nativis_rhi::TextureHandle>,
}

impl RenderGraphResources {
    pub(crate) fn new() -> Self {
        Self { texture_map: Default::default() }
    }

    /// Look up the `TextureHandle` that was allocated for a virtual resource.
    pub fn texture(&self, id: ResourceId) -> Option<nativis_rhi::TextureHandle> {
        self.texture_map.get(&id.index()).copied()
    }
}
