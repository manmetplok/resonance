//! `pool_*` / `clip_*` — placing audio samples on the timeline and
//! editing the placement.

use crate::server::ResonanceMcp;
use resonance_control::job::JobStatus;
use resonance_control::methods::{arrangement, clip, pool, reference};
use resonance_control::MutationAck;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};

/// How long to block on an import job before reporting it still running.
/// Importing decodes, resamples and copies the file, so a long stereo
/// take costs real seconds; a drum one-shot returns immediately.
const IMPORT_WAIT_MS: u64 = 120_000;

#[tool_router(router = router_clip, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "List the project's media pool: the audio files that have been imported and \
                       can be placed as clips. Returns per asset {id, original_path, name, \
                       duration_frames, duration_seconds, channels, source_sample_rate, format, \
                       usage_count, missing}. \
                       \
                       Read this before placing a sample you have used before: passing the \
                       returned id to clip_place skips the import entirely (no decode, no \
                       resample, no second copy in the project folder), which is what you want \
                       when the same one-shot is placed many times. usage_count is how many \
                       clips already play the asset. missing: true means the backing file is \
                       gone from disk — the asset is kept so it can be relinked in the GUI, but \
                       placing from it produces silence. \
                       \
                       The pool is per project and empty in a fresh one. This describes the OPEN \
                       project, so with nothing open it answers busy rather than an empty list.",
        annotations(read_only_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<pool::PoolView>()
    )]
    async fn pool_list(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(pool::LIST, &()).await
    }

    #[tool(
        description = "Import audio files into the project's media pool WITHOUT placing them on \
                       any track. Takes absolute paths (up to 64 per call) to files on the \
                       machine running the app; each is decoded, mixed to stereo, resampled to \
                       the project rate and copied into the project's audio/ folder. \
                       \
                       Use this only to pre-load samples you will place later — clip_place \
                       imports on its own, so importing first is never required. Runs as a job \
                       (waits up to 2 minutes) whose result carries {assets} in the same shape \
                       pool_list returns, so the asset ids come straight back. \
                       \
                       The project must have been SAVED at least once: imported audio lives \
                       inside the project folder, so an unsaved project is refused as busy — \
                       call project_save_as first. A file that fails to decode fails the whole \
                       job, but the batch's other files still land in the pool (check pool_list). \
                       Importing the same path twice creates a SECOND asset; match on \
                       original_path in pool_list to avoid that.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn pool_import(
        &self,
        Parameters(params): Parameters<pool::ImportParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(pool::IMPORT, &params, IMPORT_WAIT_MS).await
    }

    #[tool(
        description = "Place a sample as an audio clip on an audio track — the API equivalent of \
                       dragging a file onto the timeline. Name the sample EITHER by asset_id \
                       (already in the pool, see pool_list) OR by path (an absolute path, \
                       imported on the way); both or neither is rejected. A path that matches an \
                       asset's original_path reuses that asset instead of importing it twice. \
                       \
                       track_id must take audio: an AUDIO track (track_add with kind \"audio\") \
                       or an EXTERNAL-INSTRUMENT track, whose sound is outboard so what lands on \
                       it is recorded audio. A plain instrument, drums or vocal track is refused, \
                       because an audio clip on one would never render. start takes bar [+ beat] \
                       (1-based, beat in the time signature's unit) or sample, and \
                       defaults to bar 1 — it is NOT snapped to the grid, so the clip lands \
                       exactly where you say. The clip is named after the file's stem. \
                       \
                       Runs as a job (waits up to 2 minutes) whose result carries {clip_id, \
                       track_id, asset_id, start, length_beats, length_samples, name} — so the \
                       clip id comes straight back and no follow-up song_tracks is needed. \
                       Placing an already-pooled asset completes instantly; a fresh path waits \
                       out the import. The project must have been saved at least once for the \
                       path form (see pool_import). \
                       \
                       Placement and import together are ONE undo entry, so edit_undo removes \
                       both the clip and the asset it brought in.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn clip_place(
        &self,
        Parameters(params): Parameters<clip::PlaceParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(clip::PLACE, &params, IMPORT_WAIT_MS).await
    }

    #[tool(
        description = "Move an audio clip to a new position, and optionally to another audio \
                       track. start takes bar [+ beat] (1-based, beat in the time signature's \
                       unit — an eighth in 6/8) or sample, with no grid \
                       snapping. Omit track_id to move it in time only; a track_id that is not \
                       an audio track is refused. \
                       \
                       clip_id comes from song_tracks — audio clips are the ones with \
                       midi: false. Passing a MIDI clip is refused with a pointer to notes.*; \
                       use notes_move_clip for those. Verify with song_tracks.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn clip_move(
        &self,
        Parameters(params): Parameters<clip::MoveParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(clip::MOVE, &params).await
    }

    #[tool(
        description = "Trim an audio clip: choose which part of the source file it plays. \
                       start_offset hides that much of the source's HEAD, end_offset that much \
                       of its TAIL; both are amounts, given as exactly one of {beats}, {seconds} \
                       or {samples} — e.g. {\"end_offset\": {\"seconds\": 2.0}}. Use seconds or \
                       samples for material that was not cut to this project's tempo; beats \
                       converts against the tempo map at the clip's position. \
                       \
                       Trimming is non-destructive (the imported file is untouched) and \
                       reversible — pass 0 to restore an edge. Omitted fields keep their current \
                       value, and offsets are clamped so at least one frame stays audible. \
                       \
                       Trimming the head does NOT move the clip: the audible part starts later \
                       on the timeline. Pass start as well (bar [+ beat] or sample) to keep it \
                       where it was. Returns the geometry that actually landed after clamping: \
                       {start, start_offset_samples, end_offset_samples, length_samples, \
                       length_beats}.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<clip::TrimResult>()
    )]
    async fn clip_trim(
        &self,
        Parameters(params): Parameters<clip::TrimParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(clip::TRIM, &params).await
    }

    #[tool(
        description = "Remove an audio clip from the timeline. Destructive, so it follows the \
                       confirm convention: without confirm: true it is refused with a summary \
                       of what would be lost (name, track, bar, length) — re-send with \
                       confirm: true to proceed. The pool asset it played is NOT \
                       removed — it stays importable and its usage_count drops, so the same \
                       sample can be placed again with clip_place without re-importing. \
                       Undoable with edit_undo. Passing a MIDI clip is refused; those are \
                       deleted through the notes.* surface.",
        annotations(destructive_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn clip_delete(
        &self,
        Parameters(params): Parameters<clip::DeleteParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(clip::DELETE, &params).await
    }

    #[tool(
        description = "Set one audio clip's gain in decibels; 0.0 is unity, negative attenuates. \
                       Clamped to +24 dB. \
                       \
                       This is per CLIP, before the track's fader — reach for it to balance one \
                       sample against the others playing with it, not to set the part's level in \
                       the mix (that is mixer_set_volume_db on the track). Both apply; they \
                       multiply.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn clip_set_gain(
        &self,
        Parameters(params): Parameters<clip::SetGainParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(clip::SET_GAIN, &params).await
    }

    #[tool(
        description = "Set an audio clip's fade-in / fade-out lengths and shapes. fade_in and \
                       fade_out are amounts — exactly one of {beats}, {seconds} or {samples}, \
                       e.g. {\"fade_in\": {\"seconds\": 0.01}} for a 10 ms click-killer. Shapes \
                       are linear, equal_power (default, the constant-power ramp) or exp. \
                       \
                       Every field is optional and omitted ones keep their current value, so \
                       shapes can be changed without touching lengths; a call that sets nothing \
                       is rejected. Lengths are clamped to the clip's audible length, and the \
                       result reports what actually landed: {fade_in_samples, fade_out_samples, \
                       fade_in_shape, fade_out_shape}. Pass 0 to remove a fade.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<clip::FadeResult>()
    )]
    async fn clip_set_fade(
        &self,
        Parameters(params): Parameters<clip::SetFadeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(clip::SET_FADE, &params).await
    }

    #[tool(
        description = "Cut an audio clip in two at a timeline position — the missing half of \
                       arrangement editing. The original keeps its id and ends at the cut; the \
                       new clip plays from the cut onwards. Both are non-destructive trims of \
                       the same audio, so nothing is copied and edit_undo restores the single \
                       clip exactly. \
                       \
                       `at` is a POSITION on the timeline ({bar, beat} with beat in the time \
                       signature's unit, or an absolute {sample}; there is no seconds form), \
                       not an offset into the source, and must fall strictly inside the clip — \
                       a cut at either edge is refused rather than making an empty half. The \
                       head keeps the fade-in, the tail the fade-out. Returns {head_clip_id, \
                       tail_clip_id, head_length_samples, tail_length_samples}; both resolve \
                       immediately in song_tracks. \
                       \
                       With clip_place (which accepts external-instrument tracks), this is what \
                       lets a recorded hardware take be cut, copied and re-placed.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<clip::SplitResult>()
    )]
    async fn clip_split(
        &self,
        Parameters(params): Parameters<clip::SplitParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(clip::SPLIT, &params).await
    }

    #[tool(
        description = "Insert empty bars, moving EVERYTHING that starts at or after at_bar later \
                       — audio clips, MIDI clips, section placements, markers and automation \
                       points — in one undoable edit. This is how you give a section more room \
                       or make space for a new one; doing it by hand means moving every object \
                       after the cut without missing one. \
                       \
                       at_bar is 1-based. Anything that STARTS before at_bar stays put, even if \
                       it plays across the insertion point (a clip is never stretched). \
                       Positions move musically, so a project with tempo changes lands on the \
                       right beat. Tempo and time-signature changes move with the music they \
                       were written for; the song's opening tempo/meter at bar 1 stays, so the \
                       new bars take it. Returns what moved: {shift_samples, audio_clips_moved, \
                       midi_clips_moved, placements_moved, markers_moved, \
                       automation_points_moved, tempo_events_moved, signature_events_moved}. \
                       shift_samples is measured after the edit: moving a tempo change stretches \
                       its ramp, so it is not always count x the old bar length.",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<arrangement::ShiftResult>()
    )]
    async fn arrangement_insert_bars(
        &self,
        Parameters(params): Parameters<arrangement::InsertBarsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(arrangement::INSERT_BARS, &params)
            .await
    }

    #[tool(
        description = "Remove bars, pulling everything after them earlier — the inverse of \
                       arrangement_insert_bars, and the way to cut a section out of a song. \
                       \
                       Anything that STARTS inside the removed bars is DELETED, so the call is \
                       refused with a count of what would go unless confirm: true. Clips that \
                       start before the cut are left alone and keep their length. A tempo or \
                       time-signature change INSIDE the removed bars is not deleted: it is \
                       clamped onto the cut so the music after the splice keeps the tempo and \
                       meter it was written in, and if that lands two of a kind on one bar the \
                       later one wins (reported as tempo_events_removed / \
                       signature_events_removed). Returns the same tally as insert_bars plus \
                       clips_deleted and placements_deleted.",
        annotations(destructive_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<arrangement::ShiftResult>()
    )]
    async fn arrangement_remove_bars(
        &self,
        Parameters(params): Parameters<arrangement::RemoveBarsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(arrangement::REMOVE_BARS, &params)
            .await
    }

    #[tool(
        description = "Load a REFERENCE TRACK — a commercial master the user supplied to compare \
                       the mix against — onto the project's A/B reference list, from the media \
                       pool: import the file with pool_import first, then pass its \
                       pool_asset_id from pool_list. It appears in the GUI's reference panel, \
                       where the user can audition it against the mix, and is saved with the \
                       project; undoable. \
                       \
                       Returns reference_id. Measure the reference with meter_measure and \
                       target {reference: reference_id}: every figure and detail block a mix \
                       slice gets (spectrum tilt, per-band stereo width, PLR/PSR, ...), measured \
                       from the same pooled file a clip of it would play, so the numbers \
                       compare directly with a meter_measure of your master. It decodes in the \
                       background: a measurement asked for in the first moment answers busy — \
                       retry. To have the mastering assistant target it, use master_assist \
                       with {mode: \"reference\", pool_asset_id} (it needs no load).",
        annotations(destructive_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<reference::LoadResult>()
    )]
    async fn reference_load(
        &self,
        Parameters(params): Parameters<reference::LoadParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(reference::LOAD, &params).await
    }
}
