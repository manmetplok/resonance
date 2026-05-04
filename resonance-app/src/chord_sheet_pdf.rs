//! Chord sheet PDF export — generates a printable PDF with vector-drawn
//! chord diagrams for guitar (6/8-string), bass (4/5-string), and keyboard.
//! One page section per instrument. Each section shows a scale diagram
//! followed by chord voicing diagrams (8 per row).

use std::collections::HashSet;

use printpdf::*;
use resonance_music_theory::{
    fretboard_voicing, Chord, PitchClass, Scale, Tuning, BASS_4, BASS_5, GUITAR_6,
    GUITAR_8,
};

use crate::compose::{ChordState, ComposeState, SectionDefinitionState};

// -- Page layout (A4, mm) ----------------------------------------------------

const PAGE_W: f32 = 210.0;
const PAGE_H: f32 = 297.0;
const MARGIN: f32 = 10.0;
const USABLE_W: f32 = PAGE_W - 2.0 * MARGIN;

const CHORDS_PER_ROW: usize = 8;
const CELL_W: f32 = USABLE_W / CHORDS_PER_ROW as f32;

// -- Fretboard cell dimensions -----------------------------------------------

const FB_FRET_SPACING: f32 = 3.2;
const FB_FRET_COUNT: usize = 4;
const FB_HEADER: f32 = 5.0;
const FB_CHORD_NAME_H: f32 = 4.5;
const FB_DOT_R: f32 = 1.3;
const FB_CELL_H: f32 = FB_CHORD_NAME_H + FB_HEADER + FB_FRET_COUNT as f32 * FB_FRET_SPACING + 3.0;

// -- Scale diagram dimensions ------------------------------------------------

const SCALE_FRET_COUNT: usize = 12;
const SCALE_STRING_SPACING: f32 = 3.5;
const SCALE_DOT_R: f32 = 1.1;
const SCALE_LABEL_H: f32 = 4.0; // space for "Scale: ..." text above

// -- Keyboard cell dimensions ------------------------------------------------

const KB_WHITE_COUNT: usize = 7;
const KB_KEY_H: f32 = 9.0;
const KB_BLACK_H: f32 = 5.5;
const KB_CELL_H: f32 = FB_CHORD_NAME_H + KB_KEY_H + 3.0;

/// Pitch classes of the 7 white keys in one octave (C major).
const WHITE_PCS: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
/// Black key positions: (white-key index to the left, pitch class).
const BLACK_KEYS: [(usize, u8); 5] = [(0, 1), (1, 3), (3, 6), (4, 8), (5, 10)];

// -- Keyboard scale dimensions -----------------------------------------------

const KB_SCALE_KEY_H: f32 = 14.0;
const KB_SCALE_BLACK_H: f32 = 8.5;

const HEADER_H: f32 = 12.0;
const SECTION_HEADER_H: f32 = 7.0;
const ROW_GAP: f32 = 1.5;

// -- Colors ------------------------------------------------------------------

fn rgb(r: f32, g: f32, b: f32) -> Color {
    Color::Rgb(Rgb { r, g, b, icc_profile: None })
}
fn black() -> Color { rgb(0.0, 0.0, 0.0) }
fn white() -> Color { rgb(1.0, 1.0, 1.0) }
fn gray(v: f32) -> Color { rgb(v, v, v) }
fn accent() -> Color { rgb(0.91, 0.51, 0.16) }
fn dark_dot() -> Color { rgb(0.25, 0.25, 0.25) }
fn scale_dot() -> Color { rgb(0.35, 0.55, 0.75) }

// -- Coordinate helpers ------------------------------------------------------

fn pdf_y(top_y: f32) -> Pt { Mm(PAGE_H - top_y).into() }

fn pt(x_mm: f32, y_top: f32) -> Point {
    Point { x: Mm(x_mm).into(), y: pdf_y(y_top) }
}

fn text_op(s: &str, x: f32, y: f32, size: f32, font: BuiltinFont, col: Color) -> Vec<Op> {
    vec![
        Op::StartTextSection,
        Op::SetFont { font: PdfFontHandle::Builtin(font), size: Pt(size) },
        Op::SetFillColor { col },
        Op::SetTextCursor { pos: pt(x, y) },
        Op::ShowText { items: vec![TextItem::Text(s.to_string())] },
        Op::EndTextSection,
    ]
}

