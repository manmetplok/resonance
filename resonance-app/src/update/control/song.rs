//! `song.*` read-only introspection views (ba doc #265, todo #1148),
//! plus the read-only installed-plugin catalog (`plugins.catalog`).
//!
//! Every handler here executes against `&Resonance` — no mutation, no
//! undo entries, no engine traffic. The view types come from
//! `resonance-control` and are deliberately compact and LLM-oriented:
//! real app ids (usable verbatim in later mutation params), musical +
//! sample positions, lowercase enums, no UI/view state. All results
//! carry the app's `revision` counter so clients can detect concurrent
//! GUI edits.

use crate::compose::LaneGeneratorKind;
use crate::plugin_chain::instrument_slot;
use crate::state::{BusState, TrackState};
use crate::Resonance;
use resonance_audio::types::{TrackOutput, TrackType};
use resonance_control::methods::song::{
    self, ChordView, ClipView, LyricLineView, NoteView, NotesParams, NotesView,
    SectionDefinitionView, SectionPlacementView, SectionsView, SongSummary, SyllableView,
    TrackDetail, TrackSummary, TracksParams, TracksView, VocalParams as VocalViewParams,
    VocalRenderState, VocalView,
};
use resonance_control::methods::plugins::{self, PluginCatalog, PluginCatalogEntry};
use resonance_control::methods::track::{self, PluginKind};
use resonance_control::{
    KeyScale, Request, Response, RpcError, SongPosition, TimeSignature, TrackKind,
    TrackOutput as WireTrackOutput, TransportState,
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
        // `plugins.catalog`, plus the deprecated `track.plugins`
        // spelling it was renamed from (todo #1236) — one handler, so
        // the two can never answer differently.
        #[allow(deprecated)]
        m if m == plugins::CATALOG || m == plugins::PLUGINS_DEPRECATED_ALIAS => {
            plugin_catalog(app, request)
        }
        track::PLUGIN_PARAMS => plugin_params(app, request),
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

use super::optional_params;

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
    let mut lanes = Vec::new();
    for definition in definitions_in_placement_order(app) {
        let Some(config) = definition.lane_generators.get(&track.id) else {
            continue;
        };
        let LaneGeneratorKind::Vocal(vocal_params) = &config.kind else {
            continue;
        };
        // Per-lane identity + the render pre-flight counts (ba doc #269
        // FR-3/FR-7). Without these a client cannot tell which lane a
        // write landed on, and a notes/syllables mismatch only surfaces
        // as a render-time failure.
        let syllable_count = resonance_music_theory::g2p::resolve_draft_with_dict(
            &vocal_params.draft,
            &dictionary,
        )
        .len();
        let note_count = lane_note_count(app, definition.id, track.id);
        // Intelligibility pre-flight: per note, can the phonemes assigned
        // to it actually be articulated in the time it has, and is it
        // pitched where the voicebank sings clearly? Both failures render
        // "successfully" and simply sound like mush, so without this a
        // client had to bounce audio and listen to find out.
        let articulation = lane_articulation(app, definition.id, track.id, vocal_params);
        let short_note_count = articulation.iter().filter(|n| n.too_short).count();
        let out_of_range_note_count = articulation.iter().filter(|n| n.out_of_range).count();
        let (range_lo, range_hi) =
            crate::compose::vocal_svs::comfortable_pitch_range(vocal_params.voicebank);
        lanes.push(song::VocalLaneView {
            definition_id: definition.id.into(),
            name: definition.name.clone(),
            start_bar: app
                .compose
                .placements
                .iter()
                .filter(|p| p.definition_id == definition.id)
                .map(|p| p.start_bar)
                .min()
                // Placement bars are 0-based in the app, 1-based on the wire.
                .map(|bar| bar + 1),
            note_count,
            syllable_count,
            // A lane with lyrics but no notes is the case that most
            // needs flagging — it renders nothing a client expects —
            // and it was precisely the case the old `note_count > 0`
            // guard hid, reporting `counts_mismatch: false` and giving
            // false confidence right before a render (ba doc #271).
            // "Not generated yet" stays legible as `note_count == 0`.
            counts_mismatch: syllable_count > 0 && note_count != syllable_count,
            voicebank: Some(vocal_params.voicebank.as_str().to_owned()),
            comfortable_range: Some(song::PitchRangeView {
                low: range_lo,
                high: range_hi,
                low_name: midi_note_name(range_lo),
                high_name: midi_note_name(range_hi),
            }),
            notes: articulation
                .iter()
                .map(|a| song::VocalNoteView {
                    index: a.note_index,
                    syllable: a.label.clone(),
                    phonemes: a.phonemes.iter().map(|p| (*p).to_owned()).collect(),
                    phoneme_count: a.phonemes.len(),
                    pitch: a.pitch,
                    pitch_name: midi_note_name(a.pitch),
                    duration_ms: (a.duration_sec * 1000.0).round(),
                    min_duration_ms: (a.min_duration_sec * 1000.0).round(),
                    too_short: a.too_short,
                    out_of_range: a.out_of_range,
                })
                .collect(),
            short_note_count,
            out_of_range_note_count,
        });
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
        lanes,
        lines,
        pronunciation_overrides,
        render_state: vocal_render_state(app, track.id),
        revision: app.revision(),
    };
    super::success(request, &result)
}

