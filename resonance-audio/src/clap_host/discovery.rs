//! `clap.preset-discovery-factory`: indexing a plugin's presets without
//! instantiating it (plugin-preset-library.md §8 tier T1, slice P8).
//!
//! Each provider is created and `init`ed; what it declares during `init`
//! (file types, locations) is copied out and sealed — a declaration after
//! `init` is ignored, so the provider can never grow a list the indexer is
//! walking. Then it is asked for the metadata of each `PLUGIN` location as
//! a whole and of each file of each `FILE` location (a directory walked,
//! depth-capped, for the declared extensions), and destroyed.
//!
//! Threading: none of this touches a plugin instance, so it runs on the
//! scan's single discovery worker (`engine::scan`), never on the engine or
//! audio thread. A provider is created, used and destroyed on that thread.
//!
//! Cache (`<cache>/preset-discovery/<bundle>.json`): per provider, its
//! declarations and `PLUGIN`-location presets, keyed on the binary's size
//! and mtime; and per file of its `FILE` locations, that file's presets
//! keyed on the file's size and mtime (ns). A start with the binary
//! unchanged re-walks the `FILE` locations and asks the provider only
//! about files that are new or changed; `force` (a rescan) re-indexes all.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use clap_sys::factory::preset_discovery::*;
use clap_sys::timestamp::clap_timestamp;
use clap_sys::universal_plugin_id::clap_universal_plugin_id;
use clap_sys::version::CLAP_VERSION;
use serde::{Deserialize, Serialize};

pub use crate::types::{DiscoveredLocation, DiscoveredPreset};

// ---------------------------------------------------------------------------
// Declarations and receiver
// ---------------------------------------------------------------------------

/// One declared location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Location {
    kind: u32,
    flags: u32,
    path: Option<String>,
}

#[derive(Default)]
struct IndexerState {
    extensions: Vec<String>,
    locations: Vec<Location>,
    /// Set once `init` returned: later declarations are ignored.
    sealed: bool,
}

#[derive(Default)]
struct ReceiverState {
    location: Option<DiscoveredLocation>,
    /// The flags of the location being read, which a preset inherits when
    /// it never calls `set_flags`.
    location_flags: u32,
    /// The file being read, whose stem names a preset with no name.
    file_stem: Option<String>,
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

/// A path as a C string, byte-exact on unix (non-UTF-8 names included).
fn c_path(path: &Path) -> Option<CString> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        CString::new(path.as_os_str().as_bytes()).ok()
    }
    #[cfg(not(unix))]
    {
        CString::new(path.to_string_lossy().into_owned()).ok()
    }
}

/// A C path back to a `PathBuf`, byte-exact on unix.
unsafe fn path_from(p: *const c_char) -> Option<PathBuf> {
    if p.is_null() {
        return None;
    }
    // SAFETY: as for `opt_str`.
    let bytes = unsafe { CStr::from_ptr(p) }.to_bytes();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Some(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
    }
    #[cfg(not(unix))]
    {
        Some(PathBuf::from(String::from_utf8_lossy(bytes).into_owned()))
    }
}

