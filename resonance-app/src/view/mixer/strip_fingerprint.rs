//! Lazy-cache fingerprints for the mixer channel strips.
//!
//! Every strip's non-live body (head, buttons, plugin chain, automation
//! header, pan) is wrapped in `iced::widget::lazy` keyed on one of these
//! hashes, so the widget subtree only rebuilds when something it renders
//! actually changed — not on every 16 ms redraw tick (MEMORY ui-work
//! §11, same discipline as `view_track_headers` and the mixer
//! inspector's ROUTING/CHAIN region).
//!
//! The live meter block (`fader_section`, which embeds the per-tick
//! `StereoMeterCanvas` levels) and the collapsed-parent sub-track meter
//! column stay OUTSIDE the lazy region, exactly like the inspector's
//! SIGNAL group — never key a lazy region without the live data it
//! renders. The level fields are therefore deliberately absent from
//! every hash below.
//!
//! **Rule:** every piece of state a strip *body* renders must enter its
//! fingerprint — a missed field is a stale-UI bug (the cached tree
//! survives the state change and the strip keeps drawing the old
//! value). The tests in `tests/mixer/mixer_automation_controls.rs`
//! walk the rendered facets table-driven and assert each one moves the
//! hash.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use resonance_audio::types::ScannedPlugin;

use crate::state::{BusState, ExternalInstrumentStatus, PluginSlotState, TrackState};

use super::automation::{primary_lane, AutoChan};

/// Hash everything `view_plugin_slot_row` draws for one chain slot, plus
/// the slot's params (they are the automation picker's option labels).
fn hash_slot(h: &mut DefaultHasher, slot: &PluginSlotState, selected: bool) {
    slot.instance_id.hash(h);
    slot.plugin_name.hash(h);
    // The pill's error treatment keys off missing-vs-available only; the
    // reason string renders in the parameter panel, not on the strip.
    slot.availability.is_missing().hash(h);
    slot.has_gui.hash(h);
    slot.editor_open.hash(h);
    slot.bypassed.hash(h);
    selected.hash(h);
    // The automation header's picker builds one option per non-hidden
    // param, labelled "<plugin>: <param>" — so id/name/hidden are all
    // rendered state (in the dropdown overlay).
    slot.params.len().hash(h);
    for p in &slot.params {
        p.id.hash(h);
        p.hidden.hash(h);
        p.name.hash(h);
    }
}

/// Hash the automation lane header's rendered state for one channel: the
/// surfaced primary lane (its target picks the label, `enabled` tints the
/// READ toggle), or the lane-less "picker only" state.
fn hash_auto_header(
    h: &mut DefaultHasher,
    r: &crate::Resonance,
    chan: AutoChan,
    plugins: &[PluginSlotState],
) {
    match primary_lane(&r.automation, chan, plugins) {
        Some(lane) => {
            1u8.hash(h);
            lane.target.hash(h);
            lane.enabled.hash(h);
        }
        None => 0u8.hash(h),
    }
}

/// Hash a cached `ScannedPlugin` option list (the `+ FX` /
/// `+ Instrument` picker options). Rebuilt only on a plugin scan, so
/// this is cheap and rarely moves.
fn hash_scanned(h: &mut DefaultHasher, plugins: &[ScannedPlugin]) {
    plugins.len().hash(h);
    for p in plugins {
        p.name.hash(h);
    }
}

/// Fingerprint for a top-level track strip's lazy body — everything
/// `channel_strip_body` renders. Levels are absent on purpose: the
/// fader/meter block is built outside the lazy region every frame.
pub(super) fn track_strip_fingerprint(r: &crate::Resonance, track: &TrackState) -> u64 {
    let mut h = DefaultHasher::new();
    // Head: glyph, name, caret, indent.
    track.id.hash(&mut h);
    track.name.hash(&mut h);
    track.track_type.hash(&mut h);
    track.instrument_icon.hash(&mut h);
    let has_sub_tracks = r
        .registry
        .tracks
        .iter()
        .any(|t| matches!(t.sub_track, Some(link) if link.parent_track_id == track.id));
    has_sub_tracks.hash(&mut h);
    r.mixer
        .expanded_sub_track_parents
        .contains(&track.id)
        .hash(&mut h);
    r.track_groups.indent_depth(track.id).hash(&mut h);
    // Button rows (M/S/●/🎧 + mono/FX-bypass/bounce).
    track.muted.hash(&mut h);
    track.soloed.hash(&mut h);
    track.record_armed.hash(&mut h);
    track.monitor_enabled.hash(&mut h);
    track.mono.hash(&mut h);
    track.fx_bypassed.hash(&mut h);
    crate::update::track::classify_bounce(track, r.midi_clips.iter().map(|c| c.track_id))
        .is_ok()
        .hash(&mut h);
    // Instrument pill + FX chain rows (and the reorder carets, whose
    // enabled directions are a pure function of the chain hashed here).
    track.plugins.len().hash(&mut h);
    for p in &track.plugins {
        hash_slot(&mut h, p, r.mixer.selected_plugin == Some(p.instance_id));
    }
    // External-instrument head pill, offline flag and summary chips.
    let ext = r.external_instruments.get(&track.id);
    ext.is_some().hash(&mut h);
    if let Some(ext) = ext {
        ext.midi_out_offline.hash(&mut h);
        ext.return_input_offline.hash(&mut h);
        ext.bank.hash(&mut h);
        ext.program.hash(&mut h);
        ext.device_id.hash(&mut h);
        (match ext.status(track) {
            ExternalInstrumentStatus::Unconfigured => 0u8,
            ExternalInstrumentStatus::Configuring => 1u8,
            ExternalInstrumentStatus::Live => 2u8,
            ExternalInstrumentStatus::Offline => 3u8,
        })
        .hash(&mut h);
        track.midi_output_device.hash(&mut h);
        track.midi_output_channel.hash(&mut h);
        track.input_device_name.hash(&mut h);
        track.input_port_index.hash(&mut h);
        // Named device params feed the automation picker's option list.
        if let Some(def) = ext.device_id.as_deref().and_then(|id| r.device_registry.get(id)) {
            for p in &def.params {
                p.id.hash(&mut h);
                p.name.hash(&mut h);
                p.group.hash(&mut h);
            }
        }
    }
    // Automation lane header + the live pan tint. The tint is a real
    // rendered value inside the body, so it enters the key — it only
    // moves while a Read-enabled pan lane plays back, in which case the
    // rebuild is exactly the repaint we want.
    hash_auto_header(&mut h, r, AutoChan::Track(track.id), &track.plugins);
    track.pan.to_bits().hash(&mut h);
    super::automation::live_value(
        &r.automation,
        resonance_common::AutomationTarget::TrackPan(track.id),
    )
    .map(f32::to_bits)
    .hash(&mut h);
    // "+ Instrument" picker options / "No instruments" fallback.
    hash_scanned(&mut h, &r.view_caches.instrument_plugins);
    r.available_plugins.is_empty().hash(&mut h);
    h.finish()
}

