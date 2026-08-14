//! `vocal.*` control methods (ba doc #265, todo #1156): lyrics,
//! per-word pronunciation overrides, and the SVS render job.
//!
//! Lyrics live per `(section definition, track)` vocal lane; the wire
//! addresses a track, so these methods resolve the track's first vocal
//! lane in placement order (`song.vocal` reads the same order). All
//! mutations synthesize a [`ComposeMessage`] carrier routed through
//! [`super::run_via_update`], so each is one undoable transaction.
//!
//! `vocal.render` is the exception to the first-lane rule: it fans out
//! over **every** lane the request covers (all of a track's lanes when
//! no `section_id` is given; every vocal track when no `track_id` is
//! given either), because "render the track" that renders one lane
//! leaves the rest audibly stale while reporting success.
//!
//! `vocal.render` is a **job** (doc #265): it returns `{job_id}`
//! immediately and the render lands asynchronously; the job resolves
//! from the existing `VocalAudioReady` / `VocalAudioFailed` completion
//! messages via a [`JobToken::VocalRender`], once **all** of its lanes
//! have landed. The default voicebank is
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
use resonance_control::{Request, Response, RpcError};
use resonance_music_theory::VocalVoicebank;

// The `vocal.*` writes route ComposeMessages, so they answer with the
// compose-aware acknowledgement rather than a bare one: a reducer that
// refused parks its reason on `compose.last_error` instead of returning
// it (todo #1258).
use super::reply::{ack_or_compose_error, no_section_definition, no_track, reject};

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
        Err(e) => return reject(request, e),
    };
    let track_id: u64 = params.track_id.into();
    let definition_id = match resolve_vocal_lane(app, track_id, params.section_id) {
        Ok(id) => id,
        Err(e) => return reject(request, e),
    };
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::ControlSetVocalLyrics {
            definition_id,
            track_id,
            text: normalize_lyric_text(&params.text, params.syllabify),
        }),
    );
    ack_or_compose_error(app, request, task)
}

/// Normalise incoming lyric text into the `·`-marked form the whole
/// vocal path counts syllables from.
///
/// The lyric tokenizer treats a word with no break as **one** syllable,
/// and a syllable is one note — so `"resolution"` stored verbatim put all
/// nine of its phonemes (`r eh z ax l uw sh ax n`) on a single note. At
/// any singable tempo that is a few tens of milliseconds per phoneme:
/// the consonants never articulate and the word is heard as a smear.
/// Every lyric that arrives over the wire therefore goes through
/// [`g2p::auto_syllabify_text`] first, which inserts the missing breaks
/// (`re·so·lu·tion`) while leaving hand-broken words, `[..]` phoneme
/// blocks and line structure alone. It also normalises hand-typed `-`
/// breaks to `·`, so `re-so-lu-tion` works without the caller having to
/// type a middle dot.
///
/// `syllabify: false` still gets the `-` → `·` normalisation (that is a
/// notation detail, not a transformation) but no automatic splitting.
fn normalize_lyric_text(text: &str, syllabify: bool) -> String {
    use resonance_music_theory::g2p;
    if syllabify {
        g2p::auto_syllabify_text(text)
    } else {
        g2p::normalize_syllable_marks(text)
    }
}

