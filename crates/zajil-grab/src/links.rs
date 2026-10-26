//! The links of an HTML page (ADR-0010 section 2), read with html5ever's
//! tokenizer alone: no tree, no JavaScript, no CSS.

use std::cell::RefCell;
use std::collections::HashSet;

use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use zajil_core::Url;

/// Links kept from one page; the rest are left out ([`Links::truncated`]).
pub const MAX_LINKS_PER_PAGE: usize = 10_000;
/// The longest URL kept, in bytes.
pub const MAX_URL_BYTES: usize = 4 * 1024;

/// What a link points to, as far as its tag tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// `iframe`, `frame`, a meta refresh: a page.
    Page,
    /// `img`, `video`, `object`…: a file.
    File,
    /// `a` and `area`: a page or a file, decided by the crawler.
    Either,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// Absolute, http or https, without a fragment or user name.
    pub url: Url,
    pub kind: LinkKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Links {
    /// In the order they appear, each URL once.
    pub links: Vec<Link>,
    /// `<meta name="robots" content="nofollow">` (or `none`): the page asks
    /// that its links to pages are not followed.
    pub nofollow: bool,
    /// More than [`MAX_LINKS_PER_PAGE`] links: the rest were left out.
    pub truncated: bool,
}

/// The links of `html`, a page at `page`.
pub fn extract(html: &str, page: &Url) -> Links {
    let sink = Sink::default();
    let input = BufferQueue::default();
    input.push_back(html.into());
    let tokenizer = Tokenizer::new(sink, TokenizerOpts::default());
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    let found = tokenizer.sink.found.into_inner();

    // The first `<base href>` is the base of the whole page, also for the
    // links before it (HTML: "the document base URL").
    let base = found
        .base
        .as_deref()
        .and_then(|href| page.join(href).ok())
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or_else(|| page.clone());

    let mut out = Links {
        nofollow: found.nofollow,
        ..Links::default()
    };
    let mut seen = HashSet::new();
    for (raw, kind) in found.raw {
        let Some(url) = resolve(&base, &raw) else {
            continue;
        };
        if !seen.insert(url.as_str().to_owned()) {
            continue;
        }
        if out.links.len() == MAX_LINKS_PER_PAGE {
            out.truncated = true;
            break;
        }
        out.links.push(Link { url, kind });
    }
    out
}

/// `raw` as an absolute link worth following: http or https, no user name
/// or password, not too long; without its fragment.
fn resolve(base: &Url, raw: &str) -> Option<Url> {
    let raw = raw.trim_matches(|c: char| c.is_ascii_whitespace());
    if raw.is_empty() || raw.len() > MAX_URL_BYTES {
        return None;
    }
    let mut url = base.join(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    url.set_fragment(None);
    if let Some(path) = tidy_escapes(url.path()) {
        url.set_path(&path);
    }
    (url.as_str().len() <= MAX_URL_BYTES).then_some(url)
}

/// `path` without needless percent-encoding (ADR-0010 section 6), so one
/// page has one URL: an escaped letter, digit, `-`, `.`, `_` or `~` is
/// written as itself (`s%65lf` is `self`), and other escapes take capital
/// hex digits. `None` when there is nothing to change.
fn tidy_escapes(path: &str) -> Option<String> {
    if !path.contains('%') {
        return None;
    }
    let hex = |b: Option<&u8>| b.and_then(|b| char::from(*b).to_digit(16));
    let bytes = path.as_bytes();
    let mut out = String::with_capacity(path.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(high), Some(low)) = (hex(bytes.get(i + 1)), hex(bytes.get(i + 2)))
        {
            let byte = (high << 4 | low) as u8;
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                out.push(char::from(byte));
            } else {
                out.push_str(&format!("%{byte:02X}"));
            }
            i += 3;
        } else {
            out.push(char::from(bytes[i]));
            i += 1;
        }
    }
    (out != path).then_some(out)
}

#[derive(Default)]
struct Found {
    raw: Vec<(String, LinkKind)>,
    base: Option<String>,
    nofollow: bool,
}

