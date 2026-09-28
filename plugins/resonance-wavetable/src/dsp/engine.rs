/// Synth engine: voice allocation, rendering, portamento, effects.
use resonance_dsp::SimpleRng;
use resonance_plugin::{Smoother, SmoothingStyle};

use crate::dsp::analog::{AnalogRng, DriftCoeffs};
use crate::dsp::effects::{Chorus, DistortionStage, StereoDelay};
use crate::dsp::filter_models::FilterModel;
use crate::params::WavetableParams;
use crate::viz::{ScopeCollector, WavetableVizState};
use crate::dsp::user_table::UserTable;
use crate::dsp::voice::{Voice, VoiceState, MAX_VOICES};
use crate::dsp::wavetable::{Wavetable, NUM_WAVETABLES, USER_WAVETABLE_INDEX};

/// Oscillators, and so user-table slots.
pub const NUM_OSCS: usize = 2;

/// Depth of the mono held-note stack. More keys than this held at once is
/// not a real playing situation; the oldest simply stop being returned to.
const HELD_NOTES: usize = 16;

/// Fixed seed of the analog-instability PRNG, so a render is reproducible.
const ANALOG_SEED: u32 = 0x9E37_79B9;

pub struct SynthEngine {
    pub(crate) voices: Vec<Voice>,
    /// Indices into `voices` that are not [`VoiceState::Idle`], and how many
    /// of the fixed-size array are live.
    ///
    /// The per-sample render loop used to walk all `MAX_VOICES` slots and
    /// `continue` past the idle ones. At 32 slots × a ~1 kB `Voice` that is
    /// 32 cache lines touched every sample no matter how few notes are
    /// sounding — the dominant cost of an *idle* plugin instance, and pure
    /// overhead for the typical 4–8 voice case. The list is rebuilt at block
    /// start and after every note-on; voices that go idle mid-block simply
    /// hit the existing early-`continue`.
    pub(crate) active: [u8; MAX_VOICES],
    pub(crate) active_len: usize,
    voice_counter: u64,
    pub(crate) sample_rate: f32,

    // Global LFO phases (used when retrigger=false)
    pub global_lfo1: crate::dsp::lfo::MultiLfo,
    pub global_lfo2: crate::dsp::lfo::MultiLfo,
    pub global_lfo3: crate::dsp::lfo::MultiLfo,

    // `ModSource::SampleHold`'s own generator: a global "fifth LFO" shared
    // by every voice (see `dsp::lfo::SampleHoldGen`).
    pub mod_sample_hold: crate::dsp::lfo::SampleHoldGen,

    // Flips ±1.0 on every fresh voice trigger, feeding `ModSource::Alternate`.
    mod_alternate: f32,

    // Wavetable data: the `NUM_WAVETABLES` bundled tables, then one slot per
    // oscillator for its user table. A user slot holds a view of
    // `user_tables[osc]` when one is installed and a copy of bundled table 0
    // otherwise — which is what an oscillator set to the user index plays
    // until a table arrives (or when a project's table could not be found).
    pub wavetables: Vec<Wavetable>,

    /// Storage behind the user slots' views. Only ever replaced through
    /// [`SynthEngine::install_user_table`], which rewrites the matching view
    /// in the same call.
    user_tables: [Option<Box<UserTable>>; NUM_OSCS],

    // Effects
    pub(crate) distortion: DistortionStage,
    /// This sample's `ModDest::DistDrive` offset: the newest sounding
    /// voice's, held while nothing sounds (see `ModState::dist_drive`).
    pub(crate) dist_drive_mod: f32,
    pub(crate) chorus: Chorus,
    pub(crate) delay: StereoDelay,

    // RNG for the LFOs' own S&H *shape* (`LfoShape::SampleAndHold`).
    pub(crate) rng: SimpleRng,

