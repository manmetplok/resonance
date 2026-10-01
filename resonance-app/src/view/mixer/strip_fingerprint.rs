//! Lazy-cache fingerprints for the mixer channel strips.
//!
//! Every strip's non-live body (head, buttons, FX switch, slot lines,
//! pan) is wrapped in `iced::widget::lazy` keyed on one of these
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
//! value). The tests in `tests/mixer/mixer_automation_controls.rs` and
//! `tests/mixer/mixer_strip_anatomy.rs` walk the rendered facets
//! table-driven and assert each one moves the hash.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use resonance_audio::types::PluginInstanceId;

use crate::state::{BusState, ExternalInstrumentStatus, PluginSlotState, TrackState};

/// Hash everything a slot line draws (`strip_parts::slot_line`): the
/// name, the state dot (missing / bypassed / active) and whether it is
/// the focused slot. Hashing `focused` per slot means focusing a slot
/// moves only the hash of the strip that owns it (and the one that owned
/// the previous focus), never every strip's.
fn hash_slot(h: &mut DefaultHasher, slot: &PluginSlotState, focused: Option<PluginInstanceId>) {
    slot.instance_id.hash(h);
    slot.plugin_name.hash(h);
    // The dot's BAD treatment keys off missing-vs-available only; the
    // reason string renders in the inspector, not on the strip.
    slot.availability.reason().is_some().hash(h);
    slot.bypassed.hash(h);
    (focused == Some(slot.instance_id)).hash(h);
}

fn hash_chain(h: &mut DefaultHasher, plugins: &[PluginSlotState], focused: Option<PluginInstanceId>) {
    plugins.len().hash(h);
    for p in plugins {
        hash_slot(h, p, focused);
    }
}

/// Fingerprint for a top-level track strip's lazy body — everything
/// `channel_strip_body` renders. Levels are absent on purpose: the
/// fader/meter block is built outside the lazy region every frame.
pub(super) fn track_strip_fingerprint(r: &crate::Resonance, track: &TrackState) -> u64 {
    let mut h = DefaultHasher::new();
    // Head: colour band, glyph, name (or the rename field), caret,
    // indent.
    track.id.hash(&mut h);
    track.name.hash(&mut h);
    track.color.hash(&mut h);
    track.track_type.hash(&mut h);
    track.instrument_icon.hash(&mut h);
    r.ui.mixer
        .renaming
        .as_ref()
        .filter(|(id, _)| *id == track.id)
        .map(|(_, buffer)| buffer)
        .hash(&mut h);
    let has_sub_tracks = r
        .registry
        .tracks
        .iter()
        .any(|t| matches!(t.sub_track, Some(link) if link.parent_track_id == track.id));
    has_sub_tracks.hash(&mut h);
    r.ui.mixer
        .expanded_sub_track_parents
        .contains(&track.id)
        .hash(&mut h);
    r.track_groups.indent_depth(track.id).hash(&mut h);
    // Button row (M / S / ● / 🎧) and the FX header switch, whose state
    // also dims every slot line.
    track.muted.hash(&mut h);
    track.soloed.hash(&mut h);
    track.record_armed.hash(&mut h);
    track.monitor_enabled.hash(&mut h);
    track.fx_bypassed.hash(&mut h);
    // Slot lines, and where the instrument line sits — a function of
    // the plugin catalog (which plugins are instruments), not only of
    // the track, so it is hashed as resolved.
    hash_chain(&mut h, &track.plugins, r.ui.mixer.focused_slot);
    super::strip_parts::InstrumentSlot::of_track(r, track).hash(&mut h);
    // External-instrument head pill, offline flag and summary chips.
    let ext = r.devices.external_instruments.get(&track.id);
    ext.is_some().hash(&mut h);
    if let Some(ext) = ext {
        ext.midi_out_offline.hash(&mut h);
        ext.return_input_offline.hash(&mut h);
        ext.bank.hash(&mut h);
        ext.program.hash(&mut h);
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
        // The Return chip formats its port through `PortChoice`, which
        // reads the mono flag.
        track.mono.hash(&mut h);
    }
    // Pan knob + its value, and the live automated-pan tint. The tint
    // only moves while a Read-enabled pan lane plays back, in which case
    // the rebuild is exactly the repaint we want.
    track.pan.to_bits().hash(&mut h);
    super::automation::live_value(
        &r.automation,
        resonance_common::AutomationTarget::TrackPan(track.id),
    )
    .map(f32::to_bits)
    .hash(&mut h);
    h.finish()
}

