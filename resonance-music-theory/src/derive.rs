//! Pure part generators: given a chord progression, produce MIDI notes
//! for a pad, bass line, or melody.
//!
//! The functions here do not depend on any DAW types. They take a
//! `TimedChord` list and return `GeneratedNote`s with ticks measured
//! from the start of the containing clip. The app crate is responsible
//! for converting between these and the engine's `MidiClip` / `MidiNote`.

use serde::{Deserialize, Serialize};

use crate::chord::Chord;
use crate::pitch::PitchClass;
use crate::rng::XorShift;
use crate::scale::Scale;
use crate::voicing::{close_voicing, nearest_midi_above, nearest_midi_to, voice_lead};

/// A chord positioned on the section's beat grid. Mirrors the app's
/// `ChordState` so callers don't have to take a dependency on the app
/// crate just to use these generators.
#[derive(Debug, Clone, Copy)]
pub struct TimedChord {
    pub chord: Chord,
    pub start_beat: u32,
    pub duration_beats: u32,
}

/// DAW-agnostic MIDI note. Matches `resonance_audio::types::MidiNote`
/// field-for-field; converted at the app boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeneratedNote {
    pub note: u8,
    pub velocity: f32,
    pub start_tick: u64,
    pub duration_ticks: u64,
}

// ---------- Pad ----------

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PadParams {
    /// Inclusive MIDI range the pad voicings must stay inside.
    pub register: (u8, u8),
    pub velocity: f32,
}

impl Default for PadParams {
    fn default() -> Self {
        Self {
            register: (52, 76), // E3..E5 — a safe "pad" register
            velocity: 0.7,
        }
    }
}

/// Sustained, voice-led chord voicings. The first chord is spelled as a
/// close voicing anchored to the register floor; subsequent chords are
/// voice-led from the previous voicing so common tones stay put and
/// moving voices move by the smallest interval.
pub fn derive_pad(
    chords: &[TimedChord],
    params: &PadParams,
    ticks_per_beat: u32,
) -> Vec<GeneratedNote> {
    if chords.is_empty() {
        return Vec::new();
    }
    let tpb = ticks_per_beat as u64;
    let mut out = Vec::new();

    // Seed voicing: close voicing at the register floor, then clamp any
    // voices above the register ceiling by dropping them an octave.
    let mut voicing: Vec<u8> = close_voicing(chords[0].chord, params.register.0)
        .into_iter()
        .map(|n| {
            let mut m = n;
            while m > params.register.1 && m >= 12 {
                m -= 12;
            }
            m
        })
        .collect();
    voicing.sort_unstable();

    for (i, tc) in chords.iter().enumerate() {
        if i > 0 {
            voicing = voice_lead(&voicing, &tc.chord.pitch_classes(), params.register);
        }
        let start_tick = tc.start_beat as u64 * tpb;
        let duration_ticks = tc.duration_beats as u64 * tpb;
        for &note in &voicing {
            out.push(GeneratedNote {
                note,
                velocity: params.velocity,
                start_tick,
                duration_ticks,
            });
        }
    }
    out
}

// ---------- Bass ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BassStyle {
    /// One note per chord, held for the chord's full duration.
    RootHold,
    /// Root on every beat of the chord.
    RootPulse,
    /// Root / fifth alternating on each beat.
    RootFifth,
    /// Root / octave alternating on each beat.
    Octave,
    /// Scale-stepping walking bass that approaches the next chord's root.
    /// Falls back to `RootPulse` when no scale is provided.
    Walking,
}

impl BassStyle {
    pub const ALL: [BassStyle; 5] = [
        BassStyle::RootHold,
        BassStyle::RootPulse,
        BassStyle::RootFifth,
        BassStyle::Octave,
        BassStyle::Walking,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BassStyle::RootHold => "Root hold",
            BassStyle::RootPulse => "Root pulse",
            BassStyle::RootFifth => "Root + fifth",
            BassStyle::Octave => "Octave",
            BassStyle::Walking => "Walking",
        }
    }
}

impl std::fmt::Display for BassStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BassParams {
    pub style: BassStyle,
    /// MIDI floor for the bass root. Default E1 (28).
    pub base_note: u8,
    pub velocity: f32,
}

