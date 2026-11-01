//! The browser extension's requests (ADR-0005, M5): served on the
//! user's private channel, checked, and added to the queues.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tauri::{Emitter, Manager};
use zajil_core::queue::{QueueStore, StoreError};
use zajil_core::{Engine, Stop};

use crate::asks::when_linked;
use crate::downloads::{add_blocking, download_dir};
use crate::runner::discard_private;
use crate::view::AddRequest;
use crate::{App, in_scene, offers, tray, update, video};

/// The browser extension's requests, handed over by `zajil-host` on the
/// user's private channel (ADR-0005, M5), served by `zajil_host::serve`.
/// A second app on the same data folder cannot open the channel: it says
/// so and goes on without it.
pub(crate) fn serve_extension<R: tauri::Runtime>(handle: tauri::AppHandle<R>, data: PathBuf) {
    let listener = match zajil_platform::ipc::Listener::bind(&data) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("zajil: the browser extension cannot reach this window: {err}");
            return;
        }
    };
    let (links, asks) = {
        let app = handle.state::<App>();
        (app.links.clone(), app.asks.clone())
    };
    when_linked(&links, &asks, &handle.state::<App>().store);
    let ask_links = links.clone();
    std::thread::spawn(move || {
        zajil_host::serve(
            listener,
            move |request| {
                // In a ui-test scene, nothing reaches the queues.
                if in_scene(&handle) {
                    return zajil_host::Response::refused("a ui-test scene adds nothing");
                }
                let app = handle.state::<App>();
                let given = download_dir(&app.settings, &app.system_download_dir);
                let reach = extension_reach(&app.engine);
                // The user's language, for what the extension is told.
                let lang = app.settings.load().map(|s| s.language).unwrap_or_default();
                // A video opens «Video quality» with its page, that page's
                // cookies and the video's own link (M6, #149): the
                // extension is told it was taken.
                if let zajil_host::Request::Video {
                    page,
                    media,
                    cookies,
                    private,
                    format,
                    audio,
                    language,
                    label,
                    size,
                    ..
                } = &request
                {
                    // A format chosen in the button's list: downloaded at
                    // once, without «Video quality» (owner, 2026-10-29).
                    // A dub's language goes with its sound's itag
                    // (`140-ar`), which `by_itag` reads.
                    let sound = |itag: String| match language {
                        Some(language) => format!("{itag}-{language}"),
                        None => itag,
                    };
                    let chosen = format.clone().map(|id| video::Chosen {
                        choice: match audio.clone() {
                            Some(audio) => video::ChoiceView::Merge {
                                video: id,
                                audio: sound(audio),
                            },
                            None => video::ChoiceView::Format { id: sound(id) },
                        },
                        label: label.clone(),
                        size: *size,
                    });
                    video::receive(
                        &handle,
                        page.clone(),
                        media.clone(),
                        cookies.clone(),
                        *private,
                        chosen,
                    );
                    return zajil_host::Response::ok();
                }
                // The button showed over a video: its page is read now, in
                // the background, so its qualities wait for the press.
                if let zajil_host::Request::PeekVideo {
                    page,
                    media,
                    cookies,
                    ..
                } = &request
                {
                    video::read_ahead(&handle, page.clone(), media.clone(), cookies.clone());
                    return zajil_host::Response::ok();
                }
                // The extension is older than this Zajil needs: the main
                // window asks the user to update it.
                if let zajil_host::Request::ExtensionOld { version, .. } = &request {
                    update::extension_old(&handle, version);
                    return zajil_host::Response::ok();
                }
                // The last private window closed: its videos leave the list
                // too (the queues' downloads go below).
                if let zajil_host::Request::PrivateClosed { .. } = &request {
                    video::discard_private(&handle);
                }
                // Zajil started again on this data folder (macOS, #60).
                if let zajil_host::Request::ShowWindow { .. } = &request {
                    tray::show_window(&handle);
                    return zajil_host::Response::ok();
                }
                // A Drive file's download address's cookies, asked of the
                // browser the download came from, on its link (the
                // extension has its ticket already).
                let drive_cookies = |file: &str| {
                    let (instance, store, ticket) = drive_ask_of(&request)?;
                    ask_links
                        .ask(instance, file, store, ticket, DRIVE_ASK_PATIENCE)
                        .flatten()
                };
                // A download opens its own «Download file info», as in IDM,
                // and the main window stays as it is (`offers.rs`).
                if let zajil_host::Request::Download { .. } = &request {
                    return offers::from_browser(&handle, &request, lang, drive_cookies);
                }
                let answer = from_extension_with(
                    &app.store,
                    &request,
                    &given,
                    &app.from_browser,
                    lang,
                    reach,
                    drive_cookies,
                );
                if answer.ok {
                    let _ = handle.emit("downloads-changed", ());
                    tray::show_window(&handle);
                }
                answer
            },
            links,
        );
    });
}