#[derive(Default)]
struct Sink {
    found: RefCell<Found>,
}

impl TokenSink for Sink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let Token::TagToken(tag) = token else {
            return TokenSinkResult::Continue;
        };
        if tag.kind != TagKind::StartTag {
            return TokenSinkResult::Continue;
        }
        let mut found = self.found.borrow_mut();
        // Stop gathering once there is more than enough: the page is cut.
        if found.raw.len() <= MAX_LINKS_PER_PAGE * 2 {
            read_tag(&tag, &mut found);
        }
        // The tokenizer alone does not switch to the raw text states a
        // browser's tree builder does: text inside these tags is never
        // markup, so `<script>"<a href=x.zip>"</script>` is no link.
        match &*tag.name {
            "title" | "textarea" => TokenSinkResult::RawData(RawKind::Rcdata),
            "style" | "xmp" | "iframe" | "noembed" | "noframes" => {
                TokenSinkResult::RawData(RawKind::Rawtext)
            }
            "script" => TokenSinkResult::RawData(RawKind::ScriptData),
            "plaintext" => TokenSinkResult::Plaintext,
            _ => TokenSinkResult::Continue,
        }
    }
}

fn attr<'a>(tag: &'a Tag, name: &str) -> Option<&'a str> {
    tag.attrs
        .iter()
        .find(|a| &*a.name.local == name)
        .map(|a| &*a.value)
}

fn read_tag(tag: &Tag, found: &mut Found) {
    let mut push = |raw: &str, kind| found.raw.push((raw.to_owned(), kind));
    match &*tag.name {
        "a" | "area" => {
            if let Some(href) = attr(tag, "href") {
                push(href, LinkKind::Either);
            }
        }
        "iframe" | "frame" => {
            if let Some(src) = attr(tag, "src") {
                push(src, LinkKind::Page);
            }
        }
        "img" | "source" => {
            if let Some(src) = attr(tag, "src") {
                push(src, LinkKind::File);
            }
            if let Some(srcset) = attr(tag, "srcset") {
                for url in srcset_urls(srcset) {
                    push(url, LinkKind::File);
                }
            }
        }
        "video" | "audio" | "track" | "embed" => {
            if let Some(src) = attr(tag, "src") {
                push(src, LinkKind::File);
            }
            if &*tag.name == "video"
                && let Some(poster) = attr(tag, "poster")
            {
                push(poster, LinkKind::File);
            }
        }
        "object" => {
            if let Some(data) = attr(tag, "data") {
                push(data, LinkKind::File);
            }
        }
        "link" => {
            let rel = attr(tag, "rel").unwrap_or("").to_ascii_lowercase();
            let wanted = rel
                .split_ascii_whitespace()
                .any(|r| matches!(r, "icon" | "stylesheet" | "alternate"));
            if wanted && let Some(href) = attr(tag, "href") {
                push(href, LinkKind::File);
            }
        }
        "base" => {
            if found.base.is_none()
                && let Some(href) = attr(tag, "href")
            {
                found.base = Some(href.to_owned());
            }
        }
        "meta" => {
            let content = attr(tag, "content").unwrap_or("");
            if attr(tag, "http-equiv").is_some_and(|h| h.eq_ignore_ascii_case("refresh")) {
                if let Some(url) = refresh_url(content) {
                    push(url, LinkKind::Page);
                }
            } else if attr(tag, "name").is_some_and(|n| n.eq_ignore_ascii_case("robots")) {
                let content = content.to_ascii_lowercase();
                if content
                    .split(',')
                    .any(|d| matches!(d.trim(), "nofollow" | "none"))
                {
                    found.nofollow = true;
                }
            }
        }
        _ => {}
    }
}

/// The URLs of a `srcset`: candidates separated by commas, each a URL and
/// optional descriptors (`a.jpg 1x, b.jpg 2x`). A URL may itself hold
/// commas; one that ends with commas loses them (HTML's parsing rules).
fn srcset_urls(srcset: &str) -> Vec<&str> {
    let mut urls = Vec::new();
    let mut rest = srcset;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        if rest.is_empty() {
            return urls;
        }
        let end = rest
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let (url, after) = rest.split_at(end);
        let trimmed = url.trim_end_matches(',');
        if !trimmed.is_empty() {
            urls.push(trimmed);
        }
        rest = if url.ends_with(',') {
            after
        } else {
            // Skip the descriptors, up to the next comma.
            after.find(',').map_or("", |i| &after[i + 1..])
        };
    }
}