/// `plugins.catalog` — the catalog of INSTALLED plugins, i.e. what can
/// be loaded, not what a track currently carries (that is
/// `track.plugin_params`). Ids are the CLAP plugin ids the scanner
/// reported; they are what `track.add_instrument` / `track.add_effect`
/// accept.
///
/// Also answers the deprecated `track.plugins` alias — the name this
/// method carried until todo #1236, whose "track." prefix read as a
/// per-track query and cost a field agent a session's debugging.
///
/// Reads `app.available_plugins`, which the scanner fills at startup, so
/// it needs NO open project and is listed in
/// [`is_read_only_method`](super::is_read_only_method).
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

/// `track.plugin_params` — the plugins on one track and their
/// parameters (ba doc #272 V-3).
///
/// Until this existed a client could attach a plugin but never configure
/// it, so every instrument played its default patch: a pad, a lead and a
/// sub drone were the same wavetable with different notes, and the only
/// lever was track volume.
///
/// Plugins are addressed by the CLAP id `song.tracks` already reports,
/// with `occurrence` disambiguating a track that carries the same plugin
/// twice.
fn plugin_params(app: &Resonance, request: &Request) -> Response {
    let params: track::PluginParamsParams = match request.params() {
        Ok(p) => p,
        Err(e) => return super::failure(request, e),
    };
    let Some(t) = app.registry.tracks.iter().find(|t| t.id == params.track_id.0) else {
        return super::failure(
            request,
            RpcError::not_found(format!("no track with id {}", params.track_id)),
        );
    };

    let entries = plugin_entries(app, t);
    let plugins: Vec<track::PluginParamsEntry> = match &params.plugin_id {
        None => entries,
        Some(wanted) => {
            let occurrence = params.occurrence.unwrap_or(0);
            let matched: Vec<track::PluginParamsEntry> = entries
                .into_iter()
                .filter(|e| &e.plugin_id == wanted && e.occurrence == occurrence)
                .collect();
            if matched.is_empty() {
                return super::failure(request, unknown_plugin_on_track(app, t, wanted, occurrence));
            }
            matched
        }
    };

    let result = track::PluginParamsView {
        track_id: params.track_id,
        plugins,
        revision: app.revision(),
    };
    super::success(request, &result)
}

