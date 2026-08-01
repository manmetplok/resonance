//! Phoneme sequence + per-phoneme duration construction. Walks the
//! clip's MIDI notes back-to-back, runs each note's syllable through
//! G2P, and lays out consonant/vowel timings — plus the leading and
//! trailing AP pads, optional word-boundary SP, and optional stop
//! closure injections — into the parallel arrays the f0 / tension
//! stages downstream consume.

use resonance_audio::types::MidiNote;
use resonance_music_theory::g2p::AssignedSyllable;
use resonance_music_theory::{g2p, VocalParams};

use super::super::paths::{voicebank_language_id, voicebank_phoneme_name};
use super::super::phonemes::{floor_duration_sec, min_articulation_sec, target_duration_sec};
use super::super::SEGMENT_PAD_SEC;

/// Output of [`build_phoneme_track`]: every parallel array the
/// segment builder needs to drive the SVS pipeline and the per-frame
/// f0 / tension computations.
///
/// `entry_note_*` carry per-phoneme-entry note metadata that the f0
/// pass distributes across frames (so each f0 frame knows its parent
/// note's velocity and how far into the note it sits).
pub(super) struct PhonemeTrack {
    pub ph_seq: Vec<String>,
    pub ph_dur: Vec<f64>,
    pub note_seq: Vec<String>,
    pub note_dur: Vec<f64>,
    pub note_seq_midi: Vec<i32>,
    pub languages: Vec<i64>,
    pub entry_note_velocity: Vec<f32>,
    pub entry_note_total_sec: Vec<f64>,
    pub entry_note_start_offset: Vec<f64>,
}

