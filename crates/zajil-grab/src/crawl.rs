//! The site grabber (ADR-0010): a breadth-first crawl from one page, down
//! to a depth, that downloads the files it finds with the engine.
//!
//! Every request goes through the engine's clients. Pages are fetched with
//! [`Engine::fetch_page_until`], which never follows a redirect, so each hop
//! is checked against the [`Scope`] and robots.txt before it is asked for.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use zajil_core::limit::Clock;
use zajil_core::{Engine, Error, Event as EngineEvent, FromBrowser, Stop, Url};

use crate::links::{self, LinkKind};
use crate::robots::{MAX_ROBOTS_BYTES, Robots};
use crate::scope::{AllowedHost, Place, Refusal, Scope};

/// Depth when none is given, and the most allowed.
pub const DEFAULT_DEPTH: u8 = 2;
pub const MAX_DEPTH: u8 = 10;
/// Files when no limit is given, and the most allowed.
pub const DEFAULT_MAX_FILES: usize = 500;
pub const MAX_FILES: usize = 10_000;
/// Pages fetched when no limit is given, and the most allowed.
pub const DEFAULT_MAX_PAGES: usize = 500;
pub const MAX_PAGES: usize = 10_000;
/// Total size of the files when no limit is given: 5 GiB.
pub const DEFAULT_MAX_TOTAL: u64 = 5 * 1024 * 1024 * 1024;
/// Bytes of a page that are read.
pub const MAX_PAGE_BYTES: usize = 5 * 1024 * 1024;
/// Different origins one crawl may ask anything of.
pub const MAX_ORIGINS: usize = 100;
/// Requests per second to one origin: the default, the least and the most.
pub const DEFAULT_RATE: f64 = 1.0;
pub const MIN_RATE: f64 = 0.1;
pub const MAX_RATE: f64 = 10.0;
/// Redirects followed for one page.
pub const MAX_REDIRECTS: u8 = 10;
/// Different links one crawl remembers (as 64-bit hashes): each is asked
/// for once, and the memory stays bounded (review of #195).
pub const MAX_SEEN: usize = 1_000_000;
/// The longest one page or robots.txt may take, whatever the server sends
/// (review of #187: a byte now and then would otherwise hold the crawl).
pub const PAGE_TIME_LIMIT: Duration = Duration::from_secs(60);
/// Redirects followed for one robots.txt (RFC 9309 §2.3.1.2: at least 5).
pub const MAX_ROBOTS_REDIRECTS: u8 = 5;

/// What a link's extension says when its tag does not.
const PAGE_EXTENSIONS: &[&str] = &[
    "html", "htm", "xhtml", "shtml", "php", "asp", "aspx", "jsp", "cgi",
];

/// The kinds of files downloaded when no `--types` are given: archives,
/// documents, audio, video and programs; no images, no pages.
pub const DEFAULT_TYPES: &[&str] = &[
    // Archives and disk images.
    "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "iso", "dmg",
    // Documents.
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "epub", "rtf", "txt",
    "csv", // Audio.
    "mp3", "m4a", "aac", "flac", "wav", "ogg", "opus", // Video.
    "mp4", "mkv", "avi", "mov", "webm", "m4v", "wmv", "flv", // Programs.
    "exe", "msi", "apk", "deb", "rpm", "appimage", "pkg",
];

/// What to grab.
#[derive(Debug, Clone)]
pub struct Plan {
    pub start: Url,
    /// Links between the start page and a file, 1 to [`MAX_DEPTH`].
    pub depth: u8,
    /// File extensions to download, lowercase, without the dot.
    pub types: Vec<String>,
    /// Only files whose path matches one of these (`*` is any text).
    pub include: Vec<String>,
    /// No page or file whose path matches one of these.
    pub exclude: Vec<String>,
    pub allowed: Vec<AllowedHost>,
    /// Files linked from the site may be on any origin.
    pub files_anywhere: bool,
    pub max_files: usize,
    pub max_pages: usize,
    pub max_total: u64,
    pub max_file_size: Option<u64>,
    /// Requests per second to one origin.
    pub rate: f64,
    pub ignore_robots: bool,
    /// Fetch the pages, but only report the files.
    pub dry_run: bool,
    pub dir: PathBuf,
    /// The browser's cookies for the start page (from the extension): sent
    /// to the start origin only. Never kept anywhere.
    pub cookies: Option<String>,
}

