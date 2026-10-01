//! Roadmap group (4): global transport scalars, the transient UI a full
//! replay resets, compose sections and the drum-pattern bank, the
//! meter-dependent section chord trim, and the vocal lyric side-table
//! (ARCH-01 A-13c).
//!
//! The first domains that read `old`: [`Transport`] sends every scalar
//! after a `ClearAll` (`old = None`) and only the changed ones on the diff
//! path, exactly as `replay_globals` / `apply_global` did.

use resonance_audio::types::AudioCommand;

use super::{Reconcile, ReconcileCtx};
use crate::project::ProjectFile;
use crate::update::project_io::replay::restore_drum_patterns;
use crate::util::db_to_gain;
use crate::Resonance;

/// True when `field` differs between the live file and the target, or
/// when there is no live file (after a `ClearAll`: restore everything).
fn differs<T: PartialEq>(
    old: Option<&ProjectFile>,
    new: &ProjectFile,
    field: impl Fn(&ProjectFile) -> T,
) -> bool {
    old.is_none_or(|a| field(a) != field(new))
}

/// Transport and master scalars: BPM, time signature, metronome, master
/// volume, MIDI clock in/out, loop range. Each is set app-side and sent to
/// the engine when it changed (every one after a `ClearAll`).
///
/// Before `Timeline`: `TempoEvents` rebuilds the app tempo map from
/// `transport.bpm` / `time_sig_*`, and the engine's `SetBpm` must precede
/// `SetTempoEvents` (both write the map's `bpm`; the events' first point
/// wins). `SetTimeSignature` writes the map's fallback meter, which the bar
/// table reads only when there are no signature events — and
/// `restore_tempo_events` always installs at least one — so it commutes
/// with `SetTempoEvents`.
///
/// After a `ClearAll` the playhead goes back to 0. `loop_range_set`
/// follows `loop_enabled` whenever the loop is restored (always after a
/// `ClearAll`; on the diff path only when the loop changed, so a live range
/// the user set with the loop off survives an unrelated undo).
pub(crate) struct Transport;

impl Reconcile for Transport {
    const NAME: &'static str = "transport";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        if ctx.origin.after_clear_all() {
            r.transport.playhead = 0;
        }
        if differs(old, new, |f| f.bpm) {
            r.transport.bpm = new.bpm;
            let _ = r.engine.send(AudioCommand::SetBpm { bpm: new.bpm });
        }
        if differs(old, new, |f| (f.time_sig_num, f.time_sig_den)) {
            r.transport.time_sig_num = new.time_sig_num;
            r.transport.time_sig_den = new.time_sig_den;
            let _ = r.engine.send(AudioCommand::SetTimeSignature {
                numerator: new.time_sig_num,
                denominator: new.time_sig_den,
            });
        }
        if differs(old, new, |f| f.metronome_enabled) {
            r.transport.metronome_enabled = new.metronome_enabled;
            let _ = r.engine.send(AudioCommand::SetMetronomeEnabled {
                enabled: new.metronome_enabled,
            });
        }
        if differs(old, new, |f| f.master_volume) {
            r.master.volume = new.master_volume;
            let _ = r.engine.send(AudioCommand::SetMasterVolume {
                volume: db_to_gain(new.master_volume),
            });
        }
        // The engine treats `enabled = false` as a no-op port-wise, so this
        // is safe for legacy projects.
        if differs(old, new, |f| {
            (f.midi_clock_send_enabled, f.midi_clock_send_device.clone())
        }) {
            r.devices.midi.midi_clock_send_enabled = new.midi_clock_send_enabled;
            r.devices.midi.midi_clock_send_device = new.midi_clock_send_device.clone();
            let _ = r.engine.send(AudioCommand::SetMidiClockOutput {
                device: new.midi_clock_send_device.clone(),
                enabled: new.midi_clock_send_enabled,
            });
        }
        if differs(old, new, |f| {
            (f.midi_clock_recv_enabled, f.midi_clock_recv_device.clone())
        }) {
            r.devices.midi.midi_clock_recv_enabled = new.midi_clock_recv_enabled;
            r.devices.midi.midi_clock_recv_device = new.midi_clock_recv_device.clone();
            let _ = r.engine.send(AudioCommand::SetMidiClockInput {
                device: new.midi_clock_recv_device.clone(),
                enabled: new.midi_clock_recv_enabled,
            });
        }
        if differs(old, new, |f| (f.loop_enabled, f.loop_in, f.loop_out)) {
            r.transport.loop_enabled = new.loop_enabled;
            r.transport.loop_in = new.loop_in;
            r.transport.loop_out = new.loop_out;
            r.transport.loop_range_set = new.loop_enabled;
            let _ = r.engine.send(AudioCommand::SetLoopRange {
                enabled: new.loop_enabled,
                loop_in: new.loop_in,
                loop_out: new.loop_out,
            });
        }
    }
}

