//! `zajil batch`: download the files a pattern stands for, one after the
//! other (M7, ADR-0010 section 8).

use std::path::PathBuf;
use std::process::ExitCode;

use zajil_core::{Engine, Error, Finished, Stop, Url};
use zajil_grab::pattern::{self, PatternError, Star};

use crate::{Progress, download_dir, human_bytes, stopped};

/// Exit code of a pattern that was refused: wrong usage, like clap's own.
const EXIT_USAGE: u8 = 2;

#[derive(Debug, clap::Args)]
pub struct BatchArgs {
    /// The URL with ranges and lists: `[001-100]` (zeros give the width),
    /// `[a-z]`, `[0-100:5]` (a step), `{jan,feb}`, or `*` with --from and
    /// --to. Only after the host. `\[`, `\]`, `\{`, `\}`, `\*`, `\,` are the
    /// characters themselves.
    pattern: String,
    /// Where the `*` counter starts: a number or a letter.
    #[arg(long, value_name = "START", requires = "to")]
    from: Option<String>,
    /// Where the `*` counter ends: a number or a letter.
    #[arg(long, value_name = "END", requires = "from")]
    to: Option<String>,
    /// Digits of each `*` number, with zeros on the left: 3 gives 001.
    #[arg(long, value_name = "N", requires = "from")]
    width: Option<u8>,
    /// Most URLs the pattern may make.
    #[arg(
        long,
        value_name = "N",
        default_value_t = pattern::DEFAULT_MAX_URLS,
        value_parser = clap::value_parser!(u64).range(1..=pattern::MAX_URLS)
    )]
    max: u64,
    /// Folder to save into. Without it, the system Downloads folder.
    #[arg(short, long, value_name = "FOLDER")]
    dir: Option<PathBuf>,
    /// Only print the URLs, one per line, without downloading anything.
    #[arg(long)]
    print: bool,
}

pub async fn run(engine: Engine, args: BatchArgs) -> ExitCode {
    let star = args.from.map(|from| Star {
        from,
        to: args.to.unwrap_or_default(),
        width: args.width,
    });
    let urls = match pattern::expand(&args.pattern, star.as_ref(), args.max) {
        Ok(urls) => urls,
        Err(err) => return refused(&err),
    };
    if args.print {
        for url in &urls {
            println!("{url}");
        }
        return ExitCode::SUCCESS;
    }
    let dir = match download_dir(args.dir.as_deref()) {
        Ok(dir) => dir,
        Err(message) => return crate::fail(&message),
    };

    let stop = Stop::new();
    let on_ctrl_c = stop.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            on_ctrl_c.request();
        }
    });

    let total = urls.len();
    let (mut saved, mut failed) = (0usize, 0usize);
    for (n, url) in urls.iter().enumerate() {
        if stop.is_requested() {
            break;
        }
        eprintln!("zajil: [{}/{total}] {}", n + 1, shown(url));
        let mut progress = Progress::new();
        let result = engine
            .download_until(url, &dir, &stop, &mut |e| progress.show(e))
            .await;
        progress.clear_line();
        match result {
            Ok(Finished { file, bytes }) => {
                saved += 1;
                println!("Saved {} ({})", file.display(), human_bytes(bytes));
            }
            Err(Error::Stopped { state_file }) => {
                summary(saved, failed, total);
                return match state_file {
                    Some(state_file) => stopped("cancelled", &state_file),
                    None => crate::fail("cancelled"),
                };
            }
            Err(Error::Interrupted { state_file, reason }) => {
                failed += 1;
                // Resumable on its own: say how, and go on with the next.
                let _ = stopped(&reason, &state_file);
            }
            Err(err) => {
                failed += 1;
                eprintln!("zajil: [{}/{total}] failed: {err}", n + 1);
            }
        }
    }
    summary(saved, failed, total);
    if stop.is_requested() {
        return crate::fail("cancelled");
    }
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn refused(err: &PatternError) -> ExitCode {
    eprintln!("zajil: error: {err}");
    ExitCode::from(EXIT_USAGE)
}

fn summary(saved: usize, failed: usize, total: usize) {
    eprintln!("zajil: {saved} saved, {failed} failed, of {total}");
}

/// `url` as it may be printed: without a user name, password, query or
/// fragment, which may hold a token (owner's decision on #28).
fn shown(url: &str) -> String {
    match Url::parse(url) {
        Ok(mut url) => {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            url.into()
        }
        Err(_) => String::new(),
    }
}