    // Separate RNG for the newer modulation sources (`ModSource::
    // RandomBipolar`/`RandomUnipolar`/`SampleHold`/`Alternate` draws from
    // this, `Alternate` excepted -- it just flips). Kept apart from `rng`
    // above so a note-on drawing a random value cannot shift the sample
    // count the LFOs' own S&H shape consumes from *its* RNG -- that stream
    // is pinned bit-exact by `render_block_regression.rs`. Also kept apart
    // from `analog_rng` below, for the same reason in the other direction:
    // turning `analog` up must not reshuffle a Random/S&H mod source, and a
    // patch using the new mod sources must not reseed the drift.
    pub(crate) mod_rng: SimpleRng,

    // Analog instability: the source of each note-on's voice seed, kept
    // apart from the S&H `rng` so turning `analog` up cannot reshuffle an
    // S&H LFO's sequence. Reseeded on `initialize`/`reset`.
    analog_rng: AnalogRng,
    pub(crate) drift_coeffs: DriftCoeffs,

    // Last note for portamento
    last_note: Option<u8>,

    /// Keys currently held down, oldest first (FU-G2d). In mono, releasing
    /// the sounding key while others are still down returns the voice —
    /// legato — to the most recent of them. Fixed-size so `process()` never
    /// allocates; when full the oldest key is forgotten.
    held: [u8; HELD_NOTES],
    held_len: usize,

    // Audio → UI oscilloscope ring. Filled per-sample in `render_block`,
    // published to the shared viz state at the end of each audio block.
    pub(crate) scope_collector: ScopeCollector,

    // Per-sample de-zipper for the master volume. Lives on the engine (not
    // in the FloatParam) because `Smoother::next()` needs `&mut self` and
    // params sit behind an `Arc`. Retargeted from the param snapshot once
    // per block in `render_block`, per the `Param::set_plain` smoothing
    // contract — host automation lands instantly on the param, and this
    // ramp removes the step before it reaches the output multiply.
    pub(crate) master_vol_smoother: Smoother,

    // Same de-zipper treatment for the FX-chain parameters.
    pub(crate) fx_smoothers: FxSmoothers,

    /// The filter model the last block rendered with. A change clears every
    /// voice's filter state (see `plan_block`), so the newly selected
    /// circuit starts from rest instead of from whatever it held the last
    /// time it ran.
    pub(crate) filter_model: FilterModel,
}

/// Per-sample smoothers for the continuous FX parameters, retargeted from
/// the param snapshot once per block in `render_block` (see
/// `master_vol_smoother` for why these live on the engine and not in the
/// `FloatParam`s). The delay times are smoothed in *samples* — resolved
/// from ms at retarget time — so a time jump glides the read tap along the
/// delay line instead of relocating it discontinuously (a click). Chorus
/// rate needs no smoother: the LFO phase is already continuous across rate
/// changes.
pub(crate) struct FxSmoothers {
    pub dist_drive: Smoother,
    pub dist_mix: Smoother,
    pub chorus_depth: Smoother,
    pub chorus_mix: Smoother,
    pub delay_time_l: Smoother,
    pub delay_time_r: Smoother,
    pub delay_feedback: Smoother,
    pub delay_mix: Smoother,
}

impl FxSmoothers {
    fn new() -> Self {
        Self {
            dist_drive: Smoother::new(SmoothingStyle::Linear(5.0)),
            dist_mix: Smoother::new(SmoothingStyle::Linear(10.0)),
            chorus_depth: Smoother::new(SmoothingStyle::Linear(10.0)),
            chorus_mix: Smoother::new(SmoothingStyle::Linear(10.0)),
            delay_time_l: Smoother::new(SmoothingStyle::Linear(50.0)),
            delay_time_r: Smoother::new(SmoothingStyle::Linear(50.0)),
            delay_feedback: Smoother::new(SmoothingStyle::Linear(10.0)),
            delay_mix: Smoother::new(SmoothingStyle::Linear(10.0)),
        }
    }

