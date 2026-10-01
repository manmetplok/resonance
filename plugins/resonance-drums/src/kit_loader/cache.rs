//! The process-wide decoded-sample cache (drums-plugin-rework.md §7 E5).
//!
//! Every kit load asks this cache for its takes instead of decoding them
//! itself. A take is keyed by its file — canonical path, modification
//! time and length — and the rate it is decoded at, and the cache holds
//! only a [`Weak`] reference to it: the kits that use a take keep it
//! alive, and once the last of them is gone its memory is freed and the
//! entry is swept. So two plugin instances on the same kit at the same
//! rate hold one copy of it, and the second one decodes nothing.
//!
//! # Concurrency
//!
//! Lookups take the map lock only to find (or create) the entry's slot;
//! the decode runs under the slot's own lock. Two loaders asking for the
//! same take at once therefore decode it once — the second waits on the
//! slot and gets the first one's result — while loaders on different
//! takes never wait on each other. Nothing here runs on the audio thread.
//!
//! # Disk streaming (E14)
//!
//! What the cache shares is whatever [`SampleData`] the decode closure
//! builds. When streaming lands, that becomes the resident head of a take
//! (plus how to find its tail on disk); the key, the sharing and the sweep
//! stay as they are.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::SystemTime;

use parking_lot::Mutex;

use crate::kit::{decode_sample, SampleData};

/// What identifies one decoded take: the file as it is on disk now, and
/// the rate it was decoded at. A file rewritten in place (new mtime or
/// length) is a new key, so a stale decode is never served for it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SampleKey {
    /// Canonical path, so two spellings of one file share an entry.
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub len: u64,
    /// Target rate as `f32::to_bits`.
    pub rate_bits: u32,
}

impl SampleKey {
    /// The key of `path` as it is on disk now, decoded at `sample_rate`.
    pub fn for_file(path: &Path, sample_rate: f32) -> std::io::Result<Self> {
        let path = path.canonicalize()?;
        let meta = std::fs::metadata(&path)?;
        Ok(Self {
            path,
            modified: meta.modified().ok(),
            len: meta.len(),
            rate_bits: sample_rate.to_bits(),
        })
    }
}

/// Where [`SampleCache::get_or_decode`] got a take from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Someone already held it; nothing was decoded.
    Cached,
    /// Decoded from disk by this call.
    Decoded,
}

/// One entry: the take, while anyone holds it.
#[derive(Default)]
struct Slot {
    sample: Mutex<Weak<SampleData>>,
}

/// A snapshot of the cache's contents.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Entries in the map, live or not yet swept.
    pub entries: usize,
    /// Entries whose take is still held by some kit.
    pub live: usize,
    /// Sample bytes of the live takes: the decoded audio this process
    /// holds across every instance, each take counted once.
    pub resident_bytes: u64,
}

/// See the module docs. One per process ([`global`]); tests may build
/// their own.
#[derive(Default)]
pub struct SampleCache {
    slots: Mutex<HashMap<SampleKey, Arc<Slot>>>,
    decodes: AtomicU64,
}

