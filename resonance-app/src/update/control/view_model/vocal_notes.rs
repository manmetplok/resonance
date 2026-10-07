//! Per-note lyric facts for `song.vocal`: which notes are holds and why
//! (vocal-mcp-control-and-quality.md §6), where each note's phonemes
//! came from and whether its syllable split is trusted (§8).

use resonance_control::methods::song::{VocalG2pSource, VocalSlurKind};
use resonance_music_theory::g2p::{self, G2pSource, HoldOrigin, ResolvedSyllable};

use crate::Resonance;

/// One note's lyric facts, by note index.
pub(in crate::update::control) struct NoteLyricFacts {
    pub slur: Option<VocalSlurKind>,
    pub g2p: Option<VocalG2pSource>,
    pub syllable_warning: bool,
}

/// A lane's per-note lyric facts plus how many of its notes are the
/// generator's own holds — notes that are not lyric units, so
/// `counts_mismatch` leaves them out.
pub(in crate::update::control) struct LaneNoteLyrics {
    pub notes: Vec<NoteLyricFacts>,
    pub auto_holds: usize,
}

/// Resolve the lane's clip against its draft the way the render does
/// (`resolved` is the draft through the project and bank dictionaries;
/// per-syllable edits are applied here) and report each note.
pub(in crate::update::control) fn lane_note_lyrics(
    app: &Resonance,
    definition_id: u64,
    track_id: resonance_audio::types::TrackId,
    params: &resonance_music_theory::VocalParams,
    resolved: &[ResolvedSyllable],
    bank: Option<&g2p::PhonemeDictionary>,
) -> LaneNoteLyrics {
    let Some(clip) = super::lane::lane_clip(app, definition_id, track_id) else {
        return LaneNoteLyrics { notes: Vec::new(), auto_holds: 0 };
    };
    let note_count = clip.notes.len();
    let mut annotations = app
        .compose
        .vocal_audio
        .clip_lyrics
        .get(&clip.id)
        .cloned()
        .unwrap_or_default();
    annotations.resize(note_count, String::new());
    let origins = g2p::hold_origins(resolved, &annotations, note_count);
    let assigned = app.compose.pronunciation.resolve_lane(
        definition_id,
        track_id,
        &params.draft,
        &annotations,
        note_count,
        &[],
        bank,
    );
    let notes: Vec<NoteLyricFacts> = assigned
        .iter()
        .zip(&origins)
        .map(|(note, origin)| NoteLyricFacts {
            slur: origin.map(|o| match o {
                HoldOrigin::Draft => VocalSlurKind::User,
                HoldOrigin::Annotation => VocalSlurKind::Auto,
            }),
            // A sung note past the end of the lyrics sings no syllable.
            g2p: (note.is_slur || note.syllable_index < resolved.len())
                .then(|| g2p_source(note.source)),
            syllable_warning: note.syllable_warning,
        })
        .collect();
    let auto_holds = origins
        .iter()
        .filter(|o| **o == Some(HoldOrigin::Annotation))
        .count();
    LaneNoteLyrics { notes, auto_holds }
}

fn g2p_source(source: G2pSource) -> VocalG2pSource {
    match source {
        G2pSource::Edited => VocalG2pSource::Edited,
        G2pSource::ProjectDict => VocalG2pSource::ProjectDict,
        G2pSource::BankDict => VocalG2pSource::BankDict,
        G2pSource::Cmu => VocalG2pSource::Cmu,
        G2pSource::Rules => VocalG2pSource::Rules,
    }
}
