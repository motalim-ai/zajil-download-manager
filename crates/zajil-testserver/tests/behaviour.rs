//! Checks each simulated case with a minimal raw HTTP/1.1 client, so the
//! server is tested independently of the engine.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use zajil_testserver::{Failure, FileSpec, TestServer, random_bytes};

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// The connection ended with an error or before the declared length.
    incomplete: bool,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn request(server: &TestServer, method: &str, path: &str, extra: &[(&str, &str)]) -> Reply {
    let mut stream = TcpStream::connect(server.addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n");
    for (k, v) in extra {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    stream.write_all(req.as_bytes()).unwrap();

    let mut raw = Vec::new();
    let mut buf = [0u8; 65536];
    let mut errored = false;
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(_) => {
                errored = true;
                break;
            }
        }
    }
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("response head");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let body = raw[split + 4..].to_vec();
    let mut lines = head.lines();
    let status = lines.next().unwrap()[9..12].parse().unwrap();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let mut reply = Reply {
        status,
        headers,
        body,
        incomplete: errored,
    };
    if let Some(len) = reply.header("content-length") {
        let len: usize = len.parse().unwrap();
        reply.incomplete |= reply.body.len() < len;
    }
    reply
}

fn get(server: &TestServer, path: &str, extra: &[(&str, &str)]) -> Reply {
    request(server, "GET", path, extra)
}

#[test]
fn serves_a_whole_file_with_validators() {
    let server = TestServer::http();
    let data = random_bytes(200_000, 1);
    server.put("/f", FileSpec::new(data.clone()).etag(Some("\"abc\"")));
    let reply = get(&server, "/f", &[]);
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, data);
    assert_eq!(reply.header("etag"), Some("\"abc\""));
    assert_eq!(reply.header("accept-ranges"), Some("bytes"));
    assert!(reply.header("last-modified").is_some());
    assert_eq!(server.gets("/f")[0].bytes_sent, 200_000);
}

#[test]
fn answers_head_without_a_body() {
    let server = TestServer::http();
    server.put("/f", FileSpec::new(vec![1u8; 10]));
    let reply = request(&server, "HEAD", "/f", &[]);
    assert_eq!(reply.status, 200);
    assert_eq!(reply.header("content-length"), Some("10"));
    assert!(reply.body.is_empty());
    assert!(server.gets("/f").is_empty());
}

#[test]
fn serves_ranges_and_honours_if_range() {
    let server = TestServer::http();
    let data = random_bytes(1000, 2);
    server.put("/f", FileSpec::new(data.clone()).etag(Some("\"v\"")));

    let partial = get(
        &server,
        "/f",
        &[("Range", "bytes=100-"), ("If-Range", "\"v\"")],
    );
    assert_eq!(partial.status, 206);
    assert_eq!(partial.header("content-range"), Some("bytes 100-999/1000"));
    assert_eq!(partial.body, &data[100..]);

    // Another version: the whole new file, never a mix.
    let changed = get(
        &server,
        "/f",
        &[("Range", "bytes=100-"), ("If-Range", "\"old\"")],
    );
    assert_eq!(changed.status, 200);
    assert_eq!(changed.body, data);

    let unsatisfiable = get(&server, "/f", &[("Range", "bytes=5000-")]);
    assert_eq!(unsatisfiable.status, 416);
    assert_eq!(unsatisfiable.header("content-range"), Some("bytes */1000"));

    let log = server.gets("/f");
    assert_eq!(log[0].range.as_deref(), Some("bytes=100-"));
    assert_eq!(log[0].if_range.as_deref(), Some("\"v\""));
}

#[test]
fn can_ignore_ranges() {
    let server = TestServer::http();
    let data = random_bytes(1000, 3);
    server.put("/f", FileSpec::new(data.clone()).without_ranges());
    let reply = get(&server, "/f", &[("Range", "bytes=500-")]);
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, data);
    assert_eq!(reply.header("accept-ranges"), None);
}

#[test]
fn cuts_the_connection_for_the_first_requests_only() {
    let server = TestServer::http();
    let data = random_bytes(1_000_000, 4);
    server.put("/f", FileSpec::new(data.clone()).cut_after(300_000, 1));

    let cut = get(&server, "/f", &[]);
    assert_eq!(cut.status, 200);
    assert!(cut.incomplete);
    assert!(cut.body.len() <= 300_000);
    assert_eq!(server.gets("/f")[0].bytes_sent, 300_000);

    let whole = get(&server, "/f", &[]);
    assert!(!whole.incomplete);
    assert_eq!(whole.body, data);
}

