#![allow(clippy::not_unsafe_ptr_arg_deref)]

use nativis_protocol::{NativisAttachment, NativisFrameHeader, NATIVIS_ATTACHMENT_USAGE_COLOR};
use nativis_transport_shm::{ShmSurface, SurfaceOps};
use std::os::raw::{c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[no_mangle]
pub extern "C" fn nativis_version() -> u32 {
    3
}

pub struct NativisConsumer {
    shm_name: String,
    shm: Mutex<Option<Arc<ShmSurface>>>,
    last_shm_attempt: AtomicU64,
    last_frame_id: AtomicU64,
    fallback_buffer: Mutex<Vec<u8>>,
    width: AtomicUsize,
    height: AtomicUsize,
    stopped: AtomicBool,
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

fn convert_nv12_to_rgba(
    width: usize,
    height: usize,
    y_plane: &[u8],
    y_stride: usize,
    uv_plane: &[u8],
    uv_stride: usize,
    dst: &mut [u8],
) {
    for y in 0..height {
        let y_row_start = y * y_stride;
        let uv_row_start = (y / 2) * uv_stride;
        let dst_row_start = y * width * 4;

        if y_row_start + width > y_plane.len() || dst_row_start + width * 4 > dst.len() {
            break;
        }

        let y_row = &y_plane[y_row_start..y_row_start + width];
        let dst_row = &mut dst[dst_row_start..dst_row_start + width * 4];

        for (x, &y_byte) in y_row.iter().enumerate().take(width) {
            let y_val = y_byte as i32;
            let uv_idx = uv_row_start + (x / 2) * 2;
            if uv_idx + 1 >= uv_plane.len() {
                break;
            }
            let u_val = uv_plane[uv_idx] as i32;
            let v_val = uv_plane[uv_idx + 1] as i32;

            let c = y_val - 16;
            let d = u_val - 128;
            let e = v_val - 128;

            let r = ((298 * c + 409 * e + 128) >> 8).clamp(0, 255) as u8;
            let g = ((298 * c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
            let b = ((298 * c + 516 * d + 128) >> 8).clamp(0, 255) as u8;

            let out_idx = x * 4;
            dst_row[out_idx] = r;
            dst_row[out_idx + 1] = g;
            dst_row[out_idx + 2] = b;
            dst_row[out_idx + 3] = 255;
        }
    }
}

impl NativisConsumer {
    fn new() -> Self {
        Self::with_shm_name("/nativis_shm")
    }

    pub fn with_shm_name(name: &str) -> Self {
        let consumer = Self {
            shm_name: name.to_string(),
            shm: Mutex::new(ShmSurface::new(name, 0, false).ok().map(Arc::new)),
            last_shm_attempt: AtomicU64::new(0),
            last_frame_id: AtomicU64::new(0),
            fallback_buffer: Mutex::new(Vec::new()),
            width: AtomicUsize::new(0),
            height: AtomicUsize::new(0),
            stopped: AtomicBool::new(false),
        };
        consumer.ensure_shm();
        consumer
    }

    fn ensure_shm(&self) {
        let _ = self.get_shm();
    }

    fn get_shm(&self) -> Option<Arc<ShmSurface>> {
        let mut guard = self.shm.lock().ok()?;
        if let Some(surface) = guard.as_ref() {
            if let Ok(handle) = surface.acquire() {
                if handle.size >= std::mem::size_of::<NativisFrameHeader>() {
                    let header = unsafe { &*(handle.ptr as *const NativisFrameHeader) };
                    if header.magic == nativis_protocol::NATIVIS_MAGIC {
                        return Some(surface.clone());
                    }
                }
            }
        }

        let now = monotonic_ms();
        let last = self.last_shm_attempt.load(Ordering::Relaxed);
        if now.saturating_sub(last) >= 100 {
            self.last_shm_attempt.store(now, Ordering::Relaxed);
            *guard = ShmSurface::new(&self.shm_name, 0, false).ok().map(Arc::new);
        }
        guard.as_ref().cloned()
    }
}

#[no_mangle]
pub extern "C" fn nativis_create() -> *mut c_void {
    let runtime = Box::new(NativisConsumer::new());
    Box::into_raw(runtime) as *mut c_void
}

#[no_mangle]
pub extern "C" fn nativis_create_with_shm(name: *const std::os::raw::c_char) -> *mut c_void {
    let shm_name = if name.is_null() {
        "/nativis_shm"
    } else {
        unsafe {
            std::ffi::CStr::from_ptr(name)
                .to_str()
                .unwrap_or("/nativis_shm")
        }
    };
    let runtime = Box::new(NativisConsumer::with_shm_name(shm_name));
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
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };
    runtime.stopped.store(false, Ordering::SeqCst);
    runtime.width.store(width as usize, Ordering::Relaxed);
    runtime.height.store(height as usize, Ordering::Relaxed);
    runtime.ensure_shm();
    true
}

#[no_mangle]
pub extern "C" fn nativis_get_pixels(ctx: *mut c_void) -> *mut u8 {
    if ctx.is_null() {
        return ptr::null_mut();
    }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };

    if let Some(shm) = runtime.get_shm() {
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

                        let mut candidate_ptr = ptr::null_mut();
                        let mut is_nv12 = false;
                        let mut nv12_y_offset = 0;
                        let mut nv12_uv_offset = 0;
                        let mut nv12_y_stride = 0;
                        let mut nv12_uv_stride = 0;
                        let mut frame_w = 0;
                        let mut frame_h = 0;

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
                                    if att.format == nativis_protocol::NATIVIS_FORMAT_NV12 {
                                        is_nv12 = true;
                                        if att.surface_index == 0 {
                                            nv12_y_offset = att.data_offset as usize;
                                            nv12_y_stride = att.stride as usize;
                                            frame_w = att.width as usize;
                                            frame_h = att.height as usize;
                                        } else if att.surface_index == 1 {
                                            nv12_uv_offset = att.data_offset as usize;
                                            nv12_uv_stride = att.stride as usize;
                                        }
                                    } else {
                                        candidate_ptr =
                                            unsafe { ptr.add(att.data_offset as usize) };
                                        break;
                                    }
                                }
                            }
                        }

                        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
                        let seq2 = unsafe { std::ptr::read_volatile(&header.sequence) };
                        if seq1 == seq2 {
                            if !candidate_ptr.is_null() {
                                runtime
                                    .last_frame_id
                                    .store(header.frame_id, Ordering::Relaxed);
                                return candidate_ptr;
                            } else if is_nv12
                                && nv12_y_offset != 0
                                && nv12_uv_offset != 0
                                && frame_w > 0
                                && frame_h > 0
                            {
                                // Convert NV12 to RGBA in fallback_buffer so callers expecting RGBA pixels get valid data
                                let expected_size = frame_w * frame_h * 4;
                                let mut fallback = runtime.fallback_buffer.lock().unwrap();
                                if fallback.len() != expected_size {
                                    fallback.resize(expected_size, 0);
                                }
                                let y_plane = unsafe {
                                    std::slice::from_raw_parts(
                                        ptr.add(nv12_y_offset),
                                        handle.size.saturating_sub(nv12_y_offset),
                                    )
                                };
                                let uv_plane = unsafe {
                                    std::slice::from_raw_parts(
                                        ptr.add(nv12_uv_offset),
                                        handle.size.saturating_sub(nv12_uv_offset),
                                    )
                                };
                                convert_nv12_to_rgba(
                                    frame_w,
                                    frame_h,
                                    y_plane,
                                    nv12_y_stride,
                                    uv_plane,
                                    nv12_uv_stride,
                                    &mut fallback,
                                );
                                runtime.width.store(frame_w, Ordering::Relaxed);
                                runtime.height.store(frame_h, Ordering::Relaxed);
                                runtime
                                    .last_frame_id
                                    .store(header.frame_id, Ordering::Relaxed);
                                return fallback.as_mut_ptr();
                            }
                        }
                    }
                }
            }
        }
    }

    let w = runtime.width.load(Ordering::Relaxed).max(1);
    let h = runtime.height.load(Ordering::Relaxed).max(1);
    let expected_size = w * h * 4;
    let mut fallback = runtime.fallback_buffer.lock().unwrap();
    if fallback.len() != expected_size {
        fallback.resize(expected_size, 0);
    }
    fallback.as_mut_ptr()
}

