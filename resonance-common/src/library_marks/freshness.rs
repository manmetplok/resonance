//! Poll-based change detection shared by every library index (NAM models,
//! plugin presets). No watcher dependency: a plugin must not add inotify
//! threads, and a poll is portable.
//!
//! A [`Fingerprint`] is a cheap stat of a few paths: for a directory its
//! mtime and entry count, for a file its size and mtime, for a missing path
//! a marker. [`FreshnessPoll`] re-takes it at most once per interval and
//! reports whether it moved. The cadences are the ones both specs use:
//! [`BROWSER_POLL_INTERVAL`] while a browser is open, [`BAR_POLL_INTERVAL`]
//! from a header/bar, and nothing at all with nothing visible.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Poll cadence while a library browser is open.
pub const BROWSER_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Poll cadence while only a compact bar / header is visible.
pub const BAR_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// What one path looked like when stat'ed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PathStamp {
    Missing,
    File { len: u64, mtime: Option<SystemTime> },
    Dir { entries: usize, mtime: Option<SystemTime> },
}

/// A stat of every watched path, comparable for equality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint(Vec<PathStamp>);

fn stamp(path: &Path) -> PathStamp {
    let Ok(meta) = std::fs::metadata(path) else {
        return PathStamp::Missing;
    };
    let mtime = meta.modified().ok();
    if meta.is_dir() {
        let entries = std::fs::read_dir(path).map(|rd| rd.count()).unwrap_or(0);
        PathStamp::Dir { entries, mtime }
    } else {
        PathStamp::File {
            len: meta.len(),
            mtime,
        }
    }
}

/// Stat every path in `paths` (directories: mtime + entry count; files:
/// size + mtime).
pub fn fingerprint<P: AsRef<Path>>(paths: &[P]) -> Fingerprint {
    Fingerprint(paths.iter().map(|p| stamp(p.as_ref())).collect())
}

/// A rate-limited "did anything under these paths change?" check.
#[derive(Debug, Clone)]
pub struct FreshnessPoll {
    targets: Vec<PathBuf>,
    interval: Duration,
    last_check: Option<Instant>,
    last: Option<Fingerprint>,
}

impl FreshnessPoll {
    pub fn new(targets: Vec<PathBuf>, interval: Duration) -> Self {
        Self {
            targets,
            interval,
            last_check: None,
            last: None,
        }
    }

    pub fn targets(&self) -> &[PathBuf] {
        &self.targets
    }

    /// Change the cadence (e.g. a browser opened or closed).
    pub fn set_interval(&mut self, interval: Duration) {
        self.interval = interval;
    }

    /// Whether a [`check`](Self::check) at `now` would stat anything.
    pub fn due(&self, now: Instant) -> bool {
        self.last_check
            .is_none_or(|t| now.saturating_duration_since(t) >= self.interval)
    }

    /// If the interval has elapsed, re-stat and report whether the
    /// fingerprint moved since the last check. The first check always
    /// reports a change.
    pub fn check(&mut self, now: Instant) -> bool {
        if !self.due(now) {
            return false;
        }
        self.force(now)
    }

    /// Re-stat now regardless of the interval.
    pub fn force(&mut self, now: Instant) -> bool {
        self.last_check = Some(now);
        let fp = fingerprint(&self.targets);
        let changed = self.last.as_ref() != Some(&fp);
        self.last = Some(fp);
        changed
    }

    /// Record the current state as seen without reporting it, e.g. right
    /// after this process wrote the files itself.
    pub fn mark_seen(&mut self, now: Instant) {
        self.last_check = Some(now);
        self.last = Some(fingerprint(&self.targets));
    }
}
