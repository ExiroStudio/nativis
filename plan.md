# Nativis Engine: Advanced Contract-Driven Architecture (v2.0)

---

## 1. Engine Core Philosophy: Contract-Driven Architecture

In Nativis v2, **the engine is not a collection of concrete classes; it is a system of immutable API contracts**. Implementations of video decoders, renderers, asset loaders, and window backends can be swapped out at runtime without modifying engine logic.

### 1.1 Complete Engine API Contract Diagram

```
+--------------------------------------------------------------------------------------------------------+
|                                        MEDIA CONTRACT FLOW                                             |
|                                                                                                        |
|   +-------------------+    acquire_frame()    +------------------+   export_gpu()   +----------------+ |
|   |   IMediaSource    | ====================> |   VideoFrame     | ===============> | TextureHandle  | |
|   +-------------------+                       +------------------+                  +----------------+ |
+---------------------------------------------------------------------------------------------|----------+
                                                                                              |
+---------------------------------------------------------------------------------------------v----------+
|                                       RENDER GRAPH CONTRACT FLOW                                       |
|                                                                                                        |
|   +-------------------+   build_passes()   +------------------+    compile()    +------------------+   |
|   |  RenderGraphNode  | ================>  |   RenderGraph    | ==============> | CommandBuffer    |   |
|   +-------------------+                    +------------------+                 +------------------+   |
|            |                                        |                                    |             |
|            v                                        v                                    v             |
|   +-------------------+                    +------------------+                 +------------------+   |
|   |     Material      |                    | Transient Alloc  |                 |    GPU Queue     |   |
|   | (Shader+Uniforms) |                    | (VRAM Aliasing)  |                 |   Presentation   |   |
|   +-------------------+                    +------------------+                 +------------------+   |
+--------------------------------------------------------------------------------------------------------+
```

---

## 2. Render Graph Architecture (DAG-Based Frame Execution)

Instead of a hardcoded pipeline (`Render -> PostProcess -> Present`), Nativis uses a **Directed Acyclic Graph (DAG) Render Graph**. Every rendering operation (video YUV conversion, particle simulation, bloom, Gaussian blur, tone mapping, swapchain present) is an isolated **Render Pass Node**.

### 2.1 Render Graph Pipeline Flow

```
                      [Video Decoder Source]
                                |
                                v (NV12 Frame)
               +----------------------------------+
               | Pass 1: YUV Color Space Convert  |
               +----------------------------------+
                                |
                                v (RGBA Scene Texture)
               +----------------------------------+
               | Pass 2: Scene & Material Render  |
               +----------------------------------+
                                |
             +------------------+------------------+
             |                                     |
             v                                     v
  +--------------------+                 +--------------------+
  | Pass 3A: Bloom     |                 | Pass 3B: Blur      |
  | High-Pass & Down   |                 | Horizontal/Vert    |
  +--------------------+                 +--------------------+
             |                                     |
             +------------------+------------------+
                                |
                                v (Combine Textures)
               +----------------------------------+
               | Pass 4: Tone Mapping & Composite |
               +----------------------------------+
                                |
                                v (Final Frame)
               +----------------------------------+
               | Pass 5: Swapchain Present        |
               +----------------------------------+
```

### 2.2 Render Graph API Rust Contract

```rust
pub type ResourceId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceAccess {
    Read,
    Write,
    ReadWrite,
}

pub struct PassResourceBinding {
    pub resource_id: ResourceId,
    pub access: ResourceAccess,
}

pub trait IRenderPassNode: Send + Sync {
    fn name(&self) -> &'static str;
    
    /// Declare inputs and outputs for dependency resolution and transient VRAM allocation
    fn declare_resources(&self, builder: &mut RenderPassBuilder);
    
    /// Encode draw/compute commands into the CommandBufferBuilder
    fn execute(
        &self,
        ctx: &RenderContext,
        cmd_builder: &mut CommandBufferBuilder,
        resources: &RenderGraphResources,
    );
}

pub struct RenderPassBuilder<'a> {
    pub inputs: Vec<ResourceId>,
    pub outputs: Vec<ResourceId>,
    pub graph: &'a mut RenderGraph,
}

impl<'a> RenderPassBuilder<'a> {
    pub fn read_texture(&mut self, handle: TextureHandle) -> ResourceId {
        let id = self.graph.register_texture(handle);
        self.inputs.push(id);
        id
    }

    pub fn create_transient_texture(&mut self, desc: &TextureDescriptor) -> ResourceId {
        let id = self.graph.create_transient_resource(desc);
        self.outputs.push(id);
        id
    }
}
```

