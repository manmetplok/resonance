//! `song.*` read-only introspection views (ba doc #265, todo #1148),
//! plus the read-only built-in plugin catalog (`track.plugins`).
//!
//! Every handler here executes against `&Resonance` — no mutation, no
//! undo entries, no engine traffic. The view types come from
//! `resonance-control` and are deliberately compact and LLM-oriented:
//! real app ids (usable verbatim in later mutation params), musical +
//! sample positions, lowercase enums, no UI/view state. All results
//! carry the app's `revision` counter so clients can detect concurrent
//! GUI edits.

use crate::compose::LaneGeneratorKind;
use crate::state::{BusState, TrackState};
use crate::Resonance;
use resonance_audio::types::TrackType;
use resonance_control::methods::song::{
    self, ChordView, ClipView, LyricLineView, NoteView, NotesParams, NotesView,
    SectionDefinitionView, SectionPlacementView, SectionsView, SongSummary, SyllableView,
    TrackDetail, TrackSummary, TracksParams, TracksView, VocalParams as VocalViewParams,
    VocalRenderState, VocalView,
};
use resonance_control::methods::track::{self, PluginCatalog, PluginCatalogEntry, PluginKind};
use resonance_control::{
    KeyScale, Request, Response, RpcError, SongPosition, TimeSignature, TrackKind, TransportState,
};
use resonance_music_theory::midi_note_name;

/// Ticks per quarter note — the app's MIDI clock resolution.
const TPQ: f64 = resonance_audio::types::TICKS_PER_QUARTER_NOTE as f64;

