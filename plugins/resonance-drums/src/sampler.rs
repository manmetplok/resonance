//! Core drum sampler engine: sample loading, voice management, and audio rendering.

use crossbeam_channel::{unbounded, Receiver, Sender, TryRecvError};

use crate::drum_map::{self, NUM_PADS, PAD_MAPPINGS};
use crate::kit::{self, LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer, OVERHEAD_PORT_INDEX};
use crate::params::DrumParams;
use crate::voice::{BalanceSide, Voice, VoiceDestination, VoiceState, MAX_VOICES, RELEASE_SAMPLES};

/// Maximum velocity layer count we track per pad in the round-robin counter
/// array. Drummica's deepest pad has 28 layers, so 32 is a comfortable cap.
pub(crate) const MAX_LAYERS: usize = 32;

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
    /// Receives new kit versions from the loader thread. The audio thread is
    /// the sole consumer; `try_recv` at the top of each process block swaps
    /// in a freshly loaded kit without blocking.
    kit_receiver: Receiver<Vec<LoadedPad>>,
    /// Ships old kits to the janitor thread on swap so the large heap free
    /// happens off the audio thread. The sampler is the sole owner of this
    /// sender; when the sampler drops, the janitor's channel disconnects
    /// and the janitor thread exits cleanly.
    janitor_sender: Sender<Vec<LoadedPad>>,
}