/// How long a download from the browser waits for that browser's cookies
/// of a Google Drive file's download address: the extension is waiting
/// for Zajil's answer meanwhile, and the host gives up on it after 10 s,
/// the check of the file included (review of the Drive links).
const DRIVE_ASK_PATIENCE: Duration = Duration::from_secs(2);

/// How long the app may take to reach the file before it answers the
/// extension: within `zajil_host::CHANNEL_TIMEOUT` (10 s), so the host
/// still has the answer.
const EXTENSION_REACH: Duration = Duration::from_secs(7);

/// The check of a download from the extension: the engine, with the
/// browser's cookies and page for this download.
pub(crate) fn extension_reach(
    engine: &Engine,
) -> impl Fn(&str, &zajil_core::FromBrowser) -> Result<zajil_core::Probe, String> + '_ {
    move |url, from| reach_file(&engine.with_browser(from.clone()), url)
}

/// What the engine reaches at `url` now, as the add dialog does (the first
/// response's headers only), within [`EXTENSION_REACH`].
fn reach_file(engine: &Engine, url: &str) -> Result<zajil_core::Probe, String> {
    let stop = Stop::new();
    tauri::async_runtime::block_on(async {
        match tokio::time::timeout(EXTENSION_REACH, engine.probe_until(url, &stop)).await {
            Ok(Ok(probe)) => Ok(probe),
            Ok(Err(err)) => Err(err.to_string()),
            Err(_) => {
                stop.request();
                Err("the server did not answer in time".to_owned())
            }
        }
    })
}

/// Why what Zajil reached is not the file the browser was downloading: a
/// web page (a sign-in page a link redirected to, review of #70), or a file
/// of another type than the name the browser had.
fn not_the_file(probe: &zajil_core::Probe, browser_name: Option<&str>) -> Option<String> {
    if probe.is_page() {
        return Some("the server sent a web page (a sign-in page?), not the file".into());
    }
    let extension = |name: &str| {
        name.rsplit_once('.')
            .map(|(_, ext)| ext.to_lowercase())
            .filter(|ext| !ext.is_empty())
    };
    let wanted = browser_name.and_then(extension)?;
    (extension(&probe.name).as_deref() != Some(wanted.as_str())).then(|| {
        format!(
            "the server sent another file ({}), not a .{wanted}",
            probe.name
        )
    })
}

/// What a request from the extension does: a download goes into the
/// default queue, by category, as the add dialog adds one, **once Zajil
/// has reached the file itself** (`reach`), and it is that file (not a web
/// page, the same extension as the browser's name): otherwise it is
/// refused, and the extension leaves it to the browser (review of #70: a
/// file behind a login, a referrer check or a one-time link would vanish
/// from the browser and then fail in Zajil). The cookies (sent only when
/// the user allowed them in the extension) and the page go with the check
/// and the download, from memory (`App::from_browser`).
#[cfg(test)]
fn from_extension(
    store: &QueueStore,
    request: &zajil_host::Request,
    given: &Path,
    headers: &zajil_core::queue::FromBrowserById,
    lang: zajil_core::settings::Language,
    reach: impl FnOnce(&str, &zajil_core::FromBrowser) -> Result<zajil_core::Probe, String>,
) -> zajil_host::Response {
    from_extension_with(store, request, given, headers, lang, reach, |_| None)
}

/// Who to ask for a Drive file's download address's cookies, for a
/// download from the browser: its extension instance, cookie store and
/// ticket. None for a private window's download (nothing of it is asked
/// again), or from an extension too old to have a ticket.
fn drive_ask_of(request: &zajil_host::Request) -> Option<(&str, Option<&str>, &str)> {
    match request {
        zajil_host::Request::Download {
            instance: Some(instance),
            ticket: Some(ticket),
            store,
            private: false,
            ..
        } => Some((instance, store.as_deref(), ticket)),
        _ => None,
    }
}

/// `from` for `url`, with the cookies `ask` gives for its Google Drive
/// download address when `url` is a Drive file's link; nothing is asked
/// for another link.
pub(crate) fn with_drive_cookies(
    from: zajil_core::FromBrowser,
    url: &str,
    ask: impl FnOnce(&str) -> Option<String>,
) -> zajil_core::FromBrowser {
    match zajil_core::drive::download_link(url) {
        Some(file) => {
            let more = ask(&file);
            from.and_cookies_for(&file, more.as_deref())
        }
        None => from,
    }
}

