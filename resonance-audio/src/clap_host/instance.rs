//! `ClapInstance` is one running CLAP plugin instance. The struct lives
//! here; per-concern impl blocks are in sibling modules:
//! - parameter / note queues, transport latching, simple accessors are
//!   below in this file;
//! - GUI extension methods in [`super::gui`];
//! - state extension + reset in [`super::state`];
//! - audio-thread `process` / `process_multi` in [`super::process`].
//!
//! All struct fields are `pub(super)` so sibling impl blocks can reach
//! them without forcing every method through this file.

use std::ffi::CStr;
use std::pin::Pin;

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::events::{clap_event_note, clap_event_param_value};
use clap_sys::ext::audio_ports::{clap_audio_port_info, clap_plugin_audio_ports};
use clap_sys::ext::gui::clap_plugin_gui;
use clap_sys::ext::params::clap_plugin_params;
use clap_sys::ext::state::clap_plugin_state;
use clap_sys::plugin::clap_plugin;

use crate::types::ParamInfo;

use super::HostData;

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
    pub(super) plugin: *const clap_plugin,
    pub(super) _host_data: Pin<Box<HostData>>,
    pub(super) active: bool,
    pub(super) sample_rate: u32,
    pub(super) params_ext: Option<*const clap_plugin_params>,
    pub(super) state_ext: Option<*const clap_plugin_state>,
    pub(super) audio_ports_ext: Option<*const clap_plugin_audio_ports>,
    pub(super) gui_ext: Option<*const clap_plugin_gui>,
    /// True when `gui_create` has been called and `gui_destroy` hasn't yet.
    pub(super) gui_open: bool,
    /// Number of output audio ports as reported by the plugin's audio-ports
    /// extension at activation time. Cached so the mixer can size its
    /// per-port scratch buffers without re-querying on every block.
    /// Always >= 1 because resonance-plugin rejects empty output layouts.
    pub(super) output_port_count: usize,
    /// Pending parameter changes to send during next process() call.
    pub(super) pending_params: Vec<(u32, f64)>,
    /// Pre-allocated buffer for CLAP parameter events (reused across process() calls).
    pub(super) param_event_buf: Vec<clap_event_param_value>,
    /// Pending note events to send during next process() call.
    /// Each entry: (is_note_on, key, velocity, sample_offset)
    pub(super) pending_notes: Vec<(bool, u8, f32, u32)>,
    /// Pre-allocated buffer for CLAP note events.
    pub(super) note_event_buf: Vec<clap_event_note>,
    /// Pre-allocated scratch for the CLAP audio output buffer array,
    /// one entry per output port. Reused across every `process_multi`
    /// call so the audio thread never allocates.
    pub(super) audio_out_buffers: Vec<clap_audio_buffer>,
    /// Per-port channel pointer array (2 pointers per port). Each block's
    /// `process_multi` call refreshes these to point at the caller's
    /// supplied slices before handing them to CLAP.
    pub(super) audio_out_ptrs: Vec<[*mut f32; 2]>,
    /// Latched transport state, set by the mixer before each process() call.
    pub(super) transport_bpm: f64,
    pub(super) transport_num: u16,
    pub(super) transport_den: u16,
    pub(super) transport_playing: bool,
    pub(super) transport_pos_beats: f64,
    pub(super) transport_valid: bool,
}

impl ClapInstance {
    /// Build the instance from the parts produced by
    /// [`super::ClapBundle::create_instance`]. Internal use only — kept
    /// in this module so the field invariants stay private.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn from_parts(
        plugin: *const clap_plugin,
        host_data: Pin<Box<HostData>>,
        sample_rate: u32,
        params_ext: Option<*const clap_plugin_params>,
        state_ext: Option<*const clap_plugin_state>,
        audio_ports_ext: Option<*const clap_plugin_audio_ports>,
        gui_ext: Option<*const clap_plugin_gui>,
        output_port_count: usize,
        audio_out_buffers: Vec<clap_audio_buffer>,
        audio_out_ptrs: Vec<[*mut f32; 2]>,
    ) -> Self {
        Self {
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
            pending_notes: Vec::with_capacity(crate::limits::MAX_PENDING_NOTES),
            note_event_buf: Vec::new(),
            audio_out_buffers,
            audio_out_ptrs,
            transport_bpm: 120.0,
            transport_num: 4,
            transport_den: 4,
            transport_playing: false,
            transport_pos_beats: 0.0,
            transport_valid: false,
        }
    }

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
            let mut info =
                std::mem::MaybeUninit::<clap_sys::ext::params::clap_param_info>::uninit();
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
    /// Deduplicates by param_id (last value wins) and caps at 128 entries
    /// to prevent unbounded growth when the GUI automates many parameters
    /// between process calls.
    pub fn set_param(&mut self, param_id: u32, value: f64) {
        if let Some(existing) = self
            .pending_params
            .iter_mut()
            .find(|(id, _)| *id == param_id)
        {
            existing.1 = value;
        } else if self.pending_params.len() < crate::limits::MAX_PENDING_PARAMS {
            self.pending_params.push((param_id, value));
        }
    }

    /// Queue a note-on event to be sent during the next process() call.
    pub fn queue_note_on(&mut self, key: u8, velocity: f32, sample_offset: u32) {
        if self.pending_notes.len() < crate::limits::MAX_PENDING_NOTES {
            self.pending_notes
                .push((true, key, velocity, sample_offset));
        }
    }

    /// Queue a note-off event to be sent during the next process() call.
    pub fn queue_note_off(&mut self, key: u8, sample_offset: u32) {
        if self.pending_notes.len() < crate::limits::MAX_PENDING_NOTES {
            self.pending_notes.push((false, key, 0.0, sample_offset));
        }
    }

    /// Queue note-off for all 128 MIDI notes (to clear stuck notes).
    pub fn all_notes_off(&mut self) {
        for key in 0..128u8 {
            self.pending_notes.push((false, key, 0.0, 0));
        }
    }

    /// Latch the current transport state so the next process() call can
    /// forward it to the plugin via `clap_event_transport`.
    pub fn set_transport(&mut self, bpm: f64, num: u16, den: u16, playing: bool, pos_beats: f64) {
        self.transport_bpm = bpm;
        self.transport_num = num;
        self.transport_den = den;
        self.transport_playing = playing;
        self.transport_pos_beats = pos_beats;
        self.transport_valid = true;
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
