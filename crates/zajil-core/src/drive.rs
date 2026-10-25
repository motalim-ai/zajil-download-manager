//! Google Drive links: a file's page (`drive.google.com/file/d/<id>/view`,
//! `/open?id=`, `/uc?id=`) is turned into its download link
//! (`drive.usercontent.google.com/download?id=<id>&export=download`), and
//! the page Drive answers for a large file ("can't scan this file for
//! viruses") gives the real link.
//!
//! Only links Drive itself makes are followed: the confirmed link is built
//! again on the download address from the page's form or link, its id must
//! be the file's, and its values are plain. No cookie is ever added: the
//! engine keeps none, and the extension's go only to their own origin
//! ([`crate::browser::FromBrowser`]).

use reqwest::Url;

/// Pages larger than this are not read for a link.
pub(crate) const MAX_PAGE_BYTES: usize = 512 * 1024;

/// The longest value taken from a link or a page.
const MAX_VALUE_CHARS: usize = 512;

/// Parameters a confirmed link may carry; others are dropped.
const CONFIRM_PARAMS: &[&str] = &[
    "id",
    "export",
    "confirm",
    "uuid",
    "at",
    "resourcekey",
    "authuser",
];

/// Where Drive's links and downloads are: the real addresses, or a local
/// test server's (`Engine::with_test_drive`).
#[derive(Debug, Clone)]
pub(crate) struct DriveSite {
    /// Files' pages are under this address (`https://drive.google.com/`).
    links: Url,
    /// The download address
    /// (`https://drive.usercontent.google.com/download`).
    download: Url,
}

impl Default for DriveSite {
    fn default() -> Self {
        DriveSite {
            links: Url::parse("https://drive.google.com/").expect("a valid URL"),
            download: Url::parse("https://drive.usercontent.google.com/download")
                .expect("a valid URL"),
        }
    }
}

impl DriveSite {
    /// Test builds only: Drive at `links` and `download`.
    #[cfg(feature = "test-support")]
    pub(crate) fn new(links: Url, download: Url) -> Self {
        DriveSite { links, download }
    }

    /// The download link of the file whose page `url` is, if it is one.
    pub(crate) fn direct_link(&self, url: &Url) -> Option<Url> {
        let rest = under(url, &self.links)?;
        let segments: Vec<&str> = rest.split('/').collect();
        let id = match segments.as_slice() {
            // `file/d/<id>`, `file/d/<id>/view`, `/edit`, `/preview`…, and
            // `file/u/<n>/d/<id>/…` of a signed-in account.
            ["file", "d", id, ..] | ["file", "u", _, "d", id, ..] => (*id).to_owned(),
            ["open"] | ["uc"] => query(url, "id")?,
            _ => return None,
        };
        if !is_id(&id) {
            return None;
        }
        let mut link = self.download.clone();
        {
            let mut pairs = link.query_pairs_mut();
            pairs
                .clear()
                .append_pair("id", &id)
                .append_pair("export", "download");
            if let Some(key) = query(url, "resourcekey").filter(|k| is_plain(k)) {
                pairs.append_pair("resourcekey", &key);
            }
        }
        Some(link)
    }

    /// Whether `url` is Drive's download address.
    pub(crate) fn is_download(&self, url: &Url) -> bool {
        same_origin(url, &self.download) && url.path() == self.download.path()
    }

    /// The confirmed link on the page Drive answers `asked` with: its form
    /// (`<form action=".../download">` and its hidden fields), or its link
    /// (`<a href="/uc?export=download&confirm=…&id=…">`). `None` when the
    /// page has none for this file.
    pub(crate) fn confirmed_link(&self, page: &str, page_url: &Url, asked: &Url) -> Option<Url> {
        // The file asked for; else the one the page itself is for (a link
        // that came to Drive's address through a redirect).
        let id = query(asked, "id").or_else(|| query(page_url, "id"))?;
        candidates(page)
            .into_iter()
            .find_map(|(action, fields)| self.rebuild(page_url, &action, fields, &id))
    }

