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
const VIDEO_CONNECTIONS: usize = 8;
const AUDIO_CONNECTIONS: usize = 4;
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
fn container(parts: &[&Format]) -> String {
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

/// Why this way did not finish: before any byte came (yt-dlp may try
/// instead), or after (the parts stay, for a resume).
pub(super) enum Failed {
    Untouched,
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
    pub(super) async fn fast_download(
        &self,
        request: &Download<'_>,
        parts: &[&Format],
        stop: &Stop,
        on_event: &mut (dyn FnMut(VideoEvent) + Send),
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
                .map_err(|_| Failed::Untouched)?;
        }
        let expected: Vec<Option<u64>> = parts
            .iter()
            .map(|f| f.filesize.or(f.filesize_approx))
            .collect();

        // Each part's progress comes through a channel: the parts run at
        // once, and `on_event` is only called here.
        let (send, mut receive) =
            tokio::sync::mpsc::unbounded_channel::<(usize, u64, Option<u64>)>();
        let download = |index: usize| {
            let send = send.clone();
            let url = fast_url(parts[index]).unwrap_or_default().to_owned();
            let name = names[index].clone();
            let connections = if parts.len() > 1 && index == 1 {
                AUDIO_CONNECTIONS
            } else {
                VIDEO_CONNECTIONS
            };
            let engine = self.engine.with_connections(connections);
            async move {
                let mut events = |event: Event| match event {
                    Event::Started { total, from, .. } => {
                        let _ = send.send((index, from, total));
                    }
                    Event::Progress { done, total } => {
                        let _ = send.send((index, done, total));
                    }
                    _ => {}
                };
                part(&engine, &url, dest, &name, stop, &mut events).await
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
                    done[index] = bytes;
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
                result = &mut all => break result,
            }
        };
        let files = match downloaded {
            Ok(files) => files,
            Err(e) => {
                let got_some = done.iter().any(|&d| d > 0);
                let e = engine_error(e);
                return Err(if got_some || matches!(e, VideoError::Stopped) {
                    Failed::Partway(e)
                } else {
                    Failed::Untouched
                });
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

/// One part: continued from its state file if an earlier run left one
/// (with the new link: YouTube's links last hours, not days), else new.
async fn part(
    engine: &Engine,
    url: &str,
    dest: &Path,
    name: &str,
    stop: &Stop,
    on_event: &mut (dyn FnMut(Event) + Send),
) -> Result<PathBuf, crate::engine::Error> {
    let state_file = crate::state::state_path(&dest.join(name));
    if tokio::fs::try_exists(&state_file).await.unwrap_or(false) {
        match engine.refresh_link_until(&state_file, url, stop).await {
            Ok(()) => {
                return engine
                    .resume_until(&state_file, stop, on_event)
                    .await
                    .map(|f| f.file);
            }
            Err(e @ crate::engine::Error::Stopped { .. }) => return Err(e),
            // Another file now, or a broken state: a new download beside it.
            Err(_) => {}
        }
    }
    engine
        .named(Some(name))
        .download_until(url, dest, stop, on_event)
        .await
        .map(|f| f.file)
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
        assert_eq!(container(&got), "mp4");
        assert!(parts(&Choice::Format("22".into()), &formats).is_none());
        assert_eq!(parts(&Choice::AudioOnly, &formats).unwrap()[0].id, "140");
    }

    #[test]
    fn mixed_codecs_go_to_mkv() {
        let v = format("248", "webm", "vp9", "none", YT);
        let a = format("140", "m4a", "none", "mp4a.40.2", YT);
        let o = format("251", "webm", "none", "opus", YT);
        assert_eq!(container(&[&v, &a]), "mkv");
        assert_eq!(container(&[&v, &o]), "webm");
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
