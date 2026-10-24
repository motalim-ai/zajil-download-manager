fn main() {
    // On Windows the app manifest (Common Controls v6, which Tauri's window
    // code needs) is embedded by the linker in every target, tests included:
    // tauri-build embeds it in the app only, so a unit test running Tauri's
    // mock runtime did not start (STATUS_ENTRYPOINT_NOT_FOUND, #47). The
    // manifest is tauri-build's own default, unchanged.
    let windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("tauri-build");
    let target = |key: &str| std::env::var(key).unwrap_or_default();
    if target("CARGO_CFG_TARGET_OS") == "windows" && target("CARGO_CFG_TARGET_ENV") == "msvc" {
        let manifest =
            std::path::Path::new(&target("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
}
