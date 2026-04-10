//! Minimal CLAP plugin host for loading and running .clap plugins.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;
use std::pin::Pin;
use std::ptr;

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::entry::clap_plugin_entry;
use clap_sys::ext::audio_ports::{
    clap_audio_port_info, clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS,
};
use clap_sys::events::{
    clap_event_header, clap_event_note, clap_event_param_value, clap_input_events,
    clap_output_events, CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_NOTE_OFF, CLAP_EVENT_NOTE_ON,
    CLAP_EVENT_PARAM_VALUE,
};
use clap_sys::ext::gui::{
    clap_plugin_gui, CLAP_EXT_GUI, CLAP_WINDOW_API_WAYLAND,
};
use clap_sys::ext::params::{clap_plugin_params, CLAP_EXT_PARAMS};
use clap_sys::ext::state::{clap_plugin_state, CLAP_EXT_STATE};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::stream::{clap_istream, clap_ostream};
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::clap_process;
use clap_sys::version::CLAP_VERSION;

use crate::types::{ParamInfo, PluginDescInfo};

// ---------------------------------------------------------------------------
// Host callbacks (minimal no-op implementation)
// ---------------------------------------------------------------------------

struct HostData {
    clap_host: clap_host,
}

unsafe extern "C" fn host_get_extension(
    _host: *const clap_host,
    _extension_id: *const c_char,
) -> *const c_void {
    ptr::null()
}

unsafe extern "C" fn host_request_restart(_host: *const clap_host) {}
unsafe extern "C" fn host_request_process(_host: *const clap_host) {}
unsafe extern "C" fn host_request_callback(_host: *const clap_host) {}

fn create_host_data() -> Pin<Box<HostData>> {
    let mut host_data = Box::pin(HostData {
        clap_host: clap_host {
            clap_version: CLAP_VERSION,
            host_data: ptr::null_mut(),
            name: b"Resonance\0".as_ptr() as *const c_char,
            vendor: b"Resonance\0".as_ptr() as *const c_char,
            url: b"\0".as_ptr() as *const c_char,
            version: b"0.1.0\0".as_ptr() as *const c_char,
            get_extension: Some(host_get_extension),
            request_restart: Some(host_request_restart),
            request_process: Some(host_request_process),
            request_callback: Some(host_request_callback),
        },
    });
    let ptr = &*host_data as *const HostData as *mut c_void;
    unsafe {
        let host_data_mut = Pin::get_unchecked_mut(host_data.as_mut());
        host_data_mut.clap_host.host_data = ptr;
    }
    host_data
}

// ---------------------------------------------------------------------------
// Event list for parameter changes + note events
// ---------------------------------------------------------------------------

/// Context for input events carrying both param value and note events.
/// Param events (time=0) come first, then note events sorted by time.
struct MixedEventListCtx {
    param_events: Vec<clap_event_param_value>,
    note_events: Vec<clap_event_note>,
}

unsafe extern "C" fn mixed_events_size(list: *const clap_input_events) -> u32 {
    let ctx = &*((*list).ctx as *const MixedEventListCtx);
    (ctx.param_events.len() + ctx.note_events.len()) as u32
}

unsafe extern "C" fn mixed_events_get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    let ctx = &*((*list).ctx as *const MixedEventListCtx);
    let param_count = ctx.param_events.len();
    let idx = index as usize;
    if idx < param_count {
        &ctx.param_events[idx].header as *const clap_event_header
    } else {
        let note_idx = idx - param_count;
        if note_idx < ctx.note_events.len() {
            &ctx.note_events[note_idx].header as *const clap_event_header
        } else {
            ptr::null()
        }
    }
}

unsafe extern "C" fn discard_output_event(
    _list: *const clap_output_events,
    _event: *const clap_event_header,
) -> bool {
    true
}

// ---------------------------------------------------------------------------
// ClapBundle — loads a .clap shared library and provides the factory
// ---------------------------------------------------------------------------

pub struct ClapBundle {
    _library: libloading::Library,
    entry: *const clap_plugin_entry,
    factory: *const clap_plugin_factory,
    descriptors: Vec<PluginDescInfo>,
    _path: CString,
}