fn line_op(x1: f32, y1: f32, x2: f32, y2: f32) -> Op {
    Op::DrawLine {
        line: Line {
            points: vec![
                LinePoint { p: pt(x1, y1), bezier: false },
                LinePoint { p: pt(x2, y2), bezier: false },
            ],
            is_closed: false,
        },
    }
}

fn rect_filled(ops: &mut Vec<Op>, x: f32, y: f32, w: f32, h: f32, fill: Color, stroke: Color, sw: f32) {
    ops.push(Op::SetFillColor { col: fill });
    ops.push(Op::SetOutlineColor { col: stroke });
    ops.push(Op::SetOutlineThickness { pt: Pt(sw) });
    ops.push(Op::DrawPolygon {
        polygon: Polygon {
            rings: vec![PolygonRing {
                points: vec![
                    LinePoint { p: pt(x, y), bezier: false },
                    LinePoint { p: pt(x + w, y), bezier: false },
                    LinePoint { p: pt(x + w, y + h), bezier: false },
                    LinePoint { p: pt(x, y + h), bezier: false },
                ],
            }],
            mode: PaintMode::FillStroke,
            winding_order: WindingOrder::NonZero,
        },
    });
}

fn circle_ops(cx: f32, cy: f32, r: f32, fill: Color) -> Vec<Op> {
    // 4*(sqrt(2)-1)/3 ≈ 0.5523 — standard kappa for Bezier circle approximation
    let k = r * 0.5523;
    vec![
        Op::SetFillColor { col: fill },
        Op::DrawPolygon {
            polygon: Polygon {
                rings: vec![PolygonRing {
                    points: vec![
                        LinePoint { p: pt(cx, cy - r), bezier: false },
                        LinePoint { p: pt(cx + k, cy - r), bezier: true },
                        LinePoint { p: pt(cx + r, cy - k), bezier: true },
                        LinePoint { p: pt(cx + r, cy), bezier: false },
                        LinePoint { p: pt(cx + r, cy + k), bezier: true },
                        LinePoint { p: pt(cx + k, cy + r), bezier: true },
                        LinePoint { p: pt(cx, cy + r), bezier: false },
                        LinePoint { p: pt(cx - k, cy + r), bezier: true },
                        LinePoint { p: pt(cx - r, cy + k), bezier: true },
                        LinePoint { p: pt(cx - r, cy), bezier: false },
                        LinePoint { p: pt(cx - r, cy - k), bezier: true },
                        LinePoint { p: pt(cx - k, cy - r), bezier: true },
                    ],
                }],
                mode: PaintMode::Fill,
                winding_order: WindingOrder::NonZero,
            },
        },
    ]
}

// -- String spacing for a tuning in a chord cell -----------------------------

fn cell_string_spacing(string_count: usize) -> f32 {
    match string_count {
        4 => 3.4,
        5 => 2.8,
        6 => 2.6,
        8 => 1.9,
        _ => 2.6,
    }
}

// -- Chord fretboard cell ----------------------------------------------------

