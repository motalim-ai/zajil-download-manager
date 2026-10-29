//! The proxy of the settings: every request goes through it.

use zajil_core::settings::check_proxy;
use zajil_core::{Engine, Options};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

/// The test server stands in for the proxy: the download names a host
/// that does not exist (`.invalid`, RFC 6761), so it can only arrive
/// through the proxy, which serves the path it is asked for.
#[tokio::test]
async fn a_download_goes_through_the_proxy() {
    let server = TestServer::http();
    let data = random_bytes(300_000, 9);
    server.put("/f.bin", FileSpec::new(data.clone()));
    let engine = Engine::new(Options {
        proxy: Some(format!("http://{}", server.addr())),
        ..Options::default()
    })
    .unwrap();
    let url = format!("http://no-such-host.invalid{}", server.path("/f.bin"));
    let dir = tempfile::tempdir().unwrap();
    let done = engine
        .download(&url, dir.path(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(
        sha256_hex(&std::fs::read(&done.file).unwrap()),
        sha256_hex(&data)
    );
    assert!(!server.gets("/f.bin").is_empty());
}

#[test]
fn only_a_plain_proxy_address_is_taken() {
    for good in [
        "http://proxy.example:8080",
        "https://10.0.0.1:3128/",
        "http://[::1]:8080",
    ] {
        assert!(check_proxy(good).is_ok(), "{good}");
    }
    for bad in [
        "proxy.example:8080",
        "socks5://proxy.example:1080",
        "http://user:secret@proxy.example:8080",
        "http://proxy.example:8080/path",
        "http://proxy.example:8080/?q",
        "ftp://proxy.example",
    ] {
        let said = check_proxy(bad).unwrap_err();
        assert!(!said.contains("secret"), "{said}");
    }
    // An engine refuses one too.
    assert!(
        Engine::new(Options {
            proxy: Some("http://user:pw@proxy.example:8080".into()),
            ..Options::default()
        })
        .is_err()
    );
}
