//! Mixer Inspector panel — the right-side detail pane that shows the
//! currently selected track's signal, routing, and plugin chain. Hosts
//! the functional pickers that used to live on the strip itself: input
//! device + channel (audio) or MIDI in / channel and MIDI out / channel
//! (instruments), output destination (Master / Bus N), and an "+ FX"
//! picker for the chain.
//!
//! The implementation is split into focused submodules to keep each
//! section's reason-to-change isolated:
//! - `widgets`           — shared low-level helpers (field, toggle, tiles)
//! - `onboarding`        — status badge + onboarding card (doc #169)
//! - `io`                — audio/MIDI input/output picker blocks
//! - `routing`           — ROUTING group orchestration
//! - `sends`             — the SENDS block inside ROUTING (doc #172)
//! - `external_instrument` — EXTERNAL INSTRUMENT group (todo #454)
//! - `chain`             — CHAIN group (plugin rows + add picker)
//! - `bus`               — the whole pane for a selected BUS strip
//!
//! A bus is not a `TrackState`, so it gets its own module rather than a
//! set of `if is_bus` branches through the track path: no input device,
//! no MIDI, no output picker (a bus always sums to master), and ROUTING
//! answers what feeds it instead.

mod bus;
mod chain;
mod external_instrument;
mod io;
mod onboarding;
mod routing;
pub(crate) mod sends;
mod widgets;

use iced::widget::{column, container, row, text, Space};
use iced::{alignment, Element, Length};
use resonance_audio::types::TrackOutput;

use crate::message::*;
use crate::state::{ExternalInstrumentStatus, MixerInspectorGroup, TrackState};
use crate::theme;
use crate::util::format_pan;
// The cached-list-plus-stale-override combinator the io /
// external-instrument pickers use lives in `ui_caches` now, shared with
// the Settings overlay's MIDI-clock pickers.
use crate::view::ui_caches::midi_choices_with_override;

pub(super) fn view<'a>(r: &'a crate::Resonance) -> Element<'a, Message> {
    // A selected bus takes the pane. The two selections are mutually
    // exclusive (see `MixerUiState::selected_bus`), so this is a
    // precedence rule only for the window where a bus was deleted while
    // selected — in which case the lookup misses and the track path
    // takes over.
    let selected_bus = r
        .mixer
        .selected_bus
        .and_then(|id| r.registry.busses.iter().find(|b| b.id == id));
    if let Some(b) = selected_bus {
        return chrome(bus::view(r, b));
    }

    let selected_id = r.interaction.selected_track;
    let selected = selected_id.and_then(|id| r.registry.tracks.iter().find(|t| t.id == id));

    let body: Element<'a, Message> = match selected {
        Some(track) => {
            let signal_collapsed = r
                .mixer
                .collapsed_inspector_groups
                .contains(&MixerInspectorGroup::Signal);
            let routing_collapsed = r
                .mixer
                .collapsed_inspector_groups
                .contains(&MixerInspectorGroup::Routing);
            let chain_collapsed = r
                .mixer
                .collapsed_inspector_groups
                .contains(&MixerInspectorGroup::Chain);

            // An external-instrument track is identified purely by its
            // presence in the `external_instruments` map (todo #454) —
            // there's no track-type discriminant. When present, the
            // SIGNAL group reads "Signal · Return" (the metered signal is
            // the hardware return) and ROUTING becomes the External
            // Instrument group.
            let is_external = r.external_instruments.contains_key(&track.id);
            // Derived lifecycle status drives the inspector header badge,
            // the onboarding card, and the device-offline alert (todo
            // #459). Computed from the config + live device flags so it can
            // never drift out of sync with the state it renders.
            let ext_status = r
                .external_instruments
                .get(&track.id)
                .map(|ext| ext.status(track));

            // Title row: the track name, followed by the status badge on
            // external-instrument tracks (Unconfigured / Configuring /
            // Live / Offline), mirroring the prototype's inspector badge.
            let mut title_row = row![text(track.name.clone())
                .size(17)
                .font(theme::UI_FONT_MEDIUM)
                .color(theme::TEXT_1)]
            .spacing(0)
            .align_y(alignment::Vertical::Center);
            if let Some(status) = ext_status {
                title_row = title_row
                    .push(Space::new().width(8))
                    .push(onboarding::status_badge(status));
            }
            let header = column![
                text("INSPECTOR")
                    .size(10)
                    .font(theme::UI_FONT_SEMIBOLD)
                    .color(theme::TEXT_3),
                Space::new().height(2),
                title_row,
            ]
            .spacing(0);

            // SIGNAL stays outside the lazy region: its PEAK tile reads
            // the per-tick track levels, which the fingerprint below
            // deliberately omits (see ui-work.md §11.2 — never key a
            // lazy region without the live data it renders).
            let signal = signal_group(track, signal_collapsed, is_external);

            // ROUTING + CHAIN only change on slow events (device lists,
            // routing edits, chain edits, collapse toggles) — all hashed
            // into the fingerprint, so the cached tree is reused across
            // audio ticks.
            let fp = inspector_fingerprint(r, track, routing_collapsed, chain_collapsed);
            let lazy_groups =
                iced::widget::lazy(fp, move |_: &u64| -> Element<'static, Message> {
                    let mut col = column![].spacing(0);
                    // Fresh external-instrument track (nothing paired yet):
                    // a dashed onboarding card walks the user through the
                    // four setup steps before the routing pickers (#459).
                    if ext_status == Some(ExternalInstrumentStatus::Unconfigured) {
                        col = col
                            .push(onboarding::onboarding_card())
                            .push(Space::new().height(18));
                    }
                    col = col.push(routing::routing_group(r, track, routing_collapsed));
                    col = col
                        .push(Space::new().height(18))
                        .push(chain::chain_group(r, track, chain_collapsed));
                    col.into()
                });

            iced::widget::scrollable(
                column![
                    header,
                    Space::new().height(18),
                    signal,
                    Space::new().height(18),
                    lazy_groups,
                ]
                .spacing(0),
            )
            .height(Length::Fill)
            .into()
        }
        None => render_empty(),
    };

    chrome(body)
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

