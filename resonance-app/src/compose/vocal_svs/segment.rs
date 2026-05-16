//! `DsSegment` builder. Translates the section's MIDI notes + lyric
//! draft into the score format the DiffSinger pipeline consumes:
//! phoneme sequence, per-phoneme durations, piecewise-constant f0 curve
//! (with portamento + vibrato), and the optional tension curve for
//! voicebanks that accept it.

use resonance_audio::types::MidiNote;
use resonance_music_theory::{g2p, VocalParams, VocalTimbre};
use svs_poc::ds::{DsSegment, SampleCurve};

use super::paths::{
    substitute_phoneme, voicebank_language_id, voicebank_phoneme_name, voicebank_supports_tension,
};
use super::SEGMENT_PAD_SEC;

/// Build a single `DsSegment` covering every note in the clip. Each
/// note's syllable text (drawn from `params.draft`) is run through G2P
/// to produce ARPAbet phonemes; the note's duration is split between
/// them with consonants getting a short slice and the vowel getting
/// the remainder. `f0_seq` is sampled at a fixed `f0_timestep` interval
/// over the whole segment.
pub(super) fn build_segment(
    notes: &[MidiNote],
    params: &VocalParams,
    lyrics: &[String],
    ticks_per_quarter: u32,
    bpm: f32,
) -> DsSegment {
    // Seconds per tick at the section's tempo. Vocal-lane MIDI clips use
    // `TICKS_PER_QUARTER_NOTE` as their tick rate, same as everywhere else
    // in the app.
    let seconds_per_tick = 60.0 / (bpm.max(1.0) as f64 * ticks_per_quarter as f64);

    // Resolve every note's (label, phonemes, is_slur, is_word_end)
    // via the shared cursor walker in music-theory. Same helper drives
    // the on-note lyric labels and the phoneme strip in the vocal
    // roll, so what the user sees and what the model sings can never
    // disagree on syllable-to-note assignment.
    let resolved = g2p::resolve_draft(&params.draft);
    let mut assigned = g2p::assign_syllables_to_notes(&resolved, lyrics, notes.len());
    // Apply voicebank-specific phoneme substitutions (e.g. Lilia's
    // `aa` → `a`). The resolver returns the canonical ARPAbet form;
    // each voicebank rewrites a small set of names at the SVS boundary.
    for a in assigned.iter_mut() {
        for p in a.phonemes.iter_mut() {
            *p = substitute_phoneme(params.voicebank, *p);
        }
    }
    // Optional word-boundary SP injection. Off by default (the
    // reference DiffSinger fixtures intentionally flow phonemes
    // continuously). Set RESONANCE_WORD_BOUNDARY_SP_MS=N to insert
    // ~N ms of SP at the end of each word's last syllable for an A/B
    // listening test. Practical range: 20-80 ms.
    let word_boundary_sp_sec: f64 = std::env::var("RESONANCE_WORD_BOUNDARY_SP_MS")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|ms| (ms / 1000.0).max(0.0).min(0.2))
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
        .map(|ms| (ms / 1000.0).max(0.0).min(0.05))
        .unwrap_or(0.0);
    let consonant_emphasis = params.consonant_emphasis.clamp(0.0, 1.0) as f64;
    // Consonant target duration in seconds. `consonant_emphasis` slides
    // between a brisk 35 ms (low) and a deliberate 85 ms (high). Capped
    // later to half the note's duration so a fast syllable still has a
    // recognisable vowel.
    let cons_dur_target = 0.035 + 0.050 * consonant_emphasis;

    let mut ph_seq: Vec<String> = Vec::new();
    let mut ph_dur: Vec<f64> = Vec::new();
    let mut note_seq: Vec<String> = Vec::new();
    let mut note_dur: Vec<f64> = Vec::new();
    let mut note_seq_midi: Vec<i32> = Vec::new();
    // Per-token language ids (parallel to ph_seq). Only populated when
    // the voicebank exposes a `languages` ONNX input (Meiji); empty for
    // TIGER and Lilia, which the pipeline interprets as "skip this
    // input".
    let mut languages: Vec<i64> = Vec::new();
    // Per-entry note metadata, parallel to ph_dur. Drives the
    // dynamic tension curve (per-syllable velocity) and the vibrato
    // gate (which only applies vibrato to longer notes after a brief
    // onset delay).
    //
    // For each phoneme entry of a note: store the note's velocity,
    // the note's total sing duration, and how far into the note the
    // entry starts. AP entries (rests) get sentinel values that
    // disable both the tension modulator and vibrato.
    let mut entry_note_velocity: Vec<f32> = Vec::new();
    let mut entry_note_total_sec: Vec<f64> = Vec::new();
    let mut entry_note_start_offset: Vec<f64> = Vec::new();

    // Leading silence pad. The hand-crafted reference fixtures all
    // start with a 0.3 s `AP` so the model has time to ramp up cleanly;
    // skipping this produces an attack click on the first phoneme.
    ph_seq.push(voicebank_phoneme_name(params.voicebank, "AP"));
    ph_dur.push(SEGMENT_PAD_SEC);
    note_seq.push("rest".to_string());
    note_dur.push(SEGMENT_PAD_SEC);
    note_seq_midi.push(0);
    if let Some(id) = voicebank_language_id(params.voicebank, "AP") {
        languages.push(id);
    }
    entry_note_velocity.push(0.0);
    entry_note_total_sec.push(0.0);
    entry_note_start_offset.push(0.0);

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
        let (sing_sec, ap_sec) = if slot_sec > sing_sec_cap + 0.4 {
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

        // Word-boundary SP: when this syllable is the LAST one of its
        // word, reserve a small silence at the end of the singing
        // slot. The reference DiffSinger fixtures don't insert SP
        // between words, so this is opt-in — env-var-gated for A/B
        // testing. `AssignedSyllable::is_word_end` is already `false`
        // for slur notes, so no extra check needed here.
        let inject_sp = word_boundary_sp_sec > 0.0
            && assignment.is_word_end
            && assignment.syllable_index + 1 < resolved.len();
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
                ph_seq.push(voicebank_phoneme_name(params.voicebank, "cl"));
                ph_dur.push(cl_dur);
                note_seq.push(note_name.clone());
                note_dur.push(cl_dur);
                note_seq_midi.push(n.note as i32);
                if let Some(id) = voicebank_language_id(params.voicebank, "cl") {
                    languages.push(id);
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
            ph_seq.push(voicebank_phoneme_name(params.voicebank, ph));
            ph_dur.push(d);
            note_seq.push(note_name.clone());
            note_dur.push(d);
            note_seq_midi.push(n.note as i32);
            if let Some(id) = voicebank_language_id(params.voicebank, ph) {
                languages.push(id);
            }
            entry_note_velocity.push(n.velocity);
            entry_note_total_sec.push(sing_sec);
            entry_note_start_offset.push(offset_in_note);
            offset_in_note += d;
        }
        if sp_sec > 0.0 {
            // Insert SP within the same note's slot — the syllable's
            // pitch carries through the brief silence.
            ph_seq.push(voicebank_phoneme_name(params.voicebank, "SP"));
            ph_dur.push(sp_sec);
            note_seq.push(note_name.clone());
            note_dur.push(sp_sec);
            note_seq_midi.push(n.note as i32);
            if let Some(id) = voicebank_language_id(params.voicebank, "SP") {
                languages.push(id);
            }
            entry_note_velocity.push(n.velocity);
            entry_note_total_sec.push(sing_sec);
            entry_note_start_offset.push(offset_in_note);
            // No further entries belong to this note after the SP, so
            // we don't carry the cumulative offset forward.
        }

        if ap_sec > 0.0 {
            ph_seq.push(voicebank_phoneme_name(params.voicebank, "AP"));
            ph_dur.push(ap_sec);
            note_seq.push("rest".to_string());
            note_dur.push(ap_sec);
            note_seq_midi.push(0);
            if let Some(id) = voicebank_language_id(params.voicebank, "AP") {
                languages.push(id);
            }
            entry_note_velocity.push(0.0);
            entry_note_total_sec.push(0.0);
            entry_note_start_offset.push(0.0);
        }
    }

    // Trailing silence pad, mirroring the leading AP.
    ph_seq.push(voicebank_phoneme_name(params.voicebank, "AP"));
    ph_dur.push(SEGMENT_PAD_SEC);
    note_seq.push("rest".to_string());
    note_dur.push(SEGMENT_PAD_SEC);
    note_seq_midi.push(0);
    if let Some(id) = voicebank_language_id(params.voicebank, "AP") {
        languages.push(id);
    }
    entry_note_velocity.push(0.0);
    entry_note_total_sec.push(0.0);
    entry_note_start_offset.push(0.0);

    // f0_seq: piecewise constant pitch following the note sequence. The
    // pipeline resamples this to its internal frame rate; we just need a
    // grid dense enough to capture every note boundary.
    let f0_timestep = 0.005_f64;
    let total_sec: f64 = ph_dur.iter().sum();
    let n_samples = (total_sec / f0_timestep).ceil() as usize + 1;
    let mut f0_samples = Vec::with_capacity(n_samples);
    // Parallel per-frame metadata for the dynamic tension curve and
    // vibrato gate. Filled in lockstep with `f0_samples` so each
    // frame knows its parent note's velocity, total duration, and
    // how far we are into the note.
    let mut frame_velocity: Vec<f32> = Vec::with_capacity(n_samples);
    let mut frame_note_total_sec: Vec<f64> = Vec::with_capacity(n_samples);
    let mut frame_in_note_sec: Vec<f64> = Vec::with_capacity(n_samples);
    let mut t = 0.0;
    let mut idx = 0;
    let mut accum = note_dur.first().copied().unwrap_or(0.0);
    for _ in 0..n_samples {
        while t > accum && idx + 1 < note_dur.len() {
            idx += 1;
            accum += note_dur[idx];
        }
        let midi = note_seq_midi.get(idx).copied().unwrap_or(0);
        let hz = if midi <= 0 { 0.0 } else { midi_to_hz(midi as u8) };
        f0_samples.push(hz);
        // Per-frame metadata: note velocity / duration / elapsed.
        let vel = entry_note_velocity.get(idx).copied().unwrap_or(0.0);
        let nts = entry_note_total_sec.get(idx).copied().unwrap_or(0.0);
        let entry_start_t = accum - note_dur[idx];
        let elapsed_in_entry = (t - entry_start_t).max(0.0);
        let offset = entry_note_start_offset.get(idx).copied().unwrap_or(0.0);
        frame_velocity.push(vel);
        frame_note_total_sec.push(nts);
        frame_in_note_sec.push(offset + elapsed_in_entry);
        t += f0_timestep;
    }
    // Fill unvoiced frames (rests, leading/trailing AP) with a
    // continuous carrier pitch. The reference fixtures keep f0 > 0
    // throughout the segment — silence is signalled by the phoneme
    // being "AP", not by f0 being zero. Zeroing f0 instead causes the
    // vocoder to emit subtle noise during the silence pads (the user's
    // "noise in silent parts" report). Forward-fill from the next
    // voiced frame for the leading pad, then back-fill from the
    // previous voiced frame for everything else.
    let first_voiced_idx = f0_samples.iter().position(|v| *v > 0.0);
    if let Some(first_idx) = first_voiced_idx {
        let leading_hz = f0_samples[first_idx];
        for v in f0_samples.iter_mut().take(first_idx) {
            *v = leading_hz;
        }
        let mut last_voiced = leading_hz;
        for v in f0_samples.iter_mut().skip(first_idx) {
            if *v > 0.0 {
                last_voiced = *v;
            } else {
                *v = last_voiced;
            }
        }
    }

    // Smooth f0 step jumps between adjacent voiced notes with a brief
    // linear portamento. The reference fixtures train the model on
    // real human pitch curves that always slide between notes, so
    // hard pitch steps at every syllable boundary push the acoustic
    // model into a regime it doesn't render cleanly. The user controls
    // the slide duration (10..200 ms in the inspector); 0 disables
    // portamento entirely (hard step, only useful for stylistic
    // hard-attack effects). Skips frames that are exactly equal to
    // the previous (no slide needed).
    let portamento_sec = (params.portamento_ms.clamp(0.0, 250.0) as f64) / 1000.0;
    let portamento_frames = (portamento_sec / f0_timestep).round() as usize;
    if portamento_frames >= 2 && f0_samples.len() > portamento_frames {
        let snapshot = f0_samples.clone();
        let mut last_change_idx = 0usize;
        let mut last_val = snapshot[0];
        for (i, &cur) in snapshot.iter().enumerate().skip(1) {
            if (cur - last_val).abs() > 0.5 {
                // Pitch change detected at index i. Linearly ramp the
                // previous `portamento_frames` from `last_val` (the
                // pre-change pitch) to `cur`.
                let start = i.saturating_sub(portamento_frames).max(last_change_idx);
                let span = i.saturating_sub(start);
                if span >= 1 {
                    for (offset, sample) in f0_samples[start..i].iter_mut().enumerate() {
                        let t = (offset + 1) as f64 / (span + 1) as f64;
                        *sample = last_val * (1.0 - t) + cur * t;
                    }
                }
                last_val = cur;
                last_change_idx = i;
            }
        }
    }

    // Vibrato: sinusoidal modulation of the f0 curve. Rate (4–7 Hz)
    // is user-controlled via `vibrato_rate`; depth scales peak
    // deviation up to ~20 cents at max. Real singers don't apply
    // vibrato to short syllables and let it ramp in after the
    // consonant attack, so we gate two ways:
    //   1. Skip notes whose total sing duration is below
    //      `VIBRATO_MIN_NOTE_SEC` — too short for vibrato to make
    //      musical sense (it'd just sound like a wobble on the
    //      consonant).
    //   2. Within longer notes, fade vibrato in over
    //      `VIBRATO_ONSET_SEC` after the note's start so the
    //      consonant attack stays clean.
    const VIBRATO_MIN_NOTE_SEC: f64 = 0.35;
    const VIBRATO_ONSET_SEC: f64 = 0.15;
    let vibrato_depth = params.vibrato.clamp(0.0, 1.0) as f64;
    if vibrato_depth > 0.001 {
        let max_cents = 20.0_f64;
        let rate_hz = params.vibrato_rate.clamp(2.0, 10.0) as f64;
        let two_pi = std::f64::consts::TAU;
        for (i, v) in f0_samples.iter_mut().enumerate() {
            if *v <= 0.0 {
                continue;
            }
            let note_dur_s = frame_note_total_sec.get(i).copied().unwrap_or(0.0);
            if note_dur_s < VIBRATO_MIN_NOTE_SEC {
                continue;
            }
            let elapsed = frame_in_note_sec.get(i).copied().unwrap_or(0.0);
            let onset_gain = (elapsed / VIBRATO_ONSET_SEC).clamp(0.0, 1.0);
            if onset_gain <= 0.0 {
                continue;
            }
            let t = i as f64 * f0_timestep;
            let cents =
                max_cents * vibrato_depth * onset_gain * (two_pi * rate_hz * t).sin();
            *v *= 2.0_f64.powf(cents / 1200.0);
        }
    }

    // Gender curve maps to the acoustic model's `gender` ONNX input,
    // which shifts formants brighter / darker (range [-1, +1], 0 =
    // neutral). The dsconfig's `use_key_shift_embed` flag is unrelated
    // — that's about training-time pitch-shift augmentation, not a
    // runtime input. Other per-frame curves (`energy`, `breathiness`,
    // `voicing`, `tension`) aren't accepted by the TIGER model and are
    // left as `SampleCurve::default()`. The `timbre` chip selects a
    // landmark on the brightness axis; the curve is constant across
    // the segment so the formant character stays consistent.
    //
    // Empirically-tuned band, characterised against TIGER (the
    // tightest of the three voicebanks): the negative side has a hard
    // ceiling around `-0.20`, and the positive side starts losing
    // intelligibility past about `+0.35` — whisper transcribes a
    // `+0.50` Bright TIGER as "my my my" instead of the test lyric.
    // Lilia and Meiji are robust across the band. If you widen, do
    // it positive-side only and re-run the sweep harness to confirm
    // intelligibility doesn't collapse.
    let curve_len = f0_samples.len();
    let gender_value = match params.timbre {
        VocalTimbre::Warm => -0.15,
        VocalTimbre::Edged => -0.05,
        VocalTimbre::Airy => 0.20,
        VocalTimbre::Bright => 0.30,
    };
    let gender = SampleCurve {
        samples: vec![gender_value; curve_len],
        timestep: f0_timestep,
    };

    // NOTE on `velocity`: TIGER does accept a per-frame `velocity`
    // input, but in DiffSinger semantics velocity is a *phoneme-
    // duration* multiplier (>1.0 shortens, <1.0 lengthens), not the
    // attack-strength knob it sounds like. Feeding non-1.0 values
    // smeared the rendered audio down to ~-60 dB during testing, so
    // we deliberately leave it as default (the pipeline fills with
    // 1.0 internally). The per-syllable velocities computed by
    // `derive_vocal` still drive MIDI clip dynamics; bridging them
    // into the SVS model needs a different parameter (and probably
    // training-set characterisation) than this knob provides.

    // Tension curve maps to the `tension` ONNX input on voicebanks
    // that expose it (Lilia, Meiji). Range [-1, +1]: -1 = relaxed /
    // breathy delivery, 0 = neutral, +1 = compressed / belted.
    // TIGER doesn't accept tension (the pipeline's `flags.tension`
    // will be false for that voicebank, so the curve is ignored).
    //
    // Two modulators add per-frame movement to the slider baseline:
    //   - Velocity: strong-beat syllables (higher per-note velocity
    //     from `derive_vocal`) push tension up; weak ones push down.
    //   - Contour: notes near the top of the section's pitch range
    //     push tension up (singers belt at the top of their range);
    //     notes near the bottom push down.
    // Each modulator's strength is its own slider in [0, 1] so the
    // user can dial in either, both, or neither.
    let tension = if voicebank_supports_tension(params.voicebank) {
        let base = params.tension.clamp(-1.0, 1.0) as f64;
        let vel_amount = params.tension_velocity_amount.clamp(0.0, 1.0) as f64;
        let contour_amount = params.tension_contour_amount.clamp(0.0, 1.0) as f64;
        // Section pitch range, used to normalise the contour
        // contribution. Use the f0 sample range (excluding silence
        // fill) so the modulation is per-section rather than global.
        let (mut min_hz, mut max_hz) = (f64::INFINITY, 0.0_f64);
        for (i, &v) in f0_samples.iter().enumerate() {
            if frame_note_total_sec.get(i).copied().unwrap_or(0.0) > 0.0 && v > 0.0 {
                if v < min_hz {
                    min_hz = v;
                }
                if v > max_hz {
                    max_hz = v;
                }
            }
        }
        let mid_hz = (min_hz + max_hz) * 0.5;
        let half_range_hz = ((max_hz - min_hz) * 0.5).max(1.0);
        let mut samples = Vec::with_capacity(curve_len);
        for i in 0..curve_len {
            // Velocity modulation: derive_vocal's neutral velocity is
            // ~0.78 with strong beats around 0.86. Map to roughly
            // [-1, +1] around neutral, then scale by amount and
            // contribute up to ±0.5.
            let vel = frame_velocity.get(i).copied().unwrap_or(0.0) as f64;
            let vel_mod = if vel > 0.0 {
                ((vel - 0.78) / 0.22).clamp(-1.0, 1.0)
            } else {
                0.0
            };
            // Pitch contour modulation: position within section's f0
            // range, mapped to [-1, +1]. Silence frames contribute 0.
            let pitch = f0_samples[i];
            let in_voiced =
                frame_note_total_sec.get(i).copied().unwrap_or(0.0) > 0.0 && pitch > 0.0;
            let pitch_mod = if in_voiced {
                ((pitch - mid_hz) / half_range_hz).clamp(-1.0, 1.0)
            } else {
                0.0
            };
            let t = (base + vel_amount * vel_mod * 0.5 + contour_amount * pitch_mod * 0.5)
                .clamp(-1.0, 1.0);
            samples.push(t);
        }
        SampleCurve {
            samples,
            timestep: f0_timestep,
        }
    } else {
        SampleCurve::default()
    };

    DsSegment {
        offset: 0.0,
        ph_seq,
        ph_dur,
        ph_num: Vec::new(),
        note_seq_midi,
        note_dur,
        note_slur: Vec::new(),
        f0: SampleCurve {
            samples: f0_samples,
            timestep: f0_timestep,
        },
        gender,
        velocity: SampleCurve::default(),
        energy: SampleCurve::default(),
        breathiness: SampleCurve::default(),
        voicing: SampleCurve::default(),
        tension,
        languages,
    }
}

/// MIDI note → "C4" / "D#5" / "Bb3" notation accepted by DiffSinger's
/// `note_seq`. Mirrors `note_name_to_midi`'s inverse semantics.
fn midi_to_diffsinger_note(midi: u8) -> String {
    const SHARP: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (midi as i32 / 12) - 1;
    let pc = midi as usize % 12;
    format!("{}{}", SHARP[pc], octave)
}

fn midi_to_hz(midi: u8) -> f64 {
    // A4 (MIDI 69) = 440 Hz.
    440.0 * (2.0_f64).powf((midi as f64 - 69.0) / 12.0)
}
