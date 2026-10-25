//! `zajil batch` against the local test server (M7, ADR-0010 section 8).

mod common;

use std::fs;
use std::process::Output;

use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

fn zajil(args: &[&str]) -> Output {
    common::zajil_command()
        .args(args)
        .output()
        .expect("run zajil")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// ROADMAP M7: `img[001-100].jpg` makes exactly 100 URLs.
#[test]
fn print_shows_exactly_the_100_urls_of_img_001_to_100() {
    let out = zajil(&["batch", "https://example.com/img[001-100].jpg", "--print"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 100);
    assert_eq!(lines[0], "https://example.com/img001.jpg");
    assert_eq!(lines[99], "https://example.com/img100.jpg");
}

/// Owner's decision on ADR-0010: `*` with `--from`, `--to` and `--width`.
#[test]
fn the_star_counter_prints_the_same_urls() {
    let star = zajil(&[
        "batch",
        "https://example.com/img*.jpg",
        "--from",
        "1",
        "--to",
        "100",
        "--width",
        "3",
        "--print",
    ]);
    let brackets = zajil(&["batch", "https://example.com/img[001-100].jpg", "--print"]);
    assert!(star.status.success(), "{}", text(&star.stderr));
    assert_eq!(text(&star.stdout), text(&brackets.stdout));
}

/// A wrong pattern: exit code 2 (wrong usage), a message that says what
/// and where, and nothing printed or downloaded.
#[test]
fn a_wrong_pattern_is_refused_with_a_clear_message() {
    for (pattern, message) in [
        (
            "https://example.com/img[001-100.jpg",
            "character 24: `[` has no closing `]`",
        ),
        (
            "https://example.com/img[01-100].jpg",
            "the start 01 starts with a zero, so every number has 2 digits, but the end 100 has 3",
        ),
        (
            "https://example.com/img[1-20000].jpg",
            "the pattern makes 20000 URLs, more than the limit of 1000",
        ),
        ("https://example.com/img*.jpg", "`*` needs --from and --to"),
    ] {
        let out = zajil(&["batch", pattern, "--print"]);
        assert_eq!(out.status.code(), Some(2), "{pattern}");
        assert!(
            text(&out.stderr).contains(message),
            "{pattern}: {}",
            text(&out.stderr)
        );
        assert!(out.stdout.is_empty(), "{pattern}");
    }
}

#[test]
fn max_raises_the_limit_up_to_the_cap() {
    let out = zajil(&[
        "batch",
        "https://e.com/[1-5000]",
        "--max",
        "5000",
        "--print",
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout).lines().count(), 5000);
    let out = zajil(&["batch", "https://e.com/[1-2]", "--max", "10001", "--print"]);
    assert_eq!(out.status.code(), Some(2));
}

/// The files are downloaded one after the other; a missing one (404) is
/// reported and the rest still come, and the exit code says one failed.
#[test]
fn a_batch_downloads_every_file_and_reports_a_missing_one() {
    let server = TestServer::http();
    let one = random_bytes(200_000, 1);
    let three = random_bytes(300_000, 3);
    server.put("/part1.bin", FileSpec::new(one.clone()));
    server.put("/part3.bin", FileSpec::new(three.clone()));
    let dir = tempfile::tempdir().unwrap();

    let pattern = server.url("/part[1-3].bin");
    let out = zajil(&["batch", &pattern, "--dir", dir.path().to_str().unwrap()]);
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert_eq!(
        sha256_hex(&fs::read(dir.path().join("part1.bin")).unwrap()),
        sha256_hex(&one)
    );
    assert_eq!(
        sha256_hex(&fs::read(dir.path().join("part3.bin")).unwrap()),
        sha256_hex(&three)
    );
    assert!(!dir.path().join("part2.bin").exists());
    assert!(
        stderr.contains("[2/3] failed: the server answered 404"),
        "{stderr}"
    );
    assert!(stderr.contains("2 saved, 1 failed, of 3"), "{stderr}");
    // Each file was asked for in order.
    let gets: Vec<String> = server
        .requests()
        .into_iter()
        .filter(|r| r.method == "GET")
        .map(|r| r.path)
        .collect();
    let order: Vec<usize> = ["part1", "part2", "part3"]
        .iter()
        .map(|name| gets.iter().position(|p| p.contains(name)).unwrap())
        .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{gets:?}");
}

#[test]
fn a_batch_where_every_file_arrives_exits_zero() {
    let server = TestServer::http();
    server.put("/a.txt", FileSpec::new(b"aaa".to_vec()));
    server.put("/b.txt", FileSpec::new(b"bbb".to_vec()));
    let dir = tempfile::tempdir().unwrap();
    let out = zajil(&[
        "batch",
        &server.url("/{a,b}.txt"),
        "--dir",
        dir.path().to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(fs::read(dir.path().join("a.txt")).unwrap(), b"aaa");
    assert_eq!(fs::read(dir.path().join("b.txt")).unwrap(), b"bbb");
}

/// A token in the query is never printed while downloading (owner's
/// decision on #28).
#[test]
fn the_progress_lines_never_show_the_query() {
    let server = TestServer::http();
    server.put("/f1.bin", FileSpec::new(b"x".to_vec()));
    let dir = tempfile::tempdir().unwrap();
    let pattern = format!("{}?sig=SECRET{{1,2}}", server.url("/f1.bin"));
    let out = zajil(&["batch", &pattern, "--dir", dir.path().to_str().unwrap()]);
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(!all.contains("SECRET"), "{all}");
}

/// `--print` keeps the query the user wrote, never a password
/// (AGENTS.md §10.3).
#[test]
fn print_keeps_the_query_but_never_a_password() {
    let out = zajil(&[
        "batch",
        "https://user:secretpw@example.com/get?id=[1-2]",
        "--print",
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        "https://example.com/get?id=1\nhttps://example.com/get?id=2\n"
    );
}

/// Review of #113: the host can never be reached by a pattern.
#[test]
fn a_pattern_in_the_host_is_refused() {
    for pattern in [
        "http:///[1-3].example.com/f",
        "http://\t/[1-3].example.com/f",
    ] {
        let out = zajil(&["batch", pattern, "--print"]);
        assert_eq!(out.status.code(), Some(2), "{pattern:?}");
        assert!(out.stdout.is_empty(), "{pattern:?}");
    }
}
