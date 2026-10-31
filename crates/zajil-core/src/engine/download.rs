//! Starting a download: the first request, the file's place, and the
//! connections that fetch it.

use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use reqwest::{Response, StatusCode, Url};

use crate::limit::Limits;
use crate::naming::reserve_target;
use crate::segmented::{self, Job, Outcome};
use crate::state::{DownloadState, part_path, state_path};

#[cfg(feature = "test-support")]
use super::engine_test_hooks;
use super::files::{blocking, check_space, lock, save_state};
use super::http::{if_range_value, parse_url, state_from_response, without_credentials};
use super::probe::{is_bare_page, response_name};
use super::transfer::{EmptyDownloadCleanup, refused, resumable_stop};
use super::{Engine, Error, Event, Finished, RestartReason, Stop};
#[cfg(feature = "test-support")]
use std::io;

/// How many times one call may start over because the file changed on the
/// server, before giving up for now.
const MAX_RESTARTS: u8 = 3;

/// What one call of the engine may still do as it starts over: how many
/// restarts are left, whether the file may still be split, and how many
/// requests or connections failed in a row. Not `Debug`: it holds the user
/// name and password of the URL.
#[derive(Clone)]
pub(super) struct Attempt {
    restarts_left: u8,
    split: bool,
    /// The URL the user gave, if it has a user name or password. Kept in
    /// memory only, never in the state file (ADR-0004, rule 7), so range
    /// requests carry them too.
    pub(super) credentials: Option<Url>,
    /// Failures in a row (requests, and connections of a single-connection
    /// download) since the download last reached further into the file.
    pub(super) failures: u32,
    /// The furthest byte a single connection has written in this call. A
    /// connection that breaks before passing it made no progress: a server
    /// that cannot continue from the middle and always breaks at the same
    /// place is given up, not tried forever.
    furthest: u64,
    /// Set when the caller wants the download to stop ([`Stop`]).
    pub(super) stop: Stop,
    /// The speed limits every byte of this download goes through.
    pub(super) limits: Limits,
    /// The counter of the download's site, if the user limited it
    /// ([`crate::sites`]): further connections of a split download take a
    /// slot of it each.
    pub(super) site: Option<Arc<tokio::sync::Semaphore>>,
}

impl Attempt {
    pub(super) fn new(url: Option<&Url>, stop: &Stop, limits: &Limits) -> Self {
        Attempt {
            stop: stop.clone(),
            limits: limits.clone(),
            restarts_left: MAX_RESTARTS,
            split: true,
            credentials: url
                .filter(|url| !url.username().is_empty() || url.password().is_some())
                .cloned(),
            failures: 0,
            furthest: 0,
            site: None,
        }
    }

    pub(super) fn with_site(mut self, site: Option<Arc<tokio::sync::Semaphore>>) -> Self {
        self.site = site;
        self
    }

    /// Counts a failure of a single connection that had written up to
    /// `done`. `false` when the download must give up: `max_failures`
    /// failures in a row without reaching further into the file.
    pub(super) fn connection_failed(&mut self, done: u64, max_failures: u32) -> bool {
        if done > self.furthest {
            self.furthest = done;
            self.failures = 0;
        }
        self.failed(max_failures)
    }

    /// Counts a failure; `false` once there were `max_failures` in a row.
    pub(super) fn failed(&mut self, max_failures: u32) -> bool {
        self.failures = self.failures.saturating_add(1);
        self.failures < max_failures
    }

    /// For a start from byte 0 after a broken connection on a server that
    /// cannot continue from the middle: that is not a new version of the
    /// file, so it does not use up one of the restarts that [`Engine::start`]
    /// counts.
    pub(super) fn after_broken_connection(self) -> Self {
        Attempt {
            restarts_left: self.restarts_left.saturating_add(1),
            ..self
        }
    }

    /// `url` with the user's credentials, if it is on the same origin as the
    /// URL that had them (as `reqwest` keeps them only within one origin).
    pub(super) fn authorize(&self, url: &Url) -> Url {
        let mut url = url.clone();
        if let Some(with) = &self.credentials
            && with.origin() == url.origin()
        {
            let _ = url.set_username(with.username());
            let _ = url.set_password(with.password());
        }
        url
    }
}

impl Engine {
    /// Downloads `url` into the folder `dir` (created if it does not exist,
    /// since M4: queue items whose folder was removed, and folders kept from
    /// an imported file).
    pub async fn download(
        &self,
        url: &str,
        dir: &Path,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        self.download_until(url, dir, &Stop::new(), on_event).await
    }