    /// The download link for `action` and `fields`, on the download
    /// address, if they are Drive's own for file `id`.
    fn rebuild(
        &self,
        page_url: &Url,
        action: &str,
        mut fields: Vec<(String, String)>,
        id: &str,
    ) -> Option<Url> {
        let target = page_url.join(action).ok()?;
        let drive_uc = under(&target, &self.links).is_some_and(|rest| rest == "uc");
        if !(self.is_download(&target) || drive_uc) {
            return None;
        }
        fields.extend(
            target
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned())),
        );
        let mut kept: Vec<(&str, String)> = Vec::new();
        for (name, value) in fields {
            let Some(name) = CONFIRM_PARAMS.iter().find(|p| **p == name) else {
                continue;
            };
            if !is_plain(&value) || kept.iter().any(|(n, _)| n == name) {
                continue;
            }
            kept.push((name, value));
        }
        let value = |name: &str| {
            kept.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.as_str())
        };
        if value("id") != Some(id) || value("confirm").is_none() {
            return None;
        }
        let mut link = self.download.clone();
        {
            let mut pairs = link.query_pairs_mut();
            pairs.clear();
            for (name, value) in &kept {
                pairs.append_pair(name, value);
            }
        }
        Some(link)
    }
}

/// The part of `url`'s path under `base` (same scheme, host and port), if
/// it is under it.
fn under<'a>(url: &'a Url, base: &Url) -> Option<&'a str> {
    if !same_origin(url, base) {
        return None;
    }
    let rest = url.path().strip_prefix(base.path().trim_end_matches('/'))?;
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    Some(rest.trim_end_matches('/'))
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str().map(str::to_ascii_lowercase) == b.host_str().map(str::to_ascii_lowercase)
        && a.port_or_known_default() == b.port_or_known_default()
}

/// The first value of `name` in `url`'s query.
fn query(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

/// A Drive file id: letters, digits, `-` and `_`.
fn is_id(id: &str) -> bool {
    (10..=MAX_VALUE_CHARS).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// A value of a confirmed link: short, and nothing but what Drive's own
/// values are made of.
fn is_plain(value: &str) -> bool {
    (1..=MAX_VALUE_CHARS).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b':'))
}

/// Every form (its action and its fields) and every link (its address, no
/// fields) on `page`, in order.
fn candidates(page: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut found = Vec::new();
    let mut form: Option<(String, Vec<(String, String)>)> = None;
    for (tag, attributes) in tags(page) {
        match tag.as_str() {
            "form" => {
                found.extend(form.take());
                let action = value_of(&attributes, "action").unwrap_or_default();
                form = Some((action, Vec::new()));
            }
            "/form" => found.extend(form.take()),
            "input" => {
                if let (Some((_, fields)), Some(name)) =
                    (form.as_mut(), value_of(&attributes, "name"))
                {
                    fields.push((name, value_of(&attributes, "value").unwrap_or_default()));
                }
            }
            "a" => {
                if let Some(href) = value_of(&attributes, "href") {
                    found.push((href, Vec::new()));
                }
            }
            _ => {}
        }
    }
    found.extend(form);
    found
}

fn value_of(attributes: &[(String, String)], name: &str) -> Option<String> {
    attributes
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.clone())
}

/// The tags of `page` (name in lower case, `/form` for a closing tag) with
/// their attributes (names in lower case, values with their entities
/// decoded). Comments, scripts and styles are skipped.
fn tags(page: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut tags = Vec::new();
    let mut rest = page;
    while let Some(start) = rest.find('<') {
        rest = &rest[start + 1..];
        if let Some(after) = rest.strip_prefix("!--") {
            rest = after.find("-->").map_or("", |end| &after[end + 3..]);
            continue;
        }
        let name_end = rest
            .find(|c: char| {
                c.is_ascii_whitespace() || c == '>' || c == '/' && !rest.starts_with('/')
            })
            .unwrap_or(rest.len());
        let name = rest[..name_end].to_ascii_lowercase();
        let (attributes, after) = attributes(&rest[name_end..]);
        rest = after;
        if name == "script" || name == "style" {
            let close = format!("</{name}");
            rest = find_ignoring_case(rest, &close).map_or("", |end| &rest[end..]);
        }
        if !name.is_empty() {
            tags.push((name, attributes));
        }
    }
    tags
}