impl Plan {
    /// A plan with the defaults for everything but `start` and `dir`.
    pub fn new(start: Url, dir: PathBuf) -> Self {
        Plan {
            start,
            depth: DEFAULT_DEPTH,
            types: DEFAULT_TYPES.iter().map(|t| (*t).to_owned()).collect(),
            include: Vec::new(),
            exclude: Vec::new(),
            allowed: Vec::new(),
            files_anywhere: false,
            max_files: DEFAULT_MAX_FILES,
            max_pages: DEFAULT_MAX_PAGES,
            max_total: DEFAULT_MAX_TOTAL,
            max_file_size: None,
            rate: DEFAULT_RATE,
            ignore_robots: false,
            dry_run: false,
            dir,
            cookies: None,
        }
    }
}

/// Why a link was not asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Skip {
    /// Outside the site (another origin, not allowed).
    OutsideSite,
    /// A local or private address outside the site.
    LocalAddress,
    /// A redirect from https to http.
    Downgrade,
    /// A path that repeats itself, or is too deep.
    PathTrap,
    /// robots.txt does not allow it.
    Robots,
    /// Not one of the file types asked for.
    NotWanted,
    /// Left out by `--include` or `--exclude`.
    Filtered,
    /// The page asked not to follow its links (`meta robots nofollow`).
    NoFollow,
    /// Larger than `--max-file-size`.
    TooLarge,
    /// Too many redirects.
    TooManyRedirects,
    /// A user name or password in the link.
    Credentials,
    /// The crawl already asks [`MAX_ORIGINS`] origins.
    TooManyOrigins,
}

impl fmt::Display for Skip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Skip::OutsideSite => "outside the site",
            Skip::LocalAddress => "a local address",
            Skip::Downgrade => "a redirect from https to http",
            Skip::PathTrap => "a path that repeats itself",
            Skip::Robots => "not allowed by robots.txt",
            Skip::NotWanted => "not a type asked for",
            Skip::Filtered => "left out by --include or --exclude",
            Skip::NoFollow => "the page asks not to follow its links",
            Skip::TooLarge => "larger than --max-file-size",
            Skip::TooManyRedirects => "too many redirects",
            Skip::Credentials => "a user name or password in the link",
            Skip::TooManyOrigins => "too many different sites",
        })
    }
}

impl From<Refusal> for Skip {
    fn from(refusal: Refusal) -> Self {
        match refusal {
            Refusal::OtherSite => Skip::OutsideSite,
            Refusal::LocalAddress => Skip::LocalAddress,
            Refusal::Downgrade => Skip::Downgrade,
            Refusal::PathTrap => Skip::PathTrap,
            Refusal::Credentials => Skip::Credentials,
        }
    }
}

/// A limit the crawl reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Limit {
    Files,
    Pages,
    Total,
    /// More links waiting than pages and files may still be taken: the
    /// later ones were left out (review of #195: they were dropped unsaid).
    Waiting,
    /// [`MAX_SEEN`] different links in one crawl.
    Seen,
}

impl fmt::Display for Limit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Limit::Files => "the most files (--max-files)",
            Limit::Pages => "the most pages (--max-pages)",
            Limit::Total => "the most bytes (--max-total)",
            Limit::Waiting => "the most links waiting at once (--max-pages and --max-files): later links were left out",
            Limit::Seen => "the most different links in one crawl",
        })
    }
}

/// What robots.txt said for an origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RobotsOutcome {
    /// Its rules apply.
    Rules,
    /// Unavailable (4xx): everything is allowed.
    AllowAll,
    /// Unreachable (429, 5xx, a network error, a refused redirect):
    /// nothing is allowed.
    DisallowAll(String),
}