    /// Like [`Engine::download`], until `stop` is requested: then what is on
    /// disk is synced and recorded (unless a periodic save already did:
    /// the state file counts only synced bytes, #132), and the call returns
    /// [`Error::Stopped`] only once nothing of the download runs any more
    /// (no write, no save, the part file closed and unlocked). So the
    /// download can be resumed at once in the same process, and no older
    /// save can land after the resume's own (#5).
    ///
    /// Dropping the future instead also stops it, but its last writes and
    /// saves may still finish on blocking threads afterwards.
    pub async fn download_until(
        &self,
        url: &str,
        dir: &Path,
        stop: &Stop,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        self.download_choosing_until(url, &|_, _| dir.to_owned(), stop, on_event)
            .await
    }

    /// Like [`Engine::download_until`], but the folder is chosen by
    /// `choose` once the file's name and the final URL (after redirects)
    /// are known, before any file is created: categories (M4) need both.
    /// The chosen folder is created if it does not exist.
    pub async fn download_choosing_until(
        &self,
        url: &str,
        choose: &(dyn Fn(&str, &reqwest::Url) -> PathBuf + Send + Sync),
        stop: &Stop,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        let url = parse_url(url)?;
        // The download's own slot of its site, held until it ends.
        let site = self.sites.counter_for(&url);
        let _slot = crate::sites::take_slot(site.as_ref(), stop).await?;
        let mut attempt = Attempt::new(Some(&url), stop, &self.limits()).with_site(site);
        let response = self
            .retrying(&mut attempt, on_event, || self.get(url.clone(), None))
            .await?;
        if response.status() != StatusCode::OK {
            return Err(Error::Status(response.status()));
        }
        let server_name = response_name(&response);
        let name = self.name.clone().unwrap_or_else(|| server_name.clone());
        if self
            .hold
            .as_ref()
            .is_some_and(|hold| (hold.0)(&server_name, &name, response.content_length()))
        {
            return Err(Error::Held(name));
        }
        let dir = choose(&name, response.url());
        // Only a folder that does not exist yet (a category's) is created:
        // creating one waits for a blocking thread, and a download into an
        // existing folder must reserve its name as soon as before (the
        // cancellation test with a busy blocking pool in `download.rs`
        // found this). The check itself is one quick `stat`.
        if !dir.is_dir() {
            tokio::fs::create_dir_all(&dir).await?;
        }
        let dir = dir.as_path();
        // The size comes from the server: before creating anything, make
        // sure it fits (a huge or false size fails here, clearly).
        if let Some(len) = response.content_length() {
            check_space(dir, len, 0).await?;
        }
        let (final_path, part) = reserve_target(dir, &name)?;
        // From here until the first byte is written, any failure or
        // cancellation removes the new files again.
        let cleanup = EmptyDownloadCleanup::new(&final_path);
        lock(&part, &final_path)?;
        let state = state_from_response(
            without_credentials(&url),
            attempt.credentials.is_some(),
            &response,
        );
        self.start(
            final_path,
            part,
            state,
            response,
            Some(cleanup),
            attempt,
            on_event,
        )
        .await
    }

    /// Runs a segmented download, and starts over if the file changed.
    pub(super) async fn run_segments(
        &self,
        job: Job,
        attempt: Attempt,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        let final_path = job.final_path.clone();
        let old = job.state.clone();
        match segmented::run(job, on_event).await? {
            Outcome::Finished(finished) => Ok(finished),
            Outcome::Restart {
                part,
                reason,
                response,
                cleanup,
            } => {
                // The validator changed while segments were being asked for.
                // Some servers have several machines behind one name, each
                // with its own ETag for the same file: one connection reads
                // from one machine, so start over on one connection.
                let attempt = Attempt {
                    split: attempt.split && reason != RestartReason::FileChanged,
                    ..attempt
                };
                // A page Drive answered a connection with (its file was
                // downloaded too often, its link expired) is not the file:
                // start over through `get`, which follows or refuses it.
                let response =
                    response.filter(|r| !(is_bare_page(r) && self.is_drive_download(&old)));
                match response {
                    Some(response) => {
                        on_event(Event::Restarted { reason });
                        let state = state_from_response(old.url, old.had_credentials, &response);
                        Box::pin(self.start(
                            final_path, part, state, response, cleanup, attempt, on_event,
                        ))
                        .await
                    }
                    None => {
                        Box::pin(
                            self.restart(final_path, part, old, reason, cleanup, attempt, on_event),
                        )
                        .await
                    }
                }
            }
        }
    }

