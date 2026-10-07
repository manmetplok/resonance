//! The [`Stage`]s that group the reconcile table ([`super::DOMAINS`]).

/// Groups of [`DOMAINS`](super::DOMAINS), declared in the order they run.
/// They used to mark the points in each restore path's inline code where
/// a group was valid; since A-13f nothing runs between two stages, so
/// they only group the table and document why it is in the order it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Transport / master scalars, the transient UI a full replay resets,
    /// compose sections, pronunciation and the drum-pattern bank. The
    /// first thing both paths restore (the full path after
    /// `SetProjectDir`).
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