/// Build the phoneme + note duration track for one section.
///
/// `assigned` is the per-note resolved + voicebank-validated syllable
/// stream (override > project-dict > global-dict > CMU-auto, with the
/// active voicebank's substitutions already applied — see
/// [`super::super::resolve_clip_pronunciation`] and
/// [`super::super::validate_for_voicebank`]). It carries exactly one
/// entry per note; the note's duration is split across its phonemes with
/// consonants getting a short slice and the vowel(s) absorbing the
/// remainder.
pub(super) fn build_phoneme_track(
    notes: &[MidiNote],
    params: &VocalParams,
    assigned: &[AssignedSyllable],
    ticks_per_quarter: u32,
    bpm: f32,
) -> PhonemeTrack {
    // Seconds per tick at the section's tempo. Vocal-lane MIDI clips use
    // `TICKS_PER_QUARTER_NOTE` as their tick rate, same as everywhere else
    // in the app.
    let seconds_per_tick = 60.0 / (bpm.max(1.0) as f64 * ticks_per_quarter as f64);

    // Number of distinct resolved syllables behind these notes, used only
    // for the optional word-boundary SP gate below. Slur notes share the
    // previous note's `syllable_index`, so the max + 1 is the syllable
    // count regardless of how many notes hold each syllable.
    let syllable_count = assigned
        .iter()
        .map(|a| a.syllable_index + 1)
        .max()
        .unwrap_or(0);
    // Optional word-boundary SP injection. Off by default (the
    // reference DiffSinger fixtures intentionally flow phonemes
    // continuously). Set RESONANCE_WORD_BOUNDARY_SP_MS=N to insert
    // ~N ms of SP at the end of each word's last syllable for an A/B
    // listening test. Practical range: 20-80 ms.
    let word_boundary_sp_sec: f64 = std::env::var("RESONANCE_WORD_BOUNDARY_SP_MS")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|ms| (ms / 1000.0).clamp(0.0, 0.2))
        .unwrap_or(0.0);
    // Optional stop-closure pre-silence. English stops (B/P/T/D/K/G)
    // have an inherent closure phase the model handles internally;
    // explicit `cl` insertion will most likely double up the closure
    // and sound worse. Off by default — set RESONANCE_STOP_CLOSURE_MS=N
    // to prepend ~N ms of `cl` before each stop consonant for an A/B
    // listening test. Practical range: 5-20 ms.
    let stop_closure_sec: f64 = std::env::var("RESONANCE_STOP_CLOSURE_MS")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|ms| (ms / 1000.0).clamp(0.0, 0.05))
        .unwrap_or(0.0);
    let consonant_emphasis = params.consonant_emphasis.clamp(0.0, 1.0);
    // Onset-consonant anticipation. Singers place a syllable's leading
    // consonants *before* the beat so the vowel — which carries the pitch
    // and most of the intelligibility — lands on it. Laying the consonants
    // out from the note's nominal start instead pushes every vowel late by
    // the length of its own onset, which reads as slurred, behind-the-beat
    // diction. We shift the boundary by stealing the onset cluster's
    // target duration from the *previous* note's slot (capped, and never
    // past the previous syllable's own articulation floor), so the total
    // segment length is unchanged — only the boundary moves. Set
    // RESONANCE_ONSET_LEAD_IN=0 to disable, or to a fraction to scale it.
    let onset_lead_in: f64 = std::env::var("RESONANCE_ONSET_LEAD_IN")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|v| v.clamp(0.0, 1.0))
        .unwrap_or(1.0);

    let mut track = PhonemeTrack {
        ph_seq: Vec::new(),
        ph_dur: Vec::new(),
        note_seq: Vec::new(),
        note_dur: Vec::new(),
        note_seq_midi: Vec::new(),
        languages: Vec::new(),
        entry_note_velocity: Vec::new(),
        entry_note_total_sec: Vec::new(),
        entry_note_start_offset: Vec::new(),
    };

    // Leading silence pad. The hand-crafted reference fixtures all
    // start with a 0.3 s `AP` so the model has time to ramp up cleanly;
    // skipping this produces an attack click on the first phoneme.
    push_rest_entry(&mut track, params, "AP", SEGMENT_PAD_SEC);

    let note_name_cache: Vec<String> = notes
        .iter()
        .map(|n| midi_to_diffsinger_note(n.note))
        .collect();

    // Pass 1: lay out each note's slot. We never insert AP between
    // adjacent syllables — the reference fixtures (`twinkle.ds`,
    // `hello_tiger.ds`) keep phonemes flowing continuously and let
    // the model handle syllable boundaries naturally. Each note's
    // effective sing duration is the time *until the next note* (or
    // its stated `duration_ticks` for the final note), so any
    // articulation-trim gap is absorbed automatically. Real silences
    // (gaps > 0.4 s between consecutive notes, which only happens at
    // genuine breath / rest points) still become an explicit AP.
    let mut plans: Vec<NotePlan> = Vec::with_capacity(notes.len());
    for (i, n) in notes.iter().enumerate() {
        let next_start_tick = notes
            .get(i + 1)
            .map(|nx| nx.start_tick)
            .unwrap_or(n.start_tick + n.duration_ticks);
        let slot_ticks = next_start_tick.saturating_sub(n.start_tick);
        let slot_sec = (slot_ticks as f64 * seconds_per_tick).max(0.05);

        // For genuine silences (long gaps to the next note), cap the
        // sing duration and put the rest into a trailing AP. Threshold
        // chosen so half-bar pauses become rests but typical syllable
        // spacing doesn't.
        let sing_sec_cap = (n.duration_ticks as f64 * seconds_per_tick).max(0.05);
        let (sing_sec, ap_sec) = if slot_sec > sing_sec_cap + super::super::SILENCE_GAP_SEC {
            (sing_sec_cap, slot_sec - sing_sec_cap)
        } else {
            (slot_sec, 0.0)
        };

        // Slur notes sing only the previous syllable's vowel for the
        // whole slot — no consonants, no new attack. `AssignedSyllable`
        // already encodes this (slur entries carry a single-vowel
        // phoneme list); fall back to `"ah"` when the resolver couldn't
        // even produce a vowel (e.g. draft is empty).
        let assignment = &assigned[i];
        let phonemes: &[&'static str] = if assignment.phonemes.is_empty() {
            &FALLBACK_PHONEMES
        } else {
            &assignment.phonemes
        };
        // Lexical-stress modulation of this syllable's velocity. The
        // tension curve later reads `frame_velocity` and pushes
        // strong-velocity frames toward more compressed / belted
        // delivery, so multiplying here is enough to make primary-
        // stress syllables sing louder & brighter than the surrounding
        // function-word schwas. Stress comes from CMU via
        // `resolve_draft` and is None for inline phoneme overrides.
        let stress_factor = assignment.stress.velocity_factor();
        let stressed_velocity = (n.velocity * stress_factor).clamp(0.0, 1.0);

        // Word-boundary SP: when this syllable is the LAST one of its
        // word, reserve a small silence at the end of the singing
        // slot. The reference DiffSinger fixtures don't insert SP
        // between words, so this is opt-in — env-var-gated for A/B
        // testing. `AssignedSyllable::is_word_end` is already `false`
        // for slur notes, so no extra check needed here.
        let inject_sp = word_boundary_sp_sec > 0.0
            && assignment.is_word_end
            && assignment.syllable_index + 1 < syllable_count;
        let sp_sec = if inject_sp {
            word_boundary_sp_sec.min(sing_sec * 0.3)
        } else {
            0.0
        };
        plans.push(NotePlan {
            phonemes,
            sing_sec,
            ap_sec,
            sp_sec,
            velocity: stressed_velocity,
        });
    }

    // Pass 2: pull each syllable's onset consonants back across the
    // preceding note boundary (see `onset_lead_in` above). Purely a
    // transfer between two adjacent slots, so the segment's total length
    // is untouched and the render-unit layout still lines up.
    apply_onset_lead_in(&mut plans, consonant_emphasis, onset_lead_in);

    // Pass 3: emit the parallel arrays.
    for (i, n) in notes.iter().enumerate() {
        let plan = &plans[i];
        let (sing_sec, ap_sec, sp_sec) = (plan.sing_sec, plan.ap_sec, plan.sp_sec);
        let phonemes = plan.phonemes;
        let stressed_velocity = plan.velocity;
        let phon_sing_sec = (sing_sec - sp_sec).max(0.05);

        // Split `phon_sing_sec` across phonemes with per-class targets
        // and audibility floors — see [`allocate_phoneme_durations`].
        let durations = allocate_phoneme_durations(phonemes, phon_sing_sec, consonant_emphasis);

        let note_name = &note_name_cache[i];
        // Track per-phoneme offset within this note for the metadata
        // arrays (consumed below by the dynamic tension curve and
        // vibrato gate).
        let mut offset_in_note: f64 = 0.0;
        for (ph_idx, ph) in phonemes.iter().enumerate() {
            // Optional stop-closure: prepend `cl` before a stop
            // consonant (B/P/T/D/K/G) to manufacture a brief closure
            // phase. Steals time from the stop's own slot to keep
            // the syllable's total duration unchanged. Skipped on
            // syllable-initial consonants — those have a natural
            // closure from the preceding silence/vowel.
            let is_stop = matches!(*ph, "b" | "p" | "t" | "d" | "k" | "g");
            let own_dur = durations.get(ph_idx).copied().unwrap_or(0.0);
            if stop_closure_sec > 0.0 && is_stop && ph_idx > 0 {
                let cl_dur = stop_closure_sec.min(own_dur * 0.4);
                track.ph_seq.push(voicebank_phoneme_name(params.voicebank, "cl"));
                track.ph_dur.push(cl_dur);
                track.note_seq.push(note_name.clone());
                track.note_dur.push(cl_dur);
                track.note_seq_midi.push(n.note as i32);
                if let Some(id) = voicebank_language_id(params.voicebank, "cl") {
                    track.languages.push(id);
                }
            }
            let mut d = own_dur;
            // Subtract the borrowed closure time so total syllable
            // duration stays the same.
            if stop_closure_sec > 0.0 && is_stop && ph_idx > 0 {
                d = (d - stop_closure_sec.min(own_dur * 0.4)).max(0.005);
            }
            track.ph_seq.push(voicebank_phoneme_name(params.voicebank, ph));
            track.ph_dur.push(d);
            track.note_seq.push(note_name.clone());
            track.note_dur.push(d);
            track.note_seq_midi.push(n.note as i32);
            if let Some(id) = voicebank_language_id(params.voicebank, ph) {
                track.languages.push(id);
            }
            track.entry_note_velocity.push(stressed_velocity);
            track.entry_note_total_sec.push(sing_sec);
            track.entry_note_start_offset.push(offset_in_note);
            offset_in_note += d;
        }
        if sp_sec > 0.0 {
            // Insert SP within the same note's slot — the syllable's
            // pitch carries through the brief silence.
            track.ph_seq.push(voicebank_phoneme_name(params.voicebank, "SP"));
            track.ph_dur.push(sp_sec);
            track.note_seq.push(note_name.clone());
            track.note_dur.push(sp_sec);
            track.note_seq_midi.push(n.note as i32);
            if let Some(id) = voicebank_language_id(params.voicebank, "SP") {
                track.languages.push(id);
            }
            track.entry_note_velocity.push(stressed_velocity);
            track.entry_note_total_sec.push(sing_sec);
            track.entry_note_start_offset.push(offset_in_note);
            // No further entries belong to this note after the SP, so
            // we don't carry the cumulative offset forward.
        }

        if ap_sec > 0.0 {
            push_rest_entry(&mut track, params, "AP", ap_sec);
        }
    }

    // Trailing silence pad, mirroring the leading AP.
    push_rest_entry(&mut track, params, "AP", SEGMENT_PAD_SEC);

    track
}

