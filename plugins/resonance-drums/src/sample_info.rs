//! Per-pad sample identity published for the editor's SAMPLE stage.
//!
//! The editor cannot reach the audio thread's `LoadedPad`s, so whoever
//! builds a kit (the loader thread, or `initialize()` for the embedded
//! fallback) also builds one [`PadSampleInfo`] per pad and parks it on the
//! bridge. Everything in here is measured from the decoded samples — the
//! inspector never draws a shape it did not get from a real take.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use crate::kit::{BankKind, LoadedMicBank, LoadedPad, LoadedSample, SampleData};

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
    /// Frames of the displayed take held in memory — `frames` unless disk
    /// streaming (E14) keeps only its head; `envelope` covers these.
    pub resident_frames: usize,
    /// Min/max pairs of the displayed take's resident frames,
    /// `ENVELOPE_BUCKETS` long (shorter only when it has fewer frames
    /// than buckets).
    pub envelope: Envelope,
    /// Every take of the bank, `[layer][take]` (soft → loud, take order),
    /// so the inspector can draw the take a pad **last played** (K5,
    /// [`crate::last_hit`]) rather than the fixed one above. Measured from
    /// what is in memory: a streamed take's envelope covers its head.
    pub takes: Vec<Vec<TakeShape>>,
    /// Which banks the pad holds — what the inspector offers a trim for,
    /// so it never draws a control for a mic the pad does not have.
    pub banks: PadBanks,
}

/// A take's min/max envelope, shared: a reload that keeps a take (the
/// same decoded `SampleData`, from the kit cache) keeps its envelope too,
/// rather than measuring every take of every pad again ([`envelope_of`]).
pub type Envelope = Arc<Vec<(f32, f32)>>;

/// One take's length and waveform envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct TakeShape {
    /// Frames in the take (the whole take, streamed or not).
    pub frames: usize,
    /// Frames of it held in memory: `frames`, or the head of a streamed
    /// take. The envelope covers these — a waveform drawn across the full
    /// width would stretch the head over the take's whole length.
    pub resident_frames: usize,
    /// Min/max pairs of the resident frames, as
    /// [`PadSampleInfo::envelope`].
    pub envelope: Envelope,
}

impl TakeShape {
    /// The fraction of the take the envelope covers (`resident_frames /
    /// frames`), `1.0` for a take held whole.
    pub fn resident_fraction(&self) -> f32 {
        resident_fraction(self.resident_frames, self.frames)
    }
}

fn resident_fraction(resident: usize, frames: usize) -> f32 {
    if frames == 0 {
        1.0
    } else {
        (resident as f32 / frames as f32).clamp(0.0, 1.0)
    }
}

/// The banks one loaded pad holds.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PadBanks {
    /// The close-mic banks, in bank order: (position, setup key).
    /// Bank 0 is trimmed by `pad_N_mic1_trim`, bank 1 by `pad_N_mic2_trim`.
    pub close: Vec<(String, String)>,
    /// Any overhead bank (slot 1, 2 or 3): `pad_N_oh_trim` applies.
    pub overhead: bool,
    /// A bleed bank (E15): `pad_N_bleed_trim` applies.
    pub bleed: bool,
    /// A room bank (E15): `pad_N_room_trim` applies.
    pub room: bool,
}

impl PadBanks {
    /// What `pad` holds.
    pub fn of(pad: &LoadedPad) -> Self {
        let has = |kind: fn(&BankKind) -> bool| pad.extra_banks.iter().any(|e| kind(&e.kind));
        Self {
            close: pad
                .close_mics
                .iter()
                .map(|b| (b.position.clone(), b.setup_key.clone()))
                .collect(),
            overhead: pad.overhead.is_some() || has(|k| matches!(k, BankKind::Overhead { .. })),
            bleed: has(|k| matches!(k, BankKind::Bleed)),
            room: has(|k| matches!(k, BankKind::Room)),
        }
    }
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

    /// [`TakeShape::resident_fraction`] of the displayed take.
    pub fn resident_fraction(&self) -> f32 {
        resident_fraction(self.resident_frames, self.frames)
    }

