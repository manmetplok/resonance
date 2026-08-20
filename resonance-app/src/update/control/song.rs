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
//!
//! This file is the *handlers* only: parameter parsing, lookup failures,
//! and assembling the reply. Turning app state into wire types is
//! [`super::view_model`]'s job (ba todo #1256) — it is a library the
//! whole control layer reads, not something `song.*` owns.

use crate::compose::LaneGeneratorKind;
use crate::Resonance;
use resonance_audio::types::TrackType;
use resonance_control::methods::song::{
    self, ChordView, LyricLineView, NoteView, NotesParams, NotesView, SectionDefinitionView,
    SectionsView, SongSummary, SyllableView, TrackDetail, TracksParams, TracksView,
    VocalParams as VocalViewParams, VocalView,
};
use resonance_control::methods::plugins::{self, PluginCatalog, PluginCatalogEntry};
use resonance_control::methods::track::{self, PluginKind};
use resonance_control::{Request, Response, RpcError, TimeSignature};
use resonance_music_theory::midi_note_name;

use super::optional_params;
use super::view_model::{self, TPQ};

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

fn summary(app: &Resonance, request: &Request) -> Response {
    let end_sample = view_model::song_end_sample(app);
    let (end_bar, end_frac) = app
        .tempo_map
        .sample_to_bar(end_sample, app.sample_rate);
    let result = SongSummary {
        // The transport's BPM and meter track the PLAYHEAD, not the song
        // (ba todo #1381): on a song with changes they are whatever sits
        // under the cursor. They stay, because every client reads them,
        // but the two event lists below ship in the same response so a
        // client can see that they are only a local reading — and both
        // the wire docs and the `song_summary` tool description now say
        // so outright.
        tempo_bpm: app.transport.bpm as f64,
        time_signature: TimeSignature {
            numerator: app.transport.time_sig_num,
            denominator: app.transport.time_sig_den,
        },
        tempo_events: super::global::tempo_event_views(app),
        signature_events: super::global::signature_event_views(app),
        key: view_model::song_key(app),
        sample_rate: app.sample_rate,
        // `sample_to_bar` bars are 0-based, so bar+frac IS the length.
        length_bars: end_bar as f64 + end_frac,
        length_samples: end_sample,
        transport: view_model::transport_state(app),
        playhead: view_model::song_position(app, app.transport.playhead),
        sections: view_model::placement_views(app),
        tracks: view_model::track_summaries(app),
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
            scale: d.scale.as_ref().map(view_model::key_scale),
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
        placements: view_model::placement_views(app),
        revision: app.revision(),
    };
    super::success(request, &result)
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
        .map(|t| view_model::track_detail(app, t))
        .collect();
    if details.is_empty() {
        if let Some(id) = params.track_id {
            // Busses appear in `song.summary` but carry no clips; a
            // bus id here is a miss like any other unknown id.
            return super::failure(request, super::reply::no_track(id.into()));
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
            velocity: view_model::velocity_to_midi(n.velocity),
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
                return super::failure(request, super::reply::no_track(id.into()));
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
    for definition in view_model::definitions_in_placement_order(app) {
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
        let note_count = view_model::lane_note_count(app, definition.id, track.id);
        // Intelligibility pre-flight: per note, can the phonemes assigned
        // to it actually be articulated in the time it has, and is it
        // pitched where the voicebank sings clearly? Both failures render
        // "successfully" and simply sound like mush, so without this a
        // client had to bounce audio and listen to find out.
        let articulation =
            view_model::lane_articulation(app, definition.id, track.id, vocal_params);
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
        render_state: view_model::vocal_render_state(app, track.id),
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
        return super::failure(request, super::reply::no_track(params.track_id.into()));
    };

    let entries = view_model::plugin_entries(app, t);
    let plugins: Vec<track::PluginParamsEntry> = match &params.plugin_id {
        None => entries,
        Some(wanted) => {
            let occurrence = params.occurrence.unwrap_or(0);
            let matched: Vec<track::PluginParamsEntry> = entries
                .into_iter()
                .filter(|e| &e.plugin_id == wanted && e.occurrence == occurrence)
                .collect();
            if matched.is_empty() {
                return super::failure(
                    request,
                    view_model::unknown_plugin_on_track(app, t, wanted, occurrence),
                );
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
