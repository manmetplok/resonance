//! Mixer, plugin, freeze and external-instrument test hooks: aux
//! sends, MIDI map, plugin params, freeze lifecycle, and the device
//! definition registry.

use crate::message::Message;
use crate::state;
use crate::Resonance;

/// Which plugin chain a [`Resonance::test_chain_move_affordances`] query
/// is about. Mirrors the view's private `PluginOwner` so a test can name
/// a chain without the whole mixer view module going public.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub enum TestChain {
    Track(resonance_audio::types::TrackId),
    Bus(resonance_audio::types::BusId),
    Master,
}

/// What the mixer inspector's SENDS block renders for one aux send, and
/// the messages its controls raise (ba todo #1310).
///
/// Every field is read straight off the view builders in
/// `view::mixer::inspector::sends`, so a test that asserts on this is
/// asserting on the rendered panel, and one that feeds
/// [`Self::reroute_to`] / [`Self::set_level`] / [`Self::toggle_tap`] /
/// [`Self::toggle_enabled`] / [`Self::remove`] into `update` is pressing
/// the real affordances rather than a second copy of the wiring.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct SendSlotAffordances {
    pub send_id: u64,
    /// The bus the destination picker shows as selected.
    pub dest_bus: u64,
    /// That entry's label, exactly as the picker draws it.
    pub dest_label: String,
    /// Every `(bus_id, label)` the destination picker offers, in order.
    pub dest_options: Vec<(u64, String)>,
    /// The dB readout beside the level slider (e.g. `"-6.0 dB"`).
    pub level_readout: String,
    /// The tap toggle's label: `"PRE"` or `"POST"`.
    pub tap_label: String,
    pub pre_fader: bool,
    /// Whether the ON toggle reads as lit.
    pub enabled: bool,
}

impl SendSlotAffordances {
    /// Picking `bus_id` out of the destination picker.
    pub fn reroute_to(&self, bus_id: u64) -> Message {
        crate::view::mixer::inspector::sends::dest_message(self.send_id, bus_id)
    }

    /// Dragging the level slider to `level_db`.
    pub fn set_level(&self, level_db: f32) -> Message {
        crate::view::mixer::inspector::sends::level_message(self.send_id, level_db)
    }

    /// Clicking the PRE/POST toggle.
    pub fn toggle_tap(&self) -> Message {
        crate::view::mixer::inspector::sends::tap_message(self.send_id)
    }

    /// Clicking the ON toggle.
    pub fn toggle_enabled(&self) -> Message {
        crate::view::mixer::inspector::sends::enable_message(self.send_id)
    }

    /// Clicking the trash affordance.
    pub fn remove(&self) -> Message {
        crate::view::mixer::inspector::sends::remove_message(self.send_id)
    }
}

impl Resonance {
    /// Test-only: read the mirrored aux-send graph. Driven from
    /// `tests/aux_send_mirror.rs` to assert events reconstruct state.
    #[doc(hidden)]
    pub fn test_aux_sends(&self) -> &[resonance_audio::types::AuxSend] {
        &self.aux.sends
    }

    /// Test-only: seed the aux-send mirror directly so a handler test can
    /// exercise the "edit an existing send" upsert path without first
    /// driving the create round trip. Mirrors what an `AuxSendChanged`
    /// echo would produce.
    #[doc(hidden)]
    pub fn test_seed_aux_send(&mut self, send: resonance_audio::types::AuxSend) {
        self.aux.upsert(send);
    }

    /// Test-only: read the mirrored sidechain (key) routes, one per keyed
    /// plugin instance. Drives `tests/sidechain_persistence.rs`, which
    /// asserts routes survive a real save + reload (ba todo #1311).
    #[doc(hidden)]
    pub fn test_sidechain_routes(&self) -> &[resonance_audio::types::SidechainRoute] {
        &self.sidechain.routes
    }

    /// Test-only: every slot's per-plugin bypass flag, keyed by instance
    /// id, across all three chains (ba todo #1305).
    ///
    /// One map rather than three accessors because the flag's whole point
    /// is that it means the same thing wherever the slot lives — a test
    /// that had to ask a different question per chain could not state
    /// that.
    #[doc(hidden)]
    pub fn test_plugin_bypass_flags(&self) -> std::collections::BTreeMap<u64, bool> {
        let mut out = std::collections::BTreeMap::new();
        for t in &self.registry.tracks {
            for p in &t.plugins {
                out.insert(p.instance_id, p.bypassed);
            }
        }
        for b in &self.registry.busses {
            for p in &b.plugins {
                out.insert(p.instance_id, p.bypassed);
            }
        }
        for p in &self.master_plugins {
            out.insert(p.instance_id, p.bypassed);
        }
        out
    }