fn set_line(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::SetLineParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let track_id: u64 = params.track_id.into();
    let definition_id = match resolve_vocal_lane(app, track_id, params.section_id) {
        Ok(id) => id,
        Err(e) => return reject(request, e),
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
        return reject(
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
            text: normalize_lyric_text(&params.text, params.syllabify),
        }),
    );
    ack_or_compose_error(app, request, task)
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
        Err(e) => return reject(request, e),
    };
    let track_id: u64 = params.track_id.into();
    let definition_id = match resolve_vocal_lane(app, track_id, params.section_id) {
        Ok(id) => id,
        Err(e) => return reject(request, e),
    };
    // The vocal melody is derived from the section's chord grid; with no
    // chords the generator silently produces nothing, so refuse up front
    // (mirroring generate.part's gate) rather than acking an empty lane.
    let has_chords = app
        .compose
        .find_definition(definition_id)
        .is_some_and(|d| !d.chords.is_empty());
    if !has_chords {
        return reject(
            request,
            RpcError::invalid_params(
                "the lane's section has no chords to generate from; add chords (harmony.*) first",
            ),
        );
    }
    // Melody-only on an empty draft would derive nothing: the melody is
    // laid out one note per syllable.
    if !params.lyrics && lane_draft_is_empty(app, definition_id, track_id) {
        return reject(
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
        return reject(request, RpcError::invalid_params(error));
    }
    let Some(clip_id) = first_derived_clip(app, definition_id, track_id) else {
        return reject(
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
        Err(e) => return reject(request, e),
    };
    if params.word.trim().is_empty() {
        return reject(request, RpcError::invalid_params("word cannot be empty"));
    }
    if params.phonemes.is_empty() {
        return reject(
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
        return reject(
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
    ack_or_compose_error(app, request, task)
}

fn clear_pronunciation(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::ClearPronunciationParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let key = crate::compose::vocal_svs::clean_word(&params.word);
    let present = app
        .compose
        .pronunciation
        .project_dictionary
        .iter()
        .any(|e| e.word == key);
    if !present {
        return reject(
            request,
            RpcError::not_found(format!("no pronunciation override for {:?}", params.word)),
        );
    }
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::ControlClearPronunciation { word: params.word }),
    );
    ack_or_compose_error(app, request, task)
}

// ---------------------------------------------------------------------------
// Render (job)
// ---------------------------------------------------------------------------

fn render(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::RenderParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    // An explicitly named voicebank is parsed up front so a typo fails
    // before any job starts. `None` is resolved per lane further down —
    // it must not blanket-default, or every plain render would reset the
    // lane (ba doc #271).
    let requested = match &params.voicebank {
        Some(name) => match parse_voicebank(name) {
            Some(vb) => Some(vb),
            None => {
                let known: Vec<&str> =
                    VocalVoicebank::ALL.iter().map(|vb| vb.as_str()).collect();
                return reject(
                    request,
                    RpcError::invalid_params(format!(
                        "unknown voicebank {name:?} (one of: {})",
                        known.join(", ")
                    )),
                );
            }
        },
        None => None,
    };

    // Which lane(s) this render covers. A named `section_id` addresses
    // exactly one; otherwise it is *every* lane of the named track, and
    // with no track named, every vocal lane in the project. Resolving
    // "the track's first vocal lane" here instead (the rule the lyric
    // methods use, where one write must land on one lane) is what made
    // `vocal.render(track_id)` re-render lane 1 and silently leave lanes
    // 2..n on their previous audio — a render that reported `done` while
    // most of the track was stale, and, once the edited notes no longer
    // matched any rendered WAV, sections that saved out silent.
    let lanes = match resolve_render_lanes(app, params.track_id, params.section_id) {
        Ok(lanes) => lanes,
        Err(e) => return reject(request, e),
    };

    // Pre-flight the conditions `rerender_vocal_audio` silently no-ops
    // on so they surface as a precise error rather than a job that never
    // resolves. The section's chords are deliberately NOT among them:
    // render synthesises the notes already in the lane's clip and never
    // derives from the chord grid, so a chordless section with authored
    // notes is a legitimate render (ba doc #271 V1).
    //
    // Across a fan-out, a lane that can't render is skipped rather than
    // failing the batch — one empty lane must not stop the rest of the
    // track from re-rendering — but a batch where *nothing* can render
    // is still a precise error.
    let mut renderable: Vec<(u64, u64)> = Vec::new();
    let mut blocked: Vec<(u64, String)> = Vec::new();
    for &(definition_id, track_id) in &lanes {
        match lane_render_block(app, definition_id, track_id) {
            Some(reason) => blocked.push((definition_id, reason)),
            None => renderable.push((definition_id, track_id)),
        }
    }
    if renderable.is_empty() {
        return reject(
            request,
            RpcError::invalid_params(describe_blocked(app, &blocked)),
        );
    }

    // Any error left over from an earlier operation would otherwise be
    // misread as this render's failure.
    let _ = app.compose.last_error.take();

    let mut tasks: Vec<Task<Message>> = Vec::new();
    let mut started: Vec<(u64, u64)> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for (definition_id, track_id) in renderable {
        // Resolve an omitted voicebank against the lane rather than
        // against a constant. Blanket-defaulting to Lilia meant every
        // render that left the argument out silently reset the lane, so a
        // song with a TIGER character and a Lilia character lost the
        // split on the next plain render (ba doc #271). A lane still
        // holding the untouched `VocalParams` code default has never
        // chosen one, so it still picks up doc #265's Lilia; anything
        // explicitly set wins. Per lane, because a fan-out crosses lanes
        // that may have chosen differently.
        let voicebank = requested.unwrap_or_else(|| {
            match lane_voicebank(app, definition_id, track_id) {
                Some(vb) if vb != default_lane_voicebank() => vb,
                _ => VocalVoicebank::Lilia,
            }
        });

        let task = super::run_via_update(
            app,
            Message::Compose(ComposeMessage::ControlRenderVocal {
                definition_id,
                track_id,
                voicebank,
            }),
        );

        // A synchronous validation failure (blocked phoneme, vanished
        // clip) leaves `compose.last_error` set and never dispatches a
        // render task. Drop that lane from the batch — keeping it would
        // leave the job waiting forever for audio that will never come.
        if let Some(error) = app.compose.last_error.take() {
            errors.push(lane_error(app, definition_id, &error));
            continue;
        }
        started.push((definition_id, track_id));
        tasks.push(task);
    }

    if started.is_empty() {
        return reject(request, RpcError::invalid_params(errors.join("; ")));
    }

    // Register the job covering exactly the lanes that dispatched; it
    // resolves when the last of them reports its audio.
    let JobStarted { job_id } = app.start_control_job(
        "vocal.render",
        &describe_batch(&started),
        JobToken::VocalRender {
            lanes: started.clone(),
        },
        None,
    );

    // Chained, not batched: each lane's task drives a full SVS pipeline
    // on a blocking thread, and `Task::batch` would start every lane's at
    // once — several ONNX sessions and their working sets alive together
    // on a machine that is also playing audio. Chaining renders them one
    // after another, which is what a client looping over lanes by hand
    // already did. The synchronous half (tear-down, epoch bump) has
    // already run for every lane either way.
    let task = tasks
        .into_iter()
        .reduce(|acc, next| acc.chain(next))
        .unwrap_or_else(Task::none);

    (super::success(request, &JobStarted { job_id }), task)
}

/// The `(definition_id, track_id)` lanes a `vocal.render` covers.
///
/// - `section_id` names one lane (the only single-lane form).
/// - `track_id` alone is every vocal lane on that track.
/// - Neither is every vocal lane in the project.
fn resolve_render_lanes(
    app: &Resonance,
    track_id: Option<resonance_control::ids::TrackId>,
    section_id: Option<resonance_control::ids::SectionDefinitionId>,
) -> Result<Vec<(u64, u64)>, RpcError> {
    use crate::update::compose::vocal_render::{all_vocal_lanes, vocal_definitions_for_track};

    match (track_id, section_id) {
        (Some(track), section) => {
            let track: u64 = track.into();
            if !track_has_vocal_lane(app, track) {
                return Err(RpcError::not_found(format!(
                    "track {track} has no vocal lane to render"
                )));
            }
            if section.is_some() {
                // Routed through the same resolver the rest of the
                // namespace uses, so the error shapes match.
                let definition_id = resolve_vocal_lane(app, track, section)?;
                return Ok(vec![(definition_id, track)]);
            }
            Ok(vocal_definitions_for_track(app, track)
                .into_iter()
                .map(|def| (def, track))
                .collect())
        }
        (None, Some(section_id)) => {
            // A section with no track: every vocal lane that section has.
            let definition_id: u64 = section_id.into();
            let Some(def) = app.compose.find_definition(definition_id) else {
                return Err(no_section_definition(definition_id));
            };
            let mut lanes: Vec<(u64, u64)> = def
                .lane_generators
                .iter()
                .filter(|(_, cfg)| {
                    matches!(cfg.kind, crate::compose::LaneGeneratorKind::Vocal(_))
                })
                .map(|(track, _)| (definition_id, *track))
                .collect();
            lanes.sort_unstable();
            if lanes.is_empty() {
                return Err(RpcError::invalid_params(format!(
                    "section {definition_id} ({:?}) has no vocal lane to render",
                    def.name
                )));
            }
            Ok(lanes)
        }
        (None, None) => {
            let lanes = all_vocal_lanes(app);
            if lanes.is_empty() {
                return Err(RpcError::not_found(
                    "the project has no vocal lane to render",
                ));
            }
            Ok(lanes)
        }
    }
}

/// Why this lane cannot render right now, or `None` when it can.
fn lane_render_block(app: &Resonance, definition_id: u64, track_id: u64) -> Option<String> {
    if !lane_has_notes(app, definition_id, track_id) {
        return Some(
            "vocal lane has no notes to sing; generate a melody (vocal.generate) or write \
             notes into its clip (notes.*) before rendering"
                .to_owned(),
        );
    }
    if lane_draft_is_empty(app, definition_id, track_id) {
        return Some(
            "vocal lane has no lyrics; set lyrics (vocal.set_lyrics) before rendering".to_owned(),
        );
    }
    None
}

/// The error for a batch in which no lane could render. One lane keeps
/// its own message verbatim (the single-lane case is still the common
/// one and its wording is what clients match on); several are listed
/// with the section each belongs to.
fn describe_blocked(app: &Resonance, blocked: &[(u64, String)]) -> String {
    match blocked {
        [] => "no vocal lane to render".to_owned(),
        [(_, reason)] => reason.clone(),
        many => format!(
            "no vocal lane can render: {}",
            many.iter()
                .map(|(def, reason)| lane_error(app, *def, reason))
                .collect::<Vec<_>>()
                .join("; ")
        ),
    }
}

/// A lane-scoped error message, prefixed with the section name so a
/// fan-out failure says *which* lane it is about.
fn lane_error(app: &Resonance, definition_id: u64, reason: &str) -> String {
    match app.compose.find_definition(definition_id) {
        Some(def) => format!("{:?}: {reason}", def.name),
        None => reason.to_owned(),
    }
}

/// Human-readable job description naming what the batch covers.
fn describe_batch(lanes: &[(u64, u64)]) -> String {
    let mut tracks: Vec<u64> = lanes.iter().map(|(_, t)| *t).collect();
    tracks.sort_unstable();
    tracks.dedup();
    match (lanes.len(), tracks.as_slice()) {
        (1, [track]) => format!("SVS render for vocal track {track}"),
        (n, [track]) => format!("SVS render for {n} vocal lanes on track {track}"),
        (n, tracks) => format!("SVS render for {n} vocal lanes across {} tracks", tracks.len()),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The voicebank currently stored on a vocal lane.
fn lane_voicebank(
    app: &Resonance,
    definition_id: u64,
    track_id: u64,
) -> Option<VocalVoicebank> {
    app.compose
        .find_definition(definition_id)
        .and_then(|d| d.lane_generators.get(&track_id))
        .and_then(|c| match &c.kind {
            crate::compose::LaneGeneratorKind::Vocal(p) => Some(p.voicebank),
            _ => None,
        })
}

/// The voicebank a freshly-installed vocal lane carries — i.e. "the user
/// has not chosen one". Read from `VocalParams::default()` so this stays
/// true if that default is ever changed.
fn default_lane_voicebank() -> VocalVoicebank {
    resonance_music_theory::VocalParams::default().voicebank
}

/// Does the lane have a MIDI clip holding at least one note?
///
/// Resolves the clip the same way `rerender_vocal_audio` does — the
/// derived clip of any placement of the section — so the pre-flight and
/// the render agree on what "has notes" means.
///
/// The `derived_clips` map is not always populated for a lane: a project
/// loaded from disk can carry the clip without the mapping being
/// rehydrated for it. `song.vocal` already tolerates that (its `lane_clip`
/// falls back to matching a clip by the placement's start sample), so
/// without the same fallback here the two disagreed — `song.vocal` would
/// report `note_count: 1, counts_mismatch: false` while `vocal.render`
/// refused the lane with "has no notes to sing", leaving it permanently
/// unrenderable. Observed on a lane whose placement starts at bar 269
/// while its clip reports bar 268.
fn lane_has_notes(app: &Resonance, definition_id: u64, track_id: u64) -> bool {
    app.compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .any(|p| {
            let mapped = app
                .compose
                .derived_clips
                .get(&(definition_id, p.id, track_id))
                .and_then(|clip_id| app.midi_clips.iter().find(|c| c.id == *clip_id));
            let clip = mapped.or_else(|| {
                let start = app.tempo_map.bar_to_sample(p.start_bar);
                app.midi_clips.iter().find(|c| {
                    c.track_id == track_id
                        && c.start_sample == start
                        // Never adopt a clip another lane already owns: the
                        // fallback exists for lanes whose mapping is missing,
                        // not to let one lane borrow a neighbour's clip.
                        && !app.compose.derived_clips.values().any(|id| *id == c.id)
                })
            });
            clip.is_some_and(|c| !c.notes.is_empty())
        })
}

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
            return Err(no_section_definition(definition_id));
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
            no_track(track_id)
        }
    })
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

