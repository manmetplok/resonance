//! The keys-block parser and matcher behind `skill_keys_test!`.

use resonance_dsp_test_support::skill_keys::{
    blocks, check_block, declared_ranges, known, loose_names, matches_pattern, strip_blocks, Surface,
};

fn surface() -> Surface {
    Surface {
        clap_id: "com.example.p".into(),
        keys: vec![
            "drive".into(),
            "mode".into(),
            "speed".into(),
            "corr_b0_gain".into(),
            "corr_b1_gain".into(),
            "band12_ms".into(),
        ],
        choices: vec![
            ("mode".into(), vec!["Tape".into(), "Tube".into()]),
            ("speed".into(), vec!["15 ips".into()]),
            ("band12_ms".into(), vec!["Side".into()]),
        ],
        presets: vec!["Bus — Warm Glue".into()],
    }
}

/// The problems `check_block` finds in a one-block document.
fn problems(body: &str) -> Vec<String> {
    let text = format!("<!-- keys: com.example.p -->\n{body}\n<!-- /keys -->\n");
    let found = blocks(&text).unwrap();
    check_block(&surface(), &found[0]).into_iter().map(|(_, why)| why).collect()
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
    let columns: Vec<usize> = found[0].spans.iter().map(|s| s.cell.unwrap().column).collect();
    assert_eq!(columns, [0, 1, 1]);
}

#[test]
fn malformed_blocks_are_refused() {
    let unclosed = "<!-- keys: a -->\n`x`\n";
    let nested = "<!-- keys: a -->\n<!-- keys: b -->\n<!-- /keys -->\n";
    let stray = "<!-- /keys -->\n";
    let fenced = "<!-- keys: a -->\n```\nx\n```\n<!-- /keys -->\n";
    let no_id = "<!-- keys: -->\n<!-- /keys -->\n";
    let wrapped = "<!-- keys: a -->\nset `Bus —\nWarm Glue`\n<!-- /keys -->\n";
    for text in [unclosed, nested, stray, fenced, no_id, wrapped] {
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

#[test]
fn a_label_binds_to_the_nearest_key_in_its_row_or_paragraph() {
    assert!(problems("| `mode` | `Tape` or `Tube` |").is_empty());
    assert!(problems("Set `mode` to `Tape`, `speed` `15 ips`.").is_empty());
    assert!(problems("`band{n}_ms` `Side`").is_empty());
    // `Tape` is a `mode` value, not a `speed` one.
    let wrong = problems("| `speed` | `Tape` only |");
    assert!(wrong.len() == 1 && wrong[0].contains("follows `speed`"), "{wrong:?}");
    // No key before it: unbound.
    let unbound = problems("| Drum bus | `Tape` at `15 ips` |");
    assert_eq!(unbound.len(), 2, "{unbound:?}");
    // A key in an earlier paragraph does not bind.
    assert_eq!(problems("`mode` is the voicing.\n\nPick `Tape`.").len(), 1);
    // A list item is its own scope.
    assert_eq!(problems("- `mode` first\n- then `Tube`").len(), 1);
}

#[test]
fn a_label_in_a_table_body_falls_back_to_its_column_header() {
    let table = "| Mode (`mode`) | Start from |\n|---|---|\n| `Tape` | `Bus — Warm Glue` |\n| `Tube` | — |";
    assert!(problems(table).is_empty(), "{:?}", problems(table));
    let other_column = "| Mode (`mode`) | Notes |\n|---|---|\n| — | `Tape` |";
    assert_eq!(problems(other_column).len(), 1);
}

#[test]
fn keys_and_presets_outside_blocks_are_found() {
    let text = "Set `drive`\nthen `Bus —\nWarm Glue` and `unrelated`.\n```\n`mode`\n```\n`range`\n";
    let found = loose_names(&surface(), &strip_blocks(text));
    assert_eq!(found, [(1, "drive".to_owned()), (2, "Bus — Warm Glue".to_owned())]);
}

#[test]
fn declared_band_ranges_are_parsed() {
    assert_eq!(declared_ranges("per band n = 0-3: "), [(0, 3)]);
    assert_eq!(declared_ranges("(n=0–7)"), [(0, 7)]);
    assert_eq!(declared_ranges("an = 3-4, band n = 1"), []);
}

#[test]
fn a_declared_range_must_be_the_real_band_count() {
    assert!(problems("per band n = 0-1: `corr_b{n}_gain`").is_empty());
    let short = problems("per band n = 0-0: `corr_b{n}_gain`");
    assert!(short.len() == 1 && short[0].contains("exists too"), "{short:?}");
    let long = problems("per band n = 0-3: `corr_b{n}_gain`");
    assert!(long.len() == 1 && long[0].contains("no `corr_b2_gain`"), "{long:?}");
    // An earlier declaration in the block carries forward.
    assert_eq!(problems("Bands n = 0-3.\n\n`corr_b{n}_gain`").len(), 1);
    // No declaration: existence only.
    assert!(problems("`corr_b{n}_gain`").is_empty());
    assert_eq!(problems("`tone_b{n}_gain`").len(), 1);
}
