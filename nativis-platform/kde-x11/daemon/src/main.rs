use std::process::Command;
use std::thread;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut cmd = Command::new("target/debug/nativis-producer-image");
    for arg in args {
        cmd.arg(arg);
    }
    
    let mut child = cmd
        .spawn()
        .expect("Failed to spawn producer");
        
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {} // Still running
            Err(_) => break,
        }
        thread::sleep(Duration::from_secs(1));
    }
}
