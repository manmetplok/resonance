//! `ClapBundle` loads one `.clap` shared library and exposes its
//! plugin factory. Each bundle owns the `libloading::Library` handle and
//! keeps it resident for the lifetime of the process: dropping a bundle
//! neither calls `clap_entry.deinit()` nor unloads the binary. The `Drop`
//! impl explains why.
//!
//! Bundles are immutable after construction; everything that mutates
//! per-instance state lives on the [`super::ClapInstance`] returned by
//! [`ClapBundle::create_instance`].

use std::ffi::{c_char, CStr, CString};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::ptr;

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::entry::clap_plugin_entry;
use clap_sys::ext::audio_ports::{clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS};
use clap_sys::ext::gui::{clap_plugin_gui, CLAP_EXT_GUI};
use clap_sys::ext::latency::{clap_plugin_latency, CLAP_EXT_LATENCY};
use clap_sys::ext::params::{clap_plugin_params, CLAP_EXT_PARAMS};
use clap_sys::ext::state::{clap_plugin_state, CLAP_EXT_STATE};
use clap_sys::ext::thread_pool::{clap_plugin_thread_pool, CLAP_EXT_THREAD_POOL};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;

use clap_sys::plugin::clap_plugin;
use thiserror::Error;

use crate::types::{EngineError, EngineErrorKind, PluginDescInfo};

use super::instance::ClapInstance;
use super::{create_host_data, HostData};

