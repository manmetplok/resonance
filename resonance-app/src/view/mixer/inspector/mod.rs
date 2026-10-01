//! Mixer Inspector panel — the right-side pane that edits the selected
//! channel: a track, a bus, or the master (mixer-cleanup.md §3).
//!
//! A track shows, in order: CHAIN → SENDS → ROUTING → AUTOMATION →
//! TRACK. A bus shows CHAIN → ROUTING → AUTOMATION → BUS, the master
//! CHAIN → AUTOMATION → MASTER. Each group folds independently
//! (`MixerInspectorGroup`); TRACK, BUS and MASTER each have their own
//! group key, so folding one owner's group leaves the others open.
//!
//! The implementation is split into focused submodules to keep each
//! section's reason-to-change isolated:
//! - `widgets`           — shared low-level helpers (field, toggle, header)
//! - `onboarding`        — status badge + onboarding card (doc #169)
//! - `io`                — audio/MIDI input/output picker blocks
//! - `chain`             — CHAIN group (plugin rows + add picker)
//! - `sends`             — SENDS group (doc #172)
//! - `routing`           — ROUTING group
//! - `automation`        — AUTOMATION group (lane rows + add-lane picker)
//! - `track_group`       — TRACK group (mono, bounce, external hardware)
//! - `external_instrument` — the external-hardware section of TRACK (todo #454)
//! - `bus`               — the whole pane for a selected BUS strip
//! - `master`            — the whole pane for the selected MASTER strip
//!
//! A bus is not a `TrackState`, so it gets its own module rather than a
//! set of `if is_bus` branches through the track path; the master
//! likewise.
//!
//! Nothing the inspector draws is live per-tick state (the SIGNAL tiles
//! that read the meters were dropped — the strip already shows them), so
//! every body below its title sits in one `lazy` region keyed on a
//! fingerprint of everything it reads (ui-work.md §11).

mod automation;
mod bus;
pub(crate) mod chain;
mod external_instrument;
mod io;
mod master;
mod onboarding;
mod routing;
pub(crate) mod sends;
mod track_group;
mod widgets;

pub(crate) use bus::fingerprint as bus_fingerprint;
pub(crate) use master::fingerprint as master_fingerprint;

use iced::widget::{column, container, row, text, Space};
use iced::{alignment, Element, Length};
use resonance_audio::types::TrackType;
use resonance_common::DeviceParam;

use crate::message::*;
use crate::state::{MixerInspectorGroup, TrackState};
use crate::theme;
use crate::view::mixer::automation::AutoChan;
// The cached-list-plus-stale-override combinator the io /
// external-instrument pickers use lives in `ui_caches` now, shared with
// the Settings overlay's MIDI-clock pickers.
use crate::view::ui_caches::midi_choices_with_override;

pub(super) fn view<'a>(r: &'a crate::Resonance) -> Element<'a, Message> {
    // A selected bus takes the pane, then the master, then a track. The
    // three selections are mutually exclusive (see
    // `MixerUiState::selected_bus` / `selected_master`), so this is a
    // precedence rule only for the window where a bus was deleted while
    // selected — in which case the lookup misses and the next path takes
    // over.
    let selected_bus = r
        .ui
        .mixer
        .selected_bus
        .and_then(|id| r.registry.busses.iter().find(|b| b.id == id));
    if let Some(b) = selected_bus {
        return chrome(bus::view(r, b));
    }
    if r.ui.mixer.selected_master {
        return chrome(master::view(r));
    }

    let selected_id = r.ui.interaction.selected_track;
    let selected = selected_id.and_then(|id| r.registry.tracks.iter().find(|t| t.id == id));

    let body: Element<'a, Message> = match selected {
        Some(track) => track_view(r, track),
        None => render_empty(),
    };

    chrome(body)
}

