use std::env;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-env-changed=NATIVIS_PAYLOAD");
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR must be set by Cargo");
    let dest_path = Path::new(&out_dir).join("payload.tar.gz");

    if let Ok(payload_path) = env::var("NATIVIS_PAYLOAD") {
        if Path::new(&payload_path).exists() {
            fs::copy(&payload_path, &dest_path).expect("Failed to copy NATIVIS_PAYLOAD file to OUT_DIR");
            return;
        }
    }

    // Default placeholder for workspace checks and tests when no payload is provided
    let _ = fs::write(&dest_path, b"");
}