impl ClapBundle {
    /// Load a .clap shared library file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let path_str = path
            .to_str()
            .ok_or_else(|| "Invalid path encoding".to_string())?;
        let path_cstring =
            CString::new(path_str).map_err(|e| format!("Invalid path: {}", e))?;

        let library = unsafe { libloading::Library::new(path) }
            .map_err(|e| format!("Failed to load library: {}", e))?;

        let entry: *const clap_plugin_entry = unsafe {
            let symbol: libloading::Symbol<*const clap_plugin_entry> = library
                .get(b"clap_entry")
                .map_err(|e| format!("No clap_entry symbol: {}", e))?;
            *symbol
        };

        if entry.is_null() {
            return Err("clap_entry is null".to_string());
        }

        let init_fn = unsafe { (*entry).init }
            .ok_or_else(|| "clap_entry.init is null".to_string())?;
        let ok = unsafe { init_fn(path_cstring.as_ptr()) };
        if !ok {
            return Err("clap_entry.init() failed".to_string());
        }

        let get_factory = unsafe { (*entry).get_factory }
            .ok_or_else(|| "clap_entry.get_factory is null".to_string())?;
        let factory_ptr = unsafe { get_factory(CLAP_PLUGIN_FACTORY_ID.as_ptr()) };
        if factory_ptr.is_null() {
            return Err("No plugin factory found".to_string());
        }
        let factory = factory_ptr as *const clap_plugin_factory;

        let get_count = unsafe { (*factory).get_plugin_count }
            .ok_or_else(|| "factory.get_plugin_count is null".to_string())?;
        let get_desc = unsafe { (*factory).get_plugin_descriptor }
            .ok_or_else(|| "factory.get_plugin_descriptor is null".to_string())?;

        let count = unsafe { get_count(factory) };
        let mut descriptors = Vec::new();
        for i in 0..count {
            let desc = unsafe { get_desc(factory, i) };
            if desc.is_null() {
                continue;
            }
            let id = unsafe { CStr::from_ptr((*desc).id) }
                .to_string_lossy()
                .to_string();
            let name = unsafe { CStr::from_ptr((*desc).name) }
                .to_string_lossy()
                .to_string();
            let vendor = unsafe { CStr::from_ptr((*desc).vendor) }
                .to_string_lossy()
                .to_string();

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

            descriptors.push(PluginDescInfo { id, name, vendor, is_instrument });
        }

