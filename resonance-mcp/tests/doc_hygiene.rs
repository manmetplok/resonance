//! Regression tests for the *content* of the published tool
//! descriptions.
//!
//! Descriptions are compiled into the binary and are the only
//! documentation an agent driving this surface ever sees, so a wrong
//! example in one is worse than no example: the agent trusts it, the
//! call fails, and the error does not say the doc lied. The concrete
//! case this guards is `track_add_instrument`, which advertised
//! `"resonance-wavetable"` for months while the real CLAP id is
//! `"com.resonance.wavetable"` — a plugin id that never existed.

use resonance_mcp::ResonanceMcp;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// `(tool name, description)` for every published tool.
fn descriptions() -> Vec<(String, String)> {
    ResonanceMcp::combined_router()
        .list_all()
        .into_iter()
        .map(|tool| {
            (
                tool.name.to_string(),
                tool.description.as_deref().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// The workspace's `plugins/` directory — the ground truth for which
/// first-party plugins exist.
fn plugins_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate dir has a parent")
        .join("plugins")
}

#[test]
fn every_tool_has_a_real_description() {
    for (name, description) in descriptions() {
        assert!(
            description.len() > 20,
            "{name} has no usable description ({description:?}); the description is the only \
             documentation an agent gets"
        );
    }
}

/// Every `com.resonance.<name>` mentioned in a description must be a
/// plugin that actually exists in `plugins/`.
#[test]
fn plugin_ids_in_descriptions_exist() {
    let dir = plugins_dir();
    assert!(dir.is_dir(), "expected {} to exist", dir.display());
    let known: BTreeSet<String> = std::fs::read_dir(&dir)
        .expect("read plugins/")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()
                // `plugins/resonance-wavetable` -> `com.resonance.wavetable`
                .and_then(|dir_name| dir_name.strip_prefix("resonance-"))
                .map(|plugin| format!("com.resonance.{plugin}"))
        })
        .collect();
    assert!(
        known.contains("com.resonance.wavetable"),
        "sanity check failed: {known:?}"
    );

    for (tool, description) in descriptions() {
        for mentioned in mentions_of("com.resonance.", &description) {
            assert!(
                known.contains(&mentioned),
                "{tool}'s description names plugin id {mentioned:?}, which is not a plugin in \
                 {}; known ids: {known:?}",
                dir.display()
            );
        }
    }
}

/// No description may show a plugin id in the `resonance-<name>` shape:
/// that is the CRATE/bundle name, never an id the control API accepts,
/// and quoting it as an example is the exact defect this file exists for.
#[test]
fn descriptions_never_show_crate_names_as_plugin_ids() {
    for (tool, description) in descriptions() {
        assert!(
            !description.contains("\"resonance-"),
            "{tool}'s description quotes a plugin id starting with \"resonance-\"; plugin ids \
             are CLAP ids of the form \"com.resonance.<name>\" — the bare crate name is not \
             accepted by track_add_instrument / track_add_effect"
        );
    }
}

/// Descriptions are written as Rust string continuations, so a line
/// break must be spelled with a trailing `\` — writing the continuation
/// as plain indentation instead bakes a long run of spaces into the
/// schema an agent reads (ba doc #273, todo #1238 item 4).
#[test]
fn descriptions_carry_no_runs_of_padding_whitespace() {
    for (tool, description) in descriptions() {
        if let Some(at) = description.find("     ") {
            let around = &description[at.saturating_sub(40)..(at + 40).min(description.len())];
            panic!(
                "{tool}'s description contains a run of 5+ spaces, which lands verbatim in the \
                 published schema. Use a trailing backslash to continue the string instead of \
                 indenting the next line. Near: {around:?}"
            );
        }
    }
}

/// The aux-send persistence caveat (ba todo #482) must appear on EVERY
/// send tool, not most of them: an agent that reads only the one it is
/// calling must learn that its work is lost on reload (todo #1238 item
/// 3).
///
/// The set is DERIVED from the published tool names rather than listed
/// here, so a send tool added later is covered the day it is added — a
/// hardcoded list only pins the tools someone remembered to add to it.
#[test]
fn every_send_tool_states_the_persistence_caveat() {
    let send_tools: Vec<(String, String)> = descriptions()
        .into_iter()
        .filter(|(tool, _)| tool.contains("_send"))
        .collect();

    // Guard against the rule going vacuous: if sends are ever renamed out
    // of this shape, fail loudly instead of passing over an empty set.
    assert!(
        send_tools.len() >= 3,
        "expected at least the three aux-send tools to match `_send`, found {:?} — if sends \
         were renamed, update the rule rather than dropping the check",
        send_tools.iter().map(|(t, _)| t).collect::<Vec<_>>()
    );

    for (tool, description) in send_tools {
        assert!(
            description.contains("not SAVED")
                || description.contains("not saved")
                || description.contains("NOT SAVED"),
            "{tool} does not warn that aux sends are lost on save + reload (ba todo #482)"
        );
    }
}

/// Every id-shaped substring starting with `prefix`, up to the first
/// character that cannot appear in a CLAP id. A bare `prefix` with no
/// concrete name after it is the descriptions' `com.resonance.<name>`
/// placeholder, not a claim about a specific plugin — skipped.
fn mentions_of(prefix: &str, haystack: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = haystack;
    while let Some(at) = rest.find(prefix) {
        let tail = &rest[at..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_'))
            .unwrap_or(tail.len());
        let mention = tail[..end].trim_end_matches(['.', '-', '_']);
        if mention.len() > prefix.trim_end_matches('.').len() {
            out.push(mention.to_owned());
        }
        rest = &tail[end.max(1)..];
    }
    out
}

/// The traps an agent building a whole song trips over must stay
/// documented on the tool that springs them — each of these cost a real
/// session's worth of debugging.
#[test]
fn known_traps_stay_documented() {
    let by_name: std::collections::BTreeMap<String, String> = descriptions().into_iter().collect();
    let required: &[(&str, &[&str])] = &[
        // section.create places implicitly; `place: false` is the escape.
        ("section_create", &["place: false"]),
        // render.mixdown rejects a partial range on current builds.
        ("render_mixdown", &["range", "not supported", "NOT SUPPORTED"]),
        // generate.part silently ignores three of its own params.
        ("generate_part", &["IGNORES"]),
        // The bass default is a placeholder, not a part.
        ("generate_part", &["RootPulse"]),
    ];
    for (tool, needles) in required {
        let description = by_name
            .get(*tool)
            .unwrap_or_else(|| panic!("no tool named {tool}"));
        assert!(
            needles.iter().any(|needle| description.contains(needle)),
            "{tool}'s description no longer documents {needles:?}; if the underlying behaviour \
             changed, update this test together with the description"
        );
    }
}
