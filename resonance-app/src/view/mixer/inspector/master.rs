//! Master inspector — what the right pane shows while the MASTER strip is
//! selected (mixer-cleanup.md §3.3): CHAIN (the master inserts and the
//! add picker), AUTOMATION (master gain and the inserts' params), and a
//! MASTER group holding Bounce.

use std::hash::{Hash, Hasher};
use std::rc::Rc;

use iced::widget::{column, pick_list, row, text, Space};
use iced::{alignment, Element, Length};
use resonance_audio::types::ScannedPlugin;

use crate::message::{
    MasterMessage, Message, PluginMessage, PresetAddOwner, PresetUiMessage, ProjectIoMessage,
};
use crate::state::MixerInspectorGroup;
use crate::theme;
use crate::view::mixer::automation::AutoChan;
use crate::view::mixer::picks::PluginOwner;

pub(super) fn view<'a>(r: &'a crate::Resonance) -> Element<'a, Message> {
    let title_row = row![
        text("Master")
            .size(17)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::TEXT_1),
        // No type tag: the title already says "Master", and there is
        // only one.
    ]
    .spacing(0)
    .align_y(alignment::Vertical::Center);

    let fp = fingerprint(r);
    let groups = iced::widget::lazy(fp, move |_: &u64| -> Element<'static, Message> {
        column![
            chain_group(r, super::collapsed(r, MixerInspectorGroup::Chain)),
            Space::new().height(18),
            super::automation::automation_group(
                r,
                AutoChan::Master,
                &r.master.plugins,
                &[],
                super::collapsed(r, MixerInspectorGroup::Automation),
            ),
            Space::new().height(18),
            master_group(r, super::collapsed(r, MixerInspectorGroup::Master)),
        ]
        .spacing(0)
        .width(Length::Fill)
        .into()
    });

    iced::widget::scrollable(
        column![
            super::widgets::header(title_row),
            Space::new().height(18),
            groups,
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

/// Hash every field the lazy master body reads.
pub(crate) fn fingerprint(r: &crate::Resonance) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    super::hash_collapse_state(&mut h, r);
    for p in &r.master.plugins {
        p.instance_id.hash(&mut h);
        p.plugin_name.hash(&mut h);
        p.bypassed.hash(&mut h);
    }
    super::chain::hash_chain_ui(&mut h, r, &r.master.plugins);
    super::automation::hash_into(&mut h, r, AutoChan::Master, &r.master.plugins, &[]);
    // The MASTER group swaps Bounce for a "Bouncing…" note mid-render.
    r.io.bouncing.hash(&mut h);
    Rc::as_ptr(&r.ui.view_caches.fx_plugins).hash(&mut h);
    Rc::as_ptr(&r.presets.fx_favorite_picks).hash(&mut h);
    h.finish()
}

/// CHAIN — the master's inserts, every one an effect, and the picker.
fn chain_group(r: &crate::Resonance, collapsed: bool) -> Element<'static, Message> {
    let header = super::widgets::group_header("CHAIN", MixerInspectorGroup::Chain, collapsed);
    if collapsed {
        return header;
    }
    let mut col = column![header].spacing(10);

    col = col.push(super::chain::chain_rows(
        r,
        PluginOwner::Master,
        &r.master.plugins,
        None,
    ));
    if let Some(replace) = super::chain::replace_picker(r, &r.master.plugins) {
        return col.push(replace).into();
    }

    if !r.ui.view_caches.fx_plugins.is_empty() {
        let picker = pick_list(
            r.ui.view_caches.fx_plugins.clone(),
            None::<ScannedPlugin>,
            |plugin: ScannedPlugin| Message::Master(MasterMessage::AddPluginToMaster(plugin)),
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
                |pick| {
                    Message::Plugin(PluginMessage::PresetUi(PresetUiMessage::AddWithPreset {
                        owner: PresetAddOwner::Master,
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

/// MASTER — the master's own actions. Bounce renders the project to a
/// WAV (the same message the master strip's Bounce button sends).
fn master_group(r: &crate::Resonance, collapsed: bool) -> Element<'static, Message> {
    let header = super::widgets::group_header("MASTER", MixerInspectorGroup::Master, collapsed);
    if collapsed {
        return header;
    }
    let bounce: Element<'static, Message> = if r.io.bouncing {
        super::widgets::action_button("BOUNCING\u{2026}", None, false)
    } else {
        super::widgets::action_button(
            "BOUNCE TO WAV",
            Some(Message::ProjectIo(ProjectIoMessage::BounceToWav)),
            false,
        )
    };
    column![header, Space::new().height(10), bounce]
        .spacing(0)
        .into()
}