        Ok(ClapBundle {
            _library: library,
            entry,
            factory,
            descriptors,
            _path: path_cstring,
        })
    }

    pub fn descriptors(&self) -> &[PluginDescInfo] {
        &self.descriptors
    }

    /// Create a plugin instance from this bundle.
    pub fn create_instance(
        &self,
        plugin_id: &str,
        sample_rate: u32,
    ) -> Result<ClapInstance, String> {
        let create = unsafe { (*self.factory).create_plugin }
            .ok_or_else(|| "factory.create_plugin is null".to_string())?;

        let host_data = create_host_data();
        let host_ptr = &host_data.clap_host as *const clap_host;

        let plugin_id_c =
            CString::new(plugin_id).map_err(|e| format!("Invalid plugin id: {}", e))?;

        let plugin = unsafe { create(self.factory, host_ptr, plugin_id_c.as_ptr()) };
        if plugin.is_null() {
            return Err(format!("Failed to create plugin '{}'", plugin_id));
        }

        // Init
        if let Some(init_fn) = unsafe { (*plugin).init } {
            let ok = unsafe { init_fn(plugin) };
            if !ok {
                if let Some(destroy) = unsafe { (*plugin).destroy } {
                    unsafe { destroy(plugin) };
                }
                return Err("plugin.init() failed".to_string());
            }
        }

        // Query extensions before activation
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

        // Activate
        if let Some(activate) = unsafe { (*plugin).activate } {
            let ok = unsafe { activate(plugin, sample_rate as f64, 32, 8192) };
            if !ok {
                if let Some(destroy) = unsafe { (*plugin).destroy } {
                    unsafe { destroy(plugin) };
                }
                return Err("plugin.activate() failed".to_string());
            }
        }

        // Start processing
        if let Some(start) = unsafe { (*plugin).start_processing } {
            let ok = unsafe { start(plugin) };
            if !ok {
                if let Some(deactivate) = unsafe { (*plugin).deactivate } {
                    unsafe { deactivate(plugin) };
                }
                if let Some(destroy) = unsafe { (*plugin).destroy } {
                    unsafe { destroy(plugin) };
                }
                return Err("plugin.start_processing() failed".to_string());
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

        Ok(ClapInstance {
            plugin,
            _host_data: host_data,
            active: true,
            sample_rate,
            params_ext,
            state_ext,
            audio_ports_ext,
            gui_ext,
            gui_open: false,
            output_port_count,
            pending_params: Vec::new(),
            param_event_buf: Vec::new(),
            pending_notes: Vec::new(),
            note_event_buf: Vec::new(),
            audio_out_buffers,
            audio_out_ptrs,
        })
    }
}

impl Drop for ClapBundle {
    fn drop(&mut self) {
        if let Some(deinit) = unsafe { (*self.entry).deinit } {
            unsafe { deinit() };
        }
    }
}

// ---------------------------------------------------------------------------
// ClapInstance — a running plugin instance
// ---------------------------------------------------------------------------

/// Mutable reference to one stereo output port's buffer. Used by
/// [`ClapInstance::process_multi`] to drive plugins that declare more than
/// one output port (e.g. `resonance-drums` with its per-group outputs).
/// For regular single-output plugins use the shorter [`ClapInstance::process`]
/// convenience wrapper instead.
pub struct StereoBufMut<'a> {
    pub left: &'a mut [f32],
    pub right: &'a mut [f32],
}

pub struct ClapInstance {
    plugin: *const clap_plugin,
    _host_data: Pin<Box<HostData>>,
    active: bool,
    sample_rate: u32,
    params_ext: Option<*const clap_plugin_params>,
    state_ext: Option<*const clap_plugin_state>,
    audio_ports_ext: Option<*const clap_plugin_audio_ports>,
    gui_ext: Option<*const clap_plugin_gui>,
    /// True when `gui_create` has been called and `gui_destroy` hasn't yet.
    gui_open: bool,
    /// Number of output audio ports as reported by the plugin's audio-ports
    /// extension at activation time. Cached so the mixer can size its
    /// per-port scratch buffers without re-querying on every block.
    /// Always >= 1 because resonance-plugin rejects empty output layouts.
    output_port_count: usize,
    /// Pending parameter changes to send during next process() call.
    pending_params: Vec<(u32, f64)>,
    /// Pre-allocated buffer for CLAP parameter events (reused across process() calls).
    param_event_buf: Vec<clap_event_param_value>,
    /// Pending note events to send during next process() call.
    /// Each entry: (is_note_on, key, velocity, sample_offset)
    pending_notes: Vec<(bool, u8, f32, u32)>,
    /// Pre-allocated buffer for CLAP note events.
    note_event_buf: Vec<clap_event_note>,
    /// Pre-allocated scratch for the CLAP audio output buffer array,
    /// one entry per output port. Reused across every `process_multi`
    /// call so the audio thread never allocates.
    audio_out_buffers: Vec<clap_audio_buffer>,
    /// Per-port channel pointer array (2 pointers per port). Each block's
    /// `process_multi` call refreshes these to point at the caller's
    /// supplied slices before handing them to CLAP.
    audio_out_ptrs: Vec<[*mut f32; 2]>,
}

impl ClapInstance {
    /// Number of output audio ports this plugin declares. Stable for the
    /// lifetime of the instance — use this to size per-port scratch
    /// buffers and (in the app layer) auto-create sub-tracks. Always >= 1.
    pub fn output_port_count(&self) -> usize {
        self.output_port_count
    }

