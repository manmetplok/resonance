//! `clap.preset-discovery-factory`: indexing a plugin's presets without
//! instantiating it (plugin-preset-library.md §8 tier T1, slice P8).
//!
//! The indexer asks each provider for its file types and locations, then
//! for the metadata of every location — a `PLUGIN` location as a whole, a
//! `FILE` location per file (walking a directory for the declared
//! extensions). What comes back is a flat list of [`DiscoveredPreset`]s:
//! name, load key, location, the plugin ids it is for, creators,
//! description, features and flags.
//!
//! Threading: none of this touches a plugin instance, so it runs on the
//! scan's discovery worker (`engine::scan`), never on the engine or audio
//! thread. A provider is created, used and destroyed on that one thread,
//! which is all CLAP asks.
//!
//! The result is cached per plugin id in `<library>/discovered/<id>.json`,
//! keyed by the binary's size and modification time, so a start does not
//! re-index an unchanged plugin.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};

use clap_sys::factory::preset_discovery::*;
use clap_sys::timestamp::clap_timestamp;
use clap_sys::universal_plugin_id::clap_universal_plugin_id;
use clap_sys::version::CLAP_VERSION;
use serde::{Deserialize, Serialize};

/// Where a discovered preset lives, as `clap.preset-load` names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscoveredLocation {
    Plugin,
    File(PathBuf),
}

/// One preset a provider described.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredPreset {
    pub name: String,
    pub location: DiscoveredLocation,
    pub load_key: Option<String>,
    /// CLAP plugin ids (`abi == "clap"`) the preset is for. Empty means
    /// "the plugins of this bundle".
    pub plugin_ids: Vec<String>,
    pub creators: Vec<String>,
    pub description: Option<String>,
    pub features: Vec<String>,
    /// `CLAP_PRESET_DISCOVERY_IS_*`.
    pub flags: u32,
}

impl DiscoveredPreset {
    pub fn is_favorite(&self) -> bool {
        self.flags & CLAP_PRESET_DISCOVERY_IS_FAVORITE != 0
    }

    /// A stable id for the preset within its plugin: the load key for a
    /// `PLUGIN` location, the path (plus the key) for a file.
    pub fn stable_id(&self) -> String {
        match (&self.location, &self.load_key) {
            (DiscoveredLocation::Plugin, Some(k)) => format!("plugin:{k}"),
            (DiscoveredLocation::Plugin, None) => format!("plugin:{}", self.name),
            (DiscoveredLocation::File(p), Some(k)) => format!("file:{}#{k}", p.display()),
            (DiscoveredLocation::File(p), None) => format!("file:{}", p.display()),
        }
    }
}

// ---------------------------------------------------------------------------
// Indexer
// ---------------------------------------------------------------------------

#[derive(Default)]
struct IndexerState {
    extensions: Vec<String>,
    locations: Vec<(u32, Option<String>)>,
}

#[derive(Default)]
struct ReceiverState {
    location: Option<DiscoveredLocation>,
    presets: Vec<DiscoveredPreset>,
}

unsafe fn opt_str(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        // SAFETY: a NUL-terminated string from the provider, valid for the call.
        Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    }
}

unsafe fn indexer_state<'a>(indexer: *const clap_preset_discovery_indexer) -> &'a mut IndexerState {
    // SAFETY: `indexer_data` is the `IndexerState` `index_factory` owns
    // for the whole indexing run, on this one thread.
    unsafe { &mut *((*indexer).indexer_data as *mut IndexerState) }
}

unsafe extern "C" fn declare_filetype(
    indexer: *const clap_preset_discovery_indexer,
    filetype: *const clap_preset_discovery_filetype,
) -> bool {
    if filetype.is_null() {
        return false;
    }
    // SAFETY: see `indexer_state`; `filetype` is valid for the call.
    unsafe {
        if let Some(ext) = opt_str((*filetype).file_extension).filter(|e| !e.is_empty()) {
            indexer_state(indexer)
                .extensions
                .push(ext.trim_start_matches('.').to_ascii_lowercase());
        }
    }
    true
}

