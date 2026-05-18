//! Bulk-lyrics text editor backing logic for the vocal lane. The
//! per-lane `params.draft` is the canonical source of lyric text; this
//! module keeps the `iced::widget::text_editor::Content` mirror in sync
//! and reparses the buffer back into draft entries on every edit.

use resonance_audio::types::TrackId;
use resonance_music_theory::VocalParams;

use crate::compose::LaneGeneratorKind;

/// Rebuild the bulk-lyrics text editor `Content` from the lane's current
/// `params.draft`. Called after any path that mutates the draft outside
/// the bulk editor (per-line edits, re-rolls, generate actions) so the
/// two views stay in sync. Only touches lanes that already have an editor
/// allocated — first-use materialisation happens lazily in the view.
pub(super) fn sync_bulk_lyrics_from_draft(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) {
    let Some(def) = r.compose.find_definition(definition_id) else {
        return;
    };
    let Some(cfg) = def.lane_generators.get(&track_id) else {
        return;
    };
    let LaneGeneratorKind::Vocal(params) = &cfg.kind else {
        return;
    };
    let key = (definition_id, track_id);
    if !r.compose.vocal_bulk_lyrics.contains_key(&key) {
        return;
    }
    let body = draft_to_text(&params.draft);
    r.compose
        .vocal_bulk_lyrics
        .insert(key, iced::widget::text_editor::Content::with_text(&body));
}

/// Render the draft as a `\n`-joined plain-text body for the bulk editor.
/// Strips the typographic syllable-separator (`·`) so users see clean
/// prose; per-line entries can still hold the separator since we only
/// re-derive on bulk-side edits.
fn draft_to_text(draft: &[resonance_music_theory::LyricLine]) -> String {
    draft
        .iter()
        .map(|l| l.text.replace('\u{00B7}', "").replace("  ", " "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Apply an action from the bulk-lyrics text editor. Inserts the lane's
/// `Content` lazily on first use (seeded from the current draft), perfoms
/// the action, and — when the action is an edit — re-parses the buffer
/// into individual `LyricLine`s. Each non-empty line becomes one entry,
/// auto-locked so the next re-roll preserves it.
pub(super) fn handle_bulk_lyrics_action(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    action: iced::widget::text_editor::Action,
) {
    let key = (definition_id, track_id);

    if !r.compose.vocal_bulk_lyrics.contains_key(&key) {
        let initial = r
            .compose
            .find_definition(definition_id)
            .and_then(|d| d.lane_generators.get(&track_id))
            .and_then(|cfg| match &cfg.kind {
                LaneGeneratorKind::Vocal(p) => Some(draft_to_text(&p.draft)),
                _ => None,
            })
            .unwrap_or_default();
        r.compose.vocal_bulk_lyrics.insert(
            key,
            iced::widget::text_editor::Content::with_text(&initial),
        );
    }

    let is_edit = action.is_edit();
    if let Some(content) = r.compose.vocal_bulk_lyrics.get_mut(&key) {
        content.perform(action);
    }

    if !is_edit {
        return;
    }

    let body = r
        .compose
        .vocal_bulk_lyrics
        .get(&key)
        .map(|c| c.text())
        .unwrap_or_default();
    super::lane_inspector::update_vocal(r, definition_id, track_id, |p| {
        rebuild_draft_from_bulk(p, &body);
    });
}

/// Parse the bulk editor's text and rewrite `params.draft`. One non-empty
/// line per `LyricLine`; rhyme tags follow the lane's current rhyme
/// scheme so the per-line preview's colour chips stay coherent. Empty
/// trailing lines are stripped, blank lines in the middle are skipped.
/// `params.lines` is bumped to match so re-rolls operate on the same
/// shape.
fn rebuild_draft_from_bulk(p: &mut VocalParams, body: &str) {
    use resonance_music_theory::LyricLine;

    let pattern: &[u8] = match p.rhyme {
        resonance_music_theory::VocalRhymeScheme::Aabb => &[0, 0, 1, 1],
        resonance_music_theory::VocalRhymeScheme::Abab => &[0, 1, 0, 1],
        resonance_music_theory::VocalRhymeScheme::Abcb => &[0, 1, 2, 1],
        resonance_music_theory::VocalRhymeScheme::Abba => &[0, 1, 1, 0],
        resonance_music_theory::VocalRhymeScheme::Free => &[],
    };
    let letter_for = |slot: u8| -> char { (b'A' + (slot % 26)) as char };

    let lines: Vec<&str> = body
        .lines()
        .map(|l| l.trim_end_matches('\r').trim())
        .filter(|l| !l.is_empty())
        .collect();

    let mut out = Vec::with_capacity(lines.len());
    for (i, text) in lines.iter().enumerate() {
        let rhyme = if pattern.is_empty() {
            'F'
        } else {
            letter_for(pattern[i % pattern.len()])
        };
        out.push(LyricLine {
            n: (i + 1) as u8,
            rhyme,
            syllables: resonance_music_theory::count_syllables(text).min(255) as u8,
            text: text.to_string(),
            locked: true,
        });
    }

    if !out.is_empty() {
        p.lines = (out.len() as u8).clamp(1, 16);
    }
    p.draft = out;
}