    /// Human-readable name of each output port, as reported by the plugin's
    /// `clap.audio-ports` extension at activation time. Used by the app
    /// layer to name auto-created sub-tracks after their source port
    /// (e.g. "Kick", "Snare", "Overhead"). Falls back to "Out N" if the
    /// plugin doesn't implement the extension or returns empty names.
    pub fn output_port_names(&self) -> Vec<String> {
        let mut names = Vec::with_capacity(self.output_port_count);
        let ports_ext = self.audio_ports_ext;
        unsafe {
            let get_fn = ports_ext.and_then(|ports| (*ports).get);
            for i in 0..self.output_port_count {
                let name = get_fn.and_then(|f| {
                    let mut info = std::mem::MaybeUninit::<clap_audio_port_info>::zeroed();
                    let ok = f(self.plugin, i as u32, false, info.as_mut_ptr());
                    if !ok {
                        return None;
                    }
                    let info = info.assume_init();
                    let cstr = CStr::from_ptr(info.name.as_ptr());
                    let s = cstr.to_string_lossy().into_owned();
                    if s.is_empty() {
                        None
                    } else {
                        Some(s)
                    }
                });
                names.push(name.unwrap_or_else(|| format!("Out {}", i + 1)));
            }
        }
        names
    }

    /// Query all parameters from the plugin. Called from the engine thread.
    pub fn query_params(&self) -> Vec<ParamInfo> {
        let params = match self.params_ext {
            Some(p) => p,
            None => return Vec::new(),
        };

        let count = unsafe {
            match (*params).count {
                Some(f) => f(self.plugin),
                None => return Vec::new(),
            }
        };

        let mut result = Vec::with_capacity(count as usize);

        for i in 0..count {
            let mut info = std::mem::MaybeUninit::<clap_sys::ext::params::clap_param_info>::uninit();
            let ok = unsafe {
                match (*params).get_info {
                    Some(f) => f(self.plugin, i, info.as_mut_ptr()),
                    None => continue,
                }
            };
            if !ok {
                continue;
            }
            let info = unsafe { info.assume_init() };

            // Get current value
            let mut current = info.default_value;
            if let Some(get_value) = unsafe { (*params).get_value } {
                unsafe { get_value(self.plugin, info.id, &mut current) };
            }

            // Convert name from c_char array
            let name = unsafe {
                CStr::from_ptr(info.name.as_ptr())
                    .to_string_lossy()
                    .to_string()
            };

            // Skip hidden params
            if info.flags & clap_sys::ext::params::CLAP_PARAM_IS_HIDDEN != 0 {
                continue;
            }

            result.push(ParamInfo {
                id: info.id,
                name,
                min_value: info.min_value,
                max_value: info.max_value,
                default_value: info.default_value,
                current_value: current,
            });
        }

        result
    }

    /// Queue a parameter change to be sent during the next process() call.
    pub fn set_param(&mut self, param_id: u32, value: f64) {
        self.pending_params.push((param_id, value));
    }

    /// Queue a note-on event to be sent during the next process() call.
    pub fn queue_note_on(&mut self, key: u8, velocity: f32, sample_offset: u32) {
        self.pending_notes.push((true, key, velocity, sample_offset));
    }

    /// Queue a note-off event to be sent during the next process() call.
    pub fn queue_note_off(&mut self, key: u8, sample_offset: u32) {
        self.pending_notes.push((false, key, 0.0, sample_offset));
    }

    /// Send note-off for all 128 MIDI notes (to clear stuck notes).
    // --- GUI (CLAP_EXT_GUI) ------------------------------------------------

    /// Whether the plugin exposes a GUI that the host can open.
    pub fn has_gui(&self) -> bool {
        self.gui_ext.is_some()
    }

    /// Whether the GUI is currently open (i.e. `gui_create` was called).
    pub fn is_gui_open(&self) -> bool {
        self.gui_open
    }

