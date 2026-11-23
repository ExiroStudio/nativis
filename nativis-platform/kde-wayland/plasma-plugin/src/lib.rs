#![allow(clippy::not_unsafe_ptr_arg_deref)]

use nativis_protocol::{NativisAttachment, NativisFrameHeader, NATIVIS_ATTACHMENT_USAGE_COLOR};
use nativis_transport_shm::{ShmSurface, SurfaceOps};
use std::os::raw::{c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[no_mangle]
pub extern "C" fn nativis_version() -> u32 {
    3
}

pub struct NativisConsumer {
    shm: Mutex<Option<ShmSurface>>,
    last_shm_attempt: AtomicU64,
    last_frame_id: u64,
    fallback_buffer: Vec<u8>,
    width: usize,
    height: usize,
    active_ptr: *mut u8,
}

fn monotonic_ms() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    (ts.tv_sec as u64) * 1000 + (ts.tv_nsec as u64) / 1_000_000
}

impl NativisConsumer {
    fn new() -> Self {
        Self {
            shm: Mutex::new(ShmSurface::new("/nativis_shm", 0, false).ok()),
            last_shm_attempt: AtomicU64::new(0),
            last_frame_id: 0,
            fallback_buffer: Vec::new(),
            width: 0,
            height: 0,
            active_ptr: ptr::null_mut(),
        }
    }

