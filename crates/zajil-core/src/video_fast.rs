//! The fast way for YouTube (owner's decision, 2026-10-29): the formats
//! yt-dlp's probe found already carry their deciphered links, so the
//! picture and the sound come down through Zajil's own engine, on several
//! connections and resumable like any file, and our ffmpeg puts them
//! together without re-encoding. yt-dlp stays for everything else, and for
//! a choice this way cannot take (an HLS format, another site's links).
//!
//! The parts are written beside the video, as `<name>.f<id>.<ext>` with
//! the engine's own `.zajil-part` and `.zajil-state`: a paused or cut
//! download continues from them, with the new links of a new probe.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::{Choice, Download, Finish, Format, Hidden, VideoError, VideoEvent, VideoRunner};
use crate::engine::{Engine, Event, Stop};
use crate::tools::Tool;

/// Connections for each part: the picture takes most of them.
const VIDEO_CONNECTIONS: usize = 16;
const AUDIO_CONNECTIONS: usize = 8;
/// The most bytes one request asks YouTube for.
const REQUEST_SIZE: u64 = 10 << 20;
/// The longest part of a name kept before `.f<id>.<ext>`, in bytes: with
/// `.zajil-state` it stays far under [`crate::sanitize::MAX_NAME_BYTES`].
const MAX_BASE_BYTES: usize = 120;

/// The link of `format` when this way can take it: an https link on
/// YouTube's own video servers, of a format that is not protected.
fn fast_url(format: &Format) -> Option<&str> {
    let Hidden(url) = format.url.as_ref()?;
    if format.protocol.as_deref() != Some("https") || format.is_protected() {
        return None;
    }
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    (parsed.scheme() == "https" && host.ends_with(".googlevideo.com")).then_some(url.as_str())
}

/// The formats `choice` downloads, when this way can take every one of
/// them; `AudioOnly` takes the best m4a.
pub(super) fn parts<'f>(choice: &Choice, formats: &'f [Format]) -> Option<Vec<&'f Format>> {
    let by_id = |id: &str| formats.iter().find(|f| f.id == id);
    let chosen = match choice {
        Choice::Format(id) => vec![by_id(id)?],
        Choice::Merge { video, audio } => vec![by_id(video)?, by_id(audio)?],
        Choice::AudioOnly => vec![
            formats
                .iter()
                .filter(|f| f.has_audio() && !f.has_video() && f.ext == "m4a")
                .filter(|f| fast_url(f).is_some())
                .max_by(|a, b| {
                    a.tbr
                        .unwrap_or(0.0)
                        .partial_cmp(&b.tbr.unwrap_or(0.0))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })?,
        ],
    };
    chosen
        .iter()
        .all(|f| fast_url(f).is_some() && is_plain(&f.ext))
        .then_some(chosen)
}

/// An extension as a file can end with.
fn is_plain(ext: &str) -> bool {
    !ext.is_empty() && ext.len() <= 8 && ext.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The container of the finished video: MP4 for H.264 or AV1 with AAC,
/// WebM for VP9 or AV1 with Opus, else MKV, which holds anything.
fn container(parts: &[Part]) -> String {
    match parts {
        [one] => one.ext.clone(),
        [video, audio] => match (video.ext.as_str(), audio.ext.as_str()) {
            ("mp4", "m4a" | "mp4") => "mp4".into(),
            ("webm", "webm") => "webm".into(),
            _ => "mkv".into(),
        },
        _ => "mkv".into(),
    }
}

/// `name` cut to [`MAX_BASE_BYTES`] at a character's edge, and sanitized.
fn base_of(name: &str) -> String {
    let base = crate::sanitize::sanitize_file_name(super::without_media_extension(name));
    let mut end = base.len().min(MAX_BASE_BYTES);
    while !base.is_char_boundary(end) {
        end -= 1;
    }
    let cut = base[..end].trim_end();
    if cut.is_empty() {
        "video".into()
    } else {
        cut.into()
    }
}

/// One part to download: its format id and extension, its new link (none:
/// continued from its state file with the link saved there), its size if
/// known, and the program its link was made for.
pub(super) struct Part {
    pub id: String,
    pub ext: String,
    pub url: Option<String>,
    pub size: Option<u64>,
    pub agent: Option<String>,
}

impl Part {
    pub(super) fn of(format: &Format) -> Part {
        Part {
            id: format.id.clone(),
            ext: format.ext.clone(),
            url: fast_url(format).map(str::to_owned),
            size: format.filesize.or(format.filesize_approx),
            agent: format.user_agent().map(str::to_owned),
        }
    }
}

/// How long a saved link must still be good for to be used without
/// asking the page again: YouTube's last about six hours (`expire=`).
const LINK_MARGIN_SECS: u64 = 10 * 60;

/// Whether `url` is a YouTube video server's link still good for a while,
/// by its own `expire=` (seconds since 1970).
fn link_still_good(url: &str, now_unix: u64) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if parsed.scheme() != "https" || !host.ends_with(".googlevideo.com") {
        return false;
    }
    parsed
        .query_pairs()
        .find(|(k, _)| k == "expire")
        .and_then(|(_, v)| v.parse::<u64>().ok())
        .is_some_and(|expire| expire > now_unix.saturating_add(LINK_MARGIN_SECS))
}

