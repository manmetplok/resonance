//! Chain-reorder affordances — the ▲/▼ pair shared by the mixer
//! inspector's CHAIN rows and the channel strips' plugin slots (ba todo
//! #1302, doc #276 item 2.1).
//!
//! The backend for all three chains has been complete since ba doc #273:
//! [`PluginMessage::MovePluginInTrack`], [`BusMessage::MovePluginInBus`]
//! and [`MasterMessage::MovePluginInMaster`] each send their engine
//! command, mirror the new order app-side for read-your-writes, take one
//! undo entry, and are reachable over the control API as
//! `track/bus/master.move_effect`. The GUI simply never raised them — so
//! an agent could reorder a chain and a human could not, and correcting
//! insert order by hand meant deleting a plugin and re-adding it, losing
//! every parameter along the way.
//!
//! Whether a move is *offered* is asked of the domain rule
//! ([`crate::plugin_chain::resolve_effect_move`]), never re-derived
//! here. A greyed ▲ and a refused move have to agree, and the
//! instrument-floor rule belongs to the chain, not to the view.
//!
//! Drag-and-drop is deliberately out of scope (see the todo).

use iced::alignment::Vertical;
use iced::widget::{button, row};
use iced::Element;
use resonance_audio::types::PluginInstanceId;

use crate::message::{BusMessage, MasterMessage, Message, PluginMessage};
use crate::theme;

use super::picks::PluginOwner;

/// The move-up / move-down messages for one slot of a plugin chain.
///
/// `None` means the affordance is disabled: the slot is already at that
/// end of the chain, or the move would displace the track's instrument.
/// The button is then rendered without an `on_press` rather than raising
/// a move the pre-dispatch gate would refuse — the todo's "disabled
/// rather than erroring at the chain ends and at the instrument floor".
#[derive(Debug, Clone, Default)]
pub(crate) struct ChainMoves {
    pub up: Option<Message>,
    pub down: Option<Message>,
}

/// Resolve the reorder messages for slot `index` of `owner`'s chain,
/// where the chain holds `len` plugins.
///
/// Track chains run the request past [`crate::plugin_chain`], so the
/// instrument slot offers neither direction and the effect directly
/// after it cannot move up onto it. Bus and master chains have no
/// structural slot, so the only limits are the two ends.
pub(crate) fn chain_moves(
    r: &crate::Resonance,
    owner: PluginOwner,
    instance_id: PluginInstanceId,
    index: usize,
    len: usize,
) -> ChainMoves {
    match owner {
        PluginOwner::Track(track_id) => {
            let Some(track) = r.registry.tracks.iter().find(|t| t.id == track_id) else {
                return ChainMoves::default();
            };
            // `resolve_effect_move` answers three questions at once:
            // the instrument never moves, an effect never lands below
            // the floor, and a destination past the end clamps. A clamp
            // back onto the slot we started from is a no-op, which is
            // exactly the "already at the end" case — so filtering it
            // out is what disables ▼ on the last row.
            let resolved = |to: usize| {
                crate::plugin_chain::resolve_effect_move(r, track, index as u32, to as u32)
                    .ok()
                    .filter(|dest| *dest as usize != index)
                    .map(|dest| {
                        Message::Plugin(PluginMessage::MovePluginInTrack {
                            track_id,
                            instance_id,
                            to_index: dest as usize,
                        })
                    })
            };
            ChainMoves {
                up: index.checked_sub(1).and_then(resolved),
                down: resolved(index + 1),
            }
        }
        PluginOwner::Bus(bus_id) => ChainMoves {
            up: (index > 0).then(|| {
                Message::Bus(BusMessage::MovePluginInBus {
                    bus_id,
                    instance_id,
                    to_index: index - 1,
                })
            }),
            down: (index + 1 < len).then(|| {
                Message::Bus(BusMessage::MovePluginInBus {
                    bus_id,
                    instance_id,
                    to_index: index + 1,
                })
            }),
        },
        PluginOwner::Master => ChainMoves {
            up: (index > 0).then(|| {
                Message::Master(MasterMessage::MovePluginInMaster {
                    instance_id,
                    to_index: index - 1,
                })
            }),
            down: (index + 1 < len).then(|| {
                Message::Master(MasterMessage::MovePluginInMaster {
                    instance_id,
                    to_index: index + 1,
                })
            }),
        },
    }
}

/// The ▲ / ▼ pair for one chain row.
///
/// `size` scales the caret so the roomy inspector row and the cramped
/// 140 px strip slot can share one widget instead of drifting into two.
pub(crate) fn move_buttons(moves: &ChainMoves, size: f32) -> Element<'static, Message> {
    row![
        move_button(theme::fa::CARET_UP, moves.up.clone(), size),
        move_button(theme::fa::CARET_DOWN, moves.down.clone(), size),
    ]
    .spacing(1)
    .align_y(Vertical::Center)
    .into()
}

fn move_button(glyph: char, message: Option<Message>, size: f32) -> Element<'static, Message> {
    // TEXT_4 is the faintest step in the ramp: the control stays legible
    // as a chain-shape hint at the ends without reading as actionable.
    let color = if message.is_some() {
        theme::TEXT_2
    } else {
        theme::TEXT_4
    };
    let mut b = button(theme::icon(glyph).size(size).color(color))
        .style(|_theme, status| theme::small_button_style(status))
        .padding([1, 3]);
    if let Some(message) = message {
        b = b.on_press(message);
    }
    b.into()
}
