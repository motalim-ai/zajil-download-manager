//! The clipboard: monitoring it for download links (M5), and «Copy
//! link».

use std::sync::atomic::Ordering;

use tauri::{Emitter, Manager, State};
use zajil_core::category::CategoryStore;

use crate::{App, blocking_err, in_scene, tray, video};

/// How often the clipboard is looked at while monitoring is on.
const CLIPBOARD_EVERY: std::time::Duration = std::time::Duration::from_millis(500);

/// The least time between two links offered: a program changing the
/// clipboard all the time cannot raise the window twice a second.
const CLIPBOARD_OFFER_GAP: std::time::Duration = std::time::Duration::from_secs(2);

/// Clipboard monitoring (M5, off unless the user turns it on): while the
/// setting is on, the clipboard is looked at twice a second (and read only
/// after a change where the system says so), and a download link the user
/// copies goes to the page (`clipboard-link`). The page puts it in the
/// paste bar and shows the window, unless the add dialog is open: **no
/// request reaches the link before the user presses "Download"**, and
/// nothing is added before the user agrees. While it is off the clipboard
/// is not opened; in a ui-test scene it stays off.
pub(crate) fn watch_clipboard<R: tauri::Runtime>(handle: tauri::AppHandle<R>) {
    std::thread::spawn(move || {
        let mut watcher = ClipboardWatcher::default();
        loop {
            std::thread::sleep(CLIPBOARD_EVERY);
            let app = handle.state::<App>();
            let on = app.clipboard.load(Ordering::Relaxed) && !in_scene(&handle);
            let link = watcher.tick(
                on,
                zajil_platform::clipboard::SystemClipboard::new,
                &app.categories,
                std::time::Instant::now(),
            );
            let own = |link: &str| app.copied.lock().is_ok_and(|c| copied_here(&c, link));
            if let Some(link) = link.filter(|l| !own(l)) {
                let _ = handle.emit("clipboard-link", link);
            }
        }
    });
}

/// Clipboard monitoring between two reads.
struct ClipboardWatcher<C> {
    /// Open while monitoring is on.
    clipboard: Option<C>,
    watch: zajil_core::clipboard::Watch,
    /// When the last link was offered.
    offered: Option<std::time::Instant>,
    /// The last problem said, so a clipboard that cannot be read is not
    /// reported twice a second.
    said: Option<String>,
}

impl<C> Default for ClipboardWatcher<C> {
    fn default() -> Self {
        ClipboardWatcher {
            clipboard: None,
            watch: Default::default(),
            offered: None,
            said: None,
        }
    }
}

impl<C: zajil_platform::clipboard::Clipboard> ClipboardWatcher<C> {
    /// One look at `now`: the link to offer, if the user copied one since
    /// the last. An unchanged clipboard is not read; text longer than
    /// `MAX_TEXT` is not kept; a link copied within
    /// [`CLIPBOARD_OFFER_GAP`] of the last one offered is not offered.
    /// `on` false closes the clipboard and forgets what was seen, so what
    /// is on the clipboard when monitoring is turned on again is not
    /// offered. A clipboard that cannot be opened or read (another program
    /// holding it, no display) is tried again at the next look.
    fn tick(
        &mut self,
        on: bool,
        open: impl FnOnce() -> std::io::Result<C>,
        categories: &CategoryStore,
        now: std::time::Instant,
    ) -> Option<String> {
        if !on {
            *self = ClipboardWatcher::default();
            return None;
        }
        if self.clipboard.is_none() {
            match open() {
                Ok(clipboard) => self.clipboard = Some(clipboard),
                Err(err) => return self.say(err),
            }
        }
        let clipboard = self.clipboard.as_mut()?;
        if !clipboard.changed() {
            return None;
        }
        let content = match clipboard.text(zajil_core::clipboard::MAX_TEXT) {
            Ok(content) => content,
            Err(err) => {
                self.clipboard = None;
                return self.say(err);
            }
        };
        self.said = None;
        let link = self.watch.seen(&content, || categories.load().ok())?;
        if self
            .offered
            .is_some_and(|last| now.duration_since(last) < CLIPBOARD_OFFER_GAP)
        {
            return None;
        }
        self.offered = Some(now);
        Some(link)
    }

