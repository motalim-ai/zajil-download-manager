//! The site grabber against the local test server (M7, ADR-0010 section
//! 10): depth, robots.txt, the site's limits, loops, big counts, cookies,
//! time and HTTPS. Two servers on 127.0.0.1 with different ports are two
//! origins; the second is "public" only where a test says so
//! (`Scope::treat_as_public`).

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use zajil_core::limit::ManualClock;
use zajil_core::{Engine, Options, Stop, Url};
use zajil_grab::crawl::{self, Event, Limit, Plan, Report, RobotsOutcome, Skip};
use zajil_grab::links::MAX_LINKS_PER_PAGE;
use zajil_grab::scope::Scope;
use zajil_testserver::{Failure, FileSpec, TestServer, Tls, random_bytes, sha256_hex, test_ca_der};

fn html(body: &str) -> FileSpec {
    FileSpec::new(body.as_bytes().to_vec()).content_type("text/html; charset=utf-8")
}

/// A page of `<a>` links to `hrefs`.
fn links(hrefs: &[&str]) -> FileSpec {
    html(
        &hrefs
            .iter()
            .map(|href| format!("<a href=\"{href}\">link</a>\n"))
            .collect::<String>(),
    )
}

/// A server that answers `/robots.txt` at its root (404 until a test puts
/// one there).
fn server() -> TestServer {
    let server = TestServer::http();
    server.robots_txt_at_root();
    server
}

fn engine() -> Engine {
    Engine::new(Options::default()).unwrap()
}

/// An engine on a clock that only the test moves.
fn engine_on_frozen_clock() -> (Engine, Arc<ManualClock>) {
    let clock = ManualClock::frozen(jiff::Timestamp::UNIX_EPOCH);
    let engine = Engine::new(Options {
        clock: clock.clone(),
        ..Options::default()
    })
    .unwrap();
    (engine, clock)
}

/// The defaults, at the fastest rate so that tests on the real clock are
/// quick.
fn plan(start: &str, dir: &Path) -> Plan {
    let mut plan = Plan::new(Url::parse(start).unwrap(), dir.to_path_buf());
    plan.rate = crawl::MAX_RATE;
    plan
}

fn scope(plan: &Plan) -> Scope {
    Scope::new(&plan.start, plan.allowed.clone(), plan.files_anywhere).unwrap()
}

async fn grab(engine: &Engine, plan: &Plan, scope: &Scope) -> (Report, Vec<Event>) {
    let mut events = Vec::new();
    let report = crawl::crawl(engine, plan, scope, &Stop::new(), &mut |event| {
        events.push(event)
    })
    .await
    .unwrap();
    (report, events)
}

fn spawn_grab(
    engine: Engine,
    plan: Plan,
    scope: Scope,
) -> tokio::task::JoinHandle<(Report, Vec<Event>)> {
    tokio::spawn(async move { grab(&engine, &plan, &scope).await })
}

/// The names of the files saved, in order.
fn saved(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Saved { file, .. } => {
                Some(file.file_name().unwrap().to_string_lossy().into_owned())
            }
            _ => None,
        })
        .collect()
}

/// The URLs skipped for `why`.
fn skipped(events: &[Event], why: Skip) -> Vec<Url> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Skipped { url, why: w } if *w == why => Some(url.clone()),
            _ => None,
        })
        .collect()
}

/// The URLs reported as files found.
fn found(events: &[Event]) -> Vec<Url> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::File { url } => Some(url.clone()),
            _ => None,
        })
        .collect()
}

fn robots_outcomes(events: &[Event]) -> Vec<(String, RobotsOutcome)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Robots { origin, outcome } => Some((origin.clone(), outcome.clone())),
            _ => None,
        })
        .collect()
}

fn origin_of(server: &TestServer) -> String {
    Url::parse(&server.url("/"))
        .unwrap()
        .origin()
        .ascii_serialization()
}

/// No path was asked for twice.
fn each_asked_at_most_once(server: &TestServer) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for request in server.requests() {
        *counts.entry(request.path).or_default() += 1;
    }
    let twice: Vec<_> = counts.iter().filter(|(_, n)| **n > 1).collect();
    assert!(twice.is_empty(), "asked for more than once: {twice:?}");
}

fn asked_paths(server: &TestServer) -> BTreeSet<String> {
    server.requests().into_iter().map(|r| r.path).collect()
}

/// Waits (on the real clock) until `ready`, at most 20 s.
async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Real time for a request that should not come to have come.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(250)).await;
}

/// ADR-0010 section 5: a 3-level site with depth 2 downloads the files of
/// the start page and of level 1 only, and the server's log shows that the
/// pages of levels 2 and 3 were never asked for.
#[tokio::test(flavor = "multi_thread")]
async fn depth_two_downloads_the_files_of_levels_0_and_1_only() {
    let site = server();
    let mut data = Vec::new();
    for level in 0..4u64 {
        let bytes = random_bytes(2_000 + level as usize, level);
        site.put(&format!("/f{level}.zip"), FileSpec::new(bytes.clone()));
        data.push(bytes);
        let next = format!("l{}.html", level + 1);
        let file = format!("f{level}.zip");
        let hrefs: Vec<&str> = if level < 3 {
            vec![&next, &file]
        } else {
            vec![&file]
        };
        site.put(&format!("/l{level}.html"), links(&hrefs));
    }
    let dir = tempfile::tempdir().unwrap();
    let plan = plan(&site.url("/l0.html"), dir.path());
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    assert_eq!(saved(&events), ["f0.zip", "f1.zip"]);
    for (level, bytes) in data.iter().enumerate().take(2) {
        let file = dir.path().join(format!("f{level}.zip"));
        assert_eq!(sha256_hex(&std::fs::read(file).unwrap()), sha256_hex(bytes));
    }
    assert_eq!((report.pages, report.saved, report.failed), (2, 2, 0));
    for path in ["/l2.html", "/l3.html", "/f2.zip", "/f3.zip"] {
        assert!(site.gets(path).is_empty(), "{path} was asked for");
    }
    each_asked_at_most_once(&site);
    assert!(site.strays().is_empty(), "{:?}", site.strays());
}

