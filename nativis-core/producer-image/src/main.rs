use std::thread;
use std::time::Duration;
use nativis_protocol::{
    NativisFrameHeader, NativisAttachment, NATIVIS_MAGIC, 
    NATIVIS_ATTACHMENT_USAGE_COLOR, NATIVIS_FORMAT_RGBA8888
};
use nativis_transport_shm::{ShmSurface, SurfaceOps};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let img_path = if args.len() > 1 { Some(&args[1]) } else { None };
    
    // Load image if provided
    let (img_rgba, img_width, img_height) = if let Some(path) = img_path {
        let img = image::open(path).expect("Failed to open image file");
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        (Some(rgba), w, h)
    } else {
        (None, 1920, 1080)
    };
    
    let width = img_width as usize;
    let height = img_height as usize;
    
    // Offset attachments to start after header
    let attachment_offset = std::mem::size_of::<NativisFrameHeader>(); 
    // Data starts at 256 bytes offset
    let data_offset = 256; 
    
    let total_size = data_offset + (width * height * 4);
    
    // Create shared memory
    let shm = ShmSurface::new("/nativis_shm", total_size, true)
        .expect("Failed to create shared memory");
        
    let handle = shm.acquire().unwrap();
    let ptr = handle.ptr;
    
    let mut frame_id = 1;
    
    loop {
        unsafe {
            // Write Header
            let header_ptr = ptr as *mut NativisFrameHeader;
            (*header_ptr).magic = NATIVIS_MAGIC;
            (*header_ptr).version = 2;
            (*header_ptr).frame_id = frame_id;
            (*header_ptr).timestamp = 0; // TODO: system time
            (*header_ptr).attachment_count = 1;
            (*header_ptr).attachment_offset = attachment_offset as u32;
            
            // Write Attachment metadata
            let att_ptr = ptr.add(attachment_offset) as *mut NativisAttachment;
            (*att_ptr).usage = NATIVIS_ATTACHMENT_USAGE_COLOR;
            (*att_ptr).format = NATIVIS_FORMAT_RGBA8888;
            (*att_ptr).width = width as u32;
            (*att_ptr).height = height as u32;
            (*att_ptr).stride = (width * 4) as u32;
            (*att_ptr).planes = 1;
            (*att_ptr).surface_index = 0; // inline
            
            // Write pixel data
            let data_ptr = ptr.add(data_offset);
            let slice = std::slice::from_raw_parts_mut(data_ptr, width * height * 4);
            
            if let Some(rgba) = &img_rgba {
                // Static image path: just copy the pixels
                slice.copy_from_slice(rgba.as_raw());
            } else {
                // Gradient fallback
                let fc = (frame_id % 255) as u8;
                for y in 0..height {
                    let y_val = ((y + (fc as usize * 2)) % 255) as u8;
                    let row_offset = y * width * 4;
                    for x in 0..width {
                        let x_val = ((x + (fc as usize * 3)) % 255) as u8;
                        let idx = row_offset + x * 4;
                        slice[idx] = x_val;     // R
                        slice[idx + 1] = y_val; // G
                        slice[idx + 2] = fc;    // B
                        slice[idx + 3] = 255;   // A
                    }
                }
            }
        }
        
        frame_id += 1;
        thread::sleep(Duration::from_millis(16)); // ~60fps
    }
}
