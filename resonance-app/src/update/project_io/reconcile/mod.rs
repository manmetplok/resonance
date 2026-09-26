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

use std::path::Path;

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
    /// Tempo events, chord track, markers. Full path: inside
    /// `replay_globals`, after `SetBpm` and before the chord trim (which
    /// reads the meter). Diff path: after external instruments.
    Timeline,
    /// App-side content restored whole. Full path: after the clips (the
    /// pool counts their asset refs) and references. Diff path: right
    /// after `Timeline`.
    Content,
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
    // Before the chord trim and every clip (derived-clip bar recovery
    // reads the tempo map).
    domain::<app_side::TempoEvents>(Stage::Timeline),
    domain::<app_side::ChordTrack>(Stage::Timeline),
    domain::<app_side::Markers>(Stage::Timeline),
    // After the clips: the pool counts their asset refs. The full path's
    // order.
    domain::<app_side::Pool>(Stage::Content),
    domain::<app_side::Quantize>(Stage::Content),
    domain::<app_side::Performance>(Stage::Content),
    domain::<app_side::TrackGroups>(Stage::Content),
    domain::<app_side::TakeGroups>(Stage::Content),
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