/// robots.txt that disallows a path: neither the page nor the file there
/// is ever asked for, and the report says why.
#[tokio::test(flavor = "multi_thread")]
async fn a_path_robots_txt_disallows_is_never_asked_for() {
    let site = server();
    site.put(
        "/robots.txt",
        FileSpec::new(format!(
            "User-agent: *\nDisallow: {}\n",
            site.path("/private/")
        ))
        .content_type("text/plain"),
    );
    site.put(
        "/index.html",
        links(&["private/a.zip", "private/page.html", "public.zip"]),
    );
    site.put("/private/a.zip", FileSpec::new(random_bytes(1000, 1)));
    site.put("/private/page.html", links(&["../b.zip"]));
    site.put("/public.zip", FileSpec::new(random_bytes(1000, 2)));
    let dir = tempfile::tempdir().unwrap();
    let plan = plan(&site.url("/index.html"), dir.path());
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    assert_eq!(saved(&events), ["public.zip"]);
    assert!(site.gets("/private/a.zip").is_empty());
    assert!(site.gets("/private/page.html").is_empty());
    assert_eq!(skipped(&events, Skip::Robots).len(), 2);
    assert_eq!(report.skipped.get(&Skip::Robots), Some(&2));
    assert_eq!(
        robots_outcomes(&events),
        [(origin_of(&site), RobotsOutcome::Rules)]
    );
    assert_eq!(site.gets("/robots.txt").len(), 1, "robots.txt once");
}

/// RFC 9309 §2.3.1.3: no robots.txt (404) allows everything.
#[tokio::test(flavor = "multi_thread")]
async fn robots_txt_404_allows_everything() {
    let site = server();
    site.put("/index.html", links(&["a.zip"]));
    site.put("/a.zip", FileSpec::new(random_bytes(1000, 3)));
    let dir = tempfile::tempdir().unwrap();
    let plan = plan(&site.url("/index.html"), dir.path());
    let (_, events) = grab(&engine(), &plan, &scope(&plan)).await;

    assert_eq!(site.gets("/robots.txt")[0].status, 404);
    assert_eq!(
        robots_outcomes(&events),
        [(origin_of(&site), RobotsOutcome::AllowAll)]
    );
    assert_eq!(saved(&events), ["a.zip"]);
}

/// RFC 9309 §2.3.1.4 (and 429 out of courtesy): robots.txt that answers
/// 500 or 429 disallows everything, even the start page.
#[tokio::test(flavor = "multi_thread")]
async fn robots_txt_500_or_429_disallows_everything() {
    for status in [500, 503, 429] {
        let site = server();
        site.put(
            "/robots.txt",
            FileSpec::new(Vec::new()).fail_first(vec![Failure {
                status,
                retry_after: None,
            }]),
        );
        site.put("/index.html", links(&["a.zip"]));
        site.put("/a.zip", FileSpec::new(random_bytes(1000, 4)));
        let dir = tempfile::tempdir().unwrap();
        let plan = plan(&site.url("/index.html"), dir.path());
        let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

        let outcomes = robots_outcomes(&events);
        assert!(
            matches!(&outcomes[..], [(_, RobotsOutcome::DisallowAll(why))] if why.contains(&status.to_string())),
            "{status}: {outcomes:?}"
        );
        assert_eq!(
            asked_paths(&site),
            BTreeSet::from(["/robots.txt".to_owned()]),
            "{status}"
        );
        assert_eq!(report.pages, 0, "{status}");
        assert_eq!(report.skipped.get(&Skip::Robots), Some(&1), "{status}");
    }
}

/// A redirect of robots.txt is followed, on the same origin and to another
/// one (a public one), and the rules found at its end apply.
#[tokio::test(flavor = "multi_thread")]
async fn a_robots_txt_redirect_is_followed() {
    let rules = |site: &TestServer| {
        FileSpec::new(format!(
            "User-agent: *\nDisallow: {}\n",
            site.path("/private/")
        ))
        .content_type("text/plain")
    };
    // On the same origin.
    let site = server();
    site.redirect("/robots.txt", "/rules/robots.txt", 301);
    site.put("/rules/robots.txt", rules(&site));
    site.put("/index.html", links(&["private/a.zip", "b.zip"]));
    site.put("/private/a.zip", FileSpec::new(random_bytes(1000, 5)));
    site.put("/b.zip", FileSpec::new(random_bytes(1000, 6)));
    let dir = tempfile::tempdir().unwrap();
    let plan_a = plan(&site.url("/index.html"), dir.path());
    let (_, events) = grab(&engine(), &plan_a, &scope(&plan_a)).await;
    assert_eq!(site.gets("/rules/robots.txt").len(), 1);
    assert!(site.gets("/private/a.zip").is_empty());
    assert_eq!(saved(&events), ["b.zip"]);

    // To another origin, treated as public.
    let site = server();
    let rules_host = server();
    site.redirect("/robots.txt", &rules_host.url("/robots-of-a.txt"), 302);
    rules_host.put("/robots-of-a.txt", rules(&site));
    site.put("/index.html", links(&["private/a.zip", "b.zip"]));
    site.put("/private/a.zip", FileSpec::new(random_bytes(1000, 7)));
    site.put("/b.zip", FileSpec::new(random_bytes(1000, 8)));
    let dir = tempfile::tempdir().unwrap();
    let plan_b = plan(&site.url("/index.html"), dir.path());
    let scope_b = scope(&plan_b).treat_as_public(&Url::parse(&rules_host.url("/")).unwrap());
    let (_, events) = grab(&engine(), &plan_b, &scope_b).await;
    assert_eq!(
        asked_paths(&rules_host),
        BTreeSet::from(["/robots-of-a.txt".to_owned()])
    );
    assert!(site.gets("/private/a.zip").is_empty());
    assert_eq!(saved(&events), ["b.zip"]);
}

