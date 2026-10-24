//! HTTPS: a certificate from a trusted authority works, and anything else
//! fails, with no way around it.
//!
//! The trusted authority is `zajil-testserver`'s temporary CA, created in
//! memory for this test run. Only these tests trust it, through
//! `Engine::with_test_root`, which exists only with the `test-support`
//! feature that dev-dependencies enable. Every success test has a failure
//! twin on the same kind of server, so a broken TLS setup cannot pass as
//! "refuses bad certificates".

use std::fs;
use std::path::Path;

use zajil_core::{Engine, Error, Event, Options};
use zajil_testserver::{FileSpec, TestServer, Tls, random_bytes, sha256_hex, test_ca_der};

fn trusting_engine() -> Engine {
    Engine::with_test_root(Options::default(), &test_ca_der()).unwrap()
}

fn system_engine() -> Engine {
    Engine::new(Options::default()).unwrap()
}

async fn assert_refused(engine: Engine, server: &TestServer, expected: &str) {
    server.put("/f.bin", FileSpec::new(vec![1u8; 1000]));
    let dir = tempfile::tempdir().unwrap();
    let mut events = Vec::new();
    let err = engine
        .download(&server.url("/f.bin"), dir.path(), &mut |e| events.push(e))
        .await
        .expect_err("the certificate must be refused");
    // A certificate that cannot be trusted does not pass by itself: it is
    // never tried again (M3 retries only failures that may pass).
    assert!(
        !events.iter().any(|e| matches!(e, Event::Retrying { .. })),
        "{events:?}"
    );

    assert!(matches!(err, Error::Http(_)), "unexpected error: {err}");
    let message = err.to_string().to_lowercase();
    assert!(message.contains("certificate"), "unclear error: {err}");
    assert!(
        message.contains(expected),
        "expected {expected:?} in: {err}"
    );
    // The TLS handshake failed, so no HTTP request reached the server and
    // nothing was written.
    assert!(server.requests().is_empty());
    assert!(server.strays().is_empty(), "{:?}", server.strays());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn downloads_over_https_with_a_trusted_certificate_and_tls_1_3() {
    let server = TestServer::https(Tls::trusted(&["localhost", "127.0.0.1"]), "localhost");
    let data = random_bytes(5 * 1024 * 1024, 16);
    server.put("/f.bin", FileSpec::new(data.clone()));
    let dir = tempfile::tempdir().unwrap();

    let done = trusting_engine()
        .download(&server.url("/f.bin"), dir.path(), &mut |_| {})
        .await
        .unwrap();

    assert_eq!(sha256_hex(&fs::read(done.file).unwrap()), sha256_hex(&data));
    let request = &server.gets("/f.bin")[0];
    assert_eq!(request.tls_version.as_deref(), Some("TLSv1_3"));
}

#[tokio::test]
async fn the_same_trusted_certificate_fails_without_the_test_root() {
    // The shipped engine only trusts the system store.
    let server = TestServer::https(Tls::trusted(&["localhost"]), "localhost");
    assert_refused(system_engine(), &server, "").await;
}

#[tokio::test]
async fn refuses_a_self_signed_certificate() {
    let server = TestServer::https(Tls::self_signed(&["localhost", "127.0.0.1"]), "localhost");
    assert_refused(trusting_engine(), &server, "").await;
}

#[tokio::test]
async fn refuses_a_trusted_certificate_for_another_host() {
    let server = TestServer::https(Tls::trusted(&["other.invalid"]), "localhost");
    assert_refused(trusting_engine(), &server, "").await;
}

#[tokio::test]
async fn refuses_a_trusted_certificate_that_does_not_cover_an_ip_address_url() {
    let server = TestServer::https(Tls::trusted(&["localhost"]), "127.0.0.1");
    assert_refused(trusting_engine(), &server, "").await;
}

#[tokio::test]
async fn refuses_an_expired_certificate_from_a_trusted_authority() {
    let server = TestServer::https(Tls::expired(&["localhost"]), "localhost");
    assert_refused(trusting_engine(), &server, "expired").await;
}

#[tokio::test]
async fn refuses_a_redirect_from_https_to_http() {
    let plain = TestServer::http();
    plain.put("/f.bin", FileSpec::new(vec![1u8; 1000]));
    let secure = TestServer::https(Tls::trusted(&["localhost"]), "localhost");
    secure.redirect("/start", &plain.url("/f.bin"), 302);
    let dir = tempfile::tempdir().unwrap();

    let err = trusting_engine()
        .download(&secure.url("/start"), dir.path(), &mut |_| {})
        .await
        .unwrap_err();

    assert!(err.to_string().contains("https to http"), "{err}");
    assert_eq!(secure.requests().len(), 1);
    assert!(plain.requests().is_empty(), "the http URL was requested");
    assert!(plain.strays().is_empty(), "{:?}", plain.strays());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

/// A guard against regressions: no source file may call the escape hatches
/// that turn certificate checks off or replace the verifier.
#[test]
fn no_source_file_disables_certificate_checks() {
    let forbidden = [
        "danger_accept_invalid",
        "tls_danger_",
        "tls_certs_only",
        // Writes TLS session keys to a file when enabled.
        "tls_sslkeylogfile",
        "dangerous(",
        "NoCertificateVerification",
        "ServerCertVerifier",
        "with_custom_certificate_verifier",
        "ServerCertVerified::assertion",
    ];
    // Only this exact file may name them (in the list above).
    let this_file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("tls.rs")
        .canonicalize()
        .unwrap();
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut stack = vec![crates];
    let mut checked = 0;
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.canonicalize().unwrap() != this_file
            {
                let text = fs::read_to_string(&path).unwrap();
                for word in forbidden {
                    assert!(!text.contains(word), "{} uses {word}", path.display());
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 5, "only {checked} files checked");
}
