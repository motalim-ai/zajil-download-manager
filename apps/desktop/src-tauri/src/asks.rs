//! Asking a download's browser for its cookies again after a restart,
//! and telling it which of its downloads are still open (M5, #80).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{Emitter, State};
use zajil_core::queue::{QueueStore, QueuesFile, Status};

use crate::extension::with_drive_cookies;
use crate::live::Live;
use crate::{App, blocking_err, in_scene};

/// A download that waits for its browser goes on without the cookies, at
/// the user's choice: its browser or profile is gone, or the user wants it
/// now (review of #80: it could wait forever).
#[tauri::command]
pub(crate) async fn go_without_cookies<R: tauri::Runtime>(
    app: State<'_, App>,
    id: u64,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    if in_scene(&handle) {
        return Ok(());
    }
    let store = app.store.clone();
    let queues = tokio::task::spawn_blocking(move || store.load())
        .await
        .map_err(blocking_err)?
        .map_err(|e| e.to_string())?;
    let item = queues.item(id).ok_or_else(|| format!("no download {id}"))?;
    without_cookies(&app.from_browser, &app.live, item);
    let _ = handle.emit("downloads-changed", ());
    Ok(())
}

/// Download `item` goes on without the browser's cookies, with its page.
pub(crate) fn without_cookies(
    from_browser: &zajil_core::queue::FromBrowserById,
    live: &Live,
    item: &zajil_core::queue::Item,
) {
    let referrer = item.browser.as_ref().and_then(|b| b.referrer.as_deref());
    from_browser
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(item.id)
        .or_insert_with(|| zajil_core::FromBrowser::new(&item.url, None, referrer));
    live.set_waiting_for_browser(item.id, false);
}

/// What the window does when a browser links: it is asked at the next look
/// for the downloads that wait for it, whatever it said before, and told
/// at once which of its downloads are still open.
pub(crate) fn when_linked(links: &zajil_host::Links, asks: &Arc<BrowserAsks>, store: &QueueStore) {
    let (inner, asks, store) = (links.clone(), asks.clone(), store.clone());
    links.on_link(move |instance| {
        asks.forget_tries();
        send_keep(&inner, &store, Some(instance));
    });
}

/// Tells the linked extension `instance` (each linked one, with `None`)
/// which of its downloads' tickets are still open, in the background: it
/// forgets the others at once (owner's decision on #80: the cookies are
/// given again only for a download that is not done and not removed).
pub(crate) fn send_keep(links: &zajil_host::Links, store: &QueueStore, instance: Option<&str>) {
    let instances = match instance {
        Some(instance) => vec![instance.to_owned()],
        None => links.instances(),
    };
    if instances.is_empty() {
        return;
    }
    let (links, store) = (links.clone(), store.clone());
    std::thread::spawn(move || {
        let Ok(queues) = store.load() else { return };
        for instance in instances {
            let open = open_tickets(&queues, &instance);
            links.keep(&instance, open, ASK_PATIENCE);
        }
    });
}

/// The tickets of `instance`'s downloads that are not done.
fn open_tickets(queues: &QueuesFile, instance: &str) -> Vec<String> {
    queues
        .queues
        .iter()
        .flat_map(|q| &q.items)
        .filter(|i| i.status != Status::Done)
        .filter_map(|i| i.browser.as_ref())
        .filter(|b| b.instance == instance)
        .filter_map(|b| b.ticket.clone())
        .collect()
}

/// How long a browser that did not answer is left before it is asked
/// again (unless it links again meanwhile).
const ASK_AGAIN: Duration = Duration::from_secs(5);
/// How long Zajil waits for a browser's answer.
const ASK_PATIENCE: Duration = Duration::from_secs(15);

/// The downloads whose browser is being asked for cookies, or was lately.
#[derive(Default)]
pub(crate) struct BrowserAsks(std::sync::Mutex<HashMap<u64, Asked>>);