/// Why a new link is not the file a saved part is of: YouTube names each
/// file in its links by its format (`itag`), its exact size (`clen`) and
/// when it was made (`lmt`); none when they agree (or a link does not say).
fn not_the_same(old: &str, new: &str) -> Option<String> {
    let params = |url: &str| -> std::collections::HashMap<String, String> {
        reqwest::Url::parse(url)
            .map(|u| {
                u.query_pairs()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect()
            })
            .unwrap_or_default()
    };
    let (old, new) = (params(old), params(new));
    for (key, what) in [("itag", "format"), ("clen", "size"), ("lmt", "version")] {
        if let (Some(a), Some(b)) = (old.get(key), new.get(key))
            && a != b
        {
            return Some(format!(
                "the video's {what} on YouTube is not the one downloaded before ({key} {a} → {b})"
            ));
        }
    }
    None
}

/// Why this way did not finish: before any byte came and with nothing
/// saved before, or refused by YouTube (yt-dlp may try instead), or after
/// (the parts stay, for a resume).
pub(super) enum Failed {
    Untouched(String),
    Partway(VideoError),
}

fn engine_error(e: crate::engine::Error) -> VideoError {
    match e {
        crate::engine::Error::Stopped { .. } => VideoError::Stopped,
        e => VideoError::Tool(format!(
            "Zajil's download of the video failed: {}",
            super::shown_message(&e.to_string())
        )),
    }
}