/// Phoneme list used when the resolver produced nothing for a note (an
/// empty draft, or fewer syllables than notes). A neutral `ah` keeps the
/// note voiced rather than emitting a zero-length entry.
static FALLBACK_PHONEMES: [&str; 1] = ["ah"];

/// One note's slot layout, computed before any phoneme durations are
/// handed out so the onset lead-in pass can move time between adjacent
/// notes first.
struct NotePlan<'a> {
    phonemes: &'a [&'static str],
    /// Time this note's own entries occupy (phonemes + any trailing SP).
    sing_sec: f64,
    /// Explicit rest appended after this note (a genuine silence).
    ap_sec: f64,
    /// Word-boundary SP reserved at the end of `sing_sec`.
    sp_sec: f64,
    velocity: f32,
}

impl NotePlan<'_> {
    /// The part of the slot the phonemes themselves get.
    fn phoneme_sec(&self) -> f64 {
        (self.sing_sec - self.sp_sec).max(0.0)
    }

    /// The leading consonant run — everything before the first vowel.
    fn onset(&self) -> &[&'static str] {
        let first_vowel = self
            .phonemes
            .iter()
            .position(|p| !g2p::is_consonant(p))
            .unwrap_or(0);
        &self.phonemes[..first_vowel]
    }
}

