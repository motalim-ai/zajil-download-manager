//! What tools/yt-dlp/pins.txt says of a zip (ADR-0009): unpacks `<zip>` as
//! Zajil does, into a new temporary folder, and prints the zip's SHA-256,
//! then its program and the tree SHA-256 of what it holds.
//!
//!   cargo run -p zajil-core --example tool-tree -- <zip> <program>

use std::io::Read;

use sha2::{Digest, Sha256};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [zip, program] = args.as_slice() else {
        eprintln!("usage: tool-tree <zip> <program>");
        std::process::exit(2);
    };
    let mut file = std::fs::File::open(zip).expect("the zip opens");
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf).expect("the zip reads");
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let sha256: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let dir = tempfile::tempdir().expect("a temporary folder");
    let file = std::fs::File::open(zip).expect("the zip opens");
    match zajil_core::tools::unpack_zip(file, &dir.path().join("tool"), program) {
        Ok(tree) => println!("{sha256} {program} {tree}"),
        Err(why) => {
            eprintln!("{zip}: {why}");
            std::process::exit(1);
        }
    }
}
