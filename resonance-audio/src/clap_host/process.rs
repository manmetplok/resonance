//! Audio-thread fast path: build the CLAP input event list (param
//! changes + note events sorted by time), point the per-port buffer
//! array at the caller's slices, latch transport state into a
//! `clap_event_transport`, and call into the plugin's `process()`.
//! Allocation-free.

use std::ffi::c_void;
use std::ptr;

use clap_sys::events::{
    clap_event_header, clap_event_midi, clap_event_note, clap_event_param_value,
    clap_event_transport, clap_input_events, clap_output_events, CLAP_CORE_EVENT_SPACE_ID,
    CLAP_EVENT_MIDI, CLAP_EVENT_NOTE_OFF, CLAP_EVENT_NOTE_ON, CLAP_EVENT_TRANSPORT,
    CLAP_TRANSPORT_HAS_BEATS_TIMELINE,
    CLAP_TRANSPORT_HAS_TEMPO, CLAP_TRANSPORT_HAS_TIME_SIGNATURE, CLAP_TRANSPORT_IS_PLAYING,
};
use clap_sys::fixedpoint::CLAP_BEATTIME_FACTOR;
use clap_sys::process::clap_process;

use super::instance::{ClapInstance, StereoBufMut};

// ---------------------------------------------------------------------------
// Event list for parameter changes + note events
// ---------------------------------------------------------------------------

/// The input event list handed to `process()` (and, with no notes or
/// MIDI, to `params.flush`): parameter changes, CLAP note events and
/// `CLAP_EVENT_MIDI` messages, each list sorted by time on its own, read
/// in the merged order `order` gives — CLAP requires the plugin see one
/// list sorted by time (code review HOST-06: param events are no longer
/// all at time 0). At equal times params come first, then notes, then
/// MIDI, so a value lands before the note it shapes.
pub(super) struct MixedEventListCtx {
    pub(super) param_events: Vec<clap_event_param_value>,
    pub(super) note_events: Vec<clap_event_note>,
    pub(super) midi_events: Vec<clap_event_midi>,
    /// Each entry is a list tag (top two bits) and an index into it.
    pub(super) order: Vec<u32>,
}

const ORDER_PARAM: u32 = 0;
const ORDER_NOTE: u32 = 1 << 30;
const ORDER_MIDI: u32 = 2 << 30;
const ORDER_INDEX: u32 = (1 << 30) - 1;

impl MixedEventListCtx {
    /// Fill `order` with a three-way merge of the (individually sorted)
    /// lists. Allocation-free while `order` has room for all of them.
    pub(super) fn merge_order(&mut self) {
        self.order.clear();
        let (mut p, mut n, mut m) = (0usize, 0usize, 0usize);
        loop {
            let tp = self.param_events.get(p).map(|e| e.header.time);
            let tn = self.note_events.get(n).map(|e| e.header.time);
            let tm = self.midi_events.get(m).map(|e| e.header.time);
            let take_param = tp.is_some_and(|t| tn.is_none_or(|u| t <= u) && tm.is_none_or(|u| t <= u));
            let take_note = !take_param && tn.is_some_and(|t| tm.is_none_or(|u| t <= u));
            if take_param {
                self.order.push(ORDER_PARAM | p as u32);
                p += 1;
            } else if take_note {
                self.order.push(ORDER_NOTE | n as u32);
                n += 1;
            } else if tm.is_some() {
                self.order.push(ORDER_MIDI | m as u32);
                m += 1;
            } else {
                break;
            }
        }
    }
}

pub(super) unsafe extern "C" fn mixed_events_size(list: *const clap_input_events) -> u32 {
    let ctx = &*((*list).ctx as *const MixedEventListCtx);
    ctx.order.len() as u32
}

pub(super) unsafe extern "C" fn mixed_events_get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    let ctx = &*((*list).ctx as *const MixedEventListCtx);
    let Some(&entry) = ctx.order.get(index as usize) else {
        return ptr::null();
    };
    let idx = (entry & ORDER_INDEX) as usize;
    let header = match entry & !ORDER_INDEX {
        ORDER_PARAM => ctx.param_events.get(idx).map(|e| &e.header),
        ORDER_NOTE => ctx.note_events.get(idx).map(|e| &e.header),
        _ => ctx.midi_events.get(idx).map(|e| &e.header),
    };
    header.map_or(ptr::null(), |h| h as *const clap_event_header)
}

/// Stable in-place insertion sort of any event list by `header.time`
/// (allocation-free; the lists are short and almost always sorted).
#[inline]
fn sort_by_time<T>(events: &mut [T], time: impl Fn(&T) -> u32) {
    for i in 1..events.len() {
        let mut j = i;
        while j > 0 && time(&events[j - 1]) > time(&events[j]) {
            events.swap(j - 1, j);
            j -= 1;
        }
    }
}

