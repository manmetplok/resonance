//! `global_*` — the song-wide tempo and time-signature tracks (ba doc
//! #286). Read-only for now; the mutators land in the following slices.

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::methods::global;

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
}