/// Transient UI state a replay after `ClearAll` resets so the project
/// starts clean: the selected clip, the plugin window and focused slot,
/// the master selection, an in-flight clip drag or trim, and the
/// delete-track / quit confirmations. An undo keeps them —
/// what names an entity it removes is pruned by the removal domains
/// (A-13h/i). The scroll position is not reset here: an undo must not move
/// the view, and a disk load scrolls in `all_cleared` (FU-V3a).
pub(crate) struct TransientUi;

impl Reconcile for TransientUi {
    const NAME: &'static str = "transient_ui";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, _: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        if !ctx.origin.after_clear_all() {
            return;
        }
        r.ui.interaction.selected_clip = None;
        r.ui.mixer.plugin_window = None;
        r.ui.mixer.focused_slot = None;
        // The master outlives a load, but its selection must not: the new
        // project opens with no channel in the inspector, as it does with
        // no track or bus selected.
        r.ui.mixer.selected_master = false;
        // A strip rename names a track of the old project.
        r.ui.mixer.renaming = None;
        r.ui.interaction.clip_drag = None;
        r.ui.interaction.clip_trim = None;
        r.modals.confirm_delete_track = None;
        r.modals.confirm_quit = None;
    }
}

/// Compose section definitions and placements (drum arrangements included,
/// from `ProjectSectionDefinition::arrangement`), through
/// `ComposeState::load_from_project`, which also clears runtime-only
/// sub-state — the derived-clip map and counter among it. Before `Clips`:
/// `DerivedClips` restores both afterwards, the counter from the floor the
/// ctx carries (ARCH-01 A-6).
pub(crate) struct ComposeSections;

impl Reconcile for ComposeSections {
    const NAME: &'static str = "compose_sections";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, _: &ReconcileCtx<'_>) {
        r.compose
            .load_from_project(&new.section_definitions, &new.section_placements);
    }
}

/// The drum-pattern bank, through [`restore_drum_patterns`] (legacy
/// `drum_groups` promoted; after `ComposeSections`, whose definitions the
/// promotion points at the new bank).
///
/// **Origin rule kept.** A file with neither field keeps the seeded default
/// bank after a `ClearAll` (a project that predates drum groups), while the
/// diff path clears the bank to mirror the snapshot exactly
/// (`clear_on_empty`). After a `ClearAll` the drum-roll focus is also
/// pointed at the first pattern; the diff path leaves the focus alone.
pub(crate) struct DrumPatterns;

impl Reconcile for DrumPatterns {
    const NAME: &'static str = "drum_patterns";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        let after_clear_all = ctx.origin.after_clear_all();
        restore_drum_patterns(&mut r.compose, new, !after_clear_all);
        if !after_clear_all {
            return;
        }
        let first_group_id = r
            .compose
            .drum_patterns
            .first()
            .and_then(|p| p.groups.first().map(|g| g.id));
        r.compose.drumroll.selected_group_id = first_group_id;
        r.compose.drumroll.managing_group_id = first_group_id;
        r.compose.drumroll.managing_pattern_id = r.compose.default_drum_pattern_id;
    }
}

/// Chords past a section's end are refused by every edit; a file is held
/// to the same once the meter is known (code review FU-V4b) — hence in
/// `Timeline`, after `TempoEvents`. Only after a `ClearAll`: a diff
/// restore's target is an undo snapshot of live state, which every edit
/// already kept trimmed, and trimming it would make the restore differ
/// from the snapshot.
pub(crate) struct SectionChordTrim;

impl Reconcile for SectionChordTrim {
    const NAME: &'static str = "section_chord_trim";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, _: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        if !ctx.origin.after_clear_all() {
            return;
        }
        let trimmed = crate::update::compose::trim_chords_to_sections(r);
        if !trimmed.is_empty() {
            tracing::warn!("trimmed chords past the end of section(s) {trimmed:?} on load");
        }
    }
}

/// The vocal lyric side-table, from `ProjectMidiClip::vocal_lyrics`,
/// installed in its live form: padded to the restored clip's note count,
/// no entry for an empty saved vec (legacy projects, non-vocal clips).
/// After the MIDI clips (whose note counts it pads to), before
/// `DerivedClips`. The same body on every origin.
pub(crate) struct ClipLyrics;

impl Reconcile for ClipLyrics {
    const NAME: &'static str = "clip_lyrics";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, _: &ReconcileCtx<'_>) {
        let note_counts: std::collections::HashMap<u64, usize> =
            r.midi_clips.iter().map(|mc| (mc.id, mc.notes.len())).collect();
        r.compose.vocal_audio.clip_lyrics.clear();
        for pmc in &new.midi_clips {
            let note_count = note_counts.get(&pmc.id).copied().unwrap_or(0);
            r.compose
                .vocal_audio
                .restore_clip_lyrics(pmc.id, &pmc.vocal_lyrics, note_count);
        }
    }
}
