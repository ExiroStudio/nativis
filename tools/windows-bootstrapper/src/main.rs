use std::env;
use std::fs;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use flate2::read::GzDecoder;
use tar::Archive;

const PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.tar.gz"));

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if PAYLOAD.is_empty() {
        eprintln!("Error: Nativis standalone package payload is empty. Build with NATIVIS_PAYLOAD set.");
        std::process::exit(1);
    }

    let checksum = compute_checksum(PAYLOAD);
    let app_dir = get_app_version_dir(&checksum)?;

    #[cfg(target_os = "windows")]
    let exe_name = "nativis.exe";
    #[cfg(not(target_os = "windows"))]
    let exe_name = "nativis";

    let target_exe = app_dir.join(exe_name);

    // Extract payload if not already present
    if !target_exe.exists() {
        extract_payload(PAYLOAD, &app_dir)?;
    }

    // Forward all CLI arguments, stdio, and exit code
    let args: Vec<_> = env::args_os().skip(1).collect();
    let status = Command::new(&target_exe)
        .args(&args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    std::process::exit(status.code().unwrap_or(0));
}

/// Computes a fast 64-bit FNV-1a hash of the payload bytes.
pub fn compute_checksum(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", hash)
}

/// Determines the isolated version directory for extracting runtime files:
/// %LOCALAPPDATA%\Nativis\versions\<checksum>\ (or fallback to %TEMP%\Nativis\versions\<checksum>\).
fn get_app_version_dir(checksum: &str) -> Result<PathBuf, io::Error> {
    if let Ok(local_app_data) = env::var("LOCALAPPDATA") {
        let dir = PathBuf::from(local_app_data)
            .join("Nativis")
            .join("versions")
            .join(checksum);
        if fs::create_dir_all(&dir).is_ok() {
            return Ok(dir);
        }
    }

    // Fallback if LOCALAPPDATA is unavailable or read-only
    let temp_dir = env::temp_dir()
        .join("Nativis")
        .join("versions")
        .join(checksum);
    fs::create_dir_all(&temp_dir)?;
    Ok(temp_dir)
}

/// Unpacks the compressed tar.gz payload into the target directory.
fn extract_payload(payload: &[u8], target_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(target_dir)?;

    let tar_gz = Cursor::new(payload);
    let gz = GzDecoder::new(tar_gz);
    let mut archive = Archive::new(gz);

    archive.unpack(target_dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_checksum() {
        let data = b"nativis test payload";
        let c1 = compute_checksum(data);
        let c2 = compute_checksum(data);
        assert_eq!(c1, c2);
        assert_eq!(c1.len(), 16);
    }

    #[test]
    fn test_extract_payload_in_memory() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use tar::Builder;

        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut tar = Builder::new(&mut enc);
            let mut header = tar::Header::new_gnu();
            let content = b"hello nativis";
            header.set_size(content.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, "test.txt", &content[..]).unwrap();
            tar.finish().unwrap();
        }
        let gz_bytes = enc.finish().unwrap();

        let temp_dir = tempfile::tempdir().unwrap();
        extract_payload(&gz_bytes, temp_dir.path()).unwrap();

        let extracted_file = temp_dir.path().join("test.txt");
        assert!(extracted_file.exists());
        assert_eq!(fs::read_to_string(extracted_file).unwrap(), "hello nativis");
    }
}
