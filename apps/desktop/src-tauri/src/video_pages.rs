//! The big video sites' pages (a video, a Short, a post that plays, a
//! playlist), from the one list the page uses too, `videoPages.json`:
//! «Batch download» refuses a pattern whose links are these pages, since
//! the engine would save each page's HTML (`watch (1).html`), not its video
//! (#332). The list is read as `videoSites.ts` reads it: the link's host,
//! then its path, each matched by its regular expression.

use std::sync::OnceLock;

use regex::Regex;

/// Each site's host, then its videos' paths, as `videoSites.ts` has them.
const PAGES: &str = include_str!("../../src/lib/videoPages.json");

fn pages() -> &'static [(Regex, Regex)] {
    static LIST: OnceLock<Vec<(Regex, Regex)>> = OnceLock::new();
    LIST.get_or_init(|| {
        let list: Vec<(String, String)> =
            serde_json::from_str(PAGES).expect("videoPages.json is read by its tests");
        list.iter()
            .map(|(host, path)| {
                (
                    Regex::new(host).expect("videoPages.json is read by its tests"),
                    Regex::new(path).expect("videoPages.json is read by its tests"),
                )
            })
            .collect()
    })
}

/// Whether `link` is a video's page on one of the big sites, as
/// `isVideoPage` says on the page.
pub fn is_video_page(link: &str) -> bool {
    let Ok(url) = url::Url::parse(link.trim()) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    pages()
        .iter()
        .any(|(h, p)| h.is_match(host) && p.is_match(url.path()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list is whole: every entry is read, and both expressions work.
    #[test]
    fn the_list_is_read() {
        assert_eq!(pages().len(), PAGES.matches("\"^").count() / 2);
        assert!(pages().len() >= 10);
    }

    /// The same links as `videoSites.test.ts`, and a port, which the host
    /// does not carry.
    #[test]
    fn a_video_page_on_the_big_sites() {
        for link in [
            "https://www.youtube.com/watch?v=x1Yjj1D27Xo&t=942s",
            "https://youtu.be/x1Yjj1D27Xo",
            "https://www.youtube.com/shorts/abc",
            "https://www.youtube.com/playlist?list=PL1a2b3c",
            "https://x.com/someone/status/123",
            "https://twitter.com/someone/status/123",
            "https://www.instagram.com/reel/DfF60hBI4ge/",
            "https://www.facebook.com/100003352791153/videos/1234/",
            "https://fb.watch/abc/",
            "https://www.tiktok.com/@a/video/123",
            "https://vimeo.com/76979871",
            "https://WWW.YouTube.com:443/watch?v=1",
        ] {
            assert!(is_video_page(link), "{link}");
        }
    }

    #[test]
    fn not_a_video_page() {
        for link in [
            "https://github.com/motalim-ai/zajil-download-manager/releases/download/v1.1.3/Zajil_1.1.3_universal.dmg",
            "https://www.youtube.com/",
            "https://x.com/motalim_ai",
            "https://www.instagram.com/someone/",
            "https://vimeo.com/channels/staffpicks",
            "https://notyoutube.com/watch?v=1",
            "https://youtube.com.example.com/watch?v=1",
            "https://youtube.com@example.com/watch?v=1",
            "https://example.com/youtu.be/abc",
            "ftp://youtube.com/watch",
            "not a link",
        ] {
            assert!(!is_video_page(link), "{link}");
        }
    }
}
