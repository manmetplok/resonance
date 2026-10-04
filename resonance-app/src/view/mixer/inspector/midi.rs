//! TRACK › MIDI CONTROL — the MIDI Learn bindings on this track's
//! controls, with a per-row clear, and a "Learn MIDI for…" picker over
//! every target the track has: its fader, pan, mute and solo, its sends,
//! and every parameter of every plugin on its chain (doc #167, W1).
//!
//! The picker is how a plugin parameter gets learned when the plugin
//! draws its own editor (which the host cannot right-click into); the
//! strip's fader, pan, M and S also learn from their right-click menu.

use std::hash::{Hash, Hasher};

use iced::widget::{button, column, container, pick_list, row, text};
use iced::{alignment, Element, Length};
use resonance_common::{MidiBinding, MidiTarget, SendId};

use crate::message::{Message, MidiMapMessage};
use crate::state::{source_label, TrackState};
use crate::theme;
use crate::view::midi_learn::target_label;

/// One "Learn MIDI for…" option.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LearnChoice {
    pub target: MidiTarget,
    pub label: String,
}

impl std::fmt::Display for LearnChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// Whether `target` is one of `track`'s controls.
fn on_track(track: &TrackState, target: MidiTarget) -> bool {
    match target {
        MidiTarget::TrackVolume(id)
        | MidiTarget::TrackPan(id)
        | MidiTarget::TrackMute(id)
        | MidiTarget::TrackSolo(id) => id == track.id,
        MidiTarget::SendLevel { track: id, .. } => id == track.id,
        MidiTarget::PluginParam { instance, .. } => {
            track.plugins.iter().any(|p| p.instance_id == instance)
        }
        MidiTarget::Transport(_) => false,
    }
}

/// The bindings on `track`'s controls, ordered by id.
fn track_bindings(r: &crate::Resonance, track: &TrackState) -> Vec<MidiBinding> {
    r.devices
        .midi_map
        .sorted()
        .into_iter()
        .filter(|b| on_track(track, b.target))
        .collect()
}

/// Every target the picker offers for `track`: mixer controls, sends,
/// then each plugin's writable parameters in chain order.
pub(crate) fn learn_choices(r: &crate::Resonance, track: &TrackState) -> Vec<LearnChoice> {
    let mut out: Vec<LearnChoice> = [
        (MidiTarget::TrackVolume(track.id), "Volume"),
        (MidiTarget::TrackPan(track.id), "Pan"),
        (MidiTarget::TrackMute(track.id), "Mute"),
        (MidiTarget::TrackSolo(track.id), "Solo"),
    ]
    .into_iter()
    .map(|(target, label)| LearnChoice {
        target,
        label: label.to_string(),
    })
    .collect();
    for send in super::sends::sends_for_track(r, track.id) {
        let target = MidiTarget::SendLevel {
            track: track.id,
            send: SendId(send.id),
        };
        let label = target_label(r, target);
        let label = label.split(" \u{b7} ").nth(1).unwrap_or(&label).to_string();
        out.push(LearnChoice { target, label });
    }
    for plugin in &track.plugins {
        for p in plugin.params.iter().filter(|p| !p.hidden && !p.read_only) {
            out.push(LearnChoice {
                target: MidiTarget::PluginParam {
                    instance: plugin.instance_id,
                    param_id: p.id,
                },
                label: format!("{} \u{b7} {}", plugin.plugin_name, p.name),
            });
        }
    }
    out
}

/// The section, for the TRACK group's lazy body.
pub(super) fn midi_section(r: &crate::Resonance, track: &TrackState) -> Element<'static, Message> {
    let map = &r.devices.midi_map;
    let mut col = column![super::widgets::sub_label("MIDI CONTROL")].spacing(6);

    for b in track_bindings(r, track) {
        let label = target_label(r, b.target);
        // On the track's own inspector the track name is noise.
        let label = label
            .strip_prefix(&format!("{} \u{b7} ", track.name))
            .unwrap_or(&label)
            .to_string();
        col = col.push(
            row![
                text(label).size(11).color(theme::TEXT_1).width(Length::Fill),
                text(source_label(b.source))
                    .size(10)
                    .font(theme::MONO_FONT)
                    .color(theme::TEXT_3),
                button(theme::icon(theme::fa::TRASH).size(10).color(theme::TEXT_3))
                    .on_press(Message::MidiMap(MidiMapMessage::Clear(b.id)))
                    .padding([2, 5])
                    .style(|_theme, status| theme::small_button_style(status)),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center),
        );
    }

    if let Some(target) = map.learn_target.filter(|t| on_track(track, *t)) {
        let label = target_label(r, target);
        col = col.push(
            container(
                row![
                    text(format!("Move a control for {label}\u{2026}"))
                        .size(11)
                        .color(theme::ACCENT_SOFT)
                        .width(Length::Fill),
                    button(text("Cancel").size(11).color(theme::TEXT_2))
                        .on_press(Message::MidiMap(MidiMapMessage::CancelLearn))
                        .padding([2, 8])
                        .style(|_theme, status| theme::ghost_button_style(status)),
                ]
                .align_y(alignment::Vertical::Center),
            )
            .padding([6, 8])
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::ACCENT_DIM)),
                border: iced::Border {
                    color: theme::ACCENT_LINE,
                    width: 1.0,
                    radius: theme::RADIUS_SM.into(),
                },
                ..Default::default()
            }),
        );
    }

    col.push(
        pick_list(learn_choices(r, track), None::<LearnChoice>, |c: LearnChoice| {
            Message::MidiMap(MidiMapMessage::Learn(c.target))
        })
        .placeholder("Learn MIDI for\u{2026}")
        .text_size(12)
        .padding([8, 10])
        .width(Length::Fill),
    )
    .into()
}

/// Hash what [`midi_section`] draws for `track` (the inspector's lazy
/// fingerprint). Plugin and send names are hashed by the CHAIN / SENDS
/// parts of the fingerprint already.
pub(super) fn hash_into<H: Hasher>(h: &mut H, r: &crate::Resonance, track: &TrackState) {
    r.devices.midi_map.learn_target.hash(h);
    for b in track_bindings(r, track) {
        b.id.hash(h);
        b.source.hash(h);
        b.target.hash(h);
    }
    for plugin in &track.plugins {
        // The picker lists parameters by name; the AUTOMATION hash covers
        // the list by identity, this by content.
        plugin.params.len().hash(h);
        for p in &plugin.params {
            p.id.hash(h);
            p.name.hash(h);
            p.hidden.hash(h);
            p.read_only.hash(h);
        }
    }
}