/// What the crawl reports as it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Robots {
        origin: String,
        outcome: RobotsOutcome,
    },
    /// A page was fetched and read.
    Page {
        url: Url,
        depth: u8,
    },
    /// A page with more links than [`links::MAX_LINKS_PER_PAGE`], or larger
    /// than [`MAX_PAGE_BYTES`]: only its start was read.
    PageCut {
        url: Url,
    },
    Skipped {
        url: Url,
        why: Skip,
    },
    /// A file found: it is downloaded now (or would be, in a dry run).
    File {
        url: Url,
    },
    Saved {
        url: Url,
        file: PathBuf,
        bytes: u64,
    },
    /// A file stopped by a limit; what was written can be resumed.
    Stopped {
        url: Url,
        state_file: Option<PathBuf>,
    },
    Failed {
        url: Url,
        error: String,
    },
    Limit(Limit),
}

/// The counts at the end.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub pages: usize,
    /// Files found (in a dry run, the files that would be downloaded).
    pub files: usize,
    pub saved: usize,
    pub failed: usize,
    pub bytes: u64,
    pub skipped: BTreeMap<Skip, usize>,
    pub limits: Vec<Limit>,
    /// The crawl was stopped before its end.
    pub stopped: bool,
}

/// Why a plan cannot run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanError(pub String);

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PlanError {}

/// What is known of a link waiting in the queue.
#[derive(Debug, Clone)]
struct Item {
    url: Url,
    /// Links from the start page.
    depth: u8,
    kind: Want,
    /// The page the link was on (for the `Referer`).
    from: Option<Url>,
    /// The URL that redirected to this one, and how many redirects so far.
    redirected_from: Option<Url>,
    hops: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Want {
    Page,
    File,
    Either,
    /// The target of a page's redirect to another origin: a file only
    /// (ADR-0010 section 4, owner's decision 3).
    RedirectedFile,
}

/// Grabs `plan`, reporting each step to `on_event`, until the end or
/// `stop`. `scope` is normally `Scope::new` of the plan (tests may widen
/// it with `treat_as_public`).
pub async fn crawl(
    engine: &Engine,
    plan: &Plan,
    scope: &Scope,
    stop: &Stop,
    on_event: &mut (dyn FnMut(Event) + Send),
) -> Result<Report, PlanError> {
    check(plan)?;
    let mut crawler = Crawler {
        engine,
        plan,
        scope,
        stop,
        clock: engine.clock(),
        interval: Duration::from_secs_f64(1.0 / plan.rate),
        next_request: HashMap::new(),
        robots: HashMap::new(),
        origins: HashSet::new(),
        queued: HashSet::new(),
        hasher: std::collections::hash_map::RandomState::new(),
        queue: VecDeque::new(),
        files_found: 0,
        report: Report::default(),
    };
    crawler.push(
        Item {
            url: plan.start.clone(),
            depth: 0,
            kind: Want::Page,
            from: None,
            redirected_from: None,
            hops: 0,
        },
        on_event,
    );
    crawler.run(on_event).await;
    Ok(crawler.report)
}

fn check(plan: &Plan) -> Result<(), PlanError> {
    let bad = |text: String| Err(PlanError(text));
    if !(1..=MAX_DEPTH).contains(&plan.depth) {
        return bad(format!("the depth must be from 1 to {MAX_DEPTH}"));
    }
    if !(1..=MAX_FILES).contains(&plan.max_files) {
        return bad(format!("--max-files must be from 1 to {MAX_FILES}"));
    }
    if !(1..=MAX_PAGES).contains(&plan.max_pages) {
        return bad(format!("--max-pages must be from 1 to {MAX_PAGES}"));
    }
    if !(MIN_RATE..=MAX_RATE).contains(&plan.rate) {
        return bad(format!("--rate must be from {MIN_RATE} to {MAX_RATE}"));
    }
    if plan.types.is_empty() {
        return bad("no file types to download".to_owned());
    }
    Ok(())
}

struct Crawler<'a> {
    engine: &'a Engine,
    plan: &'a Plan,
    scope: &'a Scope,
    stop: &'a Stop,
    clock: Arc<dyn Clock>,
    interval: Duration,
    /// When each origin may be asked again.
    next_request: HashMap<String, Duration>,
    robots: HashMap<String, Robots>,
    origins: HashSet<String>,
    /// A hash of every URL that entered the queue: each is asked for once.
    /// At most [`MAX_SEEN`].
    queued: HashSet<u64>,
    hasher: std::collections::hash_map::RandomState,
    queue: VecDeque<Item>,
    files_found: usize,
    report: Report,
}