/// Move each note's onset consonants back across the preceding note
/// boundary so its vowel lands on the beat.
///
/// The amount taken is the onset cluster's *target* duration (not its
/// allocated one — that depends on the slot length we are about to
/// change, which would be circular), clamped three ways:
///
/// * never more than the previous note's slack — the time it has above
///   its own [`min_articulation_sec`] floor — so anticipating one
///   syllable never destroys the one before it;
/// * never more than 30 % of the previous note's phoneme time, so a long
///   held vowel isn't visibly clipped;
/// * skipped entirely across a genuine silence (`ap_sec > 0`) or a
///   word-boundary SP, where there is no note to steal from and the
///   consonant belongs after the gap anyway.
fn apply_onset_lead_in(plans: &mut [NotePlan], emphasis: f32, scale: f64) {
    if scale <= 0.0 {
        return;
    }
    for i in 1..plans.len() {
        let onset_target: f64 = plans[i]
            .onset()
            .iter()
            .map(|p| target_duration_sec(p, emphasis))
            .sum();
        if onset_target <= 0.0 {
            continue;
        }
        let prev = &plans[i - 1];
        if prev.ap_sec > 0.0 || prev.sp_sec > 0.0 {
            continue;
        }
        let prev_phoneme_sec = prev.phoneme_sec();
        let slack = (prev_phoneme_sec - min_articulation_sec(prev.phonemes)).max(0.0);
        let lead = (onset_target * scale)
            .min(slack)
            .min(prev_phoneme_sec * 0.30);
        if lead <= 0.0 {
            continue;
        }
        plans[i - 1].sing_sec -= lead;
        plans[i].sing_sec += lead;
    }
}

