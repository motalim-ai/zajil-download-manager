//! One download's transfer: its body written, how it stops, and the files
//! of a new download that never started.

use std::fs::File;
use std::io::{self, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use reqwest::{Response, StatusCode};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::limit::Limits;
use crate::state::{DownloadState, part_path, state_path};

use super::download::Attempt;
use super::files::{blocking, parent_dir, place_final, save_state};
use super::http::{
    changed, continues_at, http_error, if_range_value, origin_of, state_from_response,
};
use super::{Engine, Error, Event, Finished, RestartReason, Stop};

/// Why [`Transfer::pump`] stopped before the end of the body.
enum Halt {
    /// The connection broke or closed early: worth trying again.
    Broken(String),
    /// Trying again would not help (a write failed, or the server sent
    /// more than it announced).
    Fatal(String),
    /// The caller asked the download to stop ([`Stop`]).
    Requested,
}

/// An open download: the locked part file and its state.
pub(super) struct Transfer {
    file: tokio::fs::File,
    state: DownloadState,
    state_file: PathBuf,
    final_path: PathBuf,
    last_save: Instant,
    save_interval: Duration,
    /// A save of this transfer has succeeded: a stop then needs no save of
    /// its own (#132).
    saved: bool,
    /// Writing the part file or saving the state failed (a full disk). A
    /// write is reported failed only at the next one, after its bytes were
    /// counted, so no save follows: the state file saved last counts only
    /// bytes that are on disk, and the download resumes from there.
    disk_failed: bool,
    /// Set for a new download until its first byte is written. Declared
    /// last, so the part file is closed before it is removed.
    cleanup: Option<EmptyDownloadCleanup>,
}

/// Removes a new download's `.zajil-part` and `.zajil-state` when dropped,
/// unless disarmed. A download that fails or is cancelled before any byte
/// arrives would otherwise leave files that take the name (the next
/// download would become `name (1)`) and cannot be resumed usefully.
///
/// A state save may still be running on a blocking thread when the guard is
/// dropped (a blocking task cannot be cancelled). If the guard removed the
/// files at once, that save would write the state file afterwards, and the
/// freed name could already belong to a new download whose files the old
/// save would then touch. So the guard and every save made while it is
/// armed share one [`NewDownload`] in an `Arc`: dropping the guard only
/// marks it abandoned, and the files are removed when the **last** owner
/// lets go, after the guard and every save are done. Until then the
/// `.zajil-part` file keeps the name reserved, so a new download takes
/// `name (1)` instead. Nothing waits on the async thread.
pub(crate) struct EmptyDownloadCleanup {
    new: Arc<NewDownload>,
    armed: bool,
}

/// What the guard and the saves share (and, for a segmented download, the
/// writes).
pub(crate) struct NewDownload {
    final_path: PathBuf,
    pub(super) abandoned: AtomicBool,
}

impl Drop for NewDownload {
    /// Runs once the guard and every save are finished. The state file goes
    /// first and the part file last, so the name stays reserved (by the part
    /// file) until nothing of this download is left.
    ///
    /// Meanwhile a resume may have taken the files over: the state file
    /// exists while the first save runs, and a cancelled download releases
    /// the part file's lock at once. Files whose lock someone holds are
    /// left alone, and the lock is held while their names are freed, so a
    /// resume cannot take them in between (found by the cancellation test
    /// of M3, `retry.rs`).
    ///
    /// The names are freed by moving the files aside (emptied), not by removing them
    /// in place: on Windows a file removed while a handle on it is open
    /// stays "delete pending" and keeps its name until the last handle
    /// closes, and a new download of the same name then failed with
    /// `ERROR_ACCESS_DENIED` (#55). That handle was our own lock, closed
    /// only after the removal, or another program's (an antivirus). So the
    /// files are moved aside while locked, our handle is closed, and only
    /// then are the moved files removed.
    fn drop(&mut self) {
        if !self.abandoned.load(Ordering::SeqCst) {
            return;
        }
        let part = part_path(&self.final_path);
        let held = match zajil_platform::fs::open_regular_nofollow(&part, true) {
            Ok(file) => match file.try_lock() {
                Ok(()) => Some(file),
                Err(_) => return, // someone resumed it
            },
            Err(_) => None,
        };
        // Emptied first: the part was given its full size before the first
        // byte, and a moved file left behind (the process dies before
        // removing it) should not keep that space (review of #56).
        if let Some(file) = &held {
            let _ = file.set_len(0);
        }
        let state = state_path(&self.final_path);
        let aside = [state.as_path(), part.as_path()].map(|file| {
            zajil_platform::fs::move_aside(file).map_err(|err| {
                // Not movable (missing, or held without sharing its
                // deletion): removed in place, as before, if it can be.
                if err.kind() != io::ErrorKind::NotFound {
                    let _ = std::fs::remove_file(file);
                }
            })
        });
        drop(held);
        for moved in aside.into_iter().flatten() {
            let _ = std::fs::remove_file(moved);
        }
    }
}

impl EmptyDownloadCleanup {
    pub(super) fn new(final_path: &Path) -> Self {
        EmptyDownloadCleanup {
            new: Arc::new(NewDownload {
                final_path: final_path.to_path_buf(),
                abandoned: AtomicBool::new(false),
            }),
            armed: true,
        }
    }

    /// For a save made while the guard is armed.
    pub(crate) fn watch(&self) -> Option<Arc<NewDownload>> {
        self.armed.then(|| self.new.clone())
    }

    pub(crate) fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for EmptyDownloadCleanup {
    fn drop(&mut self) {
        if self.armed {
            // The files go when the last owner of `new` is dropped: here,
            // or at the end of a save that is still running.
            self.new.abandoned.store(true, Ordering::SeqCst);
        }
    }
}

impl Transfer {
    fn done(&self) -> u64 {
        self.state.segments[0].done
    }

    /// Writes the body of `response` after the current position. `Ok` when
    /// the body ended where it should.
    async fn pump(
        &mut self,
        response: &mut Response,
        stop: &Stop,
        limits: &Limits,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<(), Halt> {
        let total = self.state.content_length;
        loop {
            let chunk = tokio::select! {
                // A stop wins over data that is also ready (#194: from a
                // fast server there is nearly always more).
                biased;
                () = stop.requested() => return Err(Halt::Requested),
                chunk = response.chunk() => chunk,
            };
            let chunk = match chunk {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(err) => return Err(Halt::Broken(http_error(err))),
            };
            if total.is_some_and(|t| self.done().saturating_add(chunk.len() as u64) > t) {
                return Err(Halt::Fatal(
                    "the server sent more data than it announced".into(),
                ));
            }
            if let Err(err) = self.file.write_all(&chunk).await {
                self.disk_failed = true;
                return Err(Halt::Fatal(format!("could not write the file: {err}")));
            }
            if let Some(cleanup) = self.cleanup.take() {
                cleanup.disarm();
            }
            self.state.segments[0].done += chunk.len() as u64;
            on_event(Event::Progress {
                done: self.done(),
                total,
            });
            // The speed limits: wait before reading more.
            tokio::select! {
                biased;
                () = stop.requested() => return Err(Halt::Requested),
                () = limits.take(chunk.len() as u64) => {}
            }
            // No periodic save once a stop is asked (#194). The biased
            // select above sees a stop asked before it; this closes the
            // short window after it.
            if stop.is_requested() {
                return Err(Halt::Requested);
            }
            if self.last_save.elapsed() >= self.save_interval
                && let Err(err) = self.checkpoint().await
            {
                return Err(Halt::Fatal(err.to_string()));
            }
        }

        if let Some(total) = total
            && self.done() < total
        {
            return Err(Halt::Broken(format!(
                "the connection closed after {} of {total} bytes",
                self.done()
            )));
        }
        Ok(())
    }

    /// The caller asked to stop: returns only when nothing of the download
    /// runs any more. What is on disk is recorded unless a save already did
    /// (#132): the state file counts only synced bytes. A new download with no
    /// data yet is removed (when `self` is dropped, here: every save was
    /// awaited, so nothing else holds it).
    async fn stopped(mut self) -> Error {
        if self.cleanup.is_some() {
            return Error::Stopped { state_file: None };
        }
        // Owner's decision on #132: after a save, a stop does not save
        // again (the state file counts only synced bytes; about one
        // `save_interval` of progress is downloaded again on resume).
        if !self.saved && !self.disk_failed {
            // A failed save leaves the state file saved last.
            let _ = self.checkpoint().await;
        }
        // Waits for anything the file still does, then closes it.
        drop(self.file.into_std().await);
        Error::Stopped {
            state_file: Some(self.state_file),
        }
    }

    /// The part file and the new-download guard, to start over with.
    async fn into_parts(self) -> (PathBuf, File, Option<EmptyDownloadCleanup>) {
        let part = self.file.into_std().await;
        (self.final_path, part, self.cleanup)
    }

    /// Syncs the data, then records it in the state file (ADR-0004 rule 1).
    /// The next save comes `save_interval` after this one starts, so a
    /// slow save does not stretch the time between them (#132).
    async fn checkpoint(&mut self) -> Result<(), Error> {
        self.last_save = Instant::now();
        let saved = self.sync_and_save().await;
        // Bytes counted may not be on disk, or the disk is full: no save is
        // tried again (`disk_failed`).
        self.disk_failed |= saved.is_err();
        saved
    }

    async fn sync_and_save(&mut self) -> Result<(), Error> {
        self.file.flush().await?;
        self.file.sync_data().await?;
        let watch = self.cleanup.as_ref().and_then(EmptyDownloadCleanup::watch);
        save_state(&mut self.state, &self.state_file, watch).await?;
        self.saved = true;
        Ok(())
    }

    /// Saves what we have (unless a write failed: see `disk_failed`) and
    /// turns `reason` into a resumable error. A new download that has
    /// nothing written yet is not resumable: its files are removed (when
    /// `self` is dropped) and the error says so.
    async fn interrupt(mut self, reason: String) -> Error {
        if self.cleanup.is_some() {
            return Error::NoData(reason);
        }
        if !self.disk_failed {
            // A failed save leaves the state file saved last.
            let _ = self.checkpoint().await;
        }
        Error::Interrupted {
            state_file: self.state_file,
            reason,
        }
    }

    pub(super) async fn finish(
        mut self,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup.disarm(); // an empty file is a complete download
        }
        if let Err(err) = self.file.flush().await {
            self.disk_failed = true;
            return Err(self
                .interrupt(format!("could not write the file: {err}"))
                .await);
        }
        let file = self.file.into_std().await;
        finish_download(file, self.state, self.final_path, on_event).await
    }
}

/// Completes a download whose bytes are all written to `part`: syncs it,
/// records that everything is on disk, renames it to its final name (or a
/// numbered one) and removes the state file.
pub(crate) async fn finish_download(
    part: File,
    mut state: DownloadState,
    final_path: PathBuf,
    on_event: &mut (dyn FnMut(Event) + Send),
) -> Result<Finished, Error> {
    let bytes = state.bytes_done();
    let state_file = state_path(&final_path);
    // Every byte was written, but the disk can still refuse the sync or the
    // save (a full disk: nothing was reserved at the start). The state file
    // saved last counts only bytes on disk, so the download stops there,
    // resumable, as when a write fails (`segmented.rs`).
    let unfinished = |state_file: PathBuf, err: &dyn std::fmt::Display| Error::Interrupted {
        state_file,
        reason: format!("could not finish the file: {err}"),
    };
    let synced = blocking(move || {
        // A file of unknown size may have been pre-sized differently.
        part.set_len(bytes)?;
        part.sync_all()?;
        Ok::<_, io::Error>(part)
    })
    .await;
    let part = match synced {
        Ok(part) => part,
        Err(err) => return Err(unfinished(state_file, &err)),
    };
    // Record that everything is on disk before renaming, so a failed
    // rename leaves a download that `resume` completes at once.
    if let Err(err) = save_state(&mut state, &state_file, None).await {
        return Err(unfinished(state_file, &err));
    }
    drop(part); // releases the lock before the rename

    let part = part_path(&final_path);
    // From the internet: the system warns before it runs, and Office opens
    // it in Protected View (security review #84). On the part file, before
    // it takes its name, so the file never has its name without the mark
    // (review of #90): the mark goes with the file through the rename and
    // the link fallbacks. The link's origin only: its path and query may
    // hold a token. A file system that cannot keep the mark (FAT32, exFAT)
    // keeps the file without it.
    if let Some(origin) = origin_of(&state) {
        let marked = part.clone();
        let _ = blocking(move || zajil_platform::origin::mark(&marked, &origin)).await;
    }
    let wanted = final_path.clone();
    let placed = blocking(move || place_final(&part, &wanted)).await;
    let file = match placed {
        Ok(file) => file,
        Err(err) => {
            return Err(Error::Interrupted {
                state_file,
                reason: format!("could not give the finished file its name: {err}"),
            });
        }
    };
    // The download is complete. A state file that cannot be removed is
    // left behind rather than reported as a failed download.
    let _ = std::fs::remove_file(&state_file);
    let dir = parent_dir(&file).to_path_buf();
    let _ = blocking(move || zajil_platform::fs::sync_dir(&dir)).await;
    on_event(Event::Finished {
        file: file.clone(),
        bytes,
    });
    Ok(Finished { file, bytes })
}

/// A stop requested before any transfer began, for a download whose files
/// are on disk: it can be resumed from its state file. A new download with
/// no data (`new`) leaves no files.
pub(super) fn resumable_stop(err: Error, final_path: &Path, new: bool) -> Error {
    match err {
        Error::Stopped { state_file: None } if !new => Error::Stopped {
            state_file: Some(state_path(final_path)),
        },
        other => other,
    }
}

/// The error for a refusal (`status`) of a request made for `state`. A `401`
/// for a download that started with a user name and password, which are no
/// longer in memory, means it needs them again, and the state file never
/// keeps them (ADR-0004, rule 7): say so, rather than a bare 401.
pub(super) fn refused(
    status: StatusCode,
    state: &DownloadState,
    final_path: &Path,
    attempt: &Attempt,
) -> Error {
    // Only a download that had them: an expired signed link or session
    // also answers 401, and must not ask for a password it never had
    // (review of #29, owner's decision on #28).
    if status == StatusCode::UNAUTHORIZED && attempt.credentials.is_none() && state.had_credentials
    {
        Error::CredentialsNeeded {
            url: state.url.clone(),
            state_file: state_path(final_path),
        }
    } else {
        Error::Status(status)
    }
}

impl Engine {
    pub(super) fn transfer(
        &self,
        final_path: PathBuf,
        part: File,
        state: DownloadState,
        cleanup: Option<EmptyDownloadCleanup>,
    ) -> Transfer {
        Transfer {
            file: tokio::fs::File::from_std(part),
            state,
            state_file: state_path(&final_path),
            final_path,
            last_save: Instant::now(),
            save_interval: self.options.save_interval,
            saved: false,
            disk_failed: false,
            cleanup,
        }
    }

    /// Writes the body of `response` into `transfer` from its current
    /// position, then finishes.
    ///
    /// A connection that breaks is tried again (M3, owner's decision on
    /// #28), with the policy of the segmented connections (`retry.rs`): what
    /// is on disk is synced and recorded, and after a growing wait the rest
    /// is asked for with `Range` and `If-Range`. If the server cannot
    /// continue from there (no strong validator, ranges ignored, or the file
    /// changed), the download starts again from byte 0 with the whole-file
    /// answer: old and new data are never mixed. After `max_failures`
    /// failures in a row without reaching further into the file, the
    /// download stops and can be resumed later.
    pub(super) async fn receive(
        &self,
        mut transfer: Transfer,
        mut response: Response,
        mut attempt: Attempt,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Finished, Error> {
        on_event(Event::Started {
            file: transfer.final_path.clone(),
            state_file: transfer.state_file.clone(),
            total: transfer.state.content_length,
            from: transfer.done(),
        });
        loop {
            let stop = attempt.stop.clone();
            let reason = match transfer
                .pump(&mut response, &stop, &attempt.limits, on_event)
                .await
            {
                Ok(()) => return transfer.finish(on_event).await,
                Err(Halt::Fatal(reason)) => return Err(transfer.interrupt(reason).await),
                Err(Halt::Requested) => return Err(transfer.stopped().await),
                Err(Halt::Broken(reason)) => reason,
            };
            drop(response);
            let done = transfer.done();
            if !attempt.connection_failed(done, self.options.max_failures) {
                let failures = attempt.failures;
                return Err(transfer
                    .interrupt(format!("{reason} (gave up after {failures} tries)"))
                    .await);
            }
            // Asked meanwhile: the stop's own rule on saving applies (#132).
            if stop.is_requested() {
                return Err(transfer.stopped().await);
            }
            // A new download with no data yet has nothing to record.
            if transfer.cleanup.is_none()
                && let Err(err) = transfer.checkpoint().await
            {
                return Err(transfer.interrupt(err.to_string()).await);
            }
            let wait = crate::retry::backoff(attempt.failures);
            on_event(Event::Retrying { wait, reason });
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = stop.requested() => return Err(transfer.stopped().await),
            }

            let if_range = if done > 0 {
                if_range_value(&transfer.state.validator)
            } else {
                None
            };
            let range = if_range.clone().map(|value| (done, None, value));
            response = match self
                .get_with_fallback(&transfer.state, range, &mut attempt, on_event)
                .await
            {
                Ok(response) => response,
                Err(Error::Stopped { .. }) => return Err(transfer.stopped().await),
                Err(err) => return Err(transfer.interrupt(err.to_string()).await),
            };
            match response.status() {
                StatusCode::PARTIAL_CONTENT
                    if if_range.is_some() && continues_at(&response, &transfer.state, done) =>
                {
                    if let Err(err) = transfer.file.seek(SeekFrom::Start(done)).await {
                        return Err(transfer.interrupt(err.to_string()).await);
                    }
                }
                StatusCode::OK => {
                    // Nothing was written yet: simply the first answer again.
                    let reason = if done == 0 {
                        None
                    } else if if_range.is_none() {
                        Some(RestartReason::NoValidator)
                    } else if changed(&transfer.state, &response) {
                        Some(RestartReason::FileChanged)
                    } else {
                        Some(RestartReason::RangesIgnored)
                    };
                    if let Some(reason) = reason {
                        on_event(Event::Restarted { reason });
                    }
                    let state = state_from_response(
                        transfer.state.url.clone(),
                        transfer.state.had_credentials,
                        &response,
                    );
                    let attempt = if reason == Some(RestartReason::FileChanged) {
                        attempt
                    } else {
                        attempt.after_broken_connection()
                    };
                    let (final_path, part, cleanup) = transfer.into_parts().await;
                    return Box::pin(self.start(
                        final_path, part, state, response, cleanup, attempt, on_event,
                    ))
                    .await;
                }
                StatusCode::RANGE_NOT_SATISFIABLE
                    if if_range.is_some() && transfer.state.content_length == Some(done) =>
                {
                    // Everything was already on disk.
                    return transfer.finish(on_event).await;
                }
                StatusCode::PARTIAL_CONTENT | StatusCode::RANGE_NOT_SATISFIABLE => {
                    let old = transfer.state.clone();
                    let (final_path, part, cleanup) = transfer.into_parts().await;
                    return Box::pin(self.restart(
                        final_path,
                        part,
                        old,
                        RestartReason::RangeMismatch,
                        cleanup,
                        attempt,
                        on_event,
                    ))
                    .await;
                }
                status => {
                    return Err(
                        match refused(status, &transfer.state, &transfer.final_path, &attempt) {
                            err @ Error::CredentialsNeeded { .. } => err,
                            err => transfer.interrupt(err.to_string()).await,
                        },
                    );
                }
            }
        }
    }
}
