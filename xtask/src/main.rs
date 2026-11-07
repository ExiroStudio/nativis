use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

fn main() {
    let mut args = env::args().skip(1);
    let task = match args.next() {
        Some(t) => t,
        None => {
            eprintln!("Usage: cargo xtask <task>");
            eprintln!("Tasks:");
            eprintln!("  bundle-kde          Assemble KDE Plasma 5 (X11) bundle");
            eprintln!("  bundle-kde-wayland  Assemble KDE Plasma 6 (Wayland) bundle");
            eprintln!("  bundle-all          Assemble all platform bundles");
            std::process::exit(1);
        }
    };

    match task.as_str() {
        "bundle-kde" => bundle_kde(),
        "bundle-kde-wayland" => bundle_kde_wayland(),
        "bundle-all" => {
            bundle_kde();
            bundle_kde_wayland();
        }
        _ => {
            eprintln!("Unknown task: {}", task);
            std::process::exit(1);
        }
    }
}

fn bundle_kde_wayland() {
    let workspace_root = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .to_path_buf();

    let plasma_src_dir = workspace_root.join("nativis-platform/kde-wayland/plasma-plugin-qml");
    let bundle_dir = workspace_root.join("platforms/kde-wayland");

    println!("============================================================");
    println!("Assembling KDE Plasma 6 Wayland bundle...");
    println!("============================================================");

    // 1. Build Rust cdylib
    println!("Building Rust C-ABI (nativis-v2-core) for KDE Wayland Plugin...");
    let status = Command::new("cargo")
        .args(&["build", "--release", "-p", "nativis-v2-core"])
        .current_dir(&workspace_root)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .expect("Failed to run cargo build");

    if !status.success() {
        eprintln!("Cargo build failed");
        std::process::exit(1);
    }

    // 2. Build C++ Plugin with CMake
    println!("Building KDE Wayland Plasma Plugin with CMake...");
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

    // 3. Assemble Bundle into platforms/kde-wayland/
    let ui_dir = bundle_dir.join("contents/ui");
    fs::create_dir_all(&ui_dir).unwrap();

    fs::copy(
        build_dir.join("libnativisplugin.so"),
        ui_dir.join("libnativisplugin.so"),
    ).unwrap();

    fs::copy(
        workspace_root.join("target/release/libnativis_v2_core.so"),
        ui_dir.join("libnativis_v2_core.so"),
    ).unwrap();

    let qmldir_src = plasma_src_dir.join("qmldir.in");
    if qmldir_src.exists() {
        let _ = fs::copy(qmldir_src, ui_dir.join("qmldir"));
    }

    println!("Bundle kde-wayland assembled successfully at {:?}!", bundle_dir);
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
        .args(&["build", "--release", "-p", "nativis-v1-core"])
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
    
    // Assemble Bundle into contents/ui
    let ui_dir = bundle_dir.join("contents/ui");
    fs::create_dir_all(&ui_dir).unwrap();

    fs::copy(
        plasma_src_dir.join("demo.qml"),
        ui_dir.join("main.qml")
    ).unwrap();

    fs::copy(
        build_dir.join("libnativisplugin.so"),
        ui_dir.join("libnativisplugin.so")
    ).unwrap();

    fs::copy(
        workspace_root.join("target/release/libnativis_v1_core.so"),
        ui_dir.join("libnativis_v1_core.so")
    ).unwrap();

    fs::write(
        ui_dir.join("qmldir"),
        "module org.nativis\nplugin nativisplugin\n"
    ).unwrap();

    // Clean up old system-qml if it exists
    let _ = fs::remove_dir_all(bundle_dir.join("system-qml"));

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
    },
    "KPackageStructure": "Plasma/Wallpaper"
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
