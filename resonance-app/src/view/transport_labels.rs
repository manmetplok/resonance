//! Lazy-memoised string cache for the transport bar's stat-block labels.
//!
//! The transport renders five `format!`-derived strings every paint
//! (position, time, signature, key, loop length). With a continuous
//! window resize that's five fresh allocations per frame at 60 Hz —
//! enough to be visible on a hot resize path. This cache reformats
//! only when the inputs that feed each label actually change; the view
//! borrows the cached strings directly.
//!
//! Ownership: the cache lives on `Resonance` as a plain field and is
//! refreshed by `Resonance::refresh_transport_labels` after every
//! `update()` dispatch (plus at construction and demo seeding). The
//! view layer (`view::transport`) only reads it — `refresh` is never
//! called from `view()`.
//!
//! See `.claude/skills/ui-work.md` §11 for the broader view-perf rules.

use crate::Resonance;
use resonance_music_theory::{Mode, PitchClass};

#[derive(Debug, Clone, Default)]
pub(crate) struct TransportLabels {
    position_key: PositionKey,
    pub position: String,

    time_key: TimeKey,
    pub time: String,

    sig_key: SigKey,
    pub sig: String,

    key_key: KeyKey,
    pub key: String,

    loop_key: LoopKey,
    pub loop_text: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PositionKey {
    bar: u32,
    beat: u32,
    subdiv: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TimeKey {
    minutes: u32,
    /// Seconds × 1000 (i.e. milliseconds inside the current minute).
    sec_milli: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SigKey {
    num: u8,
    den: u8,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct KeyKey {
    scale: Option<(PitchClass, Mode)>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LoopKey {
    enabled: bool,
    loop_in: u64,
    loop_out: u64,
    sample_rate: u32,
    /// BPM × 100 — quantised to avoid float comparison.
    bpm_centi: u32,
    sig_num: u8,
}

impl TransportLabels {
    /// Refresh any labels whose inputs changed since the last call.
    /// Cheap when nothing changed (just five tuple equality checks).
    pub(crate) fn refresh(&mut self, r: &Resonance) {
        // Position: bar.beat.subdiv.
        let (bar_0, frac) = r
            .tempo_map
            .sample_to_bar(r.transport.playhead, r.sample_rate);
        let num_beats = r.transport.time_sig_num as f64;
        let beat_frac = frac * num_beats;
        let key = PositionKey {
            bar: bar_0 + 1,
            beat: beat_frac.floor() as u32 + 1,
            subdiv: (beat_frac.fract() * 1000.0) as u32,
        };
        if key != self.position_key {
            use std::fmt::Write;
            self.position.clear();
            let _ = write!(&mut self.position, "{}.{}.{:03}", key.bar, key.beat, key.subdiv);
            self.position_key = key;
        }

        // Time: MM:SS.mmm.
        let total_secs = r.transport.playhead as f64 / r.sample_rate as f64;
        let minutes = (total_secs / 60.0).floor() as u32;
        let seconds = total_secs - (minutes as f64 * 60.0);
        let time_key = TimeKey {
            minutes,
            sec_milli: (seconds * 1000.0) as u64,
        };
        if time_key != self.time_key {
            use std::fmt::Write;
            self.time.clear();
            let secs_disp = time_key.sec_milli as f64 / 1000.0;
            let _ = write!(&mut self.time, "{:02}:{:06.3}", time_key.minutes, secs_disp);
            self.time_key = time_key;
        }

        // Signature: N/D.
        let sig_key = SigKey {
            num: r.transport.time_sig_num,
            den: r.transport.time_sig_den,
        };
        if sig_key != self.sig_key {
            use std::fmt::Write;
            self.sig.clear();
            let _ = write!(&mut self.sig, "{}/{}", sig_key.num, sig_key.den);
            self.sig_key = sig_key;
        }

        // Key (compose tab's first scale).
        let scale = r
            .compose
            .definitions
            .iter()
            .find_map(|d| d.scale.as_ref());
        let key_key = KeyKey {
            scale: scale.map(|s| (s.root, s.mode)),
        };
        if key_key != self.key_key {
            self.key.clear();
            match key_key.scale {
                None => self.key.push('—'),
                Some((root, mode)) => {
                    use std::fmt::Write;
                    let mode_label = mode_short_label(mode);
                    let _ = write!(&mut self.key, "{} {}", root, mode_label);
                }
            }
            self.key_key = key_key;
        }

        // Loop length label.
        let loop_key = LoopKey {
            enabled: r.transport.loop_enabled,
            loop_in: r.transport.loop_in,
            loop_out: r.transport.loop_out,
            sample_rate: r.sample_rate,
            bpm_centi: (r.transport.bpm * 100.0) as u32,
            sig_num: r.transport.time_sig_num,
        };
        if loop_key != self.loop_key {
            self.loop_text.clear();
            format_loop_into(&mut self.loop_text, loop_key);
            self.loop_key = loop_key;
        }
    }
}

fn mode_short_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Major => "maj",
        Mode::Minor => "min",
        Mode::Dorian => "dor",
        Mode::Phrygian => "phr",
        Mode::Lydian => "lyd",
        Mode::Mixolydian => "mix",
        Mode::Locrian => "loc",
        Mode::HarmonicMinor => "hmin",
        Mode::MelodicMinor => "mmin",
    }
}

fn format_loop_into(out: &mut String, key: LoopKey) {
    use std::fmt::Write;
    if !key.enabled {
        out.push_str("off");
        return;
    }
    let len_samples = key.loop_out.saturating_sub(key.loop_in);
    if len_samples == 0 {
        out.push('—');
        return;
    }
    let len_secs = len_samples as f64 / key.sample_rate as f64;
    let secs_per_beat = 60.0 / (key.bpm_centi as f64 / 100.0);
    let secs_per_bar = secs_per_beat * key.sig_num as f64;
    let bars = (len_secs / secs_per_bar).round() as i64;
    if bars >= 1 {
        if bars == 1 {
            out.push_str("1 bar");
        } else {
            let _ = write!(out, "{} bars", bars);
        }
    } else {
        let beats = (len_secs / secs_per_beat).round() as i64;
        if beats == 1 {
            out.push_str("1 beat");
        } else {
            let _ = write!(out, "{} beats", beats);
        }
    }
}
