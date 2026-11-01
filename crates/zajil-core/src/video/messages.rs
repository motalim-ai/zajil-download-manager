//! What yt-dlp says, made safe to show, and what it means: DRM, HLS
//! protection, a challenge, no format, a tool's problem.

use super::VideoError;

/// yt-dlp's message, without what a URL in it may hide (its query,
/// fragment and credentials), on one line, at most 500 characters.
pub fn shown_message(text: &str) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        let word: String = word
            .chars()
            .filter(|&c| !c.is_control() && !crate::category::is_unsafe_in_url(c))
            .collect();
        // A link anywhere in the word (`[https://…]`, `url=https://…`):
        // what is before it stays, the link loses its secrets.
        match ["https://", "http://"].iter().find_map(|s| word.find(s)) {
            Some(at) => {
                let (before, link) = word.split_at(at);
                let link = link.trim_end_matches(|c: char| {
                    matches!(c, '"' | '\'' | '(' | ')' | '<' | '>' | ',' | ']' | '}')
                });
                out.push_str(before);
                out.push_str(&crate::queue::shown_url(link));
            }
            None => out.push_str(&word),
        }
    }
    out.chars().take(500).collect()
}

/// Whether an HLS playlist announces content protection that yt-dlp does
/// not catch by itself (ADR-0009, section 6): any key with a method other
/// than none or AES-128 (`SAMPLE-AES`, `SAMPLE-AES-CTR`…), any key format
/// other than plain `identity` (Widevine, PlayReady, FairPlay…), an `skd://`
/// key, or Adobe's `#EXT-X-FAXS-CM`. AES-128 with a plain key is not DRM:
/// yt-dlp decrypts it like a browser.
pub fn hls_is_protected(playlist: &str) -> bool {
    playlist.lines().any(|line| {
        let line = line.trim();
        if line.starts_with("#EXT-X-FAXS-CM") {
            return true;
        }
        let Some(attrs) = line
            .strip_prefix("#EXT-X-KEY:")
            .or_else(|| line.strip_prefix("#EXT-X-SESSION-KEY:"))
        else {
            return false;
        };
        let upper = attrs.to_ascii_uppercase();
        let method = attribute(&upper, "METHOD").unwrap_or_default();
        let keyformat = attribute(attrs, "KEYFORMAT");
        let uri = attribute(attrs, "URI").unwrap_or_default();
        !matches!(method.as_str(), "NONE" | "AES-128")
            || keyformat.is_some_and(|k| k != "identity")
            || uri.to_ascii_lowercase().starts_with("skd://")
    })
}

/// The value of `NAME=value` or `NAME="value"` in an attribute list.
fn attribute(attrs: &str, name: &str) -> Option<String> {
    let mut rest = attrs;
    while !rest.is_empty() {
        let (key, after) = rest.split_once('=')?;
        let (value, next) = if let Some(quoted) = after.strip_prefix('"') {
            let (value, after) = quoted.split_once('"')?;
            (value, after.strip_prefix(',').unwrap_or(after))
        } else {
            match after.split_once(',') {
                Some((value, next)) => (value, next),
                None => (after, ""),
            }
        };
        if key.trim().eq_ignore_ascii_case(name) {
            return Some(value.trim().to_owned());
        }
        rest = next;
    }
    None
}

/// The longest HLS playlist Zajil reads for its DRM check.
pub(super) const MAX_PLAYLIST: usize = 4 * 1024 * 1024;

/// Whether a message of yt-dlp says the content is protected: its own
/// phrases, not "drm" anywhere (a title or a path, "hdrmovie"…).
pub(super) fn says_drm(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    ["drm protected", "drm-protected", "uses drm", "sample-aes"]
        .iter()
        .any(|phrase| lower.contains(phrase))
}

/// yt-dlp's words when nothing but pictures was left to choose from.
pub(super) fn says_no_format(message: &str) -> bool {
    message.contains("Requested format is not available")
}

/// yt-dlp's words when YouTube's JavaScript challenge went unsolved.
pub(super) fn says_challenge(message: &str) -> bool {
    message.contains("challenge solving failed")
}

