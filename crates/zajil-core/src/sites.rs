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
//!
//! In the queues, the runner takes a download's first slot before starting
//! it and hands it over: a download whose site has no free slot waits
//! without taking room in its queue (#170).

use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{Error, Stop, Url};

/// The most sites with a limit.
pub const MAX_SITES: usize = 100;

/// The per-site limits an [`crate::Engine`] counts against.
#[derive(Debug, Default)]
pub struct SiteLimits {
    /// `(site, its counter)`, the site lowercase.
    sites: Vec<(String, Arc<Semaphore>)>,
}

impl SiteLimits {
    /// Limits from `(site, connections)` pairs, such as the settings file's
    /// `site_connections` (checked there). A connection count of 0 counts
    /// as 1, and a site listed twice keeps its first entry.
    pub fn new<'a>(list: impl IntoIterator<Item = (&'a str, u32)>) -> Self {
        let mut sites: Vec<(String, Arc<Semaphore>)> = Vec::new();
        for (site, connections) in list.into_iter().take(MAX_SITES) {
            let site = site.trim().trim_end_matches('.').to_ascii_lowercase();
            if site.is_empty() || sites.iter().any(|(s, _)| *s == site) {
                continue;
            }
            let n = (connections.max(1) as usize).min(crate::MAX_CONNECTIONS);
            sites.push((site, Arc::new(Semaphore::new(n))));
        }
        SiteLimits { sites }
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
        let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
        self.sites
            .iter()
            .filter(|(site, _)| {
                host == *site
                    || host
                        .strip_suffix(site.as_str())
                        .is_some_and(|rest| rest.ends_with('.'))
            })
            .max_by_key(|(site, _)| site.len())
            .map(|(_, counter)| counter.clone())
    }
}

/// A slot the queue runner took for a download before starting it (so a
/// download that would wait for its site takes no room in its queue, #170):
/// the download uses it instead of waiting for one.
#[derive(Debug, Clone, Default)]
pub(crate) struct TakenSlot(Arc<std::sync::Mutex<Option<OwnedSemaphorePermit>>>);

impl TakenSlot {
    pub(crate) fn new(permit: OwnedSemaphorePermit) -> Self {
        TakenSlot(Arc::new(std::sync::Mutex::new(Some(permit))))
    }

    /// The slot, once, if it is one of `counter`'s (the download's site);
    /// one of another site is let go.
    fn take_for(&self, counter: &Arc<Semaphore>) -> Option<OwnedSemaphorePermit> {
        let permit = self.0.lock().unwrap_or_else(|e| e.into_inner()).take()?;
        Arc::ptr_eq(permit.semaphore(), counter).then_some(permit)
    }
}

/// Waits for a slot of `counter` (none to wait for without a limit), until
/// `stop`; a slot `taken` for it ahead comes at once.
pub(crate) async fn take_slot(
    counter: Option<&Arc<Semaphore>>,
    taken: &TakenSlot,
    stop: &Stop,
) -> Result<Option<OwnedSemaphorePermit>, Error> {
    let Some(counter) = counter else {
        return Ok(None);
    };
    if let Some(permit) = taken.take_for(counter) {
        return Ok(Some(permit));
    }
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
            take_slot(Some(&counter), &TakenSlot::default(), &stop).await,
            Err(Error::Stopped { state_file: None })
        ));
        assert!(
            take_slot(None, &TakenSlot::default(), &stop)
                .await
                .unwrap()
                .is_none()
        );
    }

    /// A slot taken ahead by the queue runner is the download's own, once,
    /// and only for its site.
    #[tokio::test]
    async fn a_slot_taken_ahead_is_used_once_for_its_site() {
        let site = Arc::new(Semaphore::new(1));
        let other = Arc::new(Semaphore::new(1));
        let stop = Stop::new();
        let taken = TakenSlot::new(site.clone().try_acquire_owned().unwrap());
        // Another site's download lets it go and takes its own.
        let own = take_slot(Some(&other), &taken, &stop).await.unwrap();
        assert!(own.is_some());
        assert_eq!(site.available_permits(), 1);
        let taken = TakenSlot::new(site.clone().try_acquire_owned().unwrap());
        let slot = take_slot(Some(&site), &taken, &stop).await.unwrap();
        assert!(slot.is_some());
        assert_eq!(site.available_permits(), 0);
        // Used: a second download waits as any other would.
        stop.request();
        assert!(take_slot(Some(&site), &taken, &stop).await.is_err());
        drop(slot);
        assert_eq!(site.available_permits(), 1);
    }
}
