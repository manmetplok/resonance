//! Block-rate audio rendering for the wavetable engine.
//!
//! The hot path snapshots every atomic parameter once at block start into
//! [`ParamSnapshot`], resolves the block-constant [`BlockPlan`] from it, then
//! runs a tight per-sample loop over those locals. This avoids the
//! multi-million atomic loads per second a naive render-per-frame design
//! would otherwise perform, and keeps the per-sample kernel free of
//! `Param::value()` calls entirely.
//!
//! Filter coefficients are refreshed at control rate (every
//! [`FILTER_COEFF_INTERVAL`] samples) since the filter's modulation sources
//! -- LFOs, envelopes, key tracking -- are all sub-audio-rate. Freshly
//! triggered voices force an immediate coefficient refresh via
//! `Voice::filter_dirty`.
//!
//! The one exception is filter FM: a voice whose FM depth is non-zero also
//! re-derives its cutoff gain every sample from oscillator 2, through the
//! cheap `tan`/`exp2` approximations in
//! [`filter_models`](crate::dsp::filter_models). Depth zero — the default —
//! skips that path entirely.
//!
//! The block is rendered in four phases, one module each:
//!
//! * [`snapshot`] — read every parameter once ([`ParamSnapshot`]).
//! * [`plan`] — derive the block constants and push block-rate state into the
//!   engine ([`BlockPlan`]).
//! * [`kernel`] — the per-voice, per-sample work, including the marked
//!   oscillator kernel.
//! * this module — the sample loop that drives them, the event drain, the
//!   global LFOs, the effects chain and the post-block bookkeeping.
//!
//! Nothing below `render_block` allocates, locks, or dispatches virtually.

mod kernel;
mod plan;
mod snapshot;

use resonance_plugin::{EventIterator, NoteEvent, TempoInfo};

use crate::dsp::analog::DRIFT_INTERVAL;
use crate::dsp::engine::SynthEngine;
use crate::dsp::lfo::LfoMode;
use crate::dsp::voice::VoiceState;
use crate::params::WavetableParams;

use self::kernel::SampleCtx;
use self::plan::BlockPlan;
use self::snapshot::ParamSnapshot;

/// Update filter coefficients every N samples. `tan()` and the three SVF
/// coefficient divides are the bulk of per-voice filter CPU, and modulation
/// sources top out well below sample rate / this interval (~3 kHz at 48 kHz),
/// so stair-stepping here is acoustically transparent for any realistic LFO
/// or envelope sweep.
///
/// Must stay a power of two: the control-rate test compiles to an AND rather
/// than a divmod.
const FILTER_COEFF_INTERVAL: u32 = 16;

/// How far a full-scale (±1.0) `ModDest::DistDrive` modulation moves the
/// master drive, in octaves: log2(20), so +1.0 takes the minimum drive of 1
/// to the maximum of 20. The offset is exponential because drive is a gain
/// — equal steps of modulation should sound like equal steps of dirt.
const DIST_DRIVE_MOD_OCTAVES: f32 = 4.321_928;

impl SynthEngine {
    /// Render a full stereo block into `left` / `right`, draining MIDI events
    /// with sample-accurate timing. Replaces the old `render_frame`-per-sample
    /// entry point; all atomic parameter loads happen once up front rather
    /// than per sample.
    ///
    /// `tempo` is the host's transport snapshot for this block. It drives
    /// tempo-synced LFOs; `None` (an offline render, a host with no
    /// transport) makes them free-run at the equivalent rate for
    /// [`FALLBACK_BPM`](crate::dsp::lfo::FALLBACK_BPM).
    pub fn render_block(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        frames: usize,
        params: &WavetableParams,
        events: &mut EventIterator<'_>,
        tempo: Option<TempoInfo>,
    ) {
        let snap = ParamSnapshot::capture(params, self.sample_rate);
        self.retarget_smoothers(&snap);
        self.distortion.configure(snap.dist_settings);
        let plan = self.plan_block(&snap, tempo);

        let mut next_event = events.next_event();

        for sample_id in 0..frames {
            let triggered_here =
                self.drain_events(sample_id, &plan, params, events, &mut next_event);
            let ctx = self.advance_global_lfos(&snap, &plan, sample_id, triggered_here);

            let (mix_l, mix_r) = self.mix_voices(&snap, &plan, &ctx);
            let (mix_l, mix_r) = self.apply_block_effects(&snap, mix_l, mix_r);

            let master_vol = self.master_vol_smoother.next();
            let out_l = mix_l * master_vol;
            let out_r = mix_r * master_vol;

            left[sample_id] = out_l;
            right[sample_id] = out_r;

            self.scope_collector.push(out_l, out_r);
        }

        self.finish_block(&snap, frames);
    }

