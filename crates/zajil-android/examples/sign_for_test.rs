//! For the emulator test of the update only (android/README.md): signs
//! files with a throwaway key, in minisign's format, as the release workflow
//! signs with the project's key. Prints the key as the debug build takes it
//! (`-Pzajil.updateTestKey=`). The project's keys refuse what it signs.
//!
//!   cargo run -p zajil-android --example sign_for_test -- <file>...

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use blake2::{Blake2b512, Digest};
use ed25519_dalek::{Signer, SigningKey};

const KEY_ID: [u8; 8] = *b"zajiltst";

fn main() {
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let mut raw = b"Ed".to_vec();
    raw.extend_from_slice(&KEY_ID);
    raw.extend_from_slice(key.verifying_key().as_bytes());
    let public = format!(
        "untrusted comment: minisign public key: test\n{}\n",
        STANDARD.encode(raw)
    );
    println!("{}", STANDARD.encode(public));
    for path in std::env::args().skip(1) {
        let data = std::fs::read(&path).expect("read the file");
        let signature = key.sign(&Blake2b512::digest(&data)).to_bytes();
        let mut raw = b"ED".to_vec();
        raw.extend_from_slice(&KEY_ID);
        raw.extend_from_slice(&signature);
        let trusted = "timestamp:1793000000\tfile:test";
        let mut global = signature.to_vec();
        global.extend_from_slice(trusted.as_bytes());
        let text = format!(
            "untrusted comment: signature from a test key\n{}\ntrusted comment: {trusted}\n{}\n",
            STANDARD.encode(raw),
            STANDARD.encode(key.sign(&global).to_bytes()),
        );
        std::fs::write(format!("{path}.minisig"), text).expect("write the signature");
    }
}