### 2.3 Transient VRAM Allocation & Memory Aliasing
- **Transient Resource Optimization**: Intermediate textures (e.g. Bloom Downsample 1, Blur Scratch Buffer) are allocated only during execution of their target passes.
- **VRAM Memory Aliasing**: If `Pass A` (Bloom) finishes writing its output and `Pass C` (Distortion) runs later, the graph compiler reuses `Pass A`'s underlying VRAM allocation for `Pass C`'s scratch buffer, drastically reducing VRAM usage.

---

## 3. Asset Pipeline & Cooking System

Raw asset files (`.png`, `.jpg`, `.mp4`, `.wav`, `.glsl`) are **never parsed directly inside the render loop**. Nativis utilizes an explicit **Asset Pipeline**.

### 3.1 Asset Pipeline Architecture

```
+-----------------------------------------------------------------------------------+
|                               OFFLINE / IMPORT TIME                               |
|                                                                                   |
| Raw Source Asset           Importer Module               Cooked Binary Asset      |
|  [my_image.png] ========> [ImageImporter] ============> [texture_0.tex]           |
|                            - ASTC/BC7 Compress           - Header Metadata        |
|                            - Mipmap Generation            - GPU Format Descriptor |
|                            - SIMD Pre-swizzle             - Binary Raw Data Payload|
+-----------------------------------------------------------------------------------+
                                                                   |
                                                                   v
+-----------------------------------------------------------------------------------+
|                                    RUNTIME                                        |
|                                                                                   |
| Cooked Asset               Asset Manager                 GPU Fast Import          |
|  [texture_0.tex] =======> [ResourceManager] ============> [Direct GPU Memory]     |
|                            - Direct Async Stream          - Zero CPU Parsing      |
|                            - Handle Management            - Instant Bind          |
+-----------------------------------------------------------------------------------+
```

### 3.2 Asset Importer Contract Interface

```rust
pub struct AssetMetadata {
    pub uuid: u128,
    pub asset_type: AssetType,
    pub source_path: String,
    pub hash: u64,
}

pub trait IAssetImporter: Send + Sync {
    fn supported_extensions(&self) -> &[&'static str];
    
    /// Cook raw file bytes into optimized runtime binary format
    fn import(
        &self,
        source_bytes: &[u8],
        options: &ImportOptions,
    ) -> Result<CookedAssetPayload, ImportError>;
}

pub struct CookedAssetPayload {
    pub header: AssetHeader,
    pub gpu_bytes: Vec<u8>,
}
```

---

## 4. Hybrid ECS & Data-Oriented Scene Graph

To support wallpapers with 5,000+ interactive particles, hundreds of vector layers, and complex shader instances without performance degradation, Nativis implements a **Data-Oriented Archetype ECS** coupled with a hierarchical **Transform System**.

### 4.1 Hybrid Architecture Layout

```
                 HIERARCHICAL SCENE TREE (Spatial Relations)
                                 [Root Node]
                                      |
                     +----------------+----------------+
                     |                                 |
              [Layer: Background]               [Layer: Particles]
                     |                                 |
           (Entity ID: 101)                   (Entity ID: 102 - 5102)

                                      |
                                      v
                ECS DATA-ORIENTED STORAGE (Cache-Friendly Arrays)
+-----------------------------------------------------------------------------------+
| Archetype A: [EntityID | Position Vec3 | Velocity Vec3 | Scale Vec2]               |
| Array Memory: [102, 103, 104...] [Pos1, Pos2...] [Vel1, Vel2...]                 |
+-----------------------------------------------------------------------------------+
| Archetype B: [EntityID | MaterialHandle | TransformMatrix | MeshHandle]           |
| Array Memory: [101...] [Mat1...] [MatMatrix1...] [Mesh1...]                      |
+-----------------------------------------------------------------------------------+
```

### 4.2 ECS Data Structures & Systems

