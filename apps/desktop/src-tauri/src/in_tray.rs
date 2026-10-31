//! What the main page does for Zajil, done here while its window is freed
//! in the tray (`main_window.rs`): with no page, nothing would say that a
//! download finished. As the page does (`App.svelte`'s `announce`,
//! `trayTip.ts`, `sounds.ts`), once a second: the system's notification
//! when a download finishes or fails (when the settings allow it), the
//! sound the user chose, and the tray icon's tooltip with what runs now.
//!
//! From the moment the window is freed until the next main page takes its
//! first look at the list ([`hand_over`]): from then on the page says it,
//! so nothing is said twice, nor lost between the two.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::Manager;
use zajil_core::settings::{Digits, Language};
use zajil_i18n::{Arg, Lang, Localizer};

use crate::App;
use crate::view::{DownloadView, StatusView};

/// How often the list is looked at while the window is freed.
const EVERY: Duration = Duration::from_secs(1);

/// What was seen at the last look.
#[derive(Default)]
pub struct InTray {
    on: AtomicBool,
    seen: Mutex<Seen>,
}

#[derive(Default)]
struct Seen {
    /// Each download's queue and status.
    last: HashMap<u64, (String, StatusView)>,
    /// The tooltip shown.
    tip: String,
}

/// The main window was freed: Zajil says what happens, from what the list
/// shows now (nothing that happened before).
pub fn start<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    let state = handle.state::<InTray>();
    if state.on.swap(true, Ordering::SeqCst) {
        return;
    }
    *state.seen.lock().unwrap() = Seen::default();
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let mut first = true;
        while handle.state::<InTray>().on.load(Ordering::SeqCst) {
            look(&handle, first).await;
            first = false;
            tokio::time::sleep(EVERY).await;
        }
    });
}

/// The next main page takes its first look: one last look here, and the
/// page says what happens from then on.
pub async fn hand_over<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) {
    if !handle.state::<InTray>().on.load(Ordering::SeqCst) {
        return;
    }
    look(handle, false).await;
    handle.state::<InTray>().on.store(false, Ordering::SeqCst);
}

/// One look at the list: what finished or failed since the last one is
/// said; `first` only notes what is there.
async fn look<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, first: bool) {
    let app = handle.state::<App>();
    let Ok(snapshot) = crate::downloads::listed(&app, handle).await else {
        return;
    };
    let settings = app.settings.clone();
    let settings = tokio::task::spawn_blocking(move || settings.load().unwrap_or_default())
        .await
        .unwrap_or_default();
    let (lang, digits) = (lang_of(settings.language), settings.digits);
    let said = {
        let state = handle.state::<InTray>();
        let mut seen = state.seen.lock().unwrap();
        let said = changes(&mut seen.last, &snapshot.downloads);
        let tip = tray_tip(&snapshot.downloads, lang, digits);
        if tip != seen.tip {
            if let Some(tray) = handle.tray_by_id(crate::tray::ID) {
                let _ = tray.set_tooltip(Some(crate::tray::fit(&tip)));
            }
            seen.tip = tip;
        }
        said
    };
    if first {
        return;
    }
    if !said.finished.is_empty() {
        // How many, never their names (the log is the user's own, but
        // names stay out of it, as links do).
        eprintln!(
            "zajil: in the tray: {} finished or failed (notifications {}, sound {:?})",
            said.finished.len(),
            if settings.notifications { "on" } else { "off" },
            said.sound(&handle.state::<crate::closing::Closing>().prefs().sounds)
                .map(crate::sounds::Sound::name)
        );
    }
    if settings.notifications {
        notify(handle, lang, &said.finished);
    }
    let prefs = handle.state::<crate::closing::Closing>().prefs().sounds;
    if let Some(sound) = said.sound(&prefs) {
        crate::sounds::play(handle, sound);
    }
}

/// What changed between two looks.
#[derive(Debug, Default, PartialEq)]
struct Said {
    /// Downloads that finished (`true`) or failed (`false`), by name.
    finished: Vec<(String, bool)>,
    /// A queue that was running finished.
    queue: bool,
}