/// A `CLAP_EVENT_MIDI` on note port 0.
#[inline]
fn midi_event(time: u32, data: [u8; 3]) -> clap_event_midi {
    clap_event_midi {
        header: clap_event_header {
            size: std::mem::size_of::<clap_event_midi>() as u32,
            time,
            space_id: CLAP_CORE_EVENT_SPACE_ID,
            type_: CLAP_EVENT_MIDI,
            flags: 0,
        },
        port_index: 0,
        data,
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
    sort_by_time(events, |e| e.header.time);
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
    /// Works with any plugin — non-main output ports are rendered into
    /// host scratch and dropped.
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
    /// beyond the plugin's declared output-port count are left untouched;
    /// plugin ports beyond `outputs.len()` still get buffers of their own
    /// (host scratch, discarded), as CLAP requires every declared port be
    /// passed.
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
            // Automation points are queued for this block only.
            self.pending_auto.clear();
            return;
        }

        // No destination buffers: nothing could be produced, and the
        // in-place main *input* below has no pair to alias. No call site
        // passes an empty slice today; this guard keeps the function safe
        // for any input rather than safe by coincidence.
        if outputs.is_empty() {
            self.pending_auto.clear();
            return;
        }

        // Any `process()` satisfies a pending `clap_host.request_process()`
        // (`ClapInstance::take_process_request`). Cleared BEFORE the
        // plugin runs, and with an acquiring swap rather than a plain
        // store: the plugin's own reads in this `process()` cannot move
        // ahead of the clear, so a request raised after it is either
        // already visible to this call or left set for the next one —
        // never wiped unseen.
        self.host_data
            .process_requested
            .swap(false, std::sync::atomic::Ordering::AcqRel);

        // Never run past a buffer the plugin is handed (code review
        // HOST-12): the main pair, every caller output, the key, and the
        // pre-allocated port backing. A caller passing a shorter slice
        // than `frames` is a bug upstream; in release the block is cut
        // short rather than read or written out of bounds.
        // (`max_frames` is the activation's `max_frames_count`; a longer
        // block is cut to it silently, as it always was.)
        let frames = frames.min(self.ports.max_frames());
        let mut max = frames;
        for port in outputs.iter() {
            max = max.min(port.left.len()).min(port.right.len());
        }
        let key = key.filter(|_| self.has_sidechain_input());
        if let Some((l, r)) = key {
            max = max.min(l.len()).min(r.len());
        }
        debug_assert!(
            frames <= max,
            "process_multi_with_key: {frames} frames against buffers of {max}"
        );
        let frames = frames.min(max);
        if frames == 0 {
            self.pending_auto.clear();
            return;
        }
        // The stopped-transport window counts down in rendered frames
        // (`ClapInstance::wants_idle_process`, code review MIX-08).
        self.idle_hold_frames = self.idle_hold_frames.saturating_sub(frames as u32);

        // Every port the plugin declared, each with its declared channel
        // count (code review HOST-05): the caller's pairs where it has
        // them — the main input in place on the main output, CLAP allows
        // aliased in/out pointers — and pre-allocated silence / scratch
        // for the rest. The key, when this plugin declares a key port
        // and the caller routed one, is input port 1; unrouted, that port
        // reads silence.
        self.ports.bind(outputs, key, frames);

        let frames_u32 = frames as u32;
        let last_frame = frames_u32 - 1;

        // Parameter events (reuse pre-allocated buffer): changes queued
        // through `set_param` at the block start, then the automation
        // points at their own samples (code review HOST-06), sorted by
        // time — stable, so a later-queued value at the same sample wins.
        self.param_event_buf.clear();
        self.param_event_buf.extend(
            self.pending_params
                .drain(..)
                .map(|(param_id, value)| super::params::param_value_event(param_id, value)),
        );
        self.param_event_buf.extend(self.pending_auto.drain(..).map(|(time, param_id, value)| {
            // Queued for this block, so inside it; clamped in case the
            // block was cut short (`frames` above).
            super::params::param_value_event_at(param_id, value, time.min(last_frame))
        }));
        sort_by_time(&mut self.param_event_buf, |e| e.header.time);

        // Build note events (reuse pre-allocated buffer). CLAP requires
        // input events inside the block and sorted by time. Events past
        // this call's end — a live note queued against the whole callback
        // when this is the head sub-block of a loop seam — are carried,
        // re-based, to the next call instead of being handed over out of
        // range (the plugin would never reach them, and the queue is
        // drained, so a lost note-off sticks). Allocation-free: both
        // queues are filtered in place and the carry is capacity-bounded.
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

        // Raw MIDI (code review HOST-13): queued controllers, and — for a
        // plugin whose note port prefers or only takes MIDI — the notes,
        // re-spoken as MIDI 1.0 note on / off on channel 1.
        self.midi_event_buf.clear();
        self.midi_event_buf.extend(
            self.pending_midi
                .drain(..)
                .map(|(time, data)| midi_event(time.min(last_frame), data)),
        );
        if self.note_dialect.notes_as_midi {
            for n in self.note_event_buf.drain(..) {
                let key = n.key.clamp(0, 127) as u8;
                let data = if n.header.type_ == CLAP_EVENT_NOTE_ON {
                    let velocity = (n.velocity * 127.0).round().clamp(1.0, 127.0) as u8;
                    [0x90, key, velocity]
                } else {
                    [0x80, key, 0]
                };
                self.midi_event_buf.push(midi_event(n.header.time, data));
            }
        }
        if !self.note_dialect.has_note_input {
            // A plugin that declares no note input gets none.
            self.note_event_buf.clear();
            self.midi_event_buf.clear();
        }
        sort_by_time(&mut self.midi_event_buf, |e| e.header.time);

        let mut event_ctx = MixedEventListCtx {
            param_events: std::mem::take(&mut self.param_event_buf),
            note_events: std::mem::take(&mut self.note_event_buf),
            midi_events: std::mem::take(&mut self.midi_event_buf),
            order: std::mem::take(&mut self.event_order),
        };
        event_ctx.merge_order();

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
            audio_inputs: self.ports.inputs_ptr(),
            audio_outputs: self.ports.outputs_ptr(),
            audio_inputs_count: self.ports.input_count() as u32,
            audio_outputs_count: self.ports.output_count() as u32,
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
            let produced = self.ports.finish(outputs, frames);
            for port in outputs.iter_mut().take(produced) {
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
        self.midi_event_buf = event_ctx.midi_events;
        self.midi_event_buf.clear();
        self.event_order = event_ctx.order;
        self.event_order.clear();
    }
}