    fn say(&mut self, err: std::io::Error) -> Option<String> {
        let said = err.to_string();
        if self.said.as_ref() != Some(&said) {
            eprintln!("zajil: clipboard monitoring: {said}");
            self.said = Some(said);
        }
        None
    }
}

/// Whether `link` is one of the links «Copy link» put on the clipboard.
fn copied_here(copied: &str, link: &str) -> bool {
    copied.lines().any(|l| l.trim() == link.trim())
}

/// «Copy link»: the downloads' links on the clipboard, one a line, kept
/// so the clipboard monitoring does not offer them back. How many went.
#[tauri::command]
pub(crate) async fn copy_links<R: tauri::Runtime>(
    app: State<'_, App>,
    ids: Vec<u64>,
    handle: tauri::AppHandle<R>,
) -> Result<usize, String> {
    let store = app.store.clone();
    let queues = tokio::task::spawn_blocking(move || store.load())
        .await
        .map_err(blocking_err)?
        .map_err(|e| e.to_string())?;
    let links: Vec<String> = ids
        .iter()
        .filter_map(|&id| {
            if video::is_video(id) {
                video::url_of(&handle.state::<video::VideoState>(), id)
            } else {
                queues.item(id).map(|i| i.url.clone())
            }
        })
        .collect();
    if links.is_empty() || in_scene(&handle) {
        return Ok(links.len());
    }
    copy_text(&app, links.join("\n")).await?;
    Ok(links.len())
}

/// `text` on the clipboard, kept so the clipboard monitoring does not
/// offer it back.
pub(crate) async fn copy_text(app: &App, text: String) -> Result<(), String> {
    if let Ok(mut copied) = app.copied.lock() {
        copied.clone_from(&text);
    }
    let copier = app.copier.clone();
    tokio::task::spawn_blocking(move || {
        let mut copier = copier
            .lock()
            .map_err(|_| std::io::Error::other("clipboard"))?;
        if copier.is_none() {
            *copier = Some(zajil_platform::clipboard::SystemClipboard::new()?);
        }
        copier.as_mut().map_or(Ok(()), |c| c.set_text(&text))
    })
    .await
    .map_err(blocking_err)?
    .map_err(|e| e.to_string())
}

