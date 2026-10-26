//! The most connections at once to a site, shared by every download on it
//! (owner's decision, 2026-10-26: IDM's per-server exceptions). No limit by
//! default: only the sites the user lists have one. A site's entry covers
//! its subdomains (`example.com` also counts `dl.example.com`). The
//! automatic cut on `429` and `403` still applies within each download.
//!
//! Each download holds one slot of its site for as long as it runs (its
//! first connection), and a split download needs one more slot for each
//! further connection; a connection that finds none waits and looks again a
//! moment later, and holds no slot while it waits for anything. So the
//! connections to a site never pass its limit, whatever the number of
//! downloads. (The short probe and page requests, and a refreshed link's
//! first request, are not counted.) The site is the address the download
//! was given, not where it redirects.

use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{Error, Stop, Url};

/// The most sites with a limit.
pub const MAX_SITES: usize = 100;

/// The per-site limits an [`crate::Engine`] counts against. Shared by the
/// engine and its clones, and changed in place ([`SiteLimits::set`]), so a
/// change reaches the downloads already running (#165, #170).
#[derive(Debug, Default)]
pub struct SiteLimits {
    /// `(site, its counter, its limit)`, the site lowercase.
    sites: std::sync::RwLock<Vec<(String, Arc<Semaphore>, usize)>>,
}

/// `(site, connections)` pairs as kept: lowercase, at most [`MAX_SITES`],
/// 0 counting as 1, a site listed twice keeping its first entry.
fn plain<'a>(list: impl IntoIterator<Item = (&'a str, u32)>) -> Vec<(String, usize)> {
    let mut kept: Vec<(String, usize)> = Vec::new();
    for (site, connections) in list.into_iter().take(MAX_SITES) {
        let site = site.trim().trim_end_matches('.').to_ascii_lowercase();
        if site.is_empty() || kept.iter().any(|(s, _)| *s == site) {
            continue;
        }
        kept.push((
            site,
            (connections.max(1) as usize).min(crate::MAX_CONNECTIONS),
        ));
    }
    kept
}

impl SiteLimits {
    /// Limits from `(site, connections)` pairs, such as the settings file's
    /// `site_connections` (checked there). A connection count of 0 counts
    /// as 1, and a site listed twice keeps its first entry.
    pub fn new<'a>(list: impl IntoIterator<Item = (&'a str, u32)>) -> Self {
        let sites = plain(list)
            .into_iter()
            .map(|(site, n)| (site, Arc::new(Semaphore::new(n)), n))
            .collect();
        SiteLimits {
            sites: std::sync::RwLock::new(sites),
        }
    }

    /// Changes the limits to `list`, in place (#165; #170, item 2): a site
    /// that stays keeps its counter, so its running downloads and the new
    /// ones are counted together, never twice its limit. A higher limit
    /// gives its slots at once; a lower one takes back the free ones now
    /// and the others as running downloads give them back (in the
    /// background, on the current runtime). A site no longer listed has no
    /// limit for new downloads; one newly listed starts with all its
    /// slots.
    pub fn set<'a>(&self, list: impl IntoIterator<Item = (&'a str, u32)>) {
        let mut sites = self.sites.write().unwrap_or_else(|e| e.into_inner());
        let next = plain(list)
            .into_iter()
            .map(
                |(site, n)| match sites.iter().find(|(s, _, _)| *s == site) {
                    Some((_, counter, old)) => {
                        let counter = counter.clone();
                        if n > *old {
                            counter.add_permits(n - old);
                        } else if n < *old {
                            let owed = (old - n).saturating_sub(counter.forget_permits(old - n));
                            take_back(&counter, owed);
                        }
                        (site, counter, n)
                    }
                    None => (site, Arc::new(Semaphore::new(n)), n),
                },
            )
            .collect();
        *sites = next;
    }

    /// [`SiteLimits::set`] from a settings file.
    pub fn set_from_settings(&self, file: &crate::settings::SettingsFile) {
        self.set(
            file.site_connections
                .iter()
                .map(|s| (s.site.as_str(), s.connections)),
        );
    }

    /// The limits of a settings file.
    pub fn from_settings(file: &crate::settings::SettingsFile) -> Self {
        SiteLimits::new(
            file.site_connections
                .iter()
                .map(|s| (s.site.as_str(), s.connections)),
        )
    }

    /// The counter of `url`'s site: the longest listed site that is its
    /// host or a parent domain of it; `None` without a limit.
    pub fn counter_for(&self, url: &Url) -> Option<Arc<Semaphore>> {
        self.entry_for(url).map(|(counter, _)| counter)
    }

    /// The counter of `url`'s site and its limit; `None` without a limit.
    pub fn entry_for(&self, url: &Url) -> Option<(Arc<Semaphore>, usize)> {
        let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
        self.sites
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(site, _, _)| {
                host == *site
                    || host
                        .strip_suffix(site.as_str())
                        .is_some_and(|rest| rest.ends_with('.'))
            })
            .max_by_key(|(site, _, _)| site.len())
            .map(|(_, counter, limit)| (counter.clone(), *limit))
    }
}

