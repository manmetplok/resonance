//! The GUI question "what about the vocal melody?" after a chord or
//! section-length edit, and the lane badge's actions
//! (vocal-melody-on-chord-change, BUG-70).
//!
//! # One undo entry
//!
//! The chord edit is not staged: it dispatches and records its undo
//! entry as any edit does, with every affected vocal lane **kept** (notes
//! untouched, lane flagged) — the safe outcome, and the one a control
//! call gets by default. Those lanes are parked in
//! `vocal_melody.undecided`; once the outermost GUI update that made them
//! is done ([`settle`]) — and, for a drag, once the gesture has committed
//! — the question opens, recording the undo history's generation.
//!
//! - **Apply** carries out each row's choice and marks the project edited
//!   *without* a history entry: the chord edit's entry snapshots the state
//!   before that edit, so one undo takes back the edit and its vocal
//!   outcome together, and redo (snapshotted lazily at undo time) brings
//!   both back.
//! - **Cancel** (Esc, the backdrop) undoes that entry and drops the redo
//!   step it leaves, so the chord edit is gone as if never made.
//! - The question is about the entry on top of the history. When the
//!   history moves without it — an undo, a control call's edit, a project
//!   load — the question closes, leaving the lanes kept and flagged.
//!
//! A drag (a Begin…Commit gesture, e.g. a signature drag trimming chords)
//! collects lanes across all its steps and asks once, after its commit.

use crate::compose::vocal_melody::{
    VocalMelodyChoice, VocalMelodyMsg, VocalMelodyPrompt, VocalMelodyPromptRow,
};

/// Handle a [`VocalMelodyMsg`].
pub(super) fn handle(r: &mut crate::Resonance, msg: VocalMelodyMsg) {
    match msg {
        VocalMelodyMsg::Refit {
            definition_id,
            track_id,
        } => {
            let _ = super::vocal_melody::refit_lane(r, definition_id, track_id);
        }
        VocalMelodyMsg::Regenerate {
            definition_id,
            track_id,
        } => super::vocal_render::rederive_vocal_melody(r, definition_id, track_id),
        VocalMelodyMsg::SetRowChoice { row, choice } => {
            if let Some(row) = prompt_mut(r).and_then(|p| p.rows.get_mut(row)) {
                row.choice = choice;
            }
            settle_remember(r);
        }
        VocalMelodyMsg::SetAllChoices(choice) => {
            if let Some(prompt) = prompt_mut(r) {
                for row in &mut prompt.rows {
                    row.choice = choice;
                }
            }
        }
        VocalMelodyMsg::ToggleRemember => {
            if let Some(prompt) = prompt_mut(r) {
                prompt.remember = !prompt.remember && prompt.uniform_choice().is_some();
            }
        }
        VocalMelodyMsg::Apply => apply(r),
        VocalMelodyMsg::Cancel => cancel(r),
        VocalMelodyMsg::ForgetRememberedChoice => r.compose.vocal_melody.remembered = None,
    }
}

fn prompt_mut(r: &mut crate::Resonance) -> Option<&mut VocalMelodyPrompt> {
    r.compose.vocal_melody.prompt.as_mut()
}

/// "Remember" needs one choice for every row; a row changed away from
/// the others unticks it.
fn settle_remember(r: &mut crate::Resonance) {
    if let Some(prompt) = prompt_mut(r) {
        if prompt.uniform_choice().is_none() {
            prompt.remember = false;
        }
    }
}

/// Carry out the rows' choices as the rest of the chord edit.
fn apply(r: &mut crate::Resonance) {
    let Some(prompt) = r.compose.vocal_melody.prompt.take() else {
        return;
    };
    let mut edited = false;
    for row in &prompt.rows {
        if row.choice != VocalMelodyChoice::Keep {
            super::vocal_melody::apply_outcome(r, row.definition_id, row.track_id, row.choice);
            edited = true;
        }
    }
    if let Some(choice) = prompt.remember.then(|| prompt.uniform_choice()).flatten() {
        r.compose.vocal_melody.remembered = Some(choice);
        edited = true;
    }
    if edited {
        // Part of the chord edit's entry, not one of its own (module docs).
        r.mark_edited_without_history();
    }
}

/// Take the chord edit back.
fn cancel(r: &mut crate::Resonance) {
    let Some(prompt) = r.compose.vocal_melody.prompt.take() else {
        return;
    };
    let on_top = prompt.undoable && prompt.history_generation == r.session.undo.generation();
    if on_top && r.try_undo().is_some() {
        r.session.undo.drop_redo_top();
        r.banners.note_history("Cancelled the chord change".to_owned());
    } else {
        r.banners
            .note_history("The chord change stays; vocal melodies were kept".to_owned());
    }
}

/// Called at the start of every outermost update: what an earlier update
/// left undecided was a control call's (it stays kept) unless a gesture
/// is still collecting.
pub(crate) fn begin_update(r: &mut crate::Resonance) {
    r.compose.vocal_melody.affected.clear();
    if !r.session.undo.has_pending() {
        r.compose.vocal_melody.undecided.clear();
    }
}

/// Called after every outermost update. Drops a question whose edit is no
/// longer on top of the history, and opens one for the lanes a GUI edit
/// left undecided once no gesture is open.
pub(crate) fn settle(r: &mut crate::Resonance, gui: bool) {
    let generation = r.session.undo.generation();
    if r
        .compose
        .vocal_melody
        .prompt
        .as_ref()
        .is_some_and(|p| p.history_generation != generation)
    {
        r.compose.vocal_melody.prompt = None;
        r.banners.note_history(
            "The project changed while asking about vocal melodies; they were kept".to_owned(),
        );
    }
    if r.session.undo.has_pending() {
        return;
    }
    let undecided = std::mem::take(&mut r.compose.vocal_melody.undecided);
    if !gui || undecided.is_empty() {
        return;
    }
    let mut rows: Vec<VocalMelodyPromptRow> = r
        .compose
        .vocal_melody
        .prompt
        .take()
        .map(|p| p.rows)
        .unwrap_or_default();
    for (definition_id, track_id) in undecided {
        if rows
            .iter()
            .any(|row| row.definition_id == definition_id && row.track_id == track_id)
        {
            continue;
        }
        let hand_edited = super::vocal_melody::lane_flags(r, definition_id, track_id).hand_edited;
        rows.push(VocalMelodyPromptRow {
            definition_id,
            track_id,
            section_name: r
                .compose
                .find_definition(definition_id)
                .map(|d| d.name.clone())
                .unwrap_or_default(),
            track_name: r
                .registry
                .tracks
                .iter()
                .find(|t| t.id == track_id)
                .map(|t| t.name.clone())
                .unwrap_or_default(),
            hand_edited,
            choice: if hand_edited {
                VocalMelodyChoice::Keep
            } else {
                VocalMelodyChoice::Regenerate
            },
        });
    }
    r.compose.vocal_melody.prompt = Some(VocalMelodyPrompt {
        rows,
        remember: false,
        history_generation: generation,
        undoable: r.can_record_undo(),
    });
}
