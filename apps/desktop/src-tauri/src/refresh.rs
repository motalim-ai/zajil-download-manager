//! «Refresh the download link» (M5): a download goes on from a new link
//! to the same file.

use tauri::{Emitter, State};
use zajil_core::queue::{QueueStore, Status, StoreError};
use zajil_core::{Engine, Stop};

use crate::App;
#[cfg(feature = "ui-test")]
use crate::ui_test;
use crate::view::RefreshProblem;
#[cfg(feature = "ui-test")]
use tauri::Manager;

/// "Refresh the download link" (M5): download `id` goes on from `url`.
#[tauri::command]
pub(crate) async fn refresh_link<R: tauri::Runtime>(
    app: State<'_, App>,
    id: u64,
    url: String,
    handle: tauri::AppHandle<R>,
) -> Result<(), RefreshProblem> {
    // A scene's demo downloads are not real (review of #47).
    #[cfg(feature = "ui-test")]
    if handle.state::<ui_test::UiTest>().scenario.is_some() {
        return Ok(());
    }
    refresh_item(
        &app.engine,
        &app.store,
        &app.from_browser,
        &app.refreshing,
        id,
        &url,
    )
    .await?;
    let _ = handle.emit("downloads-changed", ());
    Ok(())
}

/// Gives up the refresh of download `id` (its dialog was closed): it stops
/// at once, and the download is left as it was (review of #78).
#[tauri::command]
pub(crate) fn cancel_refresh(app: State<'_, App>, id: u64) {
    app.refreshing.cancel(id);
}

/// The downloads whose link is being refreshed, each with what stops it.
#[derive(Default)]
pub(crate) struct Refreshing(std::sync::Mutex<std::collections::HashMap<u64, Stop>>);

impl Refreshing {
    /// Marks `id` as being refreshed until the guard is dropped; `None`
    /// when it already is.
    fn start(&self, id: u64) -> Option<RefreshGuard<'_>> {
        let mut ids = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // Made only when marked: a guard made and dropped here would lock
        // again, and unmark the refresh under way.
        if ids.contains_key(&id) {
            return None;
        }
        let stop = Stop::new();
        ids.insert(id, stop.clone());
        Some(RefreshGuard { of: self, id, stop })
    }

    /// Stops the refresh of `id`, if one is under way.
    fn cancel(&self, id: u64) {
        if let Some(stop) = self.0.lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
            stop.request();
        }
    }

    /// `ids` without those being refreshed.
    pub(crate) fn without(&self, mut ids: Vec<u64>) -> Vec<u64> {
        let busy = self.0.lock().unwrap_or_else(|e| e.into_inner());
        ids.retain(|id| !busy.contains_key(id));
        ids
    }
}

struct RefreshGuard<'a> {
    of: &'a Refreshing,
    id: u64,
    stop: Stop,
}

impl Drop for RefreshGuard<'_> {
    fn drop(&mut self) {
        let mut ids = self.of.0.lock().unwrap_or_else(|e| e.into_inner());
        ids.remove(&self.id);
    }
}