impl VideoRunner {
    /// Downloads `parts` of `request` with the engine, then merges them.
    /// Refused by YouTube, the parts are removed if `give_up_parts` (yt-dlp
    /// downloads it then), else kept (new links may be asked for).
    pub(super) async fn fast_download(
        &self,
        request: &Download<'_>,
        parts: &[Part],
        stop: &Stop,
        on_event: &mut (dyn FnMut(VideoEvent) + Send),
        give_up_parts: bool,
    ) -> Result<PathBuf, Failed> {
        let dest = request.dest;
        let base = base_of(request.name);
        let names: Vec<String> = parts
            .iter()
            .map(|f| crate::sanitize::sanitize_file_name(&format!("{base}.f{}.{}", f.id, f.ext)))
            .collect();
        if !dest.is_dir() {
            tokio::fs::create_dir_all(dest)
                .await
                .map_err(|e| Failed::Untouched(e.to_string()))?;
        }
        let expected: Vec<Option<u64>> = parts.iter().map(|p| p.size).collect();
        // For the log: which of YouTube's apps each new link is of (a 403
        // may depend on it), never the link.
        for part in parts {
            if let Some(url) = &part.url {
                on_event(VideoEvent::Note(format!(
                    "part {}: a new link of YouTube's client {}",
                    part.id,
                    client_of(url).unwrap_or_else(|| "?".into())
                )));
            }
        }
        // What an earlier run left (a part, or a part that finished): never
        // handed to yt-dlp, which would start it all over (owner's report,
        // 2026-10-30: a resumed video began again from nothing).
        let saved_before = {
            let files: Vec<PathBuf> = names.iter().map(|n| dest.join(n)).collect();
            tokio::task::spawn_blocking(move || {
                files
                    .iter()
                    .any(|f| f.is_file() || crate::state::state_path(f).symlink_metadata().is_ok())
            })
            .await
            .unwrap_or(true)
        };

        // Each part's progress comes through a channel: the parts run at
        // once, and `on_event` is only called here.
        let (send, mut receive) =
            tokio::sync::mpsc::unbounded_channel::<(usize, Option<u64>, Option<u64>)>();
        let (said, mut why) = tokio::sync::mpsc::unbounded_channel::<String>();
        let (noted, mut notes) = tokio::sync::mpsc::unbounded_channel::<String>();
        let download = |index: usize| {
            let send = send.clone();
            let said = said.clone();
            let noted = noted.clone();
            let url = parts[index].url.clone();
            let size = parts[index].size;
            let name = names[index].clone();
            let connections = if parts.len() > 1 && index == 1 {
                AUDIO_CONNECTIONS
            } else {
                VIDEO_CONNECTIONS
            };
            // In parts of at most 10 MiB, as yt-dlp asks YouTube: a request
            // for more is slowed (owner's report, 2026-10-29).
            let engine = self
                .engine
                .with_user_agent(parts[index].agent.as_deref())
                .with_connections(connections)
                .with_request_size(Some(REQUEST_SIZE));
            async move {
                let restarted = said.clone();
                let busy = noted.clone();
                let mut events = |event: Event| match event {
                    // Its size only: a continued part's `from` is where its
                    // first gap starts, not all it has.
                    Event::Started { total, .. } => {
                        let _ = send.send((index, None, total));
                    }
                    Event::Progress { done, total } => {
                        let _ = send.send((index, Some(done), total));
                    }
                    Event::Restarted { reason } => {
                        let _ = restarted
                            .send(format!("the part saved before started over ({reason:?})"));
                    }
                    Event::ServerBusy {
                        status,
                        connections,
                        ..
                    } => {
                        let _ = busy.send(format!(
                            "part {index}: YouTube answered {status}; {connections} connections from now on"
                        ));
                    }
                    _ => {}
                };
                part(
                    &engine,
                    url.as_deref(),
                    size,
                    dest,
                    &name,
                    stop,
                    &mut events,
                    &said,
                )
                .await
            }
        };
        let all = async {
            match parts.len() {
                1 => download(0).await.map(|file| vec![file]),
                _ => {
                    let (video, audio) = tokio::try_join!(download(0), download(1))?;
                    Ok(vec![video, audio])
                }
            }
        };
        let mut all = std::pin::pin!(all);
        let mut done = vec![0u64; parts.len()];
        let mut totals = expected.clone();
        let mut speed = Speed::default();
        let downloaded = loop {
            tokio::select! {
                Some((index, bytes, total)) = receive.recv() => {
                    if let Some(bytes) = bytes {
                        done[index] = bytes;
                    }
                    if total.is_some() {
                        totals[index] = total;
                    }
                    let so_far: u64 = done.iter().sum();
                    let whole = totals.iter().copied().sum::<Option<u64>>();
                    on_event(VideoEvent::Progress {
                        done: so_far,
                        total: whole.map(|w| w.max(so_far)),
                        speed: speed.of(so_far),
                    });
                }
                Some(reason) = why.recv() => on_event(VideoEvent::StartedOver(reason)),
                Some(note) = notes.recv() => on_event(VideoEvent::Note(note)),
                result = &mut all => break result,
            }
        };
        // What was said as the parts ended.
        while let Ok(reason) = why.try_recv() {
            on_event(VideoEvent::StartedOver(reason));
        }
        while let Ok(note) = notes.try_recv() {
            on_event(VideoEvent::Note(note));
        }
        let files = match downloaded {
            Ok(files) => files,
            Err(e) => {
                let refused = refused(&e);
                let got_some = saved_before || done.iter().any(|&d| d > 0);
                let e = engine_error(e);
                if matches!(e, VideoError::Stopped) || (got_some && !refused) {
                    return Err(Failed::Partway(e));
                }
                // Refused (owner's report, 2026-10-30: 403 on every resume,
                // the video stopped): yt-dlp gets it, so the parts would
                // only be left behind.
                if refused && give_up_parts {
                    let files: Vec<PathBuf> = names.iter().map(|n| dest.join(n)).collect();
                    let _ = tokio::task::spawn_blocking(move || {
                        for file in files {
                            for path in [
                                crate::state::part_path(&file),
                                crate::state::state_path(&file),
                                file,
                            ] {
                                let _ = zajil_platform::fs::remove_file_retrying(&path);
                            }
                        }
                    })
                    .await;
                }
                return Err(Failed::Untouched(e.to_string()));
            }
        };

        // Put together by our ffmpeg, checked now, into a name of its own,
        // then placed as `<name>.<ext>`.
        let ffmpeg = self
            .tools
            .verified(Tool::Ffmpeg)
            .await
            .map_err(|e| Failed::Partway(e.into()))?;
        let ext = container(parts);
        let merged = dest.join(format!(
            "{base}.zajil-merge-{}.{ext}",
            super::random_hex().map_err(|e| Failed::Partway(e.into()))?
        ));
        merge(ffmpeg.path(), &files, &merged, &ext, stop)
            .await
            .map_err(Failed::Partway)?;
        drop(ffmpeg);
        let finish = Finish {
            dest: dest.to_owned(),
            name: request.name.to_owned(),
            origin: reqwest::Url::parse(request.url)
                .map(|u| u.origin().ascii_serialization())
                .unwrap_or_default(),
            keep: None,
        };
        tokio::task::spawn_blocking(move || {
            let placed = finish.place(&merged);
            if placed.is_err() {
                let _ = zajil_platform::fs::remove_file_retrying(&merged);
            } else {
                for file in &files {
                    let _ = zajil_platform::fs::remove_file_retrying(file);
                }
            }
            placed
        })
        .await
        .map_err(|e| Failed::Partway(io::Error::other(e).into()))?
        .map_err(|e| Failed::Partway(e.into()))
    }
}

