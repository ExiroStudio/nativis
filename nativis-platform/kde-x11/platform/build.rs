use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let bundle_ui = Path::new(&manifest_dir).join("../../../platforms/kde-x11/contents/ui");

    let plugin_so = bundle_ui.join("libnativisplugin.so");
    let core_so = bundle_ui.join("libnativis_v1_core.so");

    println!("cargo:rerun-if-changed={}", plugin_so.display());
    println!("cargo:rerun-if-changed={}", core_so.display());

    let out_dir = env::var("OUT_DIR").unwrap();
    let out_file = Path::new(&out_dir).join("embedded_bundle.rs");

    let plugin_expr = if plugin_so.exists() {
        format!(
            "Some(include_bytes!(r#\"{}\"#))",
            plugin_so.canonicalize().unwrap_or(plugin_so).display()
        )
    } else {
        "None".to_string()
    };

    let core_expr = if core_so.exists() {
        format!(
            "Some(include_bytes!(r#\"{}\"#))",
            core_so.canonicalize().unwrap_or(core_so).display()
        )
    } else {
        "None".to_string()
    };

    let code = format!(
        "const LIBNATIVISPLUGIN_SO: Option<&[u8]> = {};\nconst LIBNATIVIS_V1_CORE_SO: Option<&[u8]> = {};\n",
        plugin_expr, core_expr
    );
    fs::write(out_file, code).expect("write embedded_bundle.rs");
}
