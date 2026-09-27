//! The callback's parameter structs.
//!
//! [`mix_audio`](super::mix_audio) used to take 28 positional arguments,
//! which every branch helper would have had to re-thread. They are split
//! here the way the borrow checker wants them — and the way
//! [`BlockInputs`] / [`BlockScratch`] already split the render core
//! (ba todo #1252):
//!
//! - [`CallbackInputs`]: everything the callback reads. All shared
//!   references and `Copy`, so passing it around copies a handful of
//!   pointers.
//! - [`CallbackScratch`]: the engine's pre-allocated buffers, mutably
//!   borrowed for one callback. The fields are deliberately disjoint, so a
//!   phase can hold the output buffer, the MIDI stash and the monitor
//!   scratch at once without copying.
//! - [`BlockTiming`]: the tempo / transport snapshot taken once per
//!   callback, so the render, the metronome and the plugin transport all
//!   agree across the buffer.
//! - [`MonitorRead`]: what the monitor ring actually yielded this block.

use std::ops::Range;

use std::sync::atomic::Ordering;

use crate::engine::reference::ABMeters;
use crate::engine::{AutomationSnapshot, SharedState};
use crate::latency::LatencyComp;
use crate::midi_hardware::LiveMidiEvent;
use crate::types::*;

use crate::mixer::common::{transport_pos_beats, TransportContinuity, TransportSnap};
use crate::mixer::midi_stash::MidiStash;
use crate::mixer::render_core::BlockScratch;

/// The mixer callback an output backend drives: fills the interleaved
/// f32 slice (`frames * channels` samples) for one output cycle.
/// Boxed so `engine::mod` can hand over the fully-captured
/// [`mix_audio`](super::mix_audio) closure without threading its dozen
/// generics through the backend builders (native PipeWire on Linux,
/// cpal everywhere).
pub(crate) type MixFn = Box<dyn FnMut(&mut [f32], usize) + Send + 'static>;

/// Everything one audio callback reads: the engine's shared state (the
/// render graph among it — no lock since ARCH-02 B-5), the wait-free
/// snapshots it may load, and the stream geometry. All borrowed and
/// `Copy`.
#[derive(Clone, Copy)]
pub(crate) struct CallbackInputs<'a> {
    /// Channel count of the interleaved output buffer.
    pub(crate) channels: usize,
    pub(crate) shared: &'a SharedState,
    pub(crate) tempo_map: &'a arc_swap::ArcSwap<TempoMap>,
    pub(crate) latency_comp: &'a arc_swap::ArcSwap<LatencyComp>,
    pub(crate) automation: &'a arc_swap::ArcSwap<AutomationSnapshot>,
    pub(crate) sample_rate: u32,
    /// Live hardware-MIDI in, and the forward channel that hands each
    /// event on to the engine thread for recording / MIDI-thru.
    pub(crate) live_midi_rx: &'a crossbeam_channel::Receiver<LiveMidiEvent>,
    pub(crate) live_midi_fwd: &'a crossbeam_channel::Sender<LiveMidiEvent>,
    /// Frames the engine sized its scratch for; a larger request from the
    /// backend is clamped to this.
    pub(crate) buf_frames: usize,
    /// The graph quantum, used as the monitor ring's jitter margin.
    pub(crate) quantum: usize,
}

