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
    if target("CARGO_CFG_TARGET_OS") == "windows" {
        let manifest =
            std::path::Path::new(&target("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        if target("CARGO_CFG_TARGET_ENV") == "msvc" {
            println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
            println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        } else if target("CARGO_CFG_TARGET_ENV") == "gnu" {
            let out_dir = std::path::PathBuf::from(target("OUT_DIR"));
            let rc_path = out_dir.join("manifest.rc");
            let res_path = out_dir.join("manifest.o");
            let manifest_str = manifest.to_string_lossy().replace('\\', "/");
            std::fs::write(&rc_path, format!("1 24 \"{}\"\n", manifest_str)).expect("write rc");
            let status = std::process::Command::new("windres")
                .arg(&rc_path)
                .arg("-O")
                .arg("coff")
                .arg("-o")
                .arg(&res_path)
                .status()
                .expect("windres");
            if status.success() {
                println!("cargo:rustc-link-arg={}", res_path.display());
            }
        }
    }
}