    fn set_sample_rate(&mut self, sr: f32) {
        self.dist_drive.set_sample_rate(sr);
        self.dist_mix.set_sample_rate(sr);
        self.chorus_depth.set_sample_rate(sr);
        self.chorus_mix.set_sample_rate(sr);
        self.delay_time_l.set_sample_rate(sr);
        self.delay_time_r.set_sample_rate(sr);
        self.delay_feedback.set_sample_rate(sr);
        self.delay_mix.set_sample_rate(sr);
    }
}

impl SynthEngine {
    pub fn new() -> Self {
        Self {
            voices: Vec::new(),
            active: [0; MAX_VOICES],
            active_len: 0,
            voice_counter: 0,
            sample_rate: 44100.0,
            global_lfo1: crate::dsp::lfo::MultiLfo::new(),
            global_lfo2: crate::dsp::lfo::MultiLfo::new(),
            global_lfo3: crate::dsp::lfo::MultiLfo::new(),
            mod_sample_hold: crate::dsp::lfo::SampleHoldGen::new(),
            // Starts negative so the first voice trigger's flip (see
            // `note_on`) lands on +1.0.
            mod_alternate: -1.0,
            wavetables: Vec::new(),
            user_tables: [None, None],
            distortion: DistortionStage::new(44100.0),
            dist_drive_mod: 0.0,
            chorus: Chorus::new(44100.0),
            delay: StereoDelay::new(44100.0),
            rng: SimpleRng::new(42),
            mod_rng: SimpleRng::new(1337),
            analog_rng: AnalogRng::new(ANALOG_SEED),
            drift_coeffs: DriftCoeffs::for_sample_rate(44100.0),
            last_note: None,
            held: [0; HELD_NOTES],
            held_len: 0,
            scope_collector: ScopeCollector::new(),
            // 5 ms, the master volume's de-zipper time; the param is
            // already linear gain, so the ramp runs in linear-gain space
            // directly. (This used to say it matched a SmoothingStyle
            // declared on the FloatParam — that declaration could never
            // advance and was deleted in ba todo #1288; the ramp here is
            // and always was the real one.)
            master_vol_smoother: Smoother::new(SmoothingStyle::Linear(5.0)),
            fx_smoothers: FxSmoothers::new(),
            filter_model: FilterModel::Clean,
        }
    }

    pub fn initialize(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.voices = (0..MAX_VOICES)
            .map(|_| {
                let mut v = Voice::new();
                v.set_sample_rate(sample_rate);
                v
            })
            .collect();
        self.voice_counter = 0;
        self.last_note = None;
        self.held_len = 0;
        self.analog_rng = AnalogRng::new(ANALOG_SEED);
        self.drift_coeffs = DriftCoeffs::for_sample_rate(sample_rate);

        // Load pre-generated wavetables from the bundled blob. Generation
        // happens once at plugin build time (see `build.rs`), not on every
        // `initialize()` — this keeps plugin instantiation fast instead of
        // burning multi-seconds on additive synthesis.
        self.wavetables = crate::dsp::wavetable::load_bundled();
        // The user slots, sized once here so installing a table later is a
        // slot write — never a push — on the audio thread.
        self.wavetables.reserve_exact(NUM_OSCS);
        for osc in 0..NUM_OSCS {
            let view = self.user_view(osc);
            self.wavetables.push(view);
        }

        // Init effects. The distortion stage designs its oversampling
        // filters here, off the audio thread.
        self.distortion = DistortionStage::new(sample_rate);
        self.dist_drive_mod = 0.0;
        self.chorus = Chorus::new(sample_rate);
        self.delay = StereoDelay::new(sample_rate);

        self.master_vol_smoother.set_sample_rate(sample_rate);
        self.fx_smoothers.set_sample_rate(sample_rate);
    }

    /// Recompute the non-idle voice index list. Called at block start and
    /// after each note-on; O(MAX_VOICES) but once per block, not per sample.
    pub(crate) fn refresh_active(&mut self) {
        let mut n = 0;
        for (i, v) in self.voices.iter().enumerate() {
            if v.state != VoiceState::Idle {
                self.active[n] = i as u8;
                n += 1;
            }
        }
        self.active_len = n;
    }

