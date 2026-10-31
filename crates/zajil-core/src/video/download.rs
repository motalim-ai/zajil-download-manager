//! A video's download: the run of yt-dlp, its progress, and the finished
//! file put in its place.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::engine::Stop;

use super::args::{Arg, base_args, command_line, environment, tool_args};
use super::files::{checked, is_key, kept_folder, place, remove_kept, without_media_extension};
use super::js::{JS_REPORTS, JsSandbox, js_held, js_reports};
use super::messages::says_drm;
use super::output::read_output;
use super::runner::{Download, Overall, VideoRunner, expected_size};
use super::runs::{COOKIE_FILE, RECORD_FILE, Run, Running, Runs, netscape_cookies, write_private};
use super::{VideoError, VideoEvent, fast};

pub(super) struct Output {
    pub(super) stdout: Vec<u8>,
    placed: Option<io::Result<PathBuf>>,
    status_ok: bool,
    pub(super) error: Option<String>,
    /// yt-dlp said which request handlers it has (`--verbose`): it started
    /// and loaded its libraries.
    pub(super) loaded: bool,
    /// What `zajil-jsbox`'s sandbox held, if it ran.
    js: Option<JsSandbox>,
}

/// Where a finished video goes, and under what name.
pub(super) struct Finish {
    pub(super) dest: PathBuf,
    pub(super) name: String,
    /// The video's own key: yt-dlp works in a folder kept when it stops,
    /// and continues there (see [`kept_folder`]); none, a new one each run.
    pub(super) keep: Option<String>,
    /// The page's origin, for the mark "from the internet" (#90): never its
    /// path or query, which may hold a token.
    pub(super) origin: String,
}

impl Finish {
    /// Marks `file` as from the internet, then moves it into `dest` as
    /// `<name>.<its extension>`, numbered if that is taken, never over
    /// another file. Blocking.
    pub(super) fn place(self, file: &Path) -> io::Result<PathBuf> {
        let ext = file
            .extension()
            .and_then(|e| e.to_str())
            .filter(|e| {
                !e.is_empty() && e.len() <= 8 && e.bytes().all(|b| b.is_ascii_alphanumeric())
            })
            .ok_or_else(|| io::Error::other("yt-dlp's file has no usable extension"))?;
        let base = crate::sanitize::sanitize_file_name(without_media_extension(&self.name));
        let base = if base.is_empty() {
            "video".to_owned()
        } else {
            base
        };
        let wanted = crate::sanitize::sanitize_file_name(&format!("{base}.{ext}"));
        // A file system that cannot keep the mark keeps the file without it.
        if !self.origin.is_empty() {
            let _ = zajil_platform::origin::mark(file, &self.origin);
        }
        for candidate in crate::naming::candidates(&wanted) {
            let target =
                crate::sanitize::confine(&self.dest, &candidate).map_err(io::Error::other)?;
            match zajil_platform::fs::rename_noreplace(file, &target) {
                Ok(()) => {
                    zajil_platform::fs::sync_dir(&self.dest)?;
                    // The tool made it under `umask 077`: the mode of any
                    // other download.
                    let placed = zajil_platform::fs::open_regular_nofollow(&target, false)?;
                    zajil_platform::fs::make_ordinary(&placed)?;
                    return Ok(target);
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            }
        }
        Err(io::Error::other(format!("no free name for {wanted}")))
    }
}

impl VideoRunner {
    /// Downloads `request.choice` of `request.url` into `request.dest`, as
    /// `request.name` (sanitized, with the extension yt-dlp chose,
    /// numbered if taken). Returns the file.
    pub async fn download(
        &self,
        request: &Download<'_>,
        stop: &Stop,
        on_event: &mut (dyn FnMut(VideoEvent) + Send),
    ) -> Result<PathBuf, VideoError> {
        self.download_keeping(request, None, stop, on_event).await
    }