#[no_mangle]
pub extern "C" fn nativis_render(_ctx: *mut c_void) {}

#[no_mangle]
pub extern "C" fn nativis_get_width(ctx: *mut c_void) -> c_int {
    if ctx.is_null() {
        return 0;
    }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };

    if let Some((_ptr, atts)) = unsafe { read_color_attachments(runtime) } {
        for att in atts {
            if att.usage == NATIVIS_ATTACHMENT_USAGE_COLOR && att.surface_index == 0 {
                return att.width as c_int;
            }
        }
    }
    runtime.width.load(Ordering::Relaxed) as c_int
}

#[no_mangle]
pub extern "C" fn nativis_get_height(ctx: *mut c_void) -> c_int {
    if ctx.is_null() {
        return 0;
    }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };

    if let Some((_ptr, atts)) = unsafe { read_color_attachments(runtime) } {
        for att in atts {
            if att.usage == NATIVIS_ATTACHMENT_USAGE_COLOR && att.surface_index == 0 {
                return att.height as c_int;
            }
        }
    }
    runtime.height.load(Ordering::Relaxed) as c_int
}

#[no_mangle]
pub extern "C" fn nativis_end_frame(_ctx: *mut c_void) {}

#[no_mangle]
pub extern "C" fn nativis_get_frame_id(ctx: *mut c_void) -> u64 {
    if ctx.is_null() {
        return 0;
    }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };

    if let Some(shm) = runtime.get_shm() {
        if let Ok(handle) = shm.acquire() {
            if handle.size >= std::mem::size_of::<NativisFrameHeader>() {
                let header = unsafe { &*(handle.ptr as *const NativisFrameHeader) };
                if header.magic == nativis_protocol::NATIVIS_MAGIC {
                    return header.frame_id;
                }
            }
        }
    }

    runtime.last_frame_id.load(Ordering::Relaxed)
}