/// Handle a read-only introspection request, or `None` when `method`
/// belongs to another namespace. Called by `execute` after the
/// handshake / compatibility checks.
pub(super) fn try_handle(app: &Resonance, request: &Request) -> Option<Response> {
    let response = match request.method.as_str() {
        song::SUMMARY => summary(app, request),
        song::SECTIONS => sections(app, request),
        song::TRACKS => tracks(app, request),
        song::NOTES => notes(app, request),
        song::VOCAL => vocal(app, request),
        track::PLUGINS => plugin_catalog(app, request),
        _ => return None,
    };
    Some(response)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn summary(app: &Resonance, request: &Request) -> Response {
    let end_sample = song_end_sample(app);
    let (end_bar, end_frac) = app
        .tempo_map
        .sample_to_bar(end_sample, app.sample_rate);
    let result = SongSummary {
        tempo_bpm: app.transport.bpm as f64,
        time_signature: TimeSignature {
            numerator: app.transport.time_sig_num,
            denominator: app.transport.time_sig_den,
        },
        key: song_key(app),
        sample_rate: app.sample_rate,
        // `sample_to_bar` bars are 0-based, so bar+frac IS the length.
        length_bars: end_bar as f64 + end_frac,
        length_samples: end_sample,
        transport: transport_state(app),
        playhead: song_position(app, app.transport.playhead),
        sections: placement_views(app),
        tracks: track_summaries(app),
        revision: app.revision(),
    };
    super::success(request, &result)
}

fn sections(app: &Resonance, request: &Request) -> Response {
    let definitions = app
        .compose
        .definitions
        .iter()
        .map(|d| SectionDefinitionView {
            id: d.id.into(),
            name: d.name.clone(),
            length_bars: d.length_bars,
            scale: d.scale.as_ref().map(key_scale),
            chords: d
                .chords
                .iter()
                .map(|c| ChordView {
                    id: c.id.into(),
                    start_beat: c.start_beat as f64,
                    duration_beats: c.duration_beats as f64,
                    symbol: c.chord.to_string(),
                })
                .collect(),
        })
        .collect();
    let result = SectionsView {
        definitions,
        placements: placement_views(app),
        revision: app.revision(),
    };
    super::success(request, &result)
}

/// Parse params for a method whose params are entirely optional:
/// absent/null params mean "defaults". (`Request::params` alone maps
/// absent to JSON `null`, which serde refuses to turn into a struct.)
fn optional_params<T: serde::de::DeserializeOwned + Default>(
    request: &Request,
) -> Result<T, RpcError> {
    match &request.params {
        None | Some(serde_json::Value::Null) => Ok(T::default()),
        Some(_) => request.params(),
    }
}

fn tracks(app: &Resonance, request: &Request) -> Response {
    let params: TracksParams = match optional_params(request) {
        Ok(p) => p,
        Err(e) => return super::failure(request, e),
    };
    let details: Vec<TrackDetail> = app
        .sorted_tracks()
        .iter()
        .filter(|t| params.track_id.is_none_or(|id| id.0 == t.id))
        .map(|t| track_detail(app, t))
        .collect();
    if details.is_empty() {
        if let Some(id) = params.track_id {
            // Busses appear in `song.summary` but carry no clips; a
            // bus id here is a miss like any other unknown id.
            return super::failure(request, RpcError::not_found(format!("no track with id {id}")));
        }
    }
    let result = TracksView {
        tracks: details,
        revision: app.revision(),
    };
    super::success(request, &result)
}

fn notes(app: &Resonance, request: &Request) -> Response {
    let params: NotesParams = match request.params() {
        Ok(p) => p,
        Err(e) => return super::failure(request, e),
    };
    let Some(clip) = app.midi_clips.iter().find(|c| c.id == params.clip_id.0) else {
        let detail = if app.clips.iter().any(|c| c.id == params.clip_id.0) {
            format!(
                "clip {} is an audio clip; song.notes reads MIDI clips",
                params.clip_id
            )
        } else {
            format!("no MIDI clip with id {}", params.clip_id)
        };
        return super::failure(request, RpcError::not_found(detail));
    };
    let notes = clip
        .notes
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            params.range.is_none_or(|r| {
                let start = n.start_tick as f64 / TPQ;
                let end = (n.start_tick + n.duration_ticks) as f64 / TPQ;
                start < r.end_beat && end > r.start_beat
            })
        })
        .map(|(index, n)| NoteView {
            id: None, // The app addresses notes by index, not stable id.
            index,
            pitch: n.note,
            pitch_name: midi_note_name(n.note),
            start_tick: n.start_tick,
            start_beat: n.start_tick as f64 / TPQ,
            duration_ticks: n.duration_ticks,
            duration_beats: n.duration_ticks as f64 / TPQ,
            velocity: velocity_to_midi(n.velocity),
        })
        .collect();
    let result = NotesView {
        clip_id: params.clip_id,
        notes,
        revision: app.revision(),
    };
    super::success(request, &result)
}

