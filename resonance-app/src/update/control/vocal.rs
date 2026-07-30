//! `vocal.*` control methods (ba doc #265, todo #1156): lyrics,
//! per-word pronunciation overrides, and the SVS render job.
//!
//! Lyrics live per `(section definition, track)` vocal lane; the wire
//! addresses a track, so these methods resolve the track's first vocal
//! lane in placement order (`song.vocal` reads the same order). All
//! mutations synthesize a [`ComposeMessage`] carrier routed through
//! [`super::run_via_update`], so each is one undoable transaction.
//!
//! `vocal.render` is a **job** (doc #265): it returns `{job_id}`
//! immediately and the render lands asynchronously; the job resolves
//! from the existing `VocalAudioReady` / `VocalAudioFailed` completion
//! messages via a [`JobToken::VocalRender`]. The default voicebank is
//! **Lilia** (doc #265) — the maintained multi-language bank — overriding
//! the code-level `VocalParams` default of TIGER.

use crate::compose::ComposeMessage;
use crate::control_jobs::JobToken;
use crate::message::Message;
use crate::update::compose::vocal_render::{first_vocal_definition, track_has_vocal_lane};
use crate::Resonance;
use iced::Task;
use resonance_control::job::JobStarted;
use resonance_control::methods::vocal as proto;
use resonance_control::{MutationAck, Request, Response, RpcError};
use resonance_music_theory::VocalVoicebank;

use super::section::fail;

pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let handled = match request.method.as_str() {
        proto::SET_LYRICS => set_lyrics(app, request),
        proto::SET_LINE => set_line(app, request),
        proto::SET_PRONUNCIATION => set_pronunciation(app, request),
        proto::CLEAR_PRONUNCIATION => clear_pronunciation(app, request),
        proto::RENDER => render(app, request),
        proto::GENERATE => generate(app, request),
        _ => return None,
    };
    Some(handled)
}

// ---------------------------------------------------------------------------
// Lyrics
// ---------------------------------------------------------------------------

fn set_lyrics(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::SetLyricsParams = match request.params() {
        Ok(p) => p,
        Err(e) => return fail(request, e),
    };
    let track_id: u64 = params.track_id.into();
    let definition_id = match resolve_vocal_lane(app, track_id, params.section_id) {
        Ok(id) => id,
        Err(e) => return fail(request, e),
    };
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::ControlSetVocalLyrics {
            definition_id,
            track_id,
            text: params.text,
        }),
    );
    ack(app, request, task)
}

fn set_line(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::SetLineParams = match request.params() {
        Ok(p) => p,
        Err(e) => return fail(request, e),
    };
    let track_id: u64 = params.track_id.into();
    let definition_id = match resolve_vocal_lane(app, track_id, params.section_id) {
        Ok(id) => id,
        Err(e) => return fail(request, e),
    };
    // Range-check against the resolved lane's draft up front so an
    // out-of-range index is a precise error, not a silent no-op.
    let line_count = app
        .compose
        .find_definition(definition_id)
        .and_then(|d| d.lane_generators.get(&track_id))
        .and_then(|c| match &c.kind {
            crate::compose::LaneGeneratorKind::Vocal(p) => Some(p.draft.len()),
            _ => None,
        })
        .unwrap_or(0);
    if params.line_index >= line_count {
        return fail(
            request,
            RpcError::invalid_params(format!(
                "line_index {} out of range (the lane has {line_count} line(s))",
                params.line_index
            )),
        );
    }
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::ControlSetVocalLine {
            definition_id,
            track_id,
            line_index: params.line_index,
            text: params.text,
        }),
    );
    ack(app, request, task)
}

// ---------------------------------------------------------------------------
// Generate
// ---------------------------------------------------------------------------