/// Fingerprint for a sub-track strip's lazy body (head + M/S/FX-bypass +
/// pan). The rail/border selection tint and the fader/meter block are
/// built outside the lazy region.
pub(super) fn sub_strip_fingerprint(track: &TrackState) -> u64 {
    let mut h = DefaultHasher::new();
    track.id.hash(&mut h);
    track.name.hash(&mut h);
    track.muted.hash(&mut h);
    track.soloed.hash(&mut h);
    track.fx_bypassed.hash(&mut h);
    track.pan.to_bits().hash(&mut h);
    h.finish()
}

/// Fingerprint for a bus strip's lazy body — everything
/// `bus_strip_body` renders (name, buttons, chain, `+ FX` picker,
/// automation header, pan). Selection tints the outer border only, so
/// it stays out of the key.
pub(super) fn bus_strip_fingerprint(r: &crate::Resonance, bus: &BusState) -> u64 {
    let mut h = DefaultHasher::new();
    bus.id.hash(&mut h);
    bus.name.hash(&mut h);
    bus.muted.hash(&mut h);
    bus.fx_bypassed.hash(&mut h);
    bus.plugins.len().hash(&mut h);
    for p in &bus.plugins {
        hash_slot(&mut h, p, r.mixer.selected_plugin == Some(p.instance_id));
    }
    hash_scanned(&mut h, &r.view_caches.fx_plugins);
    hash_auto_header(&mut h, r, AutoChan::Bus(bus.id), &bus.plugins);
    bus.pan.to_bits().hash(&mut h);
    super::automation::live_value(
        &r.automation,
        resonance_common::AutomationTarget::BusPan(bus.id),
    )
    .map(f32::to_bits)
    .hash(&mut h);
    h.finish()
}

/// Fingerprint for the master strip's lazy body (FX-bypass toggle,
/// chain, `+ FX` picker, automation header). The fader/meter and the
/// transient Bounce button stay outside the lazy region.
pub(super) fn master_strip_fingerprint(r: &crate::Resonance) -> u64 {
    let mut h = DefaultHasher::new();
    r.master_fx_bypassed.hash(&mut h);
    r.master_plugins.len().hash(&mut h);
    for p in &r.master_plugins {
        hash_slot(&mut h, p, r.mixer.selected_plugin == Some(p.instance_id));
    }
    hash_scanned(&mut h, &r.view_caches.fx_plugins);
    hash_auto_header(&mut h, r, AutoChan::Master, &r.master_plugins);
    h.finish()
}

impl crate::Resonance {
    /// Test-only: the lazy-body fingerprint the mixer computes for this
    /// track's strip (sub-tracks resolve to the sub-strip variant).
    /// Everything a strip body renders MUST move this hash — a missed
    /// field means the retained tree keeps drawing stale state — while
    /// the live meter levels must NOT. `None` when the track is unknown.
    #[doc(hidden)]
    pub fn test_track_strip_fingerprint(
        &self,
        track_id: resonance_audio::types::TrackId,
    ) -> Option<u64> {
        let track = self.registry.tracks.iter().find(|t| t.id == track_id)?;
        Some(if track.sub_track.is_some() {
            sub_strip_fingerprint(track)
        } else {
            track_strip_fingerprint(self, track)
        })
    }

    /// Test-only: same as [`Self::test_track_strip_fingerprint`] for a bus.
    #[doc(hidden)]
    pub fn test_bus_strip_fingerprint(
        &self,
        bus_id: resonance_audio::types::BusId,
    ) -> Option<u64> {
        let bus = self.registry.busses.iter().find(|b| b.id == bus_id)?;
        Some(bus_strip_fingerprint(self, bus))
    }

    /// Test-only: same as [`Self::test_track_strip_fingerprint`] for the
    /// master strip.
    #[doc(hidden)]
    pub fn test_master_strip_fingerprint(&self) -> u64 {
        master_strip_fingerprint(self)
    }

    /// Test-only: poke a track's live meter levels directly, so a test
    /// can prove the strip fingerprints ignore them (the meter renders
    /// outside the lazy region).
    #[doc(hidden)]
    pub fn test_set_track_levels(
        &mut self,
        track_id: resonance_audio::types::TrackId,
        level_l: f32,
        level_r: f32,
    ) {
        if let Some(track) = self.registry.tracks.iter_mut().find(|t| t.id == track_id) {
            track.level_l = level_l;
            track.level_r = level_r;
        }
    }
}
