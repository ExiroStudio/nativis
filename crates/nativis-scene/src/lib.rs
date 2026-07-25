//! nativis-scene — Scene graph, materials, and scene-to-render-graph compilation.

pub mod material;
pub mod node;
pub mod graph;

pub use material::{Material, BlitPass};
pub use node::{SceneNode, SceneNodePayload, BlendMode};
pub use graph::SceneGraph;