fn origin(url: &Url) -> String {
    url.origin().ascii_serialization()
}

/// The path and query a rule looks at (`/a/b?c`).
fn path_and_query(url: &Url) -> String {
    match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    }
}

/// The lowercase extension of the URL's last path segment, if any.
fn extension(url: &Url) -> Option<String> {
    extension_of(url.path_segments()?.next_back()?)
}

fn extension_of(name: &str) -> Option<String> {
    let (_, ext) = name.rsplit_once('.')?;
    (!ext.is_empty() && ext.len() <= 10 && ext.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| ext.to_ascii_lowercase())
}

/// Removes a stopped download's files: its state file and the part file
/// next to it.
fn remove_download(state_file: &std::path::Path) {
    if let Some(final_path) = zajil_core::state::final_path_for_state(state_file) {
        let _ = std::fs::remove_file(zajil_core::state::part_path(&final_path));
    }
    let _ = std::fs::remove_file(state_file);
}

/// Whether `text` matches `glob`, where `*` is any run of characters and
/// the whole text must match.
pub fn glob_matches(glob: &str, text: &str) -> bool {
    let (p, s) = (glob.as_bytes(), text.as_bytes());
    let (mut i, mut j) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while j < s.len() {
        if i < p.len() && p[i] == b'*' {
            star = Some((i, j));
            i += 1;
        } else if i < p.len() && p[i] == s[j] {
            i += 1;
            j += 1;
        } else if let Some((si, sj)) = star {
            star = Some((si, sj + 1));
            i = si + 1;
            j = sj + 1;
        } else {
            return false;
        }
    }
    p[i..].iter().all(|&b| b == b'*')
}