    /// Install (`Some`) or remove (`None`) oscillator `osc`'s user table,
    /// returning the one it displaces.
    ///
    /// Audio-thread safe: two slot writes, no allocation and no free — the
    /// displaced table is handed back so the caller can retire it off the
    /// audio thread. A sounding voice carries on at its phase into the new
    /// table, exactly as it does when `oscN_wavetable` changes.
    pub fn install_user_table(
        &mut self,
        osc: usize,
        table: Option<Box<UserTable>>,
    ) -> Option<Box<UserTable>> {
        let old = std::mem::replace(&mut self.user_tables[osc], table);
        // Before `initialize()` the slots don't exist yet; it builds them
        // from `user_tables`. The view of `old`'s storage is overwritten
        // before `old` can go anywhere, so no view outlives its table.
        let slot = NUM_WAVETABLES + osc;
        if slot < self.wavetables.len() {
            self.wavetables[slot] = self.user_view(osc);
        }
        old
    }

    /// Oscillator `osc`'s installed user table, if any.
    pub fn user_table(&self, osc: usize) -> Option<&UserTable> {
        self.user_tables[osc].as_deref()
    }

    /// What oscillator `osc`'s user slot reads: its table, or bundled table 0.
    fn user_view(&self, osc: usize) -> Wavetable {
        match &self.user_tables[osc] {
            // SAFETY: the view is only ever stored in the engine's own user
            // slot for `osc`, and `install_user_table` — the only thing that
            // takes the table out of `user_tables[osc]` — rewrites that slot
            // before handing the table back.
            Some(t) => unsafe { t.view() },
            None => self.wavetables[0],
        }
    }

    /// The `wavetables` slot oscillator `osc` reads for an `oscN_wavetable`
    /// value of `index`, or `None` when there is nothing to read (out of
    /// range, or before `initialize()`).
    pub(crate) fn resolve_wavetable(&self, osc: usize, index: usize) -> Option<usize> {
        let slot = if index == USER_WAVETABLE_INDEX {
            NUM_WAVETABLES + osc
        } else if index < NUM_WAVETABLES {
            index
        } else {
            return None;
        };
        (slot < self.wavetables.len()).then_some(slot)
    }

    pub fn reset(&mut self) {
        for v in &mut self.voices {
            v.kill();
        }
        self.active_len = 0;
        self.voice_counter = 0;
        self.last_note = None;
        self.held_len = 0;
        self.analog_rng = AnalogRng::new(ANALOG_SEED);
        self.global_lfo1.reset_phase();
        self.global_lfo2.reset_phase();
        self.global_lfo3.reset_phase();
        self.mod_sample_hold.reset_phase();
        self.mod_alternate = -1.0;
        self.distortion.reset();
        self.dist_drive_mod = 0.0;
        self.chorus.reset();
        self.delay.reset();
    }

    pub fn note_on(&mut self, note: u8, velocity: f32, params: &WavetableParams) {
        self.push_held(note);
        let max_v = params.max_voices.value().max(1) as usize;
        let voice_idx = self.find_free_voice(note, max_v);

        self.voice_counter += 1;
        let glide = params.glide_enabled.value() && self.last_note.is_some();
        let unison_count = params.unison.voices.value().max(1) as usize;
        // Detune width is *not* read here: it comes from the block snapshot
        // at control rate so `ModDest::UnisonDetune` can move it on a
        // sounding voice (ba todo #1323).
        let spread = params.unison.spread.value();

        let voice = &mut self.voices[voice_idx];
        // Mono legato (FU-G2d): a note pressed while the voice's key is
        // still held takes the voice over without restarting it — the
        // envelopes, LFOs, filter state and oscillator phases carry on,
        // and only the pitch moves (gliding if glide is on). A note after
        // the key was released (`Releasing`) is detached and retriggers.
        if max_v == 1 && voice.state == VoiceState::Playing {
            voice.legato(note, self.voice_counter, glide);
            self.last_note = Some(note);
            return;
        }

        // Both are genuine triggers only, like the envelopes above -- a
        // mono legato take-over (handled by the early return) neither
        // redraws the random value nor flips the toggle.
        self.mod_alternate = -self.mod_alternate;
        let random_value = crate::dsp::lfo::random_bipolar(&mut self.mod_rng);

        voice.trigger(
            note,
            velocity,
            self.voice_counter,
            unison_count,
            spread,
            glide,
            // A synced LFO is anchored to the timeline, so a note-on must not
            // reset its phase even if `retrigger` happens to be set.
            params.lfo1.retrigger.value() && !params.lfo1.sync.value(),
            params.lfo2.retrigger.value() && !params.lfo2.sync.value(),
            params.lfo3.retrigger.value() && !params.lfo3.sync.value(),
            random_value,
            self.mod_alternate,
        );
        voice.seed_analog(
            self.analog_rng.next_u32(),
            params.analog.phase_random.value(),
            &self.drift_coeffs,
        );

        self.last_note = Some(note);
    }