fn draw_fretboard_cell(
    ops: &mut Vec<Op>,
    cell_x: f32,
    cell_y: f32,
    tuning: &Tuning,
    frets: &[Option<u8>],
    start_fret: u8,
    chord: &Chord,
) {
    let n = tuning.string_count();
    let spacing = cell_string_spacing(n);
    let board_w = (n - 1) as f32 * spacing;
    let board_x = cell_x + (CELL_W - board_w) / 2.0;
    let name_y = cell_y;
    let header_y = name_y + FB_CHORD_NAME_H;
    let nut_y = header_y + FB_HEADER;
    let board_h = FB_FRET_COUNT as f32 * FB_FRET_SPACING;
    let root_pc = chord.bass.unwrap_or(chord.root).to_semitone();

    // Chord name
    let name = chord.to_string();
    let name_x = cell_x + (CELL_W - name.len() as f32 * 2.2) / 2.0;
    ops.extend(text_op(&name, name_x, name_y, 7.0, BuiltinFont::HelveticaBold, black()));

    // String labels (low string on left, high on right)
    for (i, label) in tuning.labels.iter().enumerate() {
        let sx = board_x + i as f32 * spacing - if label.len() > 1 { 1.2 } else { 0.6 };
        ops.extend(text_op(label, sx, header_y, 3.5, BuiltinFont::Helvetica, gray(0.4)));
    }

    // Open/mute markers
    for (i, f) in frets.iter().enumerate() {
        let sx = board_x + i as f32 * spacing;
        let marker = match f {
            Some(0) => "O",
            None => "X",
            _ => continue,
        };
        ops.extend(text_op(marker, sx - 0.7, header_y + 2.5, 3.0, BuiltinFont::HelveticaBold, gray(0.3)));
    }

    // Nut
    if start_fret == 0 {
        ops.push(Op::SetOutlineColor { col: black() });
        ops.push(Op::SetOutlineThickness { pt: Pt(1.5) });
        ops.push(line_op(board_x, nut_y, board_x + board_w, nut_y));
    } else {
        ops.extend(text_op(
            &format!("{}f", start_fret),
            board_x + board_w + 0.8, nut_y + 1.0,
            2.5, BuiltinFont::Helvetica, gray(0.4),
        ));
    }

    // Fret lines
    ops.push(Op::SetOutlineColor { col: gray(0.65) });
    ops.push(Op::SetOutlineThickness { pt: Pt(0.3) });
    for f in 0..=FB_FRET_COUNT {
        let fy = nut_y + f as f32 * FB_FRET_SPACING;
        ops.push(line_op(board_x, fy, board_x + board_w, fy));
    }

    // String lines
    ops.push(Op::SetOutlineColor { col: gray(0.2) });
    ops.push(Op::SetOutlineThickness { pt: Pt(0.25) });
    for i in 0..n {
        let sx = board_x + i as f32 * spacing;
        ops.push(line_op(sx, nut_y, sx, nut_y + board_h));
    }

    // Finger dots
    for (i, fret_opt) in frets.iter().enumerate() {
        let Some(&fret) = fret_opt.as_ref() else { continue };
        if fret == 0 { continue; }
        let display_fret = if start_fret == 0 { fret } else { fret - start_fret + 1 };
        if display_fret > FB_FRET_COUNT as u8 { continue; }

        let sx = board_x + i as f32 * spacing;
        let fy = nut_y + (display_fret as f32 - 0.5) * FB_FRET_SPACING;
        let note_pc = (tuning.open[i] + fret) % 12;
        let fill = if note_pc == root_pc { accent() } else { dark_dot() };
        ops.extend(circle_ops(sx, fy, FB_DOT_R, fill));

        let note_name = PitchClass::from_semitone(note_pc).as_str();
        let tx = sx - if note_name.len() > 1 { 1.2 } else { 0.6 };
        ops.extend(text_op(note_name, tx, fy + 0.6, 2.5, BuiltinFont::HelveticaBold, white()));
    }
}

// -- Scale fretboard diagram (full width) ------------------------------------

fn scale_diagram_height(tuning: &Tuning) -> f32 {
    SCALE_LABEL_H + (tuning.string_count() - 1) as f32 * SCALE_STRING_SPACING + 4.0
}