    /// Test-only: read the most recent aux-send rejection forwarded to
    /// the UI (`None` once a later send succeeds).
    #[doc(hidden)]
    pub fn test_aux_last_rejection(&self) -> Option<&state::AuxSendRejection> {
        self.aux.last_rejection.as_ref()
    }

    /// Test-only: the send slots the mixer inspector's ROUTING group
    /// renders for `track_id`, in the order it draws them, each carrying
    /// the messages its controls raise (ba todo #1310).
    ///
    /// Empty when the track has no sends — which is also what a track
    /// that does not exist reports, since neither draws a slot.
    #[doc(hidden)]
    pub fn test_send_affordances(
        &self,
        track_id: resonance_audio::types::TrackId,
    ) -> Vec<SendSlotAffordances> {
        use crate::view::mixer::inspector::sends;
        sends::sends_for_track(self, track_id)
            .map(|send| {
                let (options, selected) = sends::dest_options(self, send);
                SendSlotAffordances {
                    send_id: send.id,
                    dest_bus: send.dest,
                    dest_label: selected.map(|c| c.label).unwrap_or_default(),
                    dest_options: options
                        .into_iter()
                        .map(|c| (c.bus_id, c.label))
                        .collect(),
                    level_readout: sends::level_readout(send),
                    tap_label: sends::tap_label(send).to_string(),
                    pre_fader: send.pre_fader,
                    enabled: send.enabled,
                }
            })
            .collect()
    }

    /// Test-only: the "+ Add send" picker's options for `track_id` as
    /// `(label, message)` pairs — the label the dropdown shows and the
    /// message picking it raises. Always ends with the "New FX return…"
    /// entry, so the picker is never dead.
    #[doc(hidden)]
    pub fn test_add_send_options(
        &self,
        track_id: resonance_audio::types::TrackId,
    ) -> Vec<(String, Message)> {
        use crate::view::mixer::inspector::sends;
        let source = resonance_audio::types::SendSource::Track(track_id);
        sends::add_options(self, track_id)
            .into_iter()
            .map(|choice| (choice.to_string(), sends::add_message(source, &choice)))
            .collect()
    }

    /// Test-only: drive the freeze-cache rehydrate path a disk load runs
    /// (ba todo #577) without constructing a whole `LoadedProject` or
    /// pumping the ClearAll → AllCleared round-trip. `freezes` is each
    /// track's persisted [`crate::state::FreezeStatus`] source — the
    /// per-track [`resonance_common::TrackFreezeState`] paired with its id.
    #[doc(hidden)]
    pub fn test_rehydrate_frozen_tracks(
        &mut self,
        project_dir: &std::path::Path,
        freezes: &[(
            resonance_audio::types::TrackId,
            resonance_common::TrackFreezeState,
        )],
    ) {
        self.rehydrate_frozen_tracks(project_dir, freezes);
    }

    /// Test-only: read the GUI-side MIDI control-surface mapping, so the
    /// engine-event mirroring tests can assert bindings / learn state.
    #[doc(hidden)]
    pub fn test_midi_map(&self) -> &state::MidiMapState {
        &self.midi_map
    }

    /// Test-only: arm MIDI Learn for `target` (the UI-side step that
    /// normally precedes a `MidiLearnCaptured` event), so a test can then
    /// verify the capture handler clears learn mode.
    #[doc(hidden)]
    pub fn test_arm_midi_learn(&mut self, target: resonance_common::MidiTarget) {
        self.midi_map.learn_target = Some(target);
    }

    /// Test-only: push a plugin slot onto a track's chain (bypassing the
    /// engine round-trip) and index it, so the freeze fingerprint /
    /// plugin-param gating tests have a real chain to operate on.
    #[doc(hidden)]
    pub fn test_push_track_plugin(
        &mut self,
        track_id: resonance_audio::types::TrackId,
        plugin: state::PluginSlotState,
    ) {
        let instance_id = plugin.instance_id;
        if let Some(track) = self.registry.tracks.iter_mut().find(|t| t.id == track_id) {
            track.plugins.push(plugin);
            self.insert_plugin_index(instance_id, state::PluginLocator::Track(track_id));
        }
    }

