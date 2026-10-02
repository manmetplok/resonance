//! Track / bus projections: summaries, routing, the plugin chain, and
//! the per-track detail `song.tracks` returns.
//!
//! [`plugin_entries`] and [`unknown_plugin_on_track`] are the chain
//! addressing every plugin-taking method shares (`track.set_plugin_param`,
//! `track.remove_effect` / `move_effect`, `track.set_sidechain`), so a
//! plugin is named the same way in a view and in an error.

use super::clip::{clip_count, track_clip_views};
use crate::plugin_chain::instrument_slot;
use crate::state::{BusState, TrackState};
use crate::update::control::automation::{lane_count, lane_summaries};
use crate::update::control::plugin_target::ChainOwner;
use crate::util::db_to_linear;
use crate::Resonance;
use resonance_audio::types::{TrackOutput, TrackType};
use resonance_control::methods::song::{self, TrackDetail, TrackSummary};
use resonance_control::methods::track::{self, PluginKind};
use resonance_control::{RpcError, TrackKind, TrackOutput as WireTrackOutput};

/// Tracks (in mixer order) followed by busses, as compact summaries.
/// Bus ids come from a distinct allocation range, so they never collide
/// with track ids on the wire.
pub(in crate::update::control) fn track_summaries(app: &Resonance) -> Vec<TrackSummary> {
    let mut out: Vec<TrackSummary> = app
        .sorted_tracks()
        .iter()
        .map(|t| track_summary(app, t))
        .collect();
    out.extend(app.sorted_busses().iter().map(|b| bus_summary(app, b)));
    out
}

pub(in crate::update::control) fn track_summary(app: &Resonance, t: &TrackState) -> TrackSummary {
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
        automation_lanes: lane_count(app, ChainOwner::Track(t.id)),
        // A sub-track reports its parent's colour, as its strip draws it.
        color: Some(track::format_hex_color(app.registry.display_color(t))),
    }
}

fn bus_summary(app: &Resonance, b: &BusState) -> TrackSummary {
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
        automation_lanes: lane_count(app, ChainOwner::Bus(b.id)),
        color: None,
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
    if app.devices.external_instruments.contains_key(&t.id) {
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
    if let Some(ext) = app.devices.external_instruments.get(&t.id) {
        return Some(match ext.device_id.as_deref() {
            Some(device) => format!("external:{device}"),
            None => format!(
                "external:{}",
                super::super::external::status_label(ext.status(t))
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

/// One track in full: its summary, its chain, its sends and its clips.
pub(in crate::update::control) fn track_detail(app: &Resonance, t: &TrackState) -> TrackDetail {
    TrackDetail {
        summary: track_summary(app, t),
        effects: effect_chain(app, t),
        sends: track_sends(app, t),
        automation: lane_summaries(app, ChainOwner::Track(t.id)),
        // Cache attached (valid or stale): the #576 frozen-input
        // classifier rejects note/lyric/instrument/param edits, so the
        // client needs to see why its mutations bounce.
        frozen: app.freeze.status(t.id).is_frozen(),
        clips: track_clip_views(app, t.id),
    }
}

/// One parameter as the control API reports it: the number, and
/// everything that says what the number means (ba todo #1290).
///
/// Shared by the track, bus and master chains — they publish the same
/// [`track::PluginParamView`], and three hand-written copies of this
/// mapping is how one of them would quietly stop reporting a field.
pub(in crate::update::control) fn param_view(
    param: &resonance_audio::types::ParamInfo,
) -> track::PluginParamView {
    track::PluginParamView {
        id: param.id,
        name: param.name.clone(),
        value: param.current_value,
        min: param.min_value,
        max: param.max_value,
        default: param.default_value,
        text: param.text.clone(),
        unit: param.unit.clone(),
        module: param.module.clone(),
        stepped: param.stepped,
        choices: param.choices.clone(),
        hidden: param.hidden,
        read_only: param.read_only,
        automatable: param.automatable,
    }
}

/// The track's plugins as wire entries, in chain order, each tagged with
/// its role and its occurrence index among same-id siblings.
pub(in crate::update::control) fn plugin_entries(
    app: &Resonance,
    t: &TrackState,
) -> Vec<track::PluginParamsEntry> {
    chain_entries(&t.plugins, instrument_slot(app, t))
}

/// A bus chain as wire entries, in processing order, each tagged with
/// its occurrence among same-id siblings.
///
/// Deliberately the same [`track::PluginParamsEntry`] shape
/// `track.plugin_params` returns, so a client reads a bus chain with the
/// code it already has. `kind` is always `Effect`: a bus has no
/// instrument slot.
pub(in crate::update::control) fn bus_plugin_entries(
    bus: &BusState,
) -> Vec<track::PluginParamsEntry> {
    chain_entries(&bus.plugins, None)
}

/// The master chain as wire entries, in processing order — the same
/// shape as [`plugin_entries`] / [`bus_plugin_entries`], every entry an
/// `Effect` (the master has no instrument slot).
pub(in crate::update::control) fn master_plugin_entries(
    app: &Resonance,
) -> Vec<track::PluginParamsEntry> {
    chain_entries(&app.master.plugins, None)
}

/// One chain as wire entries: the builder behind [`plugin_entries`],
/// [`bus_plugin_entries`] and [`master_plugin_entries`], so the three
/// chains cannot disagree about what an entry says. `instrument` is the
/// slot index to report as the instrument, if any.
fn chain_entries(
    plugins: &[crate::state::PluginSlotState],
    instrument: Option<usize>,
) -> Vec<track::PluginParamsEntry> {
    let mut seen: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    plugins
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let occurrence = seen
                .entry(p.clap_plugin_id.as_str())
                .and_modify(|n| *n += 1)
                .or_insert(0);
            track::PluginParamsEntry {
                bypassed: p.bypassed,
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
                status: slot_status(p),
                unavailable_reason: p.availability.reason().map(str::to_owned),
                params: p.params.iter().map(param_view).collect(),
            }
        })
        .collect()
}

/// A slot's wire status (ba doc #275 P5, todo #1309).
///
/// Shared by all three `plugin_params` builders — track, bus and master
/// fill the same [`track::PluginParamsEntry`], and a chain slot means
/// the same thing wherever it sits, so the three cannot be allowed to
/// answer this differently.
pub(in crate::update::control) fn slot_status(
    p: &crate::state::PluginSlotState,
) -> track::PluginSlotStatus {
    if p.availability.is_missing() {
        track::PluginSlotStatus::Missing
    } else {
        track::PluginSlotStatus::Loaded
    }
}

/// "No such plugin on this track", listing what the track does carry so
/// the caller can correct the id rather than guess.
pub(in crate::update::control) fn unknown_plugin_on_track(
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
