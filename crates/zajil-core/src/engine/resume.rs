//! Going on with a download from its state file, and its link refreshed.

use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use reqwest::header;
use reqwest::{StatusCode, Url};

use crate::segmented::Job;
use crate::state::{DownloadState, part_path, state_path};

use super::download::Attempt;
use super::files::{checked_final_path, lock};
use super::http::{
    changed, continues_at, header_str, if_range_value, parse_content_range, parse_url, shown_url,
    state_from_response, without_credentials,
};
use super::transfer::{finish_download, refused, resumable_stop};
use super::{Engine, Error, Event, Finished, Mismatch, RestartReason, Stop};

impl Engine {
    /// Continues the download described by `state_file`
    /// (`<folder>/<name>.zajil-state`).
    ///
    /// Asks only for the bytes that are not on disk yet, with `Range` and
    /// `If-Range` (RFC 9110 §14.2, §13.1.5). If the server sends the whole
    /// file instead (it changed, or it ignores ranges), the download starts
    /// over from that response. Old and new content are never mixed.
    pub async fn resume(
        &self,
        state_file: &Path,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        self.resume_until(state_file, &Stop::new(), on_event).await
    }

    /// Like [`Engine::resume`], until `stop` is requested (see
    /// [`Engine::download_until`]).
    pub async fn resume_until(
        &self,
        state_file: &Path,
        stop: &Stop,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        let limits = self.limits();
        let final_path = checked_final_path(state_file)?;
        let state_file = state_path(&final_path);
        let state = DownloadState::load(&state_file)?;
        // The download's own slot of its site, held until it ends. Stopped
        // while waiting: still resumable from its state file.
        let site = Url::parse(&state.url)
            .ok()
            .and_then(|url| self.sites.counter_for(&url));
        let _slot = crate::sites::take_slot(site.as_ref(), stop)
            .await
            .map_err(|_| Error::Stopped {
                state_file: Some(state_file.clone()),
            })?;
        // Read again once the slot came, as the wait may be long and the
        // part file is not locked meanwhile: if another resume finished it,
        // the state file is gone and nothing is downloaded again (review of
        // #166).
        let state = if site.is_some() {
            DownloadState::load(&state_file)?
        } else {
            state
        };
        let new_attempt = || Attempt::new(None, stop, &limits).with_site(site.clone());
        let part_file = part_path(&final_path);
        let part = match zajil_platform::fs::open_regular_nofollow(&part_file, true) {
            Ok(part) => Some(part),
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) if zajil_platform::fs::is_not_regular_file(&err) => {
                return Err(Error::NotRegularFile(part_file));
            }
            Err(err) => return Err(err.into()),
        };

