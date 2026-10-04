//! Engine → GUI event dispatch.
//!
//! Each `AudioEvent` variant is routed to a per-domain handler module
//! by the free `handle_engine_event` function in `dispatch.rs`. The
//! dispatch itself stays thin so it's easy to find which file owns a
//! given event.

mod automation;
mod aux_sends;
pub(crate) mod clips;
mod dispatch;
mod export;
mod external_instrument;
mod freeze;
pub(crate) mod midi;
mod midi_map;
pub(crate) mod plugins;
pub(crate) mod pool;
mod presets;
mod project_io;
mod reference;
mod takes;
pub(crate) mod tracks;
mod transport;

pub(crate) use dispatch::handle_engine_event;

/// The bare kinds the engine names the clips it creates with (ARCH-04
/// D-7d, design doc D-6 §7a.1): a recording ("Recording"), a live-MIDI
/// capture ("MIDI Take"), a cycle-record audio pass ("Take" — never
/// mirrored as a clip, so never numbered here).
const ENGINE_CLIP_KINDS: [&str; 3] = ["Recording", "MIDI Take", "Take"];

/// The name the app shows for a clip the engine created, from the bare
/// kind the engine sent: `"<kind> <n>"`, numbered per track (design doc
/// D-6 §7a.1 — the engine's ids are large, and never appear in a name).
///
/// `n` is one more than the highest `n` any clip named `"<kind> <n>"`
/// on `siblings` — the clips of the same list already on this track —
/// carries, or 1 when none does. Derived from the names alone, so it needs
/// no counter of its own and follows the project wherever the names go:
/// it survives a reload (the names are saved), follows an undo (an undone
/// "Recording 2" frees 2 for the next take), and never repeats a number
/// still on the track, even after an earlier one was deleted or renamed. A
/// name that is not a bare engine kind is kept as it is.
pub(crate) fn number_engine_clip_name<'a>(
    kind: String,
    siblings: impl Iterator<Item = &'a str>,
) -> String {
    if !ENGINE_CLIP_KINDS.contains(&kind.as_str()) {
        return kind;
    }
    let highest = siblings
        .filter_map(|name| name.strip_prefix(kind.as_str())?.strip_prefix(' '))
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|digits| digits.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("{kind} {}", highest + 1)
}
