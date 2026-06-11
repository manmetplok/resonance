// Top-level motif-based melody generators. Wire together motif
// construction, phrase planning, and harmony alignment.

use crate::chord::Chord;
use crate::rng::XorShift;
use crate::scale::Scale;

use super::super::climax::{
    demote_at_or_above, enforce_single_climax, restore_strong_beat_contract,
    section_peak_margin, ClimaxHarmony, SectionClimaxRule,
};
use super::super::melody::MelodyParams;
use super::super::motif_source::{manual_motif_to_motif_notes, MotifParams, MotifSource};
use super::super::{GeneratedNote, TimedChord};
use super::build::build_motif;
use super::cadence::apply_cadence_formula;
use super::embellish::{apply_embellishments, resolve_embellishment_style};
use super::harmony::{apply_leap_recovery, HarmonyGrid};
use super::phrase::{
    plan_motif_transforms, plan_phrases, realize_phrase, section_cap_sources,
    section_climax_phrase, PhraseRenderCtx,
};
use super::types::PhraseGrammarRole;

/// Extract the motif's signed semitone intervals (relative to its
/// anchor pitch), skipping rests. Used by lanes that don't render the
/// motif themselves but want to trace its melodic shape — e.g. the
/// vocal generator's "use section motif" mode.
///
/// `Generated` motifs are built with `build_motif` using the same RNG
/// flow as the melody renderer so the returned intervals match what
/// the motif lanes produce. `Manual` motifs are read directly from the
/// user-drawn cells via the existing scale-step mapping.
pub fn motif_intervals(
    source: &MotifSource,
    anchor_chord: Chord,
    scale: Option<Scale>,
) -> Vec<i8> {
    let notes = match source {
        MotifSource::Generated(p) => {
            let mut rng = XorShift::new(p.seed);
            build_motif(&mut rng, anchor_chord, scale, p)
        }
        MotifSource::Manual { notes, .. } => manual_motif_to_motif_notes(notes, scale),
    };
    notes
        .iter()
        .filter(|n| !n.silent)
        .map(|n| n.interval)
        .collect()
}

/// Top-level motif-based melody generator.
///
/// Back-compat shim: pulls motif knobs from `MelodyParams`. Direct callers
/// (and the inline tests) keep working unchanged. The app routes through
/// [`derive_motif_melody_with_section`] instead so the section's
/// `MotifSource` wins.
pub(in crate::derive) fn derive_motif_melody(
    chords: &[TimedChord],
    scale: Option<Scale>,
    params: &MelodyParams,
    ticks_per_beat: u32,
    seed: u64,
) -> Vec<GeneratedNote> {
    let source = MotifSource::Generated(MotifParams {
        seed,
        complexity: params.complexity,
        motif_len: params.motif_len,
        leap_chance: params.leap_chance,
    });
    derive_motif_melody_with_section(chords, scale, params, &source, seed, ticks_per_beat)
}

