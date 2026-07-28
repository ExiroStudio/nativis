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