fn draw_scale_fretboard(ops: &mut Vec<Op>, y: f32, tuning: &Tuning, scale: &Scale) {
    let n = tuning.string_count();
    let label_y = y;
    let grid_y = y + SCALE_LABEL_H;
    let grid_h = (n - 1) as f32 * SCALE_STRING_SPACING;
    let root_pc = scale.root.to_semitone();

    // Label
    let label = format!("Scale: {}", scale);
    ops.extend(text_op(&label, MARGIN, label_y, 6.0, BuiltinFont::HelveticaBold, gray(0.3)));

    // String labels on the left
    let label_col_w = 8.0;
    let grid_x = MARGIN + label_col_w;
    let grid_w = USABLE_W - label_col_w;
    let fret_w = grid_w / (SCALE_FRET_COUNT + 1) as f32; // col 0 = open, cols 1-12 = frets

    for (s, label) in tuning.labels.iter().enumerate() {
        let vs = n - 1 - s; // flip: low string at bottom
        let sy = grid_y + vs as f32 * SCALE_STRING_SPACING;
        ops.extend(text_op(label, MARGIN, sy + 0.5, 3.5, BuiltinFont::Helvetica, gray(0.4)));
    }

    // Nut
    let nut_x = grid_x + fret_w;
    ops.push(Op::SetOutlineColor { col: black() });
    ops.push(Op::SetOutlineThickness { pt: Pt(1.0) });
    ops.push(line_op(nut_x, grid_y, nut_x, grid_y + grid_h));

    // Fret lines (vertical)
    ops.push(Op::SetOutlineColor { col: gray(0.7) });
    ops.push(Op::SetOutlineThickness { pt: Pt(0.2) });
    for f in 2..=(SCALE_FRET_COUNT + 1) {
        let fx = grid_x + f as f32 * fret_w;
        ops.push(line_op(fx, grid_y, fx, grid_y + grid_h));
    }

    // Fret numbers
    for f in 1..=SCALE_FRET_COUNT {
        let fx = grid_x + f as f32 * fret_w + fret_w * 0.35;
        ops.extend(text_op(&f.to_string(), fx, grid_y + grid_h + 1.5, 3.0, BuiltinFont::Helvetica, gray(0.5)));
    }

    // String lines (horizontal)
    ops.push(Op::SetOutlineColor { col: gray(0.3) });
    ops.push(Op::SetOutlineThickness { pt: Pt(0.2) });
    for s in 0..n {
        let sy = grid_y + s as f32 * SCALE_STRING_SPACING;
        ops.push(line_op(grid_x, sy, grid_x + grid_w, sy));
    }

    // Scale dots
    for s in 0..n {
        let vs = n - 1 - s; // flip: low string at bottom
        let sy = grid_y + vs as f32 * SCALE_STRING_SPACING;
        let open = tuning.open[s];

        // Open string (fret 0) — placed in the first column before the nut
        let open_pc = open % 12;
        if scale.contains(open) {
            let dot_x = grid_x + fret_w * 0.5;
            let fill = if open_pc == root_pc { accent() } else { scale_dot() };
            ops.extend(circle_ops(dot_x, sy, SCALE_DOT_R, fill));
        }

        // Frets 1-12
        for fret in 1..=SCALE_FRET_COUNT as u8 {
            let midi = open + fret;
            if scale.contains(midi) {
                let fx = grid_x + fret as f32 * fret_w + fret_w * 0.5;
                let pc = midi % 12;
                let fill = if pc == root_pc { accent() } else { scale_dot() };
                ops.extend(circle_ops(fx, sy, SCALE_DOT_R, fill));
            }
        }
    }
}

// -- Keyboard chord cell -----------------------------------------------------

fn draw_keyboard_cell(ops: &mut Vec<Op>, cell_x: f32, cell_y: f32, chord: &Chord) {
    let chord_pcs: Vec<u8> = chord.pitch_classes().iter().map(|pc| pc.to_semitone()).collect();
    let root_pc = chord.bass.unwrap_or(chord.root).to_semitone();

    let name = chord.to_string();
    let name_x = cell_x + (CELL_W - name.len() as f32 * 2.2) / 2.0;
    ops.extend(text_op(&name, name_x, cell_y, 7.0, BuiltinFont::HelveticaBold, black()));

    let kb_y = cell_y + FB_CHORD_NAME_H;
    draw_keyboard_keys(ops, cell_x + 1.0, kb_y, CELL_W - 2.0, KB_KEY_H, KB_BLACK_H, &chord_pcs, root_pc, true);
}

// -- Keyboard scale diagram (full width) -------------------------------------

fn draw_scale_keyboard(ops: &mut Vec<Op>, y: f32, scale: &Scale) -> f32 {
    let label = format!("Scale: {}", scale);
    ops.extend(text_op(&label, MARGIN, y, 6.0, BuiltinFont::HelveticaBold, gray(0.3)));

    let kb_y = y + SCALE_LABEL_H;
    let root_pc = scale.root.to_semitone();
    let scale_pcs: Vec<u8> = (0..12u8).filter(|&pc| {
        let diff = (pc + 12 - root_pc) % 12;
        scale.mode.intervals().contains(&diff)
    }).collect();

    // Use 2 octaves for scale, full width
    let kb_w = USABLE_W;
    draw_keyboard_keys_2oct(ops, MARGIN, kb_y, kb_w, KB_SCALE_KEY_H, KB_SCALE_BLACK_H, &scale_pcs, root_pc);

    kb_y + KB_SCALE_KEY_H + 3.0
}

