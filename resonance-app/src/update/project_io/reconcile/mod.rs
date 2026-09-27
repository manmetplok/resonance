//! The `Reconcile` driver (ARCH-01 A-13, design `docs/design/A-13-reconcile.md`).
//!
//! A project reaches the app through [`reconcile_all`], from two callers:
//! a disk load (or template instantiate) after `ClearAll`
//! (`replay_loaded_project`, `old = None`) and an undo/redo
//! (`restore_from_snapshot`, `old = Some(current)`, no `ClearAll`). Every
//! project domain is restored by one [`Reconcile`] impl, in the one order
//! [`DOMAINS`] lists. Since A-13j there is no third path: the undo's
//! `ClearAll` fallback is gone, and `Origin::UndoFull` with it.
//! The [`Stage`]s group the table and document why each group sits where
//! it does; `DOMAINS` is sorted by stage.

mod app_side;
mod clips;
mod entities;
mod globals;
mod plugin_state;
mod removals;
mod restored;
mod routing;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use resonance_audio::types::{ClipId, MidiNote, PluginInstanceId};

pub use entities::{migrate_auto_name, sort_plugins_by_saved_order};

use crate::project::ProjectFile;
use crate::Resonance;

/// Which restore is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A disk load or template instantiate, after `ClearAll`; `old` is
    /// `None`.
    DiskLoad,
    /// An undo/redo: no `ClearAll`, the engine still holds everything, and
    /// `old` is the live state's file. Every domain diffs against it.
    Undo,
}

impl Origin {
    /// An undo/redo.
    pub fn is_undo(self) -> bool {
        matches!(self, Origin::Undo)
    }

    /// The engine was emptied by `ClearAll` before this restore: a disk
    /// load. Named for what the domains that read it care about.
    pub fn after_clear_all(self) -> bool {
        matches!(self, Origin::DiskLoad)
    }
}

/// What a domain may need besides the two files.
#[derive(Debug, Clone, Copy)]
pub struct ReconcileCtx<'a> {
    pub origin: Origin,
    /// The directory project-relative paths resolve against:
    /// `LoadedProject::project_dir` on a disk load, the live
    /// `io.project_path` on an undo (`None` for an untitled project). Its
    /// sibling `.freeze` directory holds the freeze caches an undo retires.
    pub project_dir: Option<&'a Path>,
    /// The target's MIDI notes per clip id (`LoadedProject::midi_notes`),
    /// which the `ProjectFile` does not carry. Read by `MidiClips`. `Arc`
    /// so an unchanged clip's notes reach `MidiClipState::notes` by
    /// pointer, not a copy (ARCH-09 A9-3).
    pub midi_notes: &'a HashMap<ClipId, Arc<Vec<MidiNote>>>,
    /// The target's plugin state blobs per instance id
    /// (`LoadedProject::plugin_states`), which the `ProjectFile` does not
    /// carry either. Read by the entity domains' plugin chains.
    pub plugin_states: &'a HashMap<PluginInstanceId, Arc<[u8]>>,
}

/// One project domain's restore, shared by every [`Origin`].
pub(crate) trait Reconcile {
    /// Stable name, recorded in `io.reconcile_trace`.
    const NAME: &'static str;

    /// Drive the app (and engine) state of this domain to `new`. `old` is
    /// the file the live state was built from on an undo, `None` after a
    /// disk load's `ClearAll`.
    fn reconcile(
        r: &mut Resonance,
        old: Option<&ProjectFile>,
        new: &ProjectFile,
        ctx: &ReconcileCtx<'_>,
    );
}

/// Groups of [`DOMAINS`], declared in the order they run. They used to
/// mark the points in each restore path's inline code where a group was
/// valid; since A-13f nothing runs between two stages, so they only group
/// the table and document why it is in the order it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Transport / master scalars, the transient UI a full replay resets,
    /// compose sections and the drum-pattern bank. The first thing both
    /// paths restore (the full path after `SetProjectDir`).
    Globals,
    /// Tempo events, chord track, markers, and the section chord trim
    /// (reads the meter). Right after `Globals` on both paths, before any
    /// track or clip is restored.
    Timeline,
    /// What a diff restore removes: the routing edges `new` lacks, the
    /// clips it does not keep (A-13i), then the plugin instances, tracks
    /// and busses (A-13h, A-13i). Before `Entities`, so an edge or a clip
    /// is gone before its endpoint and an entity before a re-add under its
    /// id. Nothing after a `ClearAll`.
    Removals,
    /// The entities: tracks, busses, the master chain, the track outputs,
    /// then each plugin's state (blob, bypass, params), then the registry
    /// and chain order. Right after `Removals` on both paths. After a
    /// `ClearAll` every entity is added; on the diff path only changed
    /// scalars are sent, and the busses and plugin instances `old` lacks
    /// are added (A-13h).
    Entities,
    /// The routing edges between entities: aux sends, then sidechain key
    /// routes. Right after `Entities` (the engine rejects a send naming an
    /// unregistered endpoint; a key route names a plugin instance id).
    Routing,
    /// The audio and MIDI clips, then state derived from them: the lyric
    /// side-table, the derived-clip map and the vocal audio-clip map.
    /// Right after `Routing`, on every track the clips sit on.
    Clips,
    /// App-side content restored whole, and the references. After `Clips`
    /// (the pool counts the clips' asset refs).
    Content,
    /// Domains that must see everything else restored: external
    /// instruments, then the automation lanes (a `DeviceParam` lane needs
    /// the engine's device bindings), the missing-plugin warning, and
    /// freeze last (a disk load's baseline fingerprints the replayed
    /// content, lanes included). The end of both paths' restore.
    Tail,
}