/// Item 5: Blocks calling thread using Linux FUTEX_WAIT on the lower 32 bits of frame_id (offset 8).
/// Returns immediately if frame_id != last_seen_id.
/// On timeout or error, returns the current frame_id (or 0).
/// CRITICAL FIX (T4): futex wait is performed on Arc<ShmSurface> outside any mutex, preventing
/// render thread starvation and eliminating compositor ghosting/hitching.
#[no_mangle]
pub extern "C" fn nativis_wait_frame(ctx: *mut c_void, last_seen_id: u64, timeout_ms: u32) -> u64 {
    if ctx.is_null() {
        return 0;
    }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };
    if runtime.stopped.load(Ordering::SeqCst) {
        return 0;
    }

    let shm = match runtime.get_shm() {
        Some(s) => s,
        None => {
            std::thread::sleep(std::time::Duration::from_millis(timeout_ms.min(100) as u64));
            return 0;
        }
    };

    let handle = match shm.acquire() {
        Ok(h) => h,
        Err(_) => {
            std::thread::sleep(std::time::Duration::from_millis(timeout_ms.min(100) as u64));
            return 0;
        }
    };

    if handle.size < std::mem::size_of::<NativisFrameHeader>() {
        std::thread::sleep(std::time::Duration::from_millis(timeout_ms.min(100) as u64));
        return 0;
    }

    let header = unsafe { &*(handle.ptr as *const NativisFrameHeader) };
    if header.magic != nativis_protocol::NATIVIS_MAGIC {
        std::thread::sleep(std::time::Duration::from_millis(timeout_ms.min(100) as u64));
        return 0;
    }

    let current_id = header.frame_id;
    if current_id != last_seen_id && current_id != 0 {
        return current_id;
    }

    if runtime.stopped.load(Ordering::SeqCst) {
        return 0;
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

    0
}

/// Item 6: Wakes any thread currently blocked in `nativis_wait_frame` immediately.
#[no_mangle]
pub extern "C" fn nativis_wake(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };
    runtime.stopped.store(true, Ordering::SeqCst);
    if let Some(shm) = runtime.get_shm() {
        if let Ok(handle) = shm.acquire() {
            #[cfg(target_os = "linux")]
            {
                let futex_ptr = unsafe { handle.ptr.add(8) as *mut u32 };
                unsafe {
                    libc::syscall(
                        libc::SYS_futex,
                        futex_ptr,
                        libc::FUTEX_WAKE,
                        libc::INT_MAX,
                        0,
                        0,
                        0,
                    );
                }
            }
        }
    }
}

