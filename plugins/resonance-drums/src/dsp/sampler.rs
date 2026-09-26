//! Core drum sampler engine: sample loading, voice management, and audio rendering.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};

use crate::drum_map::{self, NUM_PADS, PAD_MAPPINGS};
use crate::kit::{
    self, LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer, OVERHEAD_PORT_INDEX,
};
use crate::params::DrumParams;
use crate::voice::{BalanceSide, Voice, VoiceDestination, VoiceState, MAX_VOICES, RELEASE_SAMPLES};

use super::janitor;
use super::voice_pick::{
    pick_rr, pick_rr_random, pick_velocity_layer, RoundRobinMode, MAX_LAYERS, NO_LAST_TAKE,
};

/// The global settings a hit is started with, snapshotted once per
/// block from the params (see
/// [`DrumSampler::update_global_settings`]). Held on the sampler so
/// `note_on` stays a two-argument audio-thread call and so a headless
/// caller that never sets them gets exactly the plugin's historical
/// behaviour.
#[derive(Clone, Copy, Debug)]
pub struct GlobalSettings {
    /// Ceiling on simultaneously active voices.
    pub max_voices: usize,
    /// Velocity curve, -1 (hard) … 0 (linear) … +1 (soft).
    pub velocity_curve: f32,
    /// How a layer's takes are walked.
    pub round_robin: RoundRobinMode,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            max_voices: MAX_VOICES,
            velocity_curve: 0.0,
            round_robin: RoundRobinMode::Cycle,
        }
    }
}

/// A note-on at a frame offset inside the block, for
/// [`DrumSampler::render_block`].
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    /// Frame within the block the hit starts at.
    pub frame: usize,
    pub note: u8,
    pub velocity: f32,
}

/// One stereo output buffer pair for a single plugin output port. Callers
/// build a slice of these (one per port) and hand it to `render_frame`.
pub struct PortBuffers<'a> {
    pub left: &'a mut [f32],
    pub right: &'a mut [f32],
}

pub struct DrumSampler {
    pub pads: Vec<LoadedPad>,
    pub voices: Vec<Voice>,
    voice_counter: u64,
    /// Monotonic round-robin counter per (pad, layer). Advanced on each
    /// note_on; indexed modulo the layer's RR count to pick the next take.
    rr_counters: [[u32; MAX_LAYERS]; NUM_PADS],
    /// The take that last fired per (pad, layer), or [`NO_LAST_TAKE`].
    /// Recorded in both round-robin modes so switching to Random never
    /// repeats whatever Cycle just played.
    rr_last: [[u16; MAX_LAYERS]; NUM_PADS],
    /// Xorshift state for the Random round-robin mode. Seeded to a fixed
    /// constant so a render is reproducible: bouncing the same project
    /// twice gives the same takes.
    rr_rng: u32,
    /// Global trigger settings, refreshed once per block from the params.
    globals: GlobalSettings,
    /// Shared display state for the editor: packed `rr_index | (n_rrs << 16)`.
    /// Written after each `note_on`; `None` when running headless / in tests.
    last_rr: Option<Arc<[AtomicU32; NUM_PADS]>>,
    /// Shared OUT meter for the editor: this block's peak across every
    /// output port as `f32::to_bits`, `[left, right]`. Written at the end
    /// of `render_block`; `None` when running headless / in tests.
    out_peak: Option<Arc<[AtomicU32; 2]>>,
    /// Receives new kit versions from the loader thread. The audio thread is
    /// the sole consumer; `try_recv` at the top of each process block swaps
    /// in a freshly loaded kit without blocking.
    kit_receiver: Receiver<Vec<LoadedPad>>,
    /// Ships old kits to the janitor thread on swap so the large heap free
    /// happens off the audio thread. The sampler is the sole owner of this
    /// sender; when the sampler drops, the janitor's channel disconnects
    /// and the janitor thread exits cleanly.
    janitor_sender: Sender<Vec<LoadedPad>>,
    /// The previous kit's pads, kept alive while voices that were still
    /// sounding at swap time fade out against them. Shipped to the
    /// janitor once the last retired voice ends.
    retired_pads: Option<Vec<LoadedPad>>,
    /// Last block's master volume snapshot. Used to interpolate from
    /// the previous block's value to the current one across the block
    /// so automation tweaks don't click. Initialized to 1.0 so the
    /// first block starts at unity gain.
    prev_master_volume: f32,
    /// Last block's per-pad parameter snapshots, mirroring the master
    /// volume ramp: each pad's volume / pan / OH blend / balance is
    /// linearly interpolated from the previous block's value to the
    /// current one across the block so automation jumps don't click.
    /// (`mute` folds into the volume snapshot, so mute toggles ramp
    /// too.) Seeded from the first block's snapshot (`pad_prev_valid`)
    /// so the plugin doesn't ramp from arbitrary defaults on startup.
    prev_pad_volume: [f32; NUM_PADS],
    prev_pad_pan: [f32; NUM_PADS],
    prev_pad_oh: [f32; NUM_PADS],
    prev_pad_balance: [f32; NUM_PADS],
    pad_prev_valid: bool,
    /// This block's per-pad parameter snapshots, taken by
    /// [`DrumSampler::begin_block`] and ramped toward from the `prev_*`
    /// ones by every [`DrumSampler::render_span`] of the block.
    cur_pad_volume: [f32; NUM_PADS],
    cur_pad_pan: [f32; NUM_PADS],
    cur_pad_oh: [f32; NUM_PADS],
    cur_pad_balance: [f32; NUM_PADS],
    /// `1 / frames` for the block in progress (0 for an empty block).
    block_inv_frames: f32,
    /// True when `begin_block` found nothing to render: spans are no-ops
    /// and `end_block` only publishes the (silent) OUT meter.
    block_idle: bool,
}

