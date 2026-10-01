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
    CLAP_EVENT_NOTE_ON, CLAP_EVENT_TRANSPORT, CLAP_TRANSPORT_HAS_BEATS_TIMELINE,
    CLAP_TRANSPORT_HAS_TEMPO, CLAP_TRANSPORT_HAS_TIME_SIGNATURE, CLAP_TRANSPORT_IS_PLAYING,
};
use clap_sys::fixedpoint::CLAP_BEATTIME_FACTOR;
use clap_sys::process::clap_process;

use super::instance::{ClapInstance, StereoBufMut};

// ---------------------------------------------------------------------------
// Event list for parameter changes + note events
// ---------------------------------------------------------------------------

/// Context for input events carrying both param value and note events.
/// Param events (time=0) come first, then note events, which
/// `process_multi_with_key` sorts by time and bounds to the block.
///
/// Also reused by [`super::params`] to build the param-only event list
/// handed to `clap_plugin_params.flush` (with `note_events` empty).
pub(super) struct MixedEventListCtx {
    pub(super) param_events: Vec<clap_event_param_value>,
    pub(super) note_events: Vec<clap_event_note>,
}

pub(super) unsafe extern "C" fn mixed_events_size(list: *const clap_input_events) -> u32 {
    let ctx = &*((*list).ctx as *const MixedEventListCtx);
    (ctx.param_events.len() + ctx.note_events.len()) as u32
}