fn track_view<'a>(r: &'a crate::Resonance, track: &'a TrackState) -> Element<'a, Message> {
    // Derived lifecycle status drives the header badge on an
    // external-instrument track (todo #459). Computed from the config +
    // live device flags so it can never drift out of sync with the state
    // it renders.
    let ext_status = r
        .devices
        .external_instruments
        .get(&track.id)
        .map(|ext| ext.status(track));

    // Title row: the track name, its type tag, and the status badge on
    // external-instrument tracks (Unconfigured / Configuring / Live /
    // Offline), mirroring the prototype's inspector badge.
    let mut title_row = row![
        text(track.name.clone())
            .size(17)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::TEXT_1),
        Space::new().width(8),
    ]
    .spacing(0)
    .align_y(alignment::Vertical::Center);
    // The colour swatch (mixer-cleanup.md §3.1, §6). A sub-track has no
    // colour of its own — it follows its parent — so it gets none.
    let has_color = track.sub_track.is_none();
    if has_color {
        title_row = title_row
            .push(color_swatch(track))
            .push(Space::new().width(8));
    }
    title_row = title_row.push(widgets::type_tag(type_label(track)));
    if let Some(status) = ext_status {
        title_row = title_row
            .push(Space::new().width(6))
            .push(onboarding::status_badge(status));
    }

    let fp = inspector_fingerprint(r, track);
    let lazy_groups = iced::widget::lazy(fp, move |_: &u64| -> Element<'static, Message> {
        let device_params = device_params_for(r, track);
        column![
            chain::chain_group(r, track, collapsed(r, MixerInspectorGroup::Chain)),
            Space::new().height(18),
            sends::sends_group(r, track, collapsed(r, MixerInspectorGroup::Sends)),
            Space::new().height(18),
            routing::routing_group(r, track, collapsed(r, MixerInspectorGroup::Routing)),
            Space::new().height(18),
            automation::automation_group(
                r,
                AutoChan::Track(track.id),
                &track.plugins,
                device_params,
                collapsed(r, MixerInspectorGroup::Automation),
            ),
            Space::new().height(18),
            track_group::track_group(r, track, collapsed(r, MixerInspectorGroup::Track)),
        ]
        .spacing(0)
        .width(Length::Fill)
        .into()
    });

    let mut stack = column![widgets::header(title_row)].spacing(0);
    if has_color && r.ui.mixer.color_palette == Some(track.id) {
        stack = stack
            .push(Space::new().height(8))
            .push(color_palette(track));
    }

    iced::widget::scrollable(
        stack
            .push(Space::new().height(18))
            .push(lazy_groups)
            .width(Length::Fill),
    )
    .height(Length::Fill)
    // Embedded, not floating: the scrollbar takes its own column beside
    // the groups instead of drawing over their right edge (pickers'
    // carets, the sends' dB readouts and trash buttons).
    .spacing(6)
    .into()
}

/// Widget id of the header's colour swatch (tests click it by id: it
/// draws no text).
pub(crate) fn color_swatch_id() -> iced::widget::Id {
    iced::widget::Id::new("inspector-color-swatch")
}

/// Widget id of palette entry `index` in the open colour palette.
pub(crate) fn palette_swatch_id(index: usize) -> iced::widget::Id {
    iced::widget::Id::from(format!("inspector-palette-{index}"))
}

/// A square in `color`, ringed when `ring` (the current colour, or the
/// open palette's swatch).
fn swatch_square(
    color: [u8; 3],
    size: f32,
    ring: bool,
) -> iced::widget::Container<'static, Message> {
    container(Space::new().width(size).height(size)).style(move |_theme| container::Style {
        background: Some(iced::Background::Color(theme::track_color(color))),
        border: iced::Border {
            color: if ring { theme::TEXT_1 } else { theme::LINE },
            width: if ring { 2.0 } else { 1.0 },
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    })
}

/// The header swatch: the track's colour; a click opens the palette.
fn color_swatch(track: &TrackState) -> Element<'static, Message> {
    let open = Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::ToggleColorPalette(
        track.id,
    )));
    container(
        iced::widget::button(swatch_square(track.color, 12.0, false))
            .padding(0)
            .on_press(open)
            .style(|_theme, status| theme::ghost_button_style(status)),
    )
    .id(color_swatch_id())
    .into()
}

/// The open palette: one swatch per `theme::TRACK_PALETTE` hue, the
/// current one ringed. A pick closes it and sends `SetTrackColor` (one
/// undo entry).
fn color_palette(track: &TrackState) -> Element<'static, Message> {
    let mut swatches = row![].spacing(6).align_y(alignment::Vertical::Center);
    for (index, color) in theme::TRACK_PALETTE.iter().copied().enumerate() {
        let pick = Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::Pick(Box::new(
            Message::Track(TrackMessage::SetTrackColor(track.id, color)),
        ))));
        swatches = swatches.push(
            container(
                iced::widget::button(swatch_square(color, 16.0, color == track.color))
                    .padding(0)
                    .on_press(pick)
                    .style(|_theme, status| theme::ghost_button_style(status)),
            )
            .id(palette_swatch_id(index)),
        );
    }
    container(swatches)
        .padding([8, 10])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        })
        .into()
}