impl DrumSampler {
    pub fn new(kit_receiver: Receiver<Vec<LoadedPad>>) -> Self {
        let janitor_sender = janitor::spawn();

        Self {
            pads: Vec::new(),
            voices: (0..MAX_VOICES).map(|_| Voice::new()).collect(),
            voice_counter: 0,
            rr_counters: [[0; MAX_LAYERS]; NUM_PADS],
            rr_last: [[NO_LAST_TAKE; MAX_LAYERS]; NUM_PADS],
            rr_rng: 0x9E37_79B9,
            globals: GlobalSettings::default(),
            last_rr: None,
            out_peak: None,
            kit_receiver,
            janitor_sender,
            retired_pads: None,
            prev_master_volume: 1.0,
            prev_pad_volume: [1.0; NUM_PADS],
            prev_pad_pan: [0.0; NUM_PADS],
            prev_pad_oh: [1.0; NUM_PADS],
            prev_pad_balance: [0.5; NUM_PADS],
            pad_prev_valid: false,
            cur_pad_volume: [1.0; NUM_PADS],
            cur_pad_pan: [0.0; NUM_PADS],
            cur_pad_oh: [1.0; NUM_PADS],
            cur_pad_balance: [0.5; NUM_PADS],
            block_inv_frames: 0.0,
            block_idle: true,
        }
    }

    /// Refresh the global trigger settings from the params. Called once
    /// per block from `process()`, before any event is drained, so every
    /// hit in the block is started under the same settings.
    pub fn update_global_settings(&mut self, params: &DrumParams) {
        self.globals = GlobalSettings {
            max_voices: (params.polyphony.value().max(1) as usize).min(MAX_VOICES),
            velocity_curve: params.velocity_curve.value(),
            round_robin: RoundRobinMode::from_param(params.round_robin_mode.value()),
        };
    }

    /// The settings hits are currently started with.
    pub fn global_settings(&self) -> GlobalSettings {
        self.globals
    }

    /// Attach the shared last-RR display array so the editor can show
    /// per-pad round-robin indicators.
    pub fn set_last_rr(&mut self, last_rr: Arc<[AtomicU32; NUM_PADS]>) {
        self.last_rr = Some(last_rr);
    }

    /// Attach the shared OUT meter so the editor's status bar can show the
    /// plugin's real output level instead of a dead bar.
    pub fn set_out_peak(&mut self, out_peak: Arc<[AtomicU32; 2]>) {
        self.out_peak = Some(out_peak);
    }

    /// Bytes of decoded sample data this kit holds, counting every mic
    /// bank, velocity layer and round-robin take. Used for the status
    /// bar's memory readout, which is a measurement of the kit — not a
    /// guess at the process's RSS.
    pub fn total_sample_bytes(&self) -> usize {
        crate::sample_info::total_sample_bytes(&self.pads)
    }