    /// Starts over with a plain GET, reporting `reason`.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn restart(
        &self,
        final_path: PathBuf,
        part: File,
        old: DownloadState,
        reason: RestartReason,
        cleanup: Option<EmptyDownloadCleanup>,
        attempt: Attempt,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        on_event(Event::Restarted { reason });
        let mut attempt = attempt;
        let response = self
            .get_with_fallback(&old, None, &mut attempt, on_event)
            .await
            .map_err(|err| resumable_stop(err, &final_path, cleanup.is_some()))?;
        if response.status() != StatusCode::OK {
            return Err(refused(response.status(), &old, &final_path, &attempt));
        }
        let state = state_from_response(old.url, old.had_credentials, &response);
        self.start(
            final_path, part, state, response, cleanup, attempt, on_event,
        )
        .await
    }

    /// Writes a whole-file `200` response into `part` from byte 0, over
    /// several connections when the server allows it. `cleanup` is set for a
    /// new download (see [`EmptyDownloadCleanup`]).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn start(
        &self,
        final_path: PathBuf,
        part: File,
        mut state: DownloadState,
        response: Response,
        cleanup: Option<EmptyDownloadCleanup>,
        attempt: Attempt,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        let Some(restarts_left) = attempt.restarts_left.checked_sub(1) else {
            return Err(Error::Interrupted {
                state_file: state_path(&final_path),
                reason: "the file kept changing on the server; try again later".into(),
            });
        };
        let attempt = Attempt {
            restarts_left,
            ..attempt
        };
        let len = state.content_length;
        let part_file = part_path(&final_path);
        // A download starting over (`cleanup` is set only for a new one)
        // records that nothing is done before its part file is emptied:
        // otherwise a crash between the two left the old state, which still
        // counted the old bytes as done, over a file of zeros, and a resume
        // the server then allowed kept the zeros (#163).
        if cleanup.is_none() {
            save_state(&mut state, &state_path(&final_path), None).await?;
        }
        // Like a save, the allocation keeps a cancelled new download's name
        // reserved until it is over (see `EmptyDownloadCleanup`). It hands
        // the guard's share back next to the file, so if nobody waits for
        // the result any more, the file is closed before the name is freed.
        let keep_alive = cleanup.as_ref().and_then(EmptyDownloadCleanup::watch);
        let (part, keep_alive) = blocking(move || {
            let mut part = part;
            part.set_len(0)?;
            if let Some(len) = len {
                zajil_platform::fs::allocate(&part, len).map_err(|source| Error::Allocate {
                    file: part_file,
                    bytes: len,
                    source,
                })?;
            }
            part.seek(SeekFrom::Start(0))?;
            Ok::<_, Error>((part, keep_alive))
        })
        .await?;
        drop(keep_alive);
        #[cfg(feature = "test-support")]
        if engine_test_hooks::CRASH_AFTER_ALLOCATING
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(Error::Io(io::Error::other("a crash, as the test asked")));
        }

        let split = attempt.split.then(|| self.split(&state)).flatten();
        let watch = cleanup.as_ref().and_then(EmptyDownloadCleanup::watch);
        if let Some(segments) = split {
            state.segments = segments;
            save_state(&mut state, &state_path(&final_path), watch).await?;
            let if_range = if_range_value(&state.validator).expect("checked by split");
            let job = Job {
                client: self.client.clone(),
                browser: self.browser.clone(),
                connections: self.options.connections,
                min_segment: self.options.min_segment_size,
                request_size: self.options.request_size,
                save_interval: self.options.save_interval,
                max_failures: self.options.max_failures,
                stop: attempt.stop.clone(),
                limits: attempt.limits.clone(),
                site: attempt.site.clone(),
                url: attempt.authorize(response.url()),
                if_range,
                final_path,
                part,
                state,
                first: Some((response, 0)),
                cleanup,
            };
            return self.run_segments(job, attempt, on_event).await;
        }
        save_state(&mut state, &state_path(&final_path), watch).await?;
        let transfer = self.transfer(final_path, part, state, cleanup);
        self.receive(transfer, response, attempt, on_event).await
    }

    /// The segments for a parallel download, or `None` for one connection:
    /// the server must announce ranges, the size and a strong validator.
    fn split(&self, state: &DownloadState) -> Option<Vec<crate::state::Segment>> {
        let len = state.content_length?;
        if !state.accept_ranges || if_range_value(&state.validator).is_none() {
            return None;
        }
        segmented::plan(len, self.options.connections, self.options.min_segment_size)
    }
}