impl SampleCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The take of the file at `path` at `sample_rate`: the one already
    /// held if any instance holds it, else read and decoded now (mono
    /// stays mono). An unreadable or undecodable file is an error and is
    /// not cached — the next load tries it again.
    ///
    /// The key is the file as it was stat'ed *before* the read. A file
    /// rewritten while it is being read (a kit still extracting, a sync
    /// client) would otherwise be cached under the key of a version it
    /// was not read from, and served for that version until the next
    /// rewrite. So the file is stat'ed again after the read; if it
    /// changed, the take goes to this caller but is not cached.
    pub fn get_or_decode(
        &self,
        path: &Path,
        sample_rate: f32,
    ) -> Result<(Arc<SampleData>, Source), String> {
        self.get_or_decode_with_hook(path, sample_rate, || {})
    }

    /// [`get_or_decode`](Self::get_or_decode) with `after_read` run
    /// between the read and the second stat. Test hook: lets a test
    /// rewrite the file exactly there.
    #[doc(hidden)]
    pub fn get_or_decode_with_hook(
        &self,
        path: &Path,
        sample_rate: f32,
        after_read: impl FnOnce(),
    ) -> Result<(Arc<SampleData>, Source), String> {
        let key = SampleKey::for_file(path, sample_rate)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        let read_key = key.clone();
        self.get_or_insert_checked(key, || {
            let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
            after_read();
            let unchanged =
                SampleKey::for_file(path, sample_rate).is_ok_and(|after| after == read_key);
            let sample = decode_sample(bytes, sample_rate)
                .map_err(|e| format!("decode {}: {e}", path.display()))?;
            Ok((sample, unchanged))
        })
    }

    /// The take under `key`, or `decode()`'s, which is then cached. Only
    /// one caller per key decodes at a time; the others wait for it.
    pub fn get_or_insert_with(
        &self,
        key: SampleKey,
        decode: impl FnOnce() -> Result<SampleData, String>,
    ) -> Result<(Arc<SampleData>, Source), String> {
        self.get_or_insert_checked(key, || decode().map(|sample| (sample, true)))
    }

    /// [`get_or_insert_with`](Self::get_or_insert_with) for a `decode`
    /// that also says whether its take may be cached under `key` (false:
    /// the file changed under the read — see
    /// [`get_or_decode`](Self::get_or_decode)).
    fn get_or_insert_checked(
        &self,
        key: SampleKey,
        decode: impl FnOnce() -> Result<(SampleData, bool), String>,
    ) -> Result<(Arc<SampleData>, Source), String> {
        let slot = self.slots.lock().entry(key).or_default().clone();
        let mut held = slot.sample.lock();
        if let Some(sample) = held.upgrade() {
            return Ok((sample, Source::Cached));
        }
        let (sample, cacheable) = decode()?;
        let sample = Arc::new(sample);
        self.decodes.fetch_add(1, Ordering::Relaxed);
        if cacheable {
            *held = Arc::downgrade(&sample);
        }
        Ok((sample, Source::Decoded))
    }

    /// The take under `key` if some kit still holds it.
    pub fn lookup(&self, key: &SampleKey) -> Option<Arc<SampleData>> {
        let slot = self.slots.lock().get(key)?.clone();
        let held = slot.sample.lock();
        held.upgrade()
    }

    /// Drop the entries whose take nobody holds any more. Returns how many
    /// went. An entry a loader is decoding into right now is kept.
    pub fn sweep(&self) -> usize {
        let mut slots = self.slots.lock();
        let before = slots.len();
        slots.retain(|_, slot| {
            // Another reference to the slot is a lookup in progress (the
            // map lock is held, so no new one can start).
            if Arc::strong_count(slot) > 1 {
                return true;
            }
            match slot.sample.try_lock() {
                Some(held) => held.strong_count() > 0,
                None => true,
            }
        });
        before - slots.len()
    }

    /// What the cache holds now.
    pub fn stats(&self) -> CacheStats {
        let slots = self.slots.lock();
        let mut stats = CacheStats {
            entries: slots.len(),
            ..CacheStats::default()
        };
        for slot in slots.values() {
            if let Some(sample) = slot.sample.try_lock().and_then(|w| w.upgrade()) {
                stats.live += 1;
                stats.resident_bytes += sample.bytes() as u64;
            }
        }
        stats
    }

    /// Files this cache has decoded since it was made. A test hook: the
    /// process-wide count mixes every test in a binary, so per-load
    /// assertions read [`super::LoadStats`] instead.
    #[doc(hidden)]
    pub fn decode_count(&self) -> u64 {
        self.decodes.load(Ordering::Relaxed)
    }
}

/// The process-wide cache every kit load shares.
pub fn global() -> &'static SampleCache {
    static CACHE: OnceLock<SampleCache> = OnceLock::new();
    CACHE.get_or_init(SampleCache::new)
}