    /// Load the embedded default samples as a single-bank fallback kit.
    /// Called once from `initialize()` so the plugin always boots with
    /// audible sound — even before a real Drummica kit is loaded from
    /// disk. The embedded fallback has no overhead bank, so it renders
    /// into the pad's assigned close-mic output port (or Main for Clap /
    /// Cowbell) with nothing on the Overhead port.
    pub fn load_defaults(&mut self, sample_rate: f32) {
        self.pads.clear();

        for mapping in &PAD_MAPPINGS {
            let sample = match kit::decode_wav(mapping.default_sample, sample_rate) {
                Ok(data) => LoadedSample::from_data(data),
                Err(e) => {
                    eprintln!("Failed to load sample for {}: {}", mapping.name, e);
                    self.pads.push(LoadedPad {
                        name: mapping.name.to_string(),
                        choke_group: mapping.choke_group,
                        output_group: mapping.output_group,
                        close_mics: Vec::new(),
                        overhead: None,
                    });
                    continue;
                }
            };
            self.pads.push(LoadedPad {
                name: mapping.name.to_string(),
                choke_group: mapping.choke_group,
                output_group: mapping.output_group,
                close_mics: vec![LoadedMicBank {
                    position: "fallback".to_string(),
                    setup_key: String::new(),
                    layers: vec![VelocityLayer {
                        round_robins: vec![sample],
                    }],
                }],
                overhead: None,
            });
        }
    }

    /// Audio-thread: check for a freshly loaded kit and swap it in if one is
    /// waiting. Called once per `process()` call from `lib.rs`. Voices that
    /// are still sounding get the standard `RELEASE_SAMPLES` fade instead of
    /// a hard cut; the old `Vec<LoadedPad>` is parked in `retired_pads` so
    /// those voices keep reading valid sample data until the fade ends,
    /// after which `render_block` hands it to the janitor thread so the
    /// heap free happens off-audio.
    pub fn try_swap_kit(&mut self) {
        while let Ok(new_pads) = self.kit_receiver.try_recv() {
            // A second swap while the previous kit's voices are still
            // fading: those voices lose their sample data now, so cut
            // them and retire that kit immediately.
            if let Some(prev_retired) = self.retired_pads.take() {
                for voice in &mut self.voices {
                    if voice.retired {
                        voice.active = false;
                    }
                }
                self.ship_to_janitor(prev_retired);
            }
            let mut any_fading = false;
            for voice in &mut self.voices {
                if voice.active {
                    voice.retired = true;
                    voice.trigger_release();
                    any_fading = true;
                }
            }
            self.rr_counters = [[0; MAX_LAYERS]; NUM_PADS];
            self.rr_last = [[NO_LAST_TAKE; MAX_LAYERS]; NUM_PADS];
            let old_pads = std::mem::replace(&mut self.pads, new_pads);
            if any_fading {
                self.retired_pads = Some(old_pads);
            } else {
                self.ship_to_janitor(old_pads);
            }
        }
    }

    fn ship_to_janitor(&self, pads: Vec<LoadedPad>) {
        if let Err(err) = self.janitor_sender.try_send(pads) {
            drop(err.into_inner());
        }
    }