    /// [`VideoRunner::download`], resumable whichever way it goes: `keep`
    /// is the video's own key (letters and digits), and yt-dlp's folder is
    /// kept when it stops or fails, for its next run to continue its parts
    /// (owner's report, 2026-10-30: «the program is made to resume»).
    pub async fn download_keeping(
        &self,
        request: &Download<'_>,
        keep: Option<&str>,
        stop: &Stop,
        on_event: &mut (dyn FnMut(VideoEvent) + Send),
    ) -> Result<PathBuf, VideoError> {
        let keep = keep.filter(|k| is_key(k));
        let Download {
            url,
            choice,
            cookies,
            dest,
            name,
            ..
        } = *request;
        let url = checked(url)?;
        // Checked before any check of the HLS playlists asks for them.
        let choice_args = choice.args()?;
        self.check_hls(request, stop).await?;
        // YouTube: Zajil's own engine, when the probe's links allow it.
        if let Some(parts) = fast::parts(choice, request.formats) {
            let parts: Vec<fast::Part> = parts.into_iter().map(fast::Part::of).collect();
            match self
                .fast_download(request, &parts, stop, on_event, true)
                .await
            {
                Ok(file) => {
                    // What yt-dlp kept from an earlier run is not needed.
                    if let Some(key) = keep {
                        let kept = kept_folder(dest, key);
                        let _ = tokio::task::spawn_blocking(move || remove_kept(&kept)).await;
                    }
                    return Ok(file);
                }
                Err(fast::Failed::Partway(e)) => return Err(e),
                // Nothing came and nothing was saved before, or YouTube
                // refused this way: yt-dlp tries.
                Err(fast::Failed::Untouched(why)) => on_event(VideoEvent::StartedOver(format!(
                    "Zajil's own way could not go on ({why}); yt-dlp downloads it"
                ))),
            }
        }
        let mut args = base_args();
        args.extend([
            Arg::NoPlaylist,
            Arg::NoMtime,
            Arg::Output,
            Arg::Progress,
            Arg::Newline,
            Arg::ProgressTemplate,
            Arg::PrintFinalPath,
        ]);
        args.extend(choice_args);
        let finish = Finish {
            dest: dest.to_owned(),
            name: name.to_owned(),
            origin: url.origin().ascii_serialization(),
            keep: keep.map(str::to_owned),
        };
        let mut overall = Overall::new(expected_size(choice, request.formats));
        let mut on_part = |event| on_event(overall.of(event));
        let output = self
            .run(&url, args, cookies, Some(finish), stop, &mut on_part)
            .await?;
        match output.placed {
            Some(placed) => Ok(placed?),
            None => Err(VideoError::Output("yt-dlp printed no finished file".into())),
        }
    }

    /// One run of yt-dlp, from start to the end of its whole tree; an
    /// error if it failed.
    pub(super) async fn run(
        &self,
        url: &reqwest::Url,
        args: Vec<Arg>,
        cookies: Option<&str>,
        finish: Option<Finish>,
        stop: &Stop,
        on_event: &mut (dyn FnMut(VideoEvent) + Send),
    ) -> Result<Output, VideoError> {
        let finished = self
            .run_unjudged(url, args, cookies, finish, stop, on_event)
            .await?;
        if !finished.status_ok {
            let message = finished
                .error
                .unwrap_or_else(|| "it failed without a message".into());
            return Err(if says_drm(&message) {
                VideoError::Drm
            } else {
                VideoError::Tool(message)
            });
        }
        Ok(finished)
    }

