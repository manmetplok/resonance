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
    let consonant_emphasis = params.consonant_emphasis.clamp(0.0, 1.0) as f64;
    // Consonant target duration in seconds. `consonant_emphasis` slides
    // between a brisk 35 ms (low) and a deliberate 85 ms (high). Capped
    // later to half the note's duration so a fast syllable still has a
    // recognisable vowel.
    let cons_dur_target = 0.035 + 0.050 * consonant_emphasis;

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

    // Walk notes back-to-back. We never insert AP between adjacent
    // syllables — the reference fixtures (`twinkle.ds`,
    // `hello_tiger.ds`) keep phonemes flowing continuously and let
    // the model handle syllable boundaries naturally. Each note's
    // effective sing duration is the time *until the next note* (or
    // its stated `duration_ticks` for the final note), so any
    // articulation-trim gap is absorbed automatically. Real silences
    // (gaps > 0.4 s between consecutive notes, which only happens at
    // genuine breath / rest points) still become an explicit AP.
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
        let fallback = vec!["ah"];
        let phonemes: &[&'static str] = if assignment.phonemes.is_empty() {
            &fallback
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
        let phon_sing_sec = (sing_sec - sp_sec).max(0.05);

        // Split `phon_sing_sec` across phonemes: each consonant gets
        // up to `cons_dur_target`, capped so consonants never eat
        // more than half the syllable. The vowel(s) absorb the
        // remainder evenly.
        let n_cons = phonemes.iter().filter(|p| g2p::is_consonant(p)).count();
        let n_vow = phonemes.len().saturating_sub(n_cons).max(1);
        let cons_total_cap = phon_sing_sec * 0.5;
        let cons_each = if n_cons > 0 {
            (cons_dur_target).min(cons_total_cap / n_cons as f64)
        } else {
            0.0
        };
        let vow_total = (phon_sing_sec - cons_each * n_cons as f64).max(0.05);
        let vow_each = vow_total / n_vow as f64;

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
            if stop_closure_sec > 0.0 && is_stop && ph_idx > 0 {
                let cl_dur = stop_closure_sec.min(cons_each * 0.4);
                track.ph_seq.push(voicebank_phoneme_name(params.voicebank, "cl"));
                track.ph_dur.push(cl_dur);
                track.note_seq.push(note_name.clone());
                track.note_dur.push(cl_dur);
                track.note_seq_midi.push(n.note as i32);
                if let Some(id) = voicebank_language_id(params.voicebank, "cl") {
                    track.languages.push(id);
                }
            }
            let mut d = if g2p::is_consonant(ph) {
                cons_each
            } else {
                vow_each
            };
            // Subtract the borrowed closure time so total syllable
            // duration stays the same.
            if stop_closure_sec > 0.0 && is_stop && ph_idx > 0 {
                d = (d - stop_closure_sec.min(cons_each * 0.4)).max(0.005);
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