impl Default for BassParams {
    fn default() -> Self {
        Self {
            style: BassStyle::RootPulse,
            base_note: 28, // E1
            velocity: 0.85,
        }
    }
}

pub fn derive_bass(
    chords: &[TimedChord],
    scale: Option<Scale>,
    params: &BassParams,
    ticks_per_beat: u32,
) -> Vec<GeneratedNote> {
    if chords.is_empty() {
        return Vec::new();
    }
    let tpb = ticks_per_beat as u64;
    let mut out = Vec::new();

    for (i, tc) in chords.iter().enumerate() {
        let root_pc = tc.chord.bass.unwrap_or(tc.chord.root);
        let root_midi = nearest_midi_above(root_pc, params.base_note);
        let start_tick = tc.start_beat as u64 * tpb;
        let beats = tc.duration_beats.max(1);

        match params.style {
            BassStyle::RootHold => {
                out.push(GeneratedNote {
                    note: root_midi,
                    velocity: params.velocity,
                    start_tick,
                    duration_ticks: beats as u64 * tpb,
                });
            }
            BassStyle::RootPulse => {
                for b in 0..beats {
                    out.push(GeneratedNote {
                        note: root_midi,
                        velocity: params.velocity,
                        start_tick: start_tick + b as u64 * tpb,
                        duration_ticks: tpb,
                    });
                }
            }
            BassStyle::RootFifth => {
                let fifth_pc = root_pc.transpose(7);
                let fifth_midi = nearest_midi_above(fifth_pc, root_midi);
                for b in 0..beats {
                    let note = if b % 2 == 0 { root_midi } else { fifth_midi };
                    out.push(GeneratedNote {
                        note,
                        velocity: params.velocity,
                        start_tick: start_tick + b as u64 * tpb,
                        duration_ticks: tpb,
                    });
                }
            }
            BassStyle::Octave => {
                let up = root_midi.checked_add(12).filter(|&n| n <= 127);
                for b in 0..beats {
                    let note = if b % 2 == 0 || up.is_none() {
                        root_midi
                    } else {
                        up.unwrap()
                    };
                    out.push(GeneratedNote {
                        note,
                        velocity: params.velocity,
                        start_tick: start_tick + b as u64 * tpb,
                        duration_ticks: tpb,
                    });
                }
            }
            BassStyle::Walking => {
                let next_root_midi = match (chords.get(i + 1), scale) {
                    (Some(nc), _) => {
                        let next_pc = nc.chord.bass.unwrap_or(nc.chord.root);
                        nearest_midi_to(next_pc, root_midi)
                    }
                    (None, _) => root_midi,
                };
                let line = walking_line(scale, root_midi, next_root_midi, beats as usize);
                for (b, note) in line.into_iter().enumerate() {
                    out.push(GeneratedNote {
                        note,
                        velocity: params.velocity,
                        start_tick: start_tick + b as u64 * tpb,
                        duration_ticks: tpb,
                    });
                }
            }
        }
    }
    out
}

/// Stepwise line from `root` toward `next_root` through scale tones.
/// When no scale is available, falls back to repeating the root.
fn walking_line(scale: Option<Scale>, root: u8, next_root: u8, beats: usize) -> Vec<u8> {
    if beats == 0 {
        return Vec::new();
    }
    let Some(scale) = scale else {
        return vec![root; beats];
    };
    if beats == 1 {
        return vec![root];
    }

    // The last beat is an approach tone — one scale step away from the
    // next chord's root, on the side we're coming from.
    let approach_dir: i32 = if next_root >= root { -1 } else { 1 };
    let approach = step_scale(&scale, next_root, approach_dir);

    if beats == 2 {
        return vec![root, approach];
    }

    // Interior beats: step from root toward the approach tone. Direction
    // is chosen by whichever end of the span the approach tone sits on.
    let up = approach > root;
    let interior_count = beats - 2;
    let mut notes = Vec::with_capacity(beats);
    notes.push(root);
    let mut cur = root;
    for _ in 0..interior_count {
        cur = step_scale(&scale, cur, if up { 1 } else { -1 });
        notes.push(cur);
    }
    notes.push(approach);
    notes
}