    /// Test-only: the instance ids on a track's chain, in slot order, so
    /// a test can address a plugin the way a GUI would — by where it
    /// sits — rather than through a control-API wire parameter.
    #[doc(hidden)]
    pub fn test_track_plugin_instance_ids(
        &self,
        track_id: resonance_audio::types::TrackId,
    ) -> Vec<resonance_audio::types::PluginInstanceId> {
        self.registry
            .tracks
            .iter()
            .find(|t| t.id == track_id)
            .map(|t| t.plugins.iter().map(|p| p.instance_id).collect())
            .unwrap_or_default()
    }

    /// Test-only: the chain-reorder affordances the mixer draws for one
    /// chain, slot by slot — the exact `(▲, ▼)` messages the inspector
    /// row and the strip slot attach to their carets, with `None` where
    /// the caret renders disabled (ba todo #1302).
    ///
    /// Returned as messages rather than booleans so a test can both
    /// assert what the GUI offers AND feed it straight back through
    /// `update`, which is the only way to prove the button a human
    /// presses lands the same reorder the control API does.
    #[doc(hidden)]
    pub fn test_chain_move_affordances(
        &self,
        chain: TestChain,
    ) -> Vec<(Option<crate::message::Message>, Option<crate::message::Message>)> {
        use crate::view::mixer::picks::PluginOwner;
        let (owner, slots): (PluginOwner, Vec<_>) = match chain {
            TestChain::Track(track_id) => {
                let Some(t) = self.registry.tracks.iter().find(|t| t.id == track_id) else {
                    return Vec::new();
                };
                (
                    PluginOwner::Track(track_id),
                    t.plugins.iter().map(|p| p.instance_id).collect(),
                )
            }
            TestChain::Bus(bus_id) => {
                let Some(b) = self.registry.busses.iter().find(|b| b.id == bus_id) else {
                    return Vec::new();
                };
                (
                    PluginOwner::Bus(bus_id),
                    b.plugins.iter().map(|p| p.instance_id).collect(),
                )
            }
            TestChain::Master => (
                PluginOwner::Master,
                self.master_plugins.iter().map(|p| p.instance_id).collect(),
            ),
        };
        let len = slots.len();
        slots
            .into_iter()
            .enumerate()
            .map(|(index, instance_id)| {
                let m = crate::view::mixer::reorder::chain_moves(
                    self,
                    owner,
                    instance_id,
                    index,
                    len,
                );
                (m.up, m.down)
            })
            .collect()
    }

    /// Test-only: the channel-strip slot's floating-editor toggle for
    /// `instance_id`, as the mixer would draw it — the message the glyph
    /// carries and the colour it is tinted (ba todo #1306).
    ///
    /// `None` means the strip draws no editor control for that slot.
    ///
    /// The click routing is covered end-to-end by pressing the real
    /// button in `tests/mixer_generic_param_panel.rs`; this hook exists
    /// for the tint, which the widget tree does not expose — `iced_test`
    /// can read a text candidate's content but never its colour.
    #[doc(hidden)]
    pub fn test_strip_editor_toggle(
        &self,
        instance_id: resonance_audio::types::PluginInstanceId,
    ) -> Option<(crate::message::Message, iced::Color)> {
        let plugin = self
            .registry
            .tracks
            .iter()
            .flat_map(|t| t.plugins.iter())
            .chain(self.registry.busses.iter().flat_map(|b| b.plugins.iter()))
            .chain(self.master_plugins.iter())
            .find(|p| p.instance_id == instance_id)?;
        crate::view::mixer::editor_toggle_spec(plugin)
    }

    /// Test-only: declare that a seeded plugin has a GUI, as a real
    /// scan result would (ba todo #1306).
    ///
    /// Every plugin the test seeds defaults to `has_gui: false`, which
    /// is the configuration NONE of the eleven bundled plugins actually
    /// ship in — so without this the strip's editor toggle never
    /// reaches a golden and the 140 px row is only ever pixel-checked
    /// one control short.
    #[doc(hidden)]
    pub fn test_set_plugin_has_gui(
        &mut self,
        instance_id: resonance_audio::types::PluginInstanceId,
        has_gui: bool,
    ) {
        self.with_plugin_mut(instance_id, |p| p.has_gui = has_gui);
    }

    /// Test-only: set a plugin param's current value directly (no engine
    /// round-trip), so a fingerprint test can mutate an input and recompute.
    #[doc(hidden)]
    pub fn test_set_plugin_param(
        &mut self,
        instance_id: resonance_audio::types::PluginInstanceId,
        param_id: u32,
        value: f64,
    ) {
        self.with_plugin_mut(instance_id, |p| {
            if let Some(param) = p.params.iter_mut().find(|pp| pp.id == param_id) {
                param.current_value = value;
            }
        });
    }