    /// Trigger a note-on event. Allocates **one voice per loaded mic bank**
    /// for the matching pad — so a kick hit fires up to 3 voices (KickIn,
    /// KickOut, OH), a tom hit fires 2 (close + OH), and a cymbal hit on
    /// Drummica fires only 1 (the overhead). All voices for a hit share
    /// the same velocity layer, round-robin index, choke group, and age
    /// so they play in lockstep.
    ///
    /// The cymbal case is special: with no close bank the overhead take is
    /// the pad's whole sound, so it is summed into the pad's own group
    /// port (Cymbals) rather than the shared Overhead port — otherwise the
    /// Cymbals port never carries a sample.
    ///
    /// The incoming velocity is shaped by the global velocity curve
    /// first, so the curve moves both which layer fires and how hard it
    /// is struck — the two things velocity means here. At the default
    /// (linear) the shaping is an exact identity.
    pub fn note_on(&mut self, note: u8, velocity: f32) {
        let velocity = crate::velocity::shape(velocity, self.globals.velocity_curve);
        let pad_index = match drum_map::pad_index_for_note(note) {
            Some(i) => i,
            None => return,
        };

        if pad_index >= self.pads.len() {
            return;
        }
        let pad = &self.pads[pad_index];

        // Choose a reference bank to drive the velocity layer / round-robin
        // selection. Prefer a close-mic bank (that's where the dynamics
        // tend to live); fall back to overhead. If neither exists the pad
        // is silent and note_on is a no-op.
        let reference_layers: &[VelocityLayer] = if let Some(first) = pad.close_mics.first() {
            &first.layers
        } else if let Some(oh) = &pad.overhead {
            &oh.layers
        } else {
            return;
        };
        if reference_layers.is_empty() {
            return;
        }
        let n_layers = reference_layers.len();
        let layer_index = pick_velocity_layer(velocity, n_layers);
        let layer = &reference_layers[layer_index];
        if layer.round_robins.is_empty() {
            return;
        }
        let counter_slot = layer_index.min(MAX_LAYERS - 1);
        let n_rrs = layer.round_robins.len();
        let rr_index = match self.globals.round_robin {
            RoundRobinMode::Cycle => {
                pick_rr(&mut self.rr_counters[pad_index][counter_slot], n_rrs)
            }
            RoundRobinMode::Random => pick_rr_random(
                &mut self.rr_rng,
                self.rr_last[pad_index][counter_slot],
                n_rrs,
            ),
        };
        // Recorded in both modes: Random must not repeat whatever fired
        // last, however it was chosen.
        self.rr_last[pad_index][counter_slot] = rr_index.min(NO_LAST_TAKE as usize) as u16;

        // Publish the last-played RR for the editor display: both which
        // take fired and how many the layer holds, so the pad can show
        // "take 2 of 3" rather than just "something played".
        if let Some(ref last_rr) = self.last_rr {
            last_rr[pad_index].store(
                crate::rr_display::pack(rr_index, n_rrs),
                Ordering::Relaxed,
            );
        }

        // Single-layer fallback pads bake dynamics into the MIDI velocity;
        // multi-layer kits have the velocity layer already shaped so we
        // use a flat trigger gain.
        let trigger_gain = if n_layers > 1 { 1.0 } else { velocity };
        let choke_group = pad.choke_group;
        let close_mic_count = pad.close_mics.len();
        let output_port = pad.output_group.index() as u8;
        let has_overhead = pad.overhead.is_some();

        // Handle choke groups: release any active voices in the same choke group
        if let Some(group) = choke_group {
            janitor::choke_group(&mut self.voices, group);
        }

        // Build the list of destinations we need to allocate a voice for.
        // Kick + snare: one CloseMic voice per bank (two, with
        // BalanceSide::Left/Right). Tom + hat: one CloseMic voice with
        // BalanceSide::None. Cymbal: no close mic. Plus an Overhead
        // voice if the pad has one loaded.
        let mut destinations: [Option<VoiceDestination>; 3] = [None, None, None];
        let mut dest_count = 0;
        for bank_index in 0..close_mic_count.min(2) {
            let balance_side = match (close_mic_count, bank_index) {
                (2, 0) => BalanceSide::Left,
                (2, 1) => BalanceSide::Right,
                _ => BalanceSide::None,
            };
            destinations[dest_count] = Some(VoiceDestination::CloseMic {
                bank_index,
                output_port,
                balance_side,
            });
            dest_count += 1;
        }
        if has_overhead && dest_count < destinations.len() {
            // Pads the library records with overheads only (every cymbal,
            // ride and china piece in Drummica) have no close bank, so the
            // overhead take is the pad's entire signal. Sending it to the
            // shared Overhead port would leave the pad's own group port —
            // and the sub-track the host derives from it — permanently
            // silent, which is what made the Cymbals sub-track read as
            // "the kit has no cymbals". Route those to the group port.
            let oh_port = if close_mic_count == 0 {
                output_port
            } else {
                OVERHEAD_PORT_INDEX as u8
            };
            destinations[dest_count] = Some(VoiceDestination::Overhead {
                output_port: oh_port,
            });
            dest_count += 1;
        }

        // Allocate one voice per destination. All share pad, note, layer,
        // rr, choke group, and base gain. Age is bumped together so voice
        // stealing treats the set as a single unit.
        self.voice_counter += 1;
        let shared_age = self.voice_counter;
        for dest_slot in destinations.iter().take(dest_count) {
            let Some(dest) = dest_slot else {
                continue;
            };
            let dest = *dest;
            let voice_idx =
                janitor::find_free_voice(&self.voices, pad_index, self.globals.max_voices);
            let voice = &mut self.voices[voice_idx];
            voice.active = true;
            voice.pad_index = pad_index;
            voice.note = note;
            voice.base_gain = trigger_gain;
            voice.destination = dest;
            voice.layer_index = layer_index;
            voice.rr_index = rr_index;
            voice.position = 0;
            voice.choke_group = choke_group;
            voice.retired = false;
            voice.state = VoiceState::Playing;
            voice.release_pos = 0;
            voice.age = shared_age;
        }
    }