    /// A key went up. In mono, if it was the sounding key and other keys
    /// are still held, the voice returns — legato, gliding when glide is
    /// on — to the most recent of them instead of releasing (FU-G2d).
    pub fn note_off(&mut self, note: u8, params: &WavetableParams) {
        self.remove_held(note);
        if params.max_voices.value().max(1) == 1 && self.held_len > 0 {
            let back_to = self.held[self.held_len - 1];
            let glide = params.glide_enabled.value();
            self.voice_counter += 1;
            let age = self.voice_counter;
            let mut returned = false;
            for voice in &mut self.voices {
                if voice.state == VoiceState::Playing && voice.note == note {
                    voice.legato(back_to, age, glide);
                    returned = true;
                }
            }
            if returned {
                self.last_note = Some(back_to);
            }
            return;
        }
        self.release_note(note);
    }

    /// A choke: release the note's voice outright, never returning to an
    /// earlier held key.
    pub fn choke(&mut self, note: u8) {
        self.remove_held(note);
        self.release_note(note);
    }

    fn release_note(&mut self, note: u8) {
        for voice in &mut self.voices {
            if voice.state == VoiceState::Playing && voice.note == note {
                voice.release();
            }
        }
    }

    fn push_held(&mut self, note: u8) {
        self.remove_held(note);
        if self.held_len == HELD_NOTES {
            self.held.copy_within(1.., 0);
            self.held_len -= 1;
        }
        self.held[self.held_len] = note;
        self.held_len += 1;
    }

    fn remove_held(&mut self, note: u8) {
        if let Some(i) = self.held[..self.held_len].iter().position(|&n| n == note) {
            self.held.copy_within(i + 1..self.held_len, i);
            self.held_len -= 1;
        }
    }