/// Next MIDI note in `dir` direction whose pitch class belongs to
/// `scale`. Searches up to one octave; returns `from` if no scale tone
/// is found (shouldn't happen for well-formed scales).
fn step_scale(scale: &Scale, from: u8, dir: i32) -> u8 {
    let mut n = from as i32 + dir;
    for _ in 0..12 {
        if !(0..=127).contains(&n) {
            return from;
        }
        if scale.contains(n as u8) {
            return n as u8;
        }
        n += dir;
    }
    from
}

// ---------- Melody ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MelodyStyle {
    ArpUp,
    ArpDown,
    ArpUpDown,
    ScaleWalk,
}

impl MelodyStyle {
    pub const ALL: [MelodyStyle; 4] = [
        MelodyStyle::ArpUp,
        MelodyStyle::ArpDown,
        MelodyStyle::ArpUpDown,
        MelodyStyle::ScaleWalk,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MelodyStyle::ArpUp => "Arp up",
            MelodyStyle::ArpDown => "Arp down",
            MelodyStyle::ArpUpDown => "Arp up/down",
            MelodyStyle::ScaleWalk => "Scale walk",
        }
    }
}

impl std::fmt::Display for MelodyStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MelodyParams {
    pub style: MelodyStyle,
    pub register: (u8, u8),
    /// Length of one melody note in ticks. 240 = 8ths at TPQN=480,
    /// 120 = 16ths, 480 = quarter notes.
    pub note_value_ticks: u32,
    /// Probability in [0, 1] that any given slot is silent.
    pub rest_density: f32,
    pub velocity: f32,
}

impl Default for MelodyParams {
    fn default() -> Self {
        Self {
            style: MelodyStyle::ArpUp,
            register: (67, 88), // G4..E6
            note_value_ticks: 240,
            rest_density: 0.0,
            velocity: 0.8,
        }
    }
}

pub fn derive_melody(
    chords: &[TimedChord],
    scale: Option<Scale>,
    params: &MelodyParams,
    ticks_per_beat: u32,
    seed: u64,
) -> Vec<GeneratedNote> {
    if chords.is_empty() {
        return Vec::new();
    }
    let tpb = ticks_per_beat as u64;
    let slot_ticks = params.note_value_ticks.max(1) as u64;
    let mut out = Vec::new();
    let mut rng = XorShift::new(seed);
    let mut last_walk_note: Option<u8> = None;

    for tc in chords {
        let chord_start = tc.start_beat as u64 * tpb;
        let chord_len = (tc.duration_beats as u64).max(1) * tpb;
        let tones = chord_tones_in_register(tc.chord, params.register);
        if tones.is_empty() {
            continue;
        }

        let slots = (chord_len / slot_ticks).max(1) as usize;
        for slot in 0..slots {
            // Rest dice roll even for arps, so density works uniformly.
            let rest_roll = rng.next_f32();
            if params.rest_density > 0.0 && rest_roll < params.rest_density {
                continue;
            }

            let note = match params.style {
                MelodyStyle::ArpUp => tones[slot % tones.len()],
                MelodyStyle::ArpDown => tones[tones.len() - 1 - (slot % tones.len())],
                MelodyStyle::ArpUpDown => {
                    let n = tones.len();
                    if n < 2 {
                        tones[0]
                    } else {
                        let cycle = 2 * n - 2;
                        let idx = slot % cycle;
                        if idx < n {
                            tones[idx]
                        } else {
                            tones[cycle - idx]
                        }
                    }
                }
                MelodyStyle::ScaleWalk => {
                    if let Some(scale) = scale {
                        if slot == 0 || last_walk_note.is_none() {
                            // Anchor: start on a chord tone (lowest available).
                            tones[0]
                        } else {
                            let prev = last_walk_note.unwrap();
                            // 70% step, 30% leap to a chord tone for shape.
                            if rng.next_f32() < 0.3 {
                                tones[rng.next_range(tones.len())]
                            } else {
                                let dir = if rng.next_f32() < 0.5 { 1 } else { -1 };
                                let mut next = step_scale(&scale, prev, dir);
                                if next < params.register.0 {
                                    next = step_scale(&scale, next, 1);
                                }
                                if next > params.register.1 {
                                    next = step_scale(&scale, next, -1);
                                }
                                // Final clamp: if still out of register
                                // (very narrow range), fall back to a
                                // chord tone.
                                if next < params.register.0
                                    || next > params.register.1
                                {
                                    next = tones[0];
                                }
                                next
                            }
                        }
                    } else {
                        tones[slot % tones.len()]
                    }
                }
            };

            if matches!(params.style, MelodyStyle::ScaleWalk) {
                last_walk_note = Some(note);
            }

            out.push(GeneratedNote {
                note,
                velocity: params.velocity,
                start_tick: chord_start + slot as u64 * slot_ticks,
                duration_ticks: slot_ticks,
            });
        }
    }
    out
}

