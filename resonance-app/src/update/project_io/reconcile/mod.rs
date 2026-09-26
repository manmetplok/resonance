//! The `Reconcile` driver (ARCH-01 A-13, design `docs/design/A-13-reconcile.md`).
//!
//! A project reaches the app through two restore paths: the full replay
//! after `ClearAll` (`replay_loaded_project` — a disk load or an undo's
//! structural fallback) and the undo diff replay (`try_diff_replay`). Every
//! project domain is restored by one [`Reconcile`] impl that both paths
//! run through [`reconcile_all_stages`], in the one order [`DOMAINS`]
//! lists.
//!
//! Since A-13f no domain is restored inline by either path: each entry
//! point is its setup (the ctx; on the full path the vocal side-table
//! clear and `SetProjectDir`), then `reconcile_all_stages`. What still
//! differs is how they get there: an undo always takes the diff path since
//! A-13i (`structurally_compatible` accepts every pair; A-13j deletes it
//! and the undo's `ClearAll` fallback), a disk load the full path.
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
    /// A disk load or template instantiate, after `ClearAll`.
    DiskLoad,
    /// An undo/redo's full replay after `ClearAll` (the structural
    /// fallback). Unlike a disk load, live state that survives the clear
    /// (freeze statuses, the reference monitor, the derived-clip counter)
    /// is kept.
    UndoFull,
    /// An undo/redo's diff replay: no `ClearAll`, the engine still holds
    /// everything.
    UndoDiff,
}

impl Origin {
    /// An undo/redo, on either path.
    pub fn is_undo(self) -> bool {
        !matches!(self, Origin::DiskLoad)
    }

    /// The engine was emptied by `ClearAll` before this restore.
    pub fn after_clear_all(self) -> bool {
        !matches!(self, Origin::UndoDiff)
    }
}

/// What a domain may need besides the two files.
#[derive(Debug, Clone, Copy)]
pub struct ReconcileCtx<'a> {
    pub origin: Origin,
    /// The directory project-relative paths resolve against:
    /// `LoadedProject::project_dir` on the full paths, the live
    /// `io.project_path` on the diff path (`None` for an untitled project).
    pub project_dir: Option<&'a Path>,
    /// The target's MIDI notes per clip id (`LoadedProject::midi_notes`),
    /// which the `ProjectFile` does not carry. Read by `MidiClips`.
    pub midi_notes: &'a HashMap<ClipId, Vec<MidiNote>>,
    /// The target's plugin state blobs per instance id
    /// (`LoadedProject::plugin_states`), which the `ProjectFile` does not
    /// carry either. Read by the entity domains' plugin chains.
    pub plugin_states: &'a HashMap<PluginInstanceId, Arc<[u8]>>,
    /// Live state an undo keeps, captured by the entry point before
    /// anything is restored.
    pub live: LiveCarry<'a>,
}

/// Live state an undo/redo restore keeps but the restore itself would
/// overwrite before the domain that needs it runs (ARCH-01 A-13b). Each
/// entry point captures it at its top — before `replay_loaded_project`
/// takes `io.project_path` and `ComposeSections` resets the derived
/// counter — and hands it to every domain through the ctx.
///
/// Live state a restore does *not* overwrite before its domain runs stays
/// in `Resonance` and is read there under the origin: the freeze statuses
/// (nothing in the replay touches them until `Freeze`) and the reference
/// A/B monitor (`restore_references` takes it out of `r.reference`
/// itself).
#[derive(Debug, Clone, Copy, Default)]
pub struct LiveCarry<'a> {
    /// The live project's `.rproj` path, whose sibling directory holds the
    /// freeze caches an undo retires. The full replay `take()`s
    /// `io.project_path` (the `AllCleared` handler puts it back after), so
    /// it is carried here; the diff path clones it. `None` for an untitled
    /// project.
    pub project_path: Option<&'a Path>,
    /// The derived-clip id counter before the restore, which an undo never
    /// lowers (ARCH-01 A-6). `None` on a disk load.
    pub derived_counter_floor: Option<u64>,
}

impl LiveCarry<'_> {
    /// The derived-counter floor for a restore of `origin`: the live
    /// counter on an undo, none on a disk load.
    pub(crate) fn derived_counter_floor(r: &Resonance, origin: Origin) -> Option<u64> {
        origin.is_undo().then_some(r.compose.next_derived_clip_id)
    }
}

/// One project domain's restore, shared by every [`Origin`].
pub(crate) trait Reconcile {
    /// Stable name, recorded in `io.reconcile_trace`.
    const NAME: &'static str;

    /// Drive the app (and engine) state of this domain to `new`. `old` is
    /// the file the live state was built from on the diff path, `None`
    /// after a `ClearAll`.
    fn reconcile(
        r: &mut Resonance,
        old: Option<&ProjectFile>,
        new: &ProjectFile,
        ctx: &ReconcileCtx<'_>,
    );
}

/// Groups of [`DOMAINS`], declared in the order both paths run them.
/// They used to mark the points in each path's inline code where a group
/// was valid; since A-13f nothing runs between two stages, so they only
/// group the table (collapsing them is A-13j's).
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
];

/// Run every [`DOMAINS`] entry, in table order — every [`Stage`] in
/// sequence. The whole restore on both paths since A-13f: no per-path
/// code is left between two stages.
pub(crate) fn reconcile_all_stages(
    r: &mut Resonance,
    old: Option<&ProjectFile>,
    new: &ProjectFile,
    ctx: &ReconcileCtx<'_>,
) {
    for d in DOMAINS {
        r.io.reconcile_trace.push((ctx.origin, d.name));
        (d.run)(r, old, new, ctx);
    }
}

/// The table as `(stage, name)`, for the order guard test.
pub fn domain_order() -> Vec<(Stage, &'static str)> {
    DOMAINS.iter().map(|d| (d.stage, d.name)).collect()
}
