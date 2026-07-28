use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn main() {
    let mut args = env::args().skip(1);
    let task = match args.next() {
        Some(t) => t,
        None => {
            eprintln!("Usage: cargo xtask <task>");
            eprintln!("Tasks:");
            eprintln!("  bundle-kde");
            std::process::exit(1);
        }
    };

    match task.as_str() {
        "bundle-kde" => bundle_kde(),
        _ => {
            eprintln!("Unknown task: {}", task);
            std::process::exit(1);
        }
    }
}

fn bundle_kde() {
    let workspace_root = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .to_path_buf();
    
    let plasma_src_dir = workspace_root.join("nativis-platform/kde-x11/plasma-plugin-qml");
    let bundle_dir = workspace_root.join("platforms/kde-x11");
    
    // 1. Build Rust cdylib
    println!("Building Rust C-ABI for KDE Plasma Plugin...");
    let status = Command::new("cargo")
        .args(&["build", "-p", "nativis-v1-core"])
        .current_dir(&workspace_root)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .expect("Failed to run cargo build");
    
    if !status.success() {
        eprintln!("Cargo build failed");
        std::process::exit(1);
    }

    // 2. Build C++ Plugin
    println!("Building KDE Plasma Plugin with CMake...");
    let build_dir = plasma_src_dir.join("build");
    let _ = fs::remove_dir_all(&build_dir);
    fs::create_dir_all(&build_dir).unwrap();

    let status = Command::new("cmake")
        .arg("..")
        .current_dir(&build_dir)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .expect("Failed to run cmake");
    
    if !status.success() {
        eprintln!("CMake failed");
        std::process::exit(1);
    }

    let status = Command::new("make")
        .current_dir(&build_dir)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .expect("Failed to run make");
    
    if !status.success() {
        eprintln!("Make failed");
        std::process::exit(1);
    }

    // 2. Assemble Bundle
    println!("Assembling bundle in {:?}...", bundle_dir);
    fs::create_dir_all(bundle_dir.join("contents/ui")).unwrap();

    // Copy QML
    fs::copy(
        plasma_src_dir.join("demo.qml"),
        bundle_dir.join("contents/ui/main.qml")
    ).unwrap();

    // Copy .so
    fs::copy(
        build_dir.join("libnativisplugin.so"),
        bundle_dir.join("contents/libnativisplugin.so") // Or wherever QML plugin expects it
    ).unwrap();

    // Copy qmldir
    fs::write(
        bundle_dir.join("contents/qmldir"),
        "module org.nativis\nplugin nativisplugin .\n"
    ).unwrap();

    // Copy C-ABI .so so QML plugin can load it at runtime?
    // Wait, QML plugin links against it. We should copy it to the bundle as well if it's dynamic.
    // Or bundle libnativis_v1_core.so as well.
    fs::copy(
        workspace_root.join("target/debug/libnativis_v1_core.so"),
        bundle_dir.join("contents/libnativis_v1_core.so")
    ).unwrap();

    // Generate metadata.json
    let metadata = r#"{
    "KPlugin": {
        "Name": "Nativis Wallpaper",
        "Description": "High-performance multimedia wallpaper engine",
        "Id": "com.nativis.wallpaper",
        "Version": "1.0",
        "Category": "Wallpaper",
        "Authors": [
            {
                "Name": "Nativis Contributors"
            }
        ],
        "ServiceTypes": [
            "Plasma/Wallpaper"
        ]
    }
}"#;
    fs::write(bundle_dir.join("metadata.json"), metadata).unwrap();

    // Generate manifest.toml
    let manifest = r#"name = "kde-x11"
version = "1.0.0"
protocol = 2
"#;
    fs::write(bundle_dir.join("manifest.toml"), manifest).unwrap();

    println!("Bundle kde-x11 assembled successfully!");
}
