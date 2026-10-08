//! Re-time and re-pitch a render unit with the voicebank's own duration,
//! pitch and variance models (`bank_timing` / `bank_pitch`), on the render
//! thread, just before the acoustic model runs. Planning never gets here:
//! the unit's keys hash what these models are fed, not what they return
//! (see `render_cache::content_key`).

use resonance_svs::ds::{DsSegment, SampleCurve};
use resonance_svs::predict::{midi_to_hz, BankModels, PredictOptions, Prediction};
use resonance_svs::stages::common::ExecutionProvider;

use super::paths::CurveKind;
use super::registry::BankProfile;
use super::segment::CURVE_FRAME_SEC;
use super::RenderUnit;

/// What to run for `unit`: the models it was planned with.
pub(super) fn options_for(unit: &RenderUnit) -> PredictOptions {
    PredictOptions {
        timing: unit.bank_models.duration.is_some(),
        pitch: unit.bank_models.pitch.is_some(),
        // `RESONANCE_BANK_VARIANCE=0` skips the variance model, for an A/B.
        variance: unit.bank_models.variance.is_some()
            && std::env::var("RESONANCE_BANK_VARIANCE").map_or(true, |v| v != "0"),
        expr: bank_expr(),
        ..PredictOptions::default()
    }
}

/// The seed of every model's noise for `unit` ([`RenderUnit::seed`]), so
/// the same unit renders the same samples whenever and wherever it renders.
/// `RESONANCE_SVS_SEED_SALT=<n>` mixes a number in, to measure how much a
/// take depends on its noise (the audition's spread across seeds).
pub(super) fn unit_seed(unit: &RenderUnit) -> u64 {
    let salt = std::env::var("RESONANCE_SVS_SEED_SALT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    unit.seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// Whether `unit` uses any of the bank's models.
pub(super) fn uses_bank_models(unit: &RenderUnit) -> bool {
    let m = &unit.bank_models;
    m.duration.is_some() || m.pitch.is_some()
}

/// The pitch model's expressiveness. 1 (as trained) unless
/// `RESONANCE_BANK_EXPR` overrides it for an A/B listen.
fn bank_expr() -> f32 {
    std::env::var("RESONANCE_BANK_EXPR")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .map_or(1.0, |v| v.clamp(0.0, 1.0))
}

/// Load the models `unit` asks for (once per render; every unit of a lane
/// asks for the same ones).
pub(super) fn load_models(unit: &RenderUnit) -> BankModels {
    BankModels::load(&unit.bank_models, &options_for(unit), ExecutionProvider::Cpu)
}

/// `unit`'s segment with the bank models' timing, pitch and tension.
/// `speaker` is the acoustic speaker key the lane sings with. Whatever a
/// model could not provide stays as the rules built it; when neither the
/// duration nor the pitch model ran, the rule segment comes back untouched.
pub(super) fn refine_segment(
    unit: &RenderUnit,
    models: &mut BankModels,
    speaker: Option<&str>,
    bank: &BankProfile,
) -> DsSegment {
    let opts = options_for(unit);
    let bend = |t: f64| sample_frames(&unit.bend_cents, t / CURVE_FRAME_SEC);
    let pred = models.predict(&unit.score, speaker, unit_seed(unit), &opts, &bend);
    let seg = apply_prediction(unit, &pred, bank);
    dump(unit, &pred, &seg);
    seg
}

/// With `RESONANCE_BANK_DUMP=<dir>`, write what the bank models were fed
/// and returned for `unit` to `<dir>/<key>.json` — for checking alignment
/// and pitch by measurement.
fn dump(unit: &RenderUnit, pred: &Prediction, seg: &DsSegment) {
    let Some(dir) = std::env::var_os("RESONANCE_BANK_DUMP").filter(|v| !v.is_empty()) else {
        return;
    };
    let score = &unit.score;
    let json = serde_json::json!({
        "key": format!("{:016x}", unit.key),
        "phones": score.phones.iter().map(|p| serde_json::json!({
            "symbol": p.symbol, "group": p.group, "rule_sec": p.rule_sec,
        })).collect::<Vec<_>>(),
        "groups": score.groups.iter().map(|g| serde_json::json!({
            "start_sec": g.start_sec, "midi": g.midi,
        })).collect::<Vec<_>>(),
        "notes": score.notes.iter().map(|n| serde_json::json!({
            "start_sec": n.start_sec, "end_sec": n.end_sec, "midi": n.midi,
            "slur": n.slur, "voiced": n.voiced,
        })).collect::<Vec<_>>(),
        "total_sec": score.total_sec,
        "timed_by_model": pred.timed_by_model,
        "ph_dur": pred.ph_dur,
        "frame_sec": pred.frame_sec,
        "pitch": pred.pitch,
        "tension_pred": pred.variance.tension,
        "tension": seg.tension.samples,
        "tension_timestep": seg.tension.timestep,
        "ph_seq": seg.ph_seq,
    });
    let path = std::path::Path::new(&dir).join(format!("{:016x}.json", unit.key));
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, json.to_string())) {
        tracing::warn!("RESONANCE_BANK_DUMP: {e}");
    }
}

/// Lay `pred` over `unit`'s rule segment.
pub(super) fn apply_prediction(unit: &RenderUnit, pred: &Prediction, bank: &BankProfile) -> DsSegment {
    let rule = &unit.segment;
    if !pred.timed_by_model && pred.pitch.is_none() {
        return rule.clone();
    }
    let score = &unit.score;
    let mut seg = rule.clone();
    seg.ph_seq = score.phones.iter().map(|p| p.symbol.clone()).collect();
    seg.ph_dur = pred.ph_dur.clone();
    seg.note_dur = pred.ph_dur.clone();
    seg.note_seq_midi = score
        .phones
        .iter()
        .map(|p| score.groups.get(p.group).map_or(0, |g| g.midi as i32))
        .collect();
    if !rule.languages.is_empty() {
        seg.languages = score.phones.iter().map(|p| p.language).collect();
    }
    let Some(pitch) = &pred.pitch else {
        return seg;
    };
    let frame = pred.frame_sec;
    seg.f0 = SampleCurve {
        samples: pitch.iter().map(|m| midi_to_hz(*m as f64)).collect(),
        timestep: frame,
    };
    // The bank's tension where the acoustic model takes it, with the
    // lane's own tension (slider, velocity / contour, Tension curve) on
    // top.
    if bank.curve_supported(CurveKind::Tension) {
        if let Some(predicted) = &pred.variance.tension {
            let ours = if rule.tension.is_empty() {
                vec![0.0; predicted.len()]
            } else {
                rule.tension.resample(frame, predicted.len())
            };
            seg.tension = SampleCurve {
                samples: predicted
                    .iter()
                    .zip(ours)
                    .map(|(p, o)| *p as f64 + o)
                    .collect(),
                timestep: frame,
            };
        }
    }
    seg
}

/// Linear interpolation of per-frame values at fractional frame `x`
/// (held past the ends; 0 for no values).
fn sample_frames(values: &[f64], x: f64) -> f64 {
    match values.len() {
        0 => 0.0,
        1 => values[0],
        n => {
            let x = x.clamp(0.0, (n - 1) as f64);
            let lo = x.floor() as usize;
            let hi = (lo + 1).min(n - 1);
            let f = x - lo as f64;
            values[lo] * (1.0 - f) + values[hi] * f
        }
    }
}