/// The track's plugins as wire entries, in chain order, each tagged with
/// its role and its occurrence index among same-id siblings.
pub(super) fn plugin_entries(app: &Resonance, t: &TrackState) -> Vec<track::PluginParamsEntry> {
    let instrument = instrument_slot(app, t);
    let mut seen: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    t.plugins
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let occurrence = seen
                .entry(p.clap_plugin_id.as_str())
                .and_modify(|n| *n += 1)
                .or_insert(0);
            track::PluginParamsEntry {
                plugin_id: p.clap_plugin_id.clone(),
                name: p.plugin_name.clone(),
                // Chain index, instrument included: slot order is
                // processing order (ba doc #273, todo #1223).
                slot: i as u32,
                occurrence: *occurrence,
                kind: if Some(i) == instrument {
                    PluginKind::Instrument
                } else {
                    PluginKind::Effect
                },
                params: p
                    .params
                    .iter()
                    .map(|param| track::PluginParamView {
                        id: param.id,
                        name: param.name.clone(),
                        value: param.current_value,
                        min: param.min_value,
                        max: param.max_value,
                        default: param.default_value,
                    })
                    .collect(),
            }
        })
        .collect()
}

/// "No such plugin on this track", listing what the track does carry so
/// the caller can correct the id rather than guess.
pub(super) fn unknown_plugin_on_track(
    app: &Resonance,
    t: &TrackState,
    wanted: &str,
    occurrence: u32,
) -> RpcError {
    let present: Vec<String> = plugin_entries(app, t)
        .iter()
        .map(|e| {
            if e.occurrence == 0 {
                e.plugin_id.clone()
            } else {
                format!("{} (occurrence {})", e.plugin_id, e.occurrence)
            }
        })
        .collect();
    RpcError::not_found(format!(
        "track {} has no plugin {wanted:?} at occurrence {occurrence}; it carries: [{}]",
        t.id,
        present.join(", ")
    ))
}

// ---------------------------------------------------------------------------
// View builders
// ---------------------------------------------------------------------------

/// Resolve a sample position into the protocol's bar/beat/sample triple
/// (1-based bar, 1-based fractional beat).
pub(super) fn song_position(app: &Resonance, sample: u64) -> SongPosition {
    let (bar, beat, frac) = app.tempo_map.position_to_bars(sample, app.sample_rate);
    SongPosition {
        bar,
        beat: beat as f64 + frac,
        sample,
    }
}

pub(super) fn transport_state(app: &Resonance) -> TransportState {
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
        kind: track_kind(app, t),
        instrument: instrument_summary(app, t),
        // Sub-tracks of a multi-output instrument: the ONLY signal a
        // client used to get was a `→` in the name (doc #273).
        parent_id: t
            .sub_track
            .map(|link| resonance_control::ids::TrackId(link.parent_track_id)),
        muted: t.muted,
        soloed: t.soloed,
        volume: db_to_linear(t.volume),
        // `TrackState.volume` is already dB — the linear `volume` above
        // is the derived one, not this.
        volume_db: t.volume,
        pan: t.pan,
        output: track_output(t.output),
        clip_count: clip_count(app, t.id),
    }
}

fn bus_summary(b: &BusState) -> TrackSummary {
    TrackSummary {
        id: resonance_control::ids::TrackId(b.id),
        name: b.name.clone(),
        kind: TrackKind::Bus,
        instrument: None,
        parent_id: None,
        muted: b.muted,
        soloed: false,
        volume: db_to_linear(b.volume),
        // Bus volume is stored in dB too.
        volume_db: b.volume,
        pan: b.pan,
        // Busses always feed master; nesting a bus into another bus is
        // not a routing the app models.
        output: WireTrackOutput::Master,
        clip_count: 0,
    }
}

/// The app's routing enum as its wire form.
fn track_output(output: TrackOutput) -> WireTrackOutput {
    match output {
        TrackOutput::Master => WireTrackOutput::Master,
        TrackOutput::Bus(id) => WireTrackOutput::Bus(resonance_control::ids::TrackId(id)),
    }
}

