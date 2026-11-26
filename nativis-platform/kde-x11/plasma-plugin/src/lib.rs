#![allow(clippy::not_unsafe_ptr_arg_deref)]

use nativis_protocol::{NativisAttachment, NativisFrameHeader, NATIVIS_ATTACHMENT_USAGE_COLOR};
use nativis_transport_shm::{ShmSurface, SurfaceOps};
use std::os::raw::{c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[no_mangle]
pub extern "C" fn nativis_version() -> u32 {
    nativis_protocol::NATIVIS_VERSION
}

struct SurfaceSession {
    surface: Arc<ShmSurface>,
    epoch: u64,
}

pub struct NativisConsumer {
    shm_name: String,
    shm: Mutex<Option<SurfaceSession>>,
    pinned_shm: Mutex<Option<Arc<ShmSurface>>>,
    in_frame: AtomicBool,
    // Timestamp (ms) of the last shm_open attempt when SHM was unavailable.
    // Throttles reconnect attempts to at most once per 100ms, preventing a
    // flood of syscalls from poll loops.
    last_shm_attempt: AtomicU64,
    last_validate_ms: AtomicU64,
    last_observed_frame_id: AtomicU64,
    last_frame_id: AtomicU64,
    // We keep a fallback buffer just in case the producer hasn't started yet
    fallback_buffer: Mutex<Arc<Vec<u8>>>,
    pinned_fallback: Mutex<Option<Arc<Vec<u8>>>>,
    width: AtomicUsize,
    height: AtomicUsize,
    stopped: AtomicBool,
    fstat_count: AtomicU64,
    version_warned: AtomicBool,
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

unsafe fn read_header_snapshot(ptr: *const u8) -> Option<NativisFrameHeader> {
    let header_ptr = ptr as *const NativisFrameHeader;
    let magic = std::ptr::read_volatile(&(*header_ptr).magic);
    if magic != nativis_protocol::NATIVIS_MAGIC {
        return None;
    }
    for _ in 0..5 {
        let seq1 = std::ptr::read_volatile(&(*header_ptr).sequence);
        if seq1 & 1 != 0 {
            core::hint::spin_loop();
            continue;
        }
        std::sync::atomic::fence(Ordering::Acquire);
        let copy = std::ptr::read_volatile(header_ptr);
        std::sync::atomic::fence(Ordering::Acquire);
        let seq2 = std::ptr::read_volatile(&(*header_ptr).sequence);
        if seq1 == seq2 && copy.magic == nativis_protocol::NATIVIS_MAGIC {
            return Some(copy);
        }
    }
    None
}

fn attachment_plane_range(att: &NativisAttachment, shm_size: usize) -> Option<(usize, usize)> {
    if att.width == 0 || att.height == 0 || att.stride == 0 {
        return None;
    }
    let min_row_bytes = match att.format {
        nativis_protocol::NATIVIS_FORMAT_RGBA8888 => (att.width as usize).checked_mul(4)?,
        nativis_protocol::NATIVIS_FORMAT_NV12 => {
            if att.surface_index == 0 {
                att.width as usize
            } else {
                (att.width as usize).checked_mul(2)?
            }
        }
        nativis_protocol::NATIVIS_FORMAT_P010 => {
            if att.surface_index == 0 {
                (att.width as usize).checked_mul(2)?
            } else {
                (att.width as usize).checked_mul(4)?
            }
        }
        _ => att.width as usize,
    };
    if (att.stride as usize) < min_row_bytes {
        return None;
    }
    let plane_bytes = (att.stride as usize).checked_mul(att.height as usize)?;
    let start = att.data_offset as usize;
    if start < std::mem::size_of::<NativisFrameHeader>() {
        return None;
    }
    let end = start.checked_add(plane_bytes)?;
    if end > shm_size {
        return None;
    }
    Some((start, plane_bytes))
}

fn with_fallback_buffer<F: FnOnce(&mut [u8])>(
    runtime: &NativisConsumer,
    expected_size: usize,
    fill: F,
) -> *mut u8 {
    let mut fb_guard = runtime.fallback_buffer.lock().unwrap();
    let mut pin_guard = runtime.pinned_fallback.lock().unwrap();
    if Arc::strong_count(&*fb_guard) == 1 {
        if let Some(buf) = Arc::get_mut(&mut *fb_guard) {
            if buf.len() != expected_size {
                buf.resize(expected_size, 0);
            }
            fill(buf);
            let ptr = buf.as_mut_ptr();
            *pin_guard = Some(Arc::clone(&*fb_guard));
            return ptr;
        }
    }
    let mut new_buf = vec![0u8; expected_size];
    fill(&mut new_buf);
    let arc = Arc::new(new_buf);
    let ptr = arc.as_ptr() as *mut u8;
    *fb_guard = Arc::clone(&arc);
    *pin_guard = Some(arc);
    ptr
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

    fn read_session_metadata(surface: &ShmSurface) -> (u64, u64) {
        if let Ok(handle) = surface.acquire() {
            if handle.size >= std::mem::size_of::<NativisFrameHeader>() {
                if let Some(hdr) = unsafe { read_header_snapshot(handle.ptr) } {
                    if hdr.version == nativis_protocol::NATIVIS_VERSION {
                        return (hdr.epoch, hdr.frame_id);
                    }
                }
            }
        }
        (0, 0)
    }

    pub fn with_shm_name(name: &str) -> Self {
        let now = monotonic_ms();
        let (session, obs_id) = match ShmSurface::new(name, 0, false) {
            Ok(s) => {
                let (epoch, frame_id) = Self::read_session_metadata(&s);
                (
                    Some(SurfaceSession {
                        surface: Arc::new(s),
                        epoch,
                    }),
                    frame_id,
                )
            }
            Err(_) => (None, 0),
        };
        Self {
            shm_name: name.to_string(),
            shm: Mutex::new(session),
            pinned_shm: Mutex::new(None),
            in_frame: AtomicBool::new(false),
            last_shm_attempt: AtomicU64::new(0),
            last_validate_ms: AtomicU64::new(now),
            last_observed_frame_id: AtomicU64::new(obs_id),
            last_frame_id: AtomicU64::new(0),
            fallback_buffer: Mutex::new(Arc::new(Vec::new())),
            pinned_fallback: Mutex::new(None),
            width: AtomicUsize::new(0),
            height: AtomicUsize::new(0),
            stopped: AtomicBool::new(false),
            fstat_count: AtomicU64::new(0),
            version_warned: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub fn fstat_calls(&self) -> u64 {
        self.fstat_count.load(Ordering::Relaxed)
    }

    fn active_shm(&self) -> Option<Arc<ShmSurface>> {
        if self.in_frame.load(Ordering::Acquire) {
            if let Ok(pinned) = self.pinned_shm.lock() {
                if let Some(surface) = pinned.as_ref() {
                    return Some(surface.clone());
                }
            }
        }
        let shm = self.get_shm()?;
        if let Ok(mut pinned) = self.pinned_shm.lock() {
            *pinned = Some(shm.clone());
        }
        Some(shm)
    }

    fn get_shm(&self) -> Option<Arc<ShmSurface>> {
        let mut guard = self.shm.lock().ok()?;
        let mut bypass_throttle = false;

        if let Some(session) = guard.as_mut() {
            if let Ok(handle) = session.surface.acquire() {
                if handle.size >= std::mem::size_of::<NativisFrameHeader>() {
                    let magic = unsafe {
                        std::ptr::read_volatile(&(*(handle.ptr as *const NativisFrameHeader)).magic)
                    };
                    if magic == nativis_protocol::NATIVIS_MAGIC {
                        if let Some(hdr) = unsafe { read_header_snapshot(handle.ptr) } {
                            if hdr.version != nativis_protocol::NATIVIS_VERSION {
                                if !self.version_warned.swap(true, Ordering::Relaxed) {
                                    eprintln!(
                                        "[nativis] SHM protocol version mismatch: expected {}, got {}",
                                        nativis_protocol::NATIVIS_VERSION,
                                        hdr.version
                                    );
                                }
                                let now = monotonic_ms();
                                let last_val = self.last_validate_ms.load(Ordering::Relaxed);
                                if now.saturating_sub(last_val) >= 250 {
                                    self.last_validate_ms.store(now, Ordering::Relaxed);
                                    self.fstat_count.fetch_add(1, Ordering::Relaxed);
                                    if !session.surface.is_valid() {
                                        *guard = None;
                                        bypass_throttle = true;
                                    } else {
                                        return None;
                                    }
                                } else {
                                    return None;
                                }
                            } else if hdr.shm_size != session.surface.size() as u64 {
                                *guard = None;
                                bypass_throttle = true;
                            } else if hdr.epoch != session.epoch {
                                if session.epoch == 0 {
                                    session.epoch = hdr.epoch;
                                    self.last_observed_frame_id
                                        .store(hdr.frame_id, Ordering::Relaxed);
                                    return Some(session.surface.clone());
                                }
                                self.fstat_count.fetch_add(1, Ordering::Relaxed);
                                self.last_validate_ms
                                    .store(monotonic_ms(), Ordering::Relaxed);
                                if session.surface.is_valid() {
                                    session.epoch = hdr.epoch;
                                    self.last_observed_frame_id
                                        .store(hdr.frame_id, Ordering::Relaxed);
                                    return Some(session.surface.clone());
                                }
                                *guard = None;
                                bypass_throttle = true;
                            } else {
                                let prev_id = self.last_observed_frame_id.load(Ordering::Relaxed);
                                if hdr.frame_id != 0 && hdr.frame_id != prev_id {
                                    self.last_observed_frame_id
                                        .store(hdr.frame_id, Ordering::Relaxed);
                                    self.last_validate_ms
                                        .store(monotonic_ms(), Ordering::Relaxed);
                                    return Some(session.surface.clone());
                                }
                                let now = monotonic_ms();
                                let last_val = self.last_validate_ms.load(Ordering::Relaxed);
                                if now.saturating_sub(last_val) < 250 {
                                    return Some(session.surface.clone());
                                }
                                self.last_validate_ms.store(now, Ordering::Relaxed);
                                self.fstat_count.fetch_add(1, Ordering::Relaxed);
                                if session.surface.is_valid() {
                                    return Some(session.surface.clone());
                                }
                                *guard = None;
                                bypass_throttle = true;
                            }
                        } else {
                            return Some(session.surface.clone());
                        }
                    } else {
                        let now = monotonic_ms();
                        let last_val = self.last_validate_ms.load(Ordering::Relaxed);
                        if now.saturating_sub(last_val) < 250 {
                            return Some(session.surface.clone());
                        }
                        self.last_validate_ms.store(now, Ordering::Relaxed);
                        self.fstat_count.fetch_add(1, Ordering::Relaxed);
                        if session.surface.is_valid() {
                            return Some(session.surface.clone());
                        }
                        *guard = None;
                        bypass_throttle = true;
                    }
                } else {
                    *guard = None;
                    bypass_throttle = true;
                }
            } else {
                *guard = None;
                bypass_throttle = true;
            }
        }

        // Slow path: SHM is None or invalid — throttle reconnect to
        // at most once every 100ms to avoid flooding shm_open syscalls.
        let now = monotonic_ms();
        let last = self.last_shm_attempt.load(Ordering::Relaxed);
        if bypass_throttle || now.saturating_sub(last) >= 100 {
            self.last_shm_attempt.store(now, Ordering::Relaxed);
            if let Ok(surface) = ShmSurface::new(&self.shm_name, 0, false) {
                let (epoch, frame_id) = Self::read_session_metadata(&surface);
                self.last_validate_ms.store(now, Ordering::Relaxed);
                self.last_observed_frame_id
                    .store(frame_id, Ordering::Relaxed);
                self.version_warned.store(false, Ordering::Relaxed);
                *guard = Some(SurfaceSession {
                    surface: Arc::new(surface),
                    epoch,
                });
            } else {
                *guard = None;
            }
        }
        guard.as_ref().map(|s| s.surface.clone())
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
    if let Ok(mut pin) = runtime.pinned_fallback.lock() {
        *pin = None;
    }
    let shm = runtime.get_shm();
    if let Ok(mut pin) = runtime.pinned_shm.lock() {
        *pin = shm;
    }
    runtime.in_frame.store(true, Ordering::Release);

    true
}

#[no_mangle]
pub extern "C" fn nativis_get_pixels(ctx: *mut c_void) -> *mut u8 {
    if ctx.is_null() {
        return ptr::null_mut();
    }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };

    if let Some(shm) = runtime.active_shm() {
        if let Ok(handle) = shm.acquire() {
            let ptr = handle.ptr;
            if handle.size >= std::mem::size_of::<NativisFrameHeader>() {
                let header = unsafe { &*(ptr as *const NativisFrameHeader) };
                if header.magic == nativis_protocol::NATIVIS_MAGIC
                    && header.version == nativis_protocol::NATIVIS_VERSION
                {
                    // Seqlock verification
                    for _ in 0..5 {
                        let seq1 = unsafe { std::ptr::read_volatile(&header.sequence) };
                        if seq1 & 1 != 0 {
                            core::hint::spin_loop();
                            continue;
                        }
                        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);

                        if header.shm_size > handle.size as u64 {
                            break;
                        }

                        let mut candidate_ptr = ptr::null_mut();
                        let mut is_nv12 = false;
                        let mut nv12_y_range: Option<(usize, usize)> = None;
                        let mut nv12_uv_range: Option<(usize, usize)> = None;
                        let mut nv12_y_stride = 0;
                        let mut nv12_uv_stride = 0;
                        let mut frame_w = 0;
                        let mut frame_h = 0;

                        let offset = header.attachment_offset as usize;
                        let count = header.attachment_count as usize;
                        let att_bytes = count.checked_mul(std::mem::size_of::<NativisAttachment>());
                        let att_end = att_bytes.and_then(|b| offset.checked_add(b));

                        if offset >= std::mem::size_of::<NativisFrameHeader>()
                            && att_end.is_some_and(|e| e <= handle.size)
                        {
                            let attachments_ptr =
                                unsafe { ptr.add(offset) as *const NativisAttachment };
                            let attachments =
                                unsafe { std::slice::from_raw_parts(attachments_ptr, count) };
                            for att in attachments {
                                if att.usage == NATIVIS_ATTACHMENT_USAGE_COLOR {
                                    if att.format == nativis_protocol::NATIVIS_FORMAT_NV12 {
                                        is_nv12 = true;
                                        if att.surface_index == 0 {
                                            nv12_y_range = attachment_plane_range(att, handle.size);
                                            nv12_y_stride = att.stride as usize;
                                            frame_w = att.width as usize;
                                            frame_h = att.height as usize;
                                        } else if att.surface_index == 1 {
                                            nv12_uv_range =
                                                attachment_plane_range(att, handle.size);
                                            nv12_uv_stride = att.stride as usize;
                                        }
                                    } else if let Some((start, _)) =
                                        attachment_plane_range(att, handle.size)
                                    {
                                        candidate_ptr = unsafe { ptr.add(start) };
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
                            } else if is_nv12 && frame_w > 0 && frame_h > 0 {
                                if let (Some((y_start, y_len)), Some((uv_start, uv_len))) =
                                    (nv12_y_range, nv12_uv_range)
                                {
                                    let expected_size = frame_w * frame_h * 4;
                                    let y_plane = unsafe {
                                        std::slice::from_raw_parts(ptr.add(y_start), y_len)
                                    };
                                    let uv_plane = unsafe {
                                        std::slice::from_raw_parts(ptr.add(uv_start), uv_len)
                                    };
                                    let ptr_out =
                                        with_fallback_buffer(runtime, expected_size, |dst| {
                                            convert_nv12_to_rgba(
                                                frame_w,
                                                frame_h,
                                                y_plane,
                                                nv12_y_stride,
                                                uv_plane,
                                                nv12_uv_stride,
                                                dst,
                                            );
                                        });
                                    runtime.width.store(frame_w, Ordering::Relaxed);
                                    runtime.height.store(frame_h, Ordering::Relaxed);
                                    runtime
                                        .last_frame_id
                                        .store(header.frame_id, Ordering::Relaxed);
                                    return ptr_out;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Provide opaque black fallback (alpha=255 prevents transparent desktop ghosting)
    let w = runtime.width.load(Ordering::Relaxed).max(1);
    let h = runtime.height.load(Ordering::Relaxed).max(1);
    let expected_size = w * h * 4;
    with_fallback_buffer(runtime, expected_size, |dst| {
        for px in dst.as_chunks_mut::<4>().0 {
            *px = [0, 0, 0, 255];
        }
    })
}

#[no_mangle]
pub extern "C" fn nativis_render(_ctx: *mut c_void) {
    // No-op in V1, pixel extraction is done in nativis_get_pixels
}

#[no_mangle]
pub extern "C" fn nativis_get_width(ctx: *mut c_void) -> c_int {
    if ctx.is_null() {
        return 0;
    }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };

    if let Some((_shm, _ptr, _size, atts)) = unsafe { read_color_attachments(runtime) } {
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

    if let Some((_shm, _ptr, _size, atts)) = unsafe { read_color_attachments(runtime) } {
        for att in atts {
            if att.usage == NATIVIS_ATTACHMENT_USAGE_COLOR && att.surface_index == 0 {
                return att.height as c_int;
            }
        }
    }
    runtime.height.load(Ordering::Relaxed) as c_int
}

#[no_mangle]
pub extern "C" fn nativis_end_frame(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };
    runtime.in_frame.store(false, Ordering::Release);
    if let Ok(mut pin) = runtime.pinned_shm.lock() {
        *pin = None;
    }
    if let Ok(mut pin) = runtime.pinned_fallback.lock() {
        *pin = None;
    }
}

#[no_mangle]
pub extern "C" fn nativis_get_frame_id(ctx: *mut c_void) -> u64 {
    if ctx.is_null() {
        return 0;
    }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };

    // Read live frame_id directly from SHM snapshot without holding any global lock.
    if let Some(shm) = runtime.active_shm() {
        if let Ok(handle) = shm.acquire() {
            if handle.size >= std::mem::size_of::<NativisFrameHeader>() {
                let header = unsafe { &*(handle.ptr as *const NativisFrameHeader) };
                if header.magic == nativis_protocol::NATIVIS_MAGIC
                    && header.version == nativis_protocol::NATIVIS_VERSION
                {
                    return header.frame_id;
                }
            }
        }
    }

    // SHM not available yet — fall back to cached value
    runtime.last_frame_id.load(Ordering::Relaxed)
}

/// Item 5: Blocks calling thread using Linux FUTEX_WAIT on the lower 32 bits of frame_id (offset 8).
/// Returns immediately if frame_id != last_seen_id.
/// On timeout or error, returns the current frame_id (or 0).
/// CRITICAL FIX (T4): The futex wait is performed on a cloned Arc<ShmSurface> outside any mutex,
/// so the render thread is NEVER blocked while the watcher thread is waiting for the next frame!
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
    if header.magic != nativis_protocol::NATIVIS_MAGIC
        || header.version != nativis_protocol::NATIVIS_VERSION
    {
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
        let futex_ptr = unsafe {
            handle
                .ptr
                .add(std::mem::offset_of!(NativisFrameHeader, frame_id)) as *mut u32
        };
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
        if new_header.magic == nativis_protocol::NATIVIS_MAGIC
            && new_header.version == nativis_protocol::NATIVIS_VERSION
        {
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
                let futex_ptr = unsafe {
                    handle
                        .ptr
                        .add(std::mem::offset_of!(NativisFrameHeader, frame_id))
                        as *mut u32
                };
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

/// Clears the `stopped` flag before starting a new FrameWatcher thread on re-attach.
#[no_mangle]
pub extern "C" fn nativis_resume_wait(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };
    runtime.stopped.store(false, Ordering::SeqCst);
}

/// Returns the current producer epoch from the SHM header (or 0 if unavailable).
#[no_mangle]
pub extern "C" fn nativis_get_epoch(ctx: *mut c_void) -> u64 {
    if ctx.is_null() {
        return 0;
    }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };
    if let Some(shm) = runtime.active_shm() {
        if let Ok(handle) = shm.acquire() {
            if handle.size >= std::mem::size_of::<NativisFrameHeader>() {
                let header = unsafe { &*(handle.ptr as *const NativisFrameHeader) };
                if header.magic == nativis_protocol::NATIVIS_MAGIC
                    && header.version == nativis_protocol::NATIVIS_VERSION
                {
                    return header.epoch;
                }
            }
        }
    }
    0
}

// ── Fase 5: Generic plane-based FFI ─────────────────────────────────────────

/// Helper: read color attachments from SHM.
/// Returns None if SHM is not ready or magic/version/bounds mismatch.
unsafe fn read_color_attachments(
    runtime: &NativisConsumer,
) -> Option<(Arc<ShmSurface>, *mut u8, usize, Vec<NativisAttachment>)> {
    let shm = runtime.active_shm()?;
    let handle = shm.acquire().ok()?;
    let ptr = handle.ptr;
    if handle.size < std::mem::size_of::<NativisFrameHeader>() {
        return None;
    }
    let header = &*(ptr as *const NativisFrameHeader);
    if header.magic != nativis_protocol::NATIVIS_MAGIC
        || header.version != nativis_protocol::NATIVIS_VERSION
    {
        return None;
    }

    for _ in 0..5 {
        let seq1 = std::ptr::read_volatile(&header.sequence);
        if seq1 & 1 != 0 {
            core::hint::spin_loop();
            continue;
        }
        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);

        if header.shm_size > handle.size as u64 {
            return None;
        }

        let offset = header.attachment_offset as usize;
        let count = header.attachment_count as usize;
        let att_bytes = count.checked_mul(std::mem::size_of::<NativisAttachment>())?;
        let att_end = offset.checked_add(att_bytes)?;
        if offset < std::mem::size_of::<NativisFrameHeader>() || att_end > handle.size {
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
            return Some((shm, ptr, handle.size, color_atts));
        }
    }
    None
}

/// Returns the pixel format of the current frame (NATIVIS_FORMAT_* constant).
/// Returns 0 if no frame is available.
#[no_mangle]
pub extern "C" fn nativis_get_format(ctx: *mut c_void) -> u32 {
    if ctx.is_null() {
        return 0;
    }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };

    unsafe {
        if let Some((_shm, _ptr, _size, atts)) = read_color_attachments(runtime) {
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
    if ctx.is_null() {
        return 0;
    }
    let runtime = unsafe { &*(ctx as *const NativisConsumer) };

    unsafe {
        if let Some((_shm, _ptr, _size, atts)) = read_color_attachments(runtime) {
            return atts.len() as u32;
        }
    }
    0
}

/// Get a pointer to plane data by index, along with its stride/width/height.
/// index 0 = Y plane (or RGBA), index 1 = UV plane, etc.
/// Returns null if index is out of range, out of bounds, or no frame available.
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
        if let Some((_shm, base_ptr, shm_size, atts)) = read_color_attachments(runtime) {
            // Find the attachment with matching surface_index
            for att in &atts {
                if att.surface_index == index {
                    if let Some((start, _)) = attachment_plane_range(att, shm_size) {
                        if !out_stride.is_null() {
                            *out_stride = att.stride;
                        }
                        if !out_width.is_null() {
                            *out_width = att.width;
                        }
                        if !out_height.is_null() {
                            *out_height = att.height;
                        }
                        return base_ptr.add(start);
                    }
                    return ptr::null_mut();
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
        let shm_name = "/nativis_shm_test_seq";
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
            sequence: 19, // Odd sequence: write in progress!
            attachment_count: 1,
            attachment_offset: header_size as u32,
            shm_size: size as u64,
            epoch: 1,
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

        // Because sequence was odd (write in progress), active_ptr must NOT point to SHM data_offset
        assert_ne!(pixels, unsafe { handle.ptr.add(data_offset as usize) });
        // Instead, pixels must point to the safe fallback buffer
        {
            let fallback = runtime.fallback_buffer.lock().unwrap();
            assert_eq!(pixels, fallback.as_ptr() as *mut u8);
        }

        // Fill test pixel data in SHM
        unsafe {
            let data_slice =
                std::slice::from_raw_parts_mut(handle.ptr.add(data_offset as usize), 64);
            data_slice.fill(0x42);
        }

        // Now finalize the write: update sequence to even (sequence = 20)
        let header_ptr = handle.ptr as *mut NativisFrameHeader;
        unsafe {
            (*header_ptr).sequence = 20;
        }

        let pixels_valid = nativis_get_pixels(ctx);
        // Now it must point to the verified SHM data, NOT fallback buffer!
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
        let shm_name = "/nativis_shm_test_e1";
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
            shm_size: size as u64,
            epoch: 1,
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

        // Verify initial frame_id read
        assert_eq!(nativis_get_frame_id(ctx), 100);

        // Thread A calls nativis_wait_frame for 400ms with last_seen_id = 100
        let ctx_usize = ctx as usize;
        let handle_a = std::thread::spawn(move || {
            let thread_ctx = ctx_usize as *mut c_void;
            nativis_wait_frame(thread_ctx, 100, 400)
        });

        // Give Thread A 30ms to enter nativis_wait_frame and begin FUTEX_WAIT
        std::thread::sleep(std::time::Duration::from_millis(30));

        // Thread B (render thread simulating updatePaintNode) calls nativis_get_frame_id
        let start = std::time::Instant::now();
        let frame_id = nativis_get_frame_id(ctx);
        let elapsed = start.elapsed();

        println!(
            "[E1 RESULT AFTER FIX] Reader nativis_get_frame_id took: {:?} (frame_id={})",
            elapsed, frame_id
        );

        // Assert that reader was NOT blocked by Thread A's 400ms futex sleep.
        // It must finish in well under 5ms (typically < 0.1ms).
        assert!(
            elapsed < std::time::Duration::from_millis(5),
            "Reader was blocked for {:?}! Mutex is still held across futex wait!",
            elapsed
        );

        // Wake thread A immediately using nativis_wake
        nativis_wake(ctx);

        handle_a.join().unwrap();
        nativis_destroy(ctx);
    }

    #[test]
    fn test_nv12_to_rgba_conversion_in_get_pixels() {
        let shm_name = "/nativis_shm_test_nv12_conv";
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
            shm_size: size as u64,
            epoch: 1,
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
            // Write header
            std::ptr::copy_nonoverlapping(
                &header as *const _ as *const u8,
                handle.ptr,
                header_size,
            );
            // Write attachments
            let att_ptr = handle.ptr.add(header_size) as *mut NativisAttachment;
            std::ptr::write(att_ptr, att_y);
            std::ptr::write(att_ptr.add(1), att_uv);

            // Fill Y with 128, UV with 128 (neutral gray in YUV)
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

    #[test]
    fn test_r1_oob_attachment_rejected() {
        let shm_name = format!("/nativis_test_oob_v1_{}", std::process::id());
        let size = 4096;
        let surface = ShmSurface::new(&shm_name, size, true).expect("create test SHM");
        let handle = surface.acquire().expect("acquire handle");

        let header_size = std::mem::size_of::<NativisFrameHeader>();
        let header = NativisFrameHeader {
            magic: nativis_protocol::NATIVIS_MAGIC,
            version: nativis_protocol::NATIVIS_VERSION,
            frame_id: 10,
            timestamp: 1000,
            sequence: 2,
            attachment_count: 1,
            attachment_offset: header_size as u32,
            shm_size: size as u64,
            epoch: 1,
        };
        let att_oob = NativisAttachment {
            usage: nativis_protocol::NATIVIS_ATTACHMENT_USAGE_COLOR,
            format: nativis_protocol::NATIVIS_FORMAT_NV12,
            width: 1920,
            height: 1080,
            stride: 1920,
            planes: 1,
            surface_index: 0,
            data_offset: 65536,
        };

        unsafe {
            std::ptr::copy_nonoverlapping(
                &header as *const _ as *const u8,
                handle.ptr,
                header_size,
            );
            std::ptr::write(
                handle.ptr.add(header_size) as *mut NativisAttachment,
                att_oob,
            );
        }

        let shm_c_str = std::ffi::CString::new(shm_name).unwrap();
        let ctx = nativis_create_with_shm(shm_c_str.as_ptr());
        assert!(!ctx.is_null());

        let mut stride = 0u32;
        let mut w = 0u32;
        let mut h = 0u32;
        let plane = nativis_get_plane(ctx, 0, &mut stride, &mut w, &mut h);
        assert!(
            plane.is_null(),
            "nativis_get_plane must reject out-of-bounds attachment"
        );

        let runtime = unsafe { &*(ctx as *const NativisConsumer) };
        let px = nativis_get_pixels(ctx);
        let fb = runtime.fallback_buffer.lock().unwrap();
        assert_eq!(px, fb.as_ptr() as *mut u8);

        nativis_destroy(ctx);
    }

    #[test]
    fn test_r1_resize_and_daemon_restart_reconnect() {
        use nativis_core::contract::{Frame, FrameSink};
        use nativis_core::resource::{CpuBuffer, ResourceManager};
        use nativis_transport_shm::ShmSink;

        let shm_name = format!("/nativis_test_r1_recon_v1_{}", std::process::id());
        let resources = ResourceManager::new();
        let mut sink1 = ShmSink::new(&shm_name, 4096, resources.clone()).expect("sink1");

        let h1 = resources.register(Box::new(CpuBuffer {
            width: 8,
            height: 8,
            data: vec![0x11; 8 * 8 * 4],
        }));
        sink1
            .submit(Frame::new(
                h1,
                8,
                8,
                std::time::Duration::from_millis(16),
                true,
            ))
            .expect("submit small");
        resources.free(h1);

        let shm_c_str = std::ffi::CString::new(shm_name.clone()).unwrap();
        let ctx = nativis_create_with_shm(shm_c_str.as_ptr());
        assert!(!ctx.is_null());
        assert!(nativis_begin_frame(ctx, 8, 8));
        assert_eq!(nativis_get_width(ctx), 8);
        nativis_end_frame(ctx);

        let h2 = resources.register(Box::new(CpuBuffer {
            width: 128,
            height: 128,
            data: vec![0x33; 128 * 128 * 4],
        }));
        sink1
            .submit(Frame::new(
                h2,
                128,
                128,
                std::time::Duration::from_millis(32),
                true,
            ))
            .expect("submit large");
        resources.free(h2);

        let start = std::time::Instant::now();
        assert!(nativis_begin_frame(ctx, 128, 128));
        {
            let runtime = unsafe { &*(ctx as *const NativisConsumer) };
            let guard = runtime.shm.lock().unwrap();
            let mapped_size = guard.as_ref().map(|s| s.surface.size()).unwrap_or(0);
            assert!(mapped_size >= 128 * 128 * 4);
        }
        assert_eq!(nativis_get_width(ctx), 128);
        let px = nativis_get_pixels(ctx);
        let slice = unsafe { std::slice::from_raw_parts(px, 128 * 128 * 4) };
        assert_eq!(slice[0], 0x33);
        assert_eq!(slice[128 * 128 * 4 - 1], 0x33);
        nativis_end_frame(ctx);
        assert!(start.elapsed() < std::time::Duration::from_millis(50));

        drop(sink1);
        let mut sink2 = ShmSink::new(&shm_name, 4096, resources.clone()).expect("sink2");
        let h3 = resources.register(Box::new(CpuBuffer {
            width: 16,
            height: 16,
            data: vec![0x77; 16 * 16 * 4],
        }));
        sink2
            .submit(Frame::new(
                h3,
                16,
                16,
                std::time::Duration::from_millis(16),
                true,
            ))
            .expect("submit restart");
        resources.free(h3);

        std::thread::sleep(std::time::Duration::from_millis(270));
        assert!(nativis_begin_frame(ctx, 16, 16));
        assert_eq!(nativis_get_width(ctx), 16);
        let px2 = nativis_get_pixels(ctx);
        let slice2 = unsafe { std::slice::from_raw_parts(px2, 16 * 16 * 4) };
        assert_eq!(slice2[0], 0x77);
        nativis_end_frame(ctx);

        nativis_destroy(ctx);
    }

    #[test]
    fn test_r1_same_size_frames_do_not_trigger_per_call_fstat() {
        use nativis_core::contract::{Frame, FrameSink};
        use nativis_core::resource::{CpuBuffer, ResourceManager};
        use nativis_transport_shm::ShmSink;

        let shm_name = format!("/nativis_test_r1_fstat_v1_{}", std::process::id());
        let resources = ResourceManager::new();
        let mut sink = ShmSink::new(&shm_name, 65536, resources.clone()).expect("sink");

        let h1 = resources.register(Box::new(CpuBuffer {
            width: 16,
            height: 16,
            data: vec![0x55; 16 * 16 * 4],
        }));
        sink.submit(Frame::new(
            h1,
            16,
            16,
            std::time::Duration::from_millis(16),
            true,
        ))
        .expect("initial frame");
        resources.free(h1);

        let shm_c_str = std::ffi::CString::new(shm_name).unwrap();
        let ctx = nativis_create_with_shm(shm_c_str.as_ptr());
        assert!(!ctx.is_null());
        let runtime = unsafe { &*(ctx as *const NativisConsumer) };
        let baseline_fstats = runtime.fstat_calls();

        for i in 0..200u64 {
            let h = resources.register(Box::new(CpuBuffer {
                width: 16,
                height: 16,
                data: vec![(i & 0xFF) as u8; 16 * 16 * 4],
            }));
            sink.submit(Frame::new(
                h,
                16,
                16,
                std::time::Duration::from_millis(16),
                true,
            ))
            .expect("submit");
            resources.free(h);

            assert!(nativis_begin_frame(ctx, 16, 16));
            let _ = nativis_get_frame_id(ctx);
            let _ = nativis_get_format(ctx);
            let _ = nativis_get_plane_count(ctx);
            let mut s = 0;
            let mut w = 0;
            let mut h_out = 0;
            let plane = nativis_get_plane(ctx, 0, &mut s, &mut w, &mut h_out);
            assert!(!plane.is_null());
            nativis_end_frame(ctx);
        }

        let delta = runtime.fstat_calls() - baseline_fstats;
        assert!(
            delta <= 2,
            "200 same-size frames (1200 FFI calls) triggered {} fstat syscalls, expected <= 2",
            delta
        );

        nativis_destroy(ctx);
    }

    #[test]
    fn test_r1_pinned_shm_survives_concurrent_resize_until_end_frame() {
        use nativis_core::contract::{Frame, FrameSink};
        use nativis_core::resource::{CpuBuffer, ResourceManager};
        use nativis_transport_shm::ShmSink;

        let shm_name = format!("/nativis_test_r1_pin_v1_{}", std::process::id());
        let resources = ResourceManager::new();
        let mut sink = ShmSink::new(&shm_name, 4096, resources.clone()).expect("sink");

        let h1 = resources.register(Box::new(CpuBuffer {
            width: 8,
            height: 8,
            data: vec![0x22; 8 * 8 * 4],
        }));
        sink.submit(Frame::new(
            h1,
            8,
            8,
            std::time::Duration::from_millis(16),
            true,
        ))
        .expect("submit small");
        resources.free(h1);

        let shm_c_str = std::ffi::CString::new(shm_name).unwrap();
        let ctx = nativis_create_with_shm(shm_c_str.as_ptr());
        assert!(!ctx.is_null());
        let runtime = unsafe { &*(ctx as *const NativisConsumer) };

        assert!(nativis_begin_frame(ctx, 8, 8));
        let mut s = 0;
        let mut w = 0;
        let mut h = 0;
        let ptr1 = nativis_get_plane(ctx, 0, &mut s, &mut w, &mut h);
        assert!(!ptr1.is_null());

        let old_surface = {
            let pin = runtime.pinned_shm.lock().unwrap();
            Arc::clone(pin.as_ref().expect("surface pinned during frame"))
        };
        let old_size = old_surface.size();

        let h2 = resources.register(Box::new(CpuBuffer {
            width: 128,
            height: 128,
            data: vec![0x44; 128 * 128 * 4],
        }));
        sink.submit(Frame::new(
            h2,
            128,
            128,
            std::time::Duration::from_millis(32),
            true,
        ))
        .expect("submit large");
        resources.free(h2);

        let refreshed = runtime.get_shm().expect("refreshed surface");
        assert!(refreshed.size() > old_size);

        assert!(Arc::strong_count(&old_surface) >= 2);
        nativis_end_frame(ctx);
        assert_eq!(Arc::strong_count(&old_surface), 1);
        drop(old_surface);

        assert!(nativis_begin_frame(ctx, 128, 128));
        assert_eq!(nativis_get_width(ctx), 128);
        nativis_end_frame(ctx);

        nativis_destroy(ctx);
    }

    #[test]
    fn test_opaque_fallback_and_resume_wait() {
        let shm_name = format!("/nativis_test_opaque_v1_{}", std::process::id());
        let shm_c_str = std::ffi::CString::new(shm_name).unwrap();
        let ctx = nativis_create_with_shm(shm_c_str.as_ptr());
        assert!(!ctx.is_null());

        assert!(nativis_begin_frame(ctx, 4, 4));
        let px = nativis_get_pixels(ctx);
        assert!(!px.is_null());
        let slice = unsafe { std::slice::from_raw_parts(px, 4 * 4 * 4) };
        for chunk in slice.as_chunks::<4>().0 {
            assert_eq!(
                chunk,
                &[0, 0, 0, 255],
                "fallback pixel must be opaque black"
            );
        }
        nativis_end_frame(ctx);

        nativis_wake(ctx);
        let t0 = std::time::Instant::now();
        assert_eq!(nativis_wait_frame(ctx, 0, 60), 0);
        assert!(
            t0.elapsed() < std::time::Duration::from_millis(20),
            "stopped wait must return immediately"
        );

        nativis_resume_wait(ctx);
        let t1 = std::time::Instant::now();
        assert_eq!(nativis_wait_frame(ctx, 0, 50), 0);
        assert!(
            t1.elapsed() >= std::time::Duration::from_millis(40),
            "after nativis_resume_wait, nativis_wait_frame must sleep instead of busy-spinning"
        );

        nativis_destroy(ctx);
    }
}