// -- Shared keyboard drawing -------------------------------------------------

fn draw_keyboard_keys(
    ops: &mut Vec<Op>, x: f32, y: f32, w: f32, key_h: f32, black_h: f32,
    highlighted_pcs: &[u8], root_pc: u8, show_names: bool,
) {
    let white_key_w = w / KB_WHITE_COUNT as f32;
    let black_key_w = white_key_w * 0.6;
    let white_pcs = WHITE_PCS;

    for (i, &pc) in white_pcs.iter().enumerate() {
        let kx = x + i as f32 * white_key_w;
        let is_ct = highlighted_pcs.contains(&pc);
        let is_root = pc == root_pc;
        let fill = if is_root { accent() } else if is_ct { rgb(0.95, 0.72, 0.35) } else { gray(0.93) };
        rect_filled(ops, kx, y, white_key_w - 0.3, key_h, fill, gray(0.3), 0.3);

        if is_ct && show_names {
            let name = PitchClass::from_semitone(pc).as_str();
            let col = if is_root { white() } else { gray(0.15) };
            ops.extend(text_op(name, kx + 0.3, y + key_h - 1.8, 3.0, BuiltinFont::HelveticaBold, col));
        }
    }

    let black_keys = BLACK_KEYS;
    for &(wi, pc) in &black_keys {
        let bx = x + (wi as f32 + 1.0) * white_key_w - black_key_w / 2.0;
        let is_ct = highlighted_pcs.contains(&pc);
        let is_root = pc == root_pc;
        let fill = if is_root { accent() } else if is_ct { rgb(0.80, 0.50, 0.15) } else { gray(0.12) };
        rect_filled(ops, bx, y, black_key_w, black_h, fill, black(), 0.3);

        if is_ct && show_names {
            let name = PitchClass::from_semitone(pc).as_str();
            ops.extend(text_op(name, bx + 0.2, y + black_h - 1.2, 2.5, BuiltinFont::HelveticaBold, white()));
        }
    }
}

fn draw_keyboard_keys_2oct(
    ops: &mut Vec<Op>, x: f32, y: f32, w: f32, key_h: f32, black_h: f32,
    highlighted_pcs: &[u8], root_pc: u8,
) {
    let octave_w = w / 2.0;
    let white_key_w = octave_w / KB_WHITE_COUNT as f32;
    let black_key_w = white_key_w * 0.55;
    let white_pcs = WHITE_PCS;

    for oct in 0..2 {
        let ox = x + oct as f32 * octave_w;
        for (i, &pc) in white_pcs.iter().enumerate() {
            let kx = ox + i as f32 * white_key_w;
            let is_ct = highlighted_pcs.contains(&pc);
            let is_root = pc == root_pc;
            let fill = if is_root { accent() } else if is_ct { rgb(0.92, 0.72, 0.35) } else { gray(0.93) };
            rect_filled(ops, kx, y, white_key_w - 0.3, key_h, fill, gray(0.3), 0.3);

            if is_ct {
                let name = PitchClass::from_semitone(pc).as_str();
                let col = if is_root { white() } else { gray(0.15) };
                ops.extend(text_op(name, kx + 0.5, y + key_h - 2.5, 4.0, BuiltinFont::HelveticaBold, col));
            }
        }

        let black_keys = BLACK_KEYS;
        for &(wi, pc) in &black_keys {
            let bx = ox + (wi as f32 + 1.0) * white_key_w - black_key_w / 2.0;
            let is_ct = highlighted_pcs.contains(&pc);
            let is_root = pc == root_pc;
            let fill = if is_root { accent() } else if is_ct { rgb(0.80, 0.50, 0.15) } else { gray(0.12) };
            rect_filled(ops, bx, y, black_key_w, black_h, fill, black(), 0.3);

            if is_ct {
                let name = PitchClass::from_semitone(pc).as_str();
                ops.extend(text_op(name, bx + 0.3, y + black_h - 1.5, 3.0, BuiltinFont::HelveticaBold, white()));
            }
        }
    }
}

// -- Page structure ----------------------------------------------------------