/// ADR-0010 section 11: a redirect of robots.txt to a local address that
/// is not the site is refused, which disallows everything; the local
/// server gets nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_robots_txt_redirect_to_a_local_address_is_refused() {
    let site = server();
    let local = server();
    site.redirect("/robots.txt", &local.url("/robots.txt"), 302);
    local.put(
        "/robots.txt",
        FileSpec::new(b"User-agent: *\nAllow: /\n".to_vec()),
    );
    site.put("/index.html", links(&["a.zip"]));
    let dir = tempfile::tempdir().unwrap();
    let plan = plan(&site.url("/index.html"), dir.path());
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    let outcomes = robots_outcomes(&events);
    assert!(
        matches!(&outcomes[..], [(_, RobotsOutcome::DisallowAll(why))] if why.contains("a local address")),
        "{outcomes:?}"
    );
    assert!(local.requests().is_empty(), "{:?}", local.requests());
    assert!(local.strays().is_empty(), "{:?}", local.strays());
    assert!(site.gets("/index.html").is_empty());
    assert_eq!(report.pages, 0);
}

/// `--ignore-robots`: robots.txt is not even read, and the path it would
/// disallow is asked for.
#[tokio::test(flavor = "multi_thread")]
async fn ignore_robots_asks_for_the_disallowed_path() {
    let site = server();
    site.put(
        "/robots.txt",
        FileSpec::new(b"User-agent: *\nDisallow: /\n".to_vec()),
    );
    site.put("/index.html", links(&["private/a.zip"]));
    site.put("/private/a.zip", FileSpec::new(random_bytes(1000, 9)));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.ignore_robots = true;
    let (_, events) = grab(&engine(), &plan, &scope(&plan)).await;

    assert_eq!(saved(&events), ["a.zip"]);
    assert!(site.gets("/robots.txt").is_empty());
    assert!(robots_outcomes(&events).is_empty());
}

/// Another origin (a public one) gets no request at all without the
/// user's permission: neither its pages nor its files.
#[tokio::test(flavor = "multi_thread")]
async fn another_origin_gets_no_request_without_permission() {
    let site = server();
    let other = server();
    other.put("/page.html", links(&["c.zip"]));
    other.put("/b.zip", FileSpec::new(random_bytes(1000, 10)));
    site.put(
        "/index.html",
        links(&[&other.url("/page.html"), &other.url("/b.zip"), "a.zip"]),
    );
    site.put("/a.zip", FileSpec::new(random_bytes(1000, 11)));
    let dir = tempfile::tempdir().unwrap();
    let plan = plan(&site.url("/index.html"), dir.path());
    let scope = scope(&plan).treat_as_public(&Url::parse(&other.url("/")).unwrap());
    let (_, events) = grab(&engine(), &plan, &scope).await;

    assert_eq!(saved(&events), ["a.zip"]);
    assert!(other.requests().is_empty(), "{:?}", other.requests());
    assert!(other.strays().is_empty(), "{:?}", other.strays());
    assert_eq!(skipped(&events, Skip::OutsideSite).len(), 2);
}

/// `--allow-host 127.0.0.1:<port>`: the second server is part of the site,
/// its pages are read (with its own robots.txt) and its files downloaded.
#[tokio::test(flavor = "multi_thread")]
async fn an_allowed_host_has_its_pages_followed() {
    let site = server();
    let other = server();
    other.put("/page.html", links(&["c.zip"]));
    other.put("/c.zip", FileSpec::new(random_bytes(1000, 12)));
    site.put("/index.html", links(&[&other.url("/page.html")]));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.allowed = vec![
        format!("127.0.0.1:{}", other.addr().port())
            .parse()
            .unwrap(),
    ];
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    assert_eq!(saved(&events), ["c.zip"]);
    assert_eq!(report.pages, 2);
    assert_eq!(other.gets("/robots.txt").len(), 1);
    assert_eq!(other.gets("/page.html").len(), 1);
}

/// `--files-anywhere`: a public origin's files linked from the site are
/// downloaded (after its robots.txt), and none of its pages, nor a link
/// there that might be a page, is ever asked for.
#[tokio::test(flavor = "multi_thread")]
async fn files_anywhere_takes_only_files_from_another_origin() {
    let site = server();
    let cdn = server();
    cdn.put("/page.html", links(&["d.zip"]));
    cdn.put("/maybe", links(&["e.zip"]));
    cdn.put("/b.zip", FileSpec::new(random_bytes(1000, 13)));
    let body = format!(
        "<a href=\"{}\">p</a><a href=\"{}\">m</a><a href=\"{}\">f</a>\
         <iframe src=\"{}\"></iframe>",
        cdn.url("/page.html"),
        cdn.url("/maybe"),
        cdn.url("/b.zip"),
        cdn.url("/page.html?frame"),
    );
    site.put("/index.html", html(&body));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.files_anywhere = true;
    let scope = scope(&plan).treat_as_public(&Url::parse(&cdn.url("/")).unwrap());
    let (_, events) = grab(&engine(), &plan, &scope).await;

    assert_eq!(saved(&events), ["b.zip"]);
    assert_eq!(
        asked_paths(&cdn),
        BTreeSet::from(["/b.zip".to_owned(), "/robots.txt".to_owned()])
    );
    assert_eq!(skipped(&events, Skip::OutsideSite).len(), 3);
}