fn find_ignoring_case(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// The attributes of a tag, up to its `>`, and what follows it.
fn attributes(mut rest: &str) -> (Vec<(String, String)>, &str) {
    let mut found = Vec::new();
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '/');
        if rest.is_empty() {
            return (found, rest);
        }
        if let Some(after) = rest.strip_prefix('>') {
            return (found, after);
        }
        let name_end = rest
            .find(|c: char| c.is_ascii_whitespace() || matches!(c, '=' | '>' | '/'))
            .unwrap_or(rest.len());
        // A stray character (such as a lone `=`) is passed over.
        let name_end = name_end.max(rest.chars().next().map_or(0, char::len_utf8));
        let name = rest[..name_end].to_ascii_lowercase();
        rest = rest[name_end..].trim_start_matches(|c: char| c.is_ascii_whitespace());
        let mut value = String::new();
        if let Some(after) = rest.strip_prefix('=') {
            rest = after.trim_start_matches(|c: char| c.is_ascii_whitespace());
            let (raw, after) = match rest.chars().next() {
                Some(quote @ ('"' | '\'')) => {
                    let inner = &rest[1..];
                    match inner.find(quote) {
                        Some(end) => (&inner[..end], &inner[end + 1..]),
                        None => (inner, ""),
                    }
                }
                _ => {
                    let end = rest
                        .find(|c: char| c.is_ascii_whitespace() || c == '>')
                        .unwrap_or(rest.len());
                    (&rest[..end], &rest[end..])
                }
            };
            value = decode_entities(raw);
            rest = after;
        }
        found.push((name, value));
    }
}