    /// `(note, current_pitch)` of every non-idle voice, in slot order.
    ///
    /// A read-only view for tests and diagnostics: it is how the integration
    /// tests observe the voice cap and legato glide without reaching into
    /// crate-private voice state. Not used on the audio path.
    pub fn sounding_voices(&self) -> impl Iterator<Item = (u8, f32)> + '_ {
        self.voices
            .iter()
            .filter(|v| v.state != VoiceState::Idle)
            .map(|v| (v.note, v.current_pitch))
    }

    /// Resolved osc 1 frequency, in Hz, of every sounding unison
    /// sub-voice, in slot order — read back from the `OscSetup` the kernel
    /// is actually using.
    ///
    /// Like [`Self::sounding_voices`], a read-only view for tests and
    /// diagnostics (it is how the analog-drift tests measure pitch without
    /// estimating it from audio). Not used on the audio path.
    pub fn sounding_osc1_freqs(&self) -> impl Iterator<Item = (u8, f32)> + '_ {
        let sr = self.sample_rate as f64;
        self.voices
            .iter()
            .filter(|v| v.state != VoiceState::Idle)
            .flat_map(move |v| {
                v.unison[..v.unison_count]
                    .iter()
                    .map(move |u| (v.note, (u.osc1_setup.phase_inc * sr) as f32))
            })
    }

    /// Publish the latest audio-thread state to the shared viz atomics.
    /// Called once per audio block by the plugin's `process()`.
    pub fn publish_viz(&mut self, params: &WavetableParams, viz: &WavetableVizState) {
        // Flush the oscilloscope buffer.
        self.scope_collector.publish(viz);

        // Pick the representative voice: the newest non-idle one. If nothing
        // is active we leave the scalars at their previous values, which
        // avoids glitchy snap-to-zero between notes.
        let mut rep_idx: Option<usize> = None;
        let mut rep_age: u64 = 0;
        let mut active = 0u32;
        for (i, v) in self.voices.iter().enumerate() {
            if v.state != VoiceState::Idle {
                active += 1;
                if v.age >= rep_age {
                    rep_age = v.age;
                    rep_idx = Some(i);
                }
            }
        }
        viz.store_active_voice_count(active);

        if let Some(i) = rep_idx {
            let voice = &self.voices[i];
            viz.store_env_amp(voice.amp_env.level, voice.amp_env.stage as u32);
            viz.store_env_mod(voice.mod_env.level);
            viz.store_filter_cutoff_live(voice.last_filter_cutoff);
            viz.store_osc_positions(voice.last_osc1_pos, voice.last_osc2_pos);
            for (lfo, phase) in voice.last_lfo_phases.iter().enumerate() {
                viz.store_lfo_phase(lfo, *phase);
            }
        } else {
            // No active voices: reflect the current params where it makes
            // sense so the UI still shows sensible values when idle.
            viz.store_filter_cutoff_live(params.filter.cutoff.value());
            viz.store_osc_positions(params.osc1.position.value(), params.osc2.position.value());
            viz.store_lfo_phase(0, self.global_lfo1.phase);
            viz.store_lfo_phase(1, self.global_lfo2.phase);
            viz.store_lfo_phase(2, self.global_lfo3.phase);
        }
    }

    /// Pick the slot for a new note: idle (while under `max_voices`) >
    /// oldest voice already on this note > oldest releasing > oldest overall.
    ///
    /// Every stealing step only considers *sounding* voices. They used to
    /// search all `MAX_VOICES` slots, and an idle slot keeps a stale age
    /// (usually 0), so it won "oldest" and the new note played on top of the
    /// held ones: the `max_voices` ceiling was never enforced, and in mono
    /// the legato note landed on a fresh voice that could not glide. The
    /// drums plugin fixed the same bug the same way (`janitor.rs`).
    ///
    /// The same-note step comes before the releasing one so a retriggered
    /// note (and every legato note in mono) takes over its own voice rather
    /// than a different, releasing one.
    fn find_free_voice(&self, note: u8, max_voices: usize) -> usize {
        let sounding = || {
            self.voices
                .iter()
                .enumerate()
                .filter(|(_, v)| v.state != VoiceState::Idle)
        };
        let active_count = sounding().count();

        // 1. Prefer an idle voice, as long as we're under the ceiling.
        if active_count < max_voices {
            if let Some(idx) = self.voices.iter().position(|v| v.state == VoiceState::Idle) {
                return idx;
            }
        }

        // 2. Steal the oldest voice already playing this note.
        if let Some((idx, _)) = sounding()
            .filter(|(_, v)| v.note == note)
            .min_by_key(|(_, v)| v.age)
        {
            return idx;
        }

        // 3. Steal the oldest releasing voice.
        if let Some((idx, _)) = sounding()
            .filter(|(_, v)| v.state == VoiceState::Releasing)
            .min_by_key(|(_, v)| v.age)
        {
            return idx;
        }

        // 4. Steal the oldest sounding voice.
        sounding()
            .min_by_key(|(_, v)| v.age)
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
}

impl Default for SynthEngine {
    fn default() -> Self {
        Self::new()
    }
}
