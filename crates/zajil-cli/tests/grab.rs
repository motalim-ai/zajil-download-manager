//! `zajil grab` against the local test server (M7, ADR-0010 section 9):
//! what the command itself adds to the crawler (`zajil-grab`'s own tests
//! cover the crawl).

mod common;

use std::fs;
use std::path::Path;
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

/// A page of `<a>` links to `hrefs`.
fn links(hrefs: &[&str]) -> FileSpec {
    let body: String = hrefs
        .iter()
        .map(|href| format!("<a href=\"{href}\">link</a>\n"))
        .collect();
    FileSpec::new(body.into_bytes()).content_type("text/html; charset=utf-8")
}

/// A server that answers `/robots.txt` at its root (404 until a test puts
/// one there).
fn server() -> TestServer {
    let server = TestServer::http();
    server.robots_txt_at_root();
    server
}

/// `zajil grab <start> --rate 10 --dir <dir>` and `more`.
fn grab(start: &str, dir: &Path, more: &[&str]) -> Output {
    let mut args = vec![
        "grab",
        start,
        "--rate",
        "10",
        "--dir",
        dir.to_str().unwrap(),
    ];
    args.extend_from_slice(more);
    zajil(&args)
}

/// `--dry-run` prints each file's URL on stdout, without its query, and
/// downloads nothing.
#[test]
fn dry_run_prints_the_files_without_their_query_and_downloads_none() {
    let site = server();
    site.put(
        "/index.html",
        links(&["a.zip?token=SECRET-TOKEN", "b.pdf", "next.html"]),
    );
    site.put("/next.html", links(&["c.mp4"]));
    for (path, seed) in [("/a.zip", 1), ("/b.pdf", 2), ("/c.mp4", 3)] {
        site.put(path, FileSpec::new(random_bytes(1000, seed)));
    }
    let dir = tempfile::tempdir().unwrap();
    let out = grab(&site.url("/index.html"), dir.path(), &["--dry-run"]);

    assert!(out.status.success(), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        [site.url("/a.zip"), site.url("/b.pdf"), site.url("/c.mp4")]
    );
    assert!(!format!("{stdout}{}", text(&out.stderr)).contains("SECRET"));
    assert!(text(&out.stderr).contains("3 files would be downloaded"));
    for path in ["/a.zip", "/b.pdf", "/c.mp4"] {
        assert!(site.gets(path).is_empty(), "{path} was asked for");
    }
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

/// A real small grab saves the site's files into `--dir`, whole.
#[test]
fn a_grab_saves_the_files_into_dir() {
    let site = server();
    let a = random_bytes(3000, 4);
    let b = random_bytes(5000, 5);
    site.put("/index.html", links(&["a.zip", "docs/b.pdf", "pic.png"]));
    site.put("/a.zip", FileSpec::new(a.clone()));
    site.put("/docs/b.pdf", FileSpec::new(b.clone()));
    site.put("/pic.png", FileSpec::new(random_bytes(100, 6)));
    let dir = tempfile::tempdir().unwrap();
    let out = grab(&site.url("/index.html"), dir.path(), &[]);

    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        sha256_hex(&fs::read(dir.path().join("a.zip")).unwrap()),
        sha256_hex(&a)
    );
    assert_eq!(
        sha256_hex(&fs::read(dir.path().join("b.pdf")).unwrap()),
        sha256_hex(&b)
    );
    // Images are not among the default types.
    assert!(site.gets("/pic.png").is_empty());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    assert!(
        text(&out.stderr).contains("2 files saved"),
        "{}",
        text(&out.stderr)
    );
}

/// `--ignore-robots` prints its warning and asks for what robots.txt
/// disallows; without it, the same site gives nothing.
#[test]
fn ignore_robots_prints_its_warning() {
    let site = server();
    site.put(
        "/robots.txt",
        FileSpec::new(b"User-agent: *\nDisallow: /\n".to_vec()),
    );
    site.put("/index.html", links(&["a.zip"]));
    site.put("/a.zip", FileSpec::new(random_bytes(1000, 7)));

    let dir = tempfile::tempdir().unwrap();
    let out = grab(&site.url("/index.html"), dir.path(), &[]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(!text(&out.stderr).contains("warning"));
    assert!(site.gets("/index.html").is_empty());

    let out = grab(&site.url("/index.html"), dir.path(), &["--ignore-robots"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("zajil: warning: robots.txt is ignored"),
        "{}",
        text(&out.stderr)
    );
    assert!(dir.path().join("a.zip").exists());
    assert_eq!(
        site.gets("/robots.txt").len(),
        1,
        "read by the first run only"
    );
}

/// Wrong usage: exit code 2, a message that says what is wrong, nothing on
/// stdout, and nothing asked of any server.
#[test]
fn usage_errors_exit_2_with_a_clear_message() {
    for (args, message) in [
        (
            &["grab", "ftp://example.com/files/"][..],
            "the start must be an http or https URL",
        ),
        (
            &["grab", "not a url"][..],
            "the start must be an http or https URL",
        ),
        (
            &["grab", "http://user:pass@example.com/"][..],
            "a crawl never sends a user name or password",
        ),
        (
            &["grab", "https://example.com/", "--rate", "0"][..],
            "--rate must be a number from 0.1 to 10",
        ),
        (
            &["grab", "https://example.com/", "--rate", "fast"][..],
            "--rate must be a number from 0.1 to 10",
        ),
        (
            &["grab", "https://example.com/", "--max-total", "lots"][..],
            "\"lots\" is not a size",
        ),
    ] {
        let out = zajil(args);
        let stderr = text(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {stderr}");
        assert!(stderr.contains(message), "{args:?}: {stderr}");
        assert!(!stderr.contains("user:pass"), "{args:?}: {stderr}");
        assert!(out.stdout.is_empty(), "{args:?}");
    }
}

/// Owner's decision on ADR-0010: `zajil grab` downloads each file with one
/// connection, even one the server lets split, unless `--connections` is
/// given.
#[test]
fn grab_uses_one_connection_per_file_unless_asked() {
    let data = random_bytes(4 * 1024 * 1024, 8);
    let most = |more: &[&str]| {
        let site = server();
        site.put("/index.html", links(&["big.zip"]));
        site.put(
            "/big.zip",
            FileSpec::new(data.clone()).throttle(4 * 1024 * 1024),
        );
        let dir = tempfile::tempdir().unwrap();
        let out = grab(&site.url("/index.html"), dir.path(), more);
        assert!(out.status.success(), "{}", text(&out.stderr));
        assert_eq!(
            sha256_hex(&fs::read(dir.path().join("big.zip")).unwrap()),
            sha256_hex(&data)
        );
        site.most_concurrent("/big.zip")
    };
    assert_eq!(most(&[]), 1);
    let asked = most(&["--connections", "4"]);
    assert!(asked > 1, "{asked} connections with --connections 4");
}
