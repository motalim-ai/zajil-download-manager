//! `zajil grab`: download a site's files, from one page down to a depth
//! (M7, ADR-0010).

use std::path::PathBuf;
use std::process::ExitCode;

use zajil_core::{Engine, Stop, Url};
use zajil_grab::crawl::{self, Event, Limit, Plan, Report, RobotsOutcome};
use zajil_grab::scope::{AllowedHost, Scope};

use crate::batch::shown;
use crate::{download_dir, human_bytes};

/// Exit code of a refused request: wrong usage, like clap's own.
const EXIT_USAGE: u8 = 2;
/// Exit code of a crawl stopped with Ctrl+C.
const EXIT_STOPPED: u8 = 3;

#[derive(Debug, clap::Args)]
pub struct GrabArgs {
    /// The page to start from (http or https).
    url: String,
    /// Links from the start page to a file.
    #[arg(
        long,
        value_name = "N",
        default_value_t = crawl::DEFAULT_DEPTH,
        value_parser = clap::value_parser!(u8).range(1..=crawl::MAX_DEPTH as i64)
    )]
    depth: u8,
    /// File types to download, by extension, separated by commas (such as
    /// zip,pdf,mp4). Without it: archives, documents, audio, video and
    /// programs; no images and no pages.
    #[arg(long, value_name = "EXT,EXT", value_delimiter = ',')]
    types: Vec<String>,
    /// Only files whose path matches (`*` is any text). Repeatable.
    #[arg(long, value_name = "GLOB")]
    include: Vec<String>,
    /// No page or file whose path matches (`*` is any text). Repeatable;
    /// it wins over --include.
    #[arg(long, value_name = "GLOB")]
    exclude: Vec<String>,
    /// Another site that is part of this one: its pages and files, with its
    /// own robots.txt. HOST alone is its usual ports. Repeatable.
    #[arg(long, value_name = "HOST[:PORT]")]
    allow_host: Vec<AllowedHost>,
    /// Files linked from the site may be on any site (a CDN). No page of
    /// another site is read.
    #[arg(long)]
    files_anywhere: bool,
    /// Most files to download.
    #[arg(
        long,
        value_name = "N",
        default_value_t = crawl::DEFAULT_MAX_FILES,
        value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..=crawl::MAX_FILES as u64)
    )]
    max_files: usize,
    /// Most pages to ask for: every page request counts, whatever the
    /// answer (a 404 or a redirect too).
    #[arg(
        long,
        value_name = "N",
        default_value_t = crawl::DEFAULT_MAX_PAGES,
        value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..=crawl::MAX_PAGES as u64)
    )]
    max_pages: usize,
    /// Most bytes in all, such as 500M or 2G (5G when not given). A file
    /// that would pass it is stopped, and can be resumed.
    #[arg(long, value_name = "SIZE", value_parser = size)]
    max_total: Option<u64>,
    /// Files larger than this are left out, such as 100M.
    #[arg(long, value_name = "SIZE", value_parser = size)]
    max_file_size: Option<u64>,
    /// Requests per second to one host, 0.1 to 10. A slower Crawl-delay in
    /// robots.txt is respected.
    #[arg(long, value_name = "R", default_value_t = crawl::DEFAULT_RATE, value_parser = rate)]
    rate: f64,
    /// Do not read robots.txt. The site asked not to be crawled there:
    /// use it only on a site that is yours or allows it.
    #[arg(long)]
    ignore_robots: bool,
    /// Folder to save into. Without it, the system Downloads folder.
    #[arg(short, long, value_name = "FOLDER")]
    dir: Option<PathBuf>,
    /// Read the pages and list the files, without downloading any.
    #[arg(long)]
    dry_run: bool,
}

/// A size given on the command line: bytes, or K, M or G.
fn size(text: &str) -> Result<u64, String> {
    zajil_core::limit::parse_rate(text)
        .ok_or_else(|| format!("{text:?} is not a size: use bytes, or K, M or G (such as 500M)"))
}

fn rate(text: &str) -> Result<f64, String> {
    let bad = || {
        format!(
            "--rate must be a number from {} to {}",
            crawl::MIN_RATE,
            crawl::MAX_RATE
        )
    };
    let rate: f64 = text.parse().map_err(|_| bad())?;
    (crawl::MIN_RATE..=crawl::MAX_RATE)
        .contains(&rate)
        .then_some(rate)
        .ok_or_else(bad)
}