unsafe extern "C" fn declare_location(
    indexer: *const clap_preset_discovery_indexer,
    location: *const clap_preset_discovery_location,
) -> bool {
    if location.is_null() {
        return false;
    }
    // SAFETY: as for `declare_filetype`.
    unsafe {
        let kind = (*location).kind;
        let path = opt_str((*location).location);
        indexer_state(indexer).locations.push((kind, path));
    }
    true
}

unsafe extern "C" fn declare_soundpack(
    _indexer: *const clap_preset_discovery_indexer,
    _soundpack: *const clap_preset_discovery_soundpack,
) -> bool {
    true
}

unsafe extern "C" fn indexer_get_extension(
    _indexer: *const clap_preset_discovery_indexer,
    _id: *const c_char,
) -> *const c_void {
    std::ptr::null()
}

unsafe fn receiver_state<'a>(
    receiver: *const clap_preset_discovery_metadata_receiver,
) -> &'a mut ReceiverState {
    // SAFETY: `receiver_data` is the `ReceiverState` of the running
    // `get_metadata` call, on this thread.
    unsafe { &mut *((*receiver).receiver_data as *mut ReceiverState) }
}

unsafe fn current<'a>(
    receiver: *const clap_preset_discovery_metadata_receiver,
) -> Option<&'a mut DiscoveredPreset> {
    // SAFETY: see `receiver_state`.
    unsafe { receiver_state(receiver).presets.last_mut() }
}

unsafe extern "C" fn on_error(
    _receiver: *const clap_preset_discovery_metadata_receiver,
    os_error: i32,
    message: *const c_char,
) {
    // SAFETY: valid for the call.
    let message = unsafe { opt_str(message) }.unwrap_or_default();
    tracing::debug!("preset discovery: provider error {os_error}: {message}");
}

unsafe extern "C" fn begin_preset(
    receiver: *const clap_preset_discovery_metadata_receiver,
    name: *const c_char,
    load_key: *const c_char,
) -> bool {
    // SAFETY: see `receiver_state`; strings valid for the call.
    unsafe {
        let state = receiver_state(receiver);
        let Some(location) = state.location.clone() else {
            return false;
        };
        state.presets.push(DiscoveredPreset {
            name: opt_str(name).unwrap_or_default(),
            location,
            load_key: opt_str(load_key),
            plugin_ids: Vec::new(),
            creators: Vec::new(),
            description: None,
            features: Vec::new(),
            flags: 0,
        });
    }
    true
}

unsafe extern "C" fn add_plugin_id(
    receiver: *const clap_preset_discovery_metadata_receiver,
    plugin_id: *const clap_universal_plugin_id,
) {
    if plugin_id.is_null() {
        return;
    }
    // SAFETY: as for `begin_preset`.
    unsafe {
        let abi = opt_str((*plugin_id).abi).unwrap_or_default();
        if let (true, Some(id), Some(p)) = (abi == "clap", opt_str((*plugin_id).id), current(receiver)) {
            p.plugin_ids.push(id);
        }
    }
}

unsafe extern "C" fn set_soundpack_id(
    _receiver: *const clap_preset_discovery_metadata_receiver,
    _id: *const c_char,
) {
}

unsafe extern "C" fn set_flags(receiver: *const clap_preset_discovery_metadata_receiver, flags: u32) {
    // SAFETY: as for `begin_preset`.
    if let Some(p) = unsafe { current(receiver) } {
        p.flags = flags;
    }
}

unsafe extern "C" fn add_creator(
    receiver: *const clap_preset_discovery_metadata_receiver,
    creator: *const c_char,
) {
    // SAFETY: as for `begin_preset`.
    unsafe {
        if let (Some(c), Some(p)) = (opt_str(creator), current(receiver)) {
            p.creators.push(c);
        }
    }
}

unsafe extern "C" fn set_description(
    receiver: *const clap_preset_discovery_metadata_receiver,
    description: *const c_char,
) {
    // SAFETY: as for `begin_preset`.
    unsafe {
        if let (Some(d), Some(p)) = (opt_str(description), current(receiver)) {
            p.description = Some(d);
        }
    }
}

