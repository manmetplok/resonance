//! `vocal_*` — lyrics, pronunciation overrides, and SVS rendering.
//!
//! Every lane-addressed tool here needs a vocal lane on the track first;
//! `section_set_lane_generator` with kind `vocal` is what creates one
//! (ba doc #268).

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::job::JobStatus;
use resonance_control::methods::vocal;

/// SVS rendering is heavy (neural synthesis); wait longer than for
/// project I/O before handing back the job id.
const VOCAL_RENDER_WAIT_MS: u64 = 120_000;

/// A voicebank download is hundreds of megabytes: wait a while, then hand
/// back the job id to poll.
const VOICEBANK_INSTALL_WAIT_MS: u64 = 120_000;

#[tool_router(router = router_vocal, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Generate a vocal lane: a melody into its MIDI clip and, unless \
                       lyrics: false, a fresh lyric draft from the lane's theme brief. This is \
                       what puts notes on a vocal lane — generate_part refuses vocal tracks, \
                       and vocal_render has nothing to sing without it. Needs a vocal lane \
                       (section_set_lane_generator kind \"vocal\") on a section that already \
                       has chords. section_id picks the lane; seed makes the result \
                       reproducible. Pass lyrics: false to keep lyrics you wrote with \
                       vocal_set_lyrics — the default regenerates them. Returns the clip_id to \
                       inspect with song_notes and edit with notes_*, then vocal_render. The \
                       lane's syllable_mode shapes the melody: syllabic sings one note per \
                       lyric unit; mixed gives some line-final and long stressed syllables 2-3 \
                       notes; melismatic does so more often. Those generator holds read \
                       slur: \"auto\" in song_vocal and are regenerated each time; holds you \
                       want to keep belong in the lyrics as _. When \
                       the lane's voicebank is not installed the melody is still generated \
                       and render_error {kind: voicebank_missing, message} says so; pick an \
                       installed bank (vocal_voicebanks) with vocal_set_params, then \
                       vocal_render.",
        annotations(destructive_hint = false, idempotent_hint = false, open_world_hint = false),
        output_schema = schema_for_output::<vocal::GenerateResult>()
    )]
    async fn vocal_generate(
        &self,
        Parameters(params): Parameters<vocal::GenerateParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(vocal::GENERATE, &params).await
    }

    #[tool(
        description = "Read a vocal lane's full settings: voicebank and speaker, timbre, \
                       vibrato, vibrato_rate, tension, tension_velocity_amount, \
                       tension_contour_amount, portamento_ms, articulation, consonant_emphasis, \
                       rest_threshold_beats, velocity_dynamics, bank_timing, bank_pitch; \
                       the melody generator's voice, range {low, high}, style, contour, \
                       syllable_mode, chord_tone_anchor, leap_range, phrase_length_bars, breath, \
                       stay_in_scale, avoid_clashes, use_section_motif; and the lyric \
                       generator's theme, mood, pov, rhyme, lines, syllables_min, syllables_max, \
                       match_syllables_to_melody, avoid_cliches — plus the lane's seed. \
                       section_id picks the lane (a definition_id from song_vocal's lanes); \
                       omitted, the track's FIRST vocal lane. song_vocal reports the same \
                       params per lane. Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<vocal::ParamsResult>()
    )]
    async fn vocal_get_params(
        &self,
        Parameters(params): Parameters<vocal::GetParamsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(vocal::GET_PARAMS, &params).await
    }

    #[tool(
        description = "Change a vocal lane's settings with a PARTIAL patch: send only the \
                       fields to change in params (the vocal_get_params field names, enums in \
                       snake_case, e.g. style \"pop_ballad\", pov \"first_singular\", voice \
                       \"mezzo_soprano\", range {low: 55, high: 77}); everything else is kept. \
                       voicebank is an id from vocal_voicebanks (an installed bank) or one of \
                       \"tiger\" (a new lane's default, speaker mystic), \"lilia\", \"meiji\"; \
                       any other id, or a bank \
                       that cannot sing English, is refused. Switching it resets speaker to \
                       that bank's default unless the same patch sets speaker (one of the \
                       bank's speakers ids from vocal_voicebanks — tiger: glam, fresh, disco, \
                       royal, electric, mystic, vinyl; meiji: standard, hunter, lilith, \
                       phantom; lilia has one voice and refuses a speaker; effect modes \
                       such as growl, vf (vocal fry) and whisper are accepted). The merged \
                       result is validated before anything \
                       changes and a bad value is rejected naming the field (0..1 sliders, \
                       tension -1..1, vibrato_rate 1..20 Hz, portamento_ms 0..500, \
                       rest_threshold_beats 0.125..4 — a gap between notes of at least that \
                       many beats is sung as silence, a shorter one legato; velocity_dynamics \
                       0..1 — how much note velocity changes loudness, \
                       syllables_min <= syllables_max, range.low < range.high). bank_timing and \
                       bank_pitch (both default true) let the voicebank's own duration and \
                       pitch models time the phonemes and draw the pitch line where it ships \
                       them (vocal_voicebanks models); with bank_pitch the model's own \
                       vibrato and glides replace vibrato, vibrato_rate and portamento_ms, \
                       pitch bend still applies; off, or on a bank without the model, the \
                       rule-based timing / synthesized line is used. One undo step; \
                       returns the full params. Lyrics are NOT here (vocal_set_lyrics / \
                       vocal_set_line). It never renders or regenerates: call vocal_generate \
                       after changing melody/lyric generator fields, vocal_render after \
                       changing voice fields. section_id picks the lane; omitted, the track's \
                       FIRST vocal lane.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<vocal::ParamsResult>()
    )]
    async fn vocal_set_params(
        &self,
        Parameters(params): Parameters<vocal::SetParamsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_structured(vocal::SET_PARAMS, &params).await
    }

    #[tool(
        description = "Replace a vocal track's full lyric text, one line per \\n-separated line. \
                       \
                       Each syllable STARTS ITS OWN NOTE, so the note count the text fills — \
                       not the word count — is what has to match the lane's notes. You may \
                       write breaks yourself with - or · inside a word (\"re-so-lu-tion\", \
                       \"ev·er·y\"): your breaks are exact, each piece is one note, and the \
                       piece count also picks which dictionary pronunciation is sung. By \
                       default (syllabify: true) words you did not break are split for you, \
                       and your own breaks and [..] phoneme blocks are left alone. Pass \
                       syllabify: false only when you want the text stored exactly as typed, \
                       and then expect to place every break yourself — an unbroken \
                       multi-syllable word crams all its phonemes onto one note and is heard \
                       as a smear, not as the word. \
                       \
                       Two standalone markers (separated by spaces): _ holds the previous \
                       syllable onto the next note — a melisma, \"love _ _\" is one syllable \
                       over three notes, its final consonant sung at the end of the hold (+ \
                       is an alias; a spaced - is just a dash and is ignored). Inside a broken \
                       word a _ piece holds the piece before it: \"a·_·ma·zing\". AP (uppercase) \
                       marks a breath before the next word and takes no note; it is sung as an \
                       audible breath at the end of the gap before that word. \
                       \
                       ALWAYS re-read song_vocal after writing. It is the cheapest verification \
                       available: syllable_count vs note_count with counts_mismatch, plus the \
                       per-note phoneme/duration budget that says which notes are too short to \
                       articulate what you gave them. A mismatch otherwise only surfaces as a \
                       bad render. Phonemes are derived automatically (G2P); fix individual \
                       words with vocal_set_pronunciation, then re-render with vocal_render. \
                       \
                       Lyrics live per (section, track) vocal lane, created with \
                       section_set_lane_generator kind \"vocal\": pass section_id (a \
                       definition_id from song_vocal's lanes) to pick one, or omit it to write \
                       the track's FIRST vocal lane.",
        annotations(destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn vocal_set_lyrics(
        &self,
        Parameters(params): Parameters<vocal::SetLyricsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(vocal::SET_LYRICS, &params).await
    }

    #[tool(
        description = "Replace ONE lyric line, addressed by its 0-based line_index from \
                       that lane's lines in song_vocal — the surgical alternative to rewriting \
                       the whole lane with \
                       vocal_set_lyrics. Same syllable rules: each syllable starts its own \
                       note, breaks inside a word may be written as - or · and are exact, a \
                       standalone _ (or +) holds the previous syllable onto one more note, a \
                       standalone AP marks an audible breath and takes no note, and \
                       syllabify (default true) splits the words you did \
                       not break. Like vocal_set_lyrics, section_id picks which \
                       vocal lane (lyrics live per (section, track) lane); omitted it writes the \
                       track's FIRST lane. Re-read song_vocal afterwards to confirm the lane's \
                       counts still line up.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn vocal_set_line(
        &self,
        Parameters(params): Parameters<vocal::SetLineParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(vocal::SET_LINE, &params).await
    }

    #[tool(
        description = "Override how a word is sung: lowercase ARPAbet-style phonemes, e.g. \
                       word \"lilia\", phonemes [\"l\",\"ih\",\"l\",\"iy\",\"ah\"]. Project-wide \
                       and case-insensitive; applies on the next vocal_render.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn vocal_set_pronunciation(
        &self,
        Parameters(params): Parameters<vocal::SetPronunciationParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(vocal::SET_PRONUNCIATION, &params).await
    }

    #[tool(
        description = "Remove a per-word pronunciation override, reverting to automatic G2P.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn vocal_clear_pronunciation(
        &self,
        Parameters(params): Parameters<vocal::ClearPronunciationParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke(vocal::CLEAR_PRONUNCIATION, &params).await
    }

    #[tool(
        description = "List the voicebanks installed on this machine (the user's voicebank \
                       folder, not part of the project, so no project needs to be open): per \
                       bank its id (what vocal_set_params takes as voicebank), name, version, \
                       english (false banks are refused — lyrics are English), speakers (the \
                       ids speaker takes; empty for a single-voice bank; effect: true marks an \
                       effect mode — growl, vocal fry, whisper — which is a colour for a line \
                       or a word, not a lead voice: do not pick one for a plain lead line) with \
                       default_speaker (never an effect mode), \
                       comfortable_range {low, high, range_source: builtin | default} per \
                       bank and per speaker, missing_phonemes and the substitutions they are \
                       sung as, curves (which of dynamics, tension, breathiness, pitch_bend \
                       reach the model — a curve the bank lacks does nothing), models \
                       {duration, pitch, variance} (the bank's own models a lane's \
                       bank_timing / bank_pitch use), credit, \
                       terms_url and commercial (free_with_credit | paid | permission) — tell \
                       the user the credit line when you use a bank — and vocoder with \
                       vocoder_installed. available lists the plok.org index's banks that are \
                       not installed (id, name, version, languages, speakers, size, vocoder, \
                       url, credit, terms_url, commercial) — install one with \
                       vocal_voicebank_install; index_status says whether that list is fresh, \
                       cached, still fetching (call again shortly) or unavailable (see \
                       index_error). Rescans the folders on every call, so a bank dropped in \
                       shows up. Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<vocal::VoicebankList>()
    )]
    async fn vocal_voicebanks(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(vocal::VOICEBANKS, &()).await
    }

    #[tool(
        description = "Install a voicebank from the plok.org index: id is one of \
                       vocal_voicebanks' available ids (or an installed bank's id, to update it \
                       to the index's version). Downloads the creator's own file (plok.org \
                       lists it, never hosts it), checks its sha256 against the index, unpacks \
                       it into the user's voicebank folder, and also fetches the vocoder the \
                       bank names when it ships none and none is installed. No project needs \
                       to be open; it creates no undo entry and bumps no revision. Running it \
                       ACCEPTS THE BANK'S TERMS on the user's behalf: the job's result carries \
                       terms_url, credit and commercial (free_with_credit | paid | permission) \
                       — tell the user the credit line and the terms. The result also has id \
                       (what vocal_set_params takes as voicebank), name, version, vocoder, \
                       vocoder_installed (false: the bank cannot render until its vocoder is \
                       installed), vocoder_fetched and already_installed (nothing was \
                       downloaded). Refuses an id the index does not list (not_found) and a \
                       bank that is already installing (busy); a download whose sha256 differs \
                       fails with \"upstream file changed; the index needs updating\" and \
                       installs nothing. Runs as a job — this tool waits up to 2 minutes and \
                       returns the final status; if still running, block on job_wait or poll \
                       job_status with the returned job_id.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = true),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn vocal_voicebank_install(
        &self,
        Parameters(params): Parameters<vocal::VoicebankInstallParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(vocal::VOICEBANK_INSTALL, &params, VOICEBANK_INSTALL_WAIT_MS)
            .await
    }

    #[tool(
        description = "Render the singing voice (SVS): it synthesises the notes currently in \
                       the target lane's MIDI clip and never changes them, so author with \
                       vocal_generate + notes_* first and render last. Scope widens as you \
                       omit arguments: section_id renders exactly that lane; track_id alone \
                       renders EVERY lane on that track; omitting both renders every vocal \
                       lane in the project. Lanes render sequentially and the job only \
                       reports done once the last lane lands, so one call per track is \
                       enough — a per-lane loop is not needed. Lanes that cannot render (no \
                       notes or no lyrics) are skipped rather than failing the batch. \
                       Omit voicebank to render each lane with the bank it already has (set \
                       it with vocal_set_params); passing one (an id from vocal_voicebanks) \
                       stores it on every lane the call covers. A lane whose bank or vocoder \
                       is not installed fails the whole call with voicebank_missing, naming \
                       the bank, before anything renders. \
                       Edits made through these tools (lyrics, notes, chords, params, \
                       pronunciation) never re-render on their own: the lane turns `stale` \
                       in song_vocal (its old audio keeps playing), so batch your edits and \
                       call this once; song_vocal then reads `rendered` per lane, and the \
                       track-level render_state is the worst lane's. Runs as a job — this tool waits up \
                       to 2 minutes and returns the final status; if still running, poll \
                       job_status with the returned job_id. Needs a vocal lane \
                       (section_set_lane_generator kind \"vocal\") with notes AND lyrics — \
                       pre-flight it with song_vocal and only render lanes whose \
                       counts_mismatch is false, since SVS is the most expensive operation on \
                       this surface.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn vocal_render(
        &self,
        Parameters(params): Parameters<vocal::RenderParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(vocal::RENDER, &params, VOCAL_RENDER_WAIT_MS)
            .await
    }
}