// ── Generic plane-based FFI ─────────────────────────────────────────

unsafe fn read_color_attachments(
    runtime: &NativisConsumer,
) -> Option<(*mut u8, Vec<NativisAttachment>)> {
    let shm = runtime.get_shm()?;
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
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };

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
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };

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
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };

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

#[cfg(test)]
mod tests {
    use super::*;
    use nativis_transport_shm::ShmSurface;

    #[test]
    fn test_seqlock_prevents_torn_frame_read() {
        let shm_name = "/nativis_shm_test_seq_v2";
        let size = 65536;
        let surface = ShmSurface::new(shm_name, size, true).expect("create test SHM");
        let handle = surface.acquire().expect("acquire handle");

        let header_size = std::mem::size_of::<NativisFrameHeader>();
        let att_size = std::mem::size_of::<NativisAttachment>();
        let data_offset = (header_size + att_size) as u32;

        let header = NativisFrameHeader {
            magic: nativis_protocol::NATIVIS_MAGIC,
            version: nativis_protocol::NATIVIS_VERSION,
            frame_id: 10,
            timestamp: 1000,
            sequence: 19,
            attachment_count: 1,
            attachment_offset: header_size as u32,
        };
        let attachment = NativisAttachment {
            usage: NATIVIS_ATTACHMENT_USAGE_COLOR,
            format: nativis_protocol::NATIVIS_FORMAT_RGBA8888,
            width: 16,
            height: 16,
            stride: 64,
            planes: 1,
            surface_index: 0,
            data_offset,
        };

        unsafe {
            std::ptr::copy_nonoverlapping(
                &header as *const _ as *const u8,
                handle.ptr,
                header_size,
            );
            std::ptr::copy_nonoverlapping(
                &attachment as *const _ as *const u8,
                handle.ptr.add(header_size),
                att_size,
            );
        }

        let shm_c_str = std::ffi::CString::new(shm_name).unwrap();
        let ctx = nativis_create_with_shm(shm_c_str.as_ptr());
        assert!(!ctx.is_null());

        let pixels = nativis_get_pixels(ctx);
        let runtime = unsafe { &*(ctx as *const NativisConsumer) };

        assert_ne!(pixels, unsafe { handle.ptr.add(data_offset as usize) });
        {
            let fallback = runtime.fallback_buffer.lock().unwrap();
            assert_eq!(pixels, fallback.as_ptr() as *mut u8);
        }

        unsafe {
            let data_slice =
                std::slice::from_raw_parts_mut(handle.ptr.add(data_offset as usize), 64);
            data_slice.fill(0x42);
        }

        let header_ptr = handle.ptr as *mut NativisFrameHeader;
        unsafe {
            (*header_ptr).sequence = 20;
        }

        let pixels_valid = nativis_get_pixels(ctx);
        {
            let fallback = runtime.fallback_buffer.lock().unwrap();
            assert_ne!(pixels_valid, fallback.as_ptr() as *mut u8);
        }
        let read_slice = unsafe { std::slice::from_raw_parts(pixels_valid, 64) };
        assert_eq!(read_slice, &[0x42; 64]);

        nativis_destroy(ctx);
    }

    #[test]
    fn test_e1_mutex_held_across_futex_wait_blocks_readers() {
        let shm_name = "/nativis_shm_test_e1_v2";
        let size = 65536;
        let surface = ShmSurface::new(shm_name, size, true).expect("create test SHM");
        let handle = surface.acquire().expect("acquire handle");

        let header = NativisFrameHeader {
            magic: nativis_protocol::NATIVIS_MAGIC,
            version: nativis_protocol::NATIVIS_VERSION,
            frame_id: 100,
            timestamp: 1000,
            sequence: 2,
            attachment_count: 0,
            attachment_offset: 0,
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                &header as *const _ as *const u8,
                handle.ptr,
                std::mem::size_of::<NativisFrameHeader>(),
            );
        }

        let shm_c_str = std::ffi::CString::new(shm_name).unwrap();
        let ctx = nativis_create_with_shm(shm_c_str.as_ptr());
        assert!(!ctx.is_null());

