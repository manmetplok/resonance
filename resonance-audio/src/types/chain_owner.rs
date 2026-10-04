//! Which insert chain a plugin sits on (code review ARCH2-02).
//!
//! A track, a bus and the master each own an insert chain, and every
//! chain edit — add, remove, reorder, whole-chain bypass — means the
//! same thing on all three. Before this type existed the engine API spelt
//! each edit out three times (`AddPlugin` / `AddPluginToBus` /
//! `AddPluginToMaster`, and the matching events), so every chain
//! behaviour was written, tested and fixed per surface — and fixed on
//! two of them, missed on the third. Now one command and one event carry
//! the owner, and the app keeps one owner type too (its `PluginLocator`
//! is this type).

use super::{BusId, TrackId};

/// The chain a plugin instance belongs to: a track's, a bus's, or the
/// master's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChainOwner {
    Track(TrackId),
    Bus(BusId),
    Master,
}

impl ChainOwner {
    /// The track id, for a track chain.
    pub fn track_id(self) -> Option<TrackId> {
        match self {
            Self::Track(id) => Some(id),
            _ => None,
        }
    }

    /// The bus id, for a bus chain.
    pub fn bus_id(self) -> Option<BusId> {
        match self {
            Self::Bus(id) => Some(id),
            _ => None,
        }
    }

    pub fn is_master(self) -> bool {
        matches!(self, Self::Master)
    }
}

/// The owner as a sentence fragment: "track 3", "bus 7", "the master".
impl std::fmt::Display for ChainOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Track(id) => write!(f, "track {id}"),
            Self::Bus(id) => write!(f, "bus {id}"),
            Self::Master => f.write_str("the master"),
        }
    }
}