/// Failure loading a `.clap` bundle or standing up one of its plugin
/// instances. Every variant here is a CLAP loading/lifecycle failure —
/// message text matches the historical `format!()` / literal strings,
/// and they all classify as [`EngineErrorKind::Plugin`].
#[derive(Debug, Error)]
pub enum ClapBundleError {
    #[error("Invalid path encoding")]
    InvalidPathEncoding,
    #[error("Invalid path: {0}")]
    InvalidPath(#[source] std::ffi::NulError),
    #[error("Failed to load library: {0}")]
    LoadLibrary(#[source] libloading::Error),
    #[error("No clap_entry symbol: {0}")]
    NoEntrySymbol(#[source] libloading::Error),
    #[error("clap_entry is null")]
    NullEntry,
    #[error("clap_entry.init is null")]
    NullInitFn,
    #[error("clap_entry.init() failed")]
    EntryInitFailed,
    #[error("clap_entry.get_factory is null")]
    NullGetFactoryFn,
    #[error("No plugin factory found")]
    NoFactory,
    #[error("factory.get_plugin_count is null")]
    NullGetCountFn,
    #[error("factory.get_plugin_descriptor is null")]
    NullGetDescFn,
    #[error("factory.create_plugin is null")]
    NullCreatePluginFn,
    #[error("Invalid plugin id: {0}")]
    InvalidPluginId(#[source] std::ffi::NulError),
    #[error("Failed to create plugin '{0}'")]
    CreatePluginFailed(String),
    #[error("plugin.init() failed")]
    PluginInitFailed,
    #[error("plugin.activate() failed")]
    PluginActivateFailed,
    #[error("plugin.start_processing() failed")]
    PluginStartProcessingFailed,
    /// Test-only path (`__instance_from_raw_for_test`): the hand-rolled
    /// build closure returned a null plugin.
    #[error("test build fn returned a null plugin")]
    TestPluginNull,
}

impl From<ClapBundleError> for EngineError {
    fn from(e: ClapBundleError) -> Self {
        EngineError::new(EngineErrorKind::Plugin, e.to_string())
    }
}

pub struct ClapBundle {
    /// The `dlopen` handle, wrapped so it is never closed — see the
    /// `Drop` impl for why the binary stays resident for the process
    /// lifetime.
    _library: std::mem::ManuallyDrop<libloading::Library>,
    entry: *const clap_plugin_entry,
    factory: *const clap_plugin_factory,
    descriptors: Vec<PluginDescInfo>,
    /// Factory presets baked into this binary, as `(name, state json)`.
    ///
    /// Read once at load time from the first-party
    /// `resonance_factory_presets` symbol. Empty for any plugin that does
    /// not export it, which is every third-party one (ba todo #1333).
    factory_presets: Vec<resonance_common::factory_presets::FactoryPresetEntry>,
    /// The `.clap` this bundle was loaded from. Held as a `CString`
    /// because the entry point's `init` borrows it, and read back by the
    /// scanner: a rescan has to know which files are ALREADY loaded so it
    /// can skip them rather than reload a library with live instances in
    /// it (ba todo #1307).
    path: CString,
}

impl ClapBundle {
    /// Load a .clap shared library file.
    pub fn load(path: &Path) -> Result<Self, ClapBundleError> {
        let path_str = path.to_str().ok_or(ClapBundleError::InvalidPathEncoding)?;
        let path_cstring = CString::new(path_str).map_err(ClapBundleError::InvalidPath)?;

        // `path` may be a macOS-style bundle directory; dlopen needs the
        // binary inside it. `clap_entry.init()` still receives the
        // original `.clap` path either way, as entry.h specifies.
        let library = unsafe { libloading::Library::new(bundle_binary_path(path)) }
            .map_err(ClapBundleError::LoadLibrary)?;

        let entry: *const clap_plugin_entry = unsafe {
            let symbol: libloading::Symbol<*const clap_plugin_entry> =
                library
                    .get(b"clap_entry")
                    .map_err(ClapBundleError::NoEntrySymbol)?;
            *symbol
        };

        if entry.is_null() {
            return Err(ClapBundleError::NullEntry);
        }

        let init_fn = unsafe { (*entry).init }.ok_or(ClapBundleError::NullInitFn)?;
        let ok = unsafe { init_fn(path_cstring.as_ptr()) };
        if !ok {
            return Err(ClapBundleError::EntryInitFailed);
        }

        let get_factory =
            unsafe { (*entry).get_factory }.ok_or(ClapBundleError::NullGetFactoryFn)?;
        let factory_ptr = unsafe { get_factory(CLAP_PLUGIN_FACTORY_ID.as_ptr()) };
        if factory_ptr.is_null() {
            return Err(ClapBundleError::NoFactory);
        }
        let factory = factory_ptr as *const clap_plugin_factory;

        let get_count =
            unsafe { (*factory).get_plugin_count }.ok_or(ClapBundleError::NullGetCountFn)?;
        let get_desc =
            unsafe { (*factory).get_plugin_descriptor }.ok_or(ClapBundleError::NullGetDescFn)?;

        let count = unsafe { get_count(factory) };
        let mut descriptors = Vec::new();
        for i in 0..count {
            let desc = unsafe { get_desc(factory, i) };
            if desc.is_null() {
                continue;
            }
            let Some((id, name, vendor)) =
                (unsafe { descriptor_strings((*desc).id, (*desc).name, (*desc).vendor) })
            else {
                // Mandatory identity fields missing — the descriptor is
                // unusable, so skip this plugin like a null descriptor.
                continue;
            };

            // Walk the null-terminated features array looking for "instrument".
            let mut is_instrument = false;
            unsafe {
                let mut feat_ptr = (*desc).features;
                if !feat_ptr.is_null() {
                    while !(*feat_ptr).is_null() {
                        if let Ok(feat) = CStr::from_ptr(*feat_ptr).to_str() {
                            if feat == "instrument" {
                                is_instrument = true;
                                break;
                            }
                        }
                        feat_ptr = feat_ptr.add(1);
                    }
                }
            }

            descriptors.push(PluginDescInfo {
                id,
                name,
                vendor,
                is_instrument,
            });
        }

        // First-party side channel. CLAP offers no way to enumerate
        // presets compiled into a binary, and this is read out of the
        // library we already have open. Absent is the normal case, not an
        // error: it just means we know nothing about this plugin's bank.
        let factory_presets = unsafe { read_factory_presets(&library) };

        Ok(ClapBundle {
            _library: std::mem::ManuallyDrop::new(library),
            entry,
            factory,
            descriptors,
            factory_presets,
            path: path_cstring,
        })
    }

    /// The path this bundle was loaded from, as the scanner canonicalized
    /// it. Empty only if the path was not valid UTF-8, which `load`
    /// already rejects.
    pub fn path(&self) -> &str {
        self.path.to_str().unwrap_or("")
    }

    pub fn descriptors(&self) -> &[PluginDescInfo] {
        &self.descriptors
    }

    /// Factory presets baked into this plugin, as `(name, state json)`.
    /// Empty for a plugin that ships none, and for every plugin that is
    /// not one of ours.
    pub fn factory_presets(&self) -> &[resonance_common::factory_presets::FactoryPresetEntry] {
        &self.factory_presets
    }

    /// Test-only doorway to [`descriptor_strings`]: the `bundle` module
    /// is private, so the helper rides on `ClapBundle` — already
    /// re-exported through `test_support` — to stay reachable from
    /// `tests/clap_host/clap_ffi_hardening.rs` without widening the module.
    ///
    /// # Safety
    /// Same contract as [`descriptor_strings`].
    #[doc(hidden)]
    pub unsafe fn __descriptor_strings_for_test(
        id: *const c_char,
        name: *const c_char,
        vendor: *const c_char,
    ) -> Option<(String, String, String)> {
        descriptor_strings(id, name, vendor)
    }

    /// Create a plugin instance from this bundle.
    pub fn create_instance(
        &self,
        plugin_id: &str,
        sample_rate: u32,
    ) -> Result<ClapInstance, ClapBundleError> {
        let create =
            unsafe { (*self.factory).create_plugin }.ok_or(ClapBundleError::NullCreatePluginFn)?;

        let host_data = create_host_data();
        let host_ptr = &host_data.clap_host as *const clap_host;

        let plugin_id_c = CString::new(plugin_id).map_err(ClapBundleError::InvalidPluginId)?;

        let plugin = unsafe { create(self.factory, host_ptr, plugin_id_c.as_ptr()) };
        if plugin.is_null() {
            return Err(ClapBundleError::CreatePluginFailed(plugin_id.to_string()));
        }

        build_instance(plugin, host_data, sample_rate)
    }
}

/// Init → query extensions → activate → query latency → start a freshly
/// created `clap_plugin` and wrap it in a [`ClapInstance`]. Shared by
/// [`ClapBundle::create_instance`] and the raw test-construction hook
/// ([`super::__instance_from_raw_for_test`]) so both go through the
/// identical lifecycle sequence.
pub(super) fn build_instance(
    plugin: *const clap_plugin,
    host_data: Pin<Box<HostData>>,
    sample_rate: u32,
) -> Result<ClapInstance, ClapBundleError> {
    host_data
        .plugin
        .store(plugin as *mut clap_plugin, std::sync::atomic::Ordering::Release);
    // Init
    if let Some(init_fn) = unsafe { (*plugin).init } {
        let ok = unsafe { init_fn(plugin) };
        if !ok {
            if let Some(destroy) = unsafe { (*plugin).destroy } {
                unsafe { destroy(plugin) };
            }
            return Err(ClapBundleError::PluginInitFailed);
        }
    }

    // Query extensions before activation
    // CLAP `thread-pool`: the plugin's task entry point, for the host's
    // `request_exec` (`clap_host::thread_pool`).
    unsafe {
        if let Some(get_ext) = (*plugin).get_extension {
            let ext = get_ext(plugin, CLAP_EXT_THREAD_POOL.as_ptr())
                as *const clap_plugin_thread_pool;
            if let Some(exec) = ext.as_ref().and_then(|ext| ext.exec) {
                let _ = host_data.thread_pool_exec.set(exec);
            }
        }
    }
    let params_ext = unsafe {
        if let Some(get_ext) = (*plugin).get_extension {
            let ext = get_ext(plugin, CLAP_EXT_PARAMS.as_ptr());
            if ext.is_null() {
                None
            } else {
                Some(ext as *const clap_plugin_params)
            }
        } else {
            None
        }
    };

    let state_ext = unsafe {
        if let Some(get_ext) = (*plugin).get_extension {
            let ext = get_ext(plugin, CLAP_EXT_STATE.as_ptr());
            if ext.is_null() {
                None
            } else {
                Some(ext as *const clap_plugin_state)
            }
        } else {
            None
        }
    };

    let gui_ext = unsafe {
        if let Some(get_ext) = (*plugin).get_extension {
            let ext = get_ext(plugin, CLAP_EXT_GUI.as_ptr());
            if ext.is_null() {
                None
            } else {
                Some(ext as *const clap_plugin_gui)
            }
        } else {
            None
        }
    };

    // Query the audio-ports extension to learn how many output ports
    // this plugin declares. Defaults to 1 (single stereo) if the
    // extension is absent — matches CLAP host fallback behaviour and
    // keeps pre-multi-output plugins working unchanged.
    let audio_ports_ext = unsafe {
        if let Some(get_ext) = (*plugin).get_extension {
            let ext = get_ext(plugin, CLAP_EXT_AUDIO_PORTS.as_ptr());
            if ext.is_null() {
                None
            } else {
                Some(ext as *const clap_plugin_audio_ports)
            }
        } else {
            None
        }
    };
    let output_port_count = unsafe {
        match audio_ports_ext.and_then(|ports| (*ports).count) {
            Some(count_fn) => (count_fn(plugin, false) as usize).max(1),
            None => 1,
        }
    };

    // Input ports, same query with `is_input = true`. A second port is
    // how a plugin declares an external sidechain key (resonance-plugin's
    // `SIDECHAIN_INPUT`); the host connects it only when a routing has
    // been configured for that instance. Effects default to 1 and
    // instruments to 0, matching what the extension would report.
    let input_port_count = unsafe {
        match audio_ports_ext.and_then(|ports| (*ports).count) {
            Some(count_fn) => count_fn(plugin, true) as usize,
            None => 1,
        }
    };

    let latency_ext = unsafe {
        if let Some(get_ext) = (*plugin).get_extension {
            let ext = get_ext(plugin, CLAP_EXT_LATENCY.as_ptr());
            if ext.is_null() {
                None
            } else {
                Some(ext as *const clap_plugin_latency)
            }
        } else {
            None
        }
    };

    // Activate
    if let Some(activate) = unsafe { (*plugin).activate } {
        let ok = unsafe {
            activate(
                plugin,
                sample_rate as f64,
                super::ACTIVATE_MIN_FRAMES,
                super::ACTIVATE_MAX_FRAMES,
            )
        };
        if !ok {
            if let Some(destroy) = unsafe { (*plugin).destroy } {
                unsafe { destroy(plugin) };
            }
            return Err(ClapBundleError::PluginActivateFailed);
        }
    }

    // Query the latency extension now that the plugin is activated
    // (the CLAP spec only defines `latency.get()` while active).
    // Latency changes after this point are tracked: the plugin
    // signals `clap_host_latency.changed()` / `request_restart()`
    // and the engine cycles activation + re-queries at the next
    // safe point (doc #260 finding #10).
    let latency = unsafe {
        match latency_ext.and_then(|ext| (*ext).get) {
            Some(get_fn) => get_fn(plugin),
            None => 0,
        }
    };
    // A `changed()` fired during activation is already captured by
    // the query above; clear the flag so it doesn't schedule a
    // redundant restart cycle.
    host_data
        .latency_changed
        .store(false, std::sync::atomic::Ordering::Release);

    // Start processing
    if let Some(start) = unsafe { (*plugin).start_processing } {
        // `[audio-thread]` in CLAP; see `AudioThreadScope`.
        let _audio = super::thread_check::AudioThreadScope::enter();
        let ok = unsafe { start(plugin) };
        if !ok {
            if let Some(deactivate) = unsafe { (*plugin).deactivate } {
                unsafe { deactivate(plugin) };
            }
            if let Some(destroy) = unsafe { (*plugin).destroy } {
                unsafe { destroy(plugin) };
            }
            return Err(ClapBundleError::PluginStartProcessingFailed);
        }
    }

    // Pre-allocate the audio-output buffer array once per plugin
    // instance. process_multi refreshes the data32 pointers each block
    // without ever allocating.
    let audio_out_ptrs = vec![[ptr::null_mut(); 2]; output_port_count];
    let audio_out_buffers = (0..output_port_count)
        .map(|_| clap_audio_buffer {
            data32: ptr::null_mut(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        })
        .collect();

    Ok(ClapInstance::from_parts(
        plugin,
        host_data,
        sample_rate,
        params_ext,
        state_ext,
        audio_ports_ext,
        gui_ext,
        latency_ext,
        output_port_count,
        input_port_count,
        latency,
        audio_out_buffers,
        audio_out_ptrs,
    ))
}

impl Drop for ClapBundle {
    fn drop(&mut self) {
        // Deliberately does NOT call `clap_entry.deinit()` and does NOT
        // `dlclose` the library: a loaded plugin binary stays resident for
        // the lifetime of the process.
        //
        // We used to do both, and it aborted the process. Two independent
        // core dumps (`relink_modal`, `timeline_automation_lane_rows`,
        // 2026-08-17) show the same stack: engine thread shuts down ->
        // `drop_in_place::<Vec<ClapBundle>>` -> `deinit()` inside
        // `master_me.clap` -> glibc "corrupted size vs. prev_size" ->
        // `abort()`. That plugin (DPF-based, in `/usr/lib/clap`) frees
        // state its own teardown has already released, and we cannot fix
        // third-party binaries. It presented as a rare "flaky SIGABRT" in
        // UI tests only because every test process scans and loads the
        // machine's real plugin directories.
        //
        // Leaking is what hosts do here, and not only to dodge one broken
        // plugin: unloading a plugin binary mid-process is unsound in
        // general. A `.clap` may have registered `atexit` handlers, TLS
        // destructors, or background threads whose code lives in the very
        // pages `dlclose` unmaps, and any of those turns into a jump into
        // freed memory later. The cost is bounded and small — one resident
        // library per distinct `.clap` file, reclaimed by the OS at exit.
        //
        // Re-scanning re-`dlopen`s an already-resident library, which just
        // bumps its refcount and re-runs `init()`. The CLAP entry contract
        // makes `init`/`deinit` refcounted and explicitly allows repeated
        // `init()`, so an extra `init()` with no matching `deinit()` leaves
        // the bundle initialised — exactly the state we want it in.
        let _ = self.entry;
    }
}

/// The file to actually `dlopen` for a `.clap` path. A plain file (the
/// Linux layout) is the shared object itself. A directory is a
/// macOS-style bundle whose binary lives at `Contents/MacOS/<name>`,
/// where `<name>` comes from Info.plist's `CFBundleExecutable` when
/// present and the bundle's file stem otherwise (the two match for
/// every conventionally packaged plugin).
pub fn bundle_binary_path(path: &Path) -> PathBuf {
    if !path.is_dir() {
        return path.to_path_buf();
    }
    let contents = path.join("Contents");
    let name = std::fs::read_to_string(contents.join("Info.plist"))
        .ok()
        .and_then(|plist| plist_executable(&plist))
        .or_else(|| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        });
    match name {
        Some(name) => contents.join("MacOS").join(name),
        // No stem means a bare path like "/"; hand it to dlopen
        // unchanged and let it produce the error.
        None => path.to_path_buf(),
    }
}

/// Minimal `CFBundleExecutable` extraction from an XML Info.plist: the
/// first `<string>` after the key. Deliberately not a plist parser — a
/// wrong or absent answer only means falling back to the file stem.
fn plist_executable(plist: &str) -> Option<String> {
    let rest = &plist[plist.find("<key>CFBundleExecutable</key>")?..];
    let start = rest.find("<string>")? + "<string>".len();
    let end = rest[start..].find("</string>")? + start;
    let name = rest[start..end].trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Copy the identity strings out of a plugin descriptor's raw fields.
///
/// plugin.h marks `id` and `name` as mandatory, but the remaining
/// descriptor fields are optional and CLAP represents an absent one as
/// a null pointer — `vendor` is null in the wild often enough that
/// feeding it to `CStr::from_ptr` unchecked is a real crash, not a
/// theoretical one. A descriptor missing a mandatory field is unusable:
/// `None`, and the scanner skips that plugin (mirroring the
/// null-descriptor skip and the features walk, which already
/// null-check). A null `vendor` becomes the empty string.
///
/// # Safety
/// Every non-null pointer must reference a NUL-terminated string that
/// stays valid for the duration of the call — the factory contract for
/// descriptor fields.
unsafe fn descriptor_strings(
    id: *const c_char,
    name: *const c_char,
    vendor: *const c_char,
) -> Option<(String, String, String)> {
    if id.is_null() || name.is_null() {
        return None;
    }
    let id = CStr::from_ptr(id).to_string_lossy().to_string();
    let name = CStr::from_ptr(name).to_string_lossy().to_string();
    let vendor = if vendor.is_null() {
        String::new()
    } else {
        CStr::from_ptr(vendor).to_string_lossy().to_string()
    };
    Some((id, name, vendor))
}

/// Read the first-party factory-preset bank out of a loaded plugin
/// library, if it exports one.
///
/// # Safety
/// `library` must be a loaded plugin binary. The symbol, when present, is
/// contracted to return either null or a pointer valid for the lifetime of
/// the process (see `resonance_plugin::export_clap!`); the string is
/// copied out here and never freed by us.
unsafe fn read_factory_presets(
    library: &libloading::Library,
) -> Vec<resonance_common::factory_presets::FactoryPresetEntry> {
    type Getter = unsafe extern "C" fn() -> *const std::os::raw::c_char;
    let symbol: libloading::Symbol<Getter> =
        match library.get(resonance_common::factory_presets::FACTORY_PRESETS_SYMBOL) {
            Ok(symbol) => symbol,
            // Not one of ours, or one of ours with no factory bank.
            Err(_) => return Vec::new(),
        };
    let raw = symbol();
    if raw.is_null() {
        return Vec::new();
    }
    match CStr::from_ptr(raw).to_str() {
        Ok(text) => resonance_common::factory_presets::decode_entries(text),
        Err(_) => Vec::new(),
    }
}
