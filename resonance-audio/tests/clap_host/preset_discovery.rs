//! The `clap.preset-discovery-factory` indexer (plugin-preset-library.md
//! §8 tier T1, slice P8) against a hand-rolled provider: a `PLUGIN`
//! location with two presets, and a `FILE` directory walked for the
//! declared extension; then the per-plugin cache, keyed by the binary.
//!
//! Everything writes under a private temp directory.

use std::ffi::{c_char, c_void, CStr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use clap_sys::factory::preset_discovery::*;
use clap_sys::universal_plugin_id::clap_universal_plugin_id;
use clap_sys::version::CLAP_VERSION;
use resonance_audio::test_support::discovery::{self, DiscoveredLocation};

const PLUGIN: &CStr = c"com.vendor.synth";

/// The folder the fake provider declares; set per test run.
static FOLDER: std::sync::Mutex<Option<std::ffi::CString>> = std::sync::Mutex::new(None);
/// How many times a provider was created (the cache must avoid it).
static CREATED: AtomicU32 = AtomicU32::new(0);

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

unsafe extern "C" fn init(provider: *const clap_preset_discovery_provider) -> bool {
    unsafe {
        let b = &*((*provider).provider_data as *const ProviderBox);
        let indexer = b.indexer;
        let declare_filetype = (*indexer).declare_filetype.unwrap();
        let declare_location = (*indexer).declare_location.unwrap();
        declare_filetype(
            indexer,
            &clap_preset_discovery_filetype {
                name: c"Vendor preset".as_ptr(),
                description: std::ptr::null(),
                file_extension: c"vpr".as_ptr(),
            },
        );
        declare_location(
            indexer,
            &clap_preset_discovery_location {
                flags: CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT,
                name: c"Factory".as_ptr(),
                kind: CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN,
                location: std::ptr::null(),
            },
        );
        let folder = FOLDER.lock().unwrap().clone().unwrap();
        declare_location(
            indexer,
            &clap_preset_discovery_location {
                flags: CLAP_PRESET_DISCOVERY_IS_USER_CONTENT,
                name: c"User".as_ptr(),
                kind: CLAP_PRESET_DISCOVERY_LOCATION_FILE,
                location: folder.as_ptr(),
            },
        );
    }
    true
}

unsafe extern "C" fn destroy(provider: *const clap_preset_discovery_provider) {
    unsafe {
        drop(Box::from_raw((*provider).provider_data as *mut ProviderBox));
    }
}

unsafe fn preset(
    r: *const clap_preset_discovery_metadata_receiver,
    name: &CStr,
    key: Option<&CStr>,
    flags: u32,
) {
    unsafe {
        (*r).begin_preset.unwrap()(r, name.as_ptr(), key.map_or(std::ptr::null(), |k| k.as_ptr()));
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
    _provider: *const clap_preset_discovery_provider,
    kind: u32,
    location: *const c_char,
    r: *const clap_preset_discovery_metadata_receiver,
) -> bool {
    unsafe {
        if kind == CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN {
            preset(r, c"Deep Bass", Some(c"bank0/1"), CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT);
            preset(
                r,
                c"Glass Pad",
                Some(c"bank0/2"),
                CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT | CLAP_PRESET_DISCOVERY_IS_FAVORITE,
            );
        } else {
            let path = CStr::from_ptr(location).to_string_lossy();
            let stem = std::path::Path::new(&*path)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let name = std::ffi::CString::new(stem).unwrap();
            preset(r, &name, None, CLAP_PRESET_DISCOVERY_IS_USER_CONTENT);
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

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-preset-discovery-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Everything a provider says arrives, the folder is walked for the
/// declared extension only, and a second discovery of the same binary is
/// served from the cache without creating a provider.
#[test]
fn a_provider_is_indexed_and_the_result_cached_by_binary() {
    let root = temp("index");
    let folder = root.join("presets");
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    std::fs::write(folder.join("Warm Keys.vpr"), b"x").unwrap();
    std::fs::write(folder.join("sub").join("Night.VPR"), b"x").unwrap();
    std::fs::write(folder.join("readme.txt"), b"x").unwrap();
    *FOLDER.lock().unwrap() =
        Some(std::ffi::CString::new(folder.to_string_lossy().into_owned()).unwrap());
    let binary = root.join("vendor.clap");
    std::fs::write(&binary, b"binary").unwrap();
    let cache = root.join("library");
    let ids = vec![PLUGIN.to_string_lossy().into_owned()];

    let before = CREATED.load(Ordering::SeqCst);
    let found = unsafe { discovery::discover(&FACTORY, &binary, &ids, Some(&cache)) };
    assert_eq!(CREATED.load(Ordering::SeqCst), before + 1);
    let (plugin, presets) = &found[0];
    assert_eq!(plugin, "com.vendor.synth");
    let names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["Deep Bass", "Glass Pad", "Warm Keys", "Night"]);
    assert_eq!(presets[0].location, DiscoveredLocation::Plugin);
    assert_eq!(presets[0].load_key.as_deref(), Some("bank0/1"));
    assert_eq!(presets[0].creators, vec!["Jane".to_string()]);
    assert_eq!(presets[0].features, vec!["bass".to_string()]);
    assert!(presets[1].is_favorite());
    assert!(matches!(&presets[2].location, DiscoveredLocation::File(p) if p.ends_with("Warm Keys.vpr")));
    assert!(cache.join("discovered").join("com.vendor.synth.json").exists());

    let again = unsafe { discovery::discover(&FACTORY, &binary, &ids, Some(&cache)) };
    assert_eq!(again, found);
    assert_eq!(CREATED.load(Ordering::SeqCst), before + 1, "served from the cache");

    // A new binary (size changed) is indexed again.
    std::fs::write(&binary, b"binary v2").unwrap();
    let _ = unsafe { discovery::discover(&FACTORY, &binary, &ids, Some(&cache)) };
    assert_eq!(CREATED.load(Ordering::SeqCst), before + 2);
    let _ = std::fs::remove_dir_all(&root);
}