fn new_page(ops: Vec<Op>) -> PdfPage {
    PdfPage::new(Mm(PAGE_W), Mm(PAGE_H), ops)
}

fn draw_page_header(ops: &mut Vec<Op>, title: &str, bpm: f32, time_sig_num: u8) {
    ops.extend(text_op(title, MARGIN, MARGIN, 14.0, BuiltinFont::HelveticaBold, black()));
    let info = format!("Tempo: {} BPM  |  {}/4", bpm as u32, time_sig_num);
    ops.extend(text_op(&info, MARGIN + 90.0, MARGIN, 8.0, BuiltinFont::Helvetica, gray(0.4)));
    ops.push(Op::SetOutlineColor { col: gray(0.7) });
    ops.push(Op::SetOutlineThickness { pt: Pt(0.4) });
    ops.push(line_op(MARGIN, MARGIN + 5.0, PAGE_W - MARGIN, MARGIN + 5.0));
}

fn draw_section_header(ops: &mut Vec<Op>, y: f32, def: &SectionDefinitionState) {
    let scale_str = def.scale.map(|s| format!(" - {}", s)).unwrap_or_default();
    let header = format!("{} ({} bars){}", def.name, def.length_bars, scale_str);
    ops.extend(text_op(&header, MARGIN, y, 8.0, BuiltinFont::HelveticaBold, rgb(0.15, 0.15, 0.4)));
    ops.push(Op::SetOutlineColor { col: gray(0.8) });
    ops.push(Op::SetOutlineThickness { pt: Pt(0.2) });
    ops.push(line_op(MARGIN, y + 3.5, PAGE_W - MARGIN, y + 3.5));
}

/// Collect unique chords per section in arrangement order.
fn collect_section_chords(compose: &ComposeState) -> Vec<(&SectionDefinitionState, Vec<&ChordState>)> {
    let mut placements: Vec<_> = compose.placements.iter().collect();
    placements.sort_by_key(|p| p.start_bar);

    let mut seen = std::collections::HashSet::new();
    let mut sections: Vec<&SectionDefinitionState> = Vec::new();
    for p in &placements {
        if seen.insert(p.definition_id) {
            if let Some(def) = compose.find_definition(p.definition_id) {
                sections.push(def);
            }
        }
    }
    for def in &compose.definitions {
        if !seen.contains(&def.id) {
            sections.push(def);
        }
    }

    sections
        .into_iter()
        .filter_map(|def| {
            let mut chords: Vec<_> = def.chords.iter().collect();
            chords.sort_by_key(|c| c.start_beat);
            let mut seen = HashSet::new();
            chords.retain(|c| seen.insert(c.chord));
            if chords.is_empty() { None } else { Some((def, chords)) }
        })
        .collect()
}

// -- Per-instrument page renderer (fretboard instruments) ---------------------

fn render_fretboard_pages(
    title: &str,
    bpm: f32,
    time_sig_num: u8,
    section_chords: &[(&SectionDefinitionState, Vec<&ChordState>)],
    tuning: &Tuning,
) -> Vec<PdfPage> {
    let mut pages = Vec::new();
    let max_y = PAGE_H - MARGIN;
    let mut ops: Vec<Op> = Vec::new();
    let mut cursor_y: f32;

    let scale_h = scale_diagram_height(tuning);

    for &(def, ref chords) in section_chords {
        // Start each section on a fresh page
        if !ops.is_empty() { pages.push(new_page(std::mem::take(&mut ops))); }
        draw_page_header(&mut ops, title, bpm, time_sig_num);
        cursor_y = MARGIN + HEADER_H;

        // Section header
        draw_section_header(&mut ops, cursor_y, def);
        cursor_y += SECTION_HEADER_H;

        // Scale diagram (if section has a scale)
        if let Some(scale) = def.scale {
            if cursor_y + scale_h > max_y {
                pages.push(new_page(std::mem::take(&mut ops)));
                draw_page_header(&mut ops, title, bpm, time_sig_num);
                cursor_y = MARGIN + HEADER_H;
            }
            draw_scale_fretboard(&mut ops, cursor_y, tuning, &scale);
            cursor_y += scale_h;
        }

        // Chord diagrams in rows of 8
        for row_chords in chords.chunks(CHORDS_PER_ROW) {
            if cursor_y + FB_CELL_H > max_y {
                pages.push(new_page(std::mem::take(&mut ops)));
                draw_page_header(&mut ops, title, bpm, time_sig_num);
                cursor_y = MARGIN + HEADER_H;
            }

            for (col, cs) in row_chords.iter().enumerate() {
                let cell_x = MARGIN + col as f32 * CELL_W;
                let v = fretboard_voicing(&cs.chord, tuning);
                draw_fretboard_cell(&mut ops, cell_x, cursor_y, tuning, &v.frets, v.start_fret, &cs.chord);
            }

            cursor_y += FB_CELL_H + ROW_GAP;
        }
    }

    if !ops.is_empty() { pages.push(new_page(ops)); }
    pages
}