/// One part: continued from its state file if an earlier run left one,
/// with `url` when given (a new link of the same file: YouTube's links
/// last hours, not days), else with the link saved there; kept if it
/// finished before (the sound ends long before the picture); new
/// otherwise. A saved part of another file (`not_the_same`) is removed,
/// said on `said`, and downloaded again from the start.
#[allow(clippy::too_many_arguments)]
async fn part(
    engine: &Engine,
    url: Option<&str>,
    size: Option<u64>,
    dest: &Path,
    name: &str,
    stop: &Stop,
    on_event: &mut (dyn FnMut(Event) + Send),
    said: &tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<PathBuf, crate::engine::Error> {
    let file = dest.join(name);
    let state_file = crate::state::state_path(&file);
    let has_state = tokio::fs::try_exists(&state_file).await.unwrap_or(false);
    if !has_state && let Some(bytes) = finished_part(&file, url, size).await {
        on_event(Event::Progress {
            done: bytes,
            total: Some(bytes),
        });
        return Ok(file);
    }
    if has_state {
        let Some(url) = url else {
            return engine
                .resume_until(&state_file, stop, on_event)
                .await
                .map(|f| f.file);
        };
        let saved = state_file.clone();
        let old = tokio::task::spawn_blocking(move || crate::state::DownloadState::load(&saved))
            .await
            .ok()
            .and_then(Result::ok);
        let other = old.as_ref().and_then(|old| not_the_same(&old.url, url));
        if other.is_none() {
            match engine.refresh_link_until(&state_file, url, stop).await {
                Ok(()) => {
                    return engine
                        .resume_until(&state_file, stop, on_event)
                        .await
                        .map(|f| f.file);
                }
                Err(e @ crate::engine::Error::Stopped { .. }) => return Err(e),
                Err(e) => {
                    let _ = said.send(format!(
                        "the part saved before could not be continued ({}); it starts over",
                        super::shown_message(&e.to_string())
                    ));
                }
            }
        } else if let Some(why) = other {
            let _ = said.send(format!("{why}; it starts over"));
        }
        // Removed, so the new one takes its name.
        let part_file = crate::state::part_path(&file);
        let _ = tokio::task::spawn_blocking(move || {
            let _ = zajil_platform::fs::remove_file_retrying(&part_file);
            zajil_platform::fs::remove_file_retrying(&state_file)
        })
        .await;
    }
    let Some(url) = url else {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no saved part and no link").into());
    };
    engine
        .named(Some(name))
        .download_until(url, dest, stop, on_event)
        .await
        .map(|f| f.file)
}

/// Whether YouTube refused the download (`403`, or `429` for too long),
/// as it does some links or too many requests: yt-dlp may still get it.
fn refused(e: &crate::engine::Error) -> bool {
    match e {
        crate::engine::Error::Status(status) => matches!(status.as_u16(), 403 | 429),
        // A part's connections end with the server's answer in words.
        e => {
            let said = e.to_string();
            said.contains("403 Forbidden") || said.contains("429 Too Many Requests")
        }
    }
}

/// The YouTube client a link was made for (its `c=`: `WEB`, `ANDROID_VR`…).
fn client_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()?
        .query_pairs()
        .find(|(k, _)| k == "c")
        .map(|(_, v)| {
            v.chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .take(32)
                .collect()
        })
}