impl DrumSampler {
    pub fn new(kit_receiver: Receiver<Vec<LoadedPad>>) -> Self {
        let (janitor_sender, janitor_receiver) = unbounded::<Vec<LoadedPad>>();
        std::thread::Builder::new()
            .name("resonance-drums-janitor".to_string())
            .spawn(move || {
                // Block on recv; each received Vec is dropped here, off the
                // audio thread. Exits when all senders disconnect.
                while janitor_receiver.recv().is_ok() {}
            })
            .expect("spawn drums janitor thread");

        Self {
            pads: Vec::new(),
            voices: (0..MAX_VOICES).map(|_| Voice::new()).collect(),
            voice_counter: 0,
            rr_counters: [[0; MAX_LAYERS]; NUM_PADS],
            kit_receiver,
            janitor_sender,
        }
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
    /// waiting. Called once per `process()` call from `lib.rs`. Silences all
    /// active voices so no read references the old pad data, then hands the
    /// old `Vec<LoadedPad>` to the janitor thread so the heap free happens
    /// off-audio.
    pub fn try_swap_kit(&mut self) {
        loop {
            match self.kit_receiver.try_recv() {
                Ok(new_pads) => {
                    for voice in &mut self.voices {
                        voice.active = false;
                    }
                    self.rr_counters = [[0; MAX_LAYERS]; NUM_PADS];
                    let old_pads = std::mem::replace(&mut self.pads, new_pads);
                    if let Err(err) = self.janitor_sender.try_send(old_pads) {
                        drop(err.into_inner());
                    }
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    /// Trigger a note-on event. Allocates **one voice per loaded mic bank**
    /// for the matching pad — so a kick hit fires up to 3 voices (KickIn,
    /// KickOut, OH), a tom hit fires 2 (close + OH), and a cymbal hit on
    /// Drummica fires only 1 (the overhead). All voices for a hit share
    /// the same velocity layer, round-robin index, choke group, and age
    /// so they play in lockstep.
    pub fn note_on(&mut self, note: u8, velocity: f32) {
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
        let rr_index = pick_rr(&mut self.rr_counters[pad_index][counter_slot], n_rrs);

        // Single-layer fallback pads bake dynamics into the MIDI velocity;
        // multi-layer kits have the velocity layer already shaped so we
        // use a flat trigger gain.
        let trigger_gain = if n_layers > 1 { 1.0 } else { velocity };
        let choke_group = pad.choke_group;
        let close_mic_count = pad.close_mics.len();
        let output_port = pad.output_group.index() as u8;
        let has_overhead = pad.overhead.is_some();

        // Handle choke groups: release any active voices in the same choke group
        if choke_group.is_some() {
            self.choke_group(choke_group.unwrap());
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
            destinations[dest_count] = Some(VoiceDestination::Overhead);
            dest_count += 1;
        }

        // Allocate one voice per destination. All share pad, note, layer,
        // rr, choke group, and base gain. Age is bumped together so voice
        // stealing treats the set as a single unit.
        self.voice_counter += 1;
        let shared_age = self.voice_counter;
        for i in 0..dest_count {
            let Some(dest) = destinations[i] else { continue };
            let voice_idx = self.find_free_voice(pad_index);
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
        for voice in &mut self.voices {
            if voice.active && voice.note == note && voice.state == VoiceState::Playing {
                voice.trigger_release();
            }
        }
    }

    /// Render `frames` samples into each of the 7 output ports in
    /// `outputs`. Expects `outputs.len() >= NUM_OUTPUT_PORTS` — the
    /// caller in `lib.rs` builds the slice from the plugin's per-port
    /// scratch buffers.
    pub fn render_block(
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

        if self.pads.is_empty() {
            return;
        }

        // Snapshot per-pad params once per block so the inner render loop
        // doesn't re-read atomics for every sample.
        let mut pad_volume = [0.0f32; NUM_PADS];
        let mut pad_pan = [0.0f32; NUM_PADS];
        let mut pad_oh = [0.0f32; NUM_PADS];
        let mut pad_balance = [0.5f32; NUM_PADS];
        for (i, pad) in params.pads.iter().enumerate() {
            pad_volume[i] = if pad.mute.value() { 0.0 } else { pad.volume.value() };
            pad_pan[i] = pad.pan.value();
            pad_oh[i] = pad.oh_blend.value();
            pad_balance[i] = pad.balance.value();
        }

        for voice in &mut self.voices {
            if !voice.active {
                continue;
            }
            let pad_index = voice.pad_index;
            let pad = &self.pads[pad_index];

            // Resolve the voice's source bank from its destination tag.
            let bank: Option<&LoadedMicBank> = match voice.destination {
                VoiceDestination::CloseMic { bank_index, .. } => pad.close_mics.get(bank_index),
                VoiceDestination::Overhead => pad.overhead.as_ref(),
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
            // destination-specific gain multiplier?
            let (port_index, dest_gain) = match voice.destination {
                VoiceDestination::CloseMic {
                    output_port,
                    balance_side,
                    ..
                } => {
                    let gain = match balance_side {
                        BalanceSide::None => 1.0,
                        BalanceSide::Left => 1.0 - pad_balance[pad_index],
                        BalanceSide::Right => pad_balance[pad_index],
                    };
                    (output_port as usize, gain)
                }
                VoiceDestination::Overhead => (OVERHEAD_PORT_INDEX, pad_oh[pad_index]),
            };
            if port_index >= outputs.len() {
                continue;
            }
            let vol = pad_volume[pad_index];
            let (pan_l, pan_r) = resonance_dsp::constant_power_pan(pad_pan[pad_index]);

            // Split-borrow the destination port's buffers so the inner
            // loop can write into both channels cheaply.
            let port = &mut outputs[port_index];
            let port_l = &mut port.left[..frames];
            let port_r = &mut port.right[..frames];

            for frame in 0..frames {
                if voice.position >= sample.frames {
                    voice.active = false;
                    break;
                }
                if voice.state == VoiceState::Releasing
                    && voice.release_pos >= RELEASE_SAMPLES
                {
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

                voice.position += 1;
                if voice.state == VoiceState::Releasing {
                    voice.release_pos += 1;
                }
            }
        }
    }

    /// Release all voices in the given choke group.
    fn choke_group(&mut self, group: u8) {
        for voice in &mut self.voices {
            if voice.active && voice.choke_group == Some(group) {
                voice.trigger_release();
            }
        }
    }

    /// Find the best voice slot to use: free voice > oldest same-pad > oldest overall.
    fn find_free_voice(&self, pad_index: usize) -> usize {
        // Prefer an inactive voice
        if let Some(idx) = self.voices.iter().position(|v| !v.active) {
            return idx;
        }

        // Steal the oldest voice playing the same pad
        if let Some(idx) = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, v)| v.pad_index == pad_index)
            .min_by_key(|(_, v)| v.age)
            .map(|(i, _)| i)
        {
            return idx;
        }

        // Steal the oldest voice overall
        self.voices
            .iter()
            .enumerate()
            .min_by_key(|(_, v)| v.age)
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    /// Kill all active voices immediately.
    pub fn reset(&mut self) {
        for voice in &mut self.voices {
            voice.active = false;
        }
    }
}

// ---------------------------------------------------------------------------
// Pure helpers — kept out of `DrumSampler` so they're unit-testable without
// constructing a full sampler (which spawns a janitor thread).
// ---------------------------------------------------------------------------

/// Map a MIDI velocity in [0, 1] onto a layer index in [0, n_layers).
///
/// Uses equal-width buckets. Callers must guarantee `n_layers >= 1`; with
/// `n_layers == 1` the result is always 0.
pub(crate) fn pick_velocity_layer(velocity: f32, n_layers: usize) -> usize {
    debug_assert!(n_layers >= 1, "n_layers must be at least 1");
    if n_layers <= 1 {
        return 0;
    }
    ((velocity.clamp(0.0, 1.0) * n_layers as f32) as usize).min(n_layers - 1)
}

/// Advance a round-robin counter and return the RR index for this trigger.
/// Wraps the counter at `u32::MAX` so it can run indefinitely.
pub(crate) fn pick_rr(counter: &mut u32, n_rrs: usize) -> usize {
    debug_assert!(n_rrs >= 1, "n_rrs must be at least 1");
    let idx = (*counter as usize) % n_rrs;
    *counter = counter.wrapping_add(1);
    idx
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn velocity_layer_single() {
        assert_eq!(pick_velocity_layer(0.0, 1), 0);
        assert_eq!(pick_velocity_layer(0.5, 1), 0);
        assert_eq!(pick_velocity_layer(1.0, 1), 0);
    }

    #[test]
    fn velocity_layer_two() {
        assert_eq!(pick_velocity_layer(0.0, 2), 0);
        assert_eq!(pick_velocity_layer(0.25, 2), 0);
        assert_eq!(pick_velocity_layer(0.49, 2), 0);
        assert_eq!(pick_velocity_layer(0.5, 2), 1);
        assert_eq!(pick_velocity_layer(0.99, 2), 1);
        assert_eq!(pick_velocity_layer(1.0, 2), 1);
    }

    #[test]
    fn velocity_layer_ten() {
        assert_eq!(pick_velocity_layer(0.0, 10), 0);
        assert_eq!(pick_velocity_layer(0.09, 10), 0);
        assert_eq!(pick_velocity_layer(0.1, 10), 1);
        assert_eq!(pick_velocity_layer(0.5, 10), 5);
        assert_eq!(pick_velocity_layer(0.95, 10), 9);
        assert_eq!(pick_velocity_layer(1.0, 10), 9);
    }

    #[test]
    fn velocity_layer_clamps() {
        // Out-of-range input (shouldn't happen in practice but shouldn't panic).
        assert_eq!(pick_velocity_layer(-1.0, 10), 0);
        assert_eq!(pick_velocity_layer(1.5, 10), 9);
        assert_eq!(pick_velocity_layer(f32::NAN, 10), 0);
    }

    #[test]
    fn velocity_layer_large() {
        // MAX_LAYERS boundary. Every input should still produce a valid index.
        for n in [16usize, 28, 32] {
            for v_pct in 0..=100 {
                let v = v_pct as f32 / 100.0;
                let idx = pick_velocity_layer(v, n);
                assert!(idx < n, "n={n} v={v} idx={idx}");
            }
        }
    }

    #[test]
    fn rr_cycles_round_robin() {
        let mut counter = 0u32;
        let mut picks = Vec::new();
        for _ in 0..9 {
            picks.push(pick_rr(&mut counter, 3));
        }
        assert_eq!(picks, vec![0, 1, 2, 0, 1, 2, 0, 1, 2]);
    }

    #[test]
    fn rr_single_take() {
        let mut counter = 0u32;
        for _ in 0..5 {
            assert_eq!(pick_rr(&mut counter, 1), 0);
        }
    }

    #[test]
    fn rr_counter_wraps() {
        let mut counter = u32::MAX - 1;
        assert_eq!(pick_rr(&mut counter, 3), ((u32::MAX - 1) % 3) as usize);
        assert_eq!(pick_rr(&mut counter, 3), (u32::MAX % 3) as usize);
        // Next call wraps to 0.
        assert_eq!(pick_rr(&mut counter, 3), 0);
    }
}
