use std::ffi::CString;
use std::os::raw::{c_int, c_void};
use std::ptr;
use std::time::Instant;

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
    ino: u64,
    ptr: *mut c_void,
    unlink_name: Option<CString>,
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
            return Err(format!(
                "shm_open failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        let mut actual_size = size;
        if create {
            if size == 0 {
                unsafe { libc::close(fd) };
                return Err("SHM create size cannot be 0".to_string());
            }
            let res = unsafe { libc::ftruncate(fd, size as libc::off_t) };
            if res < 0 {
                unsafe { libc::close(fd) };
                return Err(format!(
                    "ftruncate failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }

        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd, &mut stat) } < 0 {
            unsafe { libc::close(fd) };
            return Err(format!("fstat failed: {}", std::io::Error::last_os_error()));
        }
        if !create {
            actual_size = stat.st_size as usize;
            if actual_size == 0 {
                unsafe { libc::close(fd) };
                return Err("SHM size is 0".to_string());
            }
        }
        let ino = stat.st_ino;

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

        let unlink_name = if create { Some(c_name) } else { None };
        Ok(Self {
            fd,
            size: actual_size,
            ino,
            ptr,
            unlink_name,
        })
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn inode(&self) -> u64 {
        self.ino
    }

    /// Resizes the backing SHM object in place without unlinking its name from `/dev/shm`.
    pub fn resize(&mut self, new_size: usize) -> Result<(), String> {
        if new_size == 0 {
            return Err("SHM size cannot be 0".to_string());
        }
        if new_size == self.size {
            return Ok(());
        }
        let res = unsafe { libc::ftruncate(self.fd, new_size as libc::off_t) };
        if res < 0 {
            return Err(format!(
                "ftruncate failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let new_ptr = unsafe {
            libc::mmap(
                ptr::null_mut(),
                new_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                self.fd,
                0,
            )
        };
        if new_ptr == libc::MAP_FAILED {
            return Err(format!("mmap failed: {}", std::io::Error::last_os_error()));
        }
        unsafe {
            libc::munmap(self.ptr, self.size);
        }
        self.ptr = new_ptr;
        self.size = new_size;
        Ok(())
    }

    pub fn is_valid(&self) -> bool {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(self.fd, &mut stat) } < 0 {
            return false;
        }
        stat.st_nlink > 0 && stat.st_size as usize == self.size && stat.st_ino == self.ino
    }
}

impl Drop for ShmSurface {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.ptr, self.size);
            libc::close(self.fd);
            // Clean up SHM region from /dev/shm when the creator drops it
            if let Some(ref name) = self.unlink_name {
                libc::shm_unlink(name.as_ptr());
            }
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
use nativis_core::resource::{CpuBuffer, PlanarBuffer, ResourceManager};
use nativis_protocol::{
    NativisAttachment, NativisFrameHeader, NATIVIS_ATTACHMENT_USAGE_COLOR, NATIVIS_FORMAT_NV12,
    NATIVIS_FORMAT_RGBA8888, NATIVIS_MAGIC, NATIVIS_VERSION,
};

/// Maximum allowable SHM buffer size (128 MB, sufficient for 8K NV12 or 4K/5K RGBA).
pub const MAX_SHM_SIZE: usize = 128 * 1024 * 1024;

static NEXT_EPOCH_SEED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn new_session_epoch() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seq = NEXT_EPOCH_SEED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let pid = std::process::id() as u64;
    let token = nanos ^ pid.rotate_left(32) ^ seq.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    if token == 0 {
        1
    } else {
        token
    }
}

pub struct ShmSink {
    name: String,
    surface: ShmSurface,
    resources: ResourceManager,
    frame_count: u64,
    epoch: u64,
    submitted_frames: u64,
    total_submit_us: u64,
    window_start: Instant,
    truncated_warned: bool,
}

const METRICS_INTERVAL: u64 = 300;

impl ShmSink {
    pub fn new(name: &str, size: usize, resources: ResourceManager) -> Result<Self, String> {
        let surface = ShmSurface::new(name, size, true)?;
        // Clear any leftover header bytes if reusing an un-unlinked SHM object after a crash.
        let header_size = std::mem::size_of::<NativisFrameHeader>();
        if surface.size >= header_size {
            unsafe {
                ptr::write_bytes(surface.ptr as *mut u8, 0, header_size);
            }
        }
        // frame_count starts at 1, not 0.
        //
        // frame_id = 0 is reserved as the "no frame / backend not ready" sentinel.
        // FrameWatcher initializes lastSeen = 0, so it only emits a signal when
        // frame_id changes away from 0. If frame_count started at 0, the very
        // first frame would have frame_id = 0 = lastSeen → no signal ever emitted
        // on cold boot (where SHM didn't exist when the plugin first initialised).
        // Starting at 1 guarantees the first real frame always produces a signal.
        Ok(Self {
            name: name.to_string(),
            surface,
            resources,
            frame_count: 1,
            epoch: new_session_epoch(),
            submitted_frames: 0,
            total_submit_us: 0,
            window_start: Instant::now(),
            truncated_warned: false,
        })
    }
}

impl FrameSink for ShmSink {
    fn submit(&mut self, frame: Frame) -> Result<(), MediaError> {
        let started = Instant::now();

        // ── Phase 3c: Dynamic SHM sizing ─────────────────────────────────────
        // Compute the required buffer size for the incoming frame.
        // If it exceeds the current SHM surface size, dynamically re-create the
        // SHM region up to MAX_SHM_SIZE. If it exceeds MAX_SHM_SIZE, log a loud
        // warning and reject the frame without silent truncation (D6, D7).
        let needed_size = self
            .resources
            .acquire(frame.resource, |res| {
                if let Some(planar) = res.as_any().downcast_ref::<PlanarBuffer>() {
                    let header_size = std::mem::size_of::<NativisFrameHeader>();
                    let att_size = std::mem::size_of::<NativisAttachment>();
                    let att_count = planar.planes.len() as u32;
                    let data_start = header_size + att_size * att_count as usize;
                    let mut total_plane_bytes = 0;
                    for (i, plane) in planar.planes.iter().enumerate() {
                        let ph = if i == 0 {
                            planar.height
                        } else {
                            planar.height.div_ceil(2)
                        };
                        total_plane_bytes += plane.stride as usize * ph as usize;
                    }
                    Some(data_start + total_plane_bytes)
                } else if let Some(cpu) = res.as_any().downcast_ref::<CpuBuffer>() {
                    let header_size = std::mem::size_of::<NativisFrameHeader>();
                    let att_size = std::mem::size_of::<NativisAttachment>();
                    let data_offset = header_size + att_size;
                    Some(data_offset + cpu.data.len())
                } else {
                    None
                }
            })
            .flatten();

        if let Some(needed) = needed_size {
            if needed > self.surface.size {
                if needed > MAX_SHM_SIZE {
                    tracing::warn!(
                        "[SHM] Frame size ({} bytes) exceeds maximum allowable limit of {} bytes; frame is rejected",
                        needed, MAX_SHM_SIZE
                    );
                    return Err(MediaError::GpuUpload(format!(
                        "Frame size {} exceeds MAX_SHM_SIZE {}",
                        needed, MAX_SHM_SIZE
                    )));
                }
                tracing::info!(
                    "[SHM] Resizing SHM region '{}' in place from {} to {} bytes for larger frame",
                    self.name,
                    self.surface.size,
                    needed
                );
                self.surface.resize(needed).map_err(|e| {
                    MediaError::GpuUpload(format!("Failed to resize SHM region: {}", e))
                })?;
                self.epoch = self.epoch.wrapping_add(1);
                if self.epoch == 0 {
                    self.epoch = 1;
                }
                self.truncated_warned = false;
            }
        }

        let handle = self.surface.acquire().map_err(MediaError::GpuUpload)?;

        let mut frame_count_increment = 0;
        let success = self.resources.acquire(frame.resource, |res| {
            // ── NV12 multi-plane path (video) ──────────────────────────────
            if let Some(planar) = res.as_any().downcast_ref::<PlanarBuffer>() {
                let header_size = std::mem::size_of::<NativisFrameHeader>();
                let att_size    = std::mem::size_of::<NativisAttachment>();
                let att_count   = planar.planes.len() as u32; // 2 untuk NV12
                let data_start  = header_size + att_size * att_count as usize;

                let format_code = match planar.format {
                    nativis_core::resource::PixelFormat::Nv12 => NATIVIS_FORMAT_NV12,
                    _ => NATIVIS_FORMAT_NV12, // extensible for P010 etc.
                };

                let mut attachments = Vec::with_capacity(planar.planes.len());
                let mut cursor = data_start as u32;

                for (i, plane) in planar.planes.iter().enumerate() {
                    let (pw, ph) = if i == 0 {
                        (planar.width, planar.height)          // plane Y: resolusi penuh
                    } else {
                        // UV plane: half width/height in samples
                        // byte-width UV = planar.width (2 bytes per sample pair)
                        (planar.width.div_ceil(2), planar.height.div_ceil(2))
                    };
                    attachments.push(NativisAttachment {
                        usage: NATIVIS_ATTACHMENT_USAGE_COLOR,
                        format: format_code,
                        width: pw,
                        height: ph,
                        stride: plane.stride,     // stride SEBENARNYA dari FFmpeg
                        planes: att_count,
                        surface_index: i as u32,  // 0 = Y, 1 = UV
                        data_offset: cursor,
                    });
                    cursor += plane.stride * ph;
                }

                // ── Urutan penulisan (Fase 8 safety & Seqlock v4) ──
                // 1. Tulis sequence ganjil (odd = write in progress) via seqlock
                // 2. Tulis SEMUA data pixel dulu (kedua plane)
                // 3. Tulis attachment array
                // 4. Tulis header (frame_id, shm_size, epoch & sequence genap) PALING TERAKHIR
                unsafe {
                    let ptr = handle.ptr;

                    // 1. Mark sequence odd (write in progress)
                    let seq_ptr = ptr.add(std::mem::offset_of!(NativisFrameHeader, sequence)) as *mut u64;
                    let target_frame_id = self.frame_count + 1;
                    let odd_seq = (target_frame_id * 2) - 1;
                    std::ptr::write_volatile(seq_ptr, odd_seq);
                    std::sync::atomic::fence(std::sync::atomic::Ordering::Release);

                    // 2. Pixel data
                    for (i, (att, plane)) in attachments.iter().zip(planar.planes.iter()).enumerate() {
                        let dst = ptr.add(att.data_offset as usize);
                        let avail = handle.size.saturating_sub(att.data_offset as usize);
                        if plane.data.len() > avail && !self.truncated_warned {
                            self.truncated_warned = true;
                            tracing::warn!("SHM region too small: plane {} needs {} bytes, {} available; frame is truncated (further occurrences not logged)", i, plane.data.len(), avail);
                        }
                        let len = std::cmp::min(plane.data.len(), avail);
                        std::ptr::copy_nonoverlapping(plane.data.as_ptr(), dst, len);
                    }

                    // 3. Attachment array
                    let att_ptr = ptr.add(header_size);
                    std::ptr::copy_nonoverlapping(
                        attachments.as_ptr() as *const u8,
                        att_ptr,
                        att_size * attachments.len(),
                    );

                    // 4. Header LAST — sequence genap (even) signals "data is complete"
                    let even_seq = target_frame_id * 2;
                    let header = NativisFrameHeader {
                        magic: NATIVIS_MAGIC,
                        version: NATIVIS_VERSION,
                        frame_id: target_frame_id,
                        timestamp: frame.pts.as_millis() as u64,
                        sequence: even_seq,
                        attachment_count: att_count,
                        attachment_offset: header_size as u32,
                        shm_size: handle.size as u64,
                        epoch: self.epoch,
                    };
                    std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
                    std::ptr::copy_nonoverlapping(
                        &header as *const _ as *const u8,
                        ptr,
                        header_size,
                    );
                    std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
                }
                frame_count_increment = 1;
                true
            }
            // ── RGBA single-plane path (image_backend) ──
            else if let Some(cpu) = res.as_any().downcast_ref::<CpuBuffer>() {
                let header_size = std::mem::size_of::<NativisFrameHeader>();
                let att_size = std::mem::size_of::<NativisAttachment>();
                let data_offset = (header_size + att_size) as u32;

                let target_frame_id = self.frame_count + 1;
                let even_seq = target_frame_id * 2;
                let header = NativisFrameHeader {
                    magic: NATIVIS_MAGIC,
                    version: NATIVIS_VERSION,
                    frame_id: target_frame_id,
                    timestamp: frame.pts.as_millis() as u64,
                    sequence: even_seq,
                    attachment_count: 1,
                    attachment_offset: header_size as u32,
                    shm_size: handle.size as u64,
                    epoch: self.epoch,
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

                // Write to SHM:
                // 1. Mark sequence odd (write in progress)
                // 2. Pixel data FIRST (eliminates tearing window)
                // 3. Attachment
                // 4. Header LAST with even sequence
                unsafe {
                    let ptr = handle.ptr;

                    // 1. Mark sequence odd
                    let seq_ptr = ptr.add(std::mem::offset_of!(NativisFrameHeader, sequence)) as *mut u64;
                    let odd_seq = (target_frame_id * 2) - 1;
                    std::ptr::write_volatile(seq_ptr, odd_seq);
                    std::sync::atomic::fence(std::sync::atomic::Ordering::Release);

                    // 2. Pixel data first
                    let data_ptr = ptr.add(data_offset as usize);
                    let available_size = handle.size.saturating_sub(data_offset as usize);
                    if cpu.data.len() > available_size && !self.truncated_warned {
                        self.truncated_warned = true;
                        tracing::warn!("SHM region too small: frame needs {} bytes, {} available; frame is truncated (further occurrences not logged)", cpu.data.len(), available_size);
                    }
                    std::ptr::copy_nonoverlapping(
                        cpu.data.as_ptr(),
                        data_ptr,
                        std::cmp::min(cpu.data.len(), available_size)
                    );

                    // 3. Attachment
                    std::ptr::copy_nonoverlapping(&attachment as *const _ as *const u8, ptr.add(header_size), att_size);

                    // 4. Header last
                    std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
                    std::ptr::copy_nonoverlapping(&header as *const _ as *const u8, ptr, header_size);
                    std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
                }
                frame_count_increment = 1;
                true
            } else {
                false
            }
        }).unwrap_or(false);

        self.frame_count += frame_count_increment;

        #[cfg(target_os = "linux")]
        if frame_count_increment > 0 {
            // Item 5: Wake waiting FrameWatcher threads via futex on lower 32 bits of frame_id (offset 8).
            // Eliminates 240 Hz busy polling in plasmashell, dropping wakeups from 240/s to 0/s when idle.
            unsafe {
                let futex_ptr = handle.ptr.add(8) as *mut u32;
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

        self.surface.release(handle);

        if !success {
            return Err(MediaError::GpuUpload(
                "Invalid resource type or handle for ShmSink".into(),
            ));
        }

        self.submitted_frames += 1;
        self.total_submit_us += started.elapsed().as_micros() as u64;
        if self.submitted_frames.is_multiple_of(METRICS_INTERVAL) {
            let elapsed_s = self.window_start.elapsed().as_secs_f64().max(0.001);
            tracing::info!(
                "[NATIVIS SHM] submit_fps={:.1} avg_submit_us={}µs",
                self.submitted_frames as f64 / elapsed_s,
                self.total_submit_us / self.submitted_frames,
            );
            self.submitted_frames = 0;
            self.total_submit_us = 0;
            self.window_start = Instant::now();
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_concurrent_seqlock_no_tearing() {
        let shm_name = format!("/nativis_test_shm_{}", std::process::id());
        let shm_size = 64 * 1024; // 64 KB
        let resources = ResourceManager::new();
        let mut sink =
            ShmSink::new(&shm_name, shm_size, resources.clone()).expect("Failed to create ShmSink");

        let stop = Arc::new(AtomicBool::new(false));
        let stop_consumer = stop.clone();
        let shm_name_consumer = shm_name.clone();

        let consumer_handle = std::thread::spawn(move || {
            let consumer_surface =
                ShmSurface::new(&shm_name_consumer, 0, false).expect("Consumer failed to open SHM");
            let mut torn_frames = 0u64;
            let mut read_frames = 0u64;
            let mut last_seq = 0u64;

            while !stop_consumer.load(Ordering::Acquire) {
                let handle = consumer_surface.acquire().expect("acquire");
                let ptr = handle.ptr;
                if handle.size < std::mem::size_of::<NativisFrameHeader>() {
                    continue;
                }
                let header = unsafe { &*(ptr as *const NativisFrameHeader) };
                if header.magic != NATIVIS_MAGIC {
                    continue;
                }

                // Seqlock read protocol
                let seq1 = unsafe { std::ptr::read_volatile(&header.sequence) };
                if seq1 & 1 != 0 {
                    // Write in progress
                    core::hint::spin_loop();
                    continue;
                }
                std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);

                if seq1 == 0 || seq1 <= last_seq {
                    continue;
                }

                let offset = header.attachment_offset as usize;
                if handle.size < offset + std::mem::size_of::<NativisAttachment>() {
                    continue;
                }
                let att = unsafe { *(ptr.add(offset) as *const NativisAttachment) };
                let data_offset = att.data_offset as usize;
                let pixel_len = (att.width * att.height * 4) as usize;

                if handle.size < data_offset + pixel_len {
                    continue;
                }

                let mut local_buf = vec![0u8; pixel_len];
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        ptr.add(data_offset),
                        local_buf.as_mut_ptr(),
                        pixel_len,
                    );
                }

                std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
                let seq2 = unsafe { std::ptr::read_volatile(&header.sequence) };

                if seq1 == seq2 {
                    last_seq = seq1;
                    read_frames += 1;
                    // Verify data integrity: all bytes in local_buf must be equal to expected byte!
                    let expected_byte = local_buf[0];
                    for &b in &local_buf {
                        if b != expected_byte {
                            torn_frames += 1;
                            break;
                        }
                    }
                }
            }

            assert_eq!(
                torn_frames, 0,
                "Torn frames detected during concurrent read/write!"
            );
            assert!(
                read_frames > 0,
                "Consumer should have read at least one frame"
            );
        });

        // Producer thread writes 200 frames
        for frame_idx in 1..=200u32 {
            let fill_byte = (frame_idx & 0xFF) as u8;
            let pixel_data = vec![fill_byte; 64 * 64 * 4];
            let cpu_buf = CpuBuffer {
                width: 64,
                height: 64,
                data: pixel_data,
            };
            let handle = resources.register(Box::new(cpu_buf));
            let frame = Frame::new(
                handle,
                64,
                64,
                std::time::Duration::from_millis(frame_idx as u64 * 16),
                true,
            );
            sink.submit(frame).expect("submit frame");
            resources.free(handle);
            // Small sleep to allow interleaving with consumer
            std::thread::sleep(std::time::Duration::from_micros(50));
        }

        // Give consumer a moment to process final frames, then stop
        std::thread::sleep(std::time::Duration::from_millis(20));
        stop.store(true, Ordering::Release);
        consumer_handle.join().expect("Consumer thread panicked");
    }

    #[test]
    fn test_dynamic_shm_resize_on_larger_frame() {
        let shm_name = format!("/nativis_test_resize_{}", std::process::id());
        let initial_size = 4096; // 4 KB initially (smaller than 64x64x4 RGBA frame)
        let resources = ResourceManager::new();
        let mut sink = ShmSink::new(&shm_name, initial_size, resources.clone())
            .expect("Failed to create ShmSink");
        assert_eq!(sink.surface.size, 4096);

        // Submit a frame needing 64 * 64 * 4 = 16384 bytes + headers
        let cpu_buf = CpuBuffer {
            width: 64,
            height: 64,
            data: vec![0xAA; 64 * 64 * 4],
        };
        let handle = resources.register(Box::new(cpu_buf));
        let frame = Frame::new(handle, 64, 64, std::time::Duration::from_millis(16), true);

        let res = sink.submit(frame);
        assert!(res.is_ok(), "Submit should automatically resize SHM region");
        assert!(sink.surface.size >= 16384 + std::mem::size_of::<NativisFrameHeader>());
        resources.free(handle);
    }

    #[test]
    fn test_shm_rejects_frame_exceeding_max_limit() {
        let shm_name = format!("/nativis_test_overflow_{}", std::process::id());
        let initial_size = 4096;
        let resources = ResourceManager::new();
        let mut sink = ShmSink::new(&shm_name, initial_size, resources.clone())
            .expect("Failed to create ShmSink");

        // Simulate a frame exceeding MAX_SHM_SIZE by registering an oversized PlanarBuffer
        let huge_stride = (MAX_SHM_SIZE as u32) + 1024;
        let planar_buf = PlanarBuffer {
            format: nativis_core::resource::PixelFormat::Nv12,
            width: 8192,
            height: 4320,
            planes: vec![nativis_core::resource::PlaneDesc {
                data: std::sync::Arc::from([]),
                stride: huge_stride,
            }],
        };
        let handle = resources.register(Box::new(planar_buf));
        let frame = Frame::new(
            handle,
            8192,
            4320,
            std::time::Duration::from_millis(16),
            true,
        );

        let res = sink.submit(frame);
        assert!(
            res.is_err(),
            "Submit must reject frame exceeding MAX_SHM_SIZE without silent truncation"
        );
        resources.free(handle);
    }

    #[test]
    fn test_resize_preserves_shm_name_for_consumer_reopen() {
        let shm_name = format!("/nativis_test_resize_reopen_{}", std::process::id());
        let initial_size = 4096;
        let resources = ResourceManager::new();
        let mut sink = ShmSink::new(&shm_name, initial_size, resources.clone())
            .expect("Failed to create ShmSink");

        // Consumer opens SHM before resize
        let initial_consumer =
            ShmSurface::new(&shm_name, 0, false).expect("Initial consumer open failed");
        assert_eq!(initial_consumer.size, initial_size);

        // Producer submits a larger frame triggering resize
        let cpu_buf = CpuBuffer {
            width: 64,
            height: 64,
            data: vec![0xCD; 64 * 64 * 4],
        };
        let handle = resources.register(Box::new(cpu_buf));
        let frame = Frame::new(handle, 64, 64, std::time::Duration::from_millis(16), true);
        sink.submit(frame).expect("resize submit should succeed");
        resources.free(handle);

        // A consumer opening/re-opening AFTER resize must still find the SHM name in /dev/shm
        let reopened = ShmSurface::new(&shm_name, 0, false)
            .expect("Consumer must be able to reopen SHM after producer resize");
        assert!(reopened.size >= 64 * 64 * 4 + std::mem::size_of::<NativisFrameHeader>());
        assert!(reopened.is_valid());
        assert!(!initial_consumer.is_valid(), "Initial consumer size is smaller than resized SHM");

        drop(sink);
        assert!(!reopened.is_valid(), "Dropping ShmSink must unlink SHM (st_nlink == 0)");
        assert!(ShmSurface::new(&shm_name, 0, false).is_err(), "SHM name must be unlinked on sink drop");
    }
}