/// [`from_extension`], with `drive_cookies`: the browser's cookies for a
/// Google Drive file's download address (owner's decision on the Drive
/// links: the browser is asked for that address's own cookies, never
/// another site's), asked only when the download came with cookies.
#[allow(clippy::too_many_arguments)]
fn from_extension_with(
    store: &QueueStore,
    request: &zajil_host::Request,
    given: &Path,
    headers: &zajil_core::queue::FromBrowserById,
    lang: zajil_core::settings::Language,
    reach: impl FnOnce(&str, &zajil_core::FromBrowser) -> Result<zajil_core::Probe, String>,
    drive_cookies: impl FnOnce(&str) -> Option<String>,
) -> zajil_host::Response {
    match request {
        zajil_host::Request::Ping { .. } => zajil_host::Response::ok(),
        // Served before it reaches here (`serve_extension`).
        zajil_host::Request::ShowWindow { .. } => zajil_host::Response::refused("not here"),
        // The link is served apart (`Links`); an answer comes only on it.
        zajil_host::Request::Link { .. } | zajil_host::Request::Cookies { .. } => {
            zajil_host::Response::refused("not here")
        }
        // Video downloads (M6) are not in the program yet: the extension
        // is told so, in the user's language, and nothing else happens.
        zajil_host::Request::Video { .. } | zajil_host::Request::PeekVideo { .. } => {
            zajil_host::Response::refused(video_soon(lang))
        }
        // Served before it reaches here.
        zajil_host::Request::ExtensionOld { .. } => zajil_host::Response::ok(),
        zajil_host::Request::PrivateClosed { .. } => match discard_private(store) {
            Ok(()) => zajil_host::Response::ok(),
            Err(err) => zajil_host::Response::refused(err.to_string()),
        },
        zajil_host::Request::Download { .. } => {
            let page = match from_page(request, drive_cookies) {
                Some(Ok(page)) => page,
                Some(Err(why)) => return zajil_host::Response::refused(why),
                None => return zajil_host::Response::refused("not a download"),
            };
            let (_, runs_code) = match reached(&page, reach) {
                Ok(reached) => reached,
                Err(why) => return zajil_host::Response::refused(why),
            };
            let add = AddRequest {
                url: page.url.clone(),
                name: None,
                category: None,
                description: None,
                queue: None,
                folder: None,
                start: !runs_code,
                limit: None,
                connections: None,
            };
            match add_from_page(store, &page, &add, given, headers, runs_code, false) {
                Ok(_) => zajil_host::Response::ok(),
                Err(err) => zajil_host::Response::refused(err.to_string()),
            }
        }
    }
}

/// A download the browser sent, before Zajil has reached the file: its
/// link, the browser's name for it, its cookies and page (in memory only),
/// and where it came from.
#[derive(Clone)]
pub(crate) struct FromPage {
    pub url: String,
    filename: Option<String>,
    from: zajil_core::FromBrowser,
    source: Option<zajil_core::queue::BrowserSource>,
    private: bool,
}

/// What a [`zajil_host::Request::Download`] sends, checked as adding would
/// check it; `None` for another request. `drive_cookies`: as in
/// [`from_extension_with`].
pub(crate) fn from_page(
    request: &zajil_host::Request,
    drive_cookies: impl FnOnce(&str) -> Option<String>,
) -> Option<Result<FromPage, String>> {
    let zajil_host::Request::Download {
        url,
        filename,
        referrer,
        cookies,
        instance,
        store: cookie_store,
        ticket,
        private,
        ..
    } = request
    else {
        return None;
    };
    if let Err(err) = zajil_core::queue::check_url(url) {
        return Some(Err(err.to_string()));
    }
    // The browser's cookies (when the user allowed them) and page, for the
    // check and then the download: in memory only.
    let mut from = zajil_core::FromBrowser::new(url, cookies.as_deref(), referrer.as_deref());
    // A Google Drive file's page link is downloaded from Drive's download
    // address, another origin of the same site: that address's own
    // cookies are asked for.
    if from.has_cookies()
        && let Some(file) = zajil_core::drive::download_link(url)
    {
        from = from.and_cookies_for(&file, drive_cookies(&file).as_deref());
    }
    // Where it came from, so that browser can be asked for the cookies
    // again after a restart: never the cookies themselves (owner's
    // decision on #7). The page without its `#` part (owner's decision on
    // #80).
    let page = referrer.as_deref().and_then(|r| {
        let mut page = zajil_core::Url::parse(r).ok()?;
        page.set_fragment(None);
        Some(page.to_string())
    });
    // A private window's download keeps nothing of where it came from
    // (security review #84).
    let source =
        instance
            .as_ref()
            .filter(|_| !*private)
            .map(|instance| zajil_core::queue::BrowserSource {
                instance: instance.clone(),
                store: cookie_store.clone(),
                cookies: cookies.is_some(),
                referrer: page,
                ticket: ticket.clone(),
            });
    Some(Ok(FromPage {
        url: url.clone(),
        filename: filename.clone(),
        from,
        source,
        private: *private,
    }))
}

