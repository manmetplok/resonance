//! Minimal CLAP plugin host for loading and running .clap plugins.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;
use std::pin::Pin;
use std::ptr;

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::entry::clap_plugin_entry;
use clap_sys::events::{
    clap_event_header, clap_event_param_value, clap_input_events, clap_output_events,
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE,
};
use clap_sys::ext::params::{clap_plugin_params, CLAP_EXT_PARAMS};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
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
// Event list for parameter changes
// ---------------------------------------------------------------------------

/// Context for input events that carries param value events.
struct ParamEventListCtx {
    events: Vec<clap_event_param_value>,
}

unsafe extern "C" fn param_events_size(list: *const clap_input_events) -> u32 {
    let ctx = &*((*list).ctx as *const ParamEventListCtx);
    ctx.events.len() as u32
}

unsafe extern "C" fn param_events_get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    let ctx = &*((*list).ctx as *const ParamEventListCtx);
    if (index as usize) < ctx.events.len() {
        &ctx.events[index as usize].header as *const clap_event_header
    } else {
        ptr::null()
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
            descriptors.push(PluginDescInfo { id, name, vendor });
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

        // Query params extension before activation
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

        Ok(ClapInstance {
            plugin,
            _host_data: host_data,
            active: true,
            params_ext,
            pending_params: Vec::new(),
            input_buf_l: vec![0.0; 8192],
            input_buf_r: vec![0.0; 8192],
            output_buf_l: vec![0.0; 8192],
            output_buf_r: vec![0.0; 8192],
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

pub struct ClapInstance {
    plugin: *const clap_plugin,
    _host_data: Pin<Box<HostData>>,
    active: bool,
    params_ext: Option<*const clap_plugin_params>,
    /// Pending parameter changes to send during next process() call.
    pending_params: Vec<(u32, f64)>,
    // Pre-allocated de-interleaved I/O buffers
    input_buf_l: Vec<f32>,
    input_buf_r: Vec<f32>,
    output_buf_l: Vec<f32>,
    output_buf_r: Vec<f32>,
}

impl ClapInstance {
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

    /// Process audio through the plugin.
    /// Sends any pending parameter changes as input events.
    pub fn process(&mut self, buf_l: &mut [f32], buf_r: &mut [f32], frames: usize) {
        if !self.active || frames == 0 {
            return;
        }

        let frames = frames.min(8192);

        // Copy input
        self.input_buf_l[..frames].copy_from_slice(&buf_l[..frames]);
        self.input_buf_r[..frames].copy_from_slice(&buf_r[..frames]);

        // Zero output
        self.output_buf_l[..frames].fill(0.0);
        self.output_buf_r[..frames].fill(0.0);

        // Set up audio buffers
        let mut in_ptrs: [*mut f32; 2] = [
            self.input_buf_l.as_mut_ptr(),
            self.input_buf_r.as_mut_ptr(),
        ];
        let mut out_ptrs: [*mut f32; 2] = [
            self.output_buf_l.as_mut_ptr(),
            self.output_buf_r.as_mut_ptr(),
        ];

        let mut audio_in = clap_audio_buffer {
            data32: in_ptrs.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };

        let mut audio_out = clap_audio_buffer {
            data32: out_ptrs.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };

        // Build input events from pending parameter changes
        let mut param_events: Vec<clap_event_param_value> = self
            .pending_params
            .drain(..)
            .map(|(param_id, value)| clap_event_param_value {
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
            })
            .collect();

        let mut event_ctx = ParamEventListCtx {
            events: std::mem::take(&mut param_events),
        };

        let in_events = clap_input_events {
            ctx: &mut event_ctx as *mut ParamEventListCtx as *mut c_void,
            size: Some(param_events_size),
            get: Some(param_events_get),
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
            audio_outputs: &mut audio_out,
            audio_inputs_count: 1,
            audio_outputs_count: 1,
            in_events: &in_events,
            out_events: &out_events,
        };

        if let Some(process_fn) = unsafe { (*self.plugin).process } {
            unsafe { process_fn(self.plugin, &process_data) };
        }

        // Copy output back
        buf_l[..frames].copy_from_slice(&self.output_buf_l[..frames]);
        buf_r[..frames].copy_from_slice(&self.output_buf_r[..frames]);
    }
}

impl Drop for ClapInstance {
    fn drop(&mut self) {
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
