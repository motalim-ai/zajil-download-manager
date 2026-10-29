//! Downloads from the browser, as in IDM (the owner's notes on 1.0): each
//! one opens «Download file info» on its own, above every window and in
//! the middle of the screen, without the main window. The window opens at
//! once, saying it is asking the server, and fills in when Zajil has
//! reached the file. «Start download» there is the user's OK for a file
//! that can run code, so it is not held again (`ok_given`). The setting
//! «Start browser downloads at once» adds the download and starts it
//! without the window, for a file that cannot run code; one that can still
//! asks.
//!
//! The extension is answered once the file is reached, as before (review
//! of #70): a file Zajil cannot reach stays with the browser, and its
//! window closes.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use tauri::{Emitter, Manager, State};
use ts_rs::TS;

use crate::closing::Closing;
use crate::view::ProbeView;
use crate::windows::{self, InfoStart};
use crate::{App, FromPage};

/// What «Download file info» of a browser's download learns once Zajil
/// has reached the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct OfferView {
    pub probe: ProbeView,
    /// It can run code: it starts only with «Start download», never in
    /// the background while the window is open.
    pub runs_code: bool,
}

type Answer = Option<Result<OfferView, String>>;

/// A download the browser sent, with its window open.
struct Offer {
    page: FromPage,
    runs_code: bool,
    answer: tokio::sync::watch::Sender<Answer>,
}

/// The browser's downloads whose window is open, by number.
#[derive(Default)]
pub struct Offers {
    next: AtomicU64,
    open: Mutex<HashMap<u64, Offer>>,
}

impl Offers {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Offer>> {
        self.open.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn insert(&self, page: FromPage) -> u64 {
        let n = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let offer = Offer {
            page,
            runs_code: false,
            answer: tokio::sync::watch::Sender::new(None),
        };
        self.lock().insert(n, offer);
        n
    }

    fn answer(&self, n: u64, answer: Result<OfferView, String>) {
        if let Some(offer) = self.lock().get_mut(&n) {
            if let Ok(view) = &answer {
                offer.runs_code = view.runs_code;
            }
            offer.answer.send_replace(Some(answer));
        }
    }

    fn remove(&self, n: u64) {
        self.lock().remove(&n);
    }
}

/// The window of offer `n`.
pub fn label(n: u64) -> String {
    format!("{}-{n}", windows::INFO)
}

/// «Download file info», in `lang` (the translations' own text).
fn title(lang: zajil_core::settings::Language) -> String {
    let lang = match lang {
        zajil_core::settings::Language::Ar => zajil_i18n::Lang::Ar,
        zajil_core::settings::Language::En => zajil_i18n::Lang::En,
    };
    zajil_i18n::Localizer::new(lang)
        .text("info-title", &[])
        .unwrap_or_else(|_| "Download file info".to_owned())
}

/// Opens offer `n`'s window: on its own, in front of every window (as
/// IDM's), centred. Closing it forgets the offer.
fn open_window<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    n: u64,
    url: &str,
    lang: zajil_core::settings::Language,
) -> tauri::Result<()> {
    let label = label(n);
    handle.state::<windows::Shared>().lock().unwrap().set_info(
        &label,
        InfoStart {
            url: url.to_owned(),
            offer: Some(n),
        },
    );
    windows::open_page(
        handle,
        &label,
        windows::INFO,
        &title(lang),
        windows::INFO_SIZE,
        false,
    )?;
    let Some(window) = windows::find(handle, &label) else {
        return Ok(());
    };
    window.set_always_on_top(true)?;
    let app = handle.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            app.state::<Offers>().remove(n);
            app.state::<windows::Shared>()
                .lock()
                .unwrap()
                .forget_info(&label);
        }
    });
    Ok(())
}

/// Closes offer `n`'s window (the file could not be reached: the browser
/// keeps it).
fn close_window<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, n: u64) {
    handle.state::<Offers>().remove(n);
    if let Some(window) = windows::find(handle, &label(n)) {
        let _ = window.close();
    }
}

/// Offer `page`: a new number, and its window open.
fn offer<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    page: &FromPage,
    lang: zajil_core::settings::Language,
) -> Result<u64, String> {
    let offers = handle.state::<Offers>();
    let n = offers.insert(page.clone());
    open_window(handle, n, &page.url, lang).map_err(|err| {
        offers.remove(n);
        format!("Zajil cannot show it: {err}")
    })?;
    Ok(n)
}