/// Owner's decision 3: a page of the site that redirects to another
/// (public) origin is not read there as a page; its target is a file only,
/// downloaded when it is a type asked for and left out otherwise.
#[tokio::test(flavor = "multi_thread")]
async fn a_page_redirect_to_another_origin_is_a_file_only() {
    let site = server();
    let cdn = server();
    site.redirect("/download/1", &cdn.url("/real.zip"), 302);
    site.redirect("/go/2", &cdn.url("/page.html"), 302);
    cdn.put("/real.zip", FileSpec::new(random_bytes(5000, 14)));
    cdn.put("/page.html", links(&["x.zip"]));
    site.put("/index.html", links(&["download/1", "go/2"]));
    let dir = tempfile::tempdir().unwrap();
    let plan = plan(&site.url("/index.html"), dir.path());
    let scope = scope(&plan).treat_as_public(&Url::parse(&cdn.url("/")).unwrap());
    let (_, events) = grab(&engine(), &plan, &scope).await;

    assert_eq!(saved(&events), ["real.zip"]);
    assert_eq!(
        asked_paths(&cdn),
        BTreeSet::from(["/real.zip".to_owned(), "/robots.txt".to_owned()])
    );
    assert_eq!(
        skipped(&events, Skip::OutsideSite),
        [Url::parse(&cdn.url("/page.html")).unwrap()]
    );
}

/// Owner's decision 3: a file's redirect from https to http is refused,
/// whether the engine follows it (a `.zip` link) or the crawler does (a
/// page's redirect); the http server gets nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_redirect_from_https_to_http_is_refused() {
    let site = TestServer::https(Tls::trusted(&["localhost"]), "localhost");
    site.robots_txt_at_root();
    let plain = server();
    site.redirect("/x.zip", &plain.url("/x.zip"), 302);
    site.redirect("/dl", &plain.url("/y.zip"), 302);
    plain.put("/x.zip", FileSpec::new(random_bytes(1000, 15)));
    plain.put("/y.zip", FileSpec::new(random_bytes(1000, 16)));
    site.put("/index.html", links(&["x.zip", "dl"]));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.files_anywhere = true;
    let scope = scope(&plan).treat_as_public(&Url::parse(&plain.url("/")).unwrap());
    let engine = Engine::with_test_root(Options::default(), &test_ca_der()).unwrap();
    let (report, events) = grab(&engine, &plan, &scope).await;

    assert!(saved(&events).is_empty());
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Failed { url, .. } if url.path().ends_with("/x.zip"))),
        "{events:?}"
    );
    assert_eq!(report.failed, 1);
    assert_eq!(
        skipped(&events, Skip::Downgrade),
        [Url::parse(&plain.url("/y.zip")).unwrap()]
    );
    assert!(plain.requests().is_empty(), "{:?}", plain.requests());
    assert!(plain.strays().is_empty());
}

