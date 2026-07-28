use std::ffi::CString;
use std::os::raw::{c_int, c_void};
use std::ptr;

pub struct SurfaceHandle {
    pub ptr: *mut u8,
    pub size: usize,
}

pub trait SurfaceOps {
    fn acquire(&self) -> Result<SurfaceHandle, String>;
    fn release(&self, handle: SurfaceHandle);
}

pub struct ShmSurface {
    fd: c_int,
    size: usize,
    ptr: *mut c_void,
}

unsafe impl Send for ShmSurface {}
unsafe impl Sync for ShmSurface {}

impl ShmSurface {
    /// Creates or opens a POSIX shared memory object.
    /// If `create` is true, it creates it with the given size.
    /// If `create` is false, it opens an existing one.
    pub fn new(name: &str, size: usize, create: bool) -> Result<Self, String> {
        let c_name = CString::new(name).map_err(|e| e.to_string())?;
        
        let fd = unsafe {
            let oflag = if create {
                libc::O_CREAT | libc::O_RDWR
            } else {
                libc::O_RDWR
            };
            let mode = libc::S_IRUSR | libc::S_IWUSR;
            libc::shm_open(c_name.as_ptr(), oflag, mode as libc::c_uint)
        };

        if fd < 0 {
            return Err(format!("shm_open failed: {}", std::io::Error::last_os_error()));
        }

        let mut actual_size = size;
        if create {
            let res = unsafe { libc::ftruncate(fd, size as libc::off_t) };
            if res < 0 {
                unsafe { libc::close(fd) };
                return Err(format!("ftruncate failed: {}", std::io::Error::last_os_error()));
            }
        } else {
            // Retrieve size from fstat if we are just opening it
            let mut stat: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe { libc::fstat(fd, &mut stat) } < 0 {
                unsafe { libc::close(fd) };
                return Err(format!("fstat failed: {}", std::io::Error::last_os_error()));
            }
            actual_size = stat.st_size as usize;
            if actual_size == 0 {
                unsafe { libc::close(fd) };
                return Err("SHM size is 0".to_string());
            }
        }

        let ptr = unsafe {
            libc::mmap(
                ptr::null_mut(),
                actual_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            unsafe { libc::close(fd) };
            return Err(format!("mmap failed: {}", std::io::Error::last_os_error()));
        }

        Ok(Self { fd, size: actual_size, ptr })
    }

    pub fn is_valid(&self) -> bool {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(self.fd, &mut stat) } < 0 {
            return false;
        }
        stat.st_size as usize == self.size
    }
}

impl Drop for ShmSurface {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.ptr, self.size);
            libc::close(self.fd);
        }
    }
}

impl SurfaceOps for ShmSurface {
    fn acquire(&self) -> Result<SurfaceHandle, String> {
        // For SHM, acquiring just returns the mapped pointer.
        // Synchronization (e.g. semaphores) can be added here later if needed.
        Ok(SurfaceHandle {
            ptr: self.ptr as *mut u8,
            size: self.size,
        })
    }

    fn release(&self, _handle: SurfaceHandle) {
        // No-op for basic SHM.
    }
}

use nativis_core::contract::{Frame, FrameSink, MediaError};
use nativis_core::resource::{ResourceManager, CpuBuffer};
use nativis_protocol::{
    NativisFrameHeader, NativisAttachment, NATIVIS_MAGIC,
    NATIVIS_ATTACHMENT_USAGE_COLOR, NATIVIS_FORMAT_RGBA8888,
};

pub struct ShmSink {
    surface: ShmSurface,
    resources: ResourceManager,
    frame_count: u64,
}

impl ShmSink {
    pub fn new(name: &str, size: usize, resources: ResourceManager) -> Result<Self, String> {
        let surface = ShmSurface::new(name, size, true)?;
        Ok(Self { surface, resources, frame_count: 0 })
    }
}

impl FrameSink for ShmSink {
    fn submit(&mut self, frame: Frame) -> Result<(), MediaError> {
        let handle = self.surface.acquire().map_err(|e| MediaError::GpuUpload(e))?;
        
        let mut frame_count_increment = 0;
        let success = self.resources.acquire(frame.resource, |res| {
            if let Some(cpu) = res.as_any().downcast_ref::<CpuBuffer>() {
                let header_size = std::mem::size_of::<NativisFrameHeader>();
                let att_size = std::mem::size_of::<NativisAttachment>();
                let data_offset = (header_size + att_size) as u32;

                let header = NativisFrameHeader {
                    magic: NATIVIS_MAGIC,
                    version: 2,
                    frame_id: self.frame_count,
                    timestamp: frame.pts.as_millis() as u64,
                    attachment_count: 1,
                    attachment_offset: header_size as u32,
                };
                
                let attachment = NativisAttachment {
                    usage: NATIVIS_ATTACHMENT_USAGE_COLOR,
                    format: NATIVIS_FORMAT_RGBA8888,
                    width: frame.width,
                    height: frame.height,
                    stride: frame.width * 4,
                    planes: 1,
                    surface_index: 0,
                    data_offset,
                };

                // Write to SHM
                unsafe {
                    let ptr = handle.ptr;
                    std::ptr::copy_nonoverlapping(&header as *const _ as *const u8, ptr, header_size);
                    std::ptr::copy_nonoverlapping(&attachment as *const _ as *const u8, ptr.add(header_size), att_size);
                    
                    let data_ptr = ptr.add(data_offset as usize);
                    let available_size = handle.size.saturating_sub(data_offset as usize);
                    std::ptr::copy_nonoverlapping(
                        cpu.data.as_ptr(),
                        data_ptr,
                        std::cmp::min(cpu.data.len(), available_size)
                    );
                }
                frame_count_increment = 1;
                true
            } else {
                false
            }
        }).unwrap_or(false);

        self.frame_count += frame_count_increment;

        self.surface.release(handle);
        
        if !success {
            return Err(MediaError::GpuUpload("Invalid resource type or handle for ShmSink".into()));
        }
        
        Ok(())
    }
}