impl Said {
    /// One sound for what happened, as the user chose: the queue's over
    /// the download's, a failure's over a success's (`App.svelte`).
    fn sound(&self, on: &crate::sounds::Sounds) -> Option<crate::sounds::Sound> {
        use crate::sounds::Sound;
        let done = self.finished.iter().any(|(_, ok)| *ok);
        let failed = self.finished.iter().any(|(_, ok)| !ok);
        [
            (self.queue && on.queue, Sound::Queue),
            (failed && on.failed, Sound::Failed),
            (done && on.done, Sound::Done),
        ]
        .into_iter()
        .find_map(|(heard, sound)| heard.then_some(sound))
    }
}

/// What finished or failed since `last`, and whether a queue finished,
/// as the page's `announce` and `finishedQueues`; `last` becomes now.
fn changes(last: &mut HashMap<u64, (String, StatusView)>, now: &[DownloadView]) -> Said {
    let status: HashMap<u64, StatusView> = now.iter().map(|d| (d.id, d.status)).collect();
    let mut busy: HashMap<&str, Vec<u64>> = HashMap::new();
    for d in now {
        if let Some((queue, before)) = last.get(&d.id)
            && matches!(before, StatusView::Active | StatusView::Waiting)
        {
            busy.entry(queue.as_str()).or_default().push(d.id);
        }
    }
    let queue = busy.iter().any(|(queue, ids)| {
        !queue.is_empty()
            && ids
                .iter()
                .all(|id| status.get(id) == Some(&StatusView::Done))
    });
    let mut finished = Vec::new();
    for d in now {
        if let Some((_, before)) = last.get(&d.id)
            && *before != d.status
            && matches!(d.status, StatusView::Done | StatusView::Failed)
        {
            finished.push((d.name.clone(), d.status == StatusView::Done));
        }
    }
    *last = now
        .iter()
        .map(|d| (d.id, (d.queue.clone(), d.status)))
        .collect();
    Said { finished, queue }
}

/// The system's notification of each, as the page sends it: «اكتمل
/// التحميل» or «فشل التحميل», and the name.
fn notify<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    lang: Lang,
    finished: &[(String, bool)],
) {
    use tauri_plugin_notification::NotificationExt;
    let text = Localizer::new(lang);
    for (name, ok) in finished {
        let key = if *ok {
            "notify-done-title"
        } else {
            "notify-failed-title"
        };
        let Ok(title) = text.text(key, &[]) else {
            continue;
        };
        let _ = handle
            .notification()
            .builder()
            .title(title)
            .body(name)
            .show();
    }
}

fn lang_of(language: Language) -> Lang {
    match language {
        Language::Ar => Lang::Ar,
        Language::En => Lang::En,
    }
}

/// The tray icon's tooltip, as `trayTip.ts` makes it: the app's name, and
/// while downloads run, how many, how far together (when every size is
/// known) and their speed.
fn tray_tip(downloads: &[DownloadView], lang: Lang, digits: Digits) -> String {
    let text = Localizer::new(lang);
    let t = |key: &str, args: &[(&str, Arg<'_>)]| {
        text.text(key, args)
            .map(|s| in_digits(&s, digits))
            .unwrap_or_else(|_| key.to_owned())
    };
    let name = t("app-name", &[]);
    let active: Vec<&DownloadView> = downloads
        .iter()
        .filter(|d| d.status == StatusView::Active)
        .collect();
    if active.is_empty() {
        return name;
    }
    let mut parts = vec![t(
        "footer-active",
        &[("count", Arg::Number(active.len() as f64))],
    )];
    let total: u64 = active.iter().map(|d| d.size.unwrap_or(0)).sum();
    if total > 0 && active.iter().all(|d| d.size.is_some()) {
        let done: u64 = active.iter().map(|d| d.done).sum();
        parts.push(percent(lang, digits, done as f64 / total as f64));
    }
    let rate: f64 = active.iter().map(|d| d.speed).sum();
    let speed = speed(lang, digits, rate);
    parts.push(if speed.is_empty() {
        size(lang, digits, 0.0)
    } else {
        speed
    });
    format!("{name}\n{}", parts.join(" · "))
}

/// `text` with its Western digits in `digits` (Fluent here writes them
/// Western; the page's own Fluent writes them in the user's).
fn in_digits(text: &str, digits: Digits) -> String {
    match digits {
        Digits::Latn => text.to_owned(),
        Digits::Arab => text
            .chars()
            .map(|c| match c.to_digit(10) {
                Some(d) => char::from_u32(0x0660 + d).unwrap_or(c),
                None => c,
            })
            .collect(),
    }
}

/// `n` with at most `fraction` digits after the point, as
/// `Intl.NumberFormat` writes it (`format.ts`'s `num`): Western «1,023.5»,
/// or Eastern Arabic «١٬٠٢٣٫٥».
fn num(digits: Digits, n: f64, fraction: i32) -> String {
    let scale = 10f64.powi(fraction);
    let rounded = (n * scale).round() / scale;
    let text = format!("{rounded}");
    let (whole, part) = text.split_once('.').unwrap_or((&text, ""));
    let (group, point) = match digits {
        Digits::Latn => (",", "."),
        Digits::Arab => ("٬", "٫"),
    };
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push_str(group);
        }
        grouped.push(c);
    }
    let all = if part.is_empty() {
        grouped
    } else {
        format!("{grouped}{point}{part}")
    };
    in_digits(&all, digits)
}

