//! Post-processing applied to the rendered mono waveform: AP/SP gating
//! (to turn neural-vocoder hiss into actual silence), peak-aware safety
//! gain, mono→stereo expansion, optional resample to the engine rate,
//! and the public WAV writer.

/// Compute (start_sec, end_sec) intervals for every `AP`/`SP` phoneme
/// in the segment. The segment builder uses voicebank-specific names
/// for silence markers (`AP`, `SP` on TIGER/Lilia; same un-prefixed on
/// Meiji), so a plain string compare works regardless of voicebank.
pub(super) fn collect_ap_intervals(ph_seq: &[String], ph_dur: &[f64]) -> Vec<(f64, f64)> {
    let mut ap_intervals: Vec<(f64, f64)> = Vec::new();
    let mut t_cursor = 0.0_f64;
    for (ph, dur) in ph_seq.iter().zip(ph_dur.iter()) {
        let start = t_cursor;
        t_cursor += *dur;
        if ph == "AP" || ph == "SP" {
            ap_intervals.push((start, t_cursor));
        }
    }
    ap_intervals
}

/// Mute the AP intervals in `mono` and apply a ~5 ms linear fade on
/// each side so the voiced → silence → voiced transition doesn't click.
/// Operates in place at the model's native sample rate (`model_sr`).
///
/// The neural vocoder doesn't produce true silence during AP — it
/// emits a ~-50 dB hiss that the user perceives as constant noise
/// between vocal lines. Hard-muting plus a short fade is what turns
/// the gap into actual silence.
pub(super) fn apply_ap_gate(mono: &mut [f32], ap_intervals: &[(f64, f64)], model_sr: u32) {
    let fade_samples = (model_sr as f64 * 0.005).max(1.0) as usize; // 5 ms
    for (start_sec, end_sec) in ap_intervals {
        let start_idx = (*start_sec * model_sr as f64) as usize;
        let end_idx = (*end_sec * model_sr as f64) as usize;
        if start_idx >= mono.len() {
            continue;
        }
        let end_idx = end_idx.min(mono.len());
        if end_idx <= start_idx {
            continue;
        }
        // Fade out into AP at the start of the interval. We only fade
        // the LAST `fade_samples` of the voiced run preceding this AP,
        // not the AP itself, so that the AP body is fully silent.
        let fade_in_end = start_idx;
        let fade_in_start = fade_in_end.saturating_sub(fade_samples);
        let fade_len = fade_in_end.saturating_sub(fade_in_start);
        for (k, sample_idx) in (fade_in_start..fade_in_end).enumerate() {
            let t = (k + 1) as f32 / (fade_len + 1) as f32;
            mono[sample_idx] *= 1.0 - t;
        }
        // Mute the AP body completely.
        for s in mono[start_idx..end_idx].iter_mut() {
            *s = 0.0;
        }
        // Fade in from AP at the end of the interval. We fade the
        // FIRST `fade_samples` of the voiced run following this AP.
        let fade_out_start = end_idx;
        let fade_out_end = (end_idx + fade_samples).min(mono.len());
        let fade_len = fade_out_end - fade_out_start;
        for (k, sample_idx) in (fade_out_start..fade_out_end).enumerate() {
            let t = (k + 1) as f32 / (fade_len + 1) as f32;
            mono[sample_idx] *= t;
        }
    }
}

/// Peak-aware safety gain factor. Neural vocoders occasionally peak
/// above 1.0 — when that hits the engine's master clamp at [-1, 1]
/// (see `mixer/master.rs:97-105`) it produces audible hard-clip
/// distortion. Returns just enough gain reduction to leave ~1 dB of
/// headroom; quiet renders pass through unchanged (factor 1.0).
pub(super) fn safety_gain_factor(mono: &[f32]) -> f32 {
    let peak = mono.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
    let headroom_target = 0.89; // ≈ -1 dB
    if peak > headroom_target {
        headroom_target / peak
    } else {
        1.0
    }
}

/// Mono → stereo-interleaved expansion with an in-line gain scale.
/// Engine expects stereo-interleaved f32; the vocoder emits mono.
pub(super) fn mono_to_stereo_with_gain(mono: &[f32], gain: f32) -> Vec<f32> {
    let mut stereo = Vec::with_capacity(mono.len() * 2);
    for &s in mono {
        let v = s * gain;
        stereo.push(v);
        stereo.push(v);
    }
    stereo
}

/// If the model's native sample rate differs from the engine's,
/// linear-resample to match. The engine's mixer reads clip samples
/// 1:1 with timeline frames — it does not resample on the fly. If
/// the SVS WAV is 44.1 kHz but the audio device runs at 48 kHz the
/// clip would play back at the wrong speed, sounding pitched-up and
/// distorted. Returns `(samples, final_sample_rate)`.
pub(super) fn resample_to(
    stereo: Vec<f32>,
    model_sr: u32,
    engine_sample_rate: u32,
) -> (Vec<f32>, u32) {
    if model_sr != engine_sample_rate {
        let resampled =
            resonance_audio::decode::linear_resample(&stereo, model_sr, engine_sample_rate);
        (resampled, engine_sample_rate)
    } else {
        (stereo, model_sr)
    }
}

/// Write stereo-interleaved f32 samples to a WAV file compatible with
/// `ClipSource::open_wav`. Delegates to the engine's `transcode_to_wav`
/// so the in-RAM and SVS-rendered code paths share one WAV writer.
pub fn write_stereo_wav(
    path: &std::path::Path,
    samples: &[f32],
    sample_rate: u32,
) -> Result<(), String> {
    resonance_audio::transcode_to_wav(path, samples, sample_rate)
}