fn track_kind(app: &Resonance, t: &TrackState) -> TrackKind {
    // External-instrument mode has no track-type discriminant — the
    // engine track is a plain instrument track and the config
    // registered on top is what makes it external (cf. the
    // `external_instruments` map). It's reported as its own kind so a
    // client sees the same spelling `track.add` takes, and knows the
    // `external.*` methods apply.
    if app.external_instruments.contains_key(&t.id) {
        return TrackKind::External;
    }
    match t.track_type {
        TrackType::Audio => TrackKind::Audio,
        TrackType::Vocal => TrackKind::Vocal,
        TrackType::Instrument => match t.instrument_type {
            crate::state::InstrumentType::Drum => TrackKind::Drums,
            crate::state::InstrumentType::Synth => TrackKind::Instrument,
        },
    }
}

/// Chain index of the track's instrument, if it has one.
///

/// The track's sound source, compact: the external-instrument device
/// (`"external:<device-id>"`) when the track drives outboard hardware,
/// else the instrument plugin's stable CLAP id. `None` for audio tracks
/// and for instrument tracks holding no instrument.
///
/// When no device DEFINITION has been picked (the optional registry entry
/// that names patches and CC layout), this falls back to the track's
/// lifecycle state — the same word `external.status` reports — rather than
/// the flat `"unconfigured"` it used to print. A fully wired, recorded
/// track reading `external:unconfigured` here while `external.status` said
/// `live` was two answers to one question (ba doc #275 P1.5).
fn instrument_summary(app: &Resonance, t: &TrackState) -> Option<String> {
    if let Some(ext) = app.external_instruments.get(&t.id) {
        return Some(match ext.device_id.as_deref() {
            Some(device) => format!("external:{device}"),
            None => format!(
                "external:{}",
                super::external::status_label(ext.status(t))
            ),
        });
    }
    instrument_slot(app, t)
        .and_then(|i| t.plugins.get(i))
        .map(|p| p.clap_plugin_id.clone())
}

/// Effect chain as stable CLAP plugin ids, in chain order: every plugin
/// except the one [`instrument_slot`] identified as the instrument. On
/// non-instrument tracks the whole chain is effects.
fn effect_chain(app: &Resonance, t: &TrackState) -> Vec<String> {
    let instrument = instrument_slot(app, t);
    t.plugins
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != instrument)
        .map(|(_, p)| p.clap_plugin_id.clone())
        .collect()
}