/// `vocal.generate` (ba doc #269 FR-2): generate the lane's melody — and
/// by default a fresh lyric draft — into its derived clip.
///
/// `generate.part` refuses vocal tracks, and the SVS render reads notes
/// from `compose.derived_clips`, so before this method a vocal lane
/// reachable over the wire still had no notes: `render_state` stayed
/// `not_rendered` with `clip_count = 0` and there was nothing to sing.
fn generate(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::GenerateParams = match request.params() {
        Ok(p) => p,
        Err(e) => return fail(request, e),
    };
    let track_id: u64 = params.track_id.into();
    let definition_id = match resolve_vocal_lane(app, track_id, params.section_id) {
        Ok(id) => id,
        Err(e) => return fail(request, e),
    };
    // The vocal melody is derived from the section's chord grid; with no
    // chords the generator silently produces nothing, so refuse up front
    // (mirroring generate.part's gate) rather than acking an empty lane.
    let has_chords = app
        .compose
        .find_definition(definition_id)
        .is_some_and(|d| !d.chords.is_empty());
    if !has_chords {
        return fail(
            request,
            RpcError::invalid_params(
                "the lane's section has no chords to generate from; add chords (harmony.*) first",
            ),
        );
    }
    // Melody-only on an empty draft would derive nothing: the melody is
    // laid out one note per syllable.
    if !params.lyrics && lane_draft_is_empty(app, definition_id, track_id) {
        return fail(
            request,
            RpcError::invalid_params(
                "the lane has no lyrics to sing; write them with vocal.set_lyrics or call \
                 vocal.generate with lyrics: true",
            ),
        );
    }

    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::ControlGenerateVocal {
            definition_id,
            track_id,
            seed: params.seed,
            lyrics: params.lyrics,
        }),
    );
    if let Some(error) = app.compose.last_error.take() {
        return fail(request, RpcError::invalid_params(error));
    }
    let Some(clip_id) = first_derived_clip(app, definition_id, track_id) else {
        return fail(
            request,
            RpcError::internal("the vocal lane generated no clip"),
        );
    };
    let result = proto::GenerateResult {
        clip_id: resonance_control::ids::ClipId(clip_id),
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}

/// True when the lane carries no lyric lines to sing.
fn lane_draft_is_empty(app: &Resonance, definition_id: u64, track_id: u64) -> bool {
    app.compose
        .find_definition(definition_id)
        .and_then(|d| d.lane_generators.get(&track_id))
        .and_then(|c| match &c.kind {
            crate::compose::LaneGeneratorKind::Vocal(p) => Some(p.draft.is_empty()),
            _ => None,
        })
        .unwrap_or(true)
}

/// The lane's derived clip at its first placement, in placement order —
/// a lane derives one clip per placement, all with the same material.
fn first_derived_clip(app: &Resonance, definition_id: u64, track_id: u64) -> Option<u64> {
    let mut placed: Vec<(u32, u64)> = app
        .compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .filter_map(|p| {
            app.compose
                .derived_clips
                .get(&(definition_id, p.id, track_id))
                .map(|clip| (p.start_bar, *clip))
        })
        .collect();
    placed.sort_by_key(|(bar, _)| *bar);
    placed
        .first()
        .map(|(_, clip)| *clip)
        // An unplaced lane still derives into the definition's own entry.
        .or_else(|| {
            app.compose
                .derived_clips
                .iter()
                .find(|((def, _, track), _)| *def == definition_id && *track == track_id)
                .map(|(_, clip)| *clip)
        })
}

// ---------------------------------------------------------------------------
// Pronunciation
// ---------------------------------------------------------------------------

