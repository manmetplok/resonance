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
//! Drag-and-drop came into scope with mixer-cleanup.md slice S7: the
//! inspector CHAIN rows drag by their ⠿ handle, and the drop asks
//! [`drop_move`] — the same domain rule, so a drop the carets would
//! refuse (onto or off the instrument slot, into another owner's chain)
//! is refused here too. ▲/▼ stay as the ☰ menu's Move up / Move down.

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
/// `size` scales the caret and `pad_x` its horizontal padding, so the
/// roomy inspector row and the cramped 140 px strip slot can share one
/// widget instead of drifting into two. The strip slot has to fit four
/// icon controls beside the plugin's name and runs at 1 px (see
/// `SLOT_ICON_PAD_X`); the inspector has the width for 3.
pub(crate) fn move_buttons(
    moves: &ChainMoves,
    size: f32,
    pad_x: u16,
) -> Element<'static, Message> {
    row![
        move_button(theme::fa::CARET_UP, moves.up.clone(), size, pad_x),
        move_button(theme::fa::CARET_DOWN, moves.down.clone(), size, pad_x),
    ]
    .spacing(1)
    .align_y(Vertical::Center)
    .into()
}

fn move_button(
    glyph: char,
    message: Option<Message>,
    size: f32,
    pad_x: u16,
) -> Element<'static, Message> {
    // TEXT_4 is the faintest step in the ramp: the control stays legible
    // as a chain-shape hint at the ends without reading as actionable.
    let color = if message.is_some() {
        theme::TEXT_2
    } else {
        theme::TEXT_4
    };
    let mut b = button(theme::icon(glyph).size(size).color(color))
        .style(|_theme, status| theme::small_button_style(status))
        .padding([1, pad_x]);
    if let Some(message) = message {
        b = b.on_press(message);
    }
    b.into()
}

/// The reorder a CHAIN-row drag of `dragged` dropped onto the row of
/// `onto` asks for: `dragged` takes `onto`'s position in the chain
/// (slice S7). `None` when the drop is refused or would change nothing:
///
/// - either slot is gone, or the two sit in different chains — a drag
///   only ever reorders its own owner's chain;
/// - the drop lands on the slot it started from;
/// - on a track, [`crate::plugin_chain::resolve_effect_move`] refuses it
///   (the instrument never moves, and no effect goes above it).
pub(crate) fn drop_move(
    r: &crate::Resonance,
    dragged: PluginInstanceId,
    onto: PluginInstanceId,
) -> Option<Message> {
    use crate::state::PluginLocator;
    let (owner, from) = crate::update::plugin_replace::locate_slot(r, dragged)?;
    let (onto_owner, to) = crate::update::plugin_replace::locate_slot(r, onto)?;
    if owner != onto_owner || from == to {
        return None;
    }
    match owner {
        PluginLocator::Track(track_id) => {
            let track = r.registry.tracks.iter().find(|t| t.id == track_id)?;
            let dest =
                crate::plugin_chain::resolve_effect_move(r, track, from as u32, to as u32).ok()?;
            (dest as usize != from).then(|| {
                Message::Plugin(PluginMessage::MovePluginInTrack {
                    track_id,
                    instance_id: dragged,
                    to_index: dest as usize,
                })
            })
        }
        PluginLocator::Bus(bus_id) => Some(Message::Bus(BusMessage::MovePluginInBus {
            bus_id,
            instance_id: dragged,
            to_index: to,
        })),
        PluginLocator::Master => Some(Message::Master(MasterMessage::MovePluginInMaster {
            instance_id: dragged,
            to_index: to,
        })),
    }
}

/// The message that removes `instance_id` from `owner`'s chain — the
/// same one per owner wherever a remove is offered.
pub(crate) fn remove_message(owner: PluginOwner, instance_id: PluginInstanceId) -> Message {
    match owner {
        PluginOwner::Track(track_id) => {
            Message::Plugin(PluginMessage::RemovePluginFromTrack(track_id, instance_id))
        }
        PluginOwner::Bus(bus_id) => {
            Message::Bus(BusMessage::RemovePluginFromBus(bus_id, instance_id))
        }
        PluginOwner::Master => Message::Master(MasterMessage::RemovePluginFromMaster(instance_id)),
    }
}