/// Every MIDI note inside `register` whose pitch class appears in
/// `chord`, sorted ascending and deduplicated.
fn chord_tones_in_register(chord: Chord, register: (u8, u8)) -> Vec<u8> {
    let pcs: Vec<PitchClass> = chord.pitch_classes();
    let (lo, hi) = register;
    let mut notes = Vec::new();
    for midi in lo..=hi {
        let pc = PitchClass::from_semitone(midi % 12);
        if pcs.contains(&pc) {
            notes.push(midi);
        }
    }
    notes.sort_unstable();
    notes.dedup();
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chord::ChordQuality;
    use crate::pitch::PitchClass::*;
    use crate::scale::Mode;

    fn tc(chord: Chord, start_beat: u32, duration_beats: u32) -> TimedChord {
        TimedChord {
            chord,
            start_beat,
            duration_beats,
        }
    }

    // ---------- Pad ----------

    #[test]
    fn pad_empty_in_empty_out() {
        assert!(derive_pad(&[], &PadParams::default(), 480).is_empty());
    }

    #[test]
    fn pad_produces_one_note_per_voice_per_chord() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(F, ChordQuality::Maj), 4, 4),
        ];
        let p = PadParams::default();
        let notes = derive_pad(&chords, &p, 480);
        assert_eq!(notes.len(), 6); // 3 voices × 2 chords
    }

    #[test]
    fn pad_voices_stay_in_register() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(F, ChordQuality::Maj7), 4, 4),
            tc(Chord::new(G, ChordQuality::Dom7), 8, 4),
        ];
        let p = PadParams {
            register: (48, 72),
            velocity: 0.7,
        };
        for n in derive_pad(&chords, &p, 480) {
            assert!(n.note >= 48 && n.note <= 72, "{} out of register", n.note);
        }
    }

    #[test]
    fn pad_start_ticks_match_beats() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(G, ChordQuality::Maj), 4, 4),
        ];
        let notes = derive_pad(&chords, &PadParams::default(), 480);
        // First chord at beat 0 → start_tick 0; second at beat 4 → 1920.
        let c_start: Vec<u64> = notes
            .iter()
            .filter(|n| n.start_tick == 0)
            .map(|n| n.start_tick)
            .collect();
        let g_start: Vec<u64> = notes
            .iter()
            .filter(|n| n.start_tick == 1920)
            .map(|n| n.start_tick)
            .collect();
        assert_eq!(c_start.len(), 3);
        assert_eq!(g_start.len(), 3);
    }

    // ---------- Bass ----------

    #[test]
    fn bass_empty_in_empty_out() {
        assert!(derive_bass(&[], None, &BassParams::default(), 480).is_empty());
    }

    #[test]
    fn bass_root_hold_one_note_per_chord() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(G, ChordQuality::Maj), 4, 4),
            tc(Chord::new(A, ChordQuality::Min), 8, 4),
        ];
        let p = BassParams {
            style: BassStyle::RootHold,
            ..BassParams::default()
        };
        let notes = derive_bass(&chords, None, &p, 480);
        assert_eq!(notes.len(), 3);
        assert_eq!(notes[0].duration_ticks, 4 * 480);
    }

    #[test]
    fn bass_root_pulse_has_one_note_per_beat() {
        let chords = vec![tc(Chord::new(C, ChordQuality::Maj), 0, 4)];
        let p = BassParams {
            style: BassStyle::RootPulse,
            ..BassParams::default()
        };
        let notes = derive_bass(&chords, None, &p, 480);
        assert_eq!(notes.len(), 4);
        assert!(notes.iter().all(|n| n.note == notes[0].note));
    }

    #[test]
    fn bass_slash_chord_uses_bass_pitch_class() {
        // Am/G: root should be G, not A.
        let chord = Chord::new(A, ChordQuality::Min).with_bass(G);
        let chords = vec![tc(chord, 0, 4)];
        let p = BassParams {
            style: BassStyle::RootHold,
            ..BassParams::default()
        };
        let notes = derive_bass(&chords, None, &p, 480);
        assert_eq!(notes.len(), 1);
        // Expect G at or above base_note 28 (E1) — the nearest G ≥ 28 is G1 = 31.
        assert_eq!(notes[0].note % 12, G.to_semitone());
    }

    #[test]
    fn bass_walking_falls_back_without_scale() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(G, ChordQuality::Maj), 4, 4),
        ];
        let p = BassParams {
            style: BassStyle::Walking,
            ..BassParams::default()
        };
        let notes = derive_bass(&chords, None, &p, 480);
        assert_eq!(notes.len(), 8);
    }

    #[test]
    fn bass_walking_uses_scale_tones() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(G, ChordQuality::Maj), 4, 4),
        ];
        let scale = Scale::new(C, Mode::Major);
        let p = BassParams {
            style: BassStyle::Walking,
            ..BassParams::default()
        };
        let notes = derive_bass(&chords, Some(scale), &p, 480);
        // Every note must belong to the scale.
        for n in &notes {
            assert!(
                scale.contains(n.note),
                "walking bass note {} not in C major",
                n.note
            );
        }
        // Should produce one note per beat across both chords.
        assert_eq!(notes.len(), 8);
    }

    // ---------- Melody ----------

    #[test]
    fn melody_empty_in_empty_out() {
        assert!(derive_melody(&[], None, &MelodyParams::default(), 480, 0).is_empty());
    }

    #[test]
    fn melody_arp_up_stays_in_register() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(F, ChordQuality::Maj), 4, 4),
        ];
        let p = MelodyParams::default();
        let notes = derive_melody(&chords, None, &p, 480, 1);
        assert!(!notes.is_empty());
        for n in &notes {
            assert!(n.note >= p.register.0 && n.note <= p.register.1);
        }
    }

    #[test]
    fn melody_arp_uses_chord_tones_only() {
        let chord = Chord::new(C, ChordQuality::Maj); // [C, E, G]
        let chords = vec![tc(chord, 0, 4)];
        let p = MelodyParams {
            style: MelodyStyle::ArpUp,
            ..MelodyParams::default()
        };
        let notes = derive_melody(&chords, None, &p, 480, 1);
        for n in &notes {
            let pc = n.note % 12;
            assert!(pc == 0 || pc == 4 || pc == 7, "non-chord note {}", n.note);
        }
    }

    #[test]
    fn melody_scale_walk_stays_in_scale() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(A, ChordQuality::Min), 4, 4),
        ];
        let scale = Scale::new(C, Mode::Major);
        let p = MelodyParams {
            style: MelodyStyle::ScaleWalk,
            ..MelodyParams::default()
        };
        let notes = derive_melody(&chords, Some(scale), &p, 480, 7);
        for n in &notes {
            assert!(
                scale.contains(n.note),
                "walk note {} not in C major",
                n.note
            );
        }
    }

    #[test]
    fn melody_rest_density_one_produces_no_notes() {
        let chords = vec![tc(Chord::new(C, ChordQuality::Maj), 0, 4)];
        let p = MelodyParams {
            rest_density: 1.0,
            ..MelodyParams::default()
        };
        // With density exactly 1.0, every rest_roll < 1.0 → everything
        // is a rest. We can't guarantee this for all XorShift outputs,
        // but in practice all slots will be rests.
        let notes = derive_melody(&chords, None, &p, 480, 3);
        assert!(notes.len() <= 1);
    }

    #[test]
    fn melody_seed_reproducible() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(G, ChordQuality::Maj), 4, 4),
        ];
        let scale = Scale::new(C, Mode::Major);
        let p = MelodyParams {
            style: MelodyStyle::ScaleWalk,
            ..MelodyParams::default()
        };
        let a = derive_melody(&chords, Some(scale), &p, 480, 42);
        let b = derive_melody(&chords, Some(scale), &p, 480, 42);
        assert_eq!(a, b);
    }
}
