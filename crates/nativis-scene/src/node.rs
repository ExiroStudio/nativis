use nativis_rhi::TextureHandle;

/// GPU blend modes for compositing layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode { Opaque, AlphaBlend, Additive, Multiply, Screen }

/// Payload describing what a scene node renders.
#[derive(Debug, Clone)]
pub enum SceneNodePayload {
    /// Render a GPU texture through a material.
    MediaTexture {
        texture: TextureHandle,
        material_id: Option<u32>,
    },
    /// Empty container / group node.
    Group,
}

/// A node in the scene graph.
#[derive(Debug, Clone)]
pub struct SceneNode {
    pub id:         u32,
    pub name:       String,
    pub visible:    bool,
    pub blend_mode: BlendMode,
    pub opacity:    f32,
    pub z_index:    i32,
    pub payload:    SceneNodePayload,
    pub children:   Vec<u32>,
}

impl SceneNode {
    pub fn new_media(id: u32, name: &str, texture: TextureHandle) -> Self {
        Self {
            id, name: name.to_string(),
            visible: true, blend_mode: BlendMode::Opaque,
            opacity: 1.0, z_index: 0,
            payload: SceneNodePayload::MediaTexture { texture, material_id: None },
            children: Vec::new(),
        }
    }

    pub fn new_group(id: u32, name: &str) -> Self {
        Self {
            id, name: name.to_string(),
            visible: true, blend_mode: BlendMode::Opaque,
            opacity: 1.0, z_index: 0,
            payload: SceneNodePayload::Group,
            children: Vec::new(),
        }
    }
}