/// The size of `file` when it is a part that finished before: a plain
/// file of the size `url` names (`clen`), or of `size` when continued
/// with no new link (as the run that finished it saved it).
async fn finished_part(file: &Path, url: Option<&str>, size: Option<u64>) -> Option<u64> {
    let meta = tokio::fs::symlink_metadata(file).await.ok()?;
    if !meta.is_file() {
        return None;
    }
    let wanted = match url {
        Some(url) => exact_size(url)?,
        None => size?,
    };
    (meta.len() == wanted).then_some(wanted)
}

/// The exact size YouTube's link names (`clen`).
fn exact_size(url: &str) -> Option<u64> {
    reqwest::Url::parse(url)
        .ok()?
        .query_pairs()
        .find(|(k, _)| k == "clen")
        .and_then(|(_, v)| v.parse().ok())
}

/// A video Zajil downloaded its own way and stopped: its page, the ids of
/// its parts, where they are, the video's name, and the program each
/// part's link was made for (by id).
#[derive(Debug, Clone, Copy)]
pub struct Saved<'a> {
    pub page: &'a str,
    pub ids: &'a [String],
    pub dest: &'a Path,
    pub name: &'a str,
    pub agents: &'a [(String, String)],
}

impl VideoRunner {
    /// Continues `saved` from its parts' state files, with the links saved
    /// there, without asking the page again (no yt-dlp): while they are
    /// still good, and every part has its state file or finished (owner's
    /// report, 2026-10-30: with the sound finished, a paused video asked
    /// the page again and began from nothing). None when it cannot
    /// (the caller asks the page for new links then, and
    /// [`VideoRunner::download`] continues the parts with them).
    pub async fn resume_saved(
        &self,
        saved: &Saved<'_>,
        stop: &Stop,
        on_event: &mut (dyn FnMut(VideoEvent) + Send),
    ) -> Option<Result<PathBuf, VideoError>> {
        if saved.ids.is_empty()
            || saved
                .ids
                .iter()
                .any(|id| super::check_format_id(id).is_err())
        {
            return None;
        }
        let (dest, name, ids, agents) = (
            saved.dest.to_owned(),
            saved.name.to_owned(),
            saved.ids.to_vec(),
            saved.agents.to_vec(),
        );
        let agent_of = move |id: &str| {
            agents
                .iter()
                .find(|(of, _)| of == id)
                .map(|(_, agent)| agent.clone())
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(u64::MAX);
        let parts = tokio::task::spawn_blocking(move || -> Option<Vec<Part>> {
            let files = part_files(&dest, &name);
            let prefix = format!("{}.f", base_of(&name));
            ids.iter()
                .map(|id| {
                    if let Some((file, size)) = finished_file(&dest, &name, id) {
                        let ext = file.rsplit_once('.')?.1.to_owned();
                        return Some(Part {
                            id: id.clone(),
                            ext,
                            url: None,
                            size: Some(size),
                            agent: None,
                        });
                    }
                    let state = files.iter().find(|f| {
                        f.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                            n.ends_with(".zajil-state")
                                && n.strip_prefix(&prefix)
                                    .and_then(|rest| rest.split_once('.'))
                                    .is_some_and(|(found, _)| found == id)
                        })
                    })?;
                    let loaded = crate::state::DownloadState::load(state).ok()?;
                    if !link_still_good(&loaded.final_url, now) {
                        return None;
                    }
                    let file = state.file_name()?.to_str()?.strip_suffix(".zajil-state")?;
                    let ext = file.rsplit_once('.')?.1.to_owned();
                    Some(Part {
                        id: id.clone(),
                        ext,
                        url: None,
                        size: loaded.content_length,
                        agent: agent_of(id),
                    })
                })
                .collect()
        })
        .await
        .ok()??;
        let request = Download {
            url: saved.page,
            choice: &Choice::AudioOnly,
            formats: &[],
            cookies: None,
            dest: saved.dest,
            name: saved.name,
        };
        match self
            .fast_download(&request, &parts, stop, on_event, false)
            .await
        {
            Ok(file) => Some(Ok(file)),
            Err(Failed::Partway(VideoError::Stopped)) => Some(Err(VideoError::Stopped)),
            // A saved link refused (or a part saved without the program it
            // was made for): the page is asked for new ones, and the parts
            // continue with them.
            Err(Failed::Partway(_) | Failed::Untouched(_)) => None,
        }
    }
}