    /// Test-only: read a plugin param's current value (no engine round-trip).
    #[doc(hidden)]
    pub fn test_plugin_param(
        &mut self,
        instance_id: resonance_audio::types::PluginInstanceId,
        param_id: u32,
    ) -> Option<f64> {
        self.with_plugin_mut(instance_id, |p| {
            p.params
                .iter()
                .find(|pp| pp.id == param_id)
                .map(|pp| pp.current_value)
        })
        .flatten()
    }

    /// Test-only: recompute the resonance-common freeze input fingerprint
    /// for a track (ba todo #576).
    #[doc(hidden)]
    pub fn test_freeze_fingerprint(
        &self,
        track_id: resonance_audio::types::TrackId,
    ) -> Option<u64> {
        self.compute_track_freeze_fingerprint(track_id)
    }

    /// Test-only: recompute the fingerprint and downgrade a still-`Frozen`
    /// track to `Stale` if its inputs drifted. Returns whether it
    /// transitioned (ba todo #576).
    #[doc(hidden)]
    pub fn test_revalidate_frozen_track(
        &mut self,
        track_id: resonance_audio::types::TrackId,
    ) -> bool {
        self.revalidate_frozen_track(track_id)
    }

    /// Test-only: read a track's freeze status (defaults to idle).
    #[doc(hidden)]
    pub fn test_freeze_status(
        &self,
        track_id: resonance_audio::types::TrackId,
    ) -> crate::state::FreezeStatus {
        self.freeze.status(track_id)
    }

    /// Test-only: force a track's freeze status, mirroring what the engine
    /// freeze-event mirror (ba todo #575) would set on completion.
    #[doc(hidden)]
    pub fn test_set_freeze_status(
        &mut self,
        track_id: resonance_audio::types::TrackId,
        status: crate::state::FreezeStatus,
    ) {
        self.freeze.set(track_id, status);
    }

    /// Test-only: read the active freeze batch queue, if any.
    #[doc(hidden)]
    pub fn test_freeze_queue(&self) -> Option<&crate::state::FreezeQueue> {
        self.freeze.queue.as_ref()
    }

    /// Test-only: advance the freeze batch to the next track, as the
    /// engine completion mirror (ba todo #575) will once it lands. Returns
    /// `true` when a next freeze was started.
    #[doc(hidden)]
    pub fn test_advance_freeze_queue(&mut self) -> bool {
        crate::update::freeze::advance_freeze_queue(self)
    }

    /// Test-only: drive an undo-restore reconciliation directly with a
    /// target freeze map, exercising `apply_freeze_restore` without the
    /// full snapshot/replay pipeline.
    #[doc(hidden)]
    pub fn test_apply_freeze_restore(
        &mut self,
        target: std::collections::HashMap<
            resonance_audio::types::TrackId,
            crate::state::FreezeStatus,
        >,
    ) {
        self.apply_freeze_restore(target);
    }

    /// Test-only: drive the clip fade/gain undo re-apply directly with a
    /// target map, exercising `apply_clip_fade_gain_restore` (the shared
    /// re-sync used by both restore paths) without the full
    /// snapshot/replay pipeline. Mirrors `test_apply_freeze_restore`.
    #[doc(hidden)]
    pub fn test_apply_clip_fade_gain_restore(
        &mut self,
        map: &std::collections::HashMap<resonance_audio::types::ClipId, crate::undo::ClipFadeGain>,
    ) {
        self.apply_clip_fade_gain_restore(map);
    }

    /// Test-only: read the external-instrument state mirror for a track, if
    /// it's in external-instrument mode. Used by the external-instrument
    /// reducer tests to assert config + offline-flag mutations.
    #[doc(hidden)]
    pub fn test_external_instrument(
        &self,
        track_id: resonance_audio::types::TrackId,
    ) -> Option<state::ExternalInstrumentState> {
        self.external_instruments.get(&track_id).cloned()
    }

    /// Test-only: layer extra device definitions from `dir` into the device
    /// registry, exactly as the user `device_definitions` folder is scanned at
    /// startup. Lets a persistence test register a user-authored preset without
    /// a real user data dir. (epic #40, doc #201 §5.)
    #[doc(hidden)]
    pub fn test_scan_device_dir(&mut self, dir: &std::path::Path) {
        self.device_registry.scan_dir(dir);
    }

    /// Test-only: derive the lifecycle [`state::ExternalInstrumentStatus`] for
    /// a track from its external-instrument state + owning `TrackState`.
    /// `None` when the track isn't external or doesn't exist.
    #[doc(hidden)]
    pub fn test_external_instrument_status(
        &self,
        track_id: resonance_audio::types::TrackId,
    ) -> Option<state::ExternalInstrumentStatus> {
        let ext = self.external_instruments.get(&track_id)?;
        let track = self.registry.tracks.iter().find(|t| t.id == track_id)?;
        Some(ext.status(track))
    }

