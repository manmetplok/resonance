//! `ClapBundle` loads one `.clap` shared library and exposes its
//! plugin factory. Each bundle owns the `libloading::Library` handle
//! and runs `clap_entry.deinit()` in its `Drop` impl.
//!
//! Bundles are immutable after construction; everything that mutates
//! per-instance state lives on the [`super::ClapInstance`] returned by
//! [`ClapBundle::create_instance`].

use std::ffi::{CStr, CString};
use std::path::Path;
use std::ptr;

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::entry::clap_plugin_entry;
use clap_sys::ext::audio_ports::{clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS};
use clap_sys::ext::gui::{clap_plugin_gui, CLAP_EXT_GUI};
use clap_sys::ext::params::{clap_plugin_params, CLAP_EXT_PARAMS};
use clap_sys::ext::state::{clap_plugin_state, CLAP_EXT_STATE};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;

use crate::types::PluginDescInfo;

use super::create_host_data;
use super::instance::ClapInstance;

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
        let path_cstring = CString::new(path_str).map_err(|e| format!("Invalid path: {}", e))?;

        let library = unsafe { libloading::Library::new(path) }
            .map_err(|e| format!("Failed to load library: {}", e))?;

        let entry: *const clap_plugin_entry = unsafe {
            let symbol: libloading::Symbol<*const clap_plugin_entry> =
                library
                    .get(b"clap_entry")
                    .map_err(|e| format!("No clap_entry symbol: {}", e))?;
            *symbol
        };

        if entry.is_null() {
            return Err("clap_entry is null".to_string());
        }

        let init_fn =
            unsafe { (*entry).init }.ok_or_else(|| "clap_entry.init is null".to_string())?;
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

            descriptors.push(PluginDescInfo {
                id,
                name,
                vendor,
                is_instrument,
            });
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

        Ok(ClapInstance::from_parts(
            plugin,
            host_data,
            sample_rate,
            params_ext,
            state_ext,
            audio_ports_ext,
            gui_ext,
            output_port_count,
            audio_out_buffers,
            audio_out_ptrs,
        ))
    }
}

impl Drop for ClapBundle {
    fn drop(&mut self) {
        if let Some(deinit) = unsafe { (*self.entry).deinit } {
            unsafe { deinit() };
        }
    }
}
