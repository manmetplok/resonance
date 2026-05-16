use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde::Deserialize;
use std::fs;
use std::path::Path;

/// A single `.ds` segment. Fields are kept as `Option<String>` where DiffSinger uses
/// space-separated strings; the [`DsSegment`] view below converts them into typed vectors.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DsSegmentRaw {
    pub offset: Option<f64>,
    pub text: Option<String>,
    pub ph_seq: Option<String>,
    pub ph_dur: Option<String>,
    pub ph_num: Option<String>,
    pub note_seq: Option<String>,
    pub note_dur: Option<String>,
    pub note_slur: Option<String>,
    pub f0_seq: Option<String>,
    pub f0_timestep: Option<TimestepField>,
    pub gender: Option<String>,
    pub gender_timestep: Option<TimestepField>,
    pub velocity: Option<String>,
    pub velocity_timestep: Option<TimestepField>,
    pub energy: Option<String>,
    pub energy_timestep: Option<TimestepField>,
    pub breathiness: Option<String>,
    pub breathiness_timestep: Option<TimestepField>,
    pub voicing: Option<String>,
    pub voicing_timestep: Option<TimestepField>,
    pub tension: Option<String>,
    pub tension_timestep: Option<TimestepField>,
}

/// Some openvpi tools emit `f0_timestep` as a string, others as a number. Accept both.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(untagged)]
pub enum TimestepField {
    Number(f64),
    Text(StringFloat),
}

#[derive(Debug, Clone, Copy)]
pub struct StringFloat(pub f64);

impl<'de> serde::Deserialize<'de> for StringFloat {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s: String = String::deserialize(d)?;
        s.trim()
            .parse::<f64>()
            .map(StringFloat)
            .map_err(serde::de::Error::custom)
    }
}