    fn ensure_shm(&self) {
        if let Ok(guard) = self.shm.lock() {
            if guard.as_ref().map(|s| s.is_valid()).unwrap_or(false) {
                return;
            }
        }

        let now = monotonic_ms();
        let last = self.last_shm_attempt.load(Ordering::Relaxed);
        if now.saturating_sub(last) < 100 {
            return;
        }
        self.last_shm_attempt.store(now, Ordering::Relaxed);

        if let Ok(mut guard) = self.shm.lock() {
            if guard.as_ref().map(|s| s.is_valid()).unwrap_or(false) {
                return;
            }
            *guard = ShmSurface::new("/nativis_shm", 0, false).ok();
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

    if let Ok(shm_guard) = runtime.shm.lock() {
        if let Some(shm) = &*shm_guard {
            if let Ok(handle) = shm.acquire() {
                let ptr = handle.ptr;
                if handle.size >= std::mem::size_of::<NativisFrameHeader>() {
                    let header = unsafe { &*(ptr as *const NativisFrameHeader) };
                    if header.magic == nativis_protocol::NATIVIS_MAGIC {
                        // Seqlock verification
                        for _ in 0..5 {
                            let seq1 = unsafe { std::ptr::read_volatile(&header.sequence) };
                            if seq1 & 1 != 0 {
                                core::hint::spin_loop();
                                continue;
                            }
                            std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);

                            runtime.last_frame_id = header.frame_id;
                            let offset = header.attachment_offset as usize;
                            if handle.size
                                >= offset
                                    + (header.attachment_count as usize
                                        * std::mem::size_of::<NativisAttachment>())
                            {
                                let attachments_ptr =
                                    unsafe { ptr.add(offset) as *const NativisAttachment };
                                let attachments = unsafe {
                                    std::slice::from_raw_parts(
                                        attachments_ptr,
                                        header.attachment_count as usize,
                                    )
                                };
                                for att in attachments {
                                    if att.usage == NATIVIS_ATTACHMENT_USAGE_COLOR {
                                        runtime.active_ptr =
                                            unsafe { ptr.add(att.data_offset as usize) };
                                        break;
                                    }
                                }
                            }

                            std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
                            let seq2 = unsafe { std::ptr::read_volatile(&header.sequence) };
                            if seq1 == seq2 {
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    if runtime.active_ptr.is_null() {
        let expected_size = runtime.width * runtime.height * 4;
        if runtime.fallback_buffer.len() != expected_size {
            runtime.fallback_buffer.resize(expected_size, 0);
        }
        return runtime.fallback_buffer.as_mut_ptr();
    }

    runtime.active_ptr
}

#[no_mangle]
pub extern "C" fn nativis_render(_ctx: *mut c_void) {}

#[no_mangle]
pub extern "C" fn nativis_get_width(ctx: *mut c_void) -> c_int {
    if ctx.is_null() {
        return 0;
    }
    let runtime = unsafe { &mut *(ctx as *mut NativisConsumer) };
    runtime.ensure_shm();

    if let Ok(shm_guard) = runtime.shm.lock() {
        if let Some(shm) = &*shm_guard {
            if let Ok(handle) = shm.acquire() {
                let ptr = handle.ptr;
                if handle.size >= std::mem::size_of::<NativisFrameHeader>() {
                    let header = unsafe { &*(ptr as *const NativisFrameHeader) };
                    if header.magic == nativis_protocol::NATIVIS_MAGIC {
                        let offset = header.attachment_offset as usize;
                        if handle.size
                            >= offset
                                + (header.attachment_count as usize
                                    * std::mem::size_of::<NativisAttachment>())
                        {
                            let attachments_ptr =
                                unsafe { ptr.add(offset) as *const NativisAttachment };
                            let attachments = unsafe {
                                std::slice::from_raw_parts(
                                    attachments_ptr,
                                    header.attachment_count as usize,
                                )
                            };
                            for att in attachments {
                                if att.usage == NATIVIS_ATTACHMENT_USAGE_COLOR
                                    && att.surface_index == 0
                                {
                                    return att.width as c_int;
                                }
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
    if ctx.is_null() {
        return 0;
    }
    let runtime = unsafe { &mut *(ctx as *mut NativisConsumer) };
    runtime.ensure_shm();

    if let Ok(shm_guard) = runtime.shm.lock() {
        if let Some(shm) = &*shm_guard {
            if let Ok(handle) = shm.acquire() {
                let ptr = handle.ptr;
                if handle.size >= std::mem::size_of::<NativisFrameHeader>() {
                    let header = unsafe { &*(ptr as *const NativisFrameHeader) };
                    if header.magic == nativis_protocol::NATIVIS_MAGIC {
                        let offset = header.attachment_offset as usize;
                        if handle.size
                            >= offset
                                + (header.attachment_count as usize
                                    * std::mem::size_of::<NativisAttachment>())
                        {
                            let attachments_ptr =
                                unsafe { ptr.add(offset) as *const NativisAttachment };
                            let attachments = unsafe {
                                std::slice::from_raw_parts(
                                    attachments_ptr,
                                    header.attachment_count as usize,
                                )
                            };
                            for att in attachments {
                                if att.usage == NATIVIS_ATTACHMENT_USAGE_COLOR
                                    && att.surface_index == 0
                                {
                                    return att.height as c_int;
                                }
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
pub extern "C" fn nativis_end_frame(_ctx: *mut c_void) {}

#[no_mangle]
pub extern "C" fn nativis_get_frame_id(ctx: *mut c_void) -> u64 {
    if ctx.is_null() {
        return 0;
    }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };
    runtime.ensure_shm();

    if let Ok(shm_guard) = runtime.shm.lock() {
        if let Some(shm) = &*shm_guard {
            if let Ok(handle) = shm.acquire() {
                if handle.size >= std::mem::size_of::<NativisFrameHeader>() {
                    let header = unsafe { &*(handle.ptr as *const NativisFrameHeader) };
                    if header.magic == nativis_protocol::NATIVIS_MAGIC {
                        return header.frame_id;
                    }
                }
            }
        }
    }
    runtime.last_frame_id
}

/// Item 5: Blocks calling thread using Linux FUTEX_WAIT on the lower 32 bits of frame_id (offset 8).
/// Returns immediately if frame_id != last_seen_id.
/// On timeout or error, returns the current frame_id (or 0).
/// Eliminates 240 Hz busy polling in plasmashell, resulting in true 0 wakeups/s when idle.
#[no_mangle]
pub extern "C" fn nativis_wait_frame(ctx: *mut c_void, last_seen_id: u64, timeout_ms: u32) -> u64 {
    if ctx.is_null() {
        return 0;
    }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };
    runtime.ensure_shm();

    if let Ok(shm_guard) = runtime.shm.lock() {
        if let Some(shm) = &*shm_guard {
            if let Ok(handle) = shm.acquire() {
                if handle.size >= std::mem::size_of::<NativisFrameHeader>() {
                    let header = unsafe { &*(handle.ptr as *const NativisFrameHeader) };
                    if header.magic == nativis_protocol::NATIVIS_MAGIC {
                        let current_id = header.frame_id;
                        if current_id != last_seen_id && current_id != 0 {
                            return current_id;
                        }

                        #[cfg(target_os = "linux")]
                        {
                            let futex_ptr = unsafe { handle.ptr.add(8) as *mut u32 };
                            let val = (last_seen_id & 0xFFFF_FFFF) as u32;
                            let ts = libc::timespec {
                                tv_sec: (timeout_ms / 1000) as libc::time_t,
                                tv_nsec: ((timeout_ms % 1000) * 1_000_000) as libc::c_long,
                            };
                            unsafe {
                                libc::syscall(
                                    libc::SYS_futex,
                                    futex_ptr,
                                    libc::FUTEX_WAIT,
                                    val,
                                    &ts,
                                    std::ptr::null::<c_void>(),
                                    0,
                                );
                            }
                            let new_header = unsafe { &*(handle.ptr as *const NativisFrameHeader) };
                            if new_header.magic == nativis_protocol::NATIVIS_MAGIC {
                                return new_header.frame_id;
                            }
                        }
                        #[cfg(not(target_os = "linux"))]
                        {
                            std::thread::sleep(std::time::Duration::from_millis(4));
                            return current_id;
                        }
                    }
                }
            }
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(timeout_ms.min(100) as u64));
    0
}

unsafe fn read_color_attachments(
    runtime: &NativisConsumer,
) -> Option<(*mut u8, Vec<NativisAttachment>)> {
    let shm_guard = runtime.shm.lock().ok()?;
    let shm = shm_guard.as_ref()?;
    let handle = shm.acquire().ok()?;
    let ptr = handle.ptr;
    if handle.size < std::mem::size_of::<NativisFrameHeader>() {
        return None;
    }
    let header = &*(ptr as *const NativisFrameHeader);
    if header.magic != nativis_protocol::NATIVIS_MAGIC {
        return None;
    }

    for _ in 0..5 {
        let seq1 = std::ptr::read_volatile(&header.sequence);
        if seq1 & 1 != 0 {
            core::hint::spin_loop();
            continue;
        }
        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);

        let offset = header.attachment_offset as usize;
        let count = header.attachment_count as usize;
        if handle.size < offset + count * std::mem::size_of::<NativisAttachment>() {
            return None;
        }
        let att_ptr = ptr.add(offset) as *const NativisAttachment;
        let all = std::slice::from_raw_parts(att_ptr, count);
        let color_atts: Vec<NativisAttachment> = all
            .iter()
            .filter(|a| a.usage == NATIVIS_ATTACHMENT_USAGE_COLOR)
            .copied()
            .collect();

        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
        let seq2 = std::ptr::read_volatile(&header.sequence);
        if seq1 == seq2 {
            return Some((ptr, color_atts));
        }
    }
    None
}

#[no_mangle]
pub extern "C" fn nativis_get_format(ctx: *mut c_void) -> u32 {
    if ctx.is_null() {
        return 0;
    }
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

#[no_mangle]
pub extern "C" fn nativis_get_plane_count(ctx: *mut c_void) -> u32 {
    if ctx.is_null() {
        return 0;
    }
    let runtime = unsafe { &mut *(ctx as *mut NativisConsumer) };
    runtime.ensure_shm();

    unsafe {
        if let Some((_ptr, atts)) = read_color_attachments(runtime) {
            return atts.len() as u32;
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn nativis_get_plane(
    ctx: *mut c_void,
    index: u32,
    out_stride: *mut u32,
    out_width: *mut u32,
    out_height: *mut u32,
) -> *mut u8 {
    if ctx.is_null() {
        return ptr::null_mut();
    }
    let runtime = unsafe { &mut *(ctx as *mut NativisConsumer) };
    runtime.ensure_shm();

    unsafe {
        if let Some((base_ptr, atts)) = read_color_attachments(runtime) {
            for att in &atts {
                if att.surface_index == index {
                    if !out_stride.is_null() {
                        *out_stride = att.stride;
                    }
                    if !out_width.is_null() {
                        *out_width = att.width;
                    }
                    if !out_height.is_null() {
                        *out_height = att.height;
                    }
                    return base_ptr.add(att.data_offset as usize);
                }
            }
        }
    }
    ptr::null_mut()
}