/// Hash every field the lazy ROUTING + CHAIN groups read. The lazy
/// widget compares this across frames — when nothing has changed, the
/// cached widget tree is reused (which is the resize hot path). The
/// live level fields are intentionally absent: the SIGNAL group renders
/// them per-frame *outside* the lazy region.
pub(crate) fn inspector_fingerprint(
    r: &crate::Resonance,
    t: &TrackState,
    routing_collapsed: bool,
    chain_collapsed: bool,
) -> u64 {
    use std::hash::{Hash, Hasher};
    use std::rc::Rc;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    routing_collapsed.hash(&mut h);
    chain_collapsed.hash(&mut h);
    t.id.hash(&mut h);
    t.name.hash(&mut h);
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
    t.pan.to_bits().hash(&mut h);
    // External-instrument fields the routing group reads: the monitor /
    // arm toggles, the patch + latency config, and the runtime offline
    // flags. Without these the lazy region wouldn't redraw when a patch
    // is picked or a device goes offline.
    t.monitor_enabled.hash(&mut h);
    t.record_armed.hash(&mut h);
    let is_external = r.external_instruments.contains_key(&t.id);
    is_external.hash(&mut h);
    if let Some(ext) = r.external_instruments.get(&t.id) {
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
    }
    for p in &t.plugins {
        p.instance_id.hash(&mut h);
        p.plugin_name.hash(&mut h);
    }
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
    Rc::as_ptr(&r.view_caches.midi_input_choices).hash(&mut h);
    Rc::as_ptr(&r.view_caches.midi_output_choices).hash(&mut h);
    Rc::as_ptr(&r.view_caches.output_choices).hash(&mut h);
    Rc::as_ptr(&r.view_caches.fx_plugins).hash(&mut h);
    Rc::as_ptr(&r.view_caches.instrument_plugins).hash(&mut h);
    // Audio input picker uses `r.input_devices` directly (its options
    // include the per-device channel count, which the cached choice
    // lists above don't carry).
    r.input_devices.len().hash(&mut h);
    for d in &r.input_devices {
        d.name.hash(&mut h);
        d.channels.hash(&mut h);
    }
    // Live MIDI device list too — the audio block isn't a strict
    // function of the cached lists since the stale-override branch
    // peeks at `midi_input_devices` directly.
    r.midi_input_devices.len().hash(&mut h);
    r.midi_output_devices.len().hash(&mut h);
    h.finish()
}

fn render_empty() -> Element<'static, Message> {
    column![
        text("INSPECTOR")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        Space::new().height(8),
        text("Select a track or bus to view its routing and signal.")
            .size(12)
            .color(theme::TEXT_3),
    ]
    .spacing(0)
    .into()
}

// ---------------------------------------------------------------------------
// SIGNAL group — 2×2 stat tiles.
// ---------------------------------------------------------------------------

fn signal_group(
    track: &TrackState,
    collapsed: bool,
    is_external: bool,
) -> Element<'static, Message> {
    // External-instrument tracks meter the hardware *return*, so the
    // group reads "Signal · Return" per design doc #169.
    let title = if is_external {
        "SIGNAL · RETURN"
    } else {
        "SIGNAL"
    };
    if collapsed {
        return widgets::group_header(title, MixerInspectorGroup::Signal, true);
    }

    let peak = track.level_l.max(track.level_r);
    let peak_db = if peak < 1e-4 {
        "−∞ dB".to_string()
    } else {
        format!("{:+.1} dB", 20.0 * peak.log10())
    };

    let rms = "—".to_string();

    let pan = format_pan(track.pan).into_owned();
    let out = match track.output {
        TrackOutput::Master => "Master".to_string(),
        TrackOutput::Bus(_) => "Bus".to_string(),
    };

    let row1 = row![
        widgets::stat_tile("PEAK", peak_db),
        Space::new().width(10),
        widgets::stat_tile("RMS", rms),
    ]
    .align_y(alignment::Vertical::Center);
    let row2 = row![
        widgets::stat_tile("PAN", pan),
        Space::new().width(10),
        widgets::stat_tile("OUT", out),
    ]
    .align_y(alignment::Vertical::Center);

    column![
        widgets::group_header(title, MixerInspectorGroup::Signal, false),
        Space::new().height(10),
        row1,
        Space::new().height(10),
        row2,
    ]
    .spacing(0)
    .into()
}