/// Loops: two pages that link each other, and a page that links itself
/// with a fragment, a scheme in capitals and needless percent-encoding:
/// each page is asked for once and the crawl ends.
#[tokio::test(flavor = "multi_thread")]
async fn pages_that_link_each_other_are_asked_for_once() {
    let site = server();
    let own = site.url("/self.html");
    let shouted = own.replacen("http://", "HTTP://", 1);
    let encoded = own.replace("/self.html", "/s%65lf.html");
    site.put(
        "/index.html",
        links(&["a.html", "self.html", "a.html#again"]),
    );
    site.put("/a.html", links(&["b.html", "a.zip"]));
    site.put("/b.html", links(&["a.html", "index.html", "b.zip"]));
    site.put(
        "/self.html",
        links(&["#top", "self.html#x", &shouted, &encoded, "c.zip"]),
    );
    for name in ["a", "b", "c"] {
        site.put(
            &format!("/{name}.zip"),
            FileSpec::new(random_bytes(500, name.len() as u64)),
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.depth = crawl::MAX_DEPTH;
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    each_asked_at_most_once(&site);
    let pages = [
        "/robots.txt",
        "/index.html",
        "/a.html",
        "/b.html",
        "/self.html",
    ];
    let files = ["/a.zip", "/b.zip", "/c.zip"];
    assert_eq!(
        asked_paths(&site),
        pages
            .iter()
            .chain(&files)
            .map(|p| (*p).to_owned())
            .collect()
    );
    assert_eq!(report.pages, 4);
    let mut names = saved(&events);
    names.sort();
    assert_eq!(names, ["a.zip", "b.zip", "c.zip"]);
    assert!(site.strays().is_empty(), "{:?}", site.strays());
}

/// A redirect cycle (A → B → A) ends at once, and a chain longer than
/// `MAX_REDIRECTS` stops there; each URL is asked for once.
#[tokio::test(flavor = "multi_thread")]
async fn redirect_cycles_and_chains_end() {
    let site = server();
    site.redirect("/r1", "/r2", 302);
    site.redirect("/r2", "/r1", 302);
    let hops = usize::from(crawl::MAX_REDIRECTS) + 1;
    for n in 0..hops {
        site.redirect(&format!("/c{n}"), &format!("/c{}", n + 1), 301);
    }
    site.put("/index.html", links(&["r1", "c0"]));
    let dir = tempfile::tempdir().unwrap();
    let plan = plan(&site.url("/index.html"), dir.path());
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    each_asked_at_most_once(&site);
    assert_eq!(site.gets("/r1").len(), 1);
    assert_eq!(site.gets("/r2").len(), 1);
    assert_eq!(site.gets(&format!("/c{}", hops - 1)).len(), 1);
    assert!(site.gets(&format!("/c{hops}")).is_empty());
    assert_eq!(
        skipped(&events, Skip::TooManyRedirects),
        [Url::parse(&site.url(&format!("/c{hops}"))).unwrap()]
    );
    assert_eq!(report.pages, 1);
}

/// A path that repeats itself (`/t/t/t/t/`) is a trap and never asked for.
#[tokio::test(flavor = "multi_thread")]
async fn a_repeating_path_is_a_trap() {
    let site = server();
    site.put("/index.html", links(&["t/"]));
    for path in ["/t/", "/t/t/", "/t/t/t/", "/t/t/t/t/"] {
        site.put(path, links(&["t/"]));
    }
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.depth = crawl::MAX_DEPTH;
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    assert_eq!(report.pages, 4);
    assert!(site.gets("/t/t/t/t/").is_empty());
    assert_eq!(
        skipped(&events, Skip::PathTrap),
        [Url::parse(&site.url("/t/t/t/t/")).unwrap()]
    );
    each_asked_at_most_once(&site);
}

/// A site that makes new pages without end (a calendar) stops at the page
/// limit, and says so.
#[tokio::test(flavor = "multi_thread")]
async fn an_endless_site_stops_at_the_page_limit() {
    let site = server();
    site.put("/index.html", links(&["cal/0.html"]));
    for n in 0..40 {
        let next = format!("{}.html", n + 1);
        let previous = format!("{}.html", n.max(1) - 1);
        site.put(&format!("/cal/{n}.html"), links(&[&next, &previous]));
    }
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.depth = crawl::MAX_DEPTH;
    plan.max_pages = 5;
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    assert_eq!(report.pages, 5);
    assert_eq!(report.limits, [Limit::Pages]);
    assert!(events.contains(&Event::Limit(Limit::Pages)));
    assert!(site.gets("/cal/4.html").is_empty());
    each_asked_at_most_once(&site);
}

/// A page with 20,000 links: only the first `MAX_LINKS_PER_PAGE` are
/// kept, the page is reported cut, and the crawl ends.
#[tokio::test(flavor = "multi_thread")]
async fn a_page_with_20000_links_keeps_the_first_10000() {
    let site = server();
    let body: String = (0..20_000)
        .map(|n| format!("<a href=\"f{n}.zip\">{n}</a>\n"))
        .collect();
    site.put("/index.html", html(&body));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.dry_run = true;
    plan.max_files = crawl::MAX_FILES;
    plan.max_pages = crawl::MAX_PAGES;
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    let files = found(&events);
    assert_eq!(files.len(), MAX_LINKS_PER_PAGE);
    assert_eq!(report.files, MAX_LINKS_PER_PAGE);
    assert!(files[MAX_LINKS_PER_PAGE - 1].path().ends_with("/f9999.zip"));
    assert!(events.contains(&Event::PageCut {
        url: plan.start.clone()
    }));
    assert_eq!(
        asked_paths(&site),
        BTreeSet::from(["/index.html".to_owned(), "/robots.txt".to_owned()])
    );
}

/// `--max-files`: once reached, no other file is asked for.
#[tokio::test(flavor = "multi_thread")]
async fn max_files_stops_new_files() {
    let site = server();
    let names: Vec<String> = (0..5).map(|n| format!("f{n}.zip")).collect();
    for (n, name) in names.iter().enumerate() {
        site.put(
            &format!("/{name}"),
            FileSpec::new(random_bytes(500, n as u64)),
        );
    }
    let hrefs: Vec<&str> = names.iter().map(String::as_str).collect();
    site.put("/index.html", links(&hrefs));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.max_files = 2;
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    assert_eq!(saved(&events), ["f0.zip", "f1.zip"]);
    assert_eq!(report.limits, [Limit::Files]);
    for name in &names[2..] {
        assert!(site.gets(&format!("/{name}")).is_empty(), "{name}");
    }
}

/// `--max-total`: a file whose `Content-Length` passes what is left is
/// stopped as soon as its headers come, never downloaded whole. (The
/// engine may already have written the first chunk it holds when the stop
/// arrives, so the total can pass the limit by that chunk.)
#[tokio::test(flavor = "multi_thread")]
async fn max_total_stops_a_file_announced_larger_than_what_is_left() {
    let site = server();
    for n in 0..3 {
        site.put(
            &format!("/f{n}.zip"),
            FileSpec::new(random_bytes(100_000, n)),
        );
    }
    site.put("/index.html", links(&["f0.zip", "f1.zip", "f2.zip"]));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.max_total = 250_000;
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    assert_eq!(saved(&events), ["f0.zip", "f1.zip"]);
    assert_eq!(report.limits, [Limit::Total]);
    assert!(report.bytes < 300_000, "{} bytes", report.bytes);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Stopped { url, .. } if url.path().ends_with("/f2.zip"))),
        "{events:?}"
    );
}