#[test]
fn replacing_a_file_changes_its_etag() {
    let server = TestServer::http();
    server.put("/f", FileSpec::new(vec![1u8; 10]));
    let first = get(&server, "/f", &[]);
    server.put("/f", FileSpec::new(vec![2u8; 20]));
    let second = get(&server, "/f", &[]);
    assert_ne!(first.header("etag"), second.header("etag"));
    assert_eq!(second.body, vec![2u8; 20]);
}

#[test]
fn stalls_without_closing() {
    let server = TestServer::http();
    server.put(
        "/f",
        FileSpec::new(random_bytes(100_000, 5)).stall_after(1000),
    );
    let started = Instant::now();
    let reply = get(&server, "/f", &[]); // ends by the client's 2 s read timeout
    assert!(started.elapsed() >= Duration::from_secs(2));
    assert!(reply.incomplete);
    assert_eq!(reply.body.len(), 1000);
}

#[test]
fn throttles() {
    let server = TestServer::http();
    server.put(
        "/f",
        FileSpec::new(random_bytes(50_000, 6)).throttle(100_000),
    );
    let started = Instant::now();
    let reply = get(&server, "/f", &[]);
    assert_eq!(reply.body.len(), 50_000);
    assert!(started.elapsed() >= Duration::from_millis(400));
}

#[test]
fn fails_first_requests_then_serves() {
    let server = TestServer::http();
    server.put(
        "/f",
        FileSpec::new(vec![7u8; 10]).fail_first(vec![
            Failure {
                status: 429,
                retry_after: Some(2),
            },
            Failure {
                status: 503,
                retry_after: None,
            },
        ]),
    );
    let first = get(&server, "/f", &[]);
    assert_eq!(first.status, 429);
    assert_eq!(first.header("retry-after"), Some("2"));
    assert_eq!(get(&server, "/f", &[]).status, 503);
    assert_eq!(get(&server, "/f", &[]).status, 200);
}

#[test]
fn forbids_after_some_requests() {
    let server = TestServer::http();
    server.put("/f", FileSpec::new(vec![7u8; 10]).forbid_after(2));
    assert_eq!(get(&server, "/f", &[]).status, 200);
    assert_eq!(get(&server, "/f", &[]).status, 200);
    assert_eq!(get(&server, "/f", &[]).status, 403);
}

#[test]
fn redirects_and_sends_content_disposition() {
    let server = TestServer::http();
    server.redirect("/a", "/b", 302);
    server.put(
        "/b",
        FileSpec::new(vec![1u8; 4]).content_disposition("attachment; filename=\"../../x\""),
    );
    let redirect = get(&server, "/a", &[]);
    assert_eq!(redirect.status, 302);
    assert_eq!(redirect.header("location"), Some("/b"));
    let reply = get(&server, "/b", &[]);
    assert_eq!(
        reply.header("content-disposition"),
        Some("attachment; filename=\"../../x\"")
    );
    assert_eq!(get(&server, "/missing", &[]).status, 404);
}

#[test]
fn can_send_without_length() {
    let server = TestServer::http();
    server.put("/f", FileSpec::new(vec![1u8; 10]).without_length());
    let reply = get(&server, "/f", &[]);
    assert_eq!(reply.header("content-length"), None);
    assert_eq!(reply.header("transfer-encoding"), Some("chunked"));
}

#[test]
fn https_server_starts_and_reports_its_url() {
    let server = TestServer::https_self_signed(&["localhost"], "localhost");
    assert!(server.url("/f").starts_with("https://localhost:"));
}

#[test]
fn last_modified_and_date_are_set_by_the_test() {
    let server = TestServer::http();
    server.put(
        "/f",
        FileSpec::new(vec![1u8; 4])
            .etag(None)
            .last_modified(Some("Thu, 22 Oct 2026 10:00:00 GMT"))
            .date("Thu, 22 Oct 2026 10:00:00 GMT"),
    );
    let reply = get(&server, "/f", &[]);
    assert_eq!(reply.header("etag"), None);
    assert_eq!(
        reply.header("last-modified"),
        Some("Thu, 22 Oct 2026 10:00:00 GMT")
    );
    assert_eq!(reply.header("date"), Some("Thu, 22 Oct 2026 10:00:00 GMT"));
    // Exactly one Date header: hyper does not add its own.
    assert_eq!(
        reply
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("date"))
            .count(),
        1
    );

    server.put("/g", FileSpec::new(vec![1u8; 4]).last_modified(None));
    let reply = get(&server, "/g", &[]);
    assert_eq!(reply.header("last-modified"), None);
    assert!(
        reply.header("date").is_some(),
        "the current time by default"
    );
}