pub async fn run(engine: Engine, args: GrabArgs) -> ExitCode {
    let start = match Url::parse(&args.url) {
        Ok(url) => url,
        Err(_) => return refused("the start must be an http or https URL"),
    };
    let scope = match Scope::new(&start, args.allow_host.clone(), args.files_anywhere) {
        Ok(scope) => scope,
        Err(why) => return refused(&why),
    };
    let dir = if args.dry_run {
        PathBuf::new()
    } else {
        match download_dir(args.dir.as_deref()) {
            Ok(dir) => dir,
            Err(message) => return crate::fail(&message),
        }
    };
    let mut plan = Plan::new(start, dir);
    plan.depth = args.depth;
    if !args.types.is_empty() {
        plan.types = args
            .types
            .iter()
            .map(|t| t.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
    }
    plan.include = args.include;
    plan.exclude = args.exclude;
    plan.allowed = args.allow_host;
    plan.files_anywhere = args.files_anywhere;
    plan.max_files = args.max_files;
    plan.max_pages = args.max_pages;
    plan.max_total = args.max_total.unwrap_or(crawl::DEFAULT_MAX_TOTAL);
    plan.max_file_size = args.max_file_size;
    plan.rate = args.rate;
    plan.ignore_robots = args.ignore_robots;
    plan.dry_run = args.dry_run;

    if plan.ignore_robots {
        eprintln!(
            "zajil: warning: robots.txt is ignored; the site may have asked not to be crawled"
        );
    }
    let stop = Stop::new();
    crate::stop_on_ctrl_c(stop.clone());

    let dry_run = plan.dry_run;
    let mut resumable = Vec::new();
    let result = crawl::crawl(&engine, &plan, &scope, &stop, &mut |event| {
        show(&event, dry_run);
        if let Event::Stopped {
            state_file: Some(state_file),
            ..
        } = event
        {
            resumable.push(state_file);
        }
    })
    .await;
    let report = match result {
        Ok(report) => report,
        Err(err) => return refused(&err.to_string()),
    };
    summary(&report, dry_run);
    for state_file in &resumable {
        eprintln!("zajil: resume with: zajil resume {}", state_file.display());
    }
    if report.stopped {
        eprintln!("zajil: stopped");
        ExitCode::from(EXIT_STOPPED)
    } else if report.pages == 0 && report.files == 0 {
        // Nothing could be read from the start page (the site cannot be
        // reached, its robots.txt allows nothing): a failure, not "done"
        // (review of #195).
        if report.limits.contains(&Limit::Pages) {
            // A redirect of the start costs a page request too (review of
            // #232).
            eprintln!("zajil: --max-pages was reached before a page could be read: raise it");
        } else {
            eprintln!("zajil: nothing could be read from the start page");
        }
        if let Some(to) = &report.start_left_site {
            let host = match (to.host_str(), to.port()) {
                (Some(host), Some(port)) => format!("{host}:{port}"),
                (Some(host), None) => host.to_owned(),
                (None, _) => String::new(),
            };
            eprintln!(
                "zajil: the start redirects to {}, another site: to crawl it, add --allow-host {host}",
                shown(to.as_str())
            );
        }
        ExitCode::FAILURE
    } else if report.failed > 0 || !resumable.is_empty() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// One line on stderr for each step that matters. Every URL without its
/// query or credentials (owner's decision on #28).
fn show(event: &Event, dry_run: bool) {
    match event {
        Event::Robots { origin, outcome } => match outcome {
            RobotsOutcome::DisallowAll(why) => eprintln!(
                "zajil: robots.txt of {origin} could not be read ({why}): nothing there is asked for"
            ),
            RobotsOutcome::Rules | RobotsOutcome::AllowAll => {}
        },
        Event::Page { url, depth } => {
            eprintln!("zajil: page {} (depth {depth})", shown(url.as_str()))
        }
        Event::PageCut { url } => {
            eprintln!(
                "zajil: page {} is too large: only its start was read",
                shown(url.as_str())
            )
        }
        Event::Skipped { .. } => {}
        Event::File { url } if dry_run => println!("{}", shown(url.as_str())),
        Event::File { .. } => {}
        Event::Saved { url, file, bytes } => eprintln!(
            "zajil: saved {} ({}) from {}",
            file.display(),
            human_bytes(*bytes),
            shown(url.as_str())
        ),
        Event::Stopped { url, .. } => eprintln!("zajil: stopped {}", shown(url.as_str())),
        Event::Failed { url, error } => {
            eprintln!("zajil: failed {}: {error}", shown(url.as_str()))
        }
        Event::Limit(limit) => eprintln!("zajil: reached {limit}"),
    }
}

fn summary(report: &Report, dry_run: bool) {
    if dry_run {
        eprintln!(
            "zajil: {} pages read, {} files would be downloaded",
            report.pages, report.files
        );
    } else {
        eprintln!(
            "zajil: {} pages read, {} files saved ({}), {} failed",
            report.pages,
            report.saved,
            human_bytes(report.bytes),
            report.failed
        );
    }
    for (why, count) in &report.skipped {
        eprintln!("zajil: {count} left out: {why}");
    }
    if report.limits.contains(&Limit::Total) {
        eprintln!("zajil: raise --max-total to download more");
    }
}

fn refused(why: &str) -> ExitCode {
    eprintln!("zajil: error: {why}");
    ExitCode::from(EXIT_USAGE)
}