fn vocal(app: &Resonance, request: &Request) -> Response {
    let params: VocalViewParams = match optional_params(request) {
        Ok(p) => p,
        Err(e) => return super::failure(request, e),
    };
    let track = match params.track_id {
        Some(id) => {
            let Some(t) = app.sorted_tracks().iter().find(|t| t.id == id.0) else {
                return super::failure(request, RpcError::not_found(format!("no track with id {id}")));
            };
            if t.track_type != TrackType::Vocal {
                return super::failure(
                    request,
                    RpcError::invalid_params(format!("track {id} is not a vocal track")),
                );
            }
            t
        }
        None => {
            let Some(t) = app
                .sorted_tracks()
                .iter()
                .find(|t| t.track_type == TrackType::Vocal)
            else {
                return super::failure(request, RpcError::not_found("the project has no vocal track"));
            };
            t
        }
    };

    // The project pronunciation dictionary, both as the resolver input
    // and as the view's override map. (A global user dictionary has no
    // on-disk store yet — same as the render path.)
    let dictionary: resonance_music_theory::g2p::PhonemeDictionary = app
        .compose
        .pronunciation
        .project_dictionary
        .iter()
        .map(|e| (e.word.clone(), e.phonemes.clone()))
        .collect();
    let pronunciation_overrides = app
        .compose
        .pronunciation
        .project_dictionary
        .iter()
        .map(|e| {
            (
                e.word.clone(),
                e.phonemes.iter().map(|p| (*p).to_owned()).collect(),
            )
        })
        .collect();

    // Lyrics live per (section definition, track) vocal lane. Walk the
    // arrangement in placement order (each definition once) so the
    // lines read in song order.
    let mut lines = Vec::new();
    for definition in definitions_in_placement_order(app) {
        let Some(config) = definition.lane_generators.get(&track.id) else {
            continue;
        };
        let LaneGeneratorKind::Vocal(vocal_params) = &config.kind else {
            continue;
        };
        for line in &vocal_params.draft {
            let syllables =
                resonance_music_theory::g2p::resolve_draft_with_dict(
                    std::slice::from_ref(line),
                    &dictionary,
                )
                .into_iter()
                .map(|s| SyllableView {
                    text: s.label,
                    phonemes: s.phonemes.iter().map(|p| (*p).to_owned()).collect(),
                })
                .collect();
            lines.push(LyricLineView {
                index: lines.len(),
                text: line.text.clone(),
                syllables,
            });
        }
    }

    let result = VocalView {
        track_id: resonance_control::ids::TrackId(track.id),
        lines,
        pronunciation_overrides,
        render_state: vocal_render_state(app, track.id),
        revision: app.revision(),
    };
    super::success(request, &result)
}

/// `track.plugins` — the addable plugin catalog (read-only; the rest of
/// `track.*` lands in todo #1152). Ids are the CLAP plugin ids the
/// scanner reported; they are what `track.add_instrument` /
/// `track.add_effect` will accept.
fn plugin_catalog(app: &Resonance, request: &Request) -> Response {
    let plugins = app
        .available_plugins
        .iter()
        .map(|p| PluginCatalogEntry {
            id: p.clap_plugin_id.clone(),
            name: p.name.clone(),
            kind: if p.is_instrument {
                PluginKind::Instrument
            } else {
                PluginKind::Effect
            },
        })
        .collect();
    super::success(request, &PluginCatalog { plugins })
}

// ---------------------------------------------------------------------------
// View builders
// ---------------------------------------------------------------------------

/// Resolve a sample position into the protocol's bar/beat/sample triple
/// (1-based bar, 1-based fractional beat).
fn song_position(app: &Resonance, sample: u64) -> SongPosition {
    let (bar, beat, frac) = app.tempo_map.position_to_bars(sample, app.sample_rate);
    SongPosition {
        bar,
        beat: beat as f64 + frac,
        sample,
    }
}

fn transport_state(app: &Resonance) -> TransportState {
    if app.transport.recording {
        TransportState::Recording
    } else if app.transport.playing {
        TransportState::Playing
    } else {
        TransportState::Stopped
    }
}

/// The song key: the first (lowest-sample) key change on the global
/// chord track, per its "song key" convention.
fn song_key(app: &Resonance) -> Option<KeyScale> {
    app.chord_track.key_changes.first().map(|k| key_scale(&k.scale))
}

fn key_scale(scale: &resonance_music_theory::Scale) -> KeyScale {
    KeyScale {
        tonic: scale.root.to_string(),
        scale: scale.mode.as_str().to_owned(),
    }
}

/// Ordered section arrangement: placements sorted by start bar, names
/// denormalized from their definitions. App bars are 0-based; the wire
/// is 1-based.
fn placement_views(app: &Resonance) -> Vec<SectionPlacementView> {
    let mut placements: Vec<&crate::compose::SectionPlacementState> =
        app.compose.placements.iter().collect();
    placements.sort_by_key(|p| p.start_bar);
    placements
        .into_iter()
        .filter_map(|p| {
            let def = app.compose.definitions.iter().find(|d| d.id == p.definition_id)?;
            Some(SectionPlacementView {
                id: p.id.into(),
                definition_id: def.id.into(),
                name: def.name.clone(),
                start_bar: p.start_bar + 1,
                length_bars: def.length_bars,
            })
        })
        .collect()
}