    /// Retarget the per-sample de-zippers once per block, so host automation
    /// of these parameters ramps instead of stepping mid-buffer. Delay times
    /// are resolved from ms to samples here so the smoother glides the tap
    /// position itself.
    fn retarget_smoothers(&mut self, snap: &ParamSnapshot) {
        self.master_vol_smoother.set_target(snap.master_vol);

        let ms_to_samples = 0.001 * self.sample_rate;
        let fx = &mut self.fx_smoothers;
        fx.dist_drive.set_target(snap.dist_drive);
        fx.dist_mix.set_target(snap.dist_mix);
        fx.chorus_depth.set_target(snap.chorus_depth);
        fx.chorus_mix.set_target(snap.chorus_mix);
        fx.delay_time_l.set_target(snap.delay_time_l * ms_to_samples);
        fx.delay_time_r.set_target(snap.delay_time_r * ms_to_samples);
        fx.delay_feedback.set_target(snap.delay_feedback);
        fx.delay_mix.set_target(snap.delay_mix);
    }

    /// Consume every event whose timing landed on `sample_id`, applying it to
    /// voice state. Returns true when a note-on triggered a voice here — a
    /// freshly triggered voice carries `mod_dirty`, which forces an off-grid
    /// modulation-matrix evaluation, and that evaluation needs live LFO
    /// values.
    ///
    /// Note events mutate voice state but never parameters, so the block's
    /// [`ParamSnapshot`] stays valid across this call.
    #[inline]
    fn drain_events(
        &mut self,
        sample_id: usize,
        plan: &BlockPlan,
        params: &WavetableParams,
        events: &mut EventIterator<'_>,
        next_event: &mut Option<NoteEvent>,
    ) -> bool {
        let mut triggered_here = false;
        while let Some(ref event) = *next_event {
            if event.timing() > sample_id as u32 {
                break;
            }
            match event {
                NoteEvent::NoteOn { note, velocity, .. } => {
                    self.note_on(*note, *velocity, params);
                    triggered_here = true;
                    // The freshly triggered voice also needs its LFO rates
                    // seeded for this block. (`trigger()` already marks its
                    // `OscSetup` cache dirty, and the snapshot has not moved
                    // mid-block, so the other caches stay valid.)
                    self.seed_voice_lfo_rates(plan.lfo_rates, false);
                    self.refresh_active();
                }
                NoteEvent::NoteOff { note, .. } => self.note_off(*note, params),
                NoteEvent::Choke { note, .. } => self.choke(*note),
            }
            *next_event = events.next_event();
        }
        triggered_here
    }

