use crate::{
    pass::{IRenderPass, PassBuilder, PassExecuteContext},
    resources::{RenderGraphResources, ResourceId},
};
use nativis_rhi::{IRhiBackend, TextureDescriptor};
use tracing::debug;

/// Compiled pass entry, produced after topological sort.
#[allow(dead_code)]
struct CompiledPass {
    pass:       Box<dyn IRenderPass>,
    /// Transient resources to allocate before execution.
    alloc_list: Vec<(ResourceId, TextureDescriptor)>,
    /// Transient resources to "free" (return to alias pool) after execution.
    free_list:  Vec<ResourceId>,
}

/// The DAG Render Graph.
///
/// Usage pattern per frame:
/// ```ignore
/// graph.add_pass(MyPass::new(...));
/// let resources = graph.compile(rhi)?;
/// graph.execute(rhi, &resources);
/// graph.reset();
/// ```
pub struct RenderGraph {
    passes:    Vec<Box<dyn IRenderPass>>,
    next_id:   u32,
}

impl RenderGraph {
    pub fn new() -> Self {
        Self { passes: Vec::new(), next_id: 1 }
    }

    /// Register a pass. Passes are executed in registration order for Phase 1
    /// (full DAG scheduling comes in Phase 2). Returns the resource ID
    /// counter after the pass has been declared.
    pub fn add_pass<P: IRenderPass + 'static>(&mut self, pass: P) {
        let mut builder = PassBuilder::new(self.next_id);
        pass.declare(&mut builder);
        self.next_id = builder.next_id;
        self.passes.push(Box::new(pass));
    }

    /// Compile: allocate transient textures and build the execution list.
    /// Phase 1 uses sequential allocation (no aliasing yet — that requires
    /// lifetime interval analysis added in Phase 2).
    pub fn compile(&self, rhi: &mut dyn IRhiBackend) -> RenderGraphResources {
        let mut resources = RenderGraphResources::new();

        for pass in &self.passes {
            let mut builder = PassBuilder::new(self.next_id);
            pass.declare(&mut builder);

            for (id, maybe_desc) in &builder.outputs {
                if id.index() == u32::MAX {
                    // Surface resource — resolve to the current swapchain texture.
                    resources.texture_map.insert(u32::MAX, rhi.current_surface_texture());
                } else if let Some(desc) = maybe_desc {
                    match rhi.create_texture(desc) {
                        Ok(handle) => { resources.texture_map.insert(id.index(), handle); }
                        Err(e)     => tracing::error!("Transient texture alloc failed: {}", e),
                    }
                }
            }
        }

        resources
    }

    /// Execute all passes in order.
    pub fn execute(&self, rhi: &mut dyn IRhiBackend, resources: &RenderGraphResources) {
        for pass in &self.passes {
            debug!("Executing render pass: {}", pass.name());
            let mut ctx = PassExecuteContext { rhi };
            pass.execute(&mut ctx, resources);
        }
    }

    /// Clear all passes at the end of the frame. Resources persist until
    /// explicitly destroyed; only the pass list is cleared.
    pub fn reset(&mut self) {
        self.passes.clear();
        self.next_id = 1;
    }
}

impl Default for RenderGraph {
    fn default() -> Self { Self::new() }
}
