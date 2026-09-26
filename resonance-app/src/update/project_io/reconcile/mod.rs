//! The `Reconcile` driver (ARCH-01 A-13, design `docs/design/A-13-reconcile.md`).
//!
//! A project reaches the app through two restore paths: the full replay
//! after `ClearAll` (`replay_loaded_project` — a disk load or an undo's
//! structural fallback) and the undo diff replay (`try_diff_replay`). A
//! domain migrated here is restored by one [`Reconcile`] impl that both
//! paths run through [`reconcile_stage`], in the one order [`DOMAINS`]
//! lists.
//!
//! Domains not yet migrated are still restored inline by each path. The
//! [`Stage`]s mark the points in that inline code where a group of
//! migrated domains is valid; both paths call the stages in the same
//! sequence, and `DOMAINS` is sorted by stage, so both run the migrated
//! domains in table order.

mod app_side;
mod clips;
mod globals;
mod restored;

use std::collections::HashMap;
use std::path::Path;

use resonance_audio::types::{ClipId, MidiNote};

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

/// Points in the not-yet-migrated inline code at which a group of
/// migrated domains runs. Declared in the order both paths reach them.
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
    /// The audio and MIDI clips, then state derived from them: the lyric
    /// side-table and the derived-clip map. Full path: right after the
    /// tracks, busses, master and routing are replayed. Diff path: right
    /// after the plugin params are applied.
    Clips,
    /// App-side content restored whole, and the references. Full path:
    /// after the plugin chains are finalised (the pool counts the clips'
    /// asset refs). Diff path: after `Clips`.
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
    // The clips themselves, after every track they sit on (and, diff
    // path, the tempo map — no clip command reads it). Audio before MIDI,
    // as both paths always had it.
    domain::<clips::AudioClips>(Stage::Clips),
    domain::<clips::MidiClips>(Stage::Clips),
    // After the MIDI and audio clips they read / filter against.
    domain::<globals::ClipLyrics>(Stage::Clips),
    domain::<restored::DerivedClips>(Stage::Clips),
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

/// Run every [`DOMAINS`] entry of `stage`, in table order.
pub(crate) fn reconcile_stage(
    r: &mut Resonance,
    stage: Stage,
    old: Option<&ProjectFile>,
    new: &ProjectFile,
    ctx: &ReconcileCtx<'_>,
) {
    for d in DOMAINS.iter().filter(|d| d.stage == stage) {
        r.io.reconcile_trace.push((ctx.origin, d.name));
        (d.run)(r, old, new, ctx);
    }
}

/// The table as `(stage, name)`, for the order guard test.
pub fn domain_order() -> Vec<(Stage, &'static str)> {
    DOMAINS.iter().map(|d| (d.stage, d.name)).collect()
}