#[derive(Clone, Copy)]
enum Asked {
    Now,
    Tried(Instant),
}

impl BrowserAsks {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Asked>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A browser linked: every download is asked again at once.
    fn forget_tries(&self) {
        self.lock().retain(|_, asked| matches!(asked, Asked::Now));
    }
}

/// A download that came from the browser with its cookies, after a restart
/// (the runner's `NeedsBrowser`): its browser (the same extension
/// instance) is asked for the cookies of its link again, in the
/// background, if it is linked; what it answers goes into memory, and the
/// runner starts the download at its next look (without cookies if the
/// browser has none to give, as when the permission was taken back,
/// owner's decision). A browser that is not open, or does not answer:
/// the download waits, marked «افتح المتصفح ليكمل», and is asked again
/// when the browser links. `changed` runs when what the page shows
/// changed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ask_browser(
    links: &zajil_host::Links,
    asks: &Arc<BrowserAsks>,
    live: &Arc<Live>,
    from_browser: &zajil_core::queue::FromBrowserById,
    id: u64,
    url: &str,
    source: &zajil_core::queue::BrowserSource,
    changed: impl Fn() + Send + 'static,
) {
    let mut asked = asks.lock();
    match asked.get(&id) {
        Some(Asked::Now) => return,
        Some(Asked::Tried(at)) if at.elapsed() < ASK_AGAIN => return,
        _ => {}
    }
    // Without a ticket (an older extension) it cannot be asked: it goes
    // without the cookies.
    let Some(ticket) = source.ticket.clone() else {
        drop(asked);
        let from = zajil_core::FromBrowser::new(url, None, source.referrer.as_deref());
        from_browser
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(id)
            .or_insert(from);
        changed();
        return;
    };
    if !links.is_linked(&source.instance) {
        asked.insert(id, Asked::Tried(Instant::now()));
        if live.set_waiting_for_browser(id, true) {
            changed();
        }
        return;
    }
    asked.insert(id, Asked::Now);
    drop(asked);
    let (links, asks, live, from_browser) = (
        links.clone(),
        asks.clone(),
        live.clone(),
        from_browser.clone(),
    );
    let (url, source) = (url.to_owned(), source.clone());
    std::thread::spawn(move || {
        let answer = links.ask(
            &source.instance,
            &url,
            source.store.as_deref(),
            &ticket,
            ASK_PATIENCE,
        );
        match answer {
            Some(cookies) => {
                let mut from = zajil_core::FromBrowser::new(
                    &url,
                    cookies.as_deref(),
                    source.referrer.as_deref(),
                );
                // A Drive file's link: its download address's own cookies
                // too, as when it came from the browser.
                from = with_drive_cookies(from, &url, |file| {
                    links
                        .ask(
                            &source.instance,
                            file,
                            source.store.as_deref(),
                            &ticket,
                            ASK_PATIENCE,
                        )
                        .flatten()
                });
                // The cookies the extension gave with the download itself
                // are never replaced (review of #80: a look just after the
                // item was written asked, and got none).
                from_browser
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(id)
                    .or_insert(from);
                asks.lock().remove(&id);
                live.set_waiting_for_browser(id, false);
            }
            None => {
                asks.lock().insert(id, Asked::Tried(Instant::now()));
                live.set_waiting_for_browser(id, true);
            }
        }
        changed();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{on_queue_event, queue_runner};
    use std::sync::atomic::AtomicUsize;
    use zajil_core::category::CategoryStore;
    use zajil_core::queue::{DEFAULT_QUEUE, QueueEvent};
    use zajil_core::{Engine, Options, Stop};

    /// M5, after a restart: a download that came from the browser with its
    /// cookies waits while its browser is not open, marked for the page;
    /// when that browser links, it is asked for the cookies of the link,
    /// and the download goes with them; then nothing waits any more.
    #[tokio::test(flavor = "multi_thread")]
    async fn after_a_restart_the_browser_is_asked_again_and_the_download_waits_for_it() {
        use zajil_testserver::{FileSpec, TestServer, random_bytes};
        let server = TestServer::http();
        let data = random_bytes(300_000, 98);
        server.put("/signed.zip", FileSpec::new(data.clone()));
        let url = server.url("/signed.zip");
        let base = tempfile::tempdir().unwrap();
        let store = QueueStore::new(base.path());
        let dir = base.path().join("out");
        let id = store
            .update(|q| {
                let id = q.add(DEFAULT_QUEUE, &url, &dir)?;
                q.item_mut(id).unwrap().browser = Some(zajil_core::queue::BrowserSource {
                    instance: "inst-1".into(),
                    store: Some("firefox-container-2".into()),
                    cookies: true,
                    referrer: None,
                    ticket: Some("ticket-1".into()),
                });
                Ok(id)
            })
            .unwrap();
        let engine = Engine::new(Options::default()).unwrap();
        let live = Arc::new(Live::default());
        let links = zajil_host::Links::default();
        let asks = Arc::<BrowserAsks>::default();
        let from_browser = zajil_core::queue::FromBrowserById::default();
        when_linked(&links, &asks, &store);
        // The window's channel, as `serve_extension` opens it.
        let listener = zajil_platform::ipc::Listener::bind(base.path()).unwrap();
        {
            let links = links.clone();
            std::thread::spawn(move || {
                zajil_host::serve(listener, |_| zajil_host::Response::ok(), links)
            });
        }
        let stop = Stop::new();
        let categories = CategoryStore::new(base.path());
        let runner = {
            let (live, links, asks, from_browser, stop, store) = (
                live.clone(),
                links.clone(),
                asks.clone(),
                from_browser.clone(),
                stop.clone(),
                store.clone(),
            );
            tokio::spawn(async move {
                let runner = queue_runner(
                    &engine,
                    &store,
                    &categories,
                    &Arc::new(AtomicUsize::new(1)),
                    &from_browser,
                );
                runner
                    .poll(Duration::from_millis(20))
                    .run(&stop.clone(), &mut |event| {
                        if matches!(event, QueueEvent::Finished { .. }) {
                            stop.request();
                        }
                        on_queue_event(&event, &live, &links, &asks, &from_browser, &store, || {});
                    })
                    .await
            })
        };
        // The browser is not open: it waits, marked.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !live.waiting_for_browser(id) {
            assert!(Instant::now() < deadline, "not marked");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            store.load().unwrap().item(id).unwrap().status,
            Status::Waiting
        );
        assert!(
            server.gets("/signed.zip").is_empty(),
            "went without its cookies"
        );

        // The browser opens: its host links, and answers every question,
        // the cookies with "sid=BACK"; what it is asked is recorded, with
        // when, and when it linked.
        let (asked_tx, asked) = std::sync::mpsc::channel();
        let (linked_tx, linked) = std::sync::mpsc::channel();
        {
            let path = base.path().to_path_buf();
            std::thread::spawn(move || {
                let mut app = zajil_platform::ipc::connect(&path, Duration::from_secs(5)).unwrap();
                zajil_host::write_frame(
                    &mut app,
                    br#"{"type":"link","protocol_version":1,"instance":"inst-1"}"#,
                )
                .unwrap();
                let _ = linked_tx.send(Instant::now());
                while let Ok(Some(question)) =
                    zajil_host::read_frame(&mut app, zajil_host::MAX_REQUEST)
                {
                    let question = zajil_host::ToExtension::parse(&question).unwrap();
                    let cookies = matches!(question, zajil_host::ToExtension::CookiesWanted { .. })
                        .then(|| "sid=BACK".to_owned());
                    let answer = zajil_host::FromLink::Cookies {
                        protocol_version: 1,
                        ask: question.ask(),
                        cookies,
                    };
                    let _ = asked_tx.send((question, Instant::now()));
                    if zajil_host::write_json(&mut app, &answer).is_err() {
                        break;
                    }
                }
            });
        }
        tokio::time::timeout(Duration::from_secs(20), runner)
            .await
            .expect("it did not go")
            .unwrap()
            .unwrap();
        let linked_at = linked.recv().unwrap();
        let next = || asked.recv_timeout(Duration::from_secs(10)).unwrap();
        let mut seen = vec![next(), next()];
        seen.sort_by_key(|(q, _)| matches!(q, zajil_host::ToExtension::CookiesWanted { .. }));
        // Asked at once when it links, not at the next retry (review of
        // #80): timed to the question itself, not to the end of the
        // download after it (2.59 s once on Windows, the download and the
        // queue's stop included).
        let cookies_asked = seen[1].1.saturating_duration_since(linked_at);
        assert!(cookies_asked < ASK_AGAIN / 2, "{cookies_asked:?}");
        let seen: Vec<_> = seen.into_iter().map(|(q, _)| q).collect();
        // Told at once which of its downloads are open, then asked for this
        // one's cookies, by its ticket, from its store.
        assert!(
            matches!(&seen[0], zajil_host::ToExtension::Keep { tickets, .. } if tickets == &["ticket-1"]),
            "{seen:?}"
        );
        assert!(
            matches!(&seen[1], zajil_host::ToExtension::CookiesWanted { url: u, store: Some(s), ticket, .. }
                if u == &url && s == "firefox-container-2" && ticket == "ticket-1"),
            "{seen:?}"
        );
        // Done: told again, and its ticket is no longer open.
        assert!(
            matches!(next().0, zajil_host::ToExtension::Keep { tickets, .. } if tickets.is_empty())
        );
        let item = store.load().unwrap().item(id).unwrap().clone();
        assert_eq!(item.status, Status::Done, "{item:?}");
        assert_eq!(item.browser, None);
        assert!(!live.waiting_for_browser(id));
        let gets = server.gets("/signed.zip");
        assert!(!gets.is_empty());
        for get in gets {
            assert_eq!(get.cookie.as_deref(), Some("sid=BACK"), "{get:?}");
        }
    }

    /// A download that waits for its browser can go on without the
    /// cookies, at the user's choice, with its page; cookies already in
    /// memory are never replaced.
    #[test]
    fn a_download_waiting_for_its_browser_can_go_without_the_cookies() {
        let base = tempfile::tempdir().unwrap();
        let store = QueueStore::new(base.path());
        let id = store
            .update(|q| {
                let id = q.add(DEFAULT_QUEUE, "https://example.com/a.zip", base.path())?;
                q.item_mut(id).unwrap().browser = Some(zajil_core::queue::BrowserSource {
                    instance: "gone".into(),
                    store: None,
                    cookies: true,
                    referrer: Some("https://example.com/page".into()),
                    ticket: Some("ticket-1".into()),
                });
                Ok(id)
            })
            .unwrap();
        let item = store.load().unwrap().item(id).unwrap().clone();
        let (from_browser, live) = (
            zajil_core::queue::FromBrowserById::default(),
            Live::default(),
        );
        live.set_waiting_for_browser(id, true);
        without_cookies(&from_browser, &live, &item);
        assert!(!live.waiting_for_browser(id));
        assert!(
            from_browser
                .lock()
                .unwrap()
                .get(&id)
                .is_some_and(|f| !f.has_cookies())
        );
        let kept = zajil_core::FromBrowser::new("https://example.com/a.zip", Some("sid=1"), None);
        from_browser.lock().unwrap().insert(id, kept);
        without_cookies(&from_browser, &live, &item);
        assert!(
            from_browser
                .lock()
                .unwrap()
                .get(&id)
                .is_some_and(|f| f.has_cookies())
        );
    }
}