/// The file `page` names, as Zajil reaches it (`reach`), and whether it can
/// run code; refused when it is not that file (review of #70), with what
/// the extension tells the user.
pub(crate) fn reached(
    page: &FromPage,
    reach: impl FnOnce(&str, &zajil_core::FromBrowser) -> Result<zajil_core::Probe, String>,
) -> Result<(zajil_core::Probe, bool), String> {
    let probe =
        reach(&page.url, &page.from).map_err(|why| format!("Zajil cannot download it: {why}"))?;
    if let Some(why) = not_the_file(&probe, page.filename.as_deref()) {
        return Err(format!("Zajil cannot download it: {why}"));
    }
    // A file that can run code, from a page: never started by itself, the
    // user starts it (security review #84). By the server's name, the
    // browser's, and the link's.
    let from_link = zajil_core::Url::parse(&page.url).ok().and_then(|u| {
        u.path_segments()
            .and_then(|mut s| s.next_back().map(str::to_owned))
    });
    let runs_code = [
        Some(probe.name.as_str()),
        page.filename.as_deref(),
        from_link.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(zajil_core::category::runs_code);
    Ok((probe, runs_code))
}

/// Adds `page`'s download as `add` says, with the browser's headers in
/// memory (`headers`). A file that can run code waits for the user's OK
/// (`runs_code`) unless the user gave it already (`ok_given`: «Start
/// download» in its «Download file info»).
pub(crate) fn add_from_page(
    store: &QueueStore,
    page: &FromPage,
    add: &AddRequest,
    given: &Path,
    headers: &zajil_core::queue::FromBrowserById,
    runs_code: bool,
    ok_given: bool,
) -> Result<u64, StoreError> {
    let id = add_blocking(
        store,
        add,
        given,
        page.source.clone(),
        runs_code && !ok_given,
        ok_given,
        true,
        page.private,
    )?;
    headers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, page.from.clone());
    Ok(id)
}

