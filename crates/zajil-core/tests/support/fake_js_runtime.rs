//! A stand-in for `deno` or `node` in the tests of `find_js_runtime`:
//! copied under that name, it prints for `--version` what the file
//! `<its name>.version` next to it holds.

fn main() {
    let exe = std::env::current_exe().unwrap();
    let stem = exe.file_stem().unwrap().to_string_lossy().into_owned();
    let version = exe.with_file_name(format!("{stem}.version"));
    if std::env::args().nth(1).as_deref() == Some("--version") {
        print!("{}", std::fs::read_to_string(version).unwrap_or_default());
    }
}
