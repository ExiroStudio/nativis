use crate::node::{SceneNode, SceneNodePayload};
use std::collections::HashMap;

/// Flat, pool-based scene graph. Nodes are stored in a `HashMap` for O(1)
/// lookup by ID. The root node ID is always 0.
pub struct SceneGraph {
    nodes:   HashMap<u32, SceneNode>,
    root_id: u32,
    next_id: u32,
}

impl SceneGraph {
    pub fn new() -> Self {
        let mut nodes = HashMap::new();
        nodes.insert(0, SceneNode::new_group(0, "Root"));
        Self { nodes, root_id: 0, next_id: 1 }
    }

    pub fn add_node(&mut self, mut node: SceneNode) -> u32 {
        let id = self.next_id;
        node.id = id;
        self.next_id += 1;
        self.nodes.insert(id, node);
        // Auto-parent to root
        if let Some(root) = self.nodes.get_mut(&self.root_id) {
            root.children.push(id);
        }
        id
    }

    pub fn node(&self, id: u32) -> Option<&SceneNode> { self.nodes.get(&id) }
    pub fn node_mut(&mut self, id: u32) -> Option<&mut SceneNode> { self.nodes.get_mut(&id) }

    pub fn root_id(&self) -> u32 { self.root_id }

    /// Collect all visible leaf nodes in Z-index order for rendering.
    pub fn collect_render_nodes(&self) -> Vec<&SceneNode> {
        let mut result = Vec::new();
        self.collect_recursive(self.root_id, &mut result);
        result.sort_by_key(|n| n.z_index);
        result
    }

    fn collect_recursive<'a>(&'a self, id: u32, out: &mut Vec<&'a SceneNode>) {
        if let Some(node) = self.nodes.get(&id) {
            if !node.visible { return; }
            match &node.payload {
                SceneNodePayload::Group => {}
                _ => out.push(node),
            }
            for &child_id in &node.children {
                self.collect_recursive(child_id, out);
            }
        }
    }
}

impl Default for SceneGraph { fn default() -> Self { Self::new() } }