/// A download from the browser (`serve_extension`, on its own thread): its
/// window opens at once, then Zajil reaches the file and answers the
/// extension. `drive_cookies`: as in `from_extension_with`.
pub(crate) fn from_browser<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    request: &zajil_host::Request,
    lang: zajil_core::settings::Language,
    drive_cookies: impl FnOnce(&str) -> Option<String>,
) -> zajil_host::Response {
    let page = match crate::from_page(request, drive_cookies) {
        Some(Ok(page)) => page,
        Some(Err(why)) => return zajil_host::Response::refused(why),
        None => return zajil_host::Response::refused("not a download"),
    };
    let app = handle.state::<App>();
    // A quitting Zajil takes nothing new: the browser keeps it.
    if app.exiting.load(Ordering::SeqCst) {
        return zajil_host::Response::refused("Zajil is closing");
    }
    let prefs = handle.state::<Closing>().prefs().add;
    // The window first, so the user sees it while the server is asked.
    let mut shown = None;
    if !prefs.browser_at_once {
        match offer(handle, &page, lang) {
            Ok(n) => shown = Some(n),
            Err(why) => return zajil_host::Response::refused(why),
        }
    }
    let (probe, runs_code) = match crate::reached(&page, crate::extension_reach(&app.engine)) {
        Ok(reached) => reached,
        Err(why) => {
            if let Some(n) = shown {
                close_window(handle, n);
            }
            return zajil_host::Response::refused(why);
        }
    };
    let given = crate::download_dir(&app.settings, &app.system_download_dir);
    // Started at once, as the setting says: its progress window as the
    // settings say, and nothing else.
    if shown.is_none() && !runs_code {
        let add = crate::AddRequest {
            url: page.url.clone(),
            name: None,
            category: None,
            description: None,
            queue: None,
            folder: None,
            start: true,
            limit: None,
            connections: None,
        };
        return match crate::add_from_page(
            &app.store,
            &page,
            &add,
            &given,
            &app.from_browser,
            false,
            false,
        ) {
            Ok(id) => {
                let _ = handle.emit("downloads-changed", ());
                if let Err(err) =
                    crate::addprefs::show_started(handle, prefs.progress_window, id, &probe.name)
                {
                    eprintln!("zajil: the progress window of download {id}: {err}");
                }
                zajil_host::Response::ok()
            }
            Err(err) => zajil_host::Response::refused(err.to_string()),
        };
    }
    // A file that can run code asks, whatever the setting.
    let n = match shown {
        Some(n) => n,
        None => match offer(handle, &page, lang) {
            Ok(n) => n,
            Err(why) => return zajil_host::Response::refused(why),
        },
    };
    let categories = app
        .categories
        .load()
        .unwrap_or_else(|_| crate::no_categories());
    let view = crate::view::probe(&probe, &categories, &given);
    handle.state::<Offers>().answer(
        n,
        Ok(OfferView {
            probe: view,
            runs_code,
        }),
    );
    zajil_host::Response::ok()
}

/// What Zajil learned of offer `offer`'s file, once it has reached it.
#[tauri::command]
pub async fn offer_answer(offers: State<'_, Offers>, offer: u64) -> Result<OfferView, String> {
    let mut answer = {
        let open = offers.lock();
        let Some(found) = open.get(&offer) else {
            return Err(format!("no download {offer} from the browser"));
        };
        found.answer.subscribe()
    };
    let answer = answer
        .wait_for(Option::is_some)
        .await
        .map_err(|_| format!("no download {offer} from the browser"))?;
    answer.clone().expect("waited for")
}

/// Adds offer `offer`'s download as the user said in its window. `early`:
/// started in the background while the window is open (the setting
/// «Start downloading while the window is open»), never for a file that
/// can run code; otherwise «Start download» is the user's OK for it.
#[tauri::command]
pub async fn add_offer<R: tauri::Runtime>(
    app: State<'_, App>,
    offers: State<'_, Offers>,
    offer: u64,
    request: crate::AddRequest,
    early: bool,
    handle: tauri::AppHandle<R>,
) -> Result<u64, String> {
    let (page, runs_code) = {
        let open = offers.lock();
        let found = open
            .get(&offer)
            .ok_or_else(|| format!("no download {offer} from the browser"))?;
        if found.answer.borrow().is_none() {
            return Err("the server has not answered yet".into());
        }
        (found.page.clone(), found.runs_code)
    };
    if early && runs_code {
        return Err("a file that can run code starts only with «Start download»".into());
    }
    let (store, settings, system, headers) = (
        app.store.clone(),
        app.settings.clone(),
        app.system_download_dir.clone(),
        app.from_browser.clone(),
    );
    let ok_given = request.start && !early;
    let id = tokio::task::spawn_blocking(move || {
        let given = crate::chosen_folder(
            request.folder.as_deref(),
            &crate::download_dir(&settings, &system),
        )
        .map_err(zajil_core::queue::StoreError::Refused)?;
        crate::add_from_page(
            &store, &page, &request, &given, &headers, runs_code, ok_given,
        )
    })
    .await
    .map_err(crate::blocking_err)?
    .map_err(|e| e.to_string())?;
    // Taken: closing the window now changes nothing.
    offers.remove(offer);
    let _ = handle.emit("downloads-changed", ());
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> FromPage {
        let request = zajil_host::Request::Download {
            protocol_version: 1,
            url: "https://example.com/setup.exe".into(),
            referrer: Some("https://example.com/get#now".into()),
            filename: Some("setup.exe".into()),
            cookies: None,
            instance: None,
            store: None,
            ticket: None,
            private: false,
        };
        crate::from_page(&request, |_| None).unwrap().unwrap()
    }

    /// The window waits for the server's answer; a window whose download
    /// is gone (it could not be reached) is told so instead of waiting.
    #[test]
    fn the_window_waits_for_the_answer_and_a_gone_offer_ends_the_wait() {
        let offers = Offers::default();
        let n = offers.insert(page());
        assert_eq!(label(n), format!("add-info-{n}"));
        let mut answer = offers.lock().get(&n).unwrap().answer.subscribe();
        assert!(answer.borrow().is_none());
        offers.answer(n, Err("refused".into()));
        assert_eq!(*answer.borrow_and_update(), Some(Err("refused".to_owned())));
        let gone = offers.insert(page());
        let mut waiting = offers.lock().get(&gone).unwrap().answer.subscribe();
        offers.remove(gone);
        let ended = tauri::async_runtime::block_on(waiting.wait_for(Option::is_some));
        assert!(ended.is_err());
    }
}