/// What a failure of yt-dlp's comes to, when its words say: the user is
/// told that in their language, and yt-dlp's own text is only the detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProblem {
    /// No format the page offers could be chosen.
    NoFormats,
    /// The site wants the user signed in (an age check, members only, a
    /// bot check).
    SignIn,
    /// The video is private.
    Private,
    /// yt-dlp knows no way to read this site.
    Unsupported,
    /// YouTube's JavaScript challenge went unsolved.
    Challenge,
}

impl VideoError {
    /// What yt-dlp's failure comes to, when its words say.
    pub fn problem(&self) -> Option<ToolProblem> {
        let VideoError::Tool(message) = self else {
            return None;
        };
        let lower = message.to_ascii_lowercase();
        let says = |phrases: &[&str]| phrases.iter().any(|p| lower.contains(p));
        // The cause before what it caused: an unsolved challenge leaves no
        // format, and a private video asks to sign in.
        Some(if says_challenge(message) {
            ToolProblem::Challenge
        } else if says(&["private video", "video is private"]) {
            ToolProblem::Private
        } else if says(&[
            "sign in to confirm",
            "login required",
            "use --cookies",
            "--cookies-from-browser",
            "only available for registered users",
            "members-only",
            "join this channel",
        ]) {
            ToolProblem::SignIn
        } else if says(&["unsupported url"]) {
            ToolProblem::Unsupported
        } else if says_no_format(message) || says(&["no video formats found"]) {
            ToolProblem::NoFormats
        } else {
            return None;
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_hide_what_urls_carry() {
        let shown = shown_message(
            "Unable to download https://u:p@cdn.example.com/a/b.m3u8?sig=SECRET#x: HTTP 403",
        );
        assert!(
            !shown.contains("SECRET") && !shown.contains("u:p"),
            "{shown}"
        );
        assert!(
            shown.contains("https://cdn.example.com/a/b.m3u8"),
            "{shown}"
        );
    }

    #[test]
    fn hls_protection_is_recognised_and_plain_aes_is_not() {
        let with = |key: &str| format!("#EXTM3U\n#EXT-X-VERSION:7\n{key}\n#EXTINF:1,\na.m4s\n");
        for protected in [
            r#"#EXT-X-KEY:METHOD=SAMPLE-AES,URI="skd://k",KEYFORMAT="com.apple.streamingkeydelivery""#,
            r#"#EXT-X-KEY:METHOD=SAMPLE-AES-CTR,URI="data:x",KEYFORMAT="urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed""#,
            r#"#EXT-X-SESSION-KEY:METHOD=SAMPLE-AES,URI="https://k",KEYFORMAT="com.microsoft.playready""#,
            r#"#EXT-X-KEY:METHOD=AES-128,URI="skd://k""#,
            "#EXT-X-FAXS-CM:abc",
            r#"#EXT-X-KEY:METHOD=SAMPLE-AES,URI="https://k/key""#,
        ] {
            assert!(hls_is_protected(&with(protected)), "{protected}");
        }
        for plain in [
            r#"#EXT-X-KEY:METHOD=AES-128,URI="https://k/key.bin",IV=0x1234"#,
            r#"#EXT-X-KEY:METHOD=AES-128,URI="https://k/key",KEYFORMAT="identity""#,
            "#EXT-X-KEY:METHOD=NONE",
            "",
        ] {
            assert!(!hls_is_protected(&with(plain)), "{plain}");
        }
    }

    #[test]
    fn messages_lose_link_secrets_wherever_the_link_is() {
        for text in [
            "failed [https://cdn.example.com/a?sig=SECRET]",
            "url=https://u:p@cdn.example.com/a?sig=SECRET#frag",
            "(https://cdn.example.com/a?sig=SECRET)",
            "x\u{202E}https://cdn.example.com/a?sig=SECRET",
        ] {
            let shown = shown_message(text);
            assert!(
                !shown.contains("SECRET") && !shown.contains("u:p"),
                "{shown}"
            );
            assert!(!shown.contains('\u{202E}'), "{shown:?}");
            assert!(shown.contains("cdn.example.com/a"), "{shown}");
        }
    }

    #[test]
    fn only_yt_dlp_s_own_drm_phrases_are_drm() {
        assert!(says_drm("This video is DRM protected"));
        assert!(says_drm("The requested format uses SAMPLE-AES"));
        for not_drm in [
            "Unable to download hdrmovie.mp4",
            "HTTP Error 404 at /drm/x",
        ] {
            assert!(!says_drm(not_drm), "{not_drm}");
        }
    }
}
