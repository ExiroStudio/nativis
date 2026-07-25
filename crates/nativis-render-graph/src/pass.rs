use crate::resources::{ResourceId, ResourceAccess, RenderGraphResources};
use nativis_core::Handle;
use nativis_rhi::{IRhiBackend, TextureDescriptor};

/// A pass node declares its resource requirements and then encodes GPU work.
/// Passes **never** create or destroy textures directly — all resource
/// allocation flows through the `RenderGraph` compiler.
pub trait IRenderPass: Send + Sync {
    fn name(&self) -> &'static str;

    /// Called at graph compile time. The pass declares which virtual resources
    /// it reads and writes. The graph resolves lifetimes and aliases VRAM.
    fn declare(&self, builder: &mut PassBuilder);

    /// Called at graph execute time with resolved resources and RHI access.
    fn execute(&self, ctx: &mut PassExecuteContext, resources: &RenderGraphResources);
}

// ── PassBuilder ───────────────────────────────────────────────────────────────

pub struct PassBuilder {
    pub(crate) inputs:  Vec<(ResourceId, ResourceAccess)>,
    pub(crate) outputs: Vec<(ResourceId, Option<TextureDescriptor>)>,
    pub(crate) next_id: u32,
}

impl PassBuilder {
    pub(crate) fn new(next_id: u32) -> Self {
        Self { inputs: Vec::new(), outputs: Vec::new(), next_id }
    }

    /// Declare that this pass reads an existing resource (e.g. a source texture
    /// produced by a media decoder or a previous pass).
    pub fn read(&mut self, id: ResourceId) -> ResourceId {
        self.inputs.push((id, ResourceAccess::Read));
        id
    }

    /// Create a new transient texture that this pass writes into.
    /// The graph compiler allocates (and potentially aliases) its VRAM.
    pub fn create_transient(&mut self, desc: TextureDescriptor) -> ResourceId {
        let id = Handle::new(self.next_id, 0);
        self.next_id += 1;
        self.outputs.push((id, Some(desc)));
        id
    }

    /// Declare this pass writes to the swapchain surface directly.
    pub fn write_surface(&mut self) -> ResourceId {
        // Surface is always resource ID u32::MAX
        let id = Handle::new(u32::MAX, 0);
        self.outputs.push((id, None));
        id
    }
}

// ── PassExecuteContext ────────────────────────────────────────────────────────

/// Passed to `execute()`. Provides RHI access and the current frame's surface
/// texture handle so passes can encode draw commands.
pub struct PassExecuteContext<'a> {
    pub rhi: &'a mut dyn IRhiBackend,
}