    /// Evaluate the free-running LFOs for this sample and advance their
    /// phases, returning the per-sample context the voice kernel reads.
    ///
    /// LFO *values* feed only the modulation matrix, which runs at control
    /// rate (plus the sample a voice is triggered on). LFO *phases* still
    /// advance every sample. Evaluating the shape is a `sin()` for the
    /// default sine LFO, so gating it here removes three transcendental calls
    /// per sample from the engine.
    #[inline]
    fn advance_global_lfos(
        &mut self,
        snap: &ParamSnapshot,
        plan: &BlockPlan,
        sample_id: usize,
        triggered_here: bool,
    ) -> SampleCtx {
        let coeff_tick = (sample_id as u32 & (FILTER_COEFF_INTERVAL - 1)) == 0;
        let drift_tick = (sample_id as u32 & (DRIFT_INTERVAL - 1)) == 0;
        let lfo_vals_needed = coeff_tick || triggered_here;

        let global_lfo = if lfo_vals_needed {
            [
                self.global_lfo1.value(snap.lfo1_shape) * snap.lfo1_depth,
                self.global_lfo2.value(snap.lfo2_shape) * snap.lfo2_depth,
                self.global_lfo3.value(snap.lfo3_shape) * snap.lfo3_depth,
            ]
        } else {
            [0.0, 0.0, 0.0]
        };
        // Same gate as the three LFOs above: the mod matrix (the only
        // consumer) only ever reads this on a sample where `lfo_vals_needed`
        // holds, so there is nothing to gain from computing it otherwise --
        // and `sample_hold_val` mirrors `global_lfo` in zeroing when unread.
        let sample_hold_val = if lfo_vals_needed {
            self.mod_sample_hold.value()
        } else {
            0.0
        };
        self.global_lfo1.advance(snap.lfo1_shape, &mut self.rng);
        self.global_lfo2.advance(snap.lfo2_shape, &mut self.rng);
        self.global_lfo3.advance(snap.lfo3_shape, &mut self.rng);
        // Draws from `mod_rng`, never `rng` -- see the field comment on
        // `SynthEngine::mod_rng`.
        self.mod_sample_hold
            .advance(&mut self.mod_rng, plan.sh_slew_coeff);

        SampleCtx {
            coeff_tick,
            drift_tick,
            lfo_vals_needed,
            global_lfo,
            sample_hold_val,
        }
    }

    /// Sum every live voice's contribution to this sample.
    ///
    /// Walks the active-voice index list refreshed at block start and on every
    /// note-on, rather than all `MAX_VOICES` slots. A voice that drains to
    /// Idle mid-block stays in the list and is skipped by the kernel.
    #[inline]
    fn mix_voices(
        &mut self,
        snap: &ParamSnapshot,
        plan: &BlockPlan,
        ctx: &SampleCtx,
    ) -> (f32, f32) {
        // Disjoint field borrows: the kernel needs `&mut Voice` alongside the
        // shared wavetable set and the shared RNG.
        let Self {
            voices,
            wavetables,
            rng,
            active,
            active_len,
            dist_drive_mod,
            ..
        } = self;

        let mut mix_l = 0.0f32;
        let mut mix_r = 0.0f32;
        let mut newest_age = 0u64;
        for &vi in active.iter().take(*active_len) {
            let voice = &mut voices[vi as usize];
            if let Some((l, r)) = kernel::render_voice(voice, snap, plan, wavetables, rng, ctx) {
                mix_l += l;
                mix_r += r;
                // The master drive is global: take the newest sounding
                // voice's modulation for it (last-note priority). Skipped
                // outright when nothing routes there.
                if snap.dist_drive_routed && voice.age >= newest_age {
                    newest_age = voice.age;
                    *dist_drive_mod = voice.cached_mods.dist_drive;
                }
            }
        }
        (mix_l, mix_r)
    }

