use std::{env, fs, path::PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(
        env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set by Cargo"),
    );
    let endpoint_path = manifest_dir.join("../config/telemetry-endpoint.txt");
    println!("cargo:rerun-if-changed={}", endpoint_path.display());

    let endpoint = fs::read_to_string(&endpoint_path)
        .unwrap_or_else(|error| {
            panic!(
                "failed to read telemetry endpoint from {}: {error}",
                endpoint_path.display()
            )
        })
        .trim()
        .to_owned();
    assert!(
        !endpoint.is_empty(),
        "telemetry endpoint configuration must not be empty"
    );
    println!("cargo:rustc-env=OOOSPLAT_TELEMETRY_ENDPOINT={endpoint}");

    let mut attributes = tauri_build::Attributes::new();
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // Tauri's resource compiler links its default manifest only to bin
        // targets, not the library test harness. MCP's app-level export host
        // also brings in rfd/TaskDialogIndirect there, which requires comctl32 v6
        // before any Rust code can run. Let the linker embed that same dependency
        // in every MSVC image, with no duplicate RT_MANIFEST from Tauri's .res.
        // Keep Tauri's icons/version resources and the previous no-UAC manifest.
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTUAC:NO");
        println!("cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
    }
    tauri_build::try_build(attributes).expect("failed to build Tauri resources")
}