/// Fingerprint for a sub-track strip's lazy body (colour band, name,
/// M / S, FX switch, pan). The rail/border selection tint and the
/// fader/meter block are built outside the lazy region.
pub(super) fn sub_strip_fingerprint(r: &crate::Resonance, track: &TrackState) -> u64 {
    let mut h = DefaultHasher::new();
    track.id.hash(&mut h);
    track.name.hash(&mut h);
    // The colour band: the parent's colour.
    super::track_strip::sub_track_color(r, track).hash(&mut h);
    track.muted.hash(&mut h);
    track.soloed.hash(&mut h);
    track.fx_bypassed.hash(&mut h);
    track.pan.to_bits().hash(&mut h);
    h.finish()
}

/// Fingerprint for a bus strip's lazy body — everything
/// `bus_strip_body` renders (name, mute, FX switch, slot lines, pan).
/// Selection tints the outer border only, so it stays out of the key.
pub(super) fn bus_strip_fingerprint(r: &crate::Resonance, bus: &BusState) -> u64 {
    let mut h = DefaultHasher::new();
    bus.id.hash(&mut h);
    bus.name.hash(&mut h);
    bus.muted.hash(&mut h);
    bus.fx_bypassed.hash(&mut h);
    hash_chain(&mut h, &bus.plugins, r.ui.mixer.focused_slot);
    bus.pan.to_bits().hash(&mut h);
    super::automation::live_value(
        &r.automation,
        resonance_common::AutomationTarget::BusPan(bus.id),
    )
    .map(f32::to_bits)
    .hash(&mut h);
    h.finish()
}

/// Fingerprint for the master strip's lazy body (FX switch, slot lines).
/// The fader/meter and the selection border stay outside the lazy
/// region.
pub(super) fn master_strip_fingerprint(r: &crate::Resonance) -> u64 {
    let mut h = DefaultHasher::new();
    r.master.fx_bypassed.hash(&mut h);
    hash_chain(&mut h, &r.master.plugins, r.ui.mixer.focused_slot);
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
            sub_strip_fingerprint(self, track)
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

    /// Test-only: what a strip's slot line for `instance_id` shows, as
    /// `(label, dot, dimmed, focused, instrument)` — `dot` is `"active"`,
    /// `"bypassed"` or `"missing"`. The colours that carry the bypass and
    /// focus feedback are not observable through `iced_test` (it reads a
    /// text's content, never its colour), so this reads the view's own
    /// decision. `None` when no chain holds the slot.
    #[doc(hidden)]
    pub fn test_strip_slot_line(
        &self,
        instance_id: resonance_audio::types::PluginInstanceId,
    ) -> Option<(String, &'static str, bool, bool, bool)> {
        let focused = self.ui.mixer.focused_slot == Some(instance_id);
        let mut found = None;
        for track in &self.registry.tracks {
            if let Some(i) = track.plugins.iter().position(|p| p.instance_id == instance_id) {
                let instrument = super::strip_parts::InstrumentSlot::of_track(self, track)
                    == super::strip_parts::InstrumentSlot::At(i);
                found = Some((&track.plugins[i], instrument, track.fx_bypassed));
            }
        }
        for bus in &self.registry.busses {
            if let Some(p) = bus.plugins.iter().find(|p| p.instance_id == instance_id) {
                found = Some((p, false, bus.fx_bypassed));
            }
        }
        if let Some(p) = self.master.plugins.iter().find(|p| p.instance_id == instance_id) {
            found = Some((p, false, self.master.fx_bypassed));
        }
        let (plugin, instrument, chain_bypassed) = found?;
        let look = super::strip_parts::slot_line_look(plugin, instrument, chain_bypassed, focused);
        let dot = match look.dot {
            super::strip_parts::SlotDot::Active => "active",
            super::strip_parts::SlotDot::Bypassed => "bypassed",
            super::strip_parts::SlotDot::Missing => "missing",
        };
        Some((look.label, dot, look.dimmed, look.focused, look.instrument))
    }

    /// Test-only: the open strip rename, if any (track and edit buffer).
    #[doc(hidden)]
    pub fn test_strip_renaming(&self) -> Option<(resonance_audio::types::TrackId, String)> {
        self.ui.mixer.renaming.clone()
    }

    /// Test-only: the master strip card's border (colour, width) as the
    /// mixer draws it now — the selected highlight is in a style closure
    /// `iced_test` cannot read.
    #[doc(hidden)]
    pub fn test_master_strip_border(&self) -> (iced::Color, f32) {
        super::master_strip::master_strip_border(self.ui.mixer.selected_master)
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
