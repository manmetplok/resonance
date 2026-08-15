//! Bus inspector — what the mixer's right pane shows while a BUS strip
//! is selected.
//!
//! Busses were unreachable from the inspector: it looked its selection
//! up in `registry.tracks` only, and nothing ever set a bus selection in
//! the first place, so clicking a bus strip left the pane on whatever
//! track was selected before (or on the "Select a track or bus" empty
//! state, which promised a view that did not exist).
//!
//! A bus is not a track and the groups say so. There is no input device,
//! no MIDI, no output picker — a bus always sums to master — so ROUTING
//! answers the questions a bus actually raises: what feeds it, what
//! sends into it, and where it goes. SIGNAL and CHAIN are the track
//! versions with the instrument slot removed.

use iced::widget::{column, container, pick_list, row, text, Space};
use iced::{alignment, Element, Length};
use resonance_audio::types::{ScannedPlugin, SendSource, TrackOutput};

use crate::message::{BusMessage, Message};
use crate::state::{BusState, MixerInspectorGroup};
use crate::theme;
use crate::util::format_pan;
use crate::view::mixer::picks::PluginOwner;
use crate::view::mixer::reorder;

/// The inspector body for `bus`. Mirrors the track path's split: the
/// live SIGNAL tiles render every frame, ROUTING + CHAIN sit inside a
/// lazy region keyed on everything they read.
pub(super) fn view<'a>(r: &'a crate::Resonance, bus: &'a BusState) -> Element<'a, Message> {
    let signal_collapsed = collapsed(r, MixerInspectorGroup::Signal);
    let routing_collapsed = collapsed(r, MixerInspectorGroup::Routing);
    let chain_collapsed = collapsed(r, MixerInspectorGroup::Chain);

    // Title row: the bus name in the warm accent that identifies busses
    // everywhere else in the mixer, plus a RETURN badge when the bus is
    // flagged as an aux return.
    let mut title_row = row![text(bus.name.clone())
        .size(17)
        .font(theme::UI_FONT_MEDIUM)
        .color(theme::WARM)]
    .spacing(0)
    .align_y(alignment::Vertical::Center);
    if bus.is_return {
        title_row = title_row.push(Space::new().width(8)).push(return_badge());
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

    // SIGNAL stays outside the lazy region: its PEAK tile reads the
    // per-tick bus levels, which the fingerprint deliberately omits (see
    // ui-work.md §11.2).
    let signal = signal_group(bus, signal_collapsed);

    let fp = fingerprint(r, bus, routing_collapsed, chain_collapsed);
    let lazy_groups = iced::widget::lazy(fp, move |_: &u64| -> Element<'static, Message> {
        column![
            routing_group(r, bus, routing_collapsed),
            Space::new().height(18),
            chain_group(r, bus, chain_collapsed),
        ]
        .spacing(0)
        .into()
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

fn collapsed(r: &crate::Resonance, group: MixerInspectorGroup) -> bool {
    r.mixer.collapsed_inspector_groups.contains(&group)
}

/// Hash every field the lazy ROUTING + CHAIN groups read. The live level
/// fields are intentionally absent — SIGNAL renders them per-frame
/// outside the lazy region.
fn fingerprint(
    r: &crate::Resonance,
    bus: &BusState,
    routing_collapsed: bool,
    chain_collapsed: bool,
) -> u64 {
    use std::hash::{Hash, Hasher};
    use std::rc::Rc;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    routing_collapsed.hash(&mut h);
    chain_collapsed.hash(&mut h);
    bus.id.hash(&mut h);
    bus.name.hash(&mut h);
    bus.is_return.hash(&mut h);
    for p in &bus.plugins {
        p.instance_id.hash(&mut h);
        p.plugin_name.hash(&mut h);
    }
    // MEMBERS is derived from every track's routing, so it changes when
    // a track is re-routed, renamed, added or removed.
    for t in &r.registry.tracks {
        if t.output == TrackOutput::Bus(bus.id) {
            t.id.hash(&mut h);
            t.name.hash(&mut h);
        }
    }
    // SENDS IN is derived from the aux-send graph.
    for s in &r.aux.sends {
        if s.dest == bus.id {
            s.id.hash(&mut h);
            s.source.hash(&mut h);
            s.enabled.hash(&mut h);
            s.level_db.to_bits().hash(&mut h);
        }
    }
    Rc::as_ptr(&r.view_caches.fx_plugins).hash(&mut h);
    h.finish()
}

/// The warm "RETURN" pill shown beside the name of an aux return bus.
fn return_badge() -> Element<'static, Message> {
    container(
        text("RETURN")
            .size(8)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::WARM),
    )
    .padding([1, 5])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::WARM_DIM)),
        border: iced::Border {
            color: theme::WARM_LINE,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    })
    .into()
}

// ---------------------------------------------------------------------------
// SIGNAL — the same 2×2 tiles a track gets. A bus always sums to master,
// so OUT is a constant rather than a routing readout.
// ---------------------------------------------------------------------------

