//! Per-pad sample identity published for the editor's SAMPLE stage.
//!
//! The editor cannot reach the audio thread's `LoadedPad`s, so whoever
//! builds a kit (the loader thread, or `initialize()` for the embedded
//! fallback) also builds one [`PadSampleInfo`] per pad and parks it on the
//! bridge. Everything in here is measured from the decoded samples — the
//! inspector never draws a shape it did not get from a real take.

use crate::kit::{LoadedMicBank, LoadedPad};


/// Number of min/max buckets in the published waveform envelope. Sized for
/// the ~500 px inspector canvas: enough detail to read the transient, small
/// enough that publishing 30 of them costs nothing.
pub const ENVELOPE_BUCKETS: usize = 240;

/// What the editor knows about the sample a pad would play at full
/// velocity. Built by [`info_for_pad`]; `None` for a pad with no bank
/// loaded at all, which the inspector renders as "no sample loaded".
#[derive(Debug, Clone, PartialEq)]
pub struct PadSampleInfo {
    /// Mic position of the bank the pad's velocity / round-robin selection
    /// reads (`"KickIn"`, `"OHsAB"`, …). Empty for the embedded fallback.
    pub position: String,
    /// Manifest setup key that produced the bank (`"01_KickIn_e901"`).
    /// Empty for the embedded fallback bank.
    pub setup_key: String,
    /// Velocity layers in the bank, and the index of the one displayed
    /// (the loudest — what a full-velocity hit plays).
    pub layer_count: usize,
    pub layer_index: usize,
    /// Round-robin takes in the displayed layer, and the index shown.
    pub take_count: usize,
    pub take_index: usize,
    /// Frames in the displayed take, and the rate it was decoded to.
    pub frames: usize,
    pub sample_rate: f32,
    /// Min/max pairs of the displayed take, `ENVELOPE_BUCKETS` long
    /// (shorter only when the take has fewer frames than buckets).
    pub envelope: Vec<(f32, f32)>,
}

impl PadSampleInfo {
    /// Duration of the displayed take, formatted `mm:ss.mmm`. Returns
    /// `None` when no sample rate is known yet, so the caller can print a
    /// dash instead of a fabricated `00:00.000`.
    pub fn duration_text(&self) -> Option<String> {
        if self.sample_rate <= 0.0 {
            return None;
        }
        let secs = self.frames as f32 / self.sample_rate;
        let minutes = (secs / 60.0).floor() as u32;
        let rem = secs - minutes as f32 * 60.0;
        Some(format!("{minutes:02}:{:06.3}", rem))
    }

    /// Short identity line for the top-right of the sample stage.
    pub fn source_text(&self) -> String {
        match (self.position.is_empty(), self.setup_key.is_empty()) {
            (true, true) => "embedded fallback".to_string(),
            (false, true) => self.position.clone(),
            (true, false) => self.setup_key.clone(),
            (false, false) => format!("{} · {}", self.position, self.setup_key),
        }
    }

    /// Layer / take line, e.g. `layer 4/4 · take 1/3`.
    pub fn layer_text(&self) -> String {
        format!(
            "layer {}/{} · take {}/{}",
            self.layer_index + 1,
            self.layer_count,
            self.take_index + 1,
            self.take_count,
        )
    }
}

/// Build the info for one mic bank, displaying the loudest velocity layer's
/// first take — the sample a full-velocity hit plays.
pub fn info_for_bank(bank: &LoadedMicBank, sample_rate: f32) -> Option<PadSampleInfo> {
    let layer_index = bank.layers.len().checked_sub(1)?;
    let layer = &bank.layers[layer_index];
    let take = layer.round_robins.first()?;
    Some(PadSampleInfo {
        position: bank.position.clone(),
        setup_key: bank.setup_key.clone(),
        layer_count: bank.layers.len(),
        layer_index,
        take_count: layer.round_robins.len(),
        take_index: 0,
        frames: take.frames,
        sample_rate,
        envelope: envelope(&take.data, take.frames),
    })
}

/// Build the info for a pad, reading the same reference bank that
/// `DrumSampler::note_on` uses to pick the velocity layer and round robin:
/// the first close mic, else the overhead.
pub fn info_for_pad(pad: &LoadedPad, sample_rate: f32) -> Option<PadSampleInfo> {
    let bank = pad
        .close_mics
        .first()
        .or(pad.overhead.as_ref())?;
    info_for_bank(bank, sample_rate)
}

/// Build one entry per pad, in pad order.
pub fn infos_for_pads(pads: &[LoadedPad], sample_rate: f32) -> Vec<Option<PadSampleInfo>> {
    pads.iter()
        .map(|pad| info_for_pad(pad, sample_rate))
        .collect()
}

/// Bytes of decoded sample data a kit holds, counting every mic bank,
/// velocity layer and round-robin take. This is a measurement of the kit,
/// not a guess at the process's resident memory.
pub fn total_sample_bytes(pads: &[LoadedPad]) -> usize {
    let bank_bytes = |bank: &LoadedMicBank| -> usize {
        bank.layers
            .iter()
            .flat_map(|layer| layer.round_robins.iter())
            .map(|take| take.data.len() * std::mem::size_of::<f32>())
            .sum()
    };
    pads.iter()
        .map(|pad| {
            pad.close_mics.iter().map(bank_bytes).sum::<usize>()
                + pad.overhead.as_ref().map(bank_bytes).unwrap_or(0)
        })
        .sum()
}

/// Format a byte count for the status bar: `12.4 MB`, `812 kB`, `0 B`.
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} kB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Reduce a stereo-interleaved take to `ENVELOPE_BUCKETS` min/max pairs
/// over both channels. Takes shorter than the bucket count yield one
/// bucket per frame rather than padding with invented zeroes.
pub fn envelope(data: &[f32], frames: usize) -> Vec<(f32, f32)> {
    if frames == 0 || data.is_empty() {
        return Vec::new();
    }
    let buckets = ENVELOPE_BUCKETS.min(frames);
    let mut out = Vec::with_capacity(buckets);
    for b in 0..buckets {
        let start = b * frames / buckets;
        let end = ((b + 1) * frames / buckets).max(start + 1).min(frames);
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for frame in start..end {
            let idx = frame * 2;
            for s in [data.get(idx), data.get(idx + 1)].into_iter().flatten() {
                lo = lo.min(*s);
                hi = hi.max(*s);
            }
        }
        if lo > hi {
            lo = 0.0;
            hi = 0.0;
        }
        out.push((lo, hi));
    }
    out
}