    /// Open the plugin's editor window as a floating Wayland window.
    ///
    /// Walks the full CLAP GUI negotiation sequence:
    /// `is_api_supported` → `create` → `get_size` → `show`. Returns `false`
    /// at any step failure. If the GUI is already open, this is a no-op.
    pub fn open_gui(&mut self) -> bool {
        let Some(gui) = self.gui_ext else {
            return false;
        };
        if self.gui_open {
            return true;
        }
        unsafe {
            let Some(is_supported) = (*gui).is_api_supported else {
                return false;
            };
            if !is_supported(self.plugin, CLAP_WINDOW_API_WAYLAND.as_ptr(), true) {
                return false;
            }
            let Some(create) = (*gui).create else {
                return false;
            };
            if !create(self.plugin, CLAP_WINDOW_API_WAYLAND.as_ptr(), true) {
                return false;
            }
            // Best-effort size negotiation (ignore errors — the plugin has
            // its own preferred size baked into its factory).
            if let Some(get_size) = (*gui).get_size {
                let mut w: u32 = 0;
                let mut h: u32 = 0;
                get_size(self.plugin, &mut w, &mut h);
                if let Some(set_size) = (*gui).set_size {
                    if w > 0 && h > 0 {
                        set_size(self.plugin, w, h);
                    }
                }
            }
            let Some(show) = (*gui).show else {
                // If show isn't exposed, roll back the create.
                if let Some(destroy) = (*gui).destroy {
                    destroy(self.plugin);
                }
                return false;
            };
            if !show(self.plugin) {
                if let Some(destroy) = (*gui).destroy {
                    destroy(self.plugin);
                }
                return false;
            }
        }
        self.gui_open = true;
        true
    }

    /// Close the plugin's editor window (hide + destroy).
    pub fn close_gui(&mut self) {
        if !self.gui_open {
            return;
        }
        if let Some(gui) = self.gui_ext {
            unsafe {
                if let Some(hide) = (*gui).hide {
                    hide(self.plugin);
                }
                if let Some(destroy) = (*gui).destroy {
                    destroy(self.plugin);
                }
            }
        }
        self.gui_open = false;
    }

    pub fn all_notes_off(&mut self) {
        for key in 0..128u8 {
            self.pending_notes.push((false, key, 0.0, 0));
        }
    }

    /// Save the plugin's full state (params + persisted fields) via CLAP state extension.
    pub fn save_state(&self) -> Option<Vec<u8>> {
        let state_ext = self.state_ext?;
        let save_fn = unsafe { (*state_ext).save }?;

        let mut buf: Vec<u8> = Vec::new();

        unsafe extern "C" fn ostream_write(
            stream: *const clap_ostream,
            buffer: *const c_void,
            size: u64,
        ) -> i64 {
            let buf = &mut *((*stream).ctx as *mut Vec<u8>);
            let slice = std::slice::from_raw_parts(buffer as *const u8, size as usize);
            buf.extend_from_slice(slice);
            size as i64
        }

        let mut stream = clap_ostream {
            ctx: &mut buf as *mut Vec<u8> as *mut c_void,
            write: Some(ostream_write),
        };

        let ok = unsafe { save_fn(self.plugin, &mut stream) };
        if ok {
            Some(buf)
        } else {
            None
        }
    }

    /// Load plugin state from a byte buffer via CLAP state extension.
    pub fn load_state(&mut self, data: &[u8]) -> bool {
        let state_ext = match self.state_ext {
            Some(ext) => ext,
            None => return false,
        };
        let load_fn = match unsafe { (*state_ext).load } {
            Some(f) => f,
            None => return false,
        };

        struct IstreamCtx {
            data: *const u8,
            len: usize,
            pos: usize,
        }

        unsafe extern "C" fn istream_read(
            stream: *const clap_istream,
            buffer: *mut c_void,
            size: u64,
        ) -> i64 {
            let ctx = &mut *((*stream).ctx as *mut IstreamCtx);
            let remaining = ctx.len - ctx.pos;
            let to_read = (size as usize).min(remaining);
            if to_read == 0 {
                return 0;
            }
            std::ptr::copy_nonoverlapping(ctx.data.add(ctx.pos), buffer as *mut u8, to_read);
            ctx.pos += to_read;
            to_read as i64
        }

        let mut ctx = IstreamCtx {
            data: data.as_ptr(),
            len: data.len(),
            pos: 0,
        };

        let mut stream = clap_istream {
            ctx: &mut ctx as *mut IstreamCtx as *mut c_void,
            read: Some(istream_read),
        };

        unsafe { load_fn(self.plugin, &mut stream) }
    }