unsafe fn indexer_state<'a>(indexer: *const clap_preset_discovery_indexer) -> &'a mut IndexerState {
    // SAFETY: `indexer_data` is the `IndexerState` the indexing run owns,
    // used on this one thread, and never borrowed elsewhere while a
    // provider call is running.
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
        let state = indexer_state(indexer);
        if state.sealed {
            return false;
        }
        if let Some(ext) = opt_str((*filetype).file_extension).filter(|e| !e.is_empty()) {
            state
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
        let state = indexer_state(indexer);
        if state.sealed {
            return false;
        }
        state.locations.push(Location {
            kind: (*location).kind,
            flags: (*location).flags,
            path: path_from((*location).location).map(|p| p.to_string_lossy().into_owned()),
        });
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
        let name = opt_str(name)
            .filter(|n| !n.is_empty())
            .or_else(|| state.file_stem.clone())
            .unwrap_or_default();
        state.presets.push(DiscoveredPreset {
            name,
            location,
            load_key: opt_str(load_key),
            plugin_ids: Vec::new(),
            creators: Vec::new(),
            description: None,
            features: Vec::new(),
            flags: state.location_flags,
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
        if abi != "clap" {
            return;
        }
        if let (Some(id), Some(p)) = (opt_str((*plugin_id).id), current(receiver)) {
            p.plugin_ids.push(id);
        }
    }
}

unsafe extern "C" fn set_soundpack_id(
    _receiver: *const clap_preset_discovery_metadata_receiver,
    _id: *const c_char,
) {
}

unsafe extern "C" fn set_flags(
    receiver: *const clap_preset_discovery_metadata_receiver,
    flags: u32,
) {
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

// ---------------------------------------------------------------------------
// A live provider
// ---------------------------------------------------------------------------

/// A created and initialised provider, destroyed on drop.
struct Provider {
    ptr: *const clap_preset_discovery_provider,
    /// Owns what the indexer struct points at; boxed so its address holds.
    _state: Box<IndexerState>,
    _indexer: Box<clap_preset_discovery_indexer>,
    extensions: Vec<String>,
    locations: Vec<Location>,
}

impl Provider {
    /// Create and `init` the provider `id` of `factory`.
    ///
    /// # Safety
    /// `factory` is live and used on this thread only.
    unsafe fn open(
        factory: *const clap_preset_discovery_factory,
        id: *const c_char,
    ) -> Option<Self> {
        // SAFETY: forwarded; preset-discovery.h: create, then init before use.
        unsafe {
            let create = (*factory).create?;
            let mut state = Box::<IndexerState>::default();
            let indexer = Box::new(clap_preset_discovery_indexer {
                clap_version: CLAP_VERSION,
                name: c"Resonance".as_ptr(),
                vendor: c"Resonance".as_ptr(),
                url: c"".as_ptr(),
                version: c"0.1.0".as_ptr(),
                indexer_data: &mut *state as *mut IndexerState as *mut c_void,
                declare_filetype: Some(declare_filetype),
                declare_location: Some(declare_location),
                declare_soundpack: Some(declare_soundpack),
                get_extension: Some(indexer_get_extension),
            });
            let ptr = create(factory, &*indexer, id);
            if ptr.is_null() {
                return None;
            }
            let ok = (*ptr).init.is_some_and(|init| init(ptr));
            // Copy the declarations out and seal: from here on nothing the
            // provider declares can change what is being walked.
            state.sealed = true;
            let (extensions, locations) = (state.extensions.clone(), state.locations.clone());
            let provider = Self {
                ptr,
                _state: state,
                _indexer: indexer,
                extensions,
                locations,
            };
            ok.then_some(provider)
        }
    }

    /// The presets at one location (`None` path = `PLUGIN`).
    fn metadata(&self, location: &Location, file: Option<&Path>) -> Vec<DiscoveredPreset> {
        // SAFETY: the provider is initialised and live for `self`.
        unsafe {
            let Some(get_metadata) = (*self.ptr).get_metadata else {
                return Vec::new();
            };
            let mut state = ReceiverState {
                location: Some(match file {
                    Some(f) => DiscoveredLocation::File(f.to_path_buf()),
                    None => DiscoveredLocation::Plugin,
                }),
                location_flags: location.flags,
                file_stem: file
                    .and_then(|f| f.file_stem())
                    .map(|s| s.to_string_lossy().into_owned()),
                presets: Vec::new(),
            };
            let receiver = clap_preset_discovery_metadata_receiver {
                receiver_data: &mut state as *mut ReceiverState as *mut c_void,
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
            let c = file.and_then(c_path);
            if file.is_some() && c.is_none() {
                return Vec::new();
            }
            get_metadata(
                self.ptr,
                location.kind,
                c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                &receiver,
            );
            std::mem::take(&mut state.presets)
        }
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        // SAFETY: created by `open`, destroyed once, on this thread.
        unsafe {
            if let Some(destroy) = (*self.ptr).destroy {
                destroy(self.ptr);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

/// A file's identity: size and mtime in nanoseconds.
fn file_stamp(path: &Path) -> Option<(u64, u128)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((meta.len(), modified))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileEntry {
    path: PathBuf,
    size: u64,
    modified_ns: u128,
    presets: Vec<DiscoveredPreset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderCache {
    id: String,
    extensions: Vec<String>,
    locations: Vec<Location>,
    plugin_presets: Vec<DiscoveredPreset>,
    files: Vec<FileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheFile {
    binary: PathBuf,
    size: u64,
    modified_ns: u128,
    providers: Vec<ProviderCache>,
}

fn cache_path(cache_dir: &Path, binary: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    binary.hash(&mut h);
    let stem: String = binary
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .collect();
    cache_dir
        .join("preset-discovery")
        .join(format!("{stem}-{:016x}.json", h.finish()))
}

fn read_cache(cache_dir: &Path, binary: &Path) -> Option<CacheFile> {
    let (size, modified_ns) = file_stamp(binary)?;
    let text = std::fs::read_to_string(cache_path(cache_dir, binary)).ok()?;
    let file: CacheFile = serde_json::from_str(&text).ok()?;
    (file.size == size && file.modified_ns == modified_ns && file.binary == binary)
        .then_some(file)
}

fn write_cache(cache_dir: &Path, file: &CacheFile) {
    let path = cache_path(cache_dir, &file.binary);
    if let (Some(parent), Ok(text)) = (path.parent(), serde_json::to_string_pretty(file)) {
        if std::fs::create_dir_all(parent).is_ok() {
            if let Err(e) = resonance_common::atomic_file::atomic_write(&path, text.as_bytes()) {
                tracing::debug!("preset discovery: cache not written: {e}");
            }
        }
    }
}

/// The provider ids of `factory`.
unsafe fn provider_ids(factory: *const clap_preset_discovery_factory) -> Vec<CString> {
    // SAFETY: forwarded from the caller.
    unsafe {
        let (Some(count), Some(get_descriptor)) = ((*factory).count, (*factory).get_descriptor)
        else {
            return Vec::new();
        };
        (0..count(factory))
            .filter_map(|i| {
                let desc = get_descriptor(factory, i);
                (!desc.is_null() && !(*desc).id.is_null())
                    .then(|| CStr::from_ptr((*desc).id).to_owned())
            })
            .collect()
    }
}

/// Re-walk `cache`'s `FILE` locations, reusing each unchanged file's
/// entry and asking the provider (opened on first need) about the rest.
fn refresh_files(
    factory: *const clap_preset_discovery_factory,
    cache: &mut ProviderCache,
    cancel: &AtomicBool,
) {
    let mut provider: Option<Option<Provider>> = None;
    let old: std::collections::HashMap<PathBuf, FileEntry> =
        cache.files.drain(..).map(|f| (f.path.clone(), f)).collect();
    for location in cache.locations.clone() {
        if location.kind != CLAP_PRESET_DISCOVERY_LOCATION_FILE {
            continue;
        }
        let Some(root) = location.path.as_deref().map(PathBuf::from) else {
            continue;
        };
        let files = if root.is_dir() {
            let mut files = Vec::new();
            walk(&root, &cache.extensions, 0, &mut files);
            files
        } else if root.exists() {
            vec![root]
        } else {
            Vec::new()
        };
        for file in files {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let Some((size, modified_ns)) = file_stamp(&file) else {
                continue;
            };
            if let Some(entry) = old
                .get(&file)
                .filter(|e| e.size == size && e.modified_ns == modified_ns)
            {
                cache.files.push(entry.clone());
                continue;
            }
            let provider = provider.get_or_insert_with(|| {
                let Ok(id) = CString::new(cache.id.clone()) else {
                    return None;
                };
                // SAFETY: `factory` is live and on this thread (the caller's
                // contract).
                unsafe { Provider::open(factory, id.as_ptr()) }
            });
            let presets = provider
                .as_ref()
                .map(|p| p.metadata(&location, Some(&file)))
                .unwrap_or_default();
            cache.files.push(FileEntry {
                path: file,
                size,
                modified_ns,
                presets,
            });
        }
    }
}

/// Index every provider of `factory` from scratch.
fn index_all(
    factory: *const clap_preset_discovery_factory,
    cancel: &AtomicBool,
) -> Vec<ProviderCache> {
    let mut out = Vec::new();
    // SAFETY: the caller's contract (see `discover`).
    for id in unsafe { provider_ids(factory) } {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        // SAFETY: as above.
        let Some(provider) = (unsafe { Provider::open(factory, id.as_ptr()) }) else {
            continue;
        };
        let plugin_presets = provider
            .locations
            .iter()
            .filter(|l| l.kind == CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN)
            .flat_map(|l| provider.metadata(l, None))
            .collect();
        let mut cache = ProviderCache {
            id: id.to_string_lossy().into_owned(),
            extensions: provider.extensions.clone(),
            locations: provider.locations.clone(),
            plugin_presets,
            files: Vec::new(),
        };
        drop(provider);
        refresh_files(factory, &mut cache, cancel);
        out.push(cache);
    }
    out
}

/// Every preset of `factory`, flattened (index order: `PLUGIN` locations,
/// then files by path).
///
/// # Safety
/// `factory` is a live `clap_preset_discovery_factory` (from a loaded
/// bundle's `get_factory`, or a test's), used on this thread only.
pub unsafe fn index_factory(
    factory: *const clap_preset_discovery_factory,
) -> Vec<DiscoveredPreset> {
    if factory.is_null() {
        return Vec::new();
    }
    flatten(&index_all(factory, &AtomicBool::new(false)))
}

fn flatten(providers: &[ProviderCache]) -> Vec<DiscoveredPreset> {
    providers
        .iter()
        .flat_map(|p| {
            p.plugin_presets
                .iter()
                .cloned()
                .chain(p.files.iter().flat_map(|f| f.presets.iter().cloned()))
        })
        .collect()
}

/// The presets of each of `plugin_ids` (a bundle's plugins).
///
/// With a fresh cache for this binary (and no `force`), the declarations
/// and `PLUGIN` presets come from it and only new or changed files are
/// read; otherwise every provider is indexed. A preset naming no plugin
/// goes to a bundle's only plugin, never to all of several. `cancel`
/// stops between files.
///
/// # Safety
/// As for [`index_factory`].
pub unsafe fn discover(
    factory: *const clap_preset_discovery_factory,
    binary: &Path,
    plugin_ids: &[String],
    cache_dir: Option<&Path>,
    force: bool,
    cancel: &AtomicBool,
) -> Vec<(String, Vec<DiscoveredPreset>)> {
    if factory.is_null() {
        return Vec::new();
    }
    let cached = if force {
        None
    } else {
        cache_dir.and_then(|dir| read_cache(dir, binary))
    };
    let providers = match cached {
        Some(mut file) => {
            for provider in &mut file.providers {
                refresh_files(factory, provider, cancel);
            }
            file.providers
        }
        None => index_all(factory, cancel),
    };
    if cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }
    if let (Some(dir), Some((size, modified_ns))) = (cache_dir, file_stamp(binary)) {
        write_cache(
            dir,
            &CacheFile {
                binary: binary.to_path_buf(),
                size,
                modified_ns,
                providers: providers.clone(),
            },
        );
    }
    let all = flatten(&providers);
    let single = plugin_ids.len() == 1;
    plugin_ids
        .iter()
        .map(|id| {
            let mine = all
                .iter()
                .filter(|p| p.plugin_ids.contains(id) || (single && p.plugin_ids.is_empty()))
                .cloned()
                .collect();
            (id.clone(), mine)
        })
        .collect()
}
