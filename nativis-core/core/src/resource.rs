use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use crate::contract::ResourceHandle;

/// A media resource, such as a CpuBuffer, Dmabuf, or GPU Texture.
/// Implementations must support downcasting via `Any`.
pub trait Resource: Any + Send + Sync {
    fn as_any(&self) -> &dyn Any;
}

/// A CPU-backed RGBA pixel buffer.
pub struct CpuBuffer {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl Resource for CpuBuffer {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Format piksel untuk buffer planar. Menambah varian di masa depan (P010, dst.)
/// tidak mengubah signature fungsi manapun — hanya menambah satu match arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PixelFormat {
    Nv12,
    // P010 ditambahkan di sini nanti, tanpa ubah struct manapun
}

/// Satu plane data mentah + stride SEBENARNYA (bukan diasumsikan width*N).
#[derive(Clone)]
pub struct PlaneDesc {
    pub data:   Arc<[u8]>,
    pub stride: u32,
}

/// Buffer CPU multi-plane (video). Setara CpuBuffer tapi generik jumlah plane.
pub struct PlanarBuffer {
    pub format:  PixelFormat,
    pub width:   u32,
    pub height:  u32,
    pub planes:  Vec<PlaneDesc>, // NV12 = 2 elemen: [Y, UV]
}

impl Resource for PlanarBuffer {
    fn as_any(&self) -> &dyn Any { self }
}

/// An opaque platform/host GPU device context descriptor (e.g. ID3D11Device* on Windows).
/// Shared by the platform sink with hardware decoders for zero-copy GPU paths (D6).
/// Completely POD and neutral: core does not know graphics API types (D1.3, D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuDeviceResource {
    pub device_ptr: usize,
    pub device_context_ptr: usize,
    pub generation: u64,
}

impl Resource for GpuDeviceResource {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// An opaque GPU-backed texture resource (D3D11 on Windows, DMA-BUF on Linux).
/// Completely neutral across platforms and graphics APIs (D1.3, D3).
pub struct GpuTextureResource {
    pub texture_ptr: usize,
    pub subresource_index: u32,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub keeper: Option<Arc<dyn Any + Send + Sync>>,
}

impl Resource for GpuTextureResource {
    fn as_any(&self) -> &dyn Any {
        self
    }
}




/// Central registry for media resources.
/// Shared between MediaBackends (producers) and FrameSinks (consumers).
#[derive(Clone, Default)]
pub struct ResourceManager {
    resources: Arc<Mutex<HashMap<ResourceHandle, Box<dyn Resource>>>>,
    next_id: Arc<Mutex<u64>>,
}

impl ResourceManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new resource and get an opaque handle.
    pub fn register(&self, resource: Box<dyn Resource>) -> ResourceHandle {
        let mut id_lock = self.next_id.lock().unwrap();
        let id = *id_lock;
        *id_lock += 1;

        let handle = ResourceHandle(id);
        self.resources.lock().unwrap().insert(handle, resource);
        handle
    }

    /// Retrieve a reference to a resource.
    /// The consumer can downcast it via `as_any().downcast_ref::<T>()`.
    pub fn acquire<F, R>(&self, handle: ResourceHandle, f: F) -> Option<R>
    where
        F: FnOnce(&dyn Resource) -> R,
    {
        let lock = self.resources.lock().unwrap();
        lock.get(&handle).map(|res| f(res.as_ref()))
    }
    
    /// Unregister and free a resource.
    pub fn free(&self, handle: ResourceHandle) {
        self.resources.lock().unwrap().remove(&handle);
    }

    /// Reserved handle for the platform host GPU device descriptor.
    pub const GPU_DEVICE_HANDLE: ResourceHandle = ResourceHandle(u64::MAX);

    /// Publish the platform GPU device descriptor (e.g. ID3D11Device on Windows).
    pub fn set_gpu_device(&self, device: GpuDeviceResource) {
        self.resources.lock().unwrap().insert(Self::GPU_DEVICE_HANDLE, Box::new(device));
    }

    /// Retrieve the current platform GPU device descriptor, if registered.
    pub fn gpu_device(&self) -> Option<GpuDeviceResource> {
        let lock = self.resources.lock().unwrap();
        lock.get(&Self::GPU_DEVICE_HANDLE)
            .and_then(|r| r.as_any().downcast_ref::<GpuDeviceResource>())
            .copied()
    }
}