/// The header's type tag for a track.
fn type_label(track: &TrackState) -> &'static str {
    match (track.track_type, track.sub_track.is_some()) {
        (_, true) => "Sub",
        (TrackType::Instrument, false) => "Inst",
        (TrackType::Audio, false) => "Audio",
        (TrackType::Vocal, false) => "Vocal",
    }
}

/// Whether the user folded `group` shut.
fn collapsed(r: &crate::Resonance, group: MixerInspectorGroup) -> bool {
    r.ui.mixer.collapsed_inspector_groups.contains(&group)
}

/// Hash the fold state of every group, in a fixed order (the set itself
/// is a `HashSet`, whose iteration order is not a function of its
/// contents).
fn hash_collapse_state<H: std::hash::Hasher>(h: &mut H, r: &crate::Resonance) {
    use std::hash::Hash;
    for group in MixerInspectorGroup::ALL {
        collapsed(r, group).hash(h);
    }
}

/// The named parameters of the track's selected external-instrument
/// device preset (epic #40, doc #201 §5) — the AUTOMATION picker's device
/// options. Empty unless the track is external with a preset selected
/// whose id resolves in the registry; the strip's lane header resolves
/// it the same way.
fn device_params_for<'a>(r: &'a crate::Resonance, track: &TrackState) -> &'a [DeviceParam] {
    r.devices
        .external_instruments
        .get(&track.id)
        .and_then(|ext| ext.device_id.as_deref())
        .and_then(|id| r.devices.registry.get(id))
        .map(|def| def.params.as_slice())
        .unwrap_or(&[])
}

/// The pane itself — fixed width, padding and background. Shared by the
/// track, bus and empty bodies so the three can never drift apart.
fn chrome<'a>(body: Element<'a, Message>) -> Element<'a, Message> {
    container(body)
        .width(Length::Fixed(theme::INSPECTOR_WIDTH))
        .height(Length::Fill)
        .padding(26)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Hash every field the lazy track body reads. The lazy widget compares
