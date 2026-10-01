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
//! sends into it, and where it goes. Order (mixer-cleanup.md §3.3):
//! CHAIN → ROUTING → AUTOMATION → BUS, where BUS holds **Delete bus**.

use iced::widget::{column, container, pick_list, row, text, Space};
use iced::{alignment, Element, Length};
use resonance_audio::types::{ScannedPlugin, SendSource, TrackOutput};

use crate::message::{
    BusMessage, Message, MixerMessage, PluginMessage, PresetAddOwner, PresetUiMessage,
};
use crate::state::{BusState, MixerInspectorGroup};
use crate::theme;
use crate::view::mixer::automation::AutoChan;
use crate::view::mixer::picks::PluginOwner;

/// The inspector body for `bus`. The whole body below the title sits in
/// one lazy region keyed on everything it reads ([`fingerprint`]); none
/// of it is live per-tick state.
pub(super) fn view<'a>(r: &'a crate::Resonance, bus: &'a BusState) -> Element<'a, Message> {
    // Title row: the bus name in the warm accent that identifies busses
    // everywhere else in the mixer, the type tag, and a RETURN badge when
    // the bus is flagged as an aux return.
    let mut title_row = row![
        text(bus.name.clone())
            .size(17)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::WARM),
        Space::new().width(8),
        super::widgets::type_tag("Bus"),
    ]
    .spacing(0)
    .align_y(alignment::Vertical::Center);
    if bus.is_return {
        title_row = title_row.push(Space::new().width(6)).push(return_badge());
    }

    let fp = fingerprint(r, bus);
    let lazy_groups = iced::widget::lazy(fp, move |_: &u64| -> Element<'static, Message> {
        column![
            chain_group(r, bus, collapsed(r, MixerInspectorGroup::Chain)),
            Space::new().height(18),
            routing_group(r, bus, collapsed(r, MixerInspectorGroup::Routing)),
            Space::new().height(18),
            super::automation::automation_group(
                r,
                AutoChan::Bus(bus.id),
                &bus.plugins,
                &[],
                collapsed(r, MixerInspectorGroup::Automation),
            ),
            Space::new().height(18),
            bus_group(bus, collapsed(r, MixerInspectorGroup::Bus)),
        ]
        .spacing(0)
        .width(Length::Fill)
        .into()
    });

    iced::widget::scrollable(
        column![
            super::widgets::header(title_row),
            Space::new().height(18),
            lazy_groups,
        ]
        .spacing(0)
        .width(Length::Fill),
    )
    .height(Length::Fill)
    // Embedded, not floating: the scrollbar takes its own column beside
    // the groups instead of drawing over their right edge (pickers'
    // carets, the sends' dB readouts and trash buttons).
    .spacing(6)
    .into()
}

fn collapsed(r: &crate::Resonance, group: MixerInspectorGroup) -> bool {
    super::collapsed(r, group)
}

/// Hash every field the lazy bus body reads.
pub(crate) fn fingerprint(r: &crate::Resonance, bus: &BusState) -> u64 {
    use std::hash::{Hash, Hasher};
    use std::rc::Rc;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    super::hash_collapse_state(&mut h, r);
    bus.id.hash(&mut h);
    bus.name.hash(&mut h);
    bus.is_return.hash(&mut h);
    for p in &bus.plugins {
        p.instance_id.hash(&mut h);
        p.plugin_name.hash(&mut h);
        // Drives the bypass dot and its press message (review VIEW-08).
        p.bypassed.hash(&mut h);
    }
    super::chain::hash_chain_ui(&mut h, r, &bus.plugins);
    super::automation::hash_into(&mut h, r, AutoChan::Bus(bus.id), &bus.plugins, &[]);
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
            // The list names each source, so a rename must redraw it.
            match s.source {
                SendSource::Track(id) => r
                    .registry
                    .tracks
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| &t.name)
                    .hash(&mut h),
                SendSource::Bus(id) => r
                    .registry
                    .busses
                    .iter()
                    .find(|b| b.id == id)
                    .map(|b| &b.name)
                    .hash(&mut h),
            }
        }
    }
    Rc::as_ptr(&r.ui.view_caches.fx_plugins).hash(&mut h);
    Rc::as_ptr(&r.presets.fx_favorite_picks).hash(&mut h);
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
        Space::new().height(10),
        // The return-role flag was mirrored and badged but never
        // settable from the GUI (ba todo #1310) — so an existing bus
        // could not be promoted into a send destination, and the SENDS
        // block's picker would never list it. `CreateReturnFromSend`
        // raises the same command for a brand-new bus.
        super::widgets::toggle_button(
            "AUX RETURN",
            bus.is_return,
            theme::WARM,
            theme::WARM_DIM,
            Message::Mixer(MixerMessage::SetBusReturnRole(bus.id, !bus.is_return)),
        ),
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

    // Every entry is an effect: a bus has no instrument slot, so the
    // only limits on a move are the two ends.
    col = col.push(super::chain::chain_rows(
        r,
        PluginOwner::Bus(bus.id),
        &bus.plugins,
        None,
    ));
    if let Some(replace) = super::chain::replace_picker(r, &bus.plugins) {
        return col.push(replace).into();
    }

    if !r.ui.view_caches.fx_plugins.is_empty() {
        let bus_id = bus.id;
        let picker = pick_list(
            r.ui.view_caches.fx_plugins.clone(),
            None::<ScannedPlugin>,
            move |plugin: ScannedPlugin| Message::Bus(BusMessage::AddPluginToBus(bus_id, plugin)),
        )
        .placeholder("+ Add to chain")
        .text_size(12)
        .padding([8, 10])
        .width(Length::Fill);
        col = col.push(picker);
        if !r.presets.fx_favorite_picks.is_empty() {
            let with_preset = pick_list(
                r.presets.fx_favorite_picks.clone(),
                None::<crate::state::presets::PresetAddPick>,
                move |pick| {
                    Message::Plugin(PluginMessage::PresetUi(PresetUiMessage::AddWithPreset {
                        owner: PresetAddOwner::Bus(bus_id),
                        pick,
                    }))
                },
            )
            .placeholder("\u{25b8} with preset\u{2026}")
            .text_size(12)
            .padding([8, 10])
            .width(Length::Fill);
            col = col.push(with_preset);
        }
    }

    col.into()
}

// ---------------------------------------------------------------------------
// BUS — the bus's own actions.
// ---------------------------------------------------------------------------

/// BUS group: **Delete bus**, the same `RemoveBus` the strip's trash icon
/// sends (mixer-cleanup.md §3.3 — the trash leaves the strip later).
fn bus_group(bus: &BusState, collapsed: bool) -> Element<'static, Message> {
    let header = super::widgets::group_header("BUS", MixerInspectorGroup::Bus, collapsed);
    if collapsed {
        return header;
    }
    column![
        header,
        Space::new().height(10),
        super::widgets::action_button(
            "DELETE BUS",
            Some(Message::Bus(BusMessage::RemoveBus(bus.id))),
            true,
        ),
    ]
    .spacing(0)
    .into()
}