/// The part `id` of `name` in `dest` that finished, with no part or state
/// file beside it (`<base>.f<id>.<ext>`): its file name and size.
fn finished_file(dest: &Path, name: &str, id: &str) -> Option<(String, u64)> {
    let start = format!("{}.f{id}.", base_of(name));
    std::fs::read_dir(dest)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e)))
        .filter(|(file, _)| {
            file.strip_prefix(&start).is_some_and(is_plain)
                && crate::state::state_path(&dest.join(file))
                    .symlink_metadata()
                    .is_err()
        })
        .find_map(|(file, entry)| {
            let meta = entry.metadata().ok()?;
            meta.is_file().then_some((file, meta.len()))
        })
}

/// The `.zajil-part` and `.zajil-state` files of the parts of `name` in
/// `dest`: `<base>.f<id>.<ext>.zajil-part`, with an id and an extension of
/// letters, digits and `-` only.
pub(super) fn part_files(dest: &Path, name: &str) -> Vec<PathBuf> {
    let start = format!("{}.f", base_of(name));
    let Ok(entries) = std::fs::read_dir(dest) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|file| {
            let Some(rest) = file.strip_prefix(&start) else {
                return false;
            };
            let Some(rest) = rest
                .strip_suffix(".zajil-part")
                .or_else(|| rest.strip_suffix(".zajil-state"))
            else {
                return false;
            };
            rest.split_once('.').is_some_and(|(id, ext)| {
                !id.is_empty()
                    && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && is_plain(ext)
            })
        })
        .map(|file| dest.join(file))
        .collect()
}

/// The speed over the last few seconds, from the bytes so far.
#[derive(Default)]
struct Speed {
    samples: std::collections::VecDeque<(std::time::Instant, u64)>,
}

impl Speed {
    fn of(&mut self, so_far: u64) -> Option<u64> {
        let now = std::time::Instant::now();
        self.samples.push_back((now, so_far));
        while self
            .samples
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at).as_secs_f64() > 3.0)
        {
            self.samples.pop_front();
        }
        let (at, bytes) = *self.samples.front()?;
        let secs = now.duration_since(at).as_secs_f64();
        (secs >= 0.2).then(|| (so_far.saturating_sub(bytes) as f64 / secs) as u64)
    }
}

/// A path ffmpeg never reads as an option.
fn not_an_option(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        Path::new(".").join(path)
    }
}