type ReconcileFn =
    for<'a, 'b, 'c, 'd> fn(&'a mut Resonance, Option<&'b ProjectFile>, &'c ProjectFile, &'d ReconcileCtx<'_>);

/// One row of [`DOMAINS`].
pub(crate) struct Domain {
    pub name: &'static str,
    pub stage: Stage,
    run: ReconcileFn,
}

const fn domain<D: Reconcile>(stage: Stage) -> Domain {
    Domain {
        name: D::NAME,
        stage,
        run: D::reconcile,
    }
}

/// Every migrated domain, in the order both restore paths run them.
/// Sorted by [`Stage`].
pub(crate) const DOMAINS: &[Domain] = &[
    // Before `Timeline`: the tempo map is rebuilt from the transport
    // scalars, and `SetBpm` must precede `SetTempoEvents`.
    domain::<globals::Transport>(Stage::Globals),
    domain::<globals::TransientUi>(Stage::Globals),
    // Before `Clips`: the load resets the derived map `DerivedClips`
    // restores. The drum bank's legacy promotion edits the definitions.
    domain::<globals::ComposeSections>(Stage::Globals),
    domain::<globals::DrumPatterns>(Stage::Globals),
    // Before the chord trim and every clip (derived-clip bar recovery
    // reads the tempo map).
    domain::<app_side::TempoEvents>(Stage::Timeline),
    domain::<app_side::ChordTrack>(Stage::Timeline),
    domain::<app_side::Markers>(Stage::Timeline),
    // Reads the meter (tempo events) and the sections.
    domain::<globals::SectionChordTrim>(Stage::Timeline),
    // Diff path only: edges before the entities they connect, instances
    // before a re-add under the same id.
    domain::<removals::RoutingRemovals>(Stage::Removals),
    // Clips before the tracks they sit on (`RemoveTrack` drops audio
    // clips silently and keeps MIDI clips).
    domain::<removals::ClipRemovals>(Stage::Removals),
    domain::<removals::EntityRemovals>(Stage::Removals),
    // Tracks before busses (as both paths always had it), the master
    // chain, then the track outputs once every bus they name exists.
    domain::<entities::Tracks>(Stage::Entities),
    domain::<entities::Busses>(Stage::Entities),
    domain::<entities::Master>(Stage::Entities),
    domain::<entities::TrackOutputs>(Stage::Entities),
    // After every chain it names is added / matched: blobs, then per-slot
    // bypass, then params (which win over the blob).
    domain::<plugin_state::PluginState>(Stage::Entities),
    // Last: resorts every registry the domains above filled or re-ordered,
    // puts every plugin chain into the target's order, rebuilds the
    // side-index.
    domain::<entities::EntityOrder>(Stage::Entities),
    // After every entity they connect. Sends before key routes, as both
    // paths always had it (the two are independent tables in the engine).
    domain::<routing::Sends>(Stage::Routing),
    domain::<routing::SidechainRoutes>(Stage::Routing),
    // The clips themselves, after every track they sit on (and, diff
    // path, the tempo map — no clip command reads it). Audio before MIDI,
    // as both paths always had it.
    domain::<clips::AudioClips>(Stage::Clips),
    domain::<clips::MidiClips>(Stage::Clips),
    // After the MIDI and audio clips they read / filter against.
    domain::<globals::ClipLyrics>(Stage::Clips),
    domain::<restored::DerivedClips>(Stage::Clips),
    // Last in `Clips`: reads the audio clips, placements and tempo map, and
    // reserves the derived counter past the audio clip ids after
    // `DerivedClips` set it.
    domain::<clips::VocalAudioClips>(Stage::Clips),
    // After the clips: the pool counts their asset refs. The full path's
    // order.
    domain::<restored::References>(Stage::Content),
    domain::<app_side::Pool>(Stage::Content),
    domain::<app_side::Quantize>(Stage::Content),
    domain::<app_side::Performance>(Stage::Content),
    domain::<app_side::TrackGroups>(Stage::Content),
    domain::<app_side::TakeGroups>(Stage::Content),
    // External instruments before the lanes (a `DeviceParam` lane needs
    // the device bindings), freeze last.
    domain::<restored::ExternalInstruments>(Stage::Tail),
    domain::<restored::AutomationLanes>(Stage::Tail),
    domain::<restored::MissingPlugins>(Stage::Tail),
    // Last: a disk load's baseline fingerprints everything above.
    domain::<restored::Freeze>(Stage::Tail),
    // The replay's closing command (ARCH-04 D-7d): the engine's fresh
    // clip-id grant, from a counter every domain above has seeded.
    domain::<restored::ClipIdGrant>(Stage::Tail),
];

/// Drive the app and engine to `new`: every [`DOMAINS`] entry, in table
/// order — every [`Stage`] in sequence. The one restore (ARCH-01 A-13j):
/// a disk load passes `old = None` after its `ClearAll`, an undo/redo
/// `old = Some(current)`. Clears `io.reconcile_trace` first.
pub(crate) fn reconcile_all(
    r: &mut Resonance,
    old: Option<&ProjectFile>,
    new: &ProjectFile,
    ctx: &ReconcileCtx<'_>,
) {
    debug_assert_eq!(
        old.is_some(),
        ctx.origin.is_undo(),
        "an undo diffs against the live file; a disk load has none"
    );
    r.io.reconcile_trace.clear();
    for d in DOMAINS {
        r.io.reconcile_trace.push((ctx.origin, d.name));
        (d.run)(r, old, new, ctx);
    }
}

/// The table as `(stage, name)`, for the order guard test.
pub fn domain_order() -> Vec<(Stage, &'static str)> {
    DOMAINS.iter().map(|d| (d.stage, d.name)).collect()
}