/// `--max-total` with a file that announces no length: it is stopped soon
/// after the total passes the limit, not downloaded whole, and can be
/// resumed.
#[tokio::test(flavor = "multi_thread")]
async fn max_total_stops_a_file_without_length_at_the_limit() {
    let site = server();
    let size = 2_000_000;
    site.put(
        "/big.zip",
        FileSpec::new(random_bytes(size, 17))
            .without_length()
            .throttle(1_000_000),
    );
    site.put("/index.html", links(&["big.zip"]));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.max_total = 300_000;
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    assert!(saved(&events).is_empty());
    assert_eq!(report.limits, [Limit::Total]);
    assert!(
        report.bytes > plan.max_total && report.bytes < 1_000_000,
        "{} bytes",
        report.bytes
    );
    let stopped = events.iter().find_map(|e| match e {
        Event::Stopped { state_file, .. } => Some(state_file.clone()),
        _ => None,
    });
    assert!(
        matches!(&stopped, Some(Some(file)) if file.exists()),
        "{stopped:?}"
    );
    let sent = site.gets("/big.zip")[0].bytes_sent;
    assert!(sent < size as u64 / 2, "the server sent {sent} bytes");
}

/// A page larger than `MAX_PAGE_BYTES` is read up to the limit only: links
/// before it are found, links after it are not, and the page is reported
/// cut.
#[tokio::test(flavor = "multi_thread")]
async fn a_page_larger_than_5_mib_is_cut() {
    let site = server();
    let body = format!(
        "<a href=\"a.zip\">a</a><p>{}</p><a href=\"b.zip\">b</a>",
        "x".repeat(crawl::MAX_PAGE_BYTES)
    );
    site.put("/index.html", html(&body));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.dry_run = true;
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    let names: Vec<String> = found(&events)
        .iter()
        .map(|u| u.path_segments().unwrap().next_back().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["a.zip"]);
    assert_eq!(report.pages, 1);
    assert!(events.contains(&Event::PageCut {
        url: plan.start.clone()
    }));
}

/// `MAX_ORIGINS`: with `--files-anywhere`, links to more origins than the
/// limit stop at it; the rest are left out and reported. (A dry run that
/// ignores robots.txt, so none of those hosts is contacted.)
#[tokio::test(flavor = "multi_thread")]
async fn the_origins_limit_holds() {
    let site = server();
    let hrefs: Vec<String> = (0..crawl::MAX_ORIGINS + 20)
        .map(|n| format!("https://site{n}.example/f.zip"))
        .collect();
    let refs: Vec<&str> = hrefs.iter().map(String::as_str).collect();
    site.put("/index.html", links(&refs));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.files_anywhere = true;
    plan.ignore_robots = true;
    plan.dry_run = true;
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    // The start's origin is one of them.
    assert_eq!(report.files, crawl::MAX_ORIGINS - 1);
    assert_eq!(found(&events).len(), crawl::MAX_ORIGINS - 1);
    assert_eq!(report.skipped.get(&Skip::TooManyOrigins), Some(&21));
}

/// ADR-0010 section 7: the browser's cookies go to the start's origin only:
/// never to another origin's file, nor to a CDN a file redirects to. A
/// file's `Referer` is its page's origin alone (a page's is the whole page,
/// on its own origin), and the cookie is in no event or report.
#[tokio::test(flavor = "multi_thread")]
async fn cookies_go_to_the_start_origin_only() {
    let secret = "sid=SECRET-COOKIE-7";
    let site = server();
    let cdn = server();
    site.put(
        "/index.html?q=private-query",
        links(&["p.html", "a.zip", &cdn.url("/b.zip"), "cdn.zip"]),
    );
    site.put("/p.html", links(&[]));
    site.put("/a.zip", FileSpec::new(random_bytes(1000, 18)));
    site.redirect("/cdn.zip", &cdn.url("/cdn.zip"), 302);
    cdn.put("/b.zip", FileSpec::new(random_bytes(1000, 19)));
    cdn.put("/cdn.zip", FileSpec::new(random_bytes(1000, 20)));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html?q=private-query"), dir.path());
    plan.files_anywhere = true;
    plan.cookies = Some(secret.to_owned());
    let scope = scope(&plan).treat_as_public(&Url::parse(&cdn.url("/")).unwrap());
    let (report, events) = grab(&engine(), &plan, &scope).await;

    let mut names = saved(&events);
    names.sort();
    assert_eq!(names, ["a.zip", "b.zip", "cdn.zip"]);
    let site_origin = format!("{}/", origin_of(&site));
    for request in site.requests() {
        assert_eq!(request.cookie.as_deref(), Some(secret), "{}", request.path);
    }
    for request in cdn.requests() {
        assert_eq!(request.cookie, None, "{}", request.path);
    }
    assert_eq!(
        cdn.gets("/b.zip")[0].referer.as_deref(),
        Some(site_origin.as_str())
    );
    assert_eq!(
        cdn.gets("/cdn.zip")[0].referer.as_deref(),
        Some(site_origin.as_str())
    );
    assert_eq!(
        site.gets("/a.zip")[0].referer.as_deref(),
        Some(site_origin.as_str())
    );
    assert_eq!(
        site.gets("/p.html")[0].referer.as_deref(),
        Some(plan.start.as_str())
    );
    assert_eq!(cdn.gets("/robots.txt")[0].referer, None);
    let shown = format!("{events:?} {report:?}");
    assert!(!shown.contains("SECRET"), "{shown}");
}