/// Gives download `id` a new link: a paused or failed download goes on
/// from `url` once its server shows it is the same file
/// ([`Engine::refresh_link`], with the browser's cookies and page kept
/// for it); one with nothing downloaded yet just takes the link. Then it
/// waits for its turn again.
async fn refresh_item(
    engine: &Engine,
    store: &QueueStore,
    from_browser: &zajil_core::queue::FromBrowserById,
    refreshing: &Refreshing,
    id: u64,
    url: &str,
) -> Result<(), RefreshProblem> {
    zajil_core::queue::check_url(url).map_err(|e| RefreshProblem::BadLink {
        reason: e.to_string(),
    })?;
    // As the URL parser writes it, as the engine keeps it in the state, so
    // the queue still resumes that state for the item.
    let url = zajil_core::Url::parse(url)
        .map_err(|e| RefreshProblem::BadLink {
            reason: e.to_string(),
        })?
        .to_string();
    let guard = refreshing.start(id).ok_or(RefreshProblem::Busy)?;
    let loaded = store.clone();
    let queues = tokio::task::spawn_blocking(move || loaded.load())
        .await
        .map_err(|e| RefreshProblem::Failed {
            error: e.to_string(),
        })?
        .map_err(|e| RefreshProblem::Failed {
            error: e.to_string(),
        })?;
    let item = queues
        .item(id)
        .ok_or_else(|| RefreshProblem::Failed {
            error: format!("no download {id}"),
        })?
        .clone();
    match item.status {
        Status::Paused | Status::Failed => {}
        Status::Done => return Err(RefreshProblem::Done),
        Status::Waiting | Status::Active => return Err(RefreshProblem::NotStopped),
    }
    let state_file = match item.state_file {
        Some(path) => tokio::task::spawn_blocking(move || path.exists().then_some(path))
            .await
            .unwrap_or(None),
        None => None,
    };
    if let Some(state_file) = state_file {
        let from = from_browser
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned();
        let engine = match from {
            Some(from) => engine.with_browser(from),
            None => engine.clone(),
        };
        engine
            .refresh_link_until(&state_file, &url, &guard.stop)
            .await?;
    }
    // From here the state file has the new link: the item takes it too,
    // even if the dialog was closed meanwhile.
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        store.update(|q| {
            let item = q.item_mut(id).ok_or(StoreError::NoSuchItem(id))?;
            item.url = url;
            item.error = None;
            if matches!(item.status, Status::Paused | Status::Failed) {
                item.status = Status::Waiting;
            }
            Ok(())
        })
    })
    .await
    .map_err(|e| RefreshProblem::Failed {
        error: e.to_string(),
    })?
    .map_err(|e| RefreshProblem::Failed {
        error: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use zajil_core::Options;
    use zajil_core::queue::{DEFAULT_QUEUE, Runner};

    /// A queue download that failed when its link expired, with part of
    /// the file on disk.
    async fn expired_download(
        server: &zajil_testserver::TestServer,
        data: &[u8],
        base: &Path,
    ) -> (Engine, QueueStore, u64) {
        use zajil_testserver::FileSpec;
        let spec = || FileSpec::new(data.to_vec()).etag(Some("\"v1\""));
        server.put("/token1/f.bin", spec().cut_after(256 * 1024, 1));
        let engine = Engine::new(Options {
            connections: 1,
            max_failures: 1,
            ..Options::default()
        })
        .unwrap();
        let store = QueueStore::new(base);
        let url = server.url("/token1/f.bin");
        let dir = base.join("out");
        let id = store.update(|q| q.add(DEFAULT_QUEUE, &url, &dir)).unwrap();
        Runner::new(engine.clone(), store.clone())
            .poll(Duration::from_millis(20))
            .run(&Stop::new(), &mut |_| {})
            .await
            .unwrap();
        let item = store.load().unwrap().item(id).unwrap().clone();
        assert_eq!(item.status, Status::Failed, "{item:?}");
        assert!(item.state_file.as_ref().is_some_and(|p| p.exists()));
        // The signed link expired.
        server.put("/token1/f.bin", FileSpec::new(Vec::new()).forbid_after(0));
        (engine, store, id)
    }

    /// The ROADMAP criterion in the app: a download whose link expired
    /// goes on from a new link to the same file, asking only for what is
    /// missing, and its item keeps the new link.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_download_goes_on_from_its_refreshed_link() {
        use zajil_testserver::{FileSpec, TestServer, random_bytes};
        let server = TestServer::http();
        let data = random_bytes(1024 * 1024, 94);
        let base = tempfile::tempdir().unwrap();
        let (engine, store, id) = expired_download(&server, &data, base.path()).await;
        server.put(
            "/token2/f.bin",
            FileSpec::new(data.clone()).etag(Some("\"v1\"")),
        );
        // The browser's cookies and page kept for it go with the check and
        // the rest (the new link is on the same origin).
        let headers = zajil_core::queue::FromBrowserById::default();
        let page = server.url("/page");
        headers.lock().unwrap().insert(
            id,
            zajil_core::FromBrowser::new(
                &server.url("/token1/f.bin"),
                Some("sid=KEPT"),
                Some(&page),
            ),
        );
        let refreshing = Refreshing::default();
        // As a person may type it: kept as the parser writes it.
        let typed = server.url("/token2/f.bin").replace("http://", "HTTP://");
        refresh_item(&engine, &store, &headers, &refreshing, id, &typed)
            .await
            .unwrap();
        let item = store.load().unwrap().item(id).unwrap().clone();
        assert_eq!(item.url, server.url("/token2/f.bin"));
        assert_eq!((item.status, item.error), (Status::Waiting, None));
        Runner::new(engine, store.clone())
            .from_browser(headers.clone())
            .poll(Duration::from_millis(20))
            .run(&Stop::new(), &mut |_| {})
            .await
            .unwrap();
        let item = store.load().unwrap().item(id).unwrap().clone();
        assert_eq!(item.status, Status::Done, "{item:?}");
        assert_eq!(std::fs::read(item.file.unwrap()).unwrap(), data);
        let gets = server.gets("/token2/f.bin");
        assert_eq!(gets.len(), 2, "the check, then the rest: {gets:?}");
        let range = gets[1].range.as_deref().unwrap();
        assert!(!range.starts_with("bytes=0-"), "started over: {range}");
        for get in &gets {
            assert_eq!(get.cookie.as_deref(), Some("sid=KEPT"), "{get:?}");
        }
    }

    /// Closing the dialog gives the refresh up at once, even while the new
    /// link's server does not answer, and the download is left as it was
    /// (review of #78: it went on for minutes, and Resume did nothing).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_refresh_given_up_stops_at_once_and_changes_nothing() {
        use zajil_testserver::{TestServer, random_bytes};
        let server = TestServer::http();
        let data = random_bytes(1024 * 1024, 97);
        let base = tempfile::tempdir().unwrap();
        let (engine, store, id) = expired_download(&server, &data, base.path()).await;
        let before = store.load().unwrap().item(id).unwrap().clone();
        let state_before = std::fs::read(before.state_file.as_ref().unwrap()).unwrap();
        // A server that takes the connection and never answers.
        let quiet = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let quiet_url = format!("http://{}/f.bin", quiet.local_addr().unwrap());
        let headers = zajil_core::queue::FromBrowserById::default();
        let refreshing = Arc::new(Refreshing::default());
        let task = {
            let (engine, store, refreshing) = (engine.clone(), store.clone(), refreshing.clone());
            tokio::spawn(async move {
                refresh_item(&engine, &store, &headers, &refreshing, id, &quiet_url).await
            })
        };
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(refreshing.without(vec![id]).is_empty(), "not under way");
        let asked = Instant::now();
        refreshing.cancel(id);
        let result = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("still running 5 s after it was given up")
            .unwrap();
        assert_eq!(result, Err(RefreshProblem::Cancelled));
        assert!(
            asked.elapsed() < Duration::from_secs(2),
            "{:?}",
            asked.elapsed()
        );
        assert_eq!(refreshing.without(vec![id]), vec![id], "still marked");
        assert_eq!(store.load().unwrap().item(id).unwrap(), &before);
        assert_eq!(
            std::fs::read(before.state_file.as_ref().unwrap()).unwrap(),
            state_before
        );
        drop(quiet);
    }

    /// A link to another file is refused with the reason, and the download
    /// is left as it was; and what may not be refreshed is not.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_refresh_is_refused_for_another_file_and_while_it_may_not_be() {
        use view::MismatchView;
        use zajil_testserver::{FileSpec, TestServer, random_bytes};
        let server = TestServer::http();
        let data = random_bytes(1024 * 1024, 95);
        let base = tempfile::tempdir().unwrap();
        let (engine, store, id) = expired_download(&server, &data, base.path()).await;
        let before = store.load().unwrap().item(id).unwrap().clone();
        let state_before = std::fs::read(before.state_file.as_ref().unwrap()).unwrap();
        server.put(
            "/other.bin",
            FileSpec::new(random_bytes(1000, 96)).etag(Some("\"v1\"")),
        );
        let headers = zajil_core::queue::FromBrowserById::default();
        let refreshing = Refreshing::default();
        let refresh = |url: String| {
            let (engine, store, headers, refreshing) = (&engine, &store, &headers, &refreshing);
            async move { refresh_item(engine, store, headers, refreshing, id, &url).await }
        };
        assert_eq!(
            refresh(server.url("/other.bin")).await,
            Err(RefreshProblem::NotTheSameFile {
                why: MismatchView::Size,
                theirs: Some(1000),
                ours: Some(data.len() as u64),
            })
        );
        assert!(matches!(
            refresh("https://user:pw@example.com/f.bin".into()).await,
            Err(RefreshProblem::BadLink { .. })
        ));
        assert_eq!(store.load().unwrap().item(id).unwrap(), &before);
        assert_eq!(
            std::fs::read(before.state_file.as_ref().unwrap()).unwrap(),
            state_before
        );
        // Another refresh of it under way: refused, and it is not resumed.
        let held = refreshing.start(id).unwrap();
        assert_eq!(
            refresh(server.url("/other.bin")).await,
            Err(RefreshProblem::Busy)
        );
        assert_eq!(refreshing.without(vec![id, id + 1]), vec![id + 1]);
        drop(held);
        assert_eq!(refreshing.without(vec![id]), vec![id]);
        for (status, problem) in [
            (Status::Waiting, RefreshProblem::NotStopped),
            (Status::Active, RefreshProblem::NotStopped),
            (Status::Done, RefreshProblem::Done),
        ] {
            store
                .update(|q| {
                    q.item_mut(id).unwrap().status = status;
                    Ok(())
                })
                .unwrap();
            assert_eq!(refresh(server.url("/other.bin")).await, Err(problem));
        }
    }

    /// A download with nothing on disk yet just takes the new link.
    #[tokio::test]
    async fn a_download_not_started_takes_the_new_link() {
        let base = tempfile::tempdir().unwrap();
        let store = QueueStore::new(base.path());
        let dir = base.path().join("out");
        let id = store
            .update(|q| q.add(DEFAULT_QUEUE, "https://example.com/old.iso", &dir))
            .unwrap();
        store
            .update(|q| {
                q.item_mut(id).unwrap().status = Status::Paused;
                Ok(())
            })
            .unwrap();
        let engine = Engine::new(Options::default()).unwrap();
        refresh_item(
            &engine,
            &store,
            &Default::default(),
            &Refreshing::default(),
            id,
            "https://example.com/new.iso",
        )
        .await
        .unwrap();
        let item = store.load().unwrap().item(id).unwrap().clone();
        assert_eq!(item.url, "https://example.com/new.iso");
        assert_eq!(item.status, Status::Waiting);
    }
}