unsafe extern "C" fn set_timestamps(
    _receiver: *const clap_preset_discovery_metadata_receiver,
    _created: clap_timestamp,
    _modified: clap_timestamp,
) {
}

unsafe extern "C" fn add_feature(
    receiver: *const clap_preset_discovery_metadata_receiver,
    feature: *const c_char,
) {
    // SAFETY: as for `begin_preset`.
    unsafe {
        if let (Some(f), Some(p)) = (opt_str(feature), current(receiver)) {
            p.features.push(f);
        }
    }
}

unsafe extern "C" fn add_extra_info(
    _receiver: *const clap_preset_discovery_metadata_receiver,
    _key: *const c_char,
    _value: *const c_char,
) {
}

/// Files under `dir` (depth-capped) with one of `extensions` (any file
/// when none were declared), sorted.
fn walk(dir: &Path, extensions: &[String], depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            walk(&path, extensions, depth + 1, out);
        } else if extensions.is_empty()
            || path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .is_some_and(|e| extensions.contains(&e))
        {
            out.push(path);
        }
    }
}

/// Index every provider of `factory`.
///
/// # Safety
/// `factory` is a live `clap_preset_discovery_factory` (from a loaded
/// bundle's `get_factory`, or a test's), used on this thread only.
pub unsafe fn index_factory(factory: *const clap_preset_discovery_factory) -> Vec<DiscoveredPreset> {
    let mut out = Vec::new();
    if factory.is_null() {
        return out;
    }
    // SAFETY: the caller guarantees `factory`; every provider call below
    // follows preset-discovery.h (init before use, destroy after).
    unsafe {
        let (Some(count), Some(get_descriptor), Some(create)) = (
            (*factory).count,
            (*factory).get_descriptor,
            (*factory).create,
        ) else {
            return out;
        };
        for i in 0..count(factory) {
            let desc = get_descriptor(factory, i);
            if desc.is_null() || (*desc).id.is_null() {
                continue;
            }
            let mut state = IndexerState::default();
            let indexer = clap_preset_discovery_indexer {
                clap_version: CLAP_VERSION,
                name: c"Resonance".as_ptr(),
                vendor: c"Resonance".as_ptr(),
                url: c"".as_ptr(),
                version: c"0.1.0".as_ptr(),
                indexer_data: &mut state as *mut IndexerState as *mut c_void,
                declare_filetype: Some(declare_filetype),
                declare_location: Some(declare_location),
                declare_soundpack: Some(declare_soundpack),
                get_extension: Some(indexer_get_extension),
            };
            let provider = create(factory, &indexer, (*desc).id);
            if provider.is_null() {
                continue;
            }
            let ok = (*provider).init.is_some_and(|init| init(provider));
            if ok {
                index_provider(provider, &state, &mut out);
            }
            if let Some(destroy) = (*provider).destroy {
                destroy(provider);
            }
        }
    }
    out
}