```rust
pub struct Entity(pub u32, pub u32); // Index + Generation

pub struct TransformComponent {
    pub local_matrix: Mat4,
    pub world_matrix: Mat4,
    pub parent: Option<Entity>,
}

pub struct MaterialComponent {
    pub material_handle: MaterialHandle,
    pub layer_index: i32,
}

pub struct ParticleComponent {
    pub lifetime_sec: f32,
    pub velocity: Vec3,
}

pub trait IEcsSystem {
    fn update(&mut self, world: &mut World, delta_time_sec: f32);
}
```

---

## 5. Timeline & Animation System

Wallpapers rely heavily on keyframe animations that interpolate properties over time.

### 5.1 Timeline Pipeline

```
  [Timeline Clock]
         |
         v (Time: 00:03.50)
  +-----------------------------------------------------------------------+
  |                          TRACK EVALUATOR                              |
  |  Track 1 (Opacity)     : Keyframe(0.0, 0.0) -> Keyframe(5.0, 1.0)      |
  |                          Interpolation: Cubic Bezier(0.42, 0.0, 0.58, 1.0)
  |                          Result: 0.72                                 |
  |                                                                       |
  |  Track 2 (Blur Radius) : Keyframe(0.0, 10.0) -> Keyframe(5.0, 0.0)     |
  |                          Interpolation: Linear                        |
  |                          Result: 3.0                                  |
  +-----------------------------------------------------------------------+
                                     |
                                     v
  +-----------------------------------------------------------------------+
  |                          PROPERTY BINDING                             |
  |  - Bind 0.72 -> MaterialComponent.uniforms["u_opacity"]               |
  |  - Bind 3.0  -> RenderGraphNode("BlurPass").uniforms["u_radius"]        |
  +-----------------------------------------------------------------------+
```

### 5.2 Timeline & Animation Rust Contracts

```rust
#[derive(Debug, Clone, Copy)]
pub enum EasingFunction {
    Linear,
    Step,
    CubicBezier { x1: f32, y1: f32, x2: f32, y2: f32 },
}

pub struct Keyframe<T> {
    pub time_sec: f32,
    pub value: T,
    pub easing: EasingFunction,
}

pub struct PropertyTrack<T> {
    pub target_entity: Entity,
    pub property_path: &'static str, // e.g. "MaterialComponent.uniforms.u_opacity"
    pub keyframes: Vec<Keyframe<T>>,
}

pub trait IAnimationSystem {
    fn step(&mut self, delta_time_sec: f32);
    fn bind_property(&mut self, track: PropertyTrack<f32>);
}
```

---

## 6. Material System & Pipeline State Management

A `Texture` is not a `Material`. A **Material** encapsulates the complete GPU state required to render geometry or pass shaders.

### 6.1 Material Struct Layout

$$\text{Material} = \text{Shader Pipeline} + \text{Uniform Buffer} + \text{Texture Bindings} + \text{Raster State}$$

```
+-----------------------------------------------------------------------------------+
|                                 MATERIAL DEFINITION                               |
|                                                                                   |
|  +--------------------------+  +------------------------+  +-------------------+  |
|  | Shader Program           |  | Render State           |  | Uniform Buffer    |  |
|  | - Vertex Shader (SPIR-V) |  | - Blend Mode (Alpha)   |  | - Color (Vec4)    |  |
|  | - Fragment Shader (SPIR-V)| | - Depth Stencil (Off)  |  | - Speed (Float)   |  |
|  +--------------------------+  | - Cull Mode (None)     |  | - Time (Float)    |  |
|                                +------------------------+  +-------------------+  |
|                                                                                   |
|  +-----------------------------------------------------------------------------+  |
|  | Texture Sampler Bindings                                                    |  |
|  | - Slot 0: BaseColorTexture (TextureHandle)                                  |  |
|  | - Slot 1: AudioSpectrumTexture (TextureHandle)                              |  |
|  +-----------------------------------------------------------------------------+  |
+-----------------------------------------------------------------------------------+
```

### 6.2 Material Rust Contract Interface

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    Opaque,
    AlphaBlend,
    Additive,
    Multiply,
    Screen,
}

pub struct MaterialDescriptor {
    pub shader_handle: ShaderHandle,
    pub blend_mode: BlendMode,
    pub cull_mode: CullMode,
    pub depth_write: bool,
}