    /// Drum samples are one-shots: musical NOTE_OFF is intentionally
    /// ignored so the sample plays through to its natural end regardless
    /// of how short the MIDI note is. Host-level CLAP choke events take
    /// the `choke_note` path instead.
    pub fn note_off(&mut self, _note: u8) {}

    /// Host-level "silence this note now" — used by the CLAP host when
    /// playback stops or a track is muted mid-hit. Fades the matching
    /// voices out rather than clicking them off.
    pub fn choke_note(&mut self, note: u8) {
        janitor::choke_note(&mut self.voices, note);
    }

    /// Render `frames` samples into each of the 7 output ports in
    /// `outputs`, starting each of `hits` at its own frame. Expects
    /// `outputs.len() >= NUM_OUTPUT_PORTS`.
    ///
    /// Voices started before the call (a `note_on` outside any block)
    /// sound from frame 0. Hit offsets follow `process()`'s contract: one
    /// past the block lands on its last frame, and one earlier than the
    /// hit before it lands at the frame already reached. It used to take
    /// no hits at all, so a caller with events inside the block could only
    /// start them at frame 0 (FU-G1); `process()` itself interleaves
    /// chokes and editor auditions too, so it drives
    /// [`begin_block`](Self::begin_block) / [`render_span`](Self::render_span)
    /// / [`end_block`](Self::end_block) directly.
    pub fn render_block(
        &mut self,
        outputs: &mut [PortBuffers<'_>],
        frames: usize,
        params: &DrumParams,
        hits: &[Hit],
    ) {
        self.begin_block(outputs, frames, params);
        let last_frame = frames.saturating_sub(1);
        let mut cursor = 0usize;
        for hit in hits {
            let at = hit.frame.min(last_frame).max(cursor);
            if at > cursor {
                self.render_span(outputs, cursor, at);
                cursor = at;
            }
            self.note_on(hit.note, hit.velocity);
        }
        self.render_span(outputs, cursor, frames);
        self.end_block(outputs, frames, params);
    }

    /// Start a block of `frames` frames: zero every port and snapshot
    /// the per-pad params the block ramps toward. Follow with one or
    /// more [`render_span`](Self::render_span) calls covering
    /// `0..frames` in order, then [`end_block`](Self::end_block).
    pub fn begin_block(
        &mut self,
        outputs: &mut [PortBuffers<'_>],
        frames: usize,
        params: &DrumParams,
    ) {
        // Zero every port for this block before we start summing voices.
        for port in outputs.iter_mut() {
            port.left[..frames].fill(0.0);
            port.right[..frames].fill(0.0);
        }

        self.block_idle = self.pads.is_empty() && self.retired_pads.is_none();
        if self.block_idle {
            return;
        }

        // Snapshot per-pad params once per block so the inner render loop
        // doesn't re-read atomics for every sample. Each param is then
        // linearly ramped from last block's snapshot across this block
        // (same declick scheme as the master volume in `end_block`).
        let mut pad_volume = [0.0f32; NUM_PADS];
        let mut pad_pan = [0.0f32; NUM_PADS];
        let mut pad_oh = [0.0f32; NUM_PADS];
        let mut pad_balance = [0.5f32; NUM_PADS];
        for (i, pad) in params.pads.iter().enumerate() {
            pad_volume[i] = if pad.mute.value() {
                0.0
            } else {
                pad.volume.value()
            };
            pad_pan[i] = pad.pan.value();
            pad_oh[i] = pad.oh_blend.value();
            pad_balance[i] = pad.balance.value();
        }
        if !self.pad_prev_valid {
            // First block ever: start the ramps at the current values
            // so we don't sweep in from arbitrary defaults.
            self.prev_pad_volume = pad_volume;
            self.prev_pad_pan = pad_pan;
            self.prev_pad_oh = pad_oh;
            self.prev_pad_balance = pad_balance;
            self.pad_prev_valid = true;
        }
        self.cur_pad_volume = pad_volume;
        self.cur_pad_pan = pad_pan;
        self.cur_pad_oh = pad_oh;
        self.cur_pad_balance = pad_balance;
        self.block_inv_frames = if frames > 0 {
            1.0 / frames as f32
        } else {
            0.0
        };
    }

    /// Sum every active voice into frames `start..end` of the block begun
    /// by [`begin_block`](Self::begin_block). A voice started between two
    /// spans therefore sounds from the frame the second span starts at,
    /// which is how `process()` honours note-event offsets.
    pub fn render_span(&mut self, outputs: &mut [PortBuffers<'_>], start: usize, end: usize) {
        if self.block_idle || start >= end {
            return;
        }
        let inv_frames = self.block_inv_frames;
        let pad_volume = &self.cur_pad_volume;
        let pad_pan = &self.cur_pad_pan;
        let pad_oh = &self.cur_pad_oh;
        let pad_balance = &self.cur_pad_balance;

        for voice in &mut self.voices {
            if !voice.active {
                continue;
            }
            let pad_index = voice.pad_index;
            // Voices that predate a kit swap fade out against the
            // retired kit's data; everything else reads the current one.
            let pad_source = if voice.retired {
                self.retired_pads.as_deref()
            } else {
                Some(self.pads.as_slice())
            };
            let Some(pad) = pad_source.and_then(|pads| pads.get(pad_index)) else {
                voice.active = false;
                continue;
            };

            // Resolve the voice's source bank from its destination tag.
            let bank: Option<&LoadedMicBank> = match voice.destination {
                VoiceDestination::CloseMic { bank_index, .. } => pad.close_mics.get(bank_index),
                VoiceDestination::Overhead { .. } => pad.overhead.as_ref(),
            };
            let Some(bank) = bank else {
                voice.active = false;
                continue;
            };
            if voice.layer_index >= bank.layers.len() {
                voice.active = false;
                continue;
            }
            let layer = &bank.layers[voice.layer_index];
            if voice.rr_index >= layer.round_robins.len() {
                voice.active = false;
                continue;
            }
            let sample = &layer.round_robins[voice.rr_index];

            // Which port does this voice sum into, and what's the
            // destination-specific gain multiplier? Computed at both
            // the previous and current block's param snapshots so the
            // inner loop can ramp between them.
            let (port_index, dest_gain0, dest_gain1) = match voice.destination {
                VoiceDestination::CloseMic {
                    output_port,
                    balance_side,
                    ..
                } => {
                    let (g0, g1) = match balance_side {
                        BalanceSide::None => (1.0, 1.0),
                        BalanceSide::Left => (
                            1.0 - self.prev_pad_balance[pad_index],
                            1.0 - pad_balance[pad_index],
                        ),
                        BalanceSide::Right => {
                            (self.prev_pad_balance[pad_index], pad_balance[pad_index])
                        }
                    };
                    (output_port as usize, g0, g1)
                }
                VoiceDestination::Overhead { output_port } => (
                    output_port as usize,
                    self.prev_pad_oh[pad_index],
                    pad_oh[pad_index],
                ),
            };
            if port_index >= outputs.len() {
                continue;
            }
            let vol0 = self.prev_pad_volume[pad_index];
            let vol1 = pad_volume[pad_index];
            let (pan_l0, pan_r0) =
                resonance_dsp::stereo_balance(self.prev_pad_pan[pad_index]);
            let (pan_l1, pan_r1) = resonance_dsp::stereo_balance(pad_pan[pad_index]);

            // Per-sample ramp increments, mirroring the master volume
            // ramp below: start at the previous block's value and step
            // toward the current one across the block. Pan and balance
            // ramp in gain space, which keeps the path continuous (and
            // linear in the pan position, since stereo_balance is
            // piecewise-linear). A span that starts mid-block picks the
            // ramps up where they stand at its first frame (exactly the
            // start values when it starts at 0).
            let vol_step = (vol1 - vol0) * inv_frames;
            let dest_step = (dest_gain1 - dest_gain0) * inv_frames;
            let pan_l_step = (pan_l1 - pan_l0) * inv_frames;
            let pan_r_step = (pan_r1 - pan_r0) * inv_frames;
            let at = start as f32;
            let mut vol = vol0 + vol_step * at;
            let mut dest_gain = dest_gain0 + dest_step * at;
            let mut pan_l = pan_l0 + pan_l_step * at;
            let mut pan_r = pan_r0 + pan_r_step * at;

            // Split-borrow the destination port's buffers so the inner
            // loop can write into both channels cheaply.
            let port = &mut outputs[port_index];
            let port_l = &mut port.left[..end];
            let port_r = &mut port.right[..end];

            for frame in start..end {
                if voice.position >= sample.frames {
                    voice.active = false;
                    break;
                }
                if voice.state == VoiceState::Releasing && voice.release_pos >= RELEASE_SAMPLES {
                    voice.active = false;
                    break;
                }

                let idx = voice.position * 2;
                let sample_l = sample.data[idx];
                let sample_r = sample.data[idx + 1];
                let env = voice.current_gain();
                let gain = env * vol * dest_gain;

                port_l[frame] += sample_l * gain * pan_l;
                port_r[frame] += sample_r * gain * pan_r;

                vol += vol_step;
                dest_gain += dest_step;
                pan_l += pan_l_step;
                pan_r += pan_r_step;

                voice.position += 1;
                if voice.state == VoiceState::Releasing {
                    voice.release_pos += 1;
                }
            }
        }
    }

    /// Finish the block begun by [`begin_block`](Self::begin_block):
    /// retire a drained kit, roll the param snapshots forward, apply the
    /// master volume ramp and publish the OUT meter.
    pub fn end_block(
        &mut self,
        outputs: &mut [PortBuffers<'_>],
        frames: usize,
        params: &DrumParams,
    ) {
        if self.block_idle {
            // Nothing to render — the ports are silent, and the OUT meter
            // must say so rather than hold its last value.
            self.publish_out_peak(outputs, frames);
            return;
        }

        // Once the last fading pre-swap voice has ended, the retired
        // kit's samples are unreferenced: hand them to the janitor.
        if self.retired_pads.is_some() && !self.voices.iter().any(|v| v.active && v.retired) {
            if let Some(retired) = self.retired_pads.take() {
                self.ship_to_janitor(retired);
            }
        }

        // Next block ramps from this block's snapshots.
        self.prev_pad_volume = self.cur_pad_volume;
        self.prev_pad_pan = self.cur_pad_pan;
        self.prev_pad_oh = self.cur_pad_oh;
        self.prev_pad_balance = self.cur_pad_balance;

        // Apply master volume in-place over every port. Linearly
        // interpolate from the previous block's value to the current
        // one across the block so automation tweaks and user fader
        // moves don't click. With small block sizes (≤512 frames at
        // typical SR) per-sample lerp is essentially free.
        let master_vol = params.master_volume.value();
        let prev = self.prev_master_volume;
        if (prev - 1.0).abs() > f32::EPSILON
            || (master_vol - 1.0).abs() > f32::EPSILON
            || (master_vol - prev).abs() > f32::EPSILON
        {
            let step = if frames > 0 {
                (master_vol - prev) / frames as f32
            } else {
                0.0
            };
            for port in outputs.iter_mut() {
                let mut g = prev;
                for s in port.left[..frames].iter_mut() {
                    *s *= g;
                    g += step;
                }
                let mut g = prev;
                for s in port.right[..frames].iter_mut() {
                    *s *= g;
                    g += step;
                }
            }
        }
        self.prev_master_volume = master_vol;

        self.publish_out_peak(outputs, frames);
    }

    /// Publish this block's peak level across every output port for the
    /// editor's OUT meter. Written every block (including silent ones) so
    /// the meter falls back to −∞ instead of freezing at the last hit.
    fn publish_out_peak(&self, outputs: &[PortBuffers<'_>], frames: usize) {
        let Some(ref out_peak) = self.out_peak else {
            return;
        };
        let mut peak_l = 0.0f32;
        let mut peak_r = 0.0f32;
        for port in outputs.iter() {
            for s in port.left[..frames].iter() {
                peak_l = peak_l.max(s.abs());
            }
            for s in port.right[..frames].iter() {
                peak_r = peak_r.max(s.abs());
            }
        }
        out_peak[0].store(peak_l.to_bits(), Ordering::Relaxed);
        out_peak[1].store(peak_r.to_bits(), Ordering::Relaxed);
    }

    /// Kill all active voices immediately.
    pub fn reset(&mut self) {
        janitor::reset_all(&mut self.voices);
    }
}