/// Shows the window and brings it to the front: the page asks when it has
/// taken a link from the clipboard.
#[tauri::command]
pub(crate) fn show_window<R: tauri::Runtime>(handle: tauri::AppHandle<R>) {
    tray::show_window(&handle);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_copied_by_the_app_is_not_offered_back() {
        let copied = "https://a.example/x.zip\nhttps://b.example/y.iso";
        assert!(copied_here(copied, "https://b.example/y.iso"));
        assert!(copied_here(copied, " https://a.example/x.zip "));
        assert!(!copied_here(copied, "https://a.example/x"));
        assert!(!copied_here("", "https://a.example/x.zip"));
    }

    /// A clipboard for tests: what each look finds, in turn (`None`: no
    /// change since the last look).
    struct FakeClipboard(
        std::collections::VecDeque<std::io::Result<Option<zajil_core::clipboard::Content>>>,
        Option<zajil_core::clipboard::Content>,
    );

    impl zajil_platform::clipboard::Clipboard for FakeClipboard {
        fn changed(&mut self) -> bool {
            match self.0.front() {
                Some(Ok(None)) => {
                    self.0.pop_front();
                    false
                }
                _ => true,
            }
        }

        fn text(&mut self, _max: usize) -> std::io::Result<zajil_core::clipboard::Content> {
            match self.0.pop_front() {
                Some(Ok(Some(content))) => {
                    self.1 = Some(content.clone());
                    Ok(content)
                }
                Some(Err(err)) => Err(err),
                _ => Ok(self
                    .1
                    .clone()
                    .unwrap_or(zajil_core::clipboard::Content::Empty)),
            }
        }
    }

    /// M5 clipboard monitoring: a copied download link is offered once;
    /// what was there when monitoring started, other text, text too long
    /// and links to pages are not; an unchanged clipboard is not read; two
    /// links closer than the gap give one offer; off, the clipboard is not
    /// even opened, and turned on again what is there is not offered; a
    /// read that fails is tried again.
    #[test]
    fn clipboard_monitoring_offers_each_copied_download_link_once() {
        use zajil_core::clipboard::Content;
        let data = tempfile::tempdir().unwrap();
        let categories = CategoryStore::new(data.path());
        let a = "https://example.com/a.zip";
        let b = "https://example.com/b.mp4";
        let t = |s: &str| Ok(Some(Content::Text(s.to_owned())));
        let start = std::time::Instant::now();
        let at = |seconds: u64| start + std::time::Duration::from_secs(seconds * 3);
        let mut watcher = ClipboardWatcher::default();
        let mut opened = 0;
        let mut open = || {
            opened += 1;
            Ok(FakeClipboard(
                [
                    t(a),
                    Ok(None),
                    t("https://example.com/page.html"),
                    t("hello"),
                    Ok(Some(Content::Longer)),
                    t(b),
                    Ok(None),
                    Ok(Some(Content::Empty)),
                    t(a),
                ]
                .into(),
                None,
            ))
        };
        let mut seen = Vec::new();
        for i in 0..9 {
            seen.push(watcher.tick(true, &mut open, &categories, at(i)));
        }
        assert_eq!(opened, 1);
        assert_eq!(
            seen,
            [
                None,
                None,
                None,
                None,
                None,
                Some(b.to_owned()),
                None,
                None,
                Some(a.to_owned())
            ]
        );

        // Two links one second apart: the second is not offered.
        let mut watcher = ClipboardWatcher::default();
        let reads = FakeClipboard([t("x"), t(a), t(b)].into(), None);
        watcher.clipboard = Some(reads);
        let never = || -> std::io::Result<FakeClipboard> { unreachable!() };
        assert_eq!(watcher.tick(true, never, &categories, start), None);
        assert_eq!(
            watcher.tick(true, never, &categories, start).as_deref(),
            Some(a)
        );
        let soon = start + std::time::Duration::from_secs(1);
        assert_eq!(watcher.tick(true, never, &categories, soon), None);

        // Off: not opened, nothing offered; on again, what is there is
        // remembered only.
        let never = || -> std::io::Result<FakeClipboard> { panic!("opened while off") };
        assert_eq!(watcher.tick(false, never, &categories, start), None);
        assert!(watcher.clipboard.is_none());
        let mut watcher = ClipboardWatcher::default();
        let reads = || Ok(FakeClipboard([t(b)].into(), None));
        assert_eq!(watcher.tick(true, reads, &categories, at(0)), None);
        assert_eq!(
            watcher.tick(true, || unreachable!(), &categories, at(1)),
            None
        );

        // A clipboard that cannot be opened, then one whose read fails:
        // both tried again, and the link copied meanwhile is offered.
        let mut watcher = ClipboardWatcher::default();
        let busy = || std::io::Error::other("the clipboard is held by another program");
        assert_eq!(watcher.tick(true, || Err(busy()), &categories, at(0)), None);
        let empty = || Ok(FakeClipboard([Ok(Some(Content::Empty))].into(), None));
        assert_eq!(watcher.tick(true, empty, &categories, at(1)), None);
        watcher.clipboard = Some(FakeClipboard([Err(busy())].into(), None));
        assert_eq!(
            watcher.tick(true, || unreachable!(), &categories, at(2)),
            None
        );
        assert!(watcher.clipboard.is_none(), "closed to be opened again");
        let link = || Ok(FakeClipboard([t(a)].into(), None));
        assert_eq!(
            watcher.tick(true, link, &categories, at(3)).as_deref(),
            Some(a)
        );
    }
}