fn signal_group(bus: &BusState, collapsed: bool) -> Element<'static, Message> {
    if collapsed {
        return super::widgets::group_header("SIGNAL", MixerInspectorGroup::Signal, true);
    }

    let peak = bus.level_l.max(bus.level_r);
    let peak_db = if peak < 1e-4 {
        "−∞ dB".to_string()
    } else {
        format!("{:+.1} dB", 20.0 * peak.log10())
    };

    let row1 = row![
        super::widgets::stat_tile("PEAK", peak_db),
        Space::new().width(10),
        super::widgets::stat_tile("RMS", "—".to_string()),
    ]
    .align_y(alignment::Vertical::Center);
    let row2 = row![
        super::widgets::stat_tile("PAN", format_pan(bus.pan).into_owned()),
        Space::new().width(10),
        super::widgets::stat_tile("OUT", "Master".to_string()),
    ]
    .align_y(alignment::Vertical::Center);

    column![
        super::widgets::group_header("SIGNAL", MixerInspectorGroup::Signal, false),
        Space::new().height(10),
        row1,
        Space::new().height(10),
        row2,
    ]
    .spacing(0)
    .into()
}

// ---------------------------------------------------------------------------
// ROUTING — what feeds the bus, and where it goes.
// ---------------------------------------------------------------------------

fn routing_group(
    r: &crate::Resonance,
    bus: &BusState,
    collapsed: bool,
) -> Element<'static, Message> {
    if collapsed {
        return super::widgets::group_header("ROUTING", MixerInspectorGroup::Routing, true);
    }

    // Members: the tracks whose output lands on this bus. This is the
    // question a bus strip cannot answer on its own — the routing lives
    // on the tracks, so without it "why is this bus silent?" means
    // opening every track in turn.
    let members: Vec<&str> = r
        .registry
        .tracks
        .iter()
        .filter(|t| t.output == TrackOutput::Bus(bus.id))
        .map(|t| t.name.as_str())
        .collect();
    let members_value = if members.is_empty() {
        "(none)".to_string()
    } else {
        members.join(", ")
    };

    // Sends in: aux sends landing here. Only meaningful on a return bus,
    // but a send into a plain bus is legal and worth showing rather than
    // hiding behind the flag.
    let sends: Vec<String> = r
        .aux
        .sends
        .iter()
        .filter(|s| s.dest == bus.id)
        .map(|s| {
            let name = match s.source {
                SendSource::Track(id) => r
                    .registry
                    .tracks
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| format!("Track {id}")),
                SendSource::Bus(id) => r
                    .registry
                    .busses
                    .iter()
                    .find(|b| b.id == id)
                    .map(|b| b.name.clone())
                    .unwrap_or_else(|| format!("Bus {id}")),
            };
            if s.enabled {
                format!("{name} {:+.1} dB", s.level_db)
            } else {
                format!("{name} (off)")
            }
        })
        .collect();
    let sends_value = if sends.is_empty() {
        "(none)".to_string()
    } else {
        sends.join(", ")
    };

    column![
        super::widgets::group_header("ROUTING", MixerInspectorGroup::Routing, false),
        Space::new().height(10),
        super::widgets::info_row(
            "Members",
            format!("{} · {}", members.len(), members_value),
            members.is_empty(),
        ),
        super::widgets::info_row("Sends in", sends_value, sends.is_empty()),
        // A bus's output is structural, not a choice: every bus sums to
        // master. Shown so the signal path reads end-to-end rather than
        // stopping at the strip.
        super::widgets::info_row("Output", "Master".to_string(), false),
    ]
    .spacing(0)
    .into()
}

// ---------------------------------------------------------------------------
// CHAIN — the bus's effects, and the picker that adds one.
// ---------------------------------------------------------------------------

fn chain_group(
    r: &crate::Resonance,
    bus: &BusState,
    collapsed: bool,
) -> Element<'static, Message> {
    if collapsed {
        return super::widgets::group_header("CHAIN", MixerInspectorGroup::Chain, true);
    }

    let mut col = column![super::widgets::group_header(
        "CHAIN",
        MixerInspectorGroup::Chain,
        false,
    )]
    .spacing(10);

    if bus.plugins.is_empty() {
        col = col.push(super::chain::empty_chain_row());
    } else {
        let chain_len = bus.plugins.len();
        for (index, plugin) in bus.plugins.iter().enumerate() {
            // Every entry is an effect: a bus has no instrument slot, so
            // the only limits on the reorder carets are the two ends.
            let moves = reorder::chain_moves(
                r,
                PluginOwner::Bus(bus.id),
                plugin.instance_id,
                index,
                chain_len,
            );
            col = col.push(super::chain::chain_row(&plugin.plugin_name, false, &moves));
        }
    }

    if !r.view_caches.fx_plugins.is_empty() {
        let bus_id = bus.id;
        let picker = pick_list(
            r.view_caches.fx_plugins.clone(),
            None::<ScannedPlugin>,
            move |plugin: ScannedPlugin| Message::Bus(BusMessage::AddPluginToBus(bus_id, plugin)),
        )
        .placeholder("+ Add to chain")
        .text_size(12)
        .padding([8, 10])
        .width(Length::Fill);
        col = col.push(picker);
    }

    col.into()
}
