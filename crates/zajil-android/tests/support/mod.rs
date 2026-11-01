//! Signing with a throwaway key, in minisign's own format (prehashed
//! Ed25519), as `tauri signer` writes the project's real keys: so that the
//! tests can check a good signature is taken, not only that bad ones are
//! refused. The project's keys must refuse what this key signs.

#![allow(dead_code)]

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use blake2::{Blake2b512, Digest};
use ed25519_dalek::{Signer, SigningKey};

pub const KEY_ID: [u8; 8] = *b"zajiltst";

pub fn key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

/// The public key as the project keeps its own: base64 of minisign's file.
pub fn public_key(key: &SigningKey) -> String {
    let mut raw = b"Ed".to_vec();
    raw.extend_from_slice(&KEY_ID);
    raw.extend_from_slice(key.verifying_key().as_bytes());
    let file = format!(
        "untrusted comment: minisign public key: test\n{}\n",
        STANDARD.encode(raw)
    );
    STANDARD.encode(file)
}

/// minisign's signature file for `data` (prehashed, with a trusted comment).
pub fn sign(key: &SigningKey, data: &[u8]) -> String {
    let signature = key.sign(&Blake2b512::digest(data)).to_bytes();
    let mut raw = b"ED".to_vec();
    raw.extend_from_slice(&KEY_ID);
    raw.extend_from_slice(&signature);
    let trusted = "timestamp:1793000000\tfile:tools-android.json";
    let mut global = signature.to_vec();
    global.extend_from_slice(trusted.as_bytes());
    format!(
        "untrusted comment: signature from a test key\n{}\ntrusted comment: {trusted}\n{}\n",
        STANDARD.encode(raw),
        STANDARD.encode(key.sign(&global).to_bytes()),
    )
}
