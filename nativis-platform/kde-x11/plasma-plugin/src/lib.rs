use std::os::raw::{c_void, c_int};
use std::ptr;
use nativis_protocol::{NativisFrameHeader, NativisAttachment, NATIVIS_ATTACHMENT_USAGE_COLOR};
use nativis_transport_shm::{ShmSurface, SurfaceOps};

#[no_mangle]
pub extern "C" fn nativis_version() -> u32 {
    2
}

pub struct NativisConsumer {
    shm: Option<ShmSurface>,
    last_frame_id: u64,
    // We keep a fallback buffer just in case the producer hasn't started yet
    fallback_buffer: Vec<u8>,
    width: usize,
    height: usize,
    active_ptr: *mut u8,
}

impl NativisConsumer {
    fn new() -> Self {
        Self {
            shm: ShmSurface::new("/nativis_shm", 0, false).ok(),
            last_frame_id: 0,
            fallback_buffer: Vec::new(),
            width: 0,
            height: 0,
            active_ptr: ptr::null_mut(),
        }
    }

    fn ensure_shm(&mut self) {
        let needs_reopen = match &self.shm {
            Some(s) => !s.is_valid(),
            None => true,
        };
        if needs_reopen {
            self.shm = ShmSurface::new("/nativis_shm", 0, false).ok();
        }
    }
}

#[no_mangle]
pub extern "C" fn nativis_create() -> *mut c_void {
    let runtime = Box::new(NativisConsumer::new());
    Box::into_raw(runtime) as *mut c_void
}

#[no_mangle]
pub extern "C" fn nativis_destroy(ctx: *mut c_void) {
    if !ctx.is_null() {
        unsafe {
            let _ = Box::from_raw(ctx as *mut NativisConsumer);
        }
    }
}

#[no_mangle]
pub extern "C" fn nativis_begin_frame(ctx: *mut c_void, width: c_int, height: c_int) -> bool {
    if ctx.is_null() || width <= 0 || height <= 0 {
        return false;
    }
    
    let runtime = unsafe { &mut *(ctx as *mut NativisConsumer) };
    runtime.width = width as usize;
    runtime.height = height as usize;
    
    runtime.ensure_shm();
    
    true
}