/// «Video downloads are coming soon», in `lang` (the translations' own
/// text, `video-soon`).
fn video_soon(lang: zajil_core::settings::Language) -> String {
    let lang = match lang {
        zajil_core::settings::Language::Ar => zajil_i18n::Lang::Ar,
        zajil_core::settings::Language::En => zajil_i18n::Lang::En,
    };
    zajil_i18n::Localizer::new(lang)
        .text("video-soon", &[])
        .unwrap_or_else(|_| "Video downloads are coming soon.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::Live;
    use crate::runner::queue_runner;
    use crate::view;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use zajil_core::Options;
    use zajil_core::category::CategoryStore;
    use zajil_core::queue::{DEFAULT_QUEUE, Status};

    /// M5, end to end on the real channel: a download sent as `zajil-host`
    /// sends it reaches the app's loop (`zajil_host::serve`) and lands in
    /// the queues file, from where the queue runner downloads it.
    #[test]
    fn a_download_through_the_real_channel_lands_in_the_queue() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let listener = zajil_platform::ipc::Listener::bind(data.path()).unwrap();
        let (served, given) = (store.clone(), data.path().to_path_buf());
        std::thread::spawn(move || {
            zajil_host::serve(
                listener,
                move |request| {
                    from_extension(
                        &served,
                        &request,
                        &given,
                        &Default::default(),
                        zajil_core::settings::Language::Ar,
                        |_, _| Ok(probe_of("b.iso", None)),
                    )
                },
                Default::default(),
            );
        });
        let mut stream =
            zajil_platform::ipc::connect(data.path(), std::time::Duration::from_secs(5)).unwrap();
        zajil_host::write_frame(
            &mut stream,
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/b.iso"}"#,
        )
        .unwrap();
        let answer = zajil_host::read_frame(&mut stream, zajil_host::MAX_RESPONSE)
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<zajil_host::Response>(&answer).unwrap(),
            zajil_host::Response::ok()
        );
        let queues = store.load().unwrap();
        assert_eq!(queues.queues[0].items[0].url, "https://example.com/b.iso");
    }

    /// M5: a download from the extension lands in the default queue, by
    /// category, started; a ping changes nothing.
    #[test]
    fn a_download_from_the_extension_is_added_to_the_queue() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let request = zajil_host::parse(
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/a.zip","cookies":"sid=1"}"#,
        )
        .unwrap();
        assert_eq!(
            from_extension(
                &store,
                &request,
                data.path(),
                &Default::default(),
                zajil_core::settings::Language::Ar,
                |_, _| Ok(probe_of("a.zip", None))
            ),
            zajil_host::Response::ok()
        );
        let queues = store.load().unwrap();
        let item = queues.queues[0].items.first().expect("added");
        assert_eq!(queues.queues[0].name, DEFAULT_QUEUE);
        assert_eq!(item.url, "https://example.com/a.zip");
        assert!(item.categorize);
        assert_eq!(item.status, Status::Waiting);
        let ping = zajil_host::parse(br#"{"type":"ping","protocol_version":1}"#).unwrap();
        assert_eq!(
            from_extension(
                &store,
                &ping,
                data.path(),
                &Default::default(),
                zajil_core::settings::Language::Ar,
                |_, _| panic!("a ping reaches nothing")
            ),
            zajil_host::Response::ok()
        );
        assert_eq!(store.load().unwrap().queues[0].items.len(), 1);
    }

    /// What the engine's probe would say: a file `name` of `content_type`.
    fn probe_of(name: &str, content_type: Option<&str>) -> zajil_core::Probe {
        zajil_core::Probe {
            name: name.to_owned(),
            size: Some(10),
            resumable: true,
            ranges: true,
            host: Some("example.com".into()),
            content_type: content_type.map(str::to_owned),
        }
    }

    /// A link that leads to a sign-in page (a redirect answered 200 with a
    /// web page), or to a file of another type, is refused and nothing is
    /// added: the browser keeps its download (review of #70).
    #[test]
    fn a_sign_in_page_or_another_file_is_refused() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let request = zajil_host::parse(
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/l.zip","filename":"l.zip"}"#,
        )
        .unwrap();
        for (probe, why) in [
            (probe_of("signin", Some("text/html")), "a web page"),
            (
                probe_of("l.zip", Some("application/xhtml+xml")),
                "a web page",
            ),
            (
                probe_of("signin.php", Some("application/octet-stream")),
                "another file (signin.php), not a .zip",
            ),
            (
                probe_of("download", None),
                "another file (download), not a .zip",
            ),
        ] {
            let answer = from_extension(
                &store,
                &request,
                data.path(),
                &Default::default(),
                zajil_core::settings::Language::Ar,
                |_, _| Ok(probe),
            );
            assert!(!answer.ok);
            assert!(answer.error.as_deref().unwrap().contains(why), "{answer:?}");
        }
        assert!(store.load().unwrap().queues[0].items.is_empty());
        // The same file, its extension in capitals: taken.
        let answer = from_extension(
            &store,
            &request,
            data.path(),
            &Default::default(),
            zajil_core::settings::Language::Ar,
            |_, _| Ok(probe_of("L.ZIP", Some("application/zip"))),
        );
        assert!(answer.ok, "{answer:?}");
        // No name from the browser: only a web page is refused.
        let bare = zajil_host::parse(
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/get?id=1"}"#,
        )
        .unwrap();
        let answer = from_extension(
            &store,
            &bare,
            data.path(),
            &Default::default(),
            zajil_core::settings::Language::Ar,
            |_, _| Ok(probe_of("get", None)),
        );
        assert!(answer.ok, "{answer:?}");
    }

    /// M5: the cookies and the page the extension sends go with Zajil's
    /// check, and are kept in memory for the download, under its item; a
    /// download sent without them keeps none.
    #[test]
    fn the_extension_s_cookies_go_with_the_check_and_the_download() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let headers = zajil_core::queue::FromBrowserById::default();
        let request = zajil_host::parse(
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/a.zip","referrer":"https://example.com/page","cookies":"sid=1"}"#,
        )
        .unwrap();
        let mut checked_with = None;
        let answer = from_extension(
            &store,
            &request,
            data.path(),
            &headers,
            zajil_core::settings::Language::Ar,
            |_, from| {
                checked_with = Some(from.has_cookies());
                Ok(probe_of("a.zip", None))
            },
        );
        assert!(answer.ok, "{answer:?}");
        assert_eq!(checked_with, Some(true));
        let id = store.load().unwrap().queues[0].items[0].id;
        let kept = headers.lock().unwrap();
        assert!(kept.get(&id).is_some_and(|f| f.has_cookies()));
        drop(kept);
        let bare = zajil_host::parse(
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/b.zip"}"#,
        )
        .unwrap();
        let answer = from_extension(
            &store,
            &bare,
            data.path(),
            &headers,
            zajil_core::settings::Language::Ar,
            |_, from| {
                assert!(!from.has_cookies());
                Ok(probe_of("b.zip", None))
            },
        );
        assert!(answer.ok);
        let second = store.load().unwrap().queues[0].items[1].id;
        assert!(!headers.lock().unwrap()[&second].has_cookies());
    }

    /// Owner's decision on the Drive links: a Drive file's page link from
    /// the browser, with its cookies, asks the browser for the cookies of
    /// the file's own download address (and nothing else); without
    /// cookies, or for another site, nothing is asked.
    #[test]
    fn a_drive_link_asks_the_browser_for_its_download_address_s_cookies() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let id = "1AbCdEfGhIjKlMnOpQrStUvWxYz012345";
        let add = |url: &str, cookies: Option<&str>| {
            let request = zajil_host::Request::Download {
                protocol_version: 1,
                url: url.into(),
                referrer: None,
                filename: None,
                cookies: cookies.map(str::to_owned),
                instance: Some("i".into()),
                store: None,
                ticket: Some("t".into()),
                private: false,
            };
            let asked = std::cell::RefCell::new(Vec::new());
            let answer = from_extension_with(
                &store,
                &request,
                data.path(),
                &Default::default(),
                zajil_core::settings::Language::Ar,
                |_, from| {
                    assert!(from.has_cookies() == cookies.is_some());
                    Ok(probe_of("report.pdf", None))
                },
                |file| {
                    asked.borrow_mut().push(file.to_owned());
                    Some("file=1".into())
                },
            );
            assert!(answer.ok, "{answer:?}");
            asked.into_inner()
        };
        let page = format!("https://drive.google.com/file/d/{id}/view");
        assert_eq!(
            add(&page, Some("page=1")),
            [format!(
                "https://drive.usercontent.google.com/download?id={id}&export=download"
            )]
        );
        assert!(add(&page, None).is_empty(), "no cookies: nothing asked");
        assert!(
            add("https://example.com/report.pdf", Some("site=1")).is_empty(),
            "not Drive: nothing asked"
        );
    }

    /// #158: a Drive file's download address's cookies are asked of the
    /// browser the download came from, with its ticket, never for a
    /// private window's download, nor without a ticket.
    #[test]
    fn only_a_normal_window_s_download_with_a_ticket_is_asked_again() {
        let request = |private: bool, instance: Option<&str>, ticket: Option<&str>| {
            zajil_host::Request::Download {
                protocol_version: 1,
                url: "https://drive.google.com/file/d/1AbCdEfGhIjKlMnOpQrStUvWxYz012345/view"
                    .into(),
                referrer: None,
                filename: None,
                cookies: Some("page=1".into()),
                instance: instance.map(str::to_owned),
                store: Some("firefox-container-1".into()),
                ticket: ticket.map(str::to_owned),
                private,
            }
        };
        assert_eq!(
            drive_ask_of(&request(false, Some("i"), Some("t"))),
            Some(("i", Some("firefox-container-1"), "t"))
        );
        assert_eq!(drive_ask_of(&request(true, Some("i"), Some("t"))), None);
        assert_eq!(drive_ask_of(&request(false, None, Some("t"))), None);
        assert_eq!(drive_ask_of(&request(false, Some("i"), None)), None);
        assert_eq!(
            drive_ask_of(&zajil_host::Request::Ping {
                protocol_version: 1
            }),
            None
        );
    }

    /// #158: asked again after a restart, a Drive link's browser is asked a
    /// second time, for its download address; another link is not.
    #[test]
    fn a_drive_link_asked_again_gets_its_download_address_s_cookies_too() {
        let id = "1AbCdEfGhIjKlMnOpQrStUvWxYz012345";
        let page = format!("https://drive.google.com/file/d/{id}/view");
        let file = format!("https://drive.usercontent.google.com/download?id={id}&export=download");
        let ask = |url: &str, answer: Option<&str>| {
            let mut asked = Vec::new();
            let from =
                with_drive_cookies(zajil_core::FromBrowser::new(url, None, None), url, |f| {
                    asked.push(f.to_owned());
                    answer.map(str::to_owned)
                });
            (from.has_cookies(), asked)
        };
        assert_eq!(ask(&page, Some("file=1")), (true, vec![file.clone()]));
        assert_eq!(ask(&page, None), (false, vec![file]));
        assert_eq!(
            ask("https://example.com/a.zip", Some("x=1")),
            (false, vec![])
        );
    }

    /// Security review #84: a file that can run code, from the browser, is
    /// never started by itself: it waits, paused, for the user (by the
    /// server's name, or the browser's); any other file starts at once.
    #[test]
    fn a_file_that_can_run_code_from_the_browser_waits_for_the_user() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let add = |url: &str, filename: Option<&str>, served: &'static str| {
            let request = zajil_host::Request::Download {
                protocol_version: 1,
                url: url.into(),
                referrer: None,
                filename: filename.map(str::to_owned),
                cookies: None,
                instance: None,
                store: None,
                ticket: None,
                private: false,
            };
            let answer = from_extension(
                &store,
                &request,
                data.path(),
                &Default::default(),
                zajil_core::settings::Language::Ar,
                |_, _| Ok(probe_of(served, None)),
            );
            assert!(answer.ok, "{answer:?}");
        };
        add("https://example.com/setup.exe", None, "setup.exe");
        add(
            "https://example.com/get?id=7",
            Some("report.docm"),
            "report.docm",
        );
        add("https://example.com/files/a.zip", None, "a.zip");
        let queues = store.load().unwrap();
        let items: Vec<_> = queues.queues[0]
            .items
            .iter()
            .map(|i| (i.status, i.needs_ok))
            .collect();
        assert_eq!(
            items,
            [
                (Status::Paused, true),
                (Status::Paused, true),
                (Status::Waiting, false)
            ]
        );
        // The page says why it waits, while it does.
        let item = &queues.queues[0].items[0];
        let categories = zajil_core::category::CategoriesFile::built_in();
        let live = Live::default();
        let view = view::download(item, DEFAULT_QUEUE, &categories, &live);
        assert!(view.needs_ok);
        let mut started = item.clone();
        started.status = Status::Waiting;
        assert!(!view::download(&started, DEFAULT_QUEUE, &categories, &live).needs_ok);
    }

    /// Security review #84: a download from a private window keeps
    /// nothing of where it came from (no instance, page or ticket), and is
    /// marked to leave the queues once done.
    #[test]
    fn a_private_window_s_download_keeps_nothing_of_where_it_came_from() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let request = zajil_host::parse(
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/b.iso","referrer":"https://example.com/private-page","instance":"inst-1","ticket":"t-1","private":true}"#,
        )
        .unwrap();
        let answer = from_extension(
            &store,
            &request,
            data.path(),
            &Default::default(),
            zajil_core::settings::Language::Ar,
            |_, _| Ok(probe_of("b.iso", None)),
        );
        assert!(answer.ok, "{answer:?}");
        let item = store.load().unwrap().queues[0].items[0].clone();
        assert!(item.private);
        assert_eq!(item.browser, None);
        let text = std::fs::read_to_string(data.path().join("queues.json")).unwrap();
        assert!(
            !text.contains("private-page") && !text.contains("inst-1"),
            "{text}"
        );
    }

    /// Review of #91: when the last private window closes, every private
    /// window's download leaves the queues and its part and state files are
    /// deleted; the others stay.
    #[test]
    fn the_last_private_window_closing_takes_its_downloads_out() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let state = data.path().join("p.bin.zajil-state");
        std::fs::write(&state, b"{}").unwrap();
        std::fs::write(data.path().join("p.bin.zajil-part"), b"x").unwrap();
        let (private, other) = store
            .update(|q| {
                let private = q.add(DEFAULT_QUEUE, "https://example.com/p.bin", data.path())?;
                let other = q.add(DEFAULT_QUEUE, "https://example.com/o.bin", data.path())?;
                let item = q.item_mut(private).unwrap();
                item.private = true;
                item.status = Status::Paused;
                item.state_file = Some(state.clone());
                Ok((private, other))
            })
            .unwrap();
        let request =
            zajil_host::parse(br#"{"type":"private_closed","protocol_version":1}"#).unwrap();
        let answer = from_extension(
            &store,
            &request,
            data.path(),
            &Default::default(),
            zajil_core::settings::Language::Ar,
            |_, _| unreachable!("nothing is checked"),
        );
        assert!(answer.ok, "{answer:?}");
        let queues = store.load().unwrap();
        assert!(queues.item(private).is_none());
        assert!(queues.item(other).is_some());
        assert!(!state.exists());
        assert!(!data.path().join("p.bin.zajil-part").exists());
    }

    /// A video from the extension (M6 is not here yet): answered «coming
    /// soon» in the user's language, without the check (no request to the
    /// page) and without anything added to the queues.
    #[test]
    fn a_video_from_the_extension_is_answered_coming_soon() {
        use zajil_core::settings::Language;
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let request = zajil_host::parse(
            br#"{"type":"video","protocol_version":1,"page":"https://video.example.com/watch?v=1"}"#,
        )
        .unwrap();
        for (lang, text) in [
            (Language::Ar, "تحميل الفيديو قادم قريبًا."),
            (Language::En, "Video downloads are coming soon."),
        ] {
            let answer = from_extension(
                &store,
                &request,
                data.path(),
                &Default::default(),
                lang,
                |_, _| unreachable!("a video is not checked here"),
            );
            assert_eq!(answer, zajil_host::Response::refused(text));
        }
        assert!(
            store
                .load()
                .unwrap()
                .queues
                .iter()
                .all(|q| q.items.is_empty())
        );
    }

    /// M5: a download from the extension keeps where it came from (the
    /// extension instance, its container, that cookies went, the page),
    /// but never the cookies themselves.
    #[test]
    fn a_download_from_the_browser_keeps_where_it_came_from_but_never_its_cookies() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let request = zajil_host::parse(
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/b.iso","referrer":"https://example.com/page#section-2","cookies":"sid=SECRET","instance":"inst-1","store":"firefox-container-2","ticket":"ticket-9"}"#,
        )
        .unwrap();
        let answer = from_extension(
            &store,
            &request,
            data.path(),
            &Default::default(),
            zajil_core::settings::Language::Ar,
            |_, _| Ok(probe_of("b.iso", None)),
        );
        assert!(answer.ok, "{answer:?}");
        let item = store.load().unwrap().queues[0].items[0].clone();
        assert_eq!(
            item.browser,
            Some(zajil_core::queue::BrowserSource {
                instance: "inst-1".into(),
                store: Some("firefox-container-2".into()),
                cookies: true,
                referrer: Some("https://example.com/page".into()),
                ticket: Some("ticket-9".into()),
            })
        );
        let text = std::fs::read_to_string(data.path().join("queues.json")).unwrap();
        assert!(!text.contains("SECRET"), "{text}");
    }

    /// The glue in the app (review of #76): the check of a download from
    /// the extension sends its cookies and page, and the window's runner
    /// sends them with the download.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_check_and_the_window_s_runner_send_the_browser_s_headers() {
        use zajil_testserver::{FileSpec, TestServer, random_bytes};
        let server = TestServer::http();
        let data = random_bytes(300_000, 93);
        server.put("/a.zip", FileSpec::new(data.clone()));
        let url = server.url("/a.zip");
        let page = server.url("/page");
        let from = zajil_core::FromBrowser::new(&url, Some("sid=GLUE"), Some(&page));
        let engine = Engine::new(Options::default()).unwrap();

        let found = tokio::task::spawn_blocking({
            let (url, from, engine) = (url.clone(), from.clone(), engine.clone());
            move || extension_reach(&engine)(&url, &from)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(found.name, "a.zip");
        assert_eq!(server.gets("/a.zip")[0].cookie.as_deref(), Some("sid=GLUE"));

        let base = tempfile::tempdir().unwrap();
        let store = QueueStore::new(base.path());
        let dir = base.path().join("out");
        let id = store.update(|q| q.add(DEFAULT_QUEUE, &url, &dir)).unwrap();
        let headers = zajil_core::queue::FromBrowserById::default();
        headers.lock().unwrap().insert(id, from);
        let runner = queue_runner(
            &engine,
            &store,
            &CategoryStore::new(base.path()),
            &Arc::new(AtomicUsize::new(1)),
            &headers,
        );
        runner
            .poll(Duration::from_millis(20))
            .run(&Stop::new(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(std::fs::read(dir.join("a.zip")).unwrap(), data);
        for get in server.gets("/a.zip").iter().skip(1) {
            assert_eq!(get.cookie.as_deref(), Some("sid=GLUE"), "{get:?}");
            assert_eq!(get.referer.as_deref(), Some(page.as_str()));
        }
    }

    /// A file Zajil cannot reach itself (behind a login, a referrer check,
    /// or gone) is refused and nothing is added, so the extension leaves it
    /// to the browser (review of #70).
    #[test]
    fn a_file_zajil_cannot_reach_is_refused_and_not_added() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let request = zajil_host::parse(
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/private.zip"}"#,
        )
        .unwrap();
        let mut asked = Vec::new();
        let answer = from_extension(
            &store,
            &request,
            data.path(),
            &Default::default(),
            zajil_core::settings::Language::Ar,
            |url, _| {
                asked.push(url.to_owned());
                Err("the server answered 403 Forbidden".into())
            },
        );
        assert!(!answer.ok);
        assert_eq!(
            answer.error.as_deref(),
            Some("Zajil cannot download it: the server answered 403 Forbidden")
        );
        assert_eq!(asked, ["https://example.com/private.zip"]);
        assert!(store.load().unwrap().queues[0].items.is_empty());
    }
}