impl Crawler<'_> {
    fn skip(&mut self, url: &Url, why: Skip, on_event: &mut (dyn FnMut(Event) + Send)) {
        *self.report.skipped.entry(why).or_default() += 1;
        on_event(Event::Skipped {
            url: url.clone(),
            why,
        });
    }

    fn limit(&mut self, limit: Limit, on_event: &mut (dyn FnMut(Event) + Send)) {
        if !self.report.limits.contains(&limit) {
            self.report.limits.push(limit);
            on_event(Event::Limit(limit));
        }
    }

    /// Whether `url` is new to the crawl; it is remembered if so. `None`
    /// once [`MAX_SEEN`] links are remembered.
    fn first_time(&mut self, url: &Url) -> Option<bool> {
        use std::hash::BuildHasher;
        let hash = self.hasher.hash_one(url.as_str());
        if self.queued.contains(&hash) {
            return Some(false);
        }
        if self.queued.len() >= MAX_SEEN {
            return None;
        }
        self.queued.insert(hash);
        Some(true)
    }

    /// Adds `item` to the queue, unless its URL was queued before. A full
    /// queue (it never holds more than the page and file limits) or too
    /// many links seen is said as a limit.
    fn push(&mut self, item: Item, on_event: &mut (dyn FnMut(Event) + Send)) {
        if self.queue.len() >= self.plan.max_pages + self.plan.max_files {
            self.limit(Limit::Waiting, on_event);
            return;
        }
        match self.first_time(&item.url) {
            Some(true) => self.queue.push_back(item),
            Some(false) => {}
            None => self.limit(Limit::Seen, on_event),
        }
    }

    async fn run(&mut self, on_event: &mut (dyn FnMut(Event) + Send)) {
        while let Some(item) = self.queue.pop_front() {
            if self.stop.is_requested() {
                self.report.stopped = true;
                return;
            }
            let as_page = match item.kind {
                Want::Page => true,
                Want::File | Want::RedirectedFile => false,
                Want::Either => match extension(&item.url) {
                    Some(ext) if self.plan.types.contains(&ext) => false,
                    Some(ext) if !PAGE_EXTENSIONS.contains(&ext.as_str()) => {
                        self.skip(&item.url, Skip::NotWanted, on_event);
                        continue;
                    }
                    _ => true,
                },
            };
            if as_page {
                self.page(item, on_event).await;
            } else {
                self.file(item, on_event).await;
            }
        }
    }

    /// The checks every request passes: the scope, the origin limit, the
    /// exclusions and robots.txt. `Ok` is where the URL is.
    async fn admit(
        &mut self,
        item: &Item,
        as_page: bool,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Option<Place> {
        let place = match self.scope.check(
            &item.url,
            as_page,
            item.redirected_from.as_ref(),
            item.kind == Want::RedirectedFile,
        ) {
            Ok(place) => place,
            Err(refusal) => {
                self.skip(&item.url, refusal.into(), on_event);
                return None;
            }
        };
        let path = item.url.path();
        if self.plan.exclude.iter().any(|g| glob_matches(g, path)) {
            self.skip(&item.url, Skip::Filtered, on_event);
            return None;
        }
        let origin = origin(&item.url);
        if !self.origins.contains(&origin) {
            if self.origins.len() >= MAX_ORIGINS {
                self.skip(&item.url, Skip::TooManyOrigins, on_event);
                return None;
            }
            self.origins.insert(origin.clone());
        }
        if !self.plan.ignore_robots {
            if !self.robots.contains_key(&origin) {
                let robots = self.fetch_robots(&item.url, on_event).await?;
                // A slower Crawl-delay counts from the robots.txt request
                // too, not only from the next one.
                if let Some(delay) = robots.crawl_delay().filter(|d| *d > self.interval)
                    && let Some(next) = self.next_request.get_mut(&origin)
                {
                    *next += delay - self.interval;
                }
                self.robots.insert(origin.clone(), robots);
            }
            if !self.robots[&origin].allows(&path_and_query(&item.url)) {
                self.skip(&item.url, Skip::Robots, on_event);
                return None;
            }
        }
        Some(place)
    }

    /// Waits until `url`'s origin may be asked again: the rate, or a
    /// slower `Crawl-delay`. `None` when the crawl is stopped meanwhile.
    async fn wait_turn(&mut self, url: &Url) -> Option<()> {
        let origin = origin(url);
        let delay = self
            .robots
            .get(&origin)
            .and_then(Robots::crawl_delay)
            .map_or(self.interval, |d| d.max(self.interval));
        let now = self.clock.now();
        let at = self
            .next_request
            .get(&origin)
            .copied()
            .unwrap_or(now)
            .max(now);
        self.next_request.insert(origin, at + delay);
        if at > now {
            tokio::select! {
                () = self.clock.sleep_until(at) => {}
                () = self.stop.requested() => {
                    self.report.stopped = true;
                    return None;
                }
            }
        }
        Some(())
    }

    /// The engine with the browser's headers for a request to `to`: the
    /// cookies only for the start origin (`FromBrowser`), and `referer` as
    /// the page the link was on.
    fn engine_for(&self, referer: Option<&Url>) -> Engine {
        self.engine.with_browser(FromBrowser::new(
            self.plan.start.as_str(),
            self.plan.cookies.as_deref(),
            referer.map(Url::as_str),
        ))
    }

    /// A stop for one page or robots.txt: asked when the crawl is stopped,
    /// or once [`PAGE_TIME_LIMIT`] has passed. Its task ends with it.
    fn request_stop(&self) -> (Stop, tokio::task::JoinHandle<()>) {
        let stop = Stop::new();
        let (relay, crawl, clock) = (stop.clone(), self.stop.clone(), self.clock.clone());
        let at = clock.now() + PAGE_TIME_LIMIT;
        let task = tokio::spawn(async move {
            tokio::select! {
                () = crawl.requested() => {}
                () = clock.sleep_until(at) => {}
            }
            relay.request();
        });
        (stop, task)
    }

    /// robots.txt of `url`'s origin (RFC 9309 §2.3.1). `None` when the
    /// crawl was stopped.
    async fn fetch_robots(
        &mut self,
        url: &Url,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Option<Robots> {
        let origin = origin(url);
        let mut at = url.join("/robots.txt").ok()?;
        let mut from: Option<Url> = None;
        let mut redirects = 0u8;
        let outcome = loop {
            // Each hop is checked like any request: never a local address
            // outside the site, never from https to http.
            if let Some(previous) = &from
                && let Err(refusal) = self.scope.check(&at, false, Some(previous), true)
            {
                break Err(format!("its redirect is {refusal}"));
            }
            self.wait_turn(&at).await?;
            let (stop, timer) = self.request_stop();
            let result = self
                .engine_for(None)
                .fetch_robots_until(at.as_str(), MAX_ROBOTS_BYTES, &stop)
                .await;
            timer.abort();
            let page = match result {
                Ok(page) => page,
                Err(Error::Stopped { .. }) if self.stop.is_requested() => {
                    self.report.stopped = true;
                    return None;
                }
                // Unreachable: nothing is allowed (RFC 9309 §2.3.1.4).
                Err(Error::Stopped { .. }) => {
                    break Err(format!(
                        "it took longer than {} s",
                        PAGE_TIME_LIMIT.as_secs()
                    ));
                }
                Err(err) => break Err(err.to_string()),
            };
            match page.status {
                200..=299 => break Ok(Robots::parse(&page.body.unwrap_or_default())),
                // More redirects than RFC 9309 asks to follow, or one
                // without a usable target: as if unavailable.
                300..=399 => {
                    let next = page.location.and_then(|l| at.join(&l).ok());
                    match next {
                        Some(next) if redirects < MAX_ROBOTS_REDIRECTS => {
                            redirects += 1;
                            from = Some(std::mem::replace(&mut at, next));
                        }
                        _ => break Ok(Robots::allow_all()),
                    }
                }
                429 => break Err("the server answered 429".to_owned()),
                400..=499 => break Ok(Robots::allow_all()),
                status => break Err(format!("the server answered {status}")),
            }
        };
        let (robots, event) = match outcome {
            Ok(robots) if robots == Robots::allow_all() => (robots, RobotsOutcome::AllowAll),
            Ok(robots) => (robots, RobotsOutcome::Rules),
            Err(why) => (Robots::disallow_all(), RobotsOutcome::DisallowAll(why)),
        };
        on_event(Event::Robots {
            origin,
            outcome: event,
        });
        Some(robots)
    }

    /// A page: fetched (if the depth leaves something to find on it), its
    /// redirect checked and queued, or its links queued.
    async fn page(&mut self, item: Item, on_event: &mut (dyn FnMut(Event) + Send)) {
        // Every link on a page at the last depth would be deeper still.
        if item.depth >= self.plan.depth {
            return;
        }
        if self.report.pages >= self.plan.max_pages {
            self.limit(Limit::Pages, on_event);
            return;
        }
        if self.admit(&item, true, on_event).await.is_none() {
            return;
        }
        if self.wait_turn(&item.url).await.is_none() {
            return;
        }
        let (stop, timer) = self.request_stop();
        let result = self
            .engine_for(item.from.as_ref())
            .fetch_page_until(item.url.as_str(), MAX_PAGE_BYTES, &stop)
            .await;
        timer.abort();
        let page = match result {
            Ok(page) => page,
            Err(Error::Stopped { .. }) if self.stop.is_requested() => {
                self.report.stopped = true;
                return;
            }
            Err(Error::Stopped { .. }) => {
                self.report.failed += 1;
                on_event(Event::Failed {
                    url: item.url,
                    error: format!("the page took longer than {} s", PAGE_TIME_LIMIT.as_secs()),
                });
                return;
            }
            Err(err) => {
                self.report.failed += 1;
                on_event(Event::Failed {
                    url: item.url,
                    error: err.to_string(),
                });
                return;
            }
        };
        if (300..400).contains(&page.status) {
            return self.redirected(item, page.location, on_event);
        }
        if !(200..300).contains(&page.status) {
            self.report.failed += 1;
            on_event(Event::Failed {
                url: item.url,
                error: format!("the server answered {}", page.status),
            });
            return;
        }
        let Some(body) = page.body else {
            // Not HTML: a file after all, named by the server.
            let wanted = extension_of(&page.name)
                .or_else(|| extension(&item.url))
                .is_some_and(|ext| self.plan.types.contains(&ext));
            if wanted {
                self.file(item, on_event).await;
            } else {
                self.skip(&item.url, Skip::NotWanted, on_event);
            }
            return;
        };
        self.report.pages += 1;
        on_event(Event::Page {
            url: item.url.clone(),
            depth: item.depth,
        });
        let found = links::extract(&String::from_utf8_lossy(&body), &item.url);
        if page.truncated || found.truncated {
            on_event(Event::PageCut {
                url: item.url.clone(),
            });
        }
        for link in found.links {
            let kind = match link.kind {
                LinkKind::Page => Want::Page,
                LinkKind::File => Want::File,
                LinkKind::Either => Want::Either,
            };
            let to_page = kind == Want::Page
                || (kind == Want::Either
                    && !extension(&link.url).is_some_and(|e| self.plan.types.contains(&e)));
            // A page one link deeper than the depth would never be read:
            // not kept at all (review of #195: the memory grew with them).
            if to_page && item.depth + 1 >= self.plan.depth {
                continue;
            }
            if found.nofollow && to_page {
                self.skip(&link.url, Skip::NoFollow, on_event);
                continue;
            }
            // What the scope refuses anyway is not kept either; it is
            // checked again before any request.
            if let Err(refusal) = self.scope.check(&link.url, to_page, None, false) {
                self.skip(&link.url, refusal.into(), on_event);
                continue;
            }
            self.push(
                Item {
                    url: link.url,
                    depth: item.depth + 1,
                    kind,
                    from: Some(item.url.clone()),
                    redirected_from: None,
                    hops: 0,
                },
                on_event,
            );
        }
    }

    /// A page answered with a redirect: its target is queued at the same
    /// depth, as a page if it is of the site, else as a file only.
    fn redirected(
        &mut self,
        item: Item,
        location: Option<String>,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) {
        let Some(target) = location.and_then(|l| item.url.join(&l).ok()) else {
            self.report.failed += 1;
            on_event(Event::Failed {
                url: item.url,
                error: "a redirect without a target".to_owned(),
            });
            return;
        };
        if item.hops >= MAX_REDIRECTS {
            self.skip(&target, Skip::TooManyRedirects, on_event);
            return;
        }
        // Of another site, the target may still be a file (decision 3);
        // any other refusal is said as itself (review of #195: a target
        // with credentials was said to be outside the site).
        let as_page = match self.scope.check(&target, true, Some(&item.url), false) {
            Ok(_) => true,
            Err(Refusal::OtherSite) => false,
            Err(refusal) => {
                self.skip(&target, refusal.into(), on_event);
                return;
            }
        };
        let kind = if as_page {
            Want::Page
        } else {
            Want::RedirectedFile
        };
        if kind == Want::RedirectedFile
            && !extension(&target).is_some_and(|e| self.plan.types.contains(&e))
        {
            self.skip(&target, Skip::OutsideSite, on_event);
            return;
        }
        // A redirect target goes first, so A → B → A ends at once.
        if self.first_time(&target) == Some(true) {
            self.queue.push_front(Item {
                url: target,
                depth: item.depth,
                kind,
                from: item.from,
                redirected_from: Some(item.url),
                hops: item.hops + 1,
            });
        }
    }

    /// A file: checked, then downloaded by the engine (or only reported, in
    /// a dry run), within the file, size and total limits.
    async fn file(&mut self, item: Item, on_event: &mut (dyn FnMut(Event) + Send)) {
        if item.depth > self.plan.depth {
            return;
        }
        // A link that only a tag calls a file (`img`, `video`…) still has
        // to be a type asked for; `a` links and pages that turned out to be
        // files were checked by their caller.
        let wanted_type = !matches!(item.kind, Want::File | Want::RedirectedFile)
            || extension(&item.url).is_some_and(|e| self.plan.types.contains(&e));
        if !wanted_type {
            self.skip(&item.url, Skip::NotWanted, on_event);
            return;
        }
        let path = item.url.path();
        if !self.plan.include.is_empty() && !self.plan.include.iter().any(|g| glob_matches(g, path))
        {
            self.skip(&item.url, Skip::Filtered, on_event);
            return;
        }
        if self.files_found >= self.plan.max_files {
            self.limit(Limit::Files, on_event);
            return;
        }
        if self.report.bytes >= self.plan.max_total {
            self.limit(Limit::Total, on_event);
            return;
        }
        if self.admit(&item, false, on_event).await.is_none() {
            return;
        }
        self.files_found += 1;
        self.report.files += 1;
        on_event(Event::File {
            url: item.url.clone(),
        });
        if self.plan.dry_run {
            return;
        }
        if self.wait_turn(&item.url).await.is_none() {
            return;
        }
        self.download(item, on_event).await;
    }

    async fn download(&mut self, item: Item, on_event: &mut (dyn FnMut(Event) + Send)) {
        // The `Referer` of a file is its page's origin only: the engine
        // follows a file's redirects itself, and keeps the header we set.
        let referer = item
            .from
            .as_ref()
            .and_then(|page| Url::parse(&format!("{}/", origin(page))).ok());
        let engine = self.engine_for(referer.as_ref());

        // The file stops on the crawl's stop, or once it passes a limit.
        let file_stop = Stop::new();
        let relay = file_stop.clone();
        let crawl_stop = self.stop.clone();
        let relay_task = tokio::spawn(async move {
            crawl_stop.requested().await;
            relay.request();
        });
        let before = self.report.bytes;
        let max_total = self.plan.max_total;
        let max_file = self.plan.max_file_size;
        // A file the server announces larger than `--max-file-size`, or than
        // what is left of `--max-total`, is held before anything of it is
        // created (#196): no part file, nothing past the limit.
        let announced = Arc::new(AtomicU64::new(0));
        let said = announced.clone();
        let engine = engine.holding(Arc::new(move |_, _, size| {
            let Some(size) = size else {
                return false;
            };
            said.store(size, Ordering::Relaxed);
            max_file.is_some_and(|max| size > max) || before.saturating_add(size) > max_total
        }));
        let written = Arc::new(AtomicU64::new(0));
        let seen = written.clone();
        let mut why_stopped: Option<Skip> = None;
        let mut over_total = false;
        let result = engine
            .download_until(
                item.url.as_str(),
                &self.plan.dir,
                &file_stop,
                // The hold checked the first answer's size. When the
                // engine starts over with a new answer (a broken
                // connection on a server that cannot continue, a file that
                // changed), its size comes with `Started` again: checked
                // here before anything of it is written. A file with no size
                // is stopped once it passes a limit.
                &mut |event| match event {
                    EngineEvent::Started {
                        total: Some(total), ..
                    } if max_file.is_some_and(|max| total > max) => {
                        why_stopped = Some(Skip::TooLarge);
                        file_stop.request();
                    }
                    EngineEvent::Started {
                        total: Some(total), ..
                    } if before.saturating_add(total) > max_total => {
                        over_total = true;
                        file_stop.request();
                    }
                    EngineEvent::Progress { done, .. } => {
                        seen.store(done, Ordering::Relaxed);
                        if before.saturating_add(done) > max_total {
                            over_total = true;
                            file_stop.request();
                        }
                        if max_file.is_some_and(|max| done > max) {
                            why_stopped = Some(Skip::TooLarge);
                            file_stop.request();
                        }
                    }
                    _ => {}
                },
            )
            .await;
        relay_task.abort();
        match result {
            Ok(done) => {
                self.report.saved += 1;
                self.report.bytes = self.report.bytes.saturating_add(done.bytes);
                on_event(Event::Saved {
                    url: item.url,
                    file: done.file,
                    bytes: done.bytes,
                });
            }
            Err(Error::Stopped { state_file }) => {
                self.report.bytes = self
                    .report
                    .bytes
                    .saturating_add(written.load(Ordering::Relaxed));
                if let Some(why) = why_stopped {
                    // Left out, not paused: nothing of it stays to resume
                    // (review of #195).
                    if let Some(state_file) = &state_file {
                        remove_download(state_file);
                    }
                    self.skip(&item.url, why, on_event);
                    return;
                } else if over_total {
                    self.limit(Limit::Total, on_event);
                } else {
                    self.report.stopped = true;
                }
                on_event(Event::Stopped {
                    url: item.url,
                    state_file,
                });
            }
            Err(Error::Held(_)) => {
                let size = announced.load(Ordering::Relaxed);
                if max_file.is_some_and(|max| size > max) {
                    self.skip(&item.url, Skip::TooLarge, on_event);
                } else {
                    self.limit(Limit::Total, on_event);
                }
            }
            Err(err) => {
                self.report.failed += 1;
                on_event(Event::Failed {
                    url: item.url,
                    error: err.to_string(),
                });
            }
        }
    }
}
