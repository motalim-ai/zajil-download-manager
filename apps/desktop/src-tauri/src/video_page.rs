//! The video sites' pages, in the desktop's own words («Batch download»
//! must not send a video site's page to the engine: the answer is the
//! site's page, not the video, and the batch's `*.html` garbage began
//! this way). The rules are [`videoPage.ts`](../../src/lib/videoPage.ts)'s
//! (they carry the extension's `wantedHere` and `playlistPage`): the one
//! list lives on the page (it decides before a read, at no cost), this
//! one lives in Rust (the links of a batch are made again in Rust, so the
//! batch is stopped there). Both lists must stay in step.
//!
//! A host matches its site as its subdomains do (`a.b.youtube.com`), and
//! a site with `only` matches the pages of videos and playlists on it
//! (a page without a video is the rule there, YouTube above all).

use std::sync::OnceLock;

use regex::Regex;

/// Whether `url` is the page of a video (or of a playlist) of a known
/// video site, as `videoPage.ts`'s `looksLikeAVideoPage` says.
pub fn known_video_page(url: &str) -> bool {
    let Some(rest) = web_rest_of(url) else {
        return false;
    };
    let Some((host, path)) = host_path_of(rest) else {
        return false;
    };
    for (hosts, only) in rules() {
        if hosts.iter().any(|site| host_is(&host, site)) {
            return match only {
                Some(only) => only.is_match(&path),
                None => true,
            };
        }
    }
    false
}

/// What follows `http://` or `https://` (only web addresses count; every
/// where else no address sits the same way, and a link of a batch begins
/// with its scheme).
fn web_rest_of(url: &str) -> Option<&str> {
    let scheme_end = url.find("://")?;
    let scheme = url[..scheme_end].to_ascii_lowercase();
    match scheme.as_str() {
        "http" | "https" => Some(&url[scheme_end + 3..]),
        _ => None,
    }
}

/// The site's name (small letters, with its port, as `Url::host` carries
/// it; anything before the last `@` is user information, not the host)
/// and the page's path (the default root when the link says none; the
/// query and the fragment are not the path).
fn host_path_of(rest: &str) -> Option<(String, String)> {
    let slash = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..slash];
    if authority.is_empty() {
        return None;
    }
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let path = match rest.get(slash..) {
        Some(path) if path.starts_with('/') => path.to_owned(),
        _ => "/".to_owned(),
    };
    Some((host.to_ascii_lowercase(), path))
}

/// Whether `host` is `site` itself or carries it as its subdomains:
/// `a.b.site` ends in `.site`, and something else that merely ends
/// alike (`evilsite`) does not.
fn host_is(host: &str, site: &str) -> bool {
    host == site
        || (host.len() > site.len()
            && host.ends_with(site)
            && host.as_bytes()[host.len() - site.len() - 1] == b'.')
}

/// The sites the pages of videos come from, as `videoPage.ts`'s `SITES`.
type Only = Option<Regex>;