/// Definitions in the order the arrangement first plays them (each
/// definition once), then any unplaced definitions in creation order.
fn definitions_in_placement_order(
    app: &Resonance,
) -> Vec<&crate::compose::SectionDefinitionState> {
    let mut seen = std::collections::HashSet::new();
    let mut ordered = Vec::new();
    for view in placement_views(app) {
        if seen.insert(u64::from(view.definition_id)) {
            if let Some(def) = app
                .compose
                .definitions
                .iter()
                .find(|d| d.id == u64::from(view.definition_id))
            {
                ordered.push(def);
            }
        }
    }
    for def in &app.compose.definitions {
        if seen.insert(def.id) {
            ordered.push(def);
        }
    }
    ordered
}

/// Tracks (in mixer order) followed by busses, as compact summaries.
/// Bus ids come from a distinct allocation range, so they never collide
/// with track ids on the wire.
fn track_summaries(app: &Resonance) -> Vec<TrackSummary> {
    let mut out: Vec<TrackSummary> = app
        .sorted_tracks()
        .iter()
        .map(|t| track_summary(app, t))
        .collect();
    out.extend(app.sorted_busses().iter().map(bus_summary));
    out
}

fn track_summary(app: &Resonance, t: &TrackState) -> TrackSummary {
    TrackSummary {
        id: resonance_control::ids::TrackId(t.id),
        name: t.name.clone(),
        kind: track_kind(t),
        instrument: instrument_summary(app, t),
        muted: t.muted,
        soloed: t.soloed,
        volume: db_to_linear(t.volume),
        pan: t.pan,
        clip_count: clip_count(app, t.id),
    }
}

fn bus_summary(b: &BusState) -> TrackSummary {
    TrackSummary {
        id: resonance_control::ids::TrackId(b.id),
        name: b.name.clone(),
        kind: TrackKind::Bus,
        instrument: None,
        muted: b.muted,
        soloed: false,
        volume: db_to_linear(b.volume),
        pan: b.pan,
        clip_count: 0,
    }
}

fn track_kind(t: &TrackState) -> TrackKind {
    match t.track_type {
        TrackType::Audio => TrackKind::Audio,
        TrackType::Vocal => TrackKind::Vocal,
        TrackType::Instrument => match t.instrument_type {
            crate::state::InstrumentType::Drum => TrackKind::Drums,
            crate::state::InstrumentType::Synth => TrackKind::Instrument,
        },
    }
}

/// The track's sound source, compact: the external-instrument device
/// (`"external:<device-id>"`) when the track drives outboard hardware,
/// else the instrument plugin's stable CLAP id (slot 0 on instrument
/// tracks). `None` for audio tracks and empty instrument tracks.
fn instrument_summary(app: &Resonance, t: &TrackState) -> Option<String> {
    if let Some(ext) = app.external_instruments.get(&t.id) {
        return Some(format!(
            "external:{}",
            ext.device_id.as_deref().unwrap_or("unconfigured")
        ));
    }
    if t.track_type == TrackType::Instrument {
        return t.plugins.first().map(|p| p.clap_plugin_id.clone());
    }
    None
}

/// Effect chain as stable CLAP plugin ids. On instrument tracks slot 0
/// is the instrument (reported via [`instrument_summary`]); everything
/// after it is an insert effect. On other tracks the whole chain is
/// effects.
fn effect_chain(t: &TrackState) -> Vec<String> {
    let skip = usize::from(t.track_type == TrackType::Instrument && !t.plugins.is_empty());
    t.plugins
        .iter()
        .skip(skip)
        .map(|p| p.clap_plugin_id.clone())
        .collect()
}