const KIB: f64 = 1024.0;
const MIB: f64 = 1024.0 * KIB;
const GIB: f64 = 1024.0 * MIB;

/// A size, as `format.ts`'s `size`: the unit chosen after rounding.
fn size(lang: Lang, digits: Digits, bytes: f64) -> String {
    let rounded = |unit: f64, fraction: i32| {
        let scale = 10f64.powi(fraction);
        (bytes / unit * scale).round() / scale
    };
    let mib_fraction = if bytes >= 100.0 * MIB { 0 } else { 1 };
    let (key, value) = if bytes >= GIB || rounded(MIB, mib_fraction) >= 1024.0 {
        ("size-gib", num(digits, bytes / GIB, 1))
    } else if bytes >= MIB || rounded(KIB, 0) >= 1024.0 {
        ("size-mib", num(digits, bytes / MIB, mib_fraction))
    } else if bytes >= KIB {
        ("size-kib", num(digits, bytes / KIB, 0))
    } else {
        ("size-bytes", num(digits, bytes, 0))
    };
    Localizer::new(lang)
        .text(key, &[("value", Arg::Text(&value))])
        .unwrap_or_else(|_| key.to_owned())
}

/// A speed, as `format.ts`'s `speed`: none at 0.
fn speed(lang: Lang, digits: Digits, bytes_per_second: f64) -> String {
    if bytes_per_second <= 0.0 {
        return String::new();
    }
    let size = size(lang, digits, bytes_per_second);
    Localizer::new(lang)
        .text("speed-per-second", &[("size", Arg::Text(&size))])
        .unwrap_or_default()
}