    /// One run of yt-dlp, from start to the end of its whole tree, and what
    /// it said, whether it succeeded or not (a stop is still an error).
    pub(super) async fn run_unjudged(
        &self,
        url: &reqwest::Url,
        mut args: Vec<Arg>,
        cookies: Option<&str>,
        finish: Option<Finish>,
        stop: &Stop,
        on_event: &mut (dyn FnMut(VideoEvent) + Send),
    ) -> Result<Output, VideoError> {
        if stop.is_requested() {
            return Err(VideoError::Stopped);
        }
        // The tools, checked now and held until the whole tree has ended.
        let checked = self.checked_tools().await?;
        args.extend(tool_args(checked.ffmpeg_dir(), self.js.as_ref()));
        let (mut args, extra_env) = self.all_args(args);
        let yt_dlp = checked.yt_dlp.path().to_owned();
        let qjs = checked
            .qjs
            .as_ref()
            .map(|(qjs, sha256)| (qjs.path().to_owned(), sha256.clone()));
        let uses_qjs = qjs.is_some();
        let runs_root = self.runs.root.clone();
        let dest = finish.as_ref().map(|f| (f.dest.clone(), f.keep.clone()));
        let cookie_text = cookies.map(|header| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            netscape_cookies(url, header, now)
        });
        let url_text = url.to_string();
        // Prepare the run's folders and the cookie file, and start the tree.
        // A `Running` from here: whoever drops it (this task's output, if
        // the future is gone) ends the tree before the folders go.
        let running = tokio::task::spawn_blocking(move || -> io::Result<Running> {
            let run = Run::start(
                &Runs::new(runs_root),
                dest.as_ref()
                    .map(|(dest, keep)| (dest.as_path(), keep.as_deref())),
            )?;
            let fail = |run: Run, err: io::Error| {
                let _ = run.close();
                err
            };
            if let Some(text) = cookie_text {
                let file = run.dir.join(COOKIE_FILE);
                if let Err(err) = write_private(&file, text.as_bytes()) {
                    return Err(fail(run, err));
                }
                args.push(Arg::Cookies(file));
            }
            // What `zajil-jsbox` needs: the checked `qjs`, and where to say
            // what its sandbox held.
            let mut js_env = Vec::new();
            if let Some((qjs, sha256)) = qjs {
                let reports = run.dir.join(JS_REPORTS);
                if let Err(err) = zajil_platform::fs::private_dir(&reports) {
                    return Err(fail(run, err));
                }
                js_env.push(("ZAJIL_QJS".to_owned(), qjs.into_os_string()));
                js_env.push(("ZAJIL_QJS_SHA256".to_owned(), sha256.into()));
                js_env.push(("ZAJIL_JSBOX_REPORT".to_owned(), reports.into_os_string()));
            }
            let mut command = Command::new(&yt_dlp);
            command
                .args(command_line(&args, &url_text))
                .current_dir(run.work_dir())
                .env_clear()
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            for (key, value) in environment(&run).into_iter().chain(extra_env).chain(js_env) {
                command.env(key, value);
            }
            let tree = match zajil_platform::process::spawn(command) {
                Ok(tree) => tree,
                Err(err) => return Err(fail(run, err)),
            };
            let running = Running(Some((tree, run)));
            let (tree, run) = running.0.as_ref().expect("just made");
            write_private(
                &run.dir.join(RECORD_FILE),
                tree.record().to_line().as_bytes(),
            )?;
            Ok(running)
        })
        .await
        .map_err(io::Error::other)??;
        let mut running = running;
        let (killer, stdout, stderr, is_download) = {
            let (tree, run) = running.0.as_mut().expect("not taken yet");
            (
                tree.killer(),
                tree.take_stdout().expect("piped"),
                tree.take_stderr().expect("piped"),
                run.out.is_some(),
            )
        };