        assert_eq!(nativis_get_frame_id(ctx), 100);

        let ctx_usize = ctx as usize;
        let handle_a = std::thread::spawn(move || {
            let thread_ctx = ctx_usize as *mut c_void;
            nativis_wait_frame(thread_ctx, 100, 400)
        });

        std::thread::sleep(std::time::Duration::from_millis(30));

        let start = std::time::Instant::now();
        let frame_id = nativis_get_frame_id(ctx);
        let elapsed = start.elapsed();

        println!(
            "[E1 RESULT AFTER FIX V2] Reader nativis_get_frame_id took: {:?} (frame_id={})",
            elapsed, frame_id
        );

        assert!(
            elapsed < std::time::Duration::from_millis(5),
            "Reader was blocked for {:?}! Mutex is still held across futex wait!",
            elapsed
        );

        nativis_wake(ctx);
        handle_a.join().unwrap();
        nativis_destroy(ctx);
    }

    #[test]
    fn test_nv12_to_rgba_conversion_in_get_pixels() {
        let shm_name = "/nativis_shm_test_nv12_conv_v2";
        let size = 65536;
        let surface = ShmSurface::new(shm_name, size, true).expect("create test SHM");
        let handle = surface.acquire().expect("acquire handle");

        let header_size = std::mem::size_of::<NativisFrameHeader>();
        let att_size = std::mem::size_of::<NativisAttachment>();
        let att_count = 2u32;
        let data_start = header_size + (att_size * att_count as usize);

        let w = 4u32;
        let h = 4u32;
        let y_len = (w * h) as usize;
        let uv_len = (w * (h / 2)) as usize;

        let y_offset = data_start as u32;
        let uv_offset = y_offset + y_len as u32;

        let header = NativisFrameHeader {
            magic: nativis_protocol::NATIVIS_MAGIC,
            version: nativis_protocol::NATIVIS_VERSION,
            frame_id: 200,
            timestamp: 2000,
            sequence: 4,
            attachment_count: att_count,
            attachment_offset: header_size as u32,
        };

        let att_y = NativisAttachment {
            usage: nativis_protocol::NATIVIS_ATTACHMENT_USAGE_COLOR,
            format: nativis_protocol::NATIVIS_FORMAT_NV12,
            width: w,
            height: h,
            stride: w,
            planes: 2,
            surface_index: 0,
            data_offset: y_offset,
        };

        let att_uv = NativisAttachment {
            usage: nativis_protocol::NATIVIS_ATTACHMENT_USAGE_COLOR,
            format: nativis_protocol::NATIVIS_FORMAT_NV12,
            width: w / 2,
            height: h / 2,
            stride: w,
            planes: 2,
            surface_index: 1,
            data_offset: uv_offset,
        };

        unsafe {
            std::ptr::copy_nonoverlapping(
                &header as *const _ as *const u8,
                handle.ptr,
                header_size,
            );
            let att_ptr = handle.ptr.add(header_size) as *mut NativisAttachment;
            std::ptr::write(att_ptr, att_y);
            std::ptr::write(att_ptr.add(1), att_uv);

            let y_slice = std::slice::from_raw_parts_mut(handle.ptr.add(y_offset as usize), y_len);
            y_slice.fill(128);
            let uv_slice =
                std::slice::from_raw_parts_mut(handle.ptr.add(uv_offset as usize), uv_len);
            uv_slice.fill(128);
        }

        let shm_c_str = std::ffi::CString::new(shm_name).unwrap();
        let ctx = nativis_create_with_shm(shm_c_str.as_ptr());
        assert!(!ctx.is_null());

        let pixels = nativis_get_pixels(ctx);
        assert!(!pixels.is_null());

        let rgba_len = (w * h * 4) as usize;
        let rgba_slice = unsafe { std::slice::from_raw_parts(pixels, rgba_len) };

        for i in 0..(w * h) as usize {
            let px = &rgba_slice[i * 4..(i + 1) * 4];
            assert_eq!(px[0], 130, "R mismatch");
            assert_eq!(px[1], 130, "G mismatch");
            assert_eq!(px[2], 130, "B mismatch");
            assert_eq!(px[3], 255, "Alpha mismatch");
        }

        nativis_destroy(ctx);
    }
}