/// Section-aware motif-based melody generator.
///
/// In `MotifSource::Generated` mode, `motif.seed` drives the shared motif
/// (intervals + rhythm + accents) and the per-phrase Transform sequence —
/// both shared across all Motif lanes in a section. In `Manual` mode, the
/// motif cell is taken verbatim from the user-drawn notes and the seed
/// only drives the per-phrase Transform sequence so the motif still
/// develops across phrases.
///
/// `lane_seed` drives lane-local randomness only: phrase contour selection
/// (when `params.contour == Auto`) and rest-density hole placement.
/// Pressing Regenerate on a single lane should bump `lane_seed` so the
/// motif identity stays put while the lane gets a fresh surface variation.
pub fn derive_motif_melody_with_section(
    chords: &[TimedChord],
    scale: Option<Scale>,
    params: &MelodyParams,
    motif_source: &MotifSource,
    lane_seed: u64,
    ticks_per_beat: u32,
) -> Vec<GeneratedNote> {
    if chords.is_empty() {
        return Vec::new();
    }
    let tpb = ticks_per_beat as u64;

    let motif_params = motif_source.params();
    let motif = match motif_source {
        MotifSource::Generated(p) => {
            let mut motif_rng = XorShift::new(p.seed);
            build_motif(&mut motif_rng, chords[0].chord, scale, p)
        }
        MotifSource::Manual { notes, .. } => manual_motif_to_motif_notes(notes, scale),
    };
    if motif.is_empty() {
        return Vec::new();
    }

    let mut lane_rng = XorShift::new(lane_seed);
    let phrases = plan_phrases(
        chords,
        params.contour,
        params.phrase_len,
        motif_params.seed,
        &mut lane_rng,
    );
    let transforms = plan_motif_transforms(
        phrases.len(),
        motif.len(),
        motif_params.complexity,
        motif_params.seed,
    );

    // Section climax plan: one phrase per section carries the global
    // peak; consequents share their antecedent's treatment so periods
    // stay parallel.
    let roles: Vec<PhraseGrammarRole> = phrases.iter().map(|p| p.role).collect();
    let carrier = section_climax_phrase(&roles);
    let cap_sources = section_cap_sources(&roles);

    // Per-phrase octave displacement keeps the motif identity intact
    // (same intervals + rhythm) while giving each Regenerate press an
    // audible shift. Without this, lane_seed only nudges contour and
    // rest-density randomization — invisible when the user pinned a
    // specific ContourPreference and rest_density sits at its default 0.
    let mut phrase_octave_offsets: Vec<i8> = (0..phrases.len())
        .map(|i| {
            if i == 0 {
                return 0;
            }
            let roll = lane_rng.next_f32();
            if roll < 0.55 {
                0
            } else if roll < 0.85 {
                12
            } else {
                -12
            }
        })
        .collect();
    // The climax carrier never drops an octave — the designated phrase
    // must stay on top of the section — and consequents reuse their
    // antecedent's displacement so the period's "same opening" survives
    // the section pass (the pair would otherwise be demoted apart).
    if let Some(off) = phrase_octave_offsets.get_mut(carrier) {
        *off = (*off).max(0);
    }
    for i in 0..phrase_octave_offsets.len() {
        let src = cap_sources[i];
        if src != i {
            phrase_octave_offsets[i] = phrase_octave_offsets[src];
        }
    }

    let mut all_notes = Vec::new();
    let rest_gap = (tpb as f64 * (0.5 + params.rest_density as f64)) as u64;

    let render_ctx = PhraseRenderCtx {
        chords,
        scale,
        register: params.register,
        articulation: params.articulation,
        velocity_base: params.velocity,
        tpb,
    };

    // Phase 1 — realize every phrase and settle its *local* melodic
    // grammar (leap rules + per-phrase single climax), then apply the
    // per-phrase octave displacement. Runs before the section pass so
    // the section sees each phrase's true peak.
    let mut per_phrase: Vec<Vec<GeneratedNote>> = Vec::with_capacity(phrases.len());
    for (pi, phrase) in phrases.iter().enumerate() {
        let mut phrase_notes = realize_phrase(&motif, transforms[pi], phrase, &render_ctx);

        if let Some(scale) = scale {
            let grid = HarmonyGrid { chords, tpb };
            apply_leap_recovery(&mut phrase_notes, &scale, params.register, &grid);
            // Single-climax rule: one highest note per phrase, in its
            // second half, never the final note. Climax demotion and
            // the leap grammar alternate to a fixpoint: demotion only
            // lowers pitches and the grammar's repairs never lift a
            // pitch back up to the phrase maximum, so the maximum is
            // non-increasing and the loop settles; the cap is
            // belt-and-braces.
            let harmony = ClimaxHarmony {
                chords,
                tpb,
                register: params.register,
            };
            for _ in 0..32 {
                if !enforce_single_climax(
                    &mut phrase_notes,
                    Some(scale),
                    params.register,
                    Some(&harmony),
                    true,
                    true,
                ) {
                    break;
                }
                apply_leap_recovery(&mut phrase_notes, &scale, params.register, &grid);
            }
        }

        let octave_shift = phrase_octave_offsets[pi];
        if octave_shift != 0 {
            // All-or-nothing, like the bass lane's `shifted_anchor`:
            // shifting only the notes that stay in register would
            // rewrite adjacent intervals (a validated step resolution
            // becomes a 7th/9th), silently breaking the leap grammar
            // and strong-beat contracts the passes above enforced. If
            // any note would leave the register, the whole phrase
            // keeps its anchor octave.
            let fits = phrase_notes.iter().all(|n| {
                let candidate = n.note as i16 + octave_shift as i16;
                (params.register.0 as i16..=params.register.1 as i16).contains(&candidate)
            });
            if fits {
                for n in phrase_notes.iter_mut() {
                    n.note = (n.note as i16 + octave_shift as i16) as u8;
                }
            }
        }

        per_phrase.push(phrase_notes);
    }

    // Phase 2 — section-level climax orchestration (Open Music Theory
    // v2: one climax per *section*): the designated carrier phrase
    // keeps the section's highest note; every other phrase's peak is
    // demoted strictly below it (a seeded per-group margin keeps the
    // secondary-peak skyline varied). The consequent paired with the
    // carrier antecedent may tie — a period restates its material —
    // but never exceed. Demote-only, like the per-phrase pass, so the
    // local grammar settles back via the same fixpoint.
    let mut rules: Vec<SectionClimaxRule> = vec![SectionClimaxRule::Free; phrases.len()];
    if let Some(scale) = scale {
        let lo = params.register.0;
        let peak = per_phrase
            .get(carrier)
            .and_then(|pn| pn.iter().map(|n| n.note).max());
        // Headroom guard: demotion needs room below the cap — an
        // octave between the carrier's peak and the register floor
        // guarantees a chord tone exists under every demotion tier
        // (the widest gap between adjacent triad tones is a fourth),
        // so strong-beat demotion never has to fall through to a
        // dissonant scale tone that the floor then pins in place.
        // Sections riding the bottom of their register skip the
        // section pass instead of being squashed flat against it.
        if let Some(peak) = peak.filter(|&p| phrases.len() > 1 && p >= lo + 12) {
            let harmony = ClimaxHarmony {
                chords,
                tpb,
                register: params.register,
            };
            for (pi, pn) in per_phrase.iter_mut().enumerate() {
                if pi == carrier {
                    rules[pi] = SectionClimaxRule::Carrier { peak };
                    continue;
                }
                let cap = if cap_sources[pi] == carrier {
                    // The carrier's own consequent restates its
                    // material: it may tie the peak but not exceed it.
                    peak.saturating_add(1)
                } else {
                    let margin = section_peak_margin(motif_params.seed, pi / 4);
                    peak.saturating_sub(margin).max(lo + 1)
                };
                rules[pi] = SectionClimaxRule::Capped { cap };
                let Some(pmax) = pn.iter().map(|n| n.note).max() else {
                    continue;
                };
                if pmax < cap {
                    continue;
                }
                // Whole-phrase octave drop first: interval- and
                // pitch-class-preserving, so every per-phrase contract
                // (leap grammar, strong-beat chord tones, climax
                // placement) survives untouched.
                let fits = pn.iter().all(|n| n.note as i16 - 12 >= lo as i16);
                if fits && (pmax as i16 - 12) < cap as i16 {
                    for n in pn.iter_mut() {
                        n.note -= 12;
                    }
                    continue;
                }
                if demote_at_or_above(
                    pn,
                    cap,
                    None,
                    Some(scale),
                    params.register,
                    Some(&harmony),
                    9,
                ) {
                    // Settle the duplicate maxima demotion can leave
                    // (early tie-break keeps the repaired climax clear
                    // of the cadence tail). Both passes are demote-only
                    // and chord-tone aware on strong beats. The
                    // leap-recovery pass is deliberately *not* re-run
                    // here: its repairs are chord-tone-blind and, on
                    // the plateaus a deep demotion leaves, they drag
                    // strong-beat notes off their chord tones.
                    // Demotion itself can't widen any interval (every
                    // move is toward a lower neighbor), so the leap
                    // grammar holds up without it. What demotion *can*
                    // do is lower the resolution note of a legal
                    // strong-beat dissonance past its step —
                    // `restore_strong_beat_contract` then demotes the
                    // dissonance itself onto a chord tone. Alternated
                    // to a fixpoint: every pass only lowers pitches,
                    // so the loop settles.
                    for _ in 0..4 {
                        for _ in 0..8 {
                            if !enforce_single_climax(
                                pn,
                                Some(scale),
                                params.register,
                                Some(&harmony),
                                false,
                                false,
                            ) {
                                break;
                            }
                        }
                        if !restore_strong_beat_contract(pn, &harmony) {
                            break;
                        }
                    }
                }
            }
        }
    }

    // Phase 3 — per-phrase overlays (goal cadences, embellishing
    // tones) and assembly. Both overlays validate their candidates
    // against the phrase's section rule, so they cannot reintroduce a
    // peak phase 2 demoted (or rewrite the carrier's peak away).
    //
    // Embellishment flavor: resolved once per section from the motif
    // seed (when the lane asks for Auto) so every lane decorates with
    // the same vocabulary weighting.
    let emb_style = resolve_embellishment_style(params.embellishment, motif_params.seed);
    // A period's consequent reuses its antecedent's decoration stream
    // (same RNG seed): the consequent restates the antecedent's
    // opening — including its surface decorations — and only the
    // cadence ending differs.
    let mut last_antecedent_pi: Option<usize> = None;

    for (pi, phrase) in phrases.iter().enumerate() {
        let mut phrase_notes = std::mem::take(&mut per_phrase[pi]);

        if let Some(scale) = scale {
            let grid = HarmonyGrid { chords, tpb };
            // Goal-cadence targeting: rewrite the phrase's final two
            // notes to the planned HC/IAC/PAC (or deceptive) formula.
            // The overlay validates every candidate against the leap
            // grammar, the single-climax rule (per-phrase and
            // section), the dissonance discipline, and the strong-beat
            // chord-tone skeleton, so it composes with the passes
            // above instead of needing another fixpoint round.
            // Sentence presentation/continuation phrases plan no goal
            // (`cadence: None`) — they prolong without cadencing.
            if let Some(goal) = phrase.cadence {
                apply_cadence_formula(
                    &mut phrase_notes,
                    goal,
                    &chords[phrase.chord_range.0..phrase.chord_range.1],
                    &scale,
                    params.register,
                    tpb,
                    rules[pi],
                );
            }
            // Embellishing-tone decoration: re-classify the surface
            // from the OMT table (passing/neighbor on weak beats,
            // appoggiatura/suspension on strong beats, escape tone,
            // anticipation) with style-weighted probabilities. Runs
            // last and validates every candidate against the whole
            // phrase, protecting the cadence tail. Seeded lane-locally
            // per phrase: decoration is surface variation, so a
            // Regenerate press refreshes it while the motif identity
            // stays put. Consequents share their antecedent's stream
            // so the period's restated opening keeps its decorations.
            let dec_pi = match phrase.role {
                PhraseGrammarRole::Antecedent => {
                    last_antecedent_pi = Some(pi);
                    pi
                }
                PhraseGrammarRole::Consequent => last_antecedent_pi.unwrap_or(pi),
                _ => pi,
            };
            let mut dec_rng = XorShift::new(
                lane_seed ^ (dec_pi as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15),
            );
            apply_embellishments(
                &mut phrase_notes,
                &grid,
                &scale,
                params.register,
                emb_style,
                params.complexity,
                &mut dec_rng,
                rules[pi],
            );
        }

        if pi > 0 && rest_gap > 0 {
            if let Some(last) = all_notes.last_mut() {
                let last_note: &mut GeneratedNote = last;
                if last_note.duration_ticks > rest_gap {
                    last_note.duration_ticks -= rest_gap;
                }
            }
        }

        all_notes.extend(phrase_notes);
    }

    if params.rest_density > 0.0 {
        let mut filtered = Vec::with_capacity(all_notes.len());
        for note in all_notes {
            if lane_rng.next_f32() >= params.rest_density {
                filtered.push(note);
            }
        }
        all_notes = filtered;
    }

    all_notes
}
