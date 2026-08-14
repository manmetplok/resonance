//! Editor presentation of the DSP state (ba todo #1135, design doc #264
//! req-1) — deliberately *not* part of the render path.
//!
//! Everything here exists so the editor's hero canvas can draw a cloud:
//! the sounding grains are packed into the shared viz atomics, feedback
//! recirculations are *synthesized* as dimmer "ghost" copies that no
//! engine ever rendered (the [`GHOST_GENERATIONS`] /
//! [`GHOST_LEVEL_FLOOR`] / [`GHOST_FEEDBACK_FLOOR`] heuristics below are
//! display-only), and the peak bins are published together with the head
//! bin so the editor can rotate them into oldest → newest order.
//!
//! The DSP core calls this once per block, next to `store_block`, and
//! nothing it computes is ever read back by the render path. It stays
//! allocation-free and lock-free all the same — it runs on the audio
//! thread: fixed stack scratch, one relaxed `u64` store per slot.

use crate::quantize::{quantize_transpose, PitchQuantize};
use crate::viz::{GrainSnapshot, GranularViz, GRAIN_SLOTS, PEAK_BINS};

use super::{BlockParams, GranularDsp};

/// Deepest feedback generation the viz publisher synthesizes ghost
/// entries for (ba todo #1135): each sounding grain spawns ghost
/// snapshots one delay further back per generation, levels scaled by
/// the loop gain per pass, until the slots run out or the ghosts fall
/// below [`GHOST_LEVEL_FLOOR`]. The encoding allows up to 7.
const GHOST_GENERATIONS: u8 = 3;

/// Ghost entries dimmer than this are not published — at low feedback
/// the recirculation is inaudible and the slots are better spent on
/// sounding grains.
const GHOST_LEVEL_FLOOR: f32 = 0.02;

/// Loop gain below which no ghost generations are synthesized at all.
const GHOST_FEEDBACK_FLOOR: f32 = 0.05;

/// Pack the currently sounding grains and the coarse buffer peaks into
/// the shared [`GranularViz`] atomics.
///
/// Published, one entry per *logical* grain (the L/R engines are
/// lock-stepped, so the left engine is authoritative for the audible
/// cloud):
/// 1. the audible cloud, generation 0;
/// 2. the decorrelated right cloud while it sounds (ba todo #1077);
/// 3. PSOLA voices (voiced flag; pitch = the quantized musical
///    transpose, since voices realize it by onset spacing);
/// 4. synthesized feedback-recirculation ghosts on the granulated
///    feedback routes: per generation `g`, each sounding entry
///    re-appears one delay further back with its level scaled by
///    the loop gain per pass, and — with Shimmer engaged — its
///    pitch shifted by `g` more transpositions (the snapshot pitch
///    therefore already includes recirculation transposition).
///
/// If more than [`GRAIN_SLOTS`] entries sound, the first 64 in the
/// order above are kept (stable strategy: audible cloud first).
pub(super) fn publish(
    dsp: &GranularDsp,
    viz: &GranularViz,
    params: &BlockParams,
    feedback_gain: f32,
) {
    let ms_per_sample = 1000.0 / f64::from(dsp.sample_rate);
    let head = dsp.source.write_pos as f64;

    // 1–3: the sounding entries, into fixed stack scratch.
    let mut base = [GrainSnapshot::default(); GRAIN_SLOTS];
    let mut n = 0usize;
    for v in dsp
        .grains
        .engine_l
        .active_grain_views()
        .chain(dsp.grains.engine_r_decor.active_grain_views())
    {
        if n == GRAIN_SLOTS {
            break;
        }
        base[n] = GrainSnapshot {
            position_ms: ((head - v.read_pos) * ms_per_sample) as f32,
            pitch_semitones: (12.0 * v.rate.abs().max(1e-9).log2()) as f32,
            size_ms: (v.dur_samples * ms_per_sample) as f32,
            level: v.level.clamp(0.0, 1.0),
            reversed: v.rate < 0.0,
            voiced: false,
            generation: 0,
        };
        n += 1;
    }
    // PSOLA voices: unity-rate reads — the musical transpose is
    // realized by onset spacing, so publish the quantized semis.
    let mut voice_semis = params.pitch_semitones;
    if params.quantize != PitchQuantize::Off {
        voice_semis = quantize_transpose(voice_semis, params.quantize, params.scale);
    }
    for v in dsp.voice.psola.active_voice_views() {
        if n == GRAIN_SLOTS {
            break;
        }
        base[n] = GrainSnapshot {
            position_ms: ((head - v.read_pos) * ms_per_sample) as f32,
            pitch_semitones: voice_semis,
            size_ms: (v.dur_samples * ms_per_sample) as f32,
            level: v.level.clamp(0.0, 1.0),
            reversed: false,
            voiced: true,
            generation: 0,
        };
        n += 1;
    }
    for (slot, snap) in base[..n].iter().enumerate() {
        viz.store_grain(slot, snap);
    }

    // 4: recirculation ghosts (granulated-feedback routes only —
    // Output-only repeats never re-enter the grain buffer).
    let mut total = n;
    let recirc = params.fb_route.feeds_buffer();
    if recirc && feedback_gain > GHOST_FEEDBACK_FLOOR {
        let delay_ms = (dsp.time.eff_delay * 1000.0) as f32;
        let shimmer_semis = if params.fb_pitch {
            params.pitch_semitones
        } else {
            0.0
        };
        'ghosts: for gen in 1..=GHOST_GENERATIONS {
            let gain = feedback_gain.powi(i32::from(gen));
            for b in &base[..n] {
                if total == GRAIN_SLOTS {
                    break 'ghosts;
                }
                let level = b.level * gain;
                if level < GHOST_LEVEL_FLOOR {
                    continue;
                }
                viz.store_grain(
                    total,
                    &GrainSnapshot {
                        position_ms: b.position_ms + f32::from(gen) * delay_ms,
                        pitch_semitones: b.pitch_semitones + f32::from(gen) * shimmer_semis,
                        level,
                        generation: gen,
                        ..*b
                    },
                );
                total += 1;
            }
        }
    }
    viz.clear_grains_from(total);

    // Coarse backdrop peaks: fixed-position bins + the head bin so
    // the editor can rotate them into oldest → newest order.
    let head_bin =
        ((dsp.source.write_pos >> dsp.source.peak_shift) as usize) & (PEAK_BINS - 1);
    let bin_ms = ((1u64 << dsp.source.peak_shift) as f64 * ms_per_sample) as f32;
    viz.store_peaks(&dsp.source.peak_bins, head_bin, bin_ms);
}