pub trait IMaterial {
    fn set_float(&mut self, name: &str, val: f32);
    fn set_vec4(&mut self, name: &str, val: Vec4);
    fn set_texture(&mut self, slot: u32, texture: TextureHandle);
    fn bind(&self, cmd_builder: &mut CommandBufferBuilder);
}
```

---

## 7. Engine Frame Scheduler & Phase Pipeline

The frame life cycle follows a strictly sequenced, deterministic schedule.

### 7.1 Phase Scheduler Pipeline

```
  +-----------------------------------------------------------------------+
  |                     PHASE 1: EVENT POLLING & OS INPUT                 |
  |  - Poll OS Window Events, Input, Display Changes, Audio Device State  |
  |  - Dispatch events to EventBus                                        |
  +-----------------------------------------------------------------------+
                                     |
                                     v
  +-----------------------------------------------------------------------+
  |                   PHASE 2: MEDIA UPDATE & CLOCK TICK                  |
  |  - Advance Master Media Clock                                         |
  |  - Poll Video Decoders & Acquire GPU Video Frame Textures             |
  +-----------------------------------------------------------------------+
                                     |
                                     v
  +-----------------------------------------------------------------------+
  |               PHASE 3: TIMELINE & ANIMATION EVALUATION                |
  |  - Step Timeline Tracks                                               |
  |  - Apply Property Interpolations to ECS Components & Materials        |
  +-----------------------------------------------------------------------+
                                     |
                                     v
  +-----------------------------------------------------------------------+
  |                    PHASE 4: ECS & TRANSFORM SYSTEMS                   |
  |  - Execute Particle Systems & Movement Logic                          |
  |  - Compute Global World Transform Matrices                            |
  +-----------------------------------------------------------------------+
                                     |
                                     v
  +-----------------------------------------------------------------------+
  |                 PHASE 5: RENDER GRAPH COMPILATION & EXEC              |
  |  - Build Pass Nodes & Resolve Pass Dependencies                        |
  |  - Allocate Transient VRAM & Encode Command Buffers                   |
  |  - Submit Command Buffers to GPU Command Queue                        |
  +-----------------------------------------------------------------------+
                                     |
                                     v
  +-----------------------------------------------------------------------+
  |                   PHASE 6: SWAPCHAIN PRESENTATION                     |
  |  - Present Frame to Platform Desktop Layer Window                      |
  +-----------------------------------------------------------------------+
```

---

## 8. Centralized Event System (Event Bus)

To eliminate direct callback coupling between modules, all communication occurs via a decoupled **Publish-Subscribe Event Bus**.

### 8.1 Event Bus Architecture

```
  EVENT PRODUCERS                                             EVENT CONSUMERS
+-----------------------+                                 +-----------------------+
| OS Window System      |                                 | Media Subsystem       |
| Mouse / Keyboard      |                                 | (Pause video on hide) |
| Display Topology      | ---- Dispatch Event ----+       +-----------------------+
| Audio Hardware        |                         |                   ^
+-----------------------+                         v                   |
                                           +--------------+           |
                                           |  EVENT BUS   | ----------+
                                           +--------------+           |
+-----------------------+                         ^                   v
| Power Management      |                         |       +-----------------------+
| (Battery Level Drops) | ------------------------+       | Engine Scheduler      |
+-----------------------+                                 | (Reduce FPS to 15)    |
                                                          +-----------------------+
```

### 8.2 Event Bus Contract Specification

```rust
pub enum EngineEvent {
    WindowResized { width: u32, height: u32 },
    DisplayTopologyChanged { monitor_count: u32 },
    BatteryStateChanged { on_battery: bool, low_power_mode: bool },
    AudioDeviceChanged { device_id: String },
    PointerMoved { position: Vec2 },
    PointerButton { button: MouseButton, pressed: bool },
}

pub type EventCallback = Box<dyn Fn(&EngineEvent) + Send + Sync>;

pub trait IEventBus: Send + Sync {
    fn publish(&self, event: EngineEvent);
    fn subscribe(&self, event_type_id: std::any::TypeId, callback: EventCallback);
}
```

---

## 9. Hot-Swappable Render Hardware Interface (RHI Backend Bridge)

The RHI driver implementation is encapsulated behind a driver interface. The active backend (e.g. Vulkan <-> Direct3D 12) can be re-initialized at runtime without destroying scene graph entities or material descriptors.

### 9.1 RHI Hot-Swap Architecture

```
+-----------------------------------------------------------------------------------+
|                             NATIVIS HIGH-LEVEL SCENE                              |
|          Materials | Textures | Buffers | Render Graph Pass Nodes                 |
+-----------------------------------------------------------------------------------+
                                         |
                                         v