fn track_detail(app: &Resonance, t: &TrackState) -> TrackDetail {
    let mut clips: Vec<ClipView> = Vec::new();
    for c in app.clips.iter().filter(|c| c.track_id == t.id) {
        let start_tick = app.tempo_map.sample_to_abs_tick(c.start_sample, app.sample_rate);
        let end_tick = app
            .tempo_map
            .sample_to_abs_tick(c.start_sample + c.duration_samples, app.sample_rate);
        clips.push(ClipView {
            id: resonance_control::ids::ClipId(c.id),
            name: (!c.name.is_empty()).then(|| c.name.clone()),
            start: song_position(app, c.start_sample),
            length_beats: (end_tick.saturating_sub(start_tick)) as f64 / TPQ,
            length_samples: c.duration_samples,
            midi: false,
        });
    }
    for m in app.midi_clips.iter().filter(|m| m.track_id == t.id) {
        let end_sample =
            app.tempo_map
                .tick_to_abs_sample(m.start_sample, m.duration_ticks, app.sample_rate);
        clips.push(ClipView {
            id: resonance_control::ids::ClipId(m.id),
            name: (!m.name.is_empty()).then(|| m.name.clone()),
            start: song_position(app, m.start_sample),
            length_beats: m.duration_ticks as f64 / TPQ,
            length_samples: end_sample.saturating_sub(m.start_sample),
            midi: true,
        });
    }
    clips.sort_by_key(|c| c.start.sample);
    TrackDetail {
        summary: track_summary(app, t),
        effects: effect_chain(t),
        // Cache attached (valid or stale): the #576 frozen-input
        // classifier rejects note/lyric/instrument/param edits, so the
        // client needs to see why its mutations bounce.
        frozen: app.freeze.status(t.id).is_frozen(),
        clips,
    }
}

fn clip_count(app: &Resonance, id: resonance_audio::types::TrackId) -> usize {
    app.clips.iter().filter(|c| c.track_id == id).count()
        + app.midi_clips.iter().filter(|m| m.track_id == id).count()
}

/// Last sample of the song: the furthest end over audio clips, MIDI
/// clips, and placed sections. 0 for an empty project.
fn song_end_sample(app: &Resonance) -> u64 {
    let mut end: u64 = 0;
    for c in &app.clips {
        end = end.max(c.start_sample + c.duration_samples);
    }
    for m in &app.midi_clips {
        end = end.max(app.tempo_map.tick_to_abs_sample(
            m.start_sample,
            m.duration_ticks,
            app.sample_rate,
        ));
    }
    for p in &app.compose.placements {
        if let Some(def) = app.compose.definitions.iter().find(|d| d.id == p.definition_id) {
            end = end.max(app.tempo_map.bar_to_sample(p.start_bar + def.length_bars));
        }
    }
    end
}

/// SVS render state of a vocal track, from the vocal-audio registry:
/// an installed render for any of the track's lanes -> `rendered`; a
/// queued render epoch with nothing installed yet -> `rendering`; else
/// `not_rendered`. (Staleness/error tracking has no persistent app
/// state to read yet.)
fn vocal_render_state(
    app: &Resonance,
    track: resonance_audio::types::TrackId,
) -> VocalRenderState {
    let installed = app
        .compose
        .vocal_audio
        .clips
        .keys()
        .any(|(_, _, t)| *t == track);
    if installed {
        return VocalRenderState::Rendered;
    }
    let queued = app
        .compose
        .vocal_audio
        .render_epoch
        .keys()
        .any(|(_, t)| *t == track);
    if queued {
        VocalRenderState::Rendering
    } else {
        VocalRenderState::NotRendered
    }
}

/// dB fader value -> linear gain (protocol convention: 1.0 = unity).
fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// The app stores velocity as `0.0..=1.0`; the wire uses MIDI `0..=127`.
fn velocity_to_midi(velocity: f32) -> u8 {
    (velocity.clamp(0.0, 1.0) * 127.0).round() as u8
}