        // Progress comes through a channel from the blocking reader.
        let (send, mut receive) = tokio::sync::mpsc::unbounded_channel::<VideoEvent>();
        let reader =
            tokio::task::spawn_blocking(move || read_output(stdout, stderr, is_download, send));
        let mut reader = std::pin::pin!(reader);
        let stopped = loop {
            tokio::select! {
                Some(event) = receive.recv() => on_event(event),
                done = &mut reader => break (false, done),
                () = stop.requested() => {
                    // Ending the tree waits for it: on a blocking thread.
                    let killer = killer.clone();
                    let _ = tokio::task::spawn_blocking(move || killer.kill()).await;
                    break (true, reader.as_mut().await);
                }
            }
        };
        while let Ok(event) = receive.try_recv() {
            on_event(event);
        }
        let (was_stopped, read) = stopped;
        let read = read.map_err(io::Error::other)?;
        // Wait for the whole tree, then place the file and remove the
        // run's folders, all on a blocking thread.
        let finished = tokio::task::spawn_blocking(move || -> io::Result<Output> {
            let (mut tree, run) = running.take();
            let status = tree.wait();
            drop(tree);
            let status = match status {
                Ok(status) => status,
                Err(err) => {
                    let _ = run.close();
                    return Err(err);
                }
            };
            let mut output = Output {
                stdout: read.json,
                placed: None,
                status_ok: status.success(),
                error: read.error,
                loaded: read.loaded,
                js: if uses_qjs {
                    js_held(
                        js_reports(&run.dir.join(JS_REPORTS)),
                        zajil_platform::sandbox::supported().into(),
                    )
                } else {
                    None
                },
            };
            if output.status_ok
                && !was_stopped
                && let (Some(out), Some(printed), Some(finish)) =
                    (&run.out, read.final_path, finish)
            {
                // Before the run's folders go: the file is moved out.
                output.placed = Some(place(out, &printed).and_then(|file| finish.place(&file)));
            }
            // A kept folder goes once its video is out.
            let mut run = run;
            if matches!(output.placed, Some(Ok(_))) {
                run.keep_out = false;
            }
            run.close()?;
            Ok(output)
        })
        .await
        .map_err(io::Error::other)??;
        // Until here: on Windows, nobody could change a tool the tree ran.
        drop(checked);
        if let Some(this) = finished.js {
            let mut seen = self.js_sandbox.lock().unwrap_or_else(|e| e.into_inner());
            *seen = Some(this.weakest(*seen));
        }
        if was_stopped {
            return Err(VideoError::Stopped);
        }
        Ok(finished)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AGENTS.md §10.1 and §10.7: the file yt-dlp printed must be a plain
    /// file directly in the output folder (a link to elsewhere is
    /// refused), and a long title whose name is taken is numbered within
    /// the limit.
    #[test]
    fn the_printed_file_must_be_a_plain_file_and_long_titles_fit() {
        let root = tempfile::tempdir().unwrap();
        let out = root.path().join("out");
        let dest = root.path().join("dest");
        let elsewhere = root.path().join("elsewhere.mp4");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(&elsewhere, b"not yours").unwrap();
        let link = out.join("media.mp4");
        zajil_platform::fs::symlink_file(&elsewhere, &link)
            .expect("this test must be able to create a symbolic link");
        assert!(place(&out, &link).is_err(), "a link was taken");
        std::fs::remove_file(&link).unwrap();
        std::fs::hard_link(&elsewhere, &link).unwrap();
        assert!(place(&out, &link).is_err(), "a hard link was taken");
        std::fs::remove_file(&link).unwrap();
        assert_eq!(std::fs::read(&elsewhere).unwrap(), b"not yours");

        for total in [197, 198, 199, 200] {
            let title = format!(
                "{}{}",
                "ع".repeat((total - 4) / 2),
                "x".repeat((total - 4) % 2)
            );
            let taken = format!("{title}.mp4");
            assert_eq!(taken.len(), total);
            std::fs::write(dest.join(&taken), b"the user's").unwrap();
            let file = out.join("media.mp4");
            std::fs::write(&file, b"video").unwrap();
            let placed = place(&out, &file).unwrap();
            let finish = Finish {
                dest: dest.clone(),
                name: title.clone(),
                origin: "https://example.com".into(),
                keep: None,
            };
            let target = finish.place(&placed).unwrap();
            let got = target.file_name().unwrap().to_str().unwrap();
            assert!(got.len() <= crate::sanitize::MAX_NAME_BYTES, "{got}");
            assert!(got.ends_with(".mp4") && got != taken, "{got}");
            assert_eq!(std::fs::read(dest.join(&taken)).unwrap(), b"the user's");
            assert_eq!(std::fs::read(&target).unwrap(), b"video");
        }
    }
}