        let Some(part) = part else {
            // Nothing on disk to continue from.
            let part = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&part_file)?;
            lock(&part, &final_path)?;
            return self
                .restart(
                    final_path,
                    part,
                    state,
                    RestartReason::PartFileMissing,
                    None,
                    new_attempt(),
                    on_event,
                )
                .await;
        };
        lock(&part, &final_path)?;

        let done = state.bytes_done();
        // The data must reach the last byte the state counts as written.
        let written_end = state
            .segments
            .iter()
            .map(|s| s.start.saturating_add(s.done))
            .max()
            .unwrap_or(0);
        let on_disk = part.metadata()?.len();
        let if_range = if_range_value(&state.validator);
        let reason = if on_disk < written_end {
            Some(RestartReason::PartFileTooShort)
        } else if done == 0 {
            Some(RestartReason::NothingDone)
        } else if if_range.is_none() {
            // Without a validator a changed file could not be detected.
            Some(RestartReason::NoValidator)
        } else {
            None
        };
        if let Some(reason) = reason {
            return self
                .restart(
                    final_path,
                    part,
                    state,
                    reason,
                    None,
                    new_attempt(),
                    on_event,
                )
                .await;
        }
        let if_range = if_range.expect("checked above");
        if state.segments.len() > 1 {
            return self
                .resume_segments(final_path, part, state, if_range, new_attempt(), on_event)
                .await;
        }

        let mut attempt = new_attempt();
        let response = self
            .get_with_fallback(&state, Some((done, None, if_range)), &mut attempt, on_event)
            .await
            .map_err(|err| resumable_stop(err, &final_path, false))?;
        match response.status() {
            StatusCode::PARTIAL_CONTENT => {
                if !continues_at(&response, &state, done) {
                    return self
                        .restart(
                            final_path,
                            part,
                            state,
                            RestartReason::RangeMismatch,
                            None,
                            attempt,
                            on_event,
                        )
                        .await;
                }
                let mut part = part;
                part.seek(SeekFrom::Start(done))?;
                let transfer = self.transfer(final_path, part, state, None);
                self.receive(transfer, response, attempt, on_event).await
            }
            StatusCode::OK => {
                let reason = if changed(&state, &response) {
                    RestartReason::FileChanged
                } else {
                    RestartReason::RangesIgnored
                };
                on_event(Event::Restarted { reason });
                let state =
                    state_from_response(state.url.clone(), state.had_credentials, &response);
                self.start(final_path, part, state, response, None, attempt, on_event)
                    .await
            }
            StatusCode::RANGE_NOT_SATISFIABLE if state.content_length == Some(done) => {
                // Everything was already on disk; only the rename was missing.
                self.transfer(final_path, part, state, None)
                    .finish(on_event)
                    .await
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                self.restart(
                    final_path,
                    part,
                    state,
                    RestartReason::RangeMismatch,
                    None,
                    attempt,
                    on_event,
                )
                .await
            }
            status => Err(refused(status, &state, &final_path, &attempt)),
        }
    }

    /// Gives the download of `state_file` a new link (`new_url`): the same
    /// file from another address, as when a signed link expired (M5,
    /// "refresh the download link"). The server must show it is the same
    /// file: the same size and the same strong validator (an `ETag`, or a
    /// strong `Last-Modified`) as the state file. Otherwise
    /// [`Error::NotTheSameFile`], and the state file is left as it was. Only
    /// the first response's headers are read; the download then goes on
    /// with [`Engine::resume_until`], asking only for what is missing.
    ///
    /// The download must not be running ([`Error::Locked`] otherwise), and
    /// `new_url` must be an http or https URL without a user name or
    /// password.
    pub async fn refresh_link(&self, state_file: &Path, new_url: &str) -> Result<(), Error> {
        self.refresh_link_until(state_file, new_url, &Stop::new())
            .await
    }

    /// [`Engine::refresh_link`], given up when `stop` is requested
    /// ([`Error::Stopped`], the state file left as it was): the user closed
    /// the dialog while the new link's server was slow (review of #78).
    pub async fn refresh_link_until(
        &self,
        state_file: &Path,
        new_url: &str,
        stop: &Stop,
    ) -> Result<(), Error> {
        let final_path = checked_final_path(state_file)?;
        let state_file = state_path(&final_path);
        let url = parse_url(new_url)?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(Error::InvalidUrl {
                url: shown_url(new_url),
                reason: "a new link with a user name or password is not accepted".into(),
            });
        }
        // The part file's lock is held from before the state is read until
        // after it is written: a running download holds it. With no part
        // file, one is made empty to hold it (the resume then starts over,
        // as it would have), and removed again if the refresh fails
        // (review of #74).
        let part_file = part_path(&final_path);
        let (part, made) = match zajil_platform::fs::open_regular_nofollow(&part_file, true) {
            Ok(part) => (part, false),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let part = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&part_file)?;
                (part, true)
            }
            Err(err) if zajil_platform::fs::is_not_regular_file(&err) => {
                return Err(Error::NotRegularFile(part_file));
            }
            Err(err) => return Err(err.into()),
        };
        lock(&part, &final_path)?;
        let result = self.refresh_locked(&state_file, &url, stop).await;
        if result.is_err() && made {
            drop(part);
            let _ = std::fs::remove_file(&part_file);
        }
        result
    }

    /// [`Engine::refresh_link`], with the part file locked.
    async fn refresh_locked(&self, state_file: &Path, url: &Url, stop: &Stop) -> Result<(), Error> {
        let mut state = DownloadState::load(state_file)?;
        let Some(ours) = if_range_value(&state.validator) else {
            return Err(Error::NotTheSameFile(Mismatch::NoValidator));
        };
        let limits = self.limits();
        let mut attempt = Attempt::new(None, stop, &limits);
        let response = self
            .retrying(&mut attempt, &mut |_| {}, || self.get(url.clone(), None))
            .await?;
        if response.status() != StatusCode::OK {
            return Err(Error::Status(response.status()));
        }
        // Built as a download's state is: the final URL without a user
        // name or password (review of #74: a redirect's were kept), and the
        // link as the URL parser writes it, as a download's and a queue
        // item's are (a queue resumes a state only for its item's link).
        let theirs = state_from_response(without_credentials(url), false, &response);
        drop(response);
        if state.content_length.is_none() || theirs.content_length != state.content_length {
            return Err(Error::NotTheSameFile(Mismatch::Size {
                theirs: theirs.content_length,
                ours: state.content_length,
            }));
        }
        match if_range_value(&theirs.validator) {
            Some(new) if new == ours => {}
            // Another kind of validator, or none: not shown to differ, but
            // nothing shows it is the same file either.
            Some(new) if new.starts_with('"') != ours.starts_with('"') => {
                return Err(Error::NotTheSameFile(Mismatch::ValidatorKind));
            }
            None => return Err(Error::NotTheSameFile(Mismatch::NewNoValidator)),
            Some(_) => return Err(Error::NotTheSameFile(Mismatch::Version)),
        }
        state.url = theirs.url;
        state.final_url = theirs.final_url;
        state.had_credentials = false;
        state.save(state_file)?;
        Ok(())
    }

    /// Resumes a download made of several segments: the first missing range
    /// is checked like a single-connection resume (with a fallback to the
    /// original URL), then every missing range gets a connection.
    async fn resume_segments(
        &self,
        final_path: PathBuf,
        part: File,
        state: DownloadState,
        if_range: String,
        attempt: Attempt,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        let missing = state
            .segments
            .iter()
            .find(|s| s.end.is_some_and(|end| s.start + s.done <= end))
            .cloned();
        let Some(segment) = missing else {
            // Every byte is on disk; only the rename was missing.
            return finish_download(part, state, final_path, on_event).await;
        };
        let (from, to) = (segment.start + segment.done, segment.end.expect("checked"));
        let mut attempt = attempt;
        let response = self
            .get_with_fallback(
                &state,
                Some((from, Some(to), if_range.clone())),
                &mut attempt,
                on_event,
            )
            .await
            .map_err(|err| resumable_stop(err, &final_path, false))?;
        match response.status() {
            StatusCode::PARTIAL_CONTENT => {
                let range = header_str(response.headers(), header::CONTENT_RANGE)
                    .and_then(parse_content_range);
                let exact = range.is_some_and(|r| {
                    r.start == from && r.end == to && r.total == state.content_length
                });
                if !exact || changed(&state, &response) {
                    let reason = if exact {
                        RestartReason::FileChanged
                    } else {
                        RestartReason::RangeMismatch
                    };
                    return self
                        .restart(final_path, part, state, reason, None, attempt, on_event)
                        .await;
                }
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
                    url: response.url().clone(),
                    if_range,
                    final_path,
                    part,
                    state,
                    first: Some((response, segment.start)),
                    cleanup: None,
                };
                self.run_segments(job, attempt, on_event).await
            }
            StatusCode::OK => {
                let reason = if changed(&state, &response) {
                    RestartReason::FileChanged
                } else {
                    RestartReason::RangesIgnored
                };
                on_event(Event::Restarted { reason });
                let state =
                    state_from_response(state.url.clone(), state.had_credentials, &response);
                self.start(final_path, part, state, response, None, attempt, on_event)
                    .await
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                self.restart(
                    final_path,
                    part,
                    state,
                    RestartReason::RangeMismatch,
                    None,
                    attempt,
                    on_event,
                )
                .await
            }
            status => Err(refused(status, &state, &final_path, &attempt)),
        }
    }
}