    /// Load state with full lifecycle cycle: stop → deactivate → load → activate → start.
    /// This ensures `initialize()` runs again so the plugin picks up new persist fields.
    pub fn reload_with_state(&mut self, data: &[u8]) -> bool {
        if !self.active {
            return self.load_state(data);
        }

        // Stop processing
        if let Some(stop) = unsafe { (*self.plugin).stop_processing } {
            unsafe { stop(self.plugin) };
        }
        // Deactivate
        if let Some(deactivate) = unsafe { (*self.plugin).deactivate } {
            unsafe { deactivate(self.plugin) };
        }

        self.active = false;

        // Load state
        let ok = self.load_state(data);
        if !ok {
            return false;
        }

        // Reactivate
        if let Some(activate) = unsafe { (*self.plugin).activate } {
            let ok = unsafe { activate(self.plugin, self.sample_rate as f64, 32, 8192) };
            if !ok {
                return false;
            }
        }
        // Mark active immediately after successful activate,
        // so Drop will properly deactivate even if start_processing fails
        self.active = true;

        // Start processing
        if let Some(start) = unsafe { (*self.plugin).start_processing } {
            let ok = unsafe { start(self.plugin) };
            if !ok {
                // Deactivate since we can't start processing
                if let Some(deactivate) = unsafe { (*self.plugin).deactivate } {
                    unsafe { deactivate(self.plugin) };
                }
                self.active = false;
                return false;
            }
        }
        true
    }

    /// Reset plugin to clean state by cycling stop/start processing.
    /// Clears reverb tails, delay lines, model state, etc.
    pub fn reset_processing(&mut self) {
        if !self.active {
            return;
        }
        if let Some(stop) = unsafe { (*self.plugin).stop_processing } {
            unsafe { stop(self.plugin) };
        }
        if let Some(start) = unsafe { (*self.plugin).start_processing } {
            unsafe { start(self.plugin) };
        }
    }

    /// Process audio through the plugin (single-output convenience wrapper).
    /// CLAP spec allows aliased input/output buffers so this is in-place.
    /// Works with any plugin — non-main output ports are silently dropped.
    pub fn process(&mut self, buf_l: &mut [f32], buf_r: &mut [f32], frames: usize) {
        // SAFETY: we hand the single mutable borrow pair to process_multi
        // as a one-element slice; both references disappear before this
        // function returns.
        let mut outs = [StereoBufMut {
            left: buf_l,
            right: buf_r,
        }];
        self.process_multi(&mut outs, frames);
    }