/// Takes `owed` slots of `counter` back as they are given back, and keeps
/// them: a lowered limit reached once the running downloads return their
/// slots. Without a runtime (a test), only the free slots were taken.
fn take_back(counter: &Arc<Semaphore>, owed: usize) {
    let Ok(owed) = u32::try_from(owed) else {
        return;
    };
    if owed == 0 {
        return;
    }
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        let counter = counter.clone();
        runtime.spawn(async move {
            if let Ok(slots) = counter.acquire_many_owned(owed).await {
                slots.forget();
            }
        });
    }
}

/// Waits for a slot of `counter` (none to wait for without a limit), until
/// `stop`.
pub(crate) async fn take_slot(
    counter: Option<&Arc<Semaphore>>,
    stop: &Stop,
) -> Result<Option<OwnedSemaphorePermit>, Error> {
    let Some(counter) = counter else {
        return Ok(None);
    };
    tokio::select! {
        // A stop already asked wins over a free slot.
        biased;
        () = stop.requested() => Err(Error::Stopped { state_file: None }),
        permit = counter.clone().acquire_owned() => Ok(permit.ok()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn a_site_covers_itself_and_its_subdomains_only() {
        let limits = SiteLimits::new([("Example.COM.", 2), ("files.example.com", 1)]);
        let main = limits.counter_for(&url("https://example.com/a")).unwrap();
        assert_eq!(main.available_permits(), 2);
        assert!(Arc::ptr_eq(
            &main,
            &limits
                .counter_for(&url("https://dl.example.com/a"))
                .unwrap()
        ));
        // The longest match wins.
        assert_eq!(
            limits
                .counter_for(&url("https://a.files.example.com/x"))
                .unwrap()
                .available_permits(),
            1
        );
        for other in [
            "https://notexample.com/a",
            "https://example.com.evil.test/a",
            "https://example.org/a",
            "https://127.0.0.1/a",
        ] {
            assert!(limits.counter_for(&url(other)).is_none(), "{other}");
        }
    }

    #[test]
    fn no_limit_by_default() {
        assert!(
            SiteLimits::default()
                .counter_for(&url("https://example.com/"))
                .is_none()
        );
    }

    #[test]
    fn odd_entries_are_kept_safe() {
        let limits = SiteLimits::new([("a.test", 0), ("a.test", 9), ("b.test", 1000), ("", 3)]);
        assert_eq!(
            limits
                .counter_for(&url("http://a.test/"))
                .unwrap()
                .available_permits(),
            1
        );
        assert_eq!(
            limits
                .counter_for(&url("http://b.test/"))
                .unwrap()
                .available_permits(),
            crate::MAX_CONNECTIONS
        );
        let many: Vec<(String, u32)> = (0..150).map(|i| (format!("s{i}.test"), 1)).collect();
        let limits = SiteLimits::new(many.iter().map(|(s, n)| (s.as_str(), *n)));
        assert!(limits.counter_for(&url("http://s99.test/")).is_some());
        assert!(limits.counter_for(&url("http://s100.test/")).is_none());
    }

    #[tokio::test]
    async fn a_stop_ends_the_wait_for_a_slot() {
        let counter = Arc::new(Semaphore::new(1));
        let _held = counter.clone().acquire_owned().await.unwrap();
        let stop = Stop::new();
        stop.request();
        assert!(matches!(
            take_slot(Some(&counter), &stop).await,
            Err(Error::Stopped { state_file: None })
        ));
        assert!(take_slot(None, &stop).await.unwrap().is_none());
    }

    /// #165, #170 item 2: a changed list keeps a site's counter, so the
    /// downloads running and the new ones are counted together; a higher
    /// limit gives its slots at once, a lower one takes back the free ones
    /// now and the busy ones as they come back.
    #[tokio::test]
    async fn a_changed_list_keeps_each_site_s_counter() {
        let limits = SiteLimits::new([("example.com", 2)]);
        let counter = limits.counter_for(&url("https://example.com/a")).unwrap();
        let held = counter.clone().acquire_owned().await.unwrap();
        limits.set([("example.com", 4), ("other.test", 1)]);
        let same = limits
            .counter_for(&url("https://dl.example.com/b"))
            .unwrap();
        assert!(Arc::ptr_eq(&counter, &same));
        assert_eq!(same.available_permits(), 3, "one held, three free");
        assert_eq!(
            limits
                .counter_for(&url("https://other.test/c"))
                .unwrap()
                .available_permits(),
            1
        );
        // Down to 1, one slot held: the three free ones go now, and the
        // held one, once given back, comes back as the only one.
        limits.set([("example.com", 1)]);
        assert_eq!(same.available_permits(), 0);
        drop(held);
        assert_eq!(same.available_permits(), 1);
        // Down to 1 from 2 with both held: one is taken back when given.
        limits.set([("example.com", 2)]);
        let a = same.clone().acquire_owned().await.unwrap();
        let b = same.clone().acquire_owned().await.unwrap();
        limits.set([("example.com", 1)]);
        drop(a);
        tokio::task::yield_now().await;
        drop(b);
        // The taker runs on this runtime: let it.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(same.available_permits(), 1);
        // No longer listed: no limit for new downloads.
        limits.set(std::iter::empty());
        assert!(limits.counter_for(&url("https://example.com/a")).is_none());
    }
}