pub(super) unsafe extern "C" fn mixed_events_get(
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

/// Split step for one queued note: an event inside the block is pushed
/// onto `out` as a CLAP note event and dropped from its queue (`false`);
/// a later one is re-based to the next call's start and kept (`true`).
#[inline]
fn keep_for_later(
    n: &mut (bool, u8, f32, u32),
    frames: u32,
    out: &mut Vec<clap_event_note>,
) -> bool {
    let (is_on, key, vel, offset) = *n;
    if offset >= frames {
        n.3 = offset - frames;
        return true;
    }
    out.push(clap_event_note {
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
    });
    false
}

/// Stable in-place sort by `header.time`. Insertion sort: no allocation
/// (std's stable sort allocates past 20 elements), and the list is
/// short and almost always already sorted. Stable because equal-time
/// order is meaningful and only the producer knows it — a retrigger
/// wants off-then-on, a zero-length note on-then-off.
#[inline]
fn sort_notes_by_time(events: &mut [clap_event_note]) {
    for i in 1..events.len() {
        let mut j = i;
        while j > 0 && events[j - 1].header.time > events[j].header.time {
            events.swap(j - 1, j);
            j -= 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Non-finite output scrub
// ---------------------------------------------------------------------------

/// Zero every non-finite sample (NaN, ±Inf) in `buf`, returning whether
/// anything was zeroed.
///
/// This is the host's ingress guard against a misbehaving plugin: one NaN
/// re-entering the mix graph latches permanently into every recursive
/// stage downstream (channel-strip filters, sends, meters, sidechain
/// detectors, other plugins' feedback state), muting or garbling the
/// chain until the plugin is reset. Every hosted process call runs its
/// output through this before the buffer leaves the host layer.
///
/// Fast path: sum the block and check the sum — NaN/Inf propagate through
/// f32 addition and never cancel back to finite (`Inf + -Inf == NaN`), so
/// a finite sum proves a finite block at one add per sample. The sum of
/// large-but-finite samples can itself overflow to Inf; that false
/// positive just falls through to the slow pass, which is authoritative:
/// it zeroes exactly the samples that are actually non-finite (preserving
/// finite neighbours) and leaves an all-finite buffer untouched.
///
/// Silent by design — no logging here, this runs on the audio thread.
#[inline]
fn scrub_non_finite(buf: &mut [f32]) -> bool {
    let sum: f32 = buf.iter().sum();
    if sum.is_finite() {
        return false;
    }
    let mut scrubbed = false;
    for s in buf.iter_mut() {
        if !s.is_finite() {
            *s = 0.0;
            scrubbed = true;
        }
    }
    scrubbed
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
        self.process_multi_with_key(outputs, None, frames);
    }

    /// True when the plugin declares an external sidechain (key) input
    /// port alongside its main input. Only such a plugin can be handed a
    /// key by [`ClapInstance::process_multi_with_key`]; for anything else
    /// the key is ignored, because connecting a port the plugin never
    /// declared is a spec violation the plugin is entitled to trust.
    pub fn has_sidechain_input(&self) -> bool {
        self.input_port_count >= 2
    }

    /// [`ClapInstance::process_multi`] with an optional external sidechain
    /// (key) signal.
    ///
    /// `key` is a stereo pair covering at least `frames` samples. It is
    /// passed as a **second, non-main CLAP input port**, which is what the
    /// plugin's own `SIDECHAIN_INPUT` declaration opted into; the plugin
    /// reads it for detection and never writes to it. A `key` handed to a
    /// plugin that declares no sidechain port is dropped rather than
    /// connected.
    pub fn process_multi_with_key(
        &mut self,
        outputs: &mut [StereoBufMut<'_>],
        key: Option<(&[f32], &[f32])>,
        frames: usize,
    ) {
        if !self.active || frames == 0 {
            return;
        }

        // No destination buffers: nothing could be produced, and — worse
        // — for an effect the in-place main *input* below would be built
        // from the null pointer pair `outputs.first_mut()` falls back
        // to, handed over as `audio_inputs_count = 1`. A conforming
        // plugin dereferences a connected input port unconditionally.
        // No call site passes an empty slice today; this guard keeps the
        // function safe for any input rather than safe by coincidence.
        if outputs.is_empty() {
            return;
        }

        let frames = frames.min(8192);
        // The stopped-transport window counts down in rendered frames
        // (`ClapInstance::wants_idle_process`, code review MIX-08).
        self.idle_hold_frames = self.idle_hold_frames.saturating_sub(frames as u32);

        // Point CLAP input buffers at the main output pair (in-place
        // processing — CLAP allows aliased in/out pointers).
        let (main_left_ptr, main_right_ptr) = outputs
            .first_mut()
            .map(|p| (p.left.as_mut_ptr(), p.right.as_mut_ptr()))
            .unwrap_or((ptr::null_mut(), ptr::null_mut()));
        let mut in_ptrs: [*mut f32; 2] = [main_left_ptr, main_right_ptr];

        // Port 0 is the main input. Port 1, when present, is the external
        // key — only connected for a plugin that declared one AND when the
        // caller supplied a signal, so the count below stays 1 in every
        // other case and the existing path is untouched.
        let key = key.filter(|_| self.has_sidechain_input());
        let mut key_ptrs: [*mut f32; 2] = match key {
            // Cast away const: CLAP's buffer struct is shared between
            // inputs and outputs so it has no const variant. The plugin
            // contract for an input port is read-only, and the bridge in
            // `resonance-plugin` hands the key out as `&[f32]`.
            Some((l, r)) => [l.as_ptr() as *mut f32, r.as_ptr() as *mut f32],
            None => [ptr::null_mut(), ptr::null_mut()],
        };

        let mut audio_in = [
            clap_audio_buffer {
                data32: in_ptrs.as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: 2,
                latency: 0,
                constant_mask: 0,
            },
            clap_audio_buffer {
                data32: key_ptrs.as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: 2,
                latency: 0,
                constant_mask: 0,
            },
        ];
        let audio_inputs_count: u32 = if key.is_some() { 2 } else { 1 };

        // Refresh each output port's pointer array to point at the
        // caller's buffer slices for this block. We iterate up to the
        // smaller of the pre-allocated buffer array and the caller's
        // output slice so callers can pass fewer buffers (in which case
        // the plugin's extra ports are dropped) without crashing.
        let active_out_count = outputs.len().min(self.output_port_count);
        for (i, port) in outputs.iter_mut().enumerate().take(active_out_count) {
            self.audio_out_ptrs[i][0] = port.left.as_mut_ptr();
            self.audio_out_ptrs[i][1] = port.right.as_mut_ptr();
            self.audio_out_buffers[i].data32 = self.audio_out_ptrs[i].as_mut_ptr();
            self.audio_out_buffers[i].channel_count = 2;
        }

        // Build input events from pending parameter changes (reuse pre-allocated buffer)
        self.param_event_buf.clear();
        self.param_event_buf.extend(
            self.pending_params
                .drain(..)
                .map(|(param_id, value)| super::params::param_value_event(param_id, value)),
        );

        // Build note events (reuse pre-allocated buffer). CLAP requires
        // input events inside the block and sorted by time. Events past
        // this call's end — a live note queued against the whole callback
        // when this is the head sub-block of a loop seam — are carried,
        // re-based, to the next call instead of being handed over out of
        // range (the plugin would never reach them, and the queue is
        // drained, so a lost note-off sticks). Allocation-free: both
        // queues are filtered in place and the carry is capacity-bounded.
        let frames_u32 = frames as u32;
        self.note_event_buf.clear();
        let note_buf = &mut self.note_event_buf;
        // Carried events were queued earlier, so they go first; the
        // stable sort below keeps that order among equal times.
        self.carried_notes
            .retain_mut(|n| keep_for_later(n, frames_u32, note_buf));
        self.pending_notes
            .retain_mut(|n| keep_for_later(n, frames_u32, note_buf));
        for n in self.pending_notes.drain(..) {
            if self.carried_notes.len() < crate::limits::MAX_PENDING_NOTES {
                self.carried_notes.push(n);
            }
        }
        sort_notes_by_time(&mut self.note_event_buf);

        let mut event_ctx = MixedEventListCtx {
            param_events: std::mem::take(&mut self.param_event_buf),
            note_events: std::mem::take(&mut self.note_event_buf),
        };

        let in_events = clap_input_events {
            ctx: &mut event_ctx as *mut MixedEventListCtx as *mut c_void,
            size: Some(mixed_events_size),
            get: Some(mixed_events_get),
        };

        // The plugin's own param edits (output parameter events) are
        // kept for the engine thread; everything else is dropped.
        let mut out_param_events = std::mem::take(&mut self.out_param_events);
        let out_events = clap_output_events {
            ctx: &mut out_param_events as *mut Vec<super::params::OutParamEvent> as *mut c_void,
            try_push: Some(super::params::collect_output_event),
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
            audio_inputs: audio_in.as_mut_ptr() as *const clap_audio_buffer,
            audio_outputs: self.audio_out_buffers.as_mut_ptr(),
            audio_inputs_count,
            audio_outputs_count: active_out_count as u32,
            in_events: &in_events,
            out_events: &out_events,
        };

        if let Some(process_fn) = unsafe { (*self.plugin).process } {
            // `process()` is `[audio-thread]`: whichever thread renders
            // this block holds the role for the call.
            let _audio = super::thread_check::AudioThreadScope::enter();
            // Open the CLAP `thread-pool` window for exactly this call.
            self.host_data
                .in_process
                .store(true, std::sync::atomic::Ordering::Release);
            unsafe { process_fn(self.plugin, &process_data) };
            self.host_data
                .in_process
                .store(false, std::sync::atomic::Ordering::Release);

            // Finite scrub at the plugin-output boundary: whatever the
            // plugin just wrote is about to re-enter the mix graph (track
            // chains, bus/master sums, sends, sidechain taps, sub-track
            // fan-out all read these buffers), so this is the one choke
            // point that guards every path. See `scrub_non_finite`.
            for port in outputs.iter_mut().take(active_out_count) {
                scrub_non_finite(&mut port.left[..frames]);
                scrub_non_finite(&mut port.right[..frames]);
            }
        }

        // Reclaim event buffers for reuse (avoids allocation next call)
        self.out_param_events = out_param_events;
        self.param_event_buf = event_ctx.param_events;
        self.param_event_buf.clear();
        self.note_event_buf = event_ctx.note_events;
        self.note_event_buf.clear();
    }
}