impl TimestepField {
    pub fn value(self) -> f64 {
        match self {
            TimestepField::Number(v) => v,
            TimestepField::Text(StringFloat(v)) => v,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DsSegment {
    pub offset: f64,
    pub ph_seq: Vec<String>,
    pub ph_dur: Vec<f64>,
    pub ph_num: Vec<i32>,
    pub note_seq_midi: Vec<i32>,
    pub note_dur: Vec<f64>,
    pub note_slur: Vec<i32>,
    pub f0: SampleCurve,
    pub gender: SampleCurve,
    pub velocity: SampleCurve,
    pub energy: SampleCurve,
    pub breathiness: SampleCurve,
    pub voicing: SampleCurve,
    pub tension: SampleCurve,
    /// Optional per-token language ids, parallel to `ph_seq`. Used by
    /// multi-language voicebanks (Meiji) whose acoustic ONNX accepts a
    /// `languages` input. Empty when the voicebank doesn't expose such
    /// an input.
    pub languages: Vec<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct SampleCurve {
    pub samples: Vec<f64>,
    pub timestep: f64,
}

impl SampleCurve {
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty() || self.timestep == 0.0
    }

    /// Resample to `target_timestep` and pad / truncate to exactly `target_length`. Linear
    /// interpolation. Mirrors openvpi/Jobsecond `SampleCurve::resample` semantics.
    pub fn resample(&self, target_timestep: f64, target_length: usize) -> Vec<f64> {
        if self.is_empty() || target_timestep == 0.0 || target_length == 0 {
            return Vec::new();
        }
        if self.samples.len() == 1 {
            return vec![self.samples[0]; target_length];
        }
        let last_time = (self.samples.len() - 1) as f64 * self.timestep;
        let n_target = ((last_time / target_timestep).floor() as usize) + 1;
        let mut out = Vec::with_capacity(n_target.max(target_length));
        for i in 0..n_target {
            let t = i as f64 * target_timestep;
            let src_pos = t / self.timestep;
            let lo = src_pos.floor() as usize;
            let hi = (lo + 1).min(self.samples.len() - 1);
            let frac = src_pos - lo as f64;
            let v = self.samples[lo] * (1.0 - frac) + self.samples[hi] * frac;
            out.push(v);
        }
        if out.len() > target_length {
            out.truncate(target_length);
        } else if out.len() < target_length {
            let last = *out.last().unwrap_or(&0.0);
            out.resize(target_length, last);
        }
        out
    }
}

pub fn load_ds_file(path: &Path) -> Result<Vec<DsSegment>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading .ds file at {}", path.display()))?;
    let raws: Vec<DsSegmentRaw> = serde_json::from_str(&text)
        .with_context(|| format!("parsing .ds JSON at {}", path.display()))?;

    let mut out = Vec::with_capacity(raws.len());
    for (idx, raw) in raws.into_iter().enumerate() {
        out.push(
            compile_segment(&raw)
                .with_context(|| format!("compiling .ds segment {} (0-based)", idx))?,
        );
    }
    Ok(out)
}

fn compile_segment(raw: &DsSegmentRaw) -> Result<DsSegment> {
    let ph_seq = raw
        .ph_seq
        .as_deref()
        .ok_or_else(|| anyhow!("segment missing required ph_seq"))?;
    let ph_dur = raw
        .ph_dur
        .as_deref()
        .ok_or_else(|| anyhow!("segment missing required ph_dur"))?;
    let f0_seq = raw
        .f0_seq
        .as_deref()
        .ok_or_else(|| anyhow!("segment missing required f0_seq"))?;
    let f0_timestep = raw
        .f0_timestep
        .ok_or_else(|| anyhow!("segment missing required f0_timestep"))?
        .value();

    let mut seg = DsSegment {
        offset: raw.offset.unwrap_or(0.0),
        ph_seq: split_strings(ph_seq),
        ph_dur: split_floats(ph_dur)?,
        f0: SampleCurve {
            samples: split_floats(f0_seq)?,
            timestep: f0_timestep,
        },
        ..Default::default()
    };

    if let Some(s) = raw.ph_num.as_deref() {
        seg.ph_num = split_ints(s)?;
    }
    if let Some(s) = raw.note_seq.as_deref() {
        seg.note_seq_midi = split_strings(s)
            .into_iter()
            .map(|n| note_name_to_midi(&n))
            .collect();
    }
    if let Some(s) = raw.note_dur.as_deref() {
        seg.note_dur = split_floats(s)?;
    }
    if let Some(s) = raw.note_slur.as_deref() {
        seg.note_slur = split_ints(s)?;
    }
    fill_curve(&mut seg.gender, raw.gender.as_deref(), raw.gender_timestep)?;
    fill_curve(&mut seg.velocity, raw.velocity.as_deref(), raw.velocity_timestep)?;
    fill_curve(&mut seg.energy, raw.energy.as_deref(), raw.energy_timestep)?;
    fill_curve(&mut seg.breathiness, raw.breathiness.as_deref(), raw.breathiness_timestep)?;
    fill_curve(&mut seg.voicing, raw.voicing.as_deref(), raw.voicing_timestep)?;
    fill_curve(&mut seg.tension, raw.tension.as_deref(), raw.tension_timestep)?;
    Ok(seg)
}

fn split_strings(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

fn split_floats(s: &str) -> Result<Vec<f64>> {
    s.split_whitespace()
        .map(|t| t.parse::<f64>().map_err(|e| anyhow!("bad float `{t}`: {e}")))
        .collect()
}

fn split_ints(s: &str) -> Result<Vec<i32>> {
    s.split_whitespace()
        .map(|t| t.parse::<i32>().map_err(|e| anyhow!("bad int `{t}`: {e}")))
        .collect()
}

fn fill_curve(dst: &mut SampleCurve, samples: Option<&str>, ts: Option<TimestepField>) -> Result<()> {
    let (Some(s), Some(ts)) = (samples, ts) else {
        return Ok(());
    };
    dst.samples = split_floats(s)?;
    dst.timestep = ts.value();
    Ok(())
}

/// "C4", "D#4", "Bb3", "rest" → MIDI. "rest" / unparseable → 0. Mirrors Jobsecond's regex.
pub fn note_name_to_midi(s: &str) -> i32 {
    let pattern = Regex::new(r"^\s*([A-Ga-g])([#b!]*)([+-]?\d+)?\s*$").expect("static regex");
    let Some(caps) = pattern.captures(s.trim()) else {
        return 0;
    };
    let pitch_char = caps
        .get(1)
        .map(|m| m.as_str().chars().next().unwrap())
        .unwrap_or('C');
    let accidental = caps.get(2).map(|m| m.as_str()).unwrap_or("");
    let octave = caps
        .get(3)
        .and_then(|m| m.as_str().parse::<i32>().ok())
        .unwrap_or(0);

    let base = match pitch_char.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => 0,
    };
    let acc: i32 = accidental
        .chars()
        .map(|c| match c {
            '#' => 1,
            'b' | '!' => -1,
            _ => 0,
        })
        .sum();
    12 * (octave + 1) + base + acc
}