/// `inputs` (a picture and a sound, or one file) copied into `out` as is.
async fn merge(
    ffmpeg: &Path,
    inputs: &[PathBuf],
    out: &Path,
    ext: &str,
    stop: &Stop,
) -> Result<(), VideoError> {
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y"]);
    for input in inputs {
        command.arg("-i").arg(not_an_option(input));
    }
    if inputs.len() > 1 {
        command.args(["-map", "0:v:0", "-map", "1:a:0"]);
    } else {
        command.args(["-map", "0"]);
    }
    command.args(["-c", "copy"]);
    if matches!(ext, "mp4" | "m4a" | "mov") {
        command.args(["-movflags", "+faststart"]);
    }
    command
        .arg(not_an_option(out))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut tree = tokio::task::spawn_blocking(move || zajil_platform::process::spawn(command))
        .await
        .map_err(io::Error::other)??;
    let killer = tree.killer();
    let stderr = tree.take_stderr();
    let waiting = tokio::task::spawn_blocking(move || {
        let mut said = String::new();
        if let Some(stderr) = stderr {
            let _ = stderr.take(64 * 1024).read_to_string(&mut said);
        }
        (tree.wait(), said)
    });
    let mut waiting = std::pin::pin!(waiting);
    let stopped = tokio::select! {
        ended = &mut waiting => Some(ended),
        () = stop.requested() => None,
    };
    let (status, said) = match stopped {
        Some(ended) => ended.map_err(io::Error::other)?,
        None => {
            let _ = tokio::task::spawn_blocking(move || killer.kill()).await;
            let _ = waiting.await;
            let out = out.to_owned();
            let _ =
                tokio::task::spawn_blocking(move || zajil_platform::fs::remove_file_retrying(&out))
                    .await;
            return Err(VideoError::Stopped);
        }
    };
    if status?.success() {
        Ok(())
    } else {
        let out = out.to_owned();
        let _ = tokio::task::spawn_blocking(move || zajil_platform::fs::remove_file_retrying(&out))
            .await;
        Err(VideoError::Tool(format!(
            "ffmpeg could not put the video together: {}",
            super::shown_message(said.trim())
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(id: &str, ext: &str, v: &str, a: &str, url: &str) -> Format {
        serde_json::from_value(serde_json::json!({
            "format_id": id, "ext": ext, "vcodec": v, "acodec": a,
            "protocol": "https", "url": url,
        }))
        .unwrap()
    }

    const YT: &str = "https://rr1---sn-abc.googlevideo.com/videoplayback?itag=1";

    #[test]
    fn takes_youtube_links_only() {
        let formats = [
            format("137", "mp4", "avc1.640028", "none", YT),
            format("140", "m4a", "none", "mp4a.40.2", YT),
            format("22", "mp4", "avc1", "mp4a", "https://example.com/v.mp4"),
        ];
        let merge = Choice::Merge {
            video: "137".into(),
            audio: "140".into(),
        };
        let got = parts(&merge, &formats).unwrap();
        assert_eq!(got.len(), 2);
        let got: Vec<Part> = got.into_iter().map(Part::of).collect();
        assert_eq!(container(&got), "mp4");
        assert!(parts(&Choice::Format("22".into()), &formats).is_none());
        assert_eq!(parts(&Choice::AudioOnly, &formats).unwrap()[0].id, "140");
    }

    /// Of a format's `http_headers`, only the program is kept: YouTube
    /// refuses some of its apps' links to another one.
    #[test]
    fn a_format_keeps_its_program_and_no_other_header() {
        let format: Format = serde_json::from_value(serde_json::json!({
            "format_id": "137", "ext": "mp4", "protocol": "https", "url": YT,
            "http_headers": {"User-Agent": "App/1.0", "Cookie": "secret=1"},
        }))
        .unwrap();
        assert_eq!(format.user_agent(), Some("App/1.0"));
        assert!(!format!("{format:?}").contains("secret"));
        assert_eq!(Part::of(&format).agent.as_deref(), Some("App/1.0"));
        let bare = super::super::Format {
            agent: None,
            ..format
        };
        assert_eq!(bare.user_agent(), None);
    }

    /// YouTube's refusal lets yt-dlp try; another failure keeps the parts.
    #[test]
    fn a_refusal_is_told_from_other_failures() {
        use crate::engine::Error;
        assert!(refused(&Error::Status(reqwest::StatusCode::FORBIDDEN)));
        assert!(refused(&Error::NoData(
            "the server answered 403 Forbidden".into()
        )));
        assert!(!refused(&Error::Status(reqwest::StatusCode::NOT_FOUND)));
        assert!(!refused(&Error::NoData(
            "the server stopped sending parts of the file".into()
        )));
        assert_eq!(
            client_of(&format!("{YT}&c=ANDROID_VR")).as_deref(),
            Some("ANDROID_VR")
        );
        assert_eq!(client_of(&format!("{YT}&c=W%0AEB")).as_deref(), Some("WEB"));
        assert_eq!(client_of(YT), None);
    }

    #[test]
    fn mixed_codecs_go_to_mkv() {
        let v = Part::of(&format("248", "webm", "vp9", "none", YT));
        let a = Part::of(&format("140", "m4a", "none", "mp4a.40.2", YT));
        let o = Part::of(&format("251", "webm", "none", "opus", YT));
        assert_eq!(container(&[v, a]), "mkv");
        let v = Part::of(&format("248", "webm", "vp9", "none", YT));
        assert_eq!(container(&[v, o]), "webm");
    }

    /// A saved part goes on with a new link only if YouTube's link names
    /// the same file: the same format, exact size and version.
    #[test]
    fn a_new_link_must_name_the_same_file() {
        let old =
            "https://rr1---sn-a.googlevideo.com/videoplayback?itag=137&clen=100&lmt=5&expire=1";
        let same =
            "https://rr4---sn-b.googlevideo.com/videoplayback?expire=9&lmt=5&clen=100&itag=137";
        assert_eq!(not_the_same(old, same), None);
        let bigger = same.replace("clen=100", "clen=101");
        assert!(not_the_same(old, &bigger).unwrap().contains("size"));
        let newer = same.replace("lmt=5", "lmt=6");
        assert!(not_the_same(old, &newer).unwrap().contains("version"));
    }

    #[test]
    fn a_saved_link_is_used_only_while_it_is_good() {
        let link = |expire: u64| {
            format!("https://rr1---sn-a.googlevideo.com/videoplayback?itag=137&expire={expire}")
        };
        assert!(link_still_good(&link(10_000), 1_000));
        assert!(!link_still_good(&link(1_300), 1_000), "within the margin");
        assert!(!link_still_good(&link(900), 1_000));
        assert!(!link_still_good(
            "https://example.com/v.mp4?expire=99999999999",
            1_000
        ));
        assert!(!link_still_good(
            &format!("{}{}", link(10_000), "&expire=x").replace("expire=10000&", ""),
            1_000
        ));
    }

    #[test]
    fn only_this_videos_parts_are_found() {
        let dir = tempfile::tempdir().unwrap();
        for file in [
            "A b.f299.mp4.zajil-part",
            "A b.f299.mp4.zajil-state",
            "A b.f140-1.m4a.zajil-part",
            "A b.mp4",
            "A b.f299.mp4",
            "A bc.f299.mp4.zajil-part",
            "Other.f140.m4a.zajil-part",
        ] {
            std::fs::write(dir.path().join(file), b"").unwrap();
        }
        let mut found: Vec<String> = part_files(dir.path(), "A b")
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        found.sort();
        assert_eq!(
            found,
            [
                "A b.f140-1.m4a.zajil-part",
                "A b.f299.mp4.zajil-part",
                "A b.f299.mp4.zajil-state"
            ]
        );
    }

    /// The sound that finished before the pause is kept for the resume,
    /// and only it: not a part still going, nor another format's.
    #[test]
    fn a_part_that_finished_is_found() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("A b.f140.m4a"), b"sound").unwrap();
        std::fs::write(dir.path().join("A b.f1400.m4a"), b"other").unwrap();
        std::fs::write(dir.path().join("A b.f137.mp4"), b"").unwrap();
        std::fs::write(dir.path().join("A b.f137.mp4.zajil-state"), b"").unwrap();
        assert_eq!(
            finished_file(dir.path(), "A b", "140"),
            Some(("A b.f140.m4a".into(), 5))
        );
        assert_eq!(finished_file(dir.path(), "A b", "137"), None);
        assert_eq!(finished_file(dir.path(), "A b", "251"), None);
    }

    #[tokio::test]
    async fn a_finished_part_is_kept_only_at_its_size() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("A b.f140.m4a");
        std::fs::write(&file, b"sound").unwrap();
        let link = |clen: u64| format!("{YT}&clen={clen}");
        assert_eq!(finished_part(&file, Some(&link(5)), None).await, Some(5));
        assert_eq!(finished_part(&file, Some(&link(6)), Some(5)).await, None);
        assert_eq!(finished_part(&file, Some(YT), Some(5)).await, None);
        assert_eq!(finished_part(&file, None, Some(5)).await, Some(5));
        assert_eq!(finished_part(&file, None, None).await, None);
        assert_eq!(
            finished_part(&dir.path().join("none.m4a"), None, Some(5)).await,
            None
        );
    }

    #[test]
    fn long_arabic_names_are_cut_at_a_character() {
        let name = "فيديو ".repeat(60);
        let base = base_of(&name);
        assert!(base.len() <= MAX_BASE_BYTES);
        let part = format!("{base}.f137.mp4.zajil-state");
        assert!(part.len() <= crate::sanitize::MAX_NAME_BYTES);
    }
}