    /// Process audio through the plugin, delivering each declared output
    /// port into its own stereo buffer pair. `outputs[0]` is the main
    /// output (same role as [`ClapInstance::process`]). Extra entries
    /// beyond the plugin's declared output-port count are ignored; extra
    /// plugin ports beyond `outputs.len()` are silently dropped.
    ///
    /// This is the multi-output fast path used by the mixer for the drum
    /// plugin's per-group outputs.
    pub fn process_multi(&mut self, outputs: &mut [StereoBufMut<'_>], frames: usize) {
        if !self.active || frames == 0 {
            return;
        }

        let frames = frames.min(8192);

        // Point CLAP input buffers at the main output pair (in-place
        // processing — CLAP allows aliased in/out pointers).
        let (main_left_ptr, main_right_ptr) = outputs
            .first_mut()
            .map(|p| (p.left.as_mut_ptr(), p.right.as_mut_ptr()))
            .unwrap_or((ptr::null_mut(), ptr::null_mut()));
        let mut in_ptrs: [*mut f32; 2] = [main_left_ptr, main_right_ptr];

        let mut audio_in = clap_audio_buffer {
            data32: in_ptrs.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };

        // Refresh each output port's pointer array to point at the
        // caller's buffer slices for this block. We iterate up to the
        // smaller of the pre-allocated buffer array and the caller's
        // output slice so callers can pass fewer buffers (in which case
        // the plugin's extra ports are dropped) without crashing.
        let active_out_count = outputs.len().min(self.output_port_count);
        for i in 0..active_out_count {
            self.audio_out_ptrs[i][0] = outputs[i].left.as_mut_ptr();
            self.audio_out_ptrs[i][1] = outputs[i].right.as_mut_ptr();
            self.audio_out_buffers[i].data32 = self.audio_out_ptrs[i].as_mut_ptr();
            self.audio_out_buffers[i].channel_count = 2;
        }

        // Build input events from pending parameter changes (reuse pre-allocated buffer)
        self.param_event_buf.clear();
        self.param_event_buf
            .extend(self.pending_params.drain(..).map(|(param_id, value)| {
                clap_event_param_value {
                    header: clap_event_header {
                        size: std::mem::size_of::<clap_event_param_value>() as u32,
                        time: 0,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        type_: CLAP_EVENT_PARAM_VALUE,
                        flags: 0,
                    },
                    param_id,
                    cookie: ptr::null_mut(),
                    note_id: -1,
                    port_index: -1,
                    channel: -1,
                    key: -1,
                    value,
                }
            }));

        // Build note events from pending notes (reuse pre-allocated buffer)
        self.note_event_buf.clear();
        self.note_event_buf
            .extend(self.pending_notes.drain(..).map(|(is_on, key, vel, offset)| {
                clap_event_note {
                    header: clap_event_header {
                        size: std::mem::size_of::<clap_event_note>() as u32,
                        time: offset,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        type_: if is_on { CLAP_EVENT_NOTE_ON } else { CLAP_EVENT_NOTE_OFF },
                        flags: 0,
                    },
                    note_id: -1,
                    port_index: 0,
                    channel: 0,
                    key: key as i16,
                    velocity: vel as f64,
                }
            }));

        let mut event_ctx = MixedEventListCtx {
            param_events: std::mem::take(&mut self.param_event_buf),
            note_events: std::mem::take(&mut self.note_event_buf),
        };

        let in_events = clap_input_events {
            ctx: &mut event_ctx as *mut MixedEventListCtx as *mut c_void,
            size: Some(mixed_events_size),
            get: Some(mixed_events_get),
        };

        let out_events = clap_output_events {
            ctx: ptr::null_mut(),
            try_push: Some(discard_output_event),
        };

        let process_data = clap_process {
            steady_time: -1,
            frames_count: frames as u32,
            transport: ptr::null(),
            audio_inputs: &mut audio_in as *mut clap_audio_buffer as *const clap_audio_buffer,
            audio_outputs: self.audio_out_buffers.as_mut_ptr(),
            audio_inputs_count: 1,
            audio_outputs_count: active_out_count as u32,
            in_events: &in_events,
            out_events: &out_events,
        };

        if let Some(process_fn) = unsafe { (*self.plugin).process } {
            unsafe { process_fn(self.plugin, &process_data) };
        }

        // Reclaim event buffers for reuse (avoids allocation next call)
        self.param_event_buf = event_ctx.param_events;
        self.param_event_buf.clear();
        self.note_event_buf = event_ctx.note_events;
        self.note_event_buf.clear();
    }
}

impl Drop for ClapInstance {
    fn drop(&mut self) {
        // Tear down any open GUI first so the plugin can release its editor
        // thread before the rest of the plugin goes away.
        self.close_gui();
        if self.active {
            if let Some(stop) = unsafe { (*self.plugin).stop_processing } {
                unsafe { stop(self.plugin) };
            }
            if let Some(deactivate) = unsafe { (*self.plugin).deactivate } {
                unsafe { deactivate(self.plugin) };
            }
        }
        if let Some(destroy) = unsafe { (*self.plugin).destroy } {
            unsafe { destroy(self.plugin) };
        }
    }
}

// ---------------------------------------------------------------------------
// SyncClapInstance — Send + Sync wrapper
// ---------------------------------------------------------------------------

/// Wrapper that makes ClapInstance Send + Sync.
///
/// SAFETY: This is justified by the CLAP threading contract:
/// - Lifecycle methods (create/activate/destroy) are called from the engine thread only
/// - process() is called from the audio callback thread only
/// - set_param() is called from the engine thread, pending_params consumed by process()
pub struct SyncClapInstance(pub ClapInstance);

unsafe impl Send for SyncClapInstance {}
unsafe impl Sync for SyncClapInstance {}