#[no_mangle]
pub extern "C" fn nativis_get_pixels(ctx: *mut c_void) -> *mut u8 {
    if ctx.is_null() {
        return ptr::null_mut();
    }
    let runtime = unsafe { &mut *(ctx as *mut NativisConsumer) };
    
    runtime.active_ptr = ptr::null_mut();
    runtime.ensure_shm();
    
    if let Some(shm) = &runtime.shm {
        if let Ok(handle) = shm.acquire() {
            let ptr = handle.ptr;
            if handle.size >= std::mem::size_of::<NativisFrameHeader>() {
                let header = unsafe { &*(ptr as *const NativisFrameHeader) };
                if header.magic == nativis_protocol::NATIVIS_MAGIC {
                    runtime.last_frame_id = header.frame_id;
                    let offset = header.attachment_offset as usize;
                    if handle.size >= offset + (header.attachment_count as usize * std::mem::size_of::<NativisAttachment>()) {
                        let attachments_ptr = unsafe { ptr.add(offset) as *const NativisAttachment };
                        let attachments = unsafe { std::slice::from_raw_parts(attachments_ptr, header.attachment_count as usize) };
                        for att in attachments {
                            if att.usage == NATIVIS_ATTACHMENT_USAGE_COLOR {
                                runtime.active_ptr = unsafe { ptr.add(att.data_offset as usize) };
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    
    if runtime.active_ptr.is_null() {
        // Provide black fallback
        let expected_size = runtime.width * runtime.height * 4;
        if runtime.fallback_buffer.len() != expected_size {
            runtime.fallback_buffer.resize(expected_size, 0);
        }
        return runtime.fallback_buffer.as_mut_ptr();
    }
    
    runtime.active_ptr
}

#[no_mangle]
pub extern "C" fn nativis_render(_ctx: *mut c_void) {
    // No-op in V2, pixel extraction is done in nativis_get_pixels
}

#[no_mangle]
pub extern "C" fn nativis_get_width(ctx: *mut c_void) -> c_int {
    if ctx.is_null() { return 0; }
    let runtime = unsafe { &mut *(ctx as *mut NativisConsumer) };
    runtime.ensure_shm();
    
    if let Some(shm) = &runtime.shm {
        if let Ok(handle) = shm.acquire() {
            let ptr = handle.ptr;
            if handle.size >= std::mem::size_of::<NativisFrameHeader>() {
                let header = unsafe { &*(ptr as *const NativisFrameHeader) };
                if header.magic == nativis_protocol::NATIVIS_MAGIC {
                    let offset = header.attachment_offset as usize;
                    if handle.size >= offset + (header.attachment_count as usize * std::mem::size_of::<NativisAttachment>()) {
                        let attachments_ptr = unsafe { ptr.add(offset) as *const NativisAttachment };
                        let attachments = unsafe { std::slice::from_raw_parts(attachments_ptr, header.attachment_count as usize) };
                        for att in attachments {
                            if att.usage == NATIVIS_ATTACHMENT_USAGE_COLOR && att.surface_index == 0 {
                                return att.width as c_int;
                            }
                        }
                    }
                }
            }
        }
    }
    runtime.width as c_int
}

#[no_mangle]
pub extern "C" fn nativis_get_height(ctx: *mut c_void) -> c_int {
    if ctx.is_null() { return 0; }
    let runtime = unsafe { &mut *(ctx as *mut NativisConsumer) };
    runtime.ensure_shm();
    
    if let Some(shm) = &runtime.shm {
        if let Ok(handle) = shm.acquire() {
            let ptr = handle.ptr;
            if handle.size >= std::mem::size_of::<NativisFrameHeader>() {
                let header = unsafe { &*(ptr as *const NativisFrameHeader) };
                if header.magic == nativis_protocol::NATIVIS_MAGIC {
                    let offset = header.attachment_offset as usize;
                    if handle.size >= offset + (header.attachment_count as usize * std::mem::size_of::<NativisAttachment>()) {
                        let attachments_ptr = unsafe { ptr.add(offset) as *const NativisAttachment };
                        let attachments = unsafe { std::slice::from_raw_parts(attachments_ptr, header.attachment_count as usize) };
                        for att in attachments {
                            if att.usage == NATIVIS_ATTACHMENT_USAGE_COLOR && att.surface_index == 0 {
                                return att.height as c_int;
                            }
                        }
                    }
                }
            }
        }
    }
    runtime.height as c_int
}

#[no_mangle]
pub extern "C" fn nativis_end_frame(_ctx: *mut c_void) {
}

#[no_mangle]
pub extern "C" fn nativis_get_frame_id(ctx: *mut c_void) -> u64 {
    if ctx.is_null() { return 0; }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };

    // Read frame_id directly from SHM — do NOT use last_frame_id.
    //
    // last_frame_id is a cache updated only inside nativis_get_pixels(),
    // which only runs when the render thread calls updatePaintNode().
    // If the FrameWatcher reads last_frame_id, it sees a stale value:
    //
    //   old runtime stops  → last_frame_id = 1000
    //   new runtime starts → SHM frame_id resets to 0, 1, 2...
    //   FrameWatcher reads last_frame_id = 1000 (forever)
    //   1000 == lastSeen(1000) → no signal → wallpaper never changes
    //
    // Reading SHM directly fixes this: the watcher always sees the live value.
    if let Some(shm) = &runtime.shm {
        if let Ok(handle) = shm.acquire() {
            if handle.size >= std::mem::size_of::<NativisFrameHeader>() {
                let header = unsafe { &*(handle.ptr as *const NativisFrameHeader) };
                if header.magic == nativis_protocol::NATIVIS_MAGIC {
                    return header.frame_id;
                }
            }
        }
    }

    // SHM not available yet — fall back to cached value
    runtime.last_frame_id
}

// ── Fase 5: Generic plane-based FFI ─────────────────────────────────────────
//
// These functions allow the C++ consumer to access NV12 (or any multi-plane)
// data by index, without needing a new FFI function per plane type.
// `nativis_get_pixels` is preserved for RGBA backward compat.

/// Helper: read color attachments from SHM.
/// Returns None if SHM is not ready or magic mismatch.
unsafe fn read_color_attachments(runtime: &NativisConsumer) -> Option<(*mut u8, Vec<NativisAttachment>)> {
    let shm = runtime.shm.as_ref()?;
    let handle = shm.acquire().ok()?;
    let ptr = handle.ptr;
    if handle.size < std::mem::size_of::<NativisFrameHeader>() {
        return None;
    }
    let header = &*(ptr as *const NativisFrameHeader);
    if header.magic != nativis_protocol::NATIVIS_MAGIC {
        return None;
    }
    let offset = header.attachment_offset as usize;
    let count = header.attachment_count as usize;
    if handle.size < offset + count * std::mem::size_of::<NativisAttachment>() {
        return None;
    }
    let att_ptr = ptr.add(offset) as *const NativisAttachment;
    let all = std::slice::from_raw_parts(att_ptr, count);
    let color_atts: Vec<NativisAttachment> = all.iter()
        .filter(|a| a.usage == NATIVIS_ATTACHMENT_USAGE_COLOR)
        .copied()
        .collect();
    Some((ptr, color_atts))
}

/// Returns the pixel format of the current frame (NATIVIS_FORMAT_* constant).
/// Returns 0 if no frame is available.
#[no_mangle]
pub extern "C" fn nativis_get_format(ctx: *mut c_void) -> u32 {
    if ctx.is_null() { return 0; }
    let runtime = unsafe { &mut *(ctx as *mut NativisConsumer) };
    runtime.ensure_shm();
    
    unsafe {
        if let Some((_ptr, atts)) = read_color_attachments(runtime) {
            if let Some(first) = atts.first() {
                return first.format;
            }
        }
    }
    0
}

/// Returns the number of color planes in the current frame.
/// RGBA = 1, NV12 = 2, YUV420P = 3, etc.
#[no_mangle]
pub extern "C" fn nativis_get_plane_count(ctx: *mut c_void) -> u32 {
    if ctx.is_null() { return 0; }
    let runtime = unsafe { &mut *(ctx as *mut NativisConsumer) };
    runtime.ensure_shm();
    
    unsafe {
        if let Some((_ptr, atts)) = read_color_attachments(runtime) {
            return atts.len() as u32;
        }
    }
    0
}

/// Get a pointer to plane data by index, along with its stride/width/height.
/// index 0 = Y plane (or RGBA), index 1 = UV plane, etc.
/// Returns null if index is out of range or no frame available.
#[no_mangle]
pub extern "C" fn nativis_get_plane(
    ctx: *mut c_void,
    index: u32,
    out_stride: *mut u32,
    out_width: *mut u32,
    out_height: *mut u32,
) -> *mut u8 {
    if ctx.is_null() { return ptr::null_mut(); }
    let runtime = unsafe { &mut *(ctx as *mut NativisConsumer) };
    runtime.ensure_shm();
    
    unsafe {
        if let Some((base_ptr, atts)) = read_color_attachments(runtime) {
            // Find the attachment with matching surface_index
            for att in &atts {
                if att.surface_index == index {
                    if !out_stride.is_null() { *out_stride = att.stride; }
                    if !out_width.is_null()  { *out_width  = att.width;  }
                    if !out_height.is_null() { *out_height = att.height; }
                    return base_ptr.add(att.data_offset as usize);
                }
            }
        }
    }
    ptr::null_mut()
}
