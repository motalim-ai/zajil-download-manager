//! The HTTPS modes, checked with a plain rustls client that trusts only this
//! test run's temporary CA (and not the system store).

use std::sync::Arc;

use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use zajil_testserver::{FileSpec, TestServer, Tls, test_ca_der};

async fn fetch(server: &TestServer, name: &str) -> Result<String, std::io::Error> {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(test_ca_der()))
        .expect("test CA is a valid certificate");
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let tcp = TcpStream::connect(server.addr()).await?;
    let mut tls = TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from(name.to_owned()).unwrap(), tcp)
        .await?;
    tls.write_all(
        format!(
            "GET {} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
            server.path("/f")
        )
        .as_bytes(),
    )
    .await?;
    let mut reply = String::new();
    tls.read_to_string(&mut reply).await?;
    Ok(reply)
}

#[tokio::test]
async fn a_trusted_certificate_works_over_tls_1_3() {
    let server = TestServer::https(Tls::trusted(&["localhost", "127.0.0.1"]), "localhost");
    server.put("/f", FileSpec::new(b"hello".to_vec()));
    let reply = fetch(&server, "localhost").await.unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.ends_with("hello"));
    assert_eq!(server.gets("/f")[0].tls_version.as_deref(), Some("TLSv1_3"));
}

#[tokio::test]
async fn an_expired_certificate_is_refused_by_a_client_that_trusts_the_ca() {
    let server = TestServer::https(Tls::expired(&["localhost"]), "localhost");
    server.put("/f", FileSpec::new(b"hello".to_vec()));
    let err = fetch(&server, "localhost").await.unwrap_err();
    assert!(err.to_string().to_lowercase().contains("expired"), "{err}");
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn a_self_signed_certificate_is_not_signed_by_the_ca() {
    let server = TestServer::https(Tls::self_signed(&["localhost"]), "localhost");
    server.put("/f", FileSpec::new(b"hello".to_vec()));
    assert!(fetch(&server, "localhost").await.is_err());
}

#[test]
fn the_ca_is_created_once_per_test_run() {
    assert_eq!(test_ca_der(), test_ca_der());
}