/// The URL of a meta refresh `content`: `5; url=page.html`, `0;URL='x'`,
/// `3, x`.
fn refresh_url(content: &str) -> Option<&str> {
    let rest = content.trim_start();
    let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
    let rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let rest = rest.strip_prefix([';', ','])?.trim_start();
    let rest = match rest.get(..3) {
        Some(word) if word.eq_ignore_ascii_case("url") => {
            let after = rest[3..].trim_start();
            after.strip_prefix('=').map_or(rest, str::trim_start)
        }
        _ => rest,
    };
    let url = match rest.chars().next() {
        Some(q @ ('"' | '\'')) => rest[1..].split(q).next().unwrap_or(""),
        _ => rest,
    };
    let url = url.trim();
    (!url.is_empty()).then_some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(html: &str) -> Vec<(String, LinkKind)> {
        let page = Url::parse("https://site.test/dir/page.html").unwrap();
        extract(html, &page)
            .links
            .into_iter()
            .map(|l| (l.url.to_string(), l.kind))
            .collect()
    }

    fn urls(html: &str) -> Vec<String> {
        links(html).into_iter().map(|(u, _)| u).collect()
    }

    /// ADR-0010 section 6: needless percent-encoding is undone and the
    /// rest written with capital hex digits; the query stays as it is.
    #[test]
    fn needless_percent_encoding_is_undone() {
        assert_eq!(
            urls(r#"<a href="s%65lf.html"><a href="%7Euser/a%2fb%c3%a9.zip?q=%65">"#),
            [
                "https://site.test/dir/self.html",
                "https://site.test/dir/~user/a%2Fb%C3%A9.zip?q=%65",
            ]
        );
    }

    #[test]
    fn tags_and_kinds() {
        let found = links(
            r#"<a href="a.zip">x</a><area href="/b/">
               <iframe src="frame.html"></iframe><frame src="f2.html">
               <img src="i.png"><video src="v.mp4" poster="p.jpg"></video>
               <audio src="s.mp3"></audio><track src="t.vtt"><embed src="e.swf">
               <object data="o.pdf"></object><source src="src.webm">
               <link rel="stylesheet" href="s.css"><link rel="preconnect" href="https://x.test/">"#,
        );
        use LinkKind::*;
        let want: Vec<(String, LinkKind)> = [
            ("https://site.test/dir/a.zip", Either),
            ("https://site.test/b/", Either),
            ("https://site.test/dir/frame.html", Page),
            ("https://site.test/dir/f2.html", Page),
            ("https://site.test/dir/i.png", File),
            ("https://site.test/dir/v.mp4", File),
            ("https://site.test/dir/p.jpg", File),
            ("https://site.test/dir/s.mp3", File),
            ("https://site.test/dir/t.vtt", File),
            ("https://site.test/dir/e.swf", File),
            ("https://site.test/dir/o.pdf", File),
            ("https://site.test/dir/src.webm", File),
            ("https://site.test/dir/s.css", File),
        ]
        .into_iter()
        .map(|(u, k)| (u.to_owned(), k))
        .collect();
        assert_eq!(found, want);
    }

    /// The first `<base>` applies to the whole page, even before it.
    #[test]
    fn the_first_base_is_the_base_of_every_link() {
        assert_eq!(
            urls(
                r#"<a href="x.zip"></a><base href="https://cdn.test/files/"><base href="/other/"><a href="y.zip"></a>"#
            ),
            [
                "https://cdn.test/files/x.zip",
                "https://cdn.test/files/y.zip"
            ]
        );
        // A base that is not http or https is ignored.
        assert_eq!(
            urls(r#"<base href="javascript:alert(1)"><a href="x.zip"></a>"#),
            ["https://site.test/dir/x.zip"]
        );
    }

    #[test]
    fn srcset_candidates() {
        assert_eq!(
            urls(
                r#"<img srcset="small.jpg 1x, big.jpg 2x,huge.jpg 3x"><source srcset="a,b.jpg 100w, c.jpg">"#
            ),
            [
                "https://site.test/dir/small.jpg",
                "https://site.test/dir/big.jpg",
                "https://site.test/dir/huge.jpg",
                "https://site.test/dir/a,b.jpg",
                "https://site.test/dir/c.jpg",
            ]
        );
    }

    #[test]
    fn meta_refresh_and_robots() {
        for content in [
            "0; url=next.html",
            "5;URL='next.html'",
            "3, next.html",
            "0;url = 'next.html'",
        ] {
            assert_eq!(
                links(&format!(
                    r#"<meta http-equiv="Refresh" content="{content}">"#
                )),
                [("https://site.test/dir/next.html".to_owned(), LinkKind::Page)],
                "{content}"
            );
        }
        assert!(links(r#"<meta http-equiv="refresh" content="30">"#).is_empty());
        let page = Url::parse("https://site.test/").unwrap();
        assert!(extract(r#"<meta name="ROBOTS" content="noindex, NOFOLLOW">"#, &page).nofollow);
        assert!(extract(r#"<meta name="robots" content="none">"#, &page).nofollow);
        assert!(!extract(r#"<meta name="robots" content="noindex">"#, &page).nofollow);
    }

    /// Text inside raw text tags and comments is never markup.
    #[test]
    fn no_links_from_scripts_textareas_or_comments() {
        let found = urls(
            r#"<script>var s = "<a href='/s.zip'>";</script>
               <style>a { background: url(/bg.png) } </style><title><a href="/t.zip"></title>
               <textarea><a href="/ta.zip"></a></textarea><!-- <a href="/c.zip"> -->
               <noembed><a href="/ne.zip"></noembed><xmp><a href="/x.zip"></xmp>
               <a href="/real.zip">real</a><plaintext><a href="/pt.zip">"#,
        );
        assert_eq!(found, ["https://site.test/real.zip"]);
    }

    #[test]
    fn only_http_links_without_credentials_or_fragments_each_once() {
        assert_eq!(
            urls(
                r##"<a href="javascript:void(0)"></a><a href="mailto:a@b.c"></a>
                   <a href="data:text/plain,hi"></a><a href="file:///etc/passwd"></a>
                   <a href="ftp://h/f"></a><a href="https://u:p@site.test/x"></a>
                   <a href="#top"></a><a href="  a.zip#part  "></a><a href="a.zip"></a>
                   <a href="HTTPS://SITE.TEST/dir/a.zip"></a><a href=""></a>"##
            ),
            [
                "https://site.test/dir/page.html",
                "https://site.test/dir/a.zip"
            ]
        );
    }

    #[test]
    fn broken_html_is_read_as_a_browser_reads_it() {
        assert_eq!(
            urls(
                r#"<A HREF=one.zip><img src=two.png alt="x>y"><a href='three.zip'<a href="four.zip">"#
            ),
            [
                "https://site.test/dir/one.zip",
                "https://site.test/dir/two.png",
                "https://site.test/dir/three.zip",
            ]
        );
    }

    #[test]
    fn a_page_with_too_many_links_is_cut() {
        let html: String = (0..20_000)
            .map(|i| format!("<a href=\"f{i}.zip\"></a>"))
            .collect();
        let page = Url::parse("https://site.test/").unwrap();
        let found = extract(&html, &page);
        assert_eq!(found.links.len(), MAX_LINKS_PER_PAGE);
        assert!(found.truncated);
        assert_eq!(found.links[0].url.as_str(), "https://site.test/f0.zip");
    }

    #[test]
    fn very_long_urls_are_left_out() {
        let long = format!(
            "<a href=\"/{}\"></a><a href=\"/ok\"></a>",
            "a".repeat(MAX_URL_BYTES)
        );
        assert_eq!(urls(&long), ["https://site.test/ok"]);
    }
}
