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
}
