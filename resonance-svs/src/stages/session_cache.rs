//! Process-wide cache of loaded ONNX sessions, so a re-render that only
//! re-synthesises one changed unit does not first spend 0.3–0.7 s (and a
//! transient copy of every model) loading the bank's four to eight
//! sessions again.
//!
//! An entry is keyed by the model file — its path, length and
//! modification time, so a replaced file is never served from the cache —
//! the execution provider and the type of the loaded value (an acoustic
//! stage, a vocoder stage, a bare or seeded session). It belongs to a
//! **group**, the bank it was loaded for (the acoustic config's folder);
//! the cache keeps the [`MAX_GROUPS`] most recently used groups and drops
//! the rest. [`invalidate`] drops everything (the app calls it when a
//! voicebank rescan finds a bank changed).
//!
//! Values are shared as `Arc<Mutex<T>>`: a run needs the session
//! mutably, so two renders of the same bank on two threads take turns per
//! model run.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::Result;

use super::common::ExecutionProvider;

/// How many banks' sessions stay loaded.
pub const MAX_GROUPS: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    group: PathBuf,
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
    ep: ExecutionProvider,
    kind: TypeId,
}

#[derive(Default)]
struct Cache {
    generation: u64,
    entries: HashMap<Key, Arc<dyn Any + Send + Sync>>,
    /// Groups, most recently used first.
    groups: Vec<PathBuf>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn with_cache<R>(f: impl FnOnce(&mut Cache) -> R) -> R {
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(Cache::default))
}

/// The loaded value for model file `path` in bank `group`, loading it
/// with `load` when the cache does not hold it (or the file changed). The
/// load runs outside the cache's lock; two threads missing at once both
/// load, and the first to finish is kept.
pub fn get_or_load<T: Send + 'static>(
    group: &Path,
    path: &Path,
    ep: ExecutionProvider,
    load: impl FnOnce() -> Result<T>,
) -> Result<Arc<Mutex<T>>> {
    let meta = std::fs::metadata(path).ok();
    let key = Key {
        group: group.to_path_buf(),
        path: path.to_path_buf(),
        len: meta.as_ref().map_or(0, |m| m.len()),
        modified: meta.and_then(|m| m.modified().ok()),
        ep,
        kind: TypeId::of::<T>(),
    };
    let hit = with_cache(|c| {
        touch(c, group);
        c.entries.get(&key).cloned()
    });
    if let Some(hit) = hit.and_then(|v| v.downcast::<Mutex<T>>().ok()) {
        return Ok(hit);
    }
    let generation = generation();
    let value = Arc::new(Mutex::new(load()?));
    let kept = with_cache(|c| {
        // A rescan that invalidated the cache while this loaded may have
        // seen the file change: keep the value for this render only.
        if c.generation != generation {
            return value.clone();
        }
        // Another stale entry for the same file (an older length or time)
        // goes.
        c.entries.retain(|k, _| {
            !(k.group == key.group && k.path == key.path && k.kind == key.kind && k.ep == key.ep)
        });
        let entry = c
            .entries
            .entry(key)
            .or_insert_with(|| value.clone() as Arc<dyn Any + Send + Sync>)
            .clone();
        touch(c, group);
        entry.downcast::<Mutex<T>>().unwrap_or(value)
    });
    Ok(kept)
}

/// Mark `group` most recently used and drop the groups past
/// [`MAX_GROUPS`].
fn touch(c: &mut Cache, group: &Path) {
    c.groups.retain(|g| g != group);
    c.groups.insert(0, group.to_path_buf());
    if c.groups.len() > MAX_GROUPS {
        let dropped: Vec<PathBuf> = c.groups.split_off(MAX_GROUPS);
        c.entries.retain(|k, _| !dropped.contains(&k.group));
    }
}

/// Drop every cached session and start a new generation.
pub fn invalidate() {
    with_cache(|c| {
        c.generation += 1;
        c.entries.clear();
        c.groups.clear();
    });
}

/// Bumped by every [`invalidate`].
pub fn generation() -> u64 {
    with_cache(|c| c.generation)
}

/// How many values the cache holds (for tests and logs).
pub fn len() -> usize {
    with_cache(|c| c.entries.len())
}