    /// Test-only: compute the Mixer Inspector's lazy-region fingerprint for a
    /// track, exactly as `view()` does (same collapse state, same track). The
    /// inspector's onboarding card and device-offline alert render *inside*
    /// the `lazy(fp, …)` region, so this hash MUST change whenever any state
    /// those bodies depend on changes — otherwise the retained UI reuses a
    /// stale tree across an offline/recovery transition and the alert never
    /// appears (or never clears). Regression guard for ba todo #459.
    /// `None` when the track doesn't exist.
    #[doc(hidden)]
    pub fn test_inspector_fingerprint(
        &self,
        track_id: resonance_audio::types::TrackId,
    ) -> Option<u64> {
        let track = self.registry.tracks.iter().find(|t| t.id == track_id)?;
        let routing_collapsed = self
            .mixer
            .collapsed_inspector_groups
            .contains(&state::MixerInspectorGroup::Routing);
        let chain_collapsed = self
            .mixer
            .collapsed_inspector_groups
            .contains(&state::MixerInspectorGroup::Chain);
        Some(crate::view::mixer::inspector::inspector_fingerprint(
            self,
            track,
            routing_collapsed,
            chain_collapsed,
        ))
    }

    /// Test-only: drive the GUI external-instrument map (and engine) back to
    /// `extras`, the same restore path both undo replays use. Pairs with
    /// [`Self::test_snapshot_undo_extras`] to exercise a config round-trip.
    #[doc(hidden)]
    pub fn test_restore_external_instruments(&mut self, extras: &crate::undo::UndoExtras) {
        self.restore_external_instruments(extras);
    }

    /// Test-only: the ordered automation-parameter-picker labels the mixer
    /// strip for `track_id` would show (epic #40, doc #201 §5). Resolves the
    /// track's selected external-instrument device preset to its definition's
    /// named params exactly as the strip view does, so a test can assert the
    /// device params appear (grouped) only when a preset is selected and are
    /// hidden otherwise. A closed `pick_list` renders only its placeholder,
    /// so this mirrors the option list the dropdown would present.
    #[doc(hidden)]
    pub fn test_automation_picker_labels(
        &self,
        track_id: resonance_audio::types::TrackId,
    ) -> Vec<String> {
        let device_params: &[resonance_common::DeviceParam] = self
            .external_instruments
            .get(&track_id)
            .and_then(|ext| ext.device_id.as_deref())
            .and_then(|id| self.device_registry.get(id))
            .map(|def| def.params.as_slice())
            .unwrap_or(&[]);
        let plugins = self
            .registry
            .tracks
            .iter()
            .find(|t| t.id == track_id)
            .map(|t| t.plugins.as_slice())
            .unwrap_or(&[]);
        crate::view::mixer::automation::track_choice_labels(track_id, plugins, device_params)
    }

    /// Test-only: return the `id`s of every definition currently in the
    /// device-definition registry (bundled + user), in registry order. Used
    /// to assert that a `RescanDefinitions` dispatch picks up new files.
    #[doc(hidden)]
    pub fn test_device_registry_ids(&self) -> Vec<String> {
        self.device_registry
            .list()
            .iter()
            .map(|d| d.id.clone())
            .collect()
    }

    /// Test-only: scan a user-definitions directory directly into the device
    /// registry and rebuild the cached pick-list options, bypassing
    /// `user_definitions_dir()` (which depends on `$XDG_DATA_HOME`). Used by
    /// tests that need to verify re-scan behaviour without touching the real
    /// user data directory.
    #[doc(hidden)]
    pub fn test_rescan_definitions_from(&mut self, user_dir: &std::path::Path) {
        let mut registry = resonance_common::DeviceDefinitionRegistry::default();
        registry.scan_bundled();
        registry.scan_dir(user_dir);
        self.view_caches.rebuild_device_choices(&registry.list());
        self.device_registry = registry;
    }

    /// Test-only: return the ids offered by the device-preset pick-list cache
    /// (the `device_choices` options), excluding the `None` "(no device)"
    /// entry. Useful to assert that the cache is in sync with the registry
    /// after a re-scan.
    #[doc(hidden)]
    pub fn test_device_choice_ids(&self) -> Vec<String> {
        self.view_caches
            .device_choices
            .iter()
            .filter_map(|c| c.id.clone())
            .collect()
    }
}