/// The rate (1 request per second by default) spaces the requests to one
/// origin, measured on a clock only the test moves: none comes a moment
/// early.
#[tokio::test(flavor = "multi_thread")]
async fn the_rate_spaces_requests_to_one_origin() {
    let site = server();
    site.put("/index.html", links(&["p1.html", "p2.html"]));
    site.put("/p1.html", links(&[]));
    site.put("/p2.html", links(&[]));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.rate = crawl::DEFAULT_RATE;
    let (engine, clock) = engine_on_frozen_clock();
    let scope = scope(&plan);
    let task = spawn_grab(engine, plan, scope);

    // robots.txt at once, then one request each second.
    until("robots.txt", || site.requests().len() == 1).await;
    for n in 2..=4 {
        clock.advance(Duration::from_millis(999));
        settle().await;
        assert_eq!(site.requests().len(), n - 1, "a request came early");
        clock.advance(Duration::from_millis(1));
        until("the next request", || site.requests().len() == n).await;
    }
    let (report, _) = task.await.unwrap();
    assert_eq!(report.pages, 3);
}

/// A `Crawl-delay` slower than the rate is respected, from the robots.txt
/// request on.
#[tokio::test(flavor = "multi_thread")]
async fn crawl_delay_slows_the_requests() {
    let site = server();
    site.put(
        "/robots.txt",
        FileSpec::new(b"User-agent: *\nCrawl-delay: 5\n".to_vec()),
    );
    site.put("/index.html", links(&["p1.html"]));
    site.put("/p1.html", links(&[]));
    let dir = tempfile::tempdir().unwrap();
    let plan = plan(&site.url("/index.html"), dir.path());
    let (engine, clock) = engine_on_frozen_clock();
    let scope = scope(&plan);
    let task = spawn_grab(engine, plan, scope);

    until("robots.txt", || site.requests().len() == 1).await;
    for n in 2..=3 {
        clock.advance(Duration::from_millis(4_999));
        settle().await;
        assert_eq!(site.requests().len(), n - 1, "a request came early");
        clock.advance(Duration::from_millis(1));
        until("the next request", || site.requests().len() == n).await;
    }
    let (report, _) = task.await.unwrap();
    assert_eq!(report.pages, 2);
}

/// Owner's decision 2: `--ignore-robots` leaves the rate as it is.
#[tokio::test(flavor = "multi_thread")]
async fn ignore_robots_keeps_the_rate() {
    let site = server();
    site.put("/index.html", links(&["p1.html"]));
    site.put("/p1.html", links(&[]));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.rate = crawl::DEFAULT_RATE;
    plan.ignore_robots = true;
    let (engine, clock) = engine_on_frozen_clock();
    let scope = scope(&plan);
    let task = spawn_grab(engine, plan, scope);

    until("the start page", || site.requests().len() == 1).await;
    clock.advance(Duration::from_millis(999));
    settle().await;
    assert_eq!(site.requests().len(), 1, "a request came early");
    clock.advance(Duration::from_millis(1));
    until("the next request", || site.requests().len() == 2).await;
    task.await.unwrap();
    assert!(site.gets("/robots.txt").is_empty());
}