unsafe fn index_provider(
    provider: *const clap_preset_discovery_provider,
    state: &IndexerState,
    out: &mut Vec<DiscoveredPreset>,
) {
    // SAFETY: `provider` is initialized and live for this call.
    unsafe {
        let Some(get_metadata) = (*provider).get_metadata else {
            return;
        };
        let mut receiver_state = ReceiverState::default();
        let receiver = clap_preset_discovery_metadata_receiver {
            receiver_data: &mut receiver_state as *mut ReceiverState as *mut c_void,
            on_error: Some(on_error),
            begin_preset: Some(begin_preset),
            add_plugin_id: Some(add_plugin_id),
            set_soundpack_id: Some(set_soundpack_id),
            set_flags: Some(set_flags),
            add_creator: Some(add_creator),
            set_description: Some(set_description),
            set_timestamps: Some(set_timestamps),
            add_feature: Some(add_feature),
            add_extra_info: Some(add_extra_info),
        };
        for (kind, location) in &state.locations {
            if *kind == CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN {
                (*receiver.receiver_data.cast::<ReceiverState>()).location =
                    Some(DiscoveredLocation::Plugin);
                get_metadata(provider, *kind, std::ptr::null(), &receiver);
                continue;
            }
            let Some(root) = location.as_deref().map(PathBuf::from) else {
                continue;
            };
            let files = if root.is_dir() {
                let mut files = Vec::new();
                walk(&root, &state.extensions, 0, &mut files);
                files
            } else {
                vec![root]
            };
            for file in files {
                let Ok(c_path) = CString::new(file.to_string_lossy().into_owned()) else {
                    continue;
                };
                (*receiver.receiver_data.cast::<ReceiverState>()).location =
                    Some(DiscoveredLocation::File(file.clone()));
                get_metadata(provider, *kind, c_path.as_ptr(), &receiver);
            }
        }
        out.append(&mut receiver_state.presets);
    }
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

/// The binary's identity for the cache: size and modification time.
fn stamp(binary: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(binary).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some((meta.len(), modified))
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    binary: String,
    size: u64,
    modified: u64,
    presets: Vec<DiscoveredPreset>,
}

fn cache_path(cache_dir: &Path, plugin_id: &str) -> PathBuf {
    let safe: String = plugin_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' })
        .collect();
    cache_dir.join("discovered").join(format!("{safe}.json"))
}

/// The cached presets of `plugin_id`, when the cache was written for this
/// exact binary.
pub fn read_cache(cache_dir: &Path, plugin_id: &str, binary: &Path) -> Option<Vec<DiscoveredPreset>> {
    let (size, modified) = stamp(binary)?;
    let text = std::fs::read_to_string(cache_path(cache_dir, plugin_id)).ok()?;
    let file: CacheFile = serde_json::from_str(&text).ok()?;
    (file.size == size && file.modified == modified && file.binary == binary.to_string_lossy())
        .then_some(file.presets)
}

/// Write the cache for `plugin_id` (best effort, atomically).
pub fn write_cache(cache_dir: &Path, plugin_id: &str, binary: &Path, presets: &[DiscoveredPreset]) {
    let Some((size, modified)) = stamp(binary) else {
        return;
    };
    let file = CacheFile {
        binary: binary.to_string_lossy().into_owned(),
        size,
        modified,
        presets: presets.to_vec(),
    };
    let path = cache_path(cache_dir, plugin_id);
    if let (Some(parent), Ok(text)) = (path.parent(), serde_json::to_string_pretty(&file)) {
        if std::fs::create_dir_all(parent).is_ok() {
            if let Err(e) = resonance_common::atomic_file::atomic_write(&path, text.as_bytes()) {
                tracing::debug!("preset discovery: cache not written: {e}");
            }
        }
    }
}

/// The presets of each of `plugin_ids` (a bundle's plugins), from the
/// cache when it is fresh, else by indexing `factory` (and caching).
///
/// # Safety
/// As for [`index_factory`].
pub unsafe fn discover(
    factory: *const clap_preset_discovery_factory,
    binary: &Path,
    plugin_ids: &[String],
    cache_dir: Option<&Path>,
) -> Vec<(String, Vec<DiscoveredPreset>)> {
    if let Some(dir) = cache_dir {
        let cached: Option<Vec<_>> = plugin_ids
            .iter()
            .map(|id| read_cache(dir, id, binary).map(|p| (id.clone(), p)))
            .collect();
        if let Some(cached) = cached {
            return cached;
        }
    }
    // SAFETY: forwarded from the caller.
    let all = unsafe { index_factory(factory) };
    let per_plugin: Vec<(String, Vec<DiscoveredPreset>)> = plugin_ids
        .iter()
        .map(|id| {
            let mine = all
                .iter()
                .filter(|p| p.plugin_ids.is_empty() || p.plugin_ids.contains(id))
                .cloned()
                .collect();
            (id.clone(), mine)
        })
        .collect();
    if let Some(dir) = cache_dir {
        for (id, presets) in &per_plugin {
            write_cache(dir, id, binary, presets);
        }
    }
    per_plugin
}
