//! Round-robin display state shared between the audio thread and the editor.
//!
//! The sampler publishes which take it fired, and how many takes that
//! velocity layer holds, into one `AtomicU32` per pad (`KitBridge::last_rr`).
//! Both halves matter to a user auditioning a kit — "take 2 of 3" says the
//! pad is cycling, "take 1 of 1" says it will repeat the same sample every
//! hit — so the packing and unpacking live here rather than being written
//! in the sampler and thrown away at the reader (ba todo #1329).

/// Highest index or count the packing can carry; both halves are 16 bits.
pub const MAX_PACKED: usize = u16::MAX as usize;

/// Which round-robin take last fired for a pad, and how many that layer has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundRobin {
    /// Zero-based index of the take that fired.
    pub take_index: usize,
    /// Number of takes in the velocity layer it came from. Always >= 1.
    pub take_count: usize,
}

impl RoundRobin {
    /// One-based "take 2 of 3" label for the pad inspector.
    pub fn label(&self) -> String {
        format!("take {} of {}", self.take_index + 1, self.take_count)
    }

    /// Compact "2/3" form for the pad list, where a row is 22 px tall.
    pub fn compact(&self) -> String {
        format!("{}/{}", self.take_index + 1, self.take_count)
    }

    /// Whether this pad has more than one take to cycle through.
    pub fn cycles(&self) -> bool {
        self.take_count > 1
    }
}

/// Pack an index/count pair for publication by the audio thread.
///
/// The sentinel `0` means "this pad has never been triggered", which is
/// why the count is stored in the high half: take 1 of 1 packs as
/// `1 << 16`, not as zero.
pub fn pack(take_index: usize, take_count: usize) -> u32 {
    let index = take_index.min(MAX_PACKED) as u32;
    let count = take_count.min(MAX_PACKED) as u32;
    index | (count << 16)
}

/// Unpack a published value. `None` means the pad has not been triggered
/// since the plugin (or the kit) was loaded — never "take 0 of 0".
pub fn unpack(raw: u32) -> Option<RoundRobin> {
    if raw == 0 {
        return None;
    }
    let take_count = (raw >> 16) as usize;
    if take_count == 0 {
        return None;
    }
    let take_index = (raw & 0xFFFF) as usize;
    Some(RoundRobin {
        take_index: take_index.min(take_count - 1),
        take_count,
    })
}