fn set_pronunciation(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::SetPronunciationParams = match request.params() {
        Ok(p) => p,
        Err(e) => return fail(request, e),
    };
    if params.word.trim().is_empty() {
        return fail(request, RpcError::invalid_params("word cannot be empty"));
    }
    if params.phonemes.is_empty() {
        return fail(
            request,
            RpcError::invalid_params("phonemes cannot be empty; use vocal.clear_pronunciation to remove"),
        );
    }
    // Canonicalise to the SVS pipeline's `&'static str` ARPAbet symbols,
    // rejecting any the pipeline can't sing (rather than silently
    // dropping them, as the bulk canonicaliser does).
    let canonical = crate::compose::vocal_svs::canonicalize_phonemes(&params.phonemes);
    if canonical.len() != params.phonemes.len() {
        let bad: Vec<&String> = params
            .phonemes
            .iter()
            .filter(|p| crate::compose::vocal_svs::canonicalize_phonemes(std::slice::from_ref(*p)).is_empty())
            .collect();
        return fail(
            request,
            RpcError::invalid_params(format!(
                "unknown ARPAbet phoneme(s): {}",
                bad.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            )),
        );
    }
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::ControlSetPronunciation {
            word: params.word,
            phonemes: canonical,
        }),
    );
    ack(app, request, task)
}

fn clear_pronunciation(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::ClearPronunciationParams = match request.params() {
        Ok(p) => p,
        Err(e) => return fail(request, e),
    };
    let key = crate::compose::vocal_svs::clean_word(&params.word);
    let present = app
        .compose
        .pronunciation
        .project_dictionary
        .iter()
        .any(|e| e.word == key);
    if !present {
        return fail(
            request,
            RpcError::not_found(format!("no pronunciation override for {:?}", params.word)),
        );
    }
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::ControlClearPronunciation { word: params.word }),
    );
    ack(app, request, task)
}

// ---------------------------------------------------------------------------
// Render (job)
// ---------------------------------------------------------------------------

