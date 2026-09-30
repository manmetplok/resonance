//! The `clap.preset-discovery-factory` indexer (plugin-preset-library.md
//! §8 tier T1, slice P8) against a hand-rolled provider: a `PLUGIN`
//! location with two presets, and a `FILE` directory walked for the
//! declared extension; then the cache — declarations per binary, presets
//! per file — and the rules for flags, names and plugin ids.
//!
//! Everything writes under a private temp directory. The fake provider's
//! statics make these tests share one lock.

use std::ffi::{c_char, c_void, CStr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use clap_sys::factory::preset_discovery::*;
use clap_sys::universal_plugin_id::clap_universal_plugin_id;
use clap_sys::version::CLAP_VERSION;
use resonance_audio::test_support::discovery::{self, DiscoveredLocation};

const PLUGIN: &CStr = c"com.vendor.synth";

/// The fake provider is process-global; one test at a time drives it.
static SERIAL: Mutex<()> = Mutex::new(());
/// The folder the fake provider declares; set per test.
static FOLDER: Mutex<Option<std::ffi::CString>> = Mutex::new(None);
/// Providers created, and files read (what the cache must avoid).
static CREATED: AtomicU32 = AtomicU32::new(0);
static FILES_READ: AtomicU32 = AtomicU32::new(0);
/// What a `declare_location` made during `get_metadata` returned.
static LATE_DECLARE_ACCEPTED: AtomicBool = AtomicBool::new(false);

static DESC: clap_preset_discovery_provider_descriptor = clap_preset_discovery_provider_descriptor {
    clap_version: CLAP_VERSION,
    id: c"com.vendor.synth.presets".as_ptr(),
    name: c"Vendor presets".as_ptr(),
    vendor: c"Vendor".as_ptr(),
};

struct ProviderBox {
    provider: clap_preset_discovery_provider,
    indexer: *const clap_preset_discovery_indexer,
}

unsafe extern "C" fn count(_f: *const clap_preset_discovery_factory) -> u32 {
    1
}

unsafe extern "C" fn get_descriptor(
    _f: *const clap_preset_discovery_factory,
    _i: u32,
) -> *const clap_preset_discovery_provider_descriptor {
    &DESC
}

fn user_location(path: *const c_char) -> clap_preset_discovery_location {
    clap_preset_discovery_location {
        flags: CLAP_PRESET_DISCOVERY_IS_USER_CONTENT,
        name: c"User".as_ptr(),
        kind: CLAP_PRESET_DISCOVERY_LOCATION_FILE,
        location: path,
    }
}

unsafe extern "C" fn init(provider: *const clap_preset_discovery_provider) -> bool {
    unsafe {
        let b = &*((*provider).provider_data as *const ProviderBox);
        let indexer = b.indexer;
        (*indexer).declare_filetype.unwrap()(
            indexer,
            &clap_preset_discovery_filetype {
                name: c"Vendor preset".as_ptr(),
                description: std::ptr::null(),
                file_extension: c"vpr".as_ptr(),
            },
        );
        (*indexer).declare_location.unwrap()(
            indexer,
            &clap_preset_discovery_location {
                flags: CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT,
                name: c"Factory".as_ptr(),
                kind: CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN,
                location: std::ptr::null(),
            },
        );
        let folder = FOLDER.lock().unwrap().clone().unwrap();
        (*indexer).declare_location.unwrap()(indexer, &user_location(folder.as_ptr()));
    }
    true
}

unsafe extern "C" fn destroy(provider: *const clap_preset_discovery_provider) {
    unsafe {
        drop(Box::from_raw((*provider).provider_data as *mut ProviderBox));
    }
}

unsafe fn factory_preset(
    r: *const clap_preset_discovery_metadata_receiver,
    name: &CStr,
    key: &CStr,
    flags: u32,
) {
    unsafe {
        (*r).begin_preset.unwrap()(r, name.as_ptr(), key.as_ptr());
        (*r).add_plugin_id.unwrap()(
            r,
            &clap_universal_plugin_id {
                abi: c"clap".as_ptr(),
                id: PLUGIN.as_ptr(),
            },
        );
        (*r).set_flags.unwrap()(r, flags);
        (*r).add_creator.unwrap()(r, c"Jane".as_ptr());
        (*r).add_feature.unwrap()(r, c"bass".as_ptr());
    }
}

unsafe extern "C" fn get_metadata(
    provider: *const clap_preset_discovery_provider,
    kind: u32,
    _location: *const c_char,
    r: *const clap_preset_discovery_metadata_receiver,
) -> bool {
    unsafe {
        if kind == CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN {
            // A misbehaving provider declaring while it is being walked.
            let b = &*((*provider).provider_data as *const ProviderBox);
            let late = (*b.indexer).declare_location.unwrap()(
                b.indexer,
                &user_location(c"/nowhere".as_ptr()),
            );
            LATE_DECLARE_ACCEPTED.store(late, Ordering::SeqCst);
            factory_preset(r, c"Deep Bass", c"bank0/1", CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT);
            factory_preset(
                r,
                c"Glass Pad",
                c"bank0/2",
                CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT | CLAP_PRESET_DISCOVERY_IS_FAVORITE,
            );
        } else {
            FILES_READ.fetch_add(1, Ordering::SeqCst);
            // No name (the file's stem names it), no flags (the location's
            // apply), no plugin id (a single-plugin bundle's own).
            (*r).begin_preset.unwrap()(r, std::ptr::null(), std::ptr::null());
        }
    }
    true
}

unsafe extern "C" fn create(
    _f: *const clap_preset_discovery_factory,
    indexer: *const clap_preset_discovery_indexer,
    _id: *const c_char,
) -> *const clap_preset_discovery_provider {
    CREATED.fetch_add(1, Ordering::SeqCst);
    let b = Box::into_raw(Box::new(ProviderBox {
        provider: clap_preset_discovery_provider {
            desc: &DESC,
            provider_data: std::ptr::null_mut(),
            init: Some(init),
            destroy: Some(destroy),
            get_metadata: Some(get_metadata),
            get_extension: None,
        },
        indexer,
    }));
    unsafe {
        (*b).provider.provider_data = b as *mut c_void;
        &(*b).provider
    }
}

static FACTORY: clap_preset_discovery_factory = clap_preset_discovery_factory {
    count: Some(count),
    get_descriptor: Some(get_descriptor),
    create: Some(create),
};

struct Fixture {
    root: PathBuf,
    folder: PathBuf,
    binary: PathBuf,
    cache: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "resonance-preset-discovery-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let folder = root.join("presets");
        std::fs::create_dir_all(folder.join("sub")).unwrap();
        std::fs::write(folder.join("Warm Keys.vpr"), b"x").unwrap();
        std::fs::write(folder.join("sub").join("Night.VPR"), b"x").unwrap();
        std::fs::write(folder.join("readme.txt"), b"x").unwrap();
        *FOLDER.lock().unwrap() =
            Some(std::ffi::CString::new(folder.to_string_lossy().into_owned()).unwrap());
        let binary = root.join("vendor.clap");
        std::fs::write(&binary, b"binary").unwrap();
        let cache = root.join("cache");
        Self {
            root,
            folder,
            binary,
            cache,
        }
    }

    fn discover(&self, ids: &[&str], force: bool) -> Vec<(String, Vec<resonance_audio::types::DiscoveredPreset>)> {
        let ids: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
        unsafe {
            discovery::discover(
                &FACTORY,
                &self.binary,
                &ids,
                Some(&self.cache),
                force,
                &AtomicBool::new(false),
            )
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn names(presets: &[resonance_audio::types::DiscoveredPreset]) -> Vec<&str> {
    presets.iter().map(|p| p.name.as_str()).collect()
}

/// Everything a provider says arrives; a file preset with no name, flags
/// or plugin id takes the file's stem, its location's flags and the
/// bundle's only plugin; a declaration after `init` is refused.
#[test]
fn a_provider_is_indexed_with_the_fallback_rules() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new("index");
    let found = f.discover(&["com.vendor.synth"], false);
    let (plugin, presets) = &found[0];
    assert_eq!(plugin, "com.vendor.synth");
    assert_eq!(names(presets), vec!["Deep Bass", "Glass Pad", "Warm Keys", "Night"]);
    assert_eq!(presets[0].location, DiscoveredLocation::Plugin);
    assert_eq!(presets[0].load_key.as_deref(), Some("bank0/1"));
    assert_eq!(presets[0].creators, vec!["Jane".to_string()]);
    assert_eq!(presets[0].features, vec!["bass".to_string()]);
    assert!(presets[1].is_favorite());
    assert!(matches!(&presets[2].location, DiscoveredLocation::File(p) if p.ends_with("Warm Keys.vpr")));
    assert_eq!(presets[2].flags, CLAP_PRESET_DISCOVERY_IS_USER_CONTENT, "the location's flags");
    assert!(!LATE_DECLARE_ACCEPTED.load(Ordering::SeqCst), "sealed after init");
}

/// The cache: an unchanged binary reads no unchanged file again; a new or
/// changed file is read alone; a rescan (`force`) re-indexes everything.
#[test]
fn the_cache_keeps_files_by_their_own_stamp() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new("cache");
    let first = f.discover(&["com.vendor.synth"], false);
    let (created, read) = (CREATED.load(Ordering::SeqCst), FILES_READ.load(Ordering::SeqCst));

    assert_eq!(f.discover(&["com.vendor.synth"], false), first);
    assert_eq!(CREATED.load(Ordering::SeqCst), created, "no provider for nothing new");
    assert_eq!(FILES_READ.load(Ordering::SeqCst), read);

    std::fs::write(f.folder.join("Dawn.vpr"), b"x").unwrap();
    std::fs::write(f.folder.join("Warm Keys.vpr"), b"changed").unwrap();
    let again = f.discover(&["com.vendor.synth"], false);
    assert_eq!(names(&again[0].1), vec!["Deep Bass", "Glass Pad", "Dawn", "Warm Keys", "Night"]);
    assert_eq!(FILES_READ.load(Ordering::SeqCst), read + 2, "only the new and the changed file");

    let read = FILES_READ.load(Ordering::SeqCst);
    let _ = f.discover(&["com.vendor.synth"], true);
    assert_eq!(FILES_READ.load(Ordering::SeqCst), read + 3, "a rescan reads every file");
}

/// A preset that names no plugin belongs to a bundle's only plugin, never
/// to every plugin of a bundle with several.
#[test]
fn an_unnamed_plugin_is_not_everyone() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new("plugins");
    let found = f.discover(&["com.vendor.synth", "com.vendor.fx"], false);
    assert_eq!(names(&found[0].1), vec!["Deep Bass", "Glass Pad"]);
    assert!(found[1].1.is_empty());
}