/// this across frames — when nothing has changed, the cached widget tree
/// is reused (which is the resize hot path).
pub(crate) fn inspector_fingerprint(r: &crate::Resonance, t: &TrackState) -> u64 {
    use std::hash::{Hash, Hasher};
    use std::rc::Rc;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    hash_collapse_state(&mut h, r);
    t.id.hash(&mut h);
    t.name.hash(&mut h);
    // The header swatch sits outside the lazy body, but the colour is
    // the track's identity everywhere else; hashed so nothing that
    // shows it can go stale (mixer-cleanup.md §6).
    t.color.hash(&mut h);
    t.track_type.hash(&mut h);
    t.sub_track.hash(&mut h);
    t.input_device_name.hash(&mut h);
    t.input_port_index.hash(&mut h);
    t.mono.hash(&mut h);
    t.midi_input_device.hash(&mut h);
    t.midi_input_channel.hash(&mut h);
    t.midi_output_device.hash(&mut h);
    t.midi_output_channel.hash(&mut h);
    t.output.hash(&mut h);
    // TRACK's Bounce: `classify_bounce` reads the type, sub-track flag,
    // MIDI out and chain (all hashed here) plus whether the track owns a
    // MIDI clip.
    r.midi_clips.iter().any(|c| c.track_id == t.id).hash(&mut h);
    // AUTOMATION: lanes, labels, Read state, and the picker's option
    // source (plugin params, device params).
    automation::hash_into(
        &mut h,
        r,
        AutoChan::Track(t.id),
        &t.plugins,
        device_params_for(r, t),
    );
    // External-instrument fields the routing group reads: the monitor /
    // arm toggles, the patch + latency config, and the runtime offline
    // flags. Without these the lazy region wouldn't redraw when a patch
    // is picked or a device goes offline.
    t.monitor_enabled.hash(&mut h);
    t.record_armed.hash(&mut h);
    let is_external = r.devices.external_instruments.contains_key(&t.id);
    is_external.hash(&mut h);
    if let Some(ext) = r.devices.external_instruments.get(&t.id) {
        // Selected device preset drives the device picker + active
        // "<model> preset →" affordance; without it the lazy region wouldn't
        // redraw when a preset is picked or cleared.
        ext.device_id.hash(&mut h);
        ext.bank.hash(&mut h);
        ext.program.hash(&mut h);
        ext.latency_offset_samples.hash(&mut h);
        ext.midi_out_offline.hash(&mut h);
        ext.return_input_offline.hash(&mut h);
        // The latency readout is in ms, derived from the sample rate.
        r.sample_rate.hash(&mut h);
        // The Detect button's enabled state and the failure note under
        // it (review VIEW-08).
        ext.latency_detect_in_progress.hash(&mut h);
        ext.latency_detect_error.hash(&mut h);
        r.transport.playing.hash(&mut h);
        // PLAYBACK SOURCE toggles + the "Playing the recorded take" chip.
        t.playback_source.hash(&mut h);
        r.clips.iter().any(|c| c.track_id == t.id).hash(&mut h);
        // Return-device / device-preset / bank / program pickers.
        Rc::as_ptr(&r.ui.view_caches.input_devices).hash(&mut h);
        Rc::as_ptr(&r.ui.view_caches.device_choices).hash(&mut h);
        Rc::as_ptr(&r.ui.view_caches.bank_choices).hash(&mut h);
        Rc::as_ptr(&r.ui.view_caches.program_choices).hash(&mut h);
        Rc::as_ptr(&r.ui.view_caches.output_channel_choices).hash(&mut h);
    }
    for p in &t.plugins {
        p.instance_id.hash(&mut h);
        p.plugin_name.hash(&mut h);
        // The bypass dot renders — and builds its press message from —
        // this flag; without it the echo never reaches the retained
        // tree and the button can't un-bypass (review VIEW-08).
        p.bypassed.hash(&mut h);
    }
    chain::hash_chain_ui(&mut h, r, &t.plugins);
    // The SENDS block (ba todo #1310) renders every send tapped off this
    // track, so each field a slot draws has to be here — otherwise the
    // retained tree survives an `AuxSendChanged` echo and the slider
    // snaps back to the pre-drag value (or a removed send lingers).
    for s in sends::sends_for_track(r, t.id) {
        s.id.hash(&mut h);
        s.dest.hash(&mut h);
        s.level_db.to_bits().hash(&mut h);
        s.pre_fader.hash(&mut h);
        s.enabled.hash(&mut h);
    }
    // …and the destination pickers are a function of the bus list, which
    // `output_choices` only half-covers: it carries names but not the
    // return-role flag that decides which busses are offered at all.
    for b in &r.registry.busses {
        b.id.hash(&mut h);
        b.name.hash(&mut h);
        b.is_return.hash(&mut h);
    }
    // A rejected route renders an inline note under the picker.
    if let Some(rejection) = r.aux.last_rejection.as_ref() {
        rejection.source.hash(&mut h);
        rejection.dest.hash(&mut h);
        rejection.reason.hash(&mut h);
    }
    // Cache pointers — when these Rcs are replaced, the inspector
    // needs to redraw with the new options.
    Rc::as_ptr(&r.ui.view_caches.midi_input_choices).hash(&mut h);
    Rc::as_ptr(&r.ui.view_caches.midi_output_choices).hash(&mut h);
    Rc::as_ptr(&r.ui.view_caches.output_choices).hash(&mut h);
    Rc::as_ptr(&r.ui.view_caches.fx_plugins).hash(&mut h);
    Rc::as_ptr(&r.ui.view_caches.instrument_plugins).hash(&mut h);
    Rc::as_ptr(&r.presets.fx_favorite_picks).hash(&mut h);
    Rc::as_ptr(&r.presets.instrument_favorite_picks).hash(&mut h);
    // Audio input picker uses `r.devices.input.devices` directly (its options
    // include the per-device channel count, which the cached choice
    // lists above don't carry).
    r.devices.input.devices.len().hash(&mut h);
    for d in &r.devices.input.devices {
        d.name.hash(&mut h);
        d.channels.hash(&mut h);
    }
    // Live MIDI device list too — the audio block isn't a strict
    // function of the cached lists since the stale-override branch
    // peeks at `midi_input_devices` directly.
    r.devices.midi.midi_input_devices.len().hash(&mut h);
    r.devices.midi.midi_output_devices.len().hash(&mut h);
    h.finish()
}

fn render_empty() -> Element<'static, Message> {
    column![
        text("INSPECTOR")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        Space::new().height(8),
        text("Select a track, bus or the master to edit its chain, routing and automation.")
            .size(12)
            .color(theme::TEXT_3),
    ]
    .spacing(0)
    .into()
}
