//! `global_*` — the song-wide tempo and time-signature tracks (ba doc
//! #286): reading both tracks, and writing changes onto them at a bar.

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::methods::global;
use resonance_control::MutationAck;

#[tool_router(router = router_global, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Every tempo change and every meter change in the song, each with the \
                       1-based bar it takes effect at. Read-only. \
                       \
                       THIS, OR song_summary's tempo_events / signature_events, IS HOW YOU SEE \
                       WHETHER THE SONG CHANGES TEMPO OR METER — the two carry the same two \
                       lists, so a summary you have already read answers it with no extra \
                       call. What never answers it is song_summary's tempo_bpm and \
                       time_signature: those are the values AT THE PLAYHEAD, not for the song, \
                       so on a song that speeds up in the chorus or drops into 7/8 for a \
                       bridge they are a confident, wrong answer and every bar-to-time \
                       calculation built on them inherits the error. Read the event lists \
                       before working out where anything sits in time, before writing drums in \
                       an odd metre, and before assuming a bar is four beats long. \
                       \
                       Both lists come back sorted by bar and are never empty: bar 1 is the \
                       song's initial tempo and initial meter, always present, and cannot be \
                       removed. A song with no changes therefore reports exactly one entry in \
                       each list — length 1 in both means \"steady tempo, one meter throughout\". \
                       \
                       Each tempo entry is {bar, bpm} and holds until the next entry; each \
                       signature entry is {bar, numerator, denominator} with the denominator \
                       resolved (8 for 7/8, not an exponent). Events are addressed by BAR \
                       everywhere in this namespace, never by position in the list, because \
                       both lists re-sort whenever one is edited.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<global::GlobalEvents>()
    )]
    async fn global_list_events(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(global::LIST_EVENTS, &()).await
    }

    #[tool(
        description = "Put a tempo change on the song's tempo track at a bar — the way to make \
                       the song speed up for the chorus or drop back for the outro (undoable \
                       edit, exactly like doing it on the global-tracks shelf by hand). \
                       transport_set_tempo cannot do this: it rewrites only the tempo the song \
                       STARTS at, leaving every later change standing. \
                       \
                       UPSERT BY BAR: a bar carries at most one tempo event. Adding at a bar \
                       that already has one REPLACES its BPM — it never leaves two events on \
                       one bar — so calling twice at bar 33 gives you the second value and one \
                       event there, and re-sending a call you are unsure landed is safe. \
                       \
                       bar is 1-based. Bar 1 is the song's initial tempo, always present and \
                       never removable; adding there rewrites it, which is what \
                       transport_set_tempo does. bpm must be 20..=300; outside that the call is \
                       refused and nothing changes, rather than being quietly clamped to a \
                       tempo you did not ask for. \
                       \
                       WHAT IT DOES NOT DO: existing material is not re-anchored. Clips, \
                       markers and automation keep their position in TIME, so everything after \
                       this bar now falls on a different bar than it did before. Put the tempo \
                       map in place BEFORE laying out the material that follows it, or expect \
                       to move that material afterwards. (transport_set_tempo is the exception \
                       — a change to the song's starting tempo does re-anchor the arrangement, \
                       keeping clips on their bars.) \
                       \
                       You cannot hear the result, so read it back with global_list_events (or \
                       song_summary's tempo_events) to confirm the track is what you intended.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn global_add_tempo_event(
        &self,
        Parameters(params): Parameters<global::AddTempoEventParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(global::ADD_TEMPO_EVENT, &params)
            .await
    }

    #[tool(
        description = "Put a meter (time-signature) change on the song's signature track at a \
                       bar — the way to write a 7/8 bridge, a 6/8 middle eight, or a single \
                       2/4 bar of turnaround (undoable edit, exactly like doing it on the \
                       global-tracks shelf by hand). transport_set_time_signature cannot do \
                       this: it rewrites only the meter the song STARTS in and leaves every \
                       later change standing. \
                       \
                       UPSERT BY BAR: a bar carries at most one meter event. Adding at a bar \
                       that already has one REPLACES it — it never leaves two events on one \
                       bar — so a second call at bar 17 corrects the first instead of stacking \
                       on it, and re-sending a call you are unsure landed is safe. \
                       \
                       bar is 1-based. Bar 1 is the song's initial meter, always present and \
                       never removable; adding there rewrites it. numerator is 1..=32; \
                       denominator is the note value that gets the beat, RESOLVED and a power \
                       of two in 1..=32 — 8 for 7/8, not the exponent 3. Anything else is \
                       refused and nothing changes. \
                       \
                       A meter change makes the bars after it a different length, and existing \
                       material is not re-anchored: clips, markers and automation keep their \
                       position in TIME, so everything after this bar now falls on a different \
                       bar than it did before. Write the meter map first, then the parts — \
                       and note that bar/beat positions you send to other tools are read \
                       against this map, so a drum pattern written for bar 17 in 7/8 needs the \
                       7/8 event to exist first. \
                       \
                       You cannot hear the result, so read it back with global_list_events (or \
                       song_summary's signature_events) to confirm the track is what you \
                       intended.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<MutationAck>()
    )]
    async fn global_add_signature_event(
        &self,
        Parameters(params): Parameters<global::AddSignatureEventParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(global::ADD_SIGNATURE_EVENT, &params)
            .await
    }
}
