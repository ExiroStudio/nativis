//! nativis-render-graph — DAG-based Render Graph.
//!
//! Each rendering operation is an `IRenderPass` node that declares its
//! resource inputs and outputs. The `RenderGraph` compiles these nodes into
//! a topologically sorted execution list and allocates transient VRAM only
//! for the passes that need it, with memory aliasing across non-overlapping
//! lifetimes.

pub mod graph;
pub mod pass;
pub mod resources;

pub use graph::RenderGraph;
pub use pass::{IRenderPass, PassBuilder, PassExecuteContext};
pub use resources::{ResourceId, ResourceAccess, RenderGraphResources};