+-----------------------------------------------------------------------------------+
|                         IRhiBackend DRIVER INTERFACE                              |
+-----------------------------------------------------------------------------------+
                         /                       \
                        / (Hot Swap at Runtime)   \
                       v                           v
+-------------------------------+         +-------------------------------+
|  Vulkan Backend Driver        |         | Direct3D 12 Backend Driver    |
|  - VkDevice / VkQueue         |         | - ID3D12Device / CommandQueue |
|  - SPIR-V Pipelines           |         | - DXIL Pipelines              |
+-------------------------------+         +-------------------------------+
```

### 9.2 RHI Driver Contract

```rust
pub trait IRhiBackend: Send + Sync {
    fn backend_type(&self) -> BackendType;
    fn create_texture(&mut self, desc: &TextureDescriptor) -> Result<RawTextureHandle, RhiError>;
    fn create_pipeline(&mut self, desc: &PipelineDescriptor) -> Result<RawPipelineHandle, RhiError>;
    fn create_command_buffer(&mut self) -> Box<dyn ICommandBuffer>;
    fn submit(&mut self, cmd_buffers: &[Box<dyn ICommandBuffer>]);
    fn present(&mut self, swapchain: &SwapchainHandle) -> Result<(), PresentError>;
}
```

---

## 10. Plugin Capability & Security Permission System

To ensure third-party marketplace plugins cannot execute malicious actions, plugins must request **Explicit Capabilities**.

### 10.1 Permission Contract Specification

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    AudioLoopbackCapture,
    CameraStreamAccess,
    NetworkSocketAccess,
    FileSystemRead,
    GpuComputeDispatch,
}

pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub requested_capabilities: Vec<Capability>,
}

pub trait ICapabilitySecurityManager {
    fn request_permission(&self, plugin_id: &str, capability: Capability) -> bool;
    fn revoke_permission(&self, plugin_id: &str, capability: Capability);
}
```

---

## 11. Core Engine API Contracts (Interface Specifications)

### 11.1 `IMediaSource` Contract
```rust
pub trait IMediaSource: Send + Sync {
    fn name(&self) -> &str;
    fn initialize(&mut self, rhi: &mut dyn IRhiBackend) -> Result<(), MediaError>;
    fn update(&mut self, master_clock_ns: u64) -> MediaState;
    fn acquire_frame(&mut self) -> Option<VideoFrame>;
    fn release_frame(&mut self, frame: VideoFrame);
    fn dimensions(&self) -> (u32, u32);
}
```

### 11.2 `IAssetImporter` Contract
```rust
pub trait IAssetImporter: Send + Sync {
    fn asset_type(&self) -> AssetType;
    fn can_import(&self, extension: &str) -> bool;
    fn import(&self, input_path: &Path, output_dir: &Path) -> Result<AssetHandle<CookedAsset>, ImporterError>;
}
```

### 11.3 `IRenderPassNode` Contract
```rust
pub trait IRenderPassNode: Send + Sync {
    fn name(&self) -> &'static str;
    fn setup(&mut self, builder: &mut RenderPassBuilder);
    fn execute(&self, ctx: &RenderContext, cmd: &mut CommandBufferBuilder, resources: &RenderGraphResources);
}
```

### 11.4 `IMaterial` Contract
```rust
pub trait IMaterial: Send + Sync {
    fn shader(&self) -> ShaderHandle;
    fn set_param_float(&mut self, name: &str, val: f32);
    fn set_param_texture(&mut self, name: &str, texture: TextureHandle);
    fn apply_bindings(&self, cmd: &mut CommandBufferBuilder);
}
```

---

## Conclusion

Nativis v2 transitions from a simple frame-player into a **world-class, contract-driven game/rendering engine**. By incorporating:
1. A **DAG Render Graph** with memory aliasing,
2. An offline/import-time **Asset Pipeline**,
3. A data-oriented **ECS + Scene Graph hybrid**,
4. A keyframe **Timeline Animation system**,
5. A formal **Material system**,
6. A phase-based **Scheduler**,
7. A decoupled **Event Bus**,
8. A **Hot-Swappable RHI**,
9. A **Plugin Capability Sandbox**, and
10. Strict **API Interface Contracts**,

the engine is built to scale for years without architectural rewrites.
