//! Id newtypes.
//!
//! These mirror the app's real `u64` ids verbatim (project model:
//! tracks, clips, section definitions/placements, chords, markers), so
//! introspection output (`song.*`) feeds directly into mutation params.
//! All serialize as plain JSON numbers.

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u64);

        impl From<u64> for $name {
            fn from(raw: u64) -> Self {
                Self(raw)
            }
        }

        impl From<$name> for u64 {
            fn from(id: $name) -> u64 {
                id.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

id_type!(
    /// A track id (`ProjectTrack.id`).
    TrackId
);
id_type!(
    /// An audio or MIDI clip id (`ProjectClip.id` / `ProjectMidiClip.id`).
    ClipId
);
id_type!(
    /// A section definition id (`ProjectSectionDefinition.id`).
    SectionDefinitionId
);
id_type!(
    /// A section placement id (`ProjectSectionPlacement.id`).
    SectionPlacementId
);
id_type!(
    /// A chord id within a section definition (`ProjectSectionChord.id`).
    ChordId
);
id_type!(
    /// A timeline marker id.
    MarkerId
);
id_type!(
    /// A note id within a MIDI clip, where the app assigns one. Notes
    /// without stable ids are addressed by index (see `notes.*`).
    NoteId
);
id_type!(
    /// A control-endpoint job id (see [`crate::job`]). Allocated by the
    /// app, monotonic per app run.
    JobId
);