    /// The take a hit on `layer`/`take` played, if this bank has it.
    pub fn take(&self, layer: usize, take: usize) -> Option<&TakeShape> {
        self.takes.get(layer)?.get(take)
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
        frames: take.frames(),
        sample_rate,
        resident_frames: take.resident_frames(),
        envelope: envelope_of(take),
        takes: bank
            .layers
            .iter()
            .map(|layer| {
                layer
                    .round_robins
                    .iter()
                    .map(|t| TakeShape {
                        frames: t.frames(),
                        resident_frames: t.resident_frames(),
                        envelope: envelope_of(t),
                    })
                    .collect()
            })
            .collect(),
        banks: PadBanks::default(),
    })
}

/// Build the info for a pad, reading the same reference bank that
/// `DrumSampler::note_on` uses to pick the velocity layer and round robin:
/// the first close mic, else overhead slot 1, else an overhead slot
/// layered on it (E15) — a pad whose only bank is a slot-2 or slot-3
/// overhead plays that bank, so the inspector shows it (and its MICS)
/// too. Bleed and room never lead.
pub fn info_for_pad(pad: &LoadedPad, sample_rate: f32) -> Option<PadSampleInfo> {
    let bank = pad.close_mics.first().or(pad.overhead.as_ref()).or_else(|| {
        pad.extra_banks
            .iter()
            .find(|extra| matches!(extra.kind, BankKind::Overhead { .. }))
            .map(|extra| &extra.bank)
    })?;
    let mut info = info_for_bank(bank, sample_rate)?;
    info.banks = PadBanks::of(pad);
    Some(info)
}

/// Build one entry per pad, in pad order.
pub fn infos_for_pads(pads: &[LoadedPad], sample_rate: f32) -> Vec<Option<PadSampleInfo>> {
    let infos = pads
        .iter()
        .map(|pad| info_for_pad(pad, sample_rate))
        .collect();
    prune_envelopes();
    infos
}

/// Envelopes measured so far, by the address of the take's shared
/// `SampleData`, with a weak handle that tells a live take from a freed
/// one whose address was reused.
type EnvelopeCache = HashMap<usize, (Weak<SampleData>, Envelope)>;

fn envelope_cache() -> &'static Mutex<EnvelopeCache> {
    static CACHE: OnceLock<Mutex<EnvelopeCache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The envelope of `take`'s resident frames, measured once per decoded
/// take: a mic change reloads one pad, but the infos are rebuilt for all
/// thirty — every other pad's takes are the same `Arc<SampleData>` (the
/// kit cache's), so their envelopes come back from here instead of being
/// measured again.
pub fn envelope_of(take: &LoadedSample) -> Envelope {
    let shared = take.shared();
    let key = Arc::as_ptr(shared) as usize;
    let mut cache = envelope_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((weak, envelope)) = cache.get(&key) {
        if weak.upgrade().is_some_and(|live| Arc::ptr_eq(&live, shared)) {
            return envelope.clone();
        }
    }
    let envelope = Arc::new(envelope_channels(
        take.samples(),
        take.resident_frames(),
        take.channels(),
    ));
    cache.insert(key, (Arc::downgrade(shared), envelope.clone()));
    envelope
}

/// Forget the envelopes of takes no kit holds any more.
fn prune_envelopes() {
    envelope_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .retain(|_, (weak, _)| weak.strong_count() > 0);
}

/// Bytes of decoded sample data a kit holds, counting every mic bank,
/// velocity layer and round-robin take. This is a measurement of the kit,
/// not a guess at the process's resident memory.
pub fn total_sample_bytes(pads: &[LoadedPad]) -> usize {
    let bank_bytes = |bank: &LoadedMicBank| -> usize {
        bank.layers
            .iter()
            .flat_map(|layer| layer.round_robins.iter())
            .map(|take| take.bytes())
            .sum()
    };
    pads.iter()
        .map(|pad| {
            pad.banks().map(bank_bytes).sum::<usize>()
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
    envelope_channels(data, frames, 2)
}

/// [`envelope`] of a take interleaved `channels` (1 or 2) per frame.
pub fn envelope_channels(data: &[f32], frames: usize, channels: usize) -> Vec<(f32, f32)> {
    let channels = channels.max(1);
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
            let idx = frame * channels;
            for s in data.iter().skip(idx).take(channels) {
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
