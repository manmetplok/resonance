//! Audio-thread fast path: build the CLAP input event list (param
//! changes + note events sorted by time), point the per-port buffer
//! array at the caller's slices, latch transport state into a
//! `clap_event_transport`, and call into the plugin's `process()`.
//! Allocation-free.

use std::ffi::c_void;
use std::ptr;

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::events::{
    clap_event_header, clap_event_note, clap_event_param_value, clap_event_transport,
    clap_input_events, clap_output_events, CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_NOTE_OFF,
    CLAP_EVENT_NOTE_ON, CLAP_EVENT_PARAM_VALUE, CLAP_EVENT_TRANSPORT,
    CLAP_TRANSPORT_HAS_BEATS_TIMELINE, CLAP_TRANSPORT_HAS_TEMPO, CLAP_TRANSPORT_HAS_TIME_SIGNATURE,
    CLAP_TRANSPORT_IS_PLAYING,
};
use clap_sys::fixedpoint::CLAP_BEATTIME_FACTOR;
use clap_sys::process::clap_process;

use super::instance::{ClapInstance, StereoBufMut};

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
// process / process_multi
// ---------------------------------------------------------------------------

impl ClapInstance {
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
        self.note_event_buf.extend(self.pending_notes.drain(..).map(
            |(is_on, key, vel, offset)| clap_event_note {
                header: clap_event_header {
                    size: std::mem::size_of::<clap_event_note>() as u32,
                    time: offset,
                    space_id: CLAP_CORE_EVENT_SPACE_ID,
                    type_: if is_on {
                        CLAP_EVENT_NOTE_ON
                    } else {
                        CLAP_EVENT_NOTE_OFF
                    },
                    flags: 0,
                },
                note_id: -1,
                port_index: 0,
                channel: 0,
                key: key as i16,
                velocity: vel as f64,
            },
        ));

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

        let mut transport_flags: u32 = 0;
        if self.transport_valid {
            transport_flags |= CLAP_TRANSPORT_HAS_TEMPO
                | CLAP_TRANSPORT_HAS_BEATS_TIMELINE
                | CLAP_TRANSPORT_HAS_TIME_SIGNATURE;
            if self.transport_playing {
                transport_flags |= CLAP_TRANSPORT_IS_PLAYING;
            }
        }
        let beats_fp = (self.transport_pos_beats * CLAP_BEATTIME_FACTOR as f64).round() as i64;
        let transport_event = clap_event_transport {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_transport>() as u32,
                time: 0,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_TRANSPORT,
                flags: 0,
            },
            flags: transport_flags,
            song_pos_beats: beats_fp,
            song_pos_seconds: 0,
            tempo: self.transport_bpm,
            tempo_inc: 0.0,
            loop_start_beats: 0,
            loop_end_beats: 0,
            loop_start_seconds: 0,
            loop_end_seconds: 0,
            bar_start: 0,
            bar_number: 0,
            tsig_num: self.transport_num,
            tsig_denom: self.transport_den,
        };
        let transport_ptr: *const clap_event_transport = if self.transport_valid {
            &transport_event
        } else {
            ptr::null()
        };

        let process_data = clap_process {
            steady_time: -1,
            frames_count: frames as u32,
            transport: transport_ptr,
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
