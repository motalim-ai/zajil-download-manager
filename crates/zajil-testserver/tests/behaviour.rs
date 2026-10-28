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
    // Under this server's own prefix (#50).
    let path = server.path(path);
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
fn answers_late() {
    let server = TestServer::http();
    server.put(
        "/f",
        FileSpec::new(vec![7u8; 10]).answer_after(Duration::from_millis(500)),
    );
    server.put("/g", FileSpec::new(vec![8u8; 10]));
    let started = Instant::now();
    let reply = get(&server, "/f", &[]);
    assert_eq!(reply.body, vec![7u8; 10]);
    assert!(started.elapsed() >= Duration::from_millis(500));
    // Another file answers at once.
    let started = Instant::now();
    assert_eq!(get(&server, "/g", &[]).body, vec![8u8; 10]);
    assert!(started.elapsed() < Duration::from_millis(500));
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
    // A path on this server, under its own prefix (#50).
    assert_eq!(
        redirect.header("location"),
        Some(server.path("/b").as_str())
    );
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

#[test]
fn can_announce_ranges_and_still_ignore_them() {
    let server = TestServer::http();
    let data = random_bytes(1000, 20);
    server.put("/f", FileSpec::new(data.clone()).ignore_ranges());
    let reply = get(&server, "/f", &[("Range", "bytes=10-19")]);
    assert_eq!(reply.status, 200);
    assert_eq!(reply.header("accept-ranges"), Some("bytes"));
    assert_eq!(reply.body, data);
}

#[test]
fn can_announce_a_false_length() {
    let server = TestServer::http();
    server.put(
        "/f",
        FileSpec::new(random_bytes(100, 21)).announce_length(1 << 40),
    );
    let reply = request(&server, "HEAD", "/f", &[]);
    assert_eq!(reply.header("content-length"), Some("1099511627776"));
}

#[test]
fn replaces_the_file_after_some_requests() {
    let server = TestServer::http();
    let (old, new) = (random_bytes(100, 22), random_bytes(100, 23));
    server.put(
        "/f",
        FileSpec::new(old.clone()).replaced_after(2, FileSpec::new(new.clone())),
    );
    let first = get(&server, "/f", &[]);
    let second = get(&server, "/f", &[]);
    let third = get(&server, "/f", &[]);
    assert_eq!(first.body, old);
    assert_eq!(second.body, old);
    assert_eq!(third.body, new);
    assert_ne!(third.header("etag"), first.header("etag"));
}

#[test]
fn counts_bodies_sent_at_the_same_time_and_connections() {
    let server = TestServer::http();
    // Slow enough that three requests overlap.
    server.put(
        "/f",
        FileSpec::new(random_bytes(200_000, 24)).throttle(400_000),
    );
    let threads: Vec<_> = (0..3)
        .map(|_| {
            let addr = server.addr();
            let line = format!(
                "GET {} HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n",
                server.path("/f")
            );
            std::thread::spawn(move || {
                let mut stream = TcpStream::connect(addr).unwrap();
                stream.write_all(line.as_bytes()).unwrap();
                let mut sink = Vec::new();
                let _ = stream.read_to_end(&mut sink);
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(server.most_concurrent("/f"), 3);
    let gets = server.gets("/f");
    let mut concurrent: Vec<_> = gets.iter().map(|r| r.concurrent).collect();
    concurrent.sort();
    assert_eq!(concurrent, [1, 2, 3]);
    let mut connections: Vec<_> = gets.iter().map(|r| r.connection).collect();
    connections.sort();
    connections.dedup();
    assert_eq!(connections.len(), 3);

    // Two requests on one kept-alive connection share its number, and a
    // finished body no longer counts.
    let mut stream = TcpStream::connect(server.addr()).unwrap();
    let head = format!("HEAD {} HTTP/1.1\r\nHost: t\r\n\r\n", server.path("/f"));
    for _ in 0..2 {
        stream.write_all(head.as_bytes()).unwrap();
        let mut head = [0u8; 1024];
        let _ = stream.read(&mut head).unwrap();
    }
    let heads: Vec<_> = server
        .requests()
        .into_iter()
        .filter(|r| r.method == "HEAD")
        .collect();
    assert_eq!(heads.len(), 2);
    assert_eq!(heads[0].connection, heads[1].connection);
    assert!(heads.iter().all(|r| r.concurrent == 0));
}

#[test]
fn slows_only_the_response_that_starts_at_the_given_byte() {
    let server = TestServer::http();
    server.put(
        "/f",
        FileSpec::new(random_bytes(200_000, 25)).slow_at(100_000, 100_000),
    );
    let started = Instant::now();
    let fast = get(&server, "/f", &[("Range", "bytes=0-99999")]);
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(fast.body.len(), 100_000);
    let started = Instant::now();
    let slow = get(&server, "/f", &[("Range", "bytes=100000-199999")]);
    assert!(started.elapsed() >= Duration::from_millis(800));
    assert_eq!(slow.body.len(), 100_000);
}

#[test]
fn refuses_requests_beyond_a_number_of_bodies_at_once() {
    for (spec, status, retry_after) in [
        (
            FileSpec::new(random_bytes(100_000, 26)).max_concurrent(1),
            403,
            None,
        ),
        (
            FileSpec::new(random_bytes(100_000, 26)).too_many_beyond(1, 2),
            429,
            Some("2"),
        ),
    ] {
        let server = TestServer::http();
        server.put("/f", spec.throttle(100_000));
        // One body is being sent (slowly) while a second request arrives.
        let addr = server.addr();
        let line = format!(
            "GET {} HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n",
            server.path("/f")
        );
        let first = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream.write_all(line.as_bytes()).unwrap();
            let mut sink = Vec::new();
            let _ = stream.read_to_end(&mut sink);
        });
        std::thread::sleep(Duration::from_millis(200));
        let second = get(&server, "/f", &[]);
        assert_eq!(second.status, status);
        assert_eq!(second.header("retry-after"), retry_after);
        first.join().unwrap();
        // Once nothing is being sent, requests are served again.
        assert_eq!(get(&server, "/f", &[]).status, 200);
    }
}

#[test]
fn can_fail_only_range_requests_and_records_when_requests_arrived() {
    let server = TestServer::http();
    server.put(
        "/f",
        FileSpec::new(random_bytes(1000, 27)).fail_range_requests(Failure {
            status: 503,
            retry_after: None,
        }),
    );
    assert_eq!(get(&server, "/f", &[("Range", "bytes=0-9")]).status, 503);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(get(&server, "/f", &[]).status, 200);
    let gets = server.gets("/f");
    assert!(gets[1].at >= gets[0].at + Duration::from_millis(50));
}

/// #50: a request that is not this server's own, such as one from a
/// process left by another test that connects to this port number, is
/// answered 404 and counted nowhere: it cannot take the cut, or any other
/// place, of the test's own requests. Without the prefix, the stray request
/// took the cut and the test's first request came whole.
#[test]
fn a_stray_request_is_refused_and_takes_no_place_of_the_tests_own() {
    let server = TestServer::http();
    let data = random_bytes(1_000_000, 50);
    server.put("/f.bin", FileSpec::new(data.clone()).cut_after(300_000, 1));

    // The stray asks for the same path, without this server's prefix.
    let mut stream = TcpStream::connect(server.addr()).unwrap();
    stream
        .write_all(b"GET /f.bin HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut stray = Vec::new();
    let _ = stream.read_to_end(&mut stray);
    let status = String::from_utf8_lossy(&stray[..stray.len().min(40)]).into_owned();
    assert!(status.starts_with("HTTP/1.1 404"), "{status}");

    // The test's own first request is still the one that is cut.
    let first = get(&server, "/f.bin", &[]);
    assert!(first.incomplete);
    let second = get(&server, "/f.bin", &[]);
    assert_eq!(second.body, data);
    assert_eq!(server.gets("/f.bin").len(), 2);
    assert_eq!(server.strays(), ["GET /f.bin"]);
}

/// #50: what a failed test is shown: every request in order with its range,
/// status and bytes; every connection, who opened it and how it ended (a
/// cut says so, with its cause); and the stray requests.
#[test]
fn the_report_of_a_failed_test_holds_requests_connections_and_strays() {
    let server = TestServer::http();
    server.put(
        "/f.bin",
        FileSpec::new(random_bytes(500_000, 51)).cut_after(100_000, 1),
    );
    let cut = get(&server, "/f.bin", &[]);
    assert!(cut.incomplete);
    let whole = get(&server, "/f.bin", &[("Range", "bytes=1000-")]);
    assert_eq!(whole.status, 206);
    let mut stream = TcpStream::connect(server.addr()).unwrap();
    stream
        .write_all(b"GET /other HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n")
        .unwrap();
    let _ = stream.read_to_end(&mut Vec::new());
    // The server writes down a connection's end just after closing it:
    // wait for that, with a deadline (review of #52).
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let report = loop {
        let report = server.report();
        if report.contains("cut by test server") || std::time::Instant::now() > deadline {
            break report;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        report.contains("request 1: GET /f.bin range=None"),
        "{report}"
    );
    assert!(report.contains("bytes_written=100000 conn=1"), "{report}");
    assert!(
        report.contains("request 2: GET /f.bin range=Some(\"bytes=1000-\")"),
        "{report}"
    );
    assert!(report.contains("status=206"), "{report}");
    assert!(report.contains("connection 1 from 127.0.0.1:"), "{report}");
    assert!(report.contains("cut by test server"), "{report}");
    assert!(report.contains("connection 2 from 127.0.0.1:"), "{report}");
    assert!(
        report.contains("stray request (404, not counted): GET /other"),
        "{report}"
    );
}

/// The report is printed while the test is already panicking: that must
/// not panic again (which would abort the whole test binary).
#[test]
fn a_test_that_fails_with_a_server_alive_still_ends_cleanly() {
    let failed = std::panic::catch_unwind(|| {
        let server = TestServer::http();
        server.put("/f", FileSpec::new(vec![1u8; 10]));
        let _ = get(&server, "/f", &[]);
        panic!("a test fails while its server is alive");
    });
    assert!(failed.is_err());
}

/// Run only by `a_failed_test_shows_the_servers_report` below, in a process
/// of its own: a test that fails with its server alive.
#[test]
#[ignore = "fails on purpose; run by a_failed_test_shows_the_servers_report"]
fn fails_on_purpose_with_a_server_alive() {
    let server = TestServer::http();
    server.put("/f", FileSpec::new(vec![1u8; 10]));
    let _ = get(&server, "/f", &[]);
    panic!("failing on purpose");
}

/// #50 and review of #52: a test that fails with a server alive shows the
/// server's report under it, in the test harness's own output.
#[test]
fn a_failed_test_shows_the_servers_report() {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "fails_on_purpose_with_a_server_alive",
            "--exact",
            "--ignored",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("failing on purpose"), "{text}");
    assert!(text.contains("test server http://127.0.0.1:"), "{text}");
    assert!(text.contains("saw:"), "{text}");
    assert!(text.contains("request 1: GET /f range=None"), "{text}");
    assert!(text.contains("connection 1 from 127.0.0.1:"), "{text}");
}