/// The engine's pre-allocated scratch, mutably borrowed for one callback.
pub(crate) struct CallbackScratch<'a> {
    /// Interleaved output buffer handed over by the backend.
    pub(crate) data: &'a mut [f32],
    pub(crate) track_buf_l: &'a mut [f32],
    pub(crate) track_buf_r: &'a mut [f32],
    pub(crate) bus_bufs: &'a mut [(Vec<f32>, Vec<f32>)],
    /// Per-plugin-output-port scratch used for multi-output instruments
    /// (e.g. resonance-drums with its 7 group/overhead ports). Sized to
    /// `MAX_PLUGIN_OUTPUT_PORTS` pairs by the engine; a block only touches
    /// the first N slots, where N is the active plugin's port count.
    pub(crate) port_scratch: &'a mut [(Vec<f32>, Vec<f32>)],
    pub(crate) note_event_buf: &'a mut Vec<PendingNoteEvent>,
    pub(crate) midi_stash: &'a mut MidiStash,
    pub(crate) monitor_cons: &'a mut ringbuf::HeapCons<f32>,
    pub(crate) monitor_temp: &'a mut [f32],
    pub(crate) monitor_drain: &'a mut super::super::monitor::MonitorDrain,
    /// A/B metering taps: the mix tap is fed the processed-mix output at
    /// the end of a playing block; the reference tap is fed the reference
    /// PCM when the monitored source is a reference. Both publish their
    /// snapshot into `shared` for the control thread's `PollABMeters`.
    pub(crate) ab_meters: &'a mut ABMeters,
    /// Per-source key capture buffers (doc: `types::sidechain`). Owned by
    /// the audio thread and pre-allocated, so routing a sidechain never
    /// allocates on the realtime path.
    pub(crate) sidechain: &'a mut SidechainTaps,
    /// The per-track render slots (`render::slots`), grown by the engine
    /// thread and adopted at the top of each playing block.
    pub(crate) track_slots: &'a mut crate::mixer::render::slots::LiveSlots,
    /// Dry staging for the click-free bypass crossfades (`crate::bypass`),
    /// pre-allocated for the same reason.
    pub(crate) fx_dry: &'a mut crate::bypass::FxDryScratch,
    /// Where the next playing block should start, so a playhead jump
    /// flushes held voices (code review MIX-06). Audio-thread owned.
    pub(crate) continuity: &'a mut TransportContinuity,
}

impl CallbackScratch<'_> {
    /// Borrow this scratch as one render-core sub-block: the output slice
    /// `out`, the shared buffers (the MIDI stash among them) and the
    /// monitor slice `mon` — disjoint borrows the render core needs at
    /// once.
    ///
    /// Splitting them here (rather than at each call site) is what lets a
    /// seam-crossing callback render two sub-blocks over different slices
    /// of the same scratch without copying anything.
    pub(crate) fn split_block(
        &mut self,
        out: Range<usize>,
        mon: Range<usize>,
    ) -> (BlockScratch<'_>, &[f32]) {
        (
            BlockScratch {
                data: &mut self.data[out],
                bus_bufs: &mut *self.bus_bufs,
                slots: self.track_slots.current(),
                stash: Some(&mut *self.midi_stash),
                port_scratch: &mut *self.port_scratch,
                note_event_buf: &mut *self.note_event_buf,
                sidechain: &mut *self.sidechain,
                fx_dry: &mut *self.fx_dry,
            },
            &self.monitor_temp[mon],
        )
    }
}

/// Tempo and transport snapshot taken once per audio buffer, held while
/// the buffer renders so the bar/beat table stays stable across the
/// per-track render, the plugin transport events and the metronome pass.
///
/// `map` borrows the `ArcSwap` guard the callback holds for the whole
/// block; the engine thread publishes tempo changes wait-free, so a change
/// mid-block simply lands on the next one.
#[derive(Clone, Copy)]
pub(crate) struct BlockTiming<'a> {
    pub(crate) map: &'a TempoMap,
    pub(crate) bpm: f64,
    pub(crate) num: u16,
    pub(crate) metronome: bool,
    /// The transport event latched onto every plugin this block.
    pub(crate) transport: Option<TransportSnap>,
}

impl<'a> BlockTiming<'a> {
    pub(crate) fn new(
        map: &'a TempoMap,
        shared: &SharedState,
        playhead: u64,
        sample_rate: u32,
    ) -> Self {
        Self {
            map,
            bpm: map.bpm as f64,
            num: map.numerator as u16,
            metronome: map.metronome_enabled,
            transport: Some(TransportSnap {
                bpm: map.bpm as f64,
                num: map.numerator as u16,
                den: map.denominator as u16,
                playing: shared.playing.load(Ordering::Relaxed),
                pos_beats: transport_pos_beats(map, playhead, sample_rate),
            }),
        }
    }
}

/// What this callback's read of the monitor ring yielded.
#[derive(Clone, Copy)]
pub(crate) struct MonitorRead {
    /// Whole frames of live input available in `scratch.monitor_temp`.
    pub(crate) frames: usize,
    /// Channels per input frame (0 when no input stream is open).
    pub(crate) input_channels: usize,
    /// Samples per input frame — `input_channels`, floored at 1, so the
    /// whole-frame arithmetic never divides by zero.
    pub(crate) frame_stride: usize,
}