    /// Run the master effects chain for one sample. Continuous FX parameters
    /// come from the per-sample smoothers; the enable flags are constant for
    /// the whole block so these branches predict perfectly.
    #[inline]
    fn apply_block_effects(&mut self, snap: &ParamSnapshot, l: f32, r: f32) -> (f32, f32) {
        let (mut mix_l, mut mix_r) = (l, r);

        if snap.dist_enabled {
            let mut drive = self.fx_smoothers.dist_drive.next();
            if snap.dist_drive_routed {
                drive = (drive * (self.dist_drive_mod * DIST_DRIVE_MOD_OCTAVES).exp2())
                    .clamp(1.0, 20.0);
            }
            let (dl, dr) = self.distortion.process(
                mix_l,
                mix_r,
                drive,
                self.fx_smoothers.dist_mix.next(),
            );
            mix_l = dl;
            mix_r = dr;
        }

        if snap.chorus_enabled {
            let (cl, cr) = self.chorus.process_mode(
                mix_l,
                mix_r,
                snap.chorus_mode,
                snap.chorus_rate,
                self.fx_smoothers.chorus_depth.next(),
                snap.chorus_noise,
                self.fx_smoothers.chorus_mix.next(),
            );
            mix_l = cl;
            mix_r = cr;
        }

        if snap.delay_enabled {
            let (dl, dr) = self.delay.process(
                mix_l,
                mix_r,
                self.fx_smoothers.delay_time_l.next(),
                self.fx_smoothers.delay_time_r.next(),
                self.fx_smoothers.delay_feedback.next(),
                self.fx_smoothers.delay_mix.next(),
            );
            mix_l = dl;
            mix_r = dr;
        }

        (mix_l, mix_r)
    }

    /// Post-block bookkeeping: the viz snapshot and the smoothers belonging to
    /// disabled effects.
    fn finish_block(&mut self, snap: &ParamSnapshot, frames: usize) {
        self.publish_voice_viz(snap);
        self.skip_disabled_fx_smoothers(snap, frames as u32);
    }

    /// Write the per-voice fields that only `publish_viz` reads, once per
    /// block.
    ///
    /// These are read once per block and only for voices that are still
    /// non-idle — so writing them per sample inside the voice loop (as an
    /// earlier version did) burned ~10 ops per voice per sample to produce
    /// 127 values nobody ever looked at. The values written here are exactly
    /// what the final sample of the loop would have left behind.
    fn publish_voice_viz(&mut self, snap: &ParamSnapshot) {
        let (g1, g2, g3) = (
            self.global_lfo1.phase,
            self.global_lfo2.phase,
            self.global_lfo3.phase,
        );
        for voice in &mut self.voices {
            if voice.state == VoiceState::Idle {
                continue;
            }
            let mods = voice.cached_mods;
            voice.last_osc1_pos = (snap.osc1_pos + mods.osc1_position).clamp(0.0, 1.0);
            voice.last_osc2_pos = (snap.osc2_pos + mods.osc2_position).clamp(0.0, 1.0);
            // Only a retriggered LFO has a per-voice phase; free and synced
            // both read the engine-wide one.
            voice.last_lfo_phases[0] = match snap.lfo1_mode {
                LfoMode::Retrig => voice.lfo1.phase,
                _ => g1,
            };
            voice.last_lfo_phases[1] = match snap.lfo2_mode {
                LfoMode::Retrig => voice.lfo2.phase,
                _ => g2,
            };
            voice.last_lfo_phases[2] = match snap.lfo3_mode {
                LfoMode::Retrig => voice.lfo3.phase,
                _ => g3,
            };
        }
    }

    /// Disabled effects never consume their smoothers inside the sample loop;
    /// fast-forward them so re-enabling doesn't replay a stale ramp.
    fn skip_disabled_fx_smoothers(&mut self, snap: &ParamSnapshot, n: u32) {
        if !snap.dist_enabled {
            self.fx_smoothers.dist_drive.skip(n);
            self.fx_smoothers.dist_mix.skip(n);
        }
        if !snap.chorus_enabled {
            self.fx_smoothers.chorus_depth.skip(n);
            self.fx_smoothers.chorus_mix.skip(n);
            // Nothing is heard from a disabled chorus, so a mode change
            // made while it is off lands without the switch crossfade.
            self.chorus.set_mode_immediate(snap.chorus_mode);
        }
        if !snap.delay_enabled {
            self.fx_smoothers.delay_time_l.skip(n);
            self.fx_smoothers.delay_time_r.skip(n);
            self.fx_smoothers.delay_feedback.skip(n);
            self.fx_smoothers.delay_mix.skip(n);
        }
    }
}
