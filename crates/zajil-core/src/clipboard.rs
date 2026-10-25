//! Clipboard monitoring (M5, off unless the user turns it on): which copied
//! text is a download link worth offering, and noticing each copy once.
//! The app reads the clipboard (`zajil_platform::clipboard`); the rule is
//! here, the same on every system and tested without a clipboard.
//!
//! The text itself is never kept: [`Watch`] remembers a hash of the last
//! text seen, keyed afresh for each run so it cannot be looked up, and only
//! a link it offers leaves this module.

use std::hash::{BuildHasher, RandomState};

pub use zajil_platform::clipboard::Content;

use crate::category::CategoriesFile;

/// The longest text looked at, as the extension's limit for a URL: longer
/// text is not a link Zajil would add.
pub const MAX_TEXT: usize = 8 * 1024;

/// The link to offer for `text` the user copied: the whole text, without
/// the spaces around it, is one URL that adding to a queue accepts (http or
/// https, no user name or password, no control characters or direction
/// marks), and the last part of its path has the extension of one of
/// `categories` (`.zip`, `.mp4`, …). A page, a sentence with a link in it,
/// or several links are not offered.
pub fn link_to_offer(text: &str, categories: &CategoriesFile) -> Option<String> {
    if text.len() > MAX_TEXT {
        return None;
    }
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        return None;
    }
    crate::queue::check_url(text).ok()?;
    let url = crate::Url::parse(text).ok()?;
    let name = url.path_segments()?.next_back()?;
    categories.classify(name, None)?;
    Some(text.to_owned())
}

/// Each copy looked at once: what the clipboard held is remembered as a
/// hash, so the same text read again is not offered again.
#[derive(Debug, Default)]
pub struct Watch {
    last: Option<u64>,
    /// Random keys for this run (SipHash), so a hash says nothing of a
    /// short text such as a password.
    keys: RandomState,
}

impl Watch {
    /// What the clipboard holds, read now (text longer than [`MAX_TEXT`]
    /// as [`Content::Longer`]). Returns the link to offer when the content
    /// is new and [`link_to_offer`] accepts it; the categories are asked
    /// for only then. The first content seen is only remembered: what was
    /// on the clipboard when monitoring started is not offered.
    pub fn seen(
        &mut self,
        content: &Content,
        categories: impl FnOnce() -> Option<CategoriesFile>,
    ) -> Option<String> {
        let hash = self.keys.hash_one(content);
        let first = self.last.is_none();
        let changed = self.last != Some(hash);
        self.last = Some(hash);
        if first || !changed {
            return None;
        }
        match content {
            Content::Text(text) => link_to_offer(text, &categories()?),
            Content::Empty | Content::Longer => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built_in() -> CategoriesFile {
        CategoriesFile::built_in()
    }

    #[test]
    fn a_link_to_a_file_of_a_category_is_offered() {
        for text in [
            "https://example.com/files/setup.exe",
            "http://example.com/a/b/Movie.MKV",
            "  https://example.com/x.zip\n",
            "https://example.com/report.pdf?token=abc#page=2",
            "https://cdn.example.com/%D8%B2%D8%A7%D8%AC%D9%84.mp3",
        ] {
            assert_eq!(
                link_to_offer(text, &built_in()).as_deref(),
                Some(text.trim()),
                "{text:?}"
            );
        }
    }

    #[test]
    fn anything_else_is_not_offered() {
        let long = format!("https://example.com/{}.zip", "a".repeat(MAX_TEXT));
        for text in [
            "",
            "   ",
            "hello",
            "https://example.com/",
            "https://example.com/page.html",
            "https://example.com/folder/",
            "https://example.com/noextension",
            "see https://example.com/x.zip",
            "https://example.com/a.zip https://example.com/b.zip",
            "https://example.com/a.zip\nhttps://example.com/b.zip",
            "ftp://example.com/x.zip",
            "file:///home/u/x.zip",
            "https://user:secret@example.com/x.zip",
            "https://example.com/\u{202e}piz.exe",
            "my password is hunter2.zip",
            long.as_str(),
        ] {
            assert_eq!(link_to_offer(text, &built_in()), None, "{text:?}");
        }
    }

    /// A category the user added counts; one removed does not.
    #[test]
    fn the_user_s_categories_decide() {
        let mut categories = built_in();
        let url = "https://example.com/model.stl";
        assert_eq!(link_to_offer(url, &categories), None);
        categories.add("models", "Models").unwrap();
        categories.get_mut("models").unwrap().extensions = vec!["stl".into()];
        assert_eq!(link_to_offer(url, &categories).as_deref(), Some(url));
        categories.remove("archives").unwrap();
        assert_eq!(
            link_to_offer("https://example.com/x.zip", &categories),
            None
        );
    }

    fn text(text: &str) -> Content {
        Content::Text(text.to_owned())
    }

    #[test]
    fn each_copy_is_offered_once_and_not_what_was_there_first() {
        let mut watch = Watch::default();
        let cats = || Some(built_in());
        let a = "https://example.com/a.zip";
        let b = "https://example.com/b.zip";
        // On the clipboard when monitoring started: remembered only.
        assert_eq!(watch.seen(&text(a), cats), None);
        assert_eq!(watch.seen(&text(a), cats), None);
        assert_eq!(watch.seen(&text(b), cats).as_deref(), Some(b));
        assert_eq!(watch.seen(&text(b), cats), None);
        assert_eq!(watch.seen(&text("some text"), cats), None);
        // Copied again after something else: a new copy.
        assert_eq!(watch.seen(&text(b), cats).as_deref(), Some(b));
        assert_eq!(watch.seen(&Content::Empty, cats), None);
        assert_eq!(watch.seen(&text(a), cats).as_deref(), Some(a));
        assert_eq!(watch.seen(&Content::Longer, cats), None);
        assert_eq!(watch.seen(&text(a), cats).as_deref(), Some(a));
        // Starting again (monitoring turned off and on): what is there is
        // remembered, not offered.
        let mut watch = Watch::default();
        assert_eq!(watch.seen(&text(b), cats), None);
    }

    /// The categories are read only for a new text, and a damaged
    /// categories file offers nothing.
    #[test]
    fn categories_are_read_only_for_a_new_text() {
        let mut watch = Watch::default();
        watch.seen(&text("x"), || panic!("not for the first text"));
        watch.seen(&text("x"), || panic!("not for the same text"));
        watch.seen(&Content::Longer, || panic!("not for a text too long"));
        assert_eq!(
            watch.seen(&text("https://example.com/a.zip"), || None),
            None
        );
    }

    /// The hash is keyed for each run: the same text hashes differently in
    /// two watches, so a remembered hash cannot be looked up in a table.
    #[test]
    fn the_hash_is_keyed_for_each_run() {
        let (mut one, mut two) = (Watch::default(), Watch::default());
        one.seen(&text("k7#q"), || None);
        two.seen(&text("k7#q"), || None);
        assert_ne!(one.last, two.last);
    }
}
