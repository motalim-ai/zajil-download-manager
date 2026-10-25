//! Whether a newer Zajil is out (#124, owner's decision): one question to
//! GitHub's list of releases, never a download or an install. The answer
//! says the newer version and its release page, built here on Zajil's own
//! releases address (never a link taken from the answer).
//!
//! The list, not `/releases/latest`: that one leaves out pre-releases, and
//! every 0.x release is one. Drafts and tags that are not `vX.Y.Z` are
//! passed over. One request with the engine itself (its TLS, its
//! `Zajil/…` User-Agent), and at most [`MAX_ANSWER`] bytes of answer.

use serde::Deserialize;

use crate::{Engine, Error, Stop};

/// GitHub's list of Zajil's releases.
pub const RELEASES: &str =
    "https://api.github.com/repos/motalim-ai/zajil-download-manager/releases?per_page=30";

/// Every release page is under this address.
pub const RELEASE_PAGES: &str = "https://github.com/motalim-ai/zajil-download-manager/releases/";

/// The most of GitHub's answer that is read.
pub const MAX_ANSWER: usize = 1024 * 1024;

/// A release newer than the running Zajil.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRelease {
    /// `X.Y.Z`, without the `v`.
    pub version: String,
    /// Its page, always under [`RELEASE_PAGES`].
    pub page: String,
}

/// Why the question could not be answered.
#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error("could not ask GitHub: {0}")]
    Engine(#[from] Error),
    #[error("GitHub's answer could not be read: {0}")]
    Answer(String),
    #[error("{0:?} is not a version of the form X.Y.Z")]
    Version(String),
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
}

/// The newest release on GitHub if it is newer than `current` (`X.Y.Z`,
/// the running app's version), else `None`.
pub async fn newer_release(
    engine: &Engine,
    current: &str,
    stop: &Stop,
) -> Result<Option<NewRelease>, CheckError> {
    ask(engine, RELEASES, current, stop).await
}

/// **Test builds only.** [`newer_release`], asking `list` (a local test
/// server's) instead of GitHub.
#[cfg(feature = "test-support")]
pub async fn newer_release_from(
    engine: &Engine,
    list: &str,
    current: &str,
    stop: &Stop,
) -> Result<Option<NewRelease>, CheckError> {
    ask(engine, list, current, stop).await
}

async fn ask(
    engine: &Engine,
    list: &str,
    current: &str,
    stop: &Stop,
) -> Result<Option<NewRelease>, CheckError> {
    let current = version(current).ok_or_else(|| CheckError::Version(current.to_owned()))?;
    let answer = engine.fetch_text(list, MAX_ANSWER, stop).await?;
    newest(&answer, current)
}

/// The newest release in GitHub's answer `json` that is newer than
/// `current`.
fn newest(json: &str, current: (u64, u64, u64)) -> Result<Option<NewRelease>, CheckError> {
    let releases: Vec<Release> =
        serde_json::from_str(json).map_err(|e| CheckError::Answer(e.to_string()))?;
    let best = releases
        .iter()
        .filter(|r| !r.draft)
        .filter_map(|r| {
            let number = r.tag_name.strip_prefix('v')?;
            Some((version(number)?, number))
        })
        .filter(|(v, _)| *v > current)
        .max_by_key(|(v, _)| *v);
    Ok(best.map(|(_, number)| NewRelease {
        version: number.to_owned(),
        page: format!("{RELEASE_PAGES}tag/v{number}"),
    }))
}

/// `X.Y.Z` (digits only, each at most 9 of them), or `None`.
fn version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('.');
    let mut part = || {
        let p = parts.next()?;
        (!p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse().ok())
            .flatten()
    };
    let found = (part()?, part()?, part()?);
    parts.next().is_none().then_some(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, draft: bool) -> String {
        format!(r#"{{"tag_name":"{tag}","draft":{draft},"html_url":"https://evil.example/x"}}"#)
    }

    #[test]
    fn the_newest_release_newer_than_this_one_is_found() {
        let json = format!(
            "[{},{},{},{},{}]",
            release("v0.1.0", false),
            release("v0.2.0", false),
            release("v0.10.1", false),
            release("v0.11.0", true),
            release("nightly", false)
        );
        let found = newest(&json, (0, 1, 0)).unwrap().unwrap();
        // 0.10.1 is newer than 0.2.0; a draft and a tag of another form
        // do not count; the page is built here, never the answer's.
        assert_eq!(found.version, "0.10.1");
        assert_eq!(
            found.page,
            "https://github.com/motalim-ai/zajil-download-manager/releases/tag/v0.10.1"
        );
        assert_eq!(newest(&json, (0, 10, 1)).unwrap(), None);
        assert_eq!(newest("[]", (0, 1, 0)).unwrap(), None);
    }

    #[test]
    fn only_plain_versions_count() {
        assert_eq!(version("1.2.3"), Some((1, 2, 3)));
        for bad in [
            "1.2",
            "1.2.3.4",
            "1.2.3-beta",
            "v1.2.3",
            "1..3",
            "1.2.x",
            "1234567890.0.0",
            "",
        ] {
            assert_eq!(version(bad), None, "{bad}");
        }
        let json = format!("[{}]", release("v1.2.3-rc.1", false));
        assert_eq!(newest(&json, (0, 1, 0)).unwrap(), None);
    }

    #[test]
    fn an_answer_that_is_not_a_list_is_an_error() {
        assert!(matches!(
            newest("{}", (0, 1, 0)),
            Err(CheckError::Answer(_))
        ));
        assert!(matches!(
            newest("not json", (0, 1, 0)),
            Err(CheckError::Answer(_))
        ));
    }
}