/// This track's aux sends, in the mirror's insertion order (ba doc #273,
/// todo #1229). An agent that cannot hear must be able to read back what
/// it wired.
fn track_sends(app: &Resonance, t: &TrackState) -> Vec<song::SendView> {
    app.aux
        .sends
        .iter()
        .filter(|s| s.source == resonance_audio::types::SendSource::Track(t.id))
        .map(|s| song::SendView {
            send_id: resonance_control::ids::SendId(s.id),
            to_bus: resonance_control::ids::TrackId(s.dest),
            level_db: s.level_db,
            pre_fader: s.pre_fader,
            enabled: s.enabled,
        })
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
        effects: effect_chain(app, t),
        sends: track_sends(app, t),
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
///
/// Shared with `meter.*` (todo #1219), which clamps a measurement range
/// to it, so "the whole song" means the same thing to a reader and to a
/// measurement.
pub(super) fn song_end_sample(app: &Resonance) -> u64 {
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

/// Notes in a vocal lane's derived clip — what the SVS render actually
/// sings (ba doc #269 FR-7). A lane is derived once per placement of its
/// section, and every placement carries the same material, so the first
/// entry found for `(definition, track)` is the lane's note count. `0`
/// means the lane has not been generated yet.
fn lane_note_count(
    app: &Resonance,
    definition_id: u64,
    track_id: resonance_audio::types::TrackId,
) -> usize {
    // Prefer the derived-clip map, but do not trust it as the only
    // answer: a lane whose map entry is missing or points at a clip that
    // is no longer in `midi_clips` reported 0 notes while `song.notes`
    // on that lane's clip plainly returned some, which reads as "not
    // generated" and silenced the mismatch flag (ba doc #271).
    let mapped = app
        .compose
        .derived_clips
        .iter()
        .filter(|((def, _, track), _)| *def == definition_id && *track == track_id)
        .find_map(|(_, clip_id)| app.midi_clips.iter().find(|c| c.id == *clip_id));
    if let Some(clip) = mapped {
        return clip.notes.len();
    }
    // Fall back to the rule `rebuild_derived_clips` uses to recover the
    // mapping after a load: a MIDI clip on this track starting at one of
    // the section's placement bars is this lane's clip.
    app.compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .find_map(|p| {
            let start = app.tempo_map.bar_to_sample(p.start_bar);
            app.midi_clips
                .iter()
                .find(|c| c.track_id == track_id && c.start_sample == start)
        })
        .map_or(0, |clip| clip.notes.len())
}

/// The MIDI clip a vocal lane sings from, resolved exactly the way
/// [`lane_note_count`] counts its notes (derived-clip map first, then the
/// placement-start fallback) so the two never disagree about which clip
/// the lane owns.
fn lane_clip<'a>(
    app: &'a Resonance,
    definition_id: u64,
    track_id: resonance_audio::types::TrackId,
) -> Option<&'a crate::state::MidiClipState> {
    let mapped = app
        .compose
        .derived_clips
        .iter()
        .filter(|((def, _, track), _)| *def == definition_id && *track == track_id)
        .find_map(|(_, clip_id)| app.midi_clips.iter().find(|c| c.id == *clip_id));
    if mapped.is_some() {
        return mapped;
    }
    app.compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .find_map(|p| {
            let start = app.tempo_map.bar_to_sample(p.start_bar);
            app.midi_clips
                .iter()
                .find(|c| c.track_id == track_id && c.start_sample == start)
        })
}

/// Per-note articulation report for one vocal lane — the data behind
/// `song.vocal`'s `too_short` / `out_of_range` flags.
///
/// Resolves the lane's pronunciation the same way the render does
/// (`override > project-dict > CMU-auto`, then the voicebank's phoneme
/// substitutions) so the phonemes reported are the ones that will be
/// sung. A lane whose phonemes fail the voicebank gate outright reports
/// on the unsubstituted stream rather than nothing — the render will
/// refuse with its own precise error, and the durations are still true.
fn lane_articulation(
    app: &Resonance,
    definition_id: u64,
    track_id: resonance_audio::types::TrackId,
    params: &resonance_music_theory::VocalParams,
) -> Vec<crate::compose::vocal_svs::NoteArticulation> {
    let Some(clip) = lane_clip(app, definition_id, track_id) else {
        return Vec::new();
    };
    if clip.notes.is_empty() {
        return Vec::new();
    }
    let empty = std::collections::HashMap::new();
    let overrides = app
        .compose
        .pronunciation
        .clip_overrides(clip.id)
        .unwrap_or(&empty);
    let annotations = app
        .compose
        .vocal_audio
        .clip_lyrics
        .get(&clip.id)
        .cloned()
        .unwrap_or_else(|| vec![String::new(); clip.notes.len()]);
    let resolved = crate::compose::vocal_svs::resolve_clip_pronunciation(
        &params.draft,
        &annotations,
        clip.notes.len(),
        overrides,
        &app.compose.pronunciation.project_dictionary,
        &[],
    );
    let assigned = crate::compose::vocal_svs::validate_for_voicebank(&resolved, params.voicebank)
        .unwrap_or(resolved);
    crate::compose::vocal_svs::articulation_report(
        &clip.notes,
        &assigned,
        resonance_audio::types::TICKS_PER_QUARTER_NOTE as u32,
        // The render path reads the transport tempo the same way
        // (`vocal_render::rerender_vocal_audio`), so the durations
        // reported here are the ones the segment builder will divide up.
        app.transport.bpm,
        params.voicebank,
    )
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