/// A whole percent, never 100 before the end, as `Intl.NumberFormat`
/// writes it, its marks included.
fn percent(lang: Lang, digits: Digits, fraction: f64) -> String {
    let whole = (fraction.clamp(0.0, 1.0) * 100.0).floor();
    let n = num(digits, whole, 0);
    match (digits, lang) {
        (Digits::Arab, _) => format!("{n}٪\u{061c}"),
        (Digits::Latn, Lang::Ar) => format!("{n}\u{200e}%\u{200e}"),
        (Digits::Latn, Lang::En) => format!("{n}%"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(id: u64, queue: &str, status: StatusView) -> DownloadView {
        DownloadView {
            id,
            name: format!("f{id}.bin"),
            queue: queue.into(),
            status,
            ..crate::view::DownloadView::empty_for_test()
        }
    }

    /// As `Intl.NumberFormat` in the page (the expected texts are Node's,
    /// the same ICU rules as the page's WebView).
    #[test]
    fn numbers_are_written_as_the_page_writes_them() {
        assert_eq!(num(Digits::Latn, 2.5, 1), "2.5");
        assert_eq!(num(Digits::Latn, 1023.0, 1), "1,023");
        assert_eq!(num(Digits::Latn, 12.04, 1), "12");
        assert_eq!(num(Digits::Arab, 2.5, 1), "٢٫٥");
        assert_eq!(num(Digits::Arab, 1023.0, 1), "١٬٠٢٣");
        assert_eq!(percent(Lang::Ar, Digits::Latn, 0.45), "45\u{200e}%\u{200e}");
        assert_eq!(percent(Lang::En, Digits::Latn, 0.999), "99%");
        assert_eq!(percent(Lang::Ar, Digits::Arab, 1.0), "١٠٠٪\u{061c}");
        assert_eq!(percent(Lang::En, Digits::Arab, 0.0), "٠٪\u{061c}");
    }

    /// The units, as `format.ts` picks them.
    #[test]
    fn sizes_and_speeds_take_the_page_s_units() {
        let plain = |s: String| zajil_i18n::without_isolation(&s);
        assert_eq!(plain(size(Lang::En, Digits::Latn, 512.0)), "512 B");
        assert_eq!(plain(size(Lang::En, Digits::Latn, 1_048_575.0)), "1 MiB");
        assert_eq!(plain(size(Lang::En, Digits::Latn, 2.5 * MIB)), "2.5 MiB");
        assert_eq!(plain(size(Lang::En, Digits::Latn, 300.0 * MIB)), "300 MiB");
        assert_eq!(plain(size(Lang::En, Digits::Latn, 6.1 * GIB)), "6.1 GiB");
        assert_eq!(plain(speed(Lang::En, Digits::Latn, 0.0)), "");
        assert!(plain(speed(Lang::Ar, Digits::Arab, 2.0 * MIB)).starts_with("٢ ميجابايت"));
    }

    /// The tooltip, as `trayTip.ts`: the name alone with nothing running.
    #[test]
    fn the_tooltip_says_what_runs() {
        let plain = |s: String| zajil_i18n::without_isolation(&s);
        assert_eq!(plain(tray_tip(&[], Lang::En, Digits::Latn)), "Zajil");
        let mut a = d(1, "main", StatusView::Active);
        a.size = Some(100 * 1024 * 1024);
        a.done = 45 * 1024 * 1024;
        a.speed = 2.0 * MIB;
        let paused = d(2, "main", StatusView::Paused);
        assert_eq!(
            plain(tray_tip(&[a.clone(), paused], Lang::En, Digits::Latn)),
            "Zajil\nDownloading: 1 download · 45% · 2 MiB/s"
        );
        a.size = None;
        assert_eq!(
            plain(tray_tip(&[a], Lang::En, Digits::Latn)),
            "Zajil\nDownloading: 1 download · 2 MiB/s"
        );
    }

    /// As the page's `announce`: what finished or failed since the last
    /// look, and a queue whose running downloads all finished; nothing at
    /// the first look, nor for a download paused.
    #[test]
    fn what_finished_since_the_last_look_is_said() {
        let mut last = HashMap::new();
        let first = [
            d(1, "main", StatusView::Active),
            d(2, "main", StatusView::Waiting),
        ];
        assert_eq!(changes(&mut last, &first), Said::default());
        let one_done = [
            d(1, "main", StatusView::Done),
            d(2, "main", StatusView::Active),
        ];
        assert_eq!(
            changes(&mut last, &one_done),
            Said {
                finished: vec![("f1.bin".into(), true)],
                queue: false
            }
        );
        let all_done = [
            d(1, "main", StatusView::Done),
            d(2, "main", StatusView::Done),
        ];
        assert_eq!(
            changes(&mut last, &all_done),
            Said {
                finished: vec![("f2.bin".into(), true)],
                queue: true
            }
        );
        let mut last = HashMap::new();
        changes(&mut last, &[d(3, "night", StatusView::Active)]);
        assert_eq!(
            changes(&mut last, &[d(3, "night", StatusView::Paused)]),
            Said::default()
        );
        assert_eq!(
            changes(&mut last, &[d(3, "night", StatusView::Failed)]),
            Said {
                finished: vec![("f3.bin".into(), false)],
                queue: false
            }
        );
    }

    /// One sound, as the page picks it, and only one the user turned on.
    #[test]
    fn one_sound_as_the_user_chose() {
        use crate::sounds::{Sound, Sounds};
        let all = Sounds {
            done: true,
            failed: true,
            queue: true,
        };
        let said = Said {
            finished: vec![("a".into(), true), ("b".into(), false)],
            queue: true,
        };
        assert_eq!(said.sound(&all), Some(Sound::Queue));
        assert_eq!(
            said.sound(&Sounds {
                queue: false,
                ..all
            }),
            Some(Sound::Failed)
        );
        assert_eq!(
            said.sound(&Sounds {
                done: true,
                failed: false,
                queue: false
            }),
            Some(Sound::Done)
        );
        assert_eq!(Said::default().sound(&all), None);
    }
}