fn render(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::RenderParams = match request.params() {
        Ok(p) => p,
        Err(e) => return fail(request, e),
    };
    // Doc #265: default voicebank is Lilia when the client doesn't name
    // one, overriding the VocalParams code default (TIGER).
    let voicebank = match &params.voicebank {
        Some(name) => match parse_voicebank(name) {
            Some(vb) => vb,
            None => {
                let known: Vec<&str> =
                    VocalVoicebank::ALL.iter().map(|vb| vb.as_str()).collect();
                return fail(
                    request,
                    RpcError::invalid_params(format!(
                        "unknown voicebank {name:?} (one of: {})",
                        known.join(", ")
                    )),
                );
            }
        },
        None => VocalVoicebank::Lilia,
    };

    // Which lane(s): a specific track, or every vocal track when omitted.
    let track_id = match params.track_id {
        Some(t) => {
            let raw: u64 = t.into();
            if !track_has_vocal_lane(app, raw) {
                return fail(
                    request,
                    RpcError::not_found(format!("track {raw} has no vocal lane to render")),
                );
            }
            raw
        }
        None => match first_vocal_lane_track(app) {
            Some(raw) => raw,
            None => {
                return fail(
                    request,
                    RpcError::not_found("the project has no vocal lane to render"),
                )
            }
        },
    };

    let definition_id = match first_vocal_definition(app, track_id) {
        Some(id) => id,
        None => {
            return fail(
                request,
                RpcError::not_found(format!("track {track_id} has no vocal lane to render")),
            )
        }
    };

    // Pre-flight the conditions `roll_vocal_melody` silently no-ops on
    // (empty draft, no chords) so they surface as a precise error rather
    // than a job that never resolves.
    if let Some(def) = app.compose.find_definition(definition_id) {
        if def.chords.is_empty() {
            return fail(
                request,
                RpcError::invalid_params(
                    "section has no chords; add chords (harmony.*) before rendering vocals",
                ),
            );
        }
        let empty_draft = def
            .lane_generators
            .get(&track_id)
            .and_then(|c| match &c.kind {
                crate::compose::LaneGeneratorKind::Vocal(p) => Some(p.draft.is_empty()),
                _ => None,
            })
            .unwrap_or(true);
        if empty_draft {
            return fail(
                request,
                RpcError::invalid_params(
                    "vocal lane has no lyrics; set lyrics (vocal.set_lyrics) before rendering",
                ),
            );
        }
    }

    // Register the job first so the completion hook (which fires
    // synchronously if the render fails fast, or later on the audio-ready
    // message) can resolve it by token.
    let JobStarted { job_id } = app.start_control_job(
        "vocal.render",
        &format!("SVS render for vocal track {track_id}"),
        JobToken::VocalRender {
            definition_id,
            track_id,
        },
        None,
    );

    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::ControlRenderVocal {
            definition_id,
            track_id,
            voicebank,
        }),
    );

    // A synchronous validation failure (empty draft, no chords, blocked
    // phoneme) leaves `compose.last_error` set and never dispatches a
    // render task — fail the job now so `job.wait` resolves immediately.
    if let Some(error) = app.compose.last_error.take() {
        app.control_jobs().fail(job_id.into(), error.clone());
        return fail(request, RpcError::invalid_params(error));
    }

    (super::success(request, &JobStarted { job_id }), task)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the vocal lane a track addresses (its first in placement
/// order), or a precise error when the track has none.
fn resolve_vocal_lane(
    app: &Resonance,
    track_id: u64,
    section_id: Option<resonance_control::ids::SectionDefinitionId>,
) -> Result<u64, RpcError> {
    // An explicit section addresses one lane (ba doc #269 FR-3): the
    // track may sing in several sections, and without this the writes
    // all landed on the first lane in placement order.
    if let Some(section_id) = section_id {
        let definition_id: u64 = section_id.into();
        let Some(def) = app.compose.find_definition(definition_id) else {
            return Err(RpcError::not_found(format!(
                "no section definition with id {definition_id}"
            )));
        };
        return match def.lane_generators.get(&track_id).map(|c| &c.kind) {
            Some(crate::compose::LaneGeneratorKind::Vocal(_)) => Ok(definition_id),
            _ => Err(RpcError::invalid_params(format!(
                "section {definition_id} ({:?}) has no vocal lane on track {track_id}",
                def.name
            ))),
        };
    }
    first_vocal_definition(app, track_id).ok_or_else(|| {
        if app.registry.tracks.iter().any(|t| t.id == track_id) {
            RpcError::invalid_params(format!(
                "track {track_id} has no vocal lane (configure a vocal generator on it first)"
            ))
        } else {
            RpcError::not_found(format!("no track with id {track_id}"))
        }
    })
}

/// The track id of any vocal lane in the project (first in placement
/// order), for a `vocal.render` with no explicit track.
fn first_vocal_lane_track(app: &Resonance) -> Option<u64> {
    let mut placed: Vec<(u32, u64)> = Vec::new();
    for p in &app.compose.placements {
        if let Some(def) = app.compose.find_definition(p.definition_id) {
            for (track_id, cfg) in &def.lane_generators {
                if matches!(cfg.kind, crate::compose::LaneGeneratorKind::Vocal(_)) {
                    placed.push((p.start_bar, *track_id));
                }
            }
        }
    }
    placed.sort_by_key(|(bar, _)| *bar);
    if let Some((_, track)) = placed.first() {
        return Some(*track);
    }
    for def in &app.compose.definitions {
        for (track_id, cfg) in &def.lane_generators {
            if matches!(cfg.kind, crate::compose::LaneGeneratorKind::Vocal(_)) {
                return Some(*track_id);
            }
        }
    }
    None
}

/// Parse a wire voicebank name against [`VocalVoicebank`] (case-
/// insensitive; matches the strum `as_str` names `TIGER` / `Lilia` /
/// `Meiji`).
fn parse_voicebank(name: &str) -> Option<VocalVoicebank> {
    let wanted = name.trim().to_ascii_lowercase();
    VocalVoicebank::ALL
        .iter()
        .copied()
        .find(|vb| vb.as_str().to_ascii_lowercase() == wanted)
}

/// A `MutationAck` reply plus the routed task, or the compose error left
/// behind by the dispatch as `invalid_params`.
fn ack(
    app: &mut Resonance,
    request: &Request,
    task: Task<Message>,
) -> (Response, Task<Message>) {
    if let Some(error) = app.compose.last_error.take() {
        return fail(request, RpcError::invalid_params(error));
    }
    (super::success(request, &MutationAck { revision: app.revision() }), task)
}