/// `&amp;`, `&quot;`, `&#39;`, `&lt;`, `&gt;` and numeric entities; any
/// other `&` stays as it is.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        // Only the next few bytes: a long run of `&` stays linear.
        let window = &rest.as_bytes()[..rest.len().min(13)];
        let Some(end) = window.iter().position(|&b| b == b';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "lt" => Some('<'),
            "gt" => Some('>'),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "1AbC-dEf_GhIjKlMnOpQrStUvWxYz0123";

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn direct() -> String {
        format!("https://drive.usercontent.google.com/download?id={ID}&export=download")
    }

    #[test]
    fn a_file_s_page_becomes_its_download_link() {
        let drive = DriveSite::default();
        for link in [
            format!("https://drive.google.com/file/d/{ID}/view"),
            format!("https://drive.google.com/file/d/{ID}/view?usp=sharing"),
            format!("https://drive.google.com/file/d/{ID}"),
            format!("https://drive.google.com/file/d/{ID}/edit"),
            format!("https://drive.google.com/file/u/0/d/{ID}/view"),
            format!("https://DRIVE.google.com/open?id={ID}"),
            format!("https://drive.google.com/uc?id={ID}"),
            format!("https://drive.google.com/uc?export=download&id={ID}"),
        ] {
            let got = drive.direct_link(&url(&link)).map(|u| u.to_string());
            assert_eq!(got.as_deref(), Some(direct().as_str()), "{link}");
        }
        let with_key = format!("https://drive.google.com/file/d/{ID}/view?resourcekey=0-AbC_d");
        assert_eq!(
            drive.direct_link(&url(&with_key)).unwrap().as_str(),
            format!("{}&resourcekey=0-AbC_d", direct())
        );
    }

    #[test]
    fn other_links_are_left_alone() {
        let drive = DriveSite::default();
        for link in [
            format!("http://drive.google.com/file/d/{ID}/view"),
            format!("https://drive.google.com.evil.example/file/d/{ID}/view"),
            format!("https://evil.example/file/d/{ID}/view"),
            format!("https://drive.google.com:8443/file/d/{ID}/view"),
            "https://drive.google.com/drive/folders/1AbCdEfGhIjKlMn".to_owned(),
            "https://drive.google.com/file/d/short/view".to_owned(),
            "https://drive.google.com/file/d/a%2F..%2Fb1234567890/view".to_owned(),
            "https://drive.google.com/open?id=has%20space123".to_owned(),
            "https://drive.google.com/uc".to_owned(),
            direct(),
        ] {
            assert_eq!(drive.direct_link(&url(&link)), None, "{link}");
        }
    }

    /// The page Drive answers a large file with (2024 and later): a form.
    #[test]
    fn the_confirm_form_gives_the_real_link() {
        let drive = DriveSite::default();
        let page = format!(
            r#"<!DOCTYPE html><html><head><title>Google Drive - Virus scan warning</title>
            <script>var x = "<form action='https://evil.example/download'>";</script></head>
            <body><!-- <a href="https://evil.example/uc?id=x&confirm=t"> -->
            <form id="download-form" action="https://drive.usercontent.google.com/download" method="get">
            <input type="submit" id="uc-download-link" class="goog-inline-block jfk-button" value="Download anyway"/>
            <input type="hidden" name="id" value="{ID}"><input type="hidden" name="export" value="download">
            <input type="hidden" name="confirm" value="t"><input type="hidden" name="uuid" value="4f1c2d3e-aaaa-bbbb-cccc-0123456789ab">
            <input type="hidden" name="evil" value="x"></form></body></html>"#
        );
        let asked = url(&direct());
        let got = drive.confirmed_link(&page, &asked, &asked).unwrap();
        assert_eq!(
            got.as_str(),
            format!(
                "https://drive.usercontent.google.com/download?id={ID}&export=download&confirm=t&uuid=4f1c2d3e-aaaa-bbbb-cccc-0123456789ab"
            )
        );
    }

    /// The older page: a link on drive.google.com, with `&amp;`.
    #[test]
    fn the_older_confirm_link_gives_the_real_link() {
        let drive = DriveSite::default();
        let page = format!(
            r#"<a id="uc-download-link" class="x" href="/uc?export=download&amp;confirm=AbCd&amp;id={ID}">Download anyway</a>"#
        );
        let asked = url(&direct());
        let page_url = url(&format!(
            "https://drive.google.com/uc?id={ID}&export=download"
        ));
        let got = drive.confirmed_link(&page, &page_url, &asked).unwrap();
        assert_eq!(
            got.as_str(),
            format!(
                "https://drive.usercontent.google.com/download?export=download&confirm=AbCd&id={ID}"
            )
        );
    }

    /// Nothing but Drive's own link for this file is taken: another host,
    /// another file, no `confirm`, odd values.
    #[test]
    fn a_page_without_drive_s_own_link_gives_none() {
        let drive = DriveSite::default();
        let asked = url(&direct());
        let form = |action: &str, id: &str, confirm: &str| {
            format!(
                r#"<form action="{action}"><input type="hidden" name="id" value="{id}"><input name="confirm" value="{confirm}"></form>"#
            )
        };
        for page in [
            form("https://evil.example/download", ID, "t"),
            form("http://drive.usercontent.google.com/download", ID, "t"),
            form("https://drive.usercontent.google.com/other", ID, "t"),
            form(
                "https://drive.usercontent.google.com/download",
                "1OtherFileId0123456789",
                "t",
            ),
            form("https://drive.usercontent.google.com/download", ID, ""),
            form(
                "https://drive.usercontent.google.com/download",
                ID,
                "t&lt;x",
            ),
            form("https://drive.usercontent.google.com/download", ID, "a b"),
            format!(r#"<a href="https://evil.example/uc?id={ID}&confirm=t">x</a>"#),
            format!(r#"<a href="//evil.example/download?id={ID}&confirm=t">x</a>"#),
            "<html><body>Sign in</body></html>".to_owned(),
            "<form action=".to_owned(),
            "<<<>>>&&&;;;<a href='".to_owned(),
            String::new(),
        ] {
            assert_eq!(drive.confirmed_link(&page, &asked, &asked), None, "{page}");
        }
    }

    #[test]
    fn entities_are_decoded_and_bad_ones_kept() {
        assert_eq!(decode_entities("a&amp;b&#38;c&#x26;d&quot;"), "a&b&c&d\"");
        assert_eq!(
            decode_entities("a&b; &zz; &#xFFFFFFFF; &"),
            "a&b; &zz; &#xFFFFFFFF; &"
        );
        // Never cut inside a character.
        assert_eq!(decode_entities("&عربيعربي;"), "&عربيعربي;");
    }
}
