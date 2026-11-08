use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // On Windows the app manifest (Common Controls v6, which Tauri's window
    // code needs) is embedded in every target, tests included: tauri-build
    // embeds it in the app only, so a unit test running Tauri's mock runtime
    // did not start (STATUS_ENTRYPOINT_NOT_FOUND, #47). The manifest is
    // tauri-build's own default, unchanged. MSVC's linker takes it as it is;
    // with the GNU toolchain it goes in as a resource made by windres (#333).
    let windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("tauri-build");
    if target("CARGO_CFG_TARGET_OS") != "windows" {
        return;
    }
    let manifest = Path::new(&target("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    match target("CARGO_CFG_TARGET_ENV").as_str() {
        "msvc" => {
            println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
            println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        }
        "gnu" => embed_with_windres(&manifest),
        _ => {}
    }
}

fn target(key: &str) -> String {
    std::env::var(key).unwrap_or_default()
}

/// GNU's linker has no manifest options: the manifest becomes a resource
/// (`1 24`, the app's own manifest) that windres compiles, linked in every
/// target. The tool is `WINDRES` when set, else the target's own name (as
/// when building from Linux or macOS), else `windres`. Without one the build
/// goes on with a warning: only the tests need the manifest to start.
fn embed_with_windres(manifest: &Path) {
    println!("cargo:rerun-if-env-changed=WINDRES");
    let out = PathBuf::from(target("OUT_DIR"));
    let rc = out.join("manifest.rc");
    let object = out.join("manifest.o");
    let path = manifest.display().to_string().replace('\\', "/");
    std::fs::write(&rc, format!("1 24 \"{path}\"\n")).expect("write manifest.rc");
    let tools = std::env::var("WINDRES").ok().into_iter().chain([
        format!("{}-w64-mingw32-windres", target("CARGO_CFG_TARGET_ARCH")),
        "windres".to_owned(),
    ]);
    for tool in tools {
        match Command::new(&tool)
            .arg(&rc)
            .args(["-O", "coff", "-o"])
            .arg(&object)
            .status()
        {
            Ok(status) if status.success() => {
                println!("cargo:rustc-link-arg={}", object.display());
                return;
            }
            Ok(status) => {
                println!(
                    "cargo:warning={tool} could not compile the app manifest ({status}): the tests will not start"
                );
                return;
            }
            // Not there: the next name.
            Err(_) => {}
        }
    }
    println!(
        "cargo:warning=no windres (set WINDRES to it): the app manifest is not embedded, and the tests will not start"
    );
}
