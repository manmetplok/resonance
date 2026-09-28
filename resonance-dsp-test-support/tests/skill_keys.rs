//! The keys-block parser and matcher behind `skill_keys_test!`.

use resonance_dsp_test_support::skill_keys::{blocks, known, matches_pattern, strip_blocks, Surface};

fn surface() -> Surface {
    Surface {
        clap_id: "com.example.p".into(),
        keys: vec!["drive".into(), "corr_b0_gain".into(), "band12_ms".into()],
        labels: vec!["Tape".into()],
        presets: vec!["Bus — Warm Glue".into()],
    }
}

#[test]
fn a_block_collects_its_inline_spans() {
    let text = "`outside`\n<!-- keys: com.example.p -->\n| `drive` | `Tape` then `Bus — Warm Glue` |\n<!-- /keys -->\n`after`\n";
    let found = blocks(text).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].plugin_id, "com.example.p");
    assert_eq!(found[0].line, 2);
    let spans: Vec<&str> = found[0].spans.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(spans, ["drive", "Tape", "Bus — Warm Glue"]);
    assert!(found[0].spans.iter().all(|s| s.line == 3));
}

#[test]
fn malformed_blocks_are_refused() {
    let unclosed = "<!-- keys: a -->\n`x`\n";
    let nested = "<!-- keys: a -->\n<!-- keys: b -->\n<!-- /keys -->\n";
    let stray = "<!-- /keys -->\n";
    let fenced = "<!-- keys: a -->\n```\nx\n```\n<!-- /keys -->\n";
    let no_id = "<!-- keys: -->\n<!-- /keys -->\n";
    for text in [unclosed, nested, stray, fenced, no_id] {
        assert!(blocks(text).is_err(), "accepted: {text:?}");
    }
}

#[test]
fn a_marker_inside_fenced_code_is_an_example_not_a_block() {
    let text = "```markdown\n<!-- keys: com.example.p -->\n`nope`\n<!-- /keys -->\n```\n";
    assert_eq!(blocks(text).unwrap(), []);
    assert!(strip_blocks(text).contains("nope"));
}

#[test]
fn strip_blanks_block_content_and_keeps_line_numbers() {
    let text = "a\n<!-- keys: p -->\n`clip_on`\n<!-- /keys -->\nb\n";
    let stripped = strip_blocks(text);
    assert_eq!(stripped.lines().count(), text.lines().count());
    assert!(!stripped.contains("clip_on"));
    assert!(stripped.contains('a') && stripped.contains('b'));
}

#[test]
fn band_patterns_match_digit_runs_only() {
    assert!(matches_pattern("corr_b{n}_gain", "corr_b0_gain"));
    assert!(matches_pattern("band{n}_ms", "band12_ms"));
    assert!(!matches_pattern("corr_b{n}_gain", "corr_bx_gain"));
    assert!(!matches_pattern("corr_b{n}_gain", "corr_b_gain"));
    assert!(!matches_pattern("corr_b{n}_gain", "corr_b0_gain_x"));
}

#[test]
fn known_accepts_keys_labels_presets_and_patterns() {
    let s = surface();
    for ok in ["drive", "Tape", "Bus — Warm Glue", "corr_b{n}_gain", "band{n}_ms"] {
        assert!(known(&s, ok), "{ok}");
    }
    for bad in ["driv", "tape", "Bus - Warm Glue", "tone_b{n}_gain"] {
        assert!(!known(&s, bad), "{bad}");
    }
}