fn rules() -> &'static Vec<(&'static [&'static str], Only)> {
    static RULES: OnceLock<Vec<(&'static [&'static str], Only)>> = OnceLock::new();
    RULES.get_or_init(|| {
        fn only(pattern: &'static str) -> Only {
            Some(Regex::new(pattern).expect("a rules test keeps every pattern valid"))
        }
        vec![
            // A video's page, a Short, a going live, an embedded player,
            // or a playlist's page.
            (
                &["youtube.com", "youtube-nocookie.com"],
                only("^/(?:watch\\b|shorts/|live/|embed/|playlist\\b)"),
            ),
            // Links of its length (`youtu.be/<id>`): every path but no one
            // is an id.
            (&["youtu.be"], only(r"^/[^/]")),
            (&["vimeo.com"], None),
            (&["dailymotion.com"], None),
            (&["dai.ly"], None),
            (&["tiktok.com"], None),
            (&["bilibili.com"], None),
            (&["rumble.com"], None),
            // A post that plays, not the site: x.com/<account>/status/<id>
            (&["x.com", "twitter.com"], only(r"/status/[^/]+")),
            (
                &["instagram.com"],
                only(r"^/[\w.-]*/?(?:p|reel|reels|tv)/[^/]+"),
            ),
            (
                &["facebook.com"],
                only(r"^/(?:[^/]+/)?(?:watch\b|video\.php|reel\b|videos/|share/v/)"),
            ),
            (&["fb.watch"], None),
            (&["vk.com"], only(r"^/video")),
            (&["twitch.tv"], only(r"^/(?:[^/]+/)?(?:v|videos|clip)/")),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A YouTube video's page, in every form it has.
    #[test]
    fn a_youtube_video_page() {
        for url in [
            "https://www.youtube.com/watch?v=2hFGsSB02JY",
            "https://m.youtube.com/watch?v=2hFGsSB02JY",
            "https://www.youtube.com/shorts/abc123",
            "https://www.youtube.com/live/xyz456",
            "https://www.youtube.com/playlist?list=PL1a2b3c",
            "https://www.youtube-nocookie.com/embed/abc123",
        ] {
            assert!(known_video_page(url), "{url}");
        }
    }

    /// YouTube's pages without a video: the host is a video site's, the
    /// path is not one of the pages of videos.
    #[test]
    fn a_youtube_page_without_a_video() {
        for url in [
            "https://www.youtube.com/",
            "https://www.youtube.com/feed/subscriptions",
            "https://www.youtube.com/results?search_query=zajil",
            "https://www.youtube.com/@motalim-ai",
            "https://www.youtube.com/channel/UCabc",
        ] {
            assert!(!known_video_page(url), "{url}");
        }
    }

    /// The other video sites' pages: each of their rules says yes.
    #[test]
    fn another_site_says_yes() {
        for url in [
            "https://vimeo.com/123456789",
            "https://www.dailymotion.com/video/x8abcde",
            "https://dai.ly/x8abcde",
            "https://www.tiktok.com/@user/video/7300000000000000000",
            "https://www.bilibili.com/video/BV1abc",
            "https://rumble.com/v1abc-something.html",
            "https://x.com/user/status/1800000000000000000",
            "https://twitter.com/user/status/1700000000000000000",
            "https://www.instagram.com/p/abc123def/",
            "https://www.instagram.com/reel/abc123def/",
            "https://www.facebook.com/watch/?v=123456789",
            "https://www.facebook.com/Page/videos/987654/",
            "https://fb.watch/abcxyz?si=token",
            "https://vk.com/video-123456_789012345",
            "https://www.twitch.tv/videos/987654321",
            "https://www.twitch.tv/channel/clip/HappyCat",
        ] {
            assert!(known_video_page(url), "{url}");
        }
    }

    /// The sites that post (and the rest of theirs): not everything on
    /// them plays.
    #[test]
    fn another_site_says_no() {
        for url in [
            "https://x.com",
            "https://x.com/user",
            "https://twitter.com/user",
            "https://www.instagram.com/",
            "https://www.facebook.com/profile.php?id=10",
            "https://www.twitch.tv/channel",
            "https://vk.com/feed",
        ] {
            assert!(!known_video_page(url), "{url}");
        }
    }

    /// The page's own host says it is a video site's: a link in the path
    /// does not, and neither does a host that ends alike, nor one whose
    /// user information reads like one.
    #[test]
    fn a_host_lies_says_no() {
        for url in [
            "https://notyoutube.com/watch?v=1",
            "https://youtu.be.evil.com/watch",
            "https://example.com/youtu.be/2hFGsSB02JY",
            "https://youtube.com.example.com/watch?v=1",
            "https://youtube.com@evil.com/watch?v=1",
        ] {
            assert!(!known_video_page(url), "{url}");
        }
    }

    /// What is not a web address: every answer is no.
    #[test]
    fn not_a_web_address_says_no() {
        for url in [
            "",
            "not a link",
            "ftp://youtu.be/2hFGsSB02JY",
            "file:///C:/Downloads/watch.html",
            "https://",
        ] {
            assert!(!known_video_page(url), "{url}");
        }
    }
}