// -- Keyboard page renderer --------------------------------------------------

fn render_keyboard_pages(
    bpm: f32,
    time_sig_num: u8,
    section_chords: &[(&SectionDefinitionState, Vec<&ChordState>)],
) -> Vec<PdfPage> {
    let mut pages = Vec::new();
    let max_y = PAGE_H - MARGIN;
    let mut ops: Vec<Op> = Vec::new();
    let mut cursor_y: f32;

    for &(def, ref chords) in section_chords {
        // Each section on a fresh page
        if !ops.is_empty() { pages.push(new_page(std::mem::take(&mut ops))); }
        draw_page_header(&mut ops, "Keyboard Chords", bpm, time_sig_num);
        cursor_y = MARGIN + HEADER_H;

        draw_section_header(&mut ops, cursor_y, def);
        cursor_y += SECTION_HEADER_H;

        // Scale diagram
        if let Some(scale) = def.scale {
            let end_y = draw_scale_keyboard(&mut ops, cursor_y, &scale);
            cursor_y = end_y;
        }

        // Chord diagrams
        for row_chords in chords.chunks(CHORDS_PER_ROW) {
            if cursor_y + KB_CELL_H > max_y {
                pages.push(new_page(std::mem::take(&mut ops)));
                draw_page_header(&mut ops, "Keyboard Chords", bpm, time_sig_num);
                cursor_y = MARGIN + HEADER_H;
            }

            for (col, cs) in row_chords.iter().enumerate() {
                let cell_x = MARGIN + col as f32 * CELL_W;
                draw_keyboard_cell(&mut ops, cell_x, cursor_y, &cs.chord);
            }

            cursor_y += KB_CELL_H + ROW_GAP;
        }
    }

    if !ops.is_empty() { pages.push(new_page(ops)); }
    pages
}

// -- Public entry point ------------------------------------------------------

pub fn build_chord_sheet_pdf(compose: &ComposeState, bpm: f32, time_sig_num: u8) -> Vec<u8> {
    let section_chords = collect_section_chords(compose);

    if section_chords.is_empty() {
        let mut ops = Vec::new();
        draw_page_header(&mut ops, "Chord Sheet", bpm, time_sig_num);
        ops.extend(text_op("No chords to display.", MARGIN, MARGIN + 18.0, 10.0, BuiltinFont::Helvetica, gray(0.5)));
        let mut doc = PdfDocument::new("Chord Sheet");
        doc.pages.push(new_page(ops));
        let mut w = Vec::new();
        return doc.save(&PdfSaveOptions::default(), &mut w);
    }

    let mut doc = PdfDocument::new("Chord Sheet");

    // Guitar 6-string
    doc.pages.extend(render_fretboard_pages(GUITAR_6.name, bpm, time_sig_num, &section_chords, &GUITAR_6));
    // Guitar 8-string
    doc.pages.extend(render_fretboard_pages(GUITAR_8.name, bpm, time_sig_num, &section_chords, &GUITAR_8));
    // Bass 4-string
    doc.pages.extend(render_fretboard_pages(BASS_4.name, bpm, time_sig_num, &section_chords, &BASS_4));
    // Bass 5-string
    doc.pages.extend(render_fretboard_pages(BASS_5.name, bpm, time_sig_num, &section_chords, &BASS_5));
    // Keyboard
    doc.pages.extend(render_keyboard_pages(bpm, time_sig_num, &section_chords));

    let mut warnings = Vec::new();
    doc.save(&PdfSaveOptions::default(), &mut warnings)
}