/// Split `slot_sec` across `phonemes`, returning one duration per phone
/// that sums to exactly `slot_sec`.
///
/// The old allocator gave every consonant the same slice, capped at
/// `slot / 2 / n_consonants`. On a phoneme-dense note that cap dominated:
/// `"resolution"` (nine phonemes) on a 214 ms note left each consonant
/// 21 ms and each vowel 27 ms — an order of magnitude under what either
/// needs to be identified, so the whole word arrived as a smear. Three
/// changes fix that:
///
/// 1. **Per-class targets.** An `s` is given roughly twice a `t`'s slice,
///    because that is what it takes to hear the difference (see
///    [`super::super::phonemes::ArticulationClass`]).
/// 2. **Floors before fairness.** When the slot is tight, consonants are
///    interpolated down toward their floors rather than divided evenly to
///    nothing, and the vowel is held at its own floor instead of soaking
///    up the shortfall.
/// 3. **Honest overflow.** If even every floor doesn't fit, everything is
///    scaled proportionally — the note is genuinely too short, which is
///    exactly what `song.vocal`'s articulation report flags so the caller
///    can lengthen it or add a syllable break.
fn allocate_phoneme_durations(phonemes: &[&str], slot_sec: f64, emphasis: f32) -> Vec<f64> {
    if phonemes.is_empty() {
        return Vec::new();
    }
    let slot_sec = slot_sec.max(0.001);
    let n_vow = phonemes.iter().filter(|p| !g2p::is_consonant(p)).count();

    // All-consonant syllable (only reachable via a pathological override):
    // no nucleus to absorb the remainder, so distribute by target weight.
    if n_vow == 0 {
        let targets: Vec<f64> = phonemes
            .iter()
            .map(|p| target_duration_sec(p, emphasis))
            .collect();
        let total: f64 = targets.iter().sum();
        return targets.iter().map(|t| slot_sec * t / total).collect();
    }

    let cons_target: f64 = phonemes
        .iter()
        .filter(|p| g2p::is_consonant(p))
        .map(|p| target_duration_sec(p, emphasis))
        .sum();
    let cons_floor: f64 = phonemes
        .iter()
        .filter(|p| g2p::is_consonant(p))
        .map(|p| floor_duration_sec(p))
        .sum();
    let vow_floor: f64 = phonemes
        .iter()
        .filter(|p| !g2p::is_consonant(p))
        .map(|p| floor_duration_sec(p))
        .sum();

    // How far along the floor→target line the consonants land. 1.0 when
    // the note is roomy enough for every target; 0.0 when it can only
    // just clear the floors.
    let t = if cons_target + vow_floor <= slot_sec {
        1.0
    } else if cons_floor + vow_floor >= slot_sec {
        // Over-full: every phone at its floor still overruns the note.
        // Scale the floors down proportionally and let the report flag it.
        let scale = slot_sec / (cons_floor + vow_floor);
        return phonemes
            .iter()
            .map(|p| floor_duration_sec(p) * scale)
            .collect();
    } else {
        ((slot_sec - vow_floor - cons_floor) / (cons_target - cons_floor)).clamp(0.0, 1.0)
    };

    let mut out: Vec<f64> = phonemes
        .iter()
        .map(|p| {
            if g2p::is_consonant(p) {
                let floor = floor_duration_sec(p);
                floor + (target_duration_sec(p, emphasis) - floor) * t
            } else {
                0.0
            }
        })
        .collect();
    // The nucleus takes exactly what's left, split evenly between
    // multiple vowels (a diphthong the dict spells as two symbols, or a
    // syllable that ended up with more than one).
    let cons_total: f64 = out.iter().sum();
    let vow_each = ((slot_sec - cons_total) / n_vow as f64).max(0.0);
    for (d, p) in out.iter_mut().zip(phonemes.iter()) {
        if !g2p::is_consonant(p) {
            *d = vow_each;
        }
    }
    out
}

/// Append a rest entry (`AP`/`SP`) to every parallel array in `track`.
/// The leading pad, trailing pad, and long-gap rest insertions all
/// shared the same boilerplate before this helper.
fn push_rest_entry(track: &mut PhonemeTrack, params: &VocalParams, kind: &str, dur: f64) {
    track.ph_seq.push(voicebank_phoneme_name(params.voicebank, kind));
    track.ph_dur.push(dur);
    track.note_seq.push("rest".to_string());
    track.note_dur.push(dur);
    track.note_seq_midi.push(0);
    if let Some(id) = voicebank_language_id(params.voicebank, kind) {
        track.languages.push(id);
    }
    track.entry_note_velocity.push(0.0);
    track.entry_note_total_sec.push(0.0);
    track.entry_note_start_offset.push(0.0);
}

/// MIDI note → "C4" / "D#5" notation accepted by DiffSinger's
/// `note_seq`. Mirrors `note_name_to_midi`'s inverse semantics.
fn midi_to_diffsinger_note(midi: u8) -> String {
    resonance_music_theory::midi_note_name(midi)
}