/// `PAGE_TIME_LIMIT`: a page that stalls fails with "took longer" once the
/// engine's clock passes the limit, and the crawl goes on to the next page.
#[tokio::test(flavor = "multi_thread")]
async fn a_page_that_stalls_fails_and_the_crawl_goes_on() {
    let site = server();
    site.put("/index.html", links(&["slow.html", "next.html"]));
    let slow = format!("<a href=\"never.zip\">n</a>{}", " ".repeat(10_000));
    site.put("/slow.html", html(&slow).stall_after(10));
    site.put("/next.html", links(&["f.zip"]));
    site.put("/f.zip", FileSpec::new(random_bytes(1000, 21)));
    let dir = tempfile::tempdir().unwrap();
    let plan = plan(&site.url("/index.html"), dir.path());
    let (engine, clock) = engine_on_frozen_clock();
    let scope = scope(&plan);
    let task = spawn_grab(engine, plan, scope);

    // The clock runs at about real speed, and jumps past the limit once the
    // slow page has been asked for.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut jumped = false;
    while !task.is_finished() {
        assert!(Instant::now() < deadline, "the crawl did not end");
        if !jumped && !site.gets("/slow.html").is_empty() {
            clock.advance(crawl::PAGE_TIME_LIMIT);
            jumped = true;
        } else {
            clock.advance(Duration::from_millis(10));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (report, events) = task.await.unwrap();

    let failed: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::Failed { url, error } => Some((url.path().to_owned(), error.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert!(failed[0].0.ends_with("/slow.html"));
    assert!(failed[0].1.contains("took longer"), "{}", failed[0].1);
    assert_eq!(saved(&events), ["f.zip"]);
    assert_eq!(report.pages, 2);
    assert!(site.gets("/never.zip").is_empty());
}

/// AGENTS.md §10.4: a crawl over HTTPS works with the test authority, and
/// the same site is refused with the system roots alone (robots.txt cannot
/// be read, so nothing is asked for).
#[tokio::test(flavor = "multi_thread")]
async fn https_works_with_the_test_root_and_is_refused_without_it() {
    let site = TestServer::https(Tls::trusted(&["localhost"]), "localhost");
    site.robots_txt_at_root();
    site.put("/index.html", links(&["a.zip"]));
    site.put("/a.zip", FileSpec::new(random_bytes(1000, 22)));
    let dir = tempfile::tempdir().unwrap();
    let plan = plan(&site.url("/index.html"), dir.path());
    let trusting = Engine::with_test_root(Options::default(), &test_ca_der()).unwrap();
    let (report, events) = grab(&trusting, &plan, &scope(&plan)).await;
    assert_eq!(saved(&events), ["a.zip"]);
    assert_eq!(report.pages, 1);
    assert_eq!(
        site.gets("/a.zip")[0].tls_version.as_deref(),
        Some("TLSv1_3")
    );

    let before = site.requests().len();
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;
    let outcomes = robots_outcomes(&events);
    assert!(
        matches!(&outcomes[..], [(_, RobotsOutcome::DisallowAll(_))]),
        "{outcomes:?}"
    );
    assert_eq!(report.pages, 0);
    assert_eq!(site.requests().len(), before, "a request got through");
}

/// ADR-0010 section 11, in a real crawl: a file on the second server (a
/// local address and another origin) is refused even with
/// `--files-anywhere`, the reason is "a local address", and that server
/// gets nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_on_a_local_address_is_refused_with_files_anywhere() {
    let site = server();
    let local = server();
    local.put("/x.zip", FileSpec::new(random_bytes(1000, 23)));
    site.put("/index.html", links(&[&local.url("/x.zip")]));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.files_anywhere = true;
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    assert_eq!(
        skipped(&events, Skip::LocalAddress),
        [Url::parse(&local.url("/x.zip")).unwrap()]
    );
    assert_eq!(report.skipped.get(&Skip::LocalAddress), Some(&1));
    assert!(saved(&events).is_empty());
    assert!(local.requests().is_empty(), "{:?}", local.requests());
    assert!(local.strays().is_empty(), "{:?}", local.strays());
}

/// `--dry-run`: the pages are read and the files reported, but no file is
/// asked for or written.
#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_reports_the_files_and_downloads_none() {
    let site = server();
    site.put("/index.html", links(&["a.zip", "b.pdf", "next.html"]));
    site.put("/next.html", links(&["c.mp4"]));
    for (path, seed) in [("/a.zip", 24), ("/b.pdf", 25), ("/c.mp4", 26)] {
        site.put(path, FileSpec::new(random_bytes(1000, seed)));
    }
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.dry_run = true;
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    let urls: Vec<String> = found(&events).iter().map(|u| u.to_string()).collect();
    assert_eq!(
        urls,
        [site.url("/a.zip"), site.url("/b.pdf"), site.url("/c.mp4")]
    );
    assert_eq!((report.files, report.saved, report.pages), (3, 0, 2));
    for path in ["/a.zip", "/b.pdf", "/c.mp4"] {
        assert!(site.gets(path).is_empty(), "{path} was asked for");
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

/// Review of #195: a redirect to a URL with a user name and password is
/// refused like a link with them; nothing is sent with `Authorization`.
#[tokio::test(flavor = "multi_thread")]
async fn a_redirect_with_credentials_is_refused() {
    let site = server();
    let target = site
        .url("/next.html")
        .replacen("http://", "http://victim:hunter2@", 1);
    site.redirect("/go", &target, 302);
    site.put("/next.html", links(&["n.zip"]));
    site.put("/index.html", links(&["go"]));
    let dir = tempfile::tempdir().unwrap();
    let plan = plan(&site.url("/index.html"), dir.path());
    let (_, events) = grab(&engine(), &plan, &scope(&plan)).await;

    assert_eq!(skipped(&events, Skip::Credentials).len(), 1);
    assert!(site.gets("/next.html").is_empty());
    assert!(site.requests().iter().all(|r| r.authorization.is_none()));
}

/// Review of #195: links beyond what the queue may hold are said as a
/// limit, never dropped unsaid.
#[tokio::test(flavor = "multi_thread")]
async fn a_full_queue_is_said() {
    let site = server();
    let names: Vec<String> = (0..5).map(|n| format!("q{n}.zip")).collect();
    let hrefs: Vec<&str> = names.iter().map(String::as_str).collect();
    site.put("/index.html", links(&hrefs));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.max_pages = 1;
    plan.max_files = 1;
    plan.dry_run = true;
    let (report, _) = grab(&engine(), &plan, &scope(&plan)).await;

    assert!(
        report.limits.contains(&Limit::Waiting),
        "{:?}",
        report.limits
    );
}

/// Review of #195: a file over `--max-file-size` is left out, not paused:
/// nothing of it stays in the folder, and it is not offered for resuming.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_over_the_size_limit_leaves_nothing() {
    let site = server();
    site.put("/big.zip", FileSpec::new(random_bytes(2_000_000, 31)));
    site.put("/small.zip", FileSpec::new(random_bytes(500, 32)));
    site.put("/index.html", links(&["big.zip", "small.zip"]));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.max_file_size = Some(1000);
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    assert_eq!(saved(&events), ["small.zip"]);
    assert_eq!(skipped(&events, Skip::TooLarge).len(), 1);
    assert!(!events.iter().any(|e| matches!(e, Event::Stopped { .. })));
    assert!(!report.stopped);
    let left: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["small.zip"]);
}

/// Review of #195: links to pages one level deeper than the depth are
/// never kept nor asked for (they would never be read).
#[tokio::test(flavor = "multi_thread")]
async fn pages_past_the_depth_are_not_kept() {
    let site = server();
    site.put("/index.html", links(&["deeper.html", "f.zip"]));
    site.put("/deeper.html", links(&["g.zip"]));
    site.put("/f.zip", FileSpec::new(random_bytes(300, 33)));
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&site.url("/index.html"), dir.path());
    plan.depth = 1;
    let (report, events) = grab(&engine(), &plan, &scope(&plan)).await;

    assert_eq!(saved(&events), ["f.zip"]);
    assert_eq!(report.pages, 1);
    assert!(site.gets("/deeper.html").is_empty());
}
