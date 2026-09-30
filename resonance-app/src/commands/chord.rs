//! Key chords: the portable modifier+key model and its iced bridge.

// ===========================================================================
// Key chords
// ===========================================================================

/// Modifier-key set for a [`KeyChord`]. Order-independent; `ctrl` and `cmd`
/// are kept distinct so the model is faithful on every platform even though
/// the default presets are authored macOS-first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: false,
        cmd: false,
    };

    pub const fn cmd() -> Mods {
        Mods {
            cmd: true,
            ..Mods::NONE
        }
    }
    pub const fn cmd_shift() -> Mods {
        Mods {
            cmd: true,
            shift: true,
            ..Mods::NONE
        }
    }
}

/// A named (non-character) key usable in a chord. Kept deliberately small —
/// just the keys the registry actually binds — and decoupled from iced so
/// parse/format are unit-testable without a windowing backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Enter,
    Escape,
    Space,
    Tab,
    Backspace,
    Delete,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Plus,
    Minus,
    Comma,
    Home,
    End,
    PageUp,
    PageDown,
}

impl NamedKey {
    /// Canonical token used by [`KeyChord`] parse/format round-tripping.
    fn token(self) -> &'static str {
        match self {
            NamedKey::Enter => "Enter",
            NamedKey::Escape => "Escape",
            NamedKey::Space => "Space",
            NamedKey::Tab => "Tab",
            NamedKey::Backspace => "Backspace",
            NamedKey::Delete => "Delete",
            NamedKey::ArrowUp => "ArrowUp",
            NamedKey::ArrowDown => "ArrowDown",
            NamedKey::ArrowLeft => "ArrowLeft",
            NamedKey::ArrowRight => "ArrowRight",
            NamedKey::Plus => "Plus",
            NamedKey::Minus => "Minus",
            NamedKey::Comma => "Comma",
            NamedKey::Home => "Home",
            NamedKey::End => "End",
            NamedKey::PageUp => "PageUp",
            NamedKey::PageDown => "PageDown",
        }
    }

    /// Keycap glyph used when formatting a chord for display.
    fn glyph(self) -> &'static str {
        match self {
            NamedKey::Enter => "↵",
            NamedKey::Escape => "Esc",
            NamedKey::Space => "Space",
            NamedKey::Tab => "⇥",
            NamedKey::Backspace => "⌫",
            NamedKey::Delete => "⌦",
            NamedKey::ArrowUp => "↑",
            NamedKey::ArrowDown => "↓",
            NamedKey::ArrowLeft => "←",
            NamedKey::ArrowRight => "→",
            NamedKey::Plus => "+",
            NamedKey::Minus => "−",
            NamedKey::Comma => ",",
            NamedKey::Home => "Home",
            NamedKey::End => "End",
            NamedKey::PageUp => "PgUp",
            NamedKey::PageDown => "PgDn",
        }
    }

    fn from_token(s: &str) -> Option<NamedKey> {
        let k = match s.to_ascii_lowercase().as_str() {
            "enter" | "return" | "↵" => NamedKey::Enter,
            "escape" | "esc" | "⎋" => NamedKey::Escape,
            "space" | "␣" => NamedKey::Space,
            "tab" | "⇥" => NamedKey::Tab,
            "backspace" | "⌫" => NamedKey::Backspace,
            "delete" | "del" | "⌦" => NamedKey::Delete,
            "arrowup" | "up" | "↑" => NamedKey::ArrowUp,
            "arrowdown" | "down" | "↓" => NamedKey::ArrowDown,
            "arrowleft" | "left" | "←" => NamedKey::ArrowLeft,
            "arrowright" | "right" | "→" => NamedKey::ArrowRight,
            "plus" => NamedKey::Plus,
            "minus" => NamedKey::Minus,
            "comma" => NamedKey::Comma,
            "home" | "↖" => NamedKey::Home,
            "end" | "↘" => NamedKey::End,
            "pageup" | "pgup" | "⇞" => NamedKey::PageUp,
            "pagedown" | "pgdn" | "⇟" => NamedKey::PageDown,
            _ => return None,
        };
        Some(k)
    }
}

/// The non-modifier portion of a chord: either a printable character (stored
/// lowercased so `Shift` is the single source of truth for case) or a named
/// key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChordKey {
    Char(char),
    Named(NamedKey),
}

/// A keyboard shortcut: a set of modifiers plus one key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub mods: Mods,
    pub key: ChordKey,
}

impl KeyChord {
    /// Construct a chord from a single character (lowercased) and modifiers.
    pub fn char(c: char, mods: Mods) -> KeyChord {
        KeyChord {
            mods,
            key: ChordKey::Char(c.to_ascii_lowercase()),
        }
    }

    /// Construct a chord from a named key and modifiers.
    pub fn named(key: NamedKey, mods: Mods) -> KeyChord {
        KeyChord {
            mods,
            key: ChordKey::Named(key),
        }
    }

    /// Parse a chord from a `"+"`-separated spec such as `"Cmd+Shift+S"`,
    /// `"Ctrl+Alt+Enter"`, `"F"`, or `"Escape"`. Modifier and key tokens are
    /// case-insensitive; glyphs (`⌘ ⌥ ⇧`) are accepted too. Returns `None` for
    /// an empty spec, an unknown token, or a missing/duplicate key.
    pub fn parse(spec: &str) -> Option<KeyChord> {
        let mut mods = Mods::NONE;
        let mut key: Option<ChordKey> = None;
        for raw in spec.split('+') {
            let token = raw.trim();
            if token.is_empty() {
                continue;
            }
            if let Some(()) = apply_modifier(&mut mods, token) {
                continue;
            }
            // Not a modifier — must be the (single) key.
            if key.is_some() {
                return None;
            }
            if let Some(named) = NamedKey::from_token(token) {
                key = Some(ChordKey::Named(named));
            } else {
                let mut chars = token.chars();
                let c = chars.next()?;
                if chars.next().is_some() {
                    // Multi-character token that isn't a known named key.
                    return None;
                }
                key = Some(ChordKey::Char(c.to_ascii_lowercase()));
            }
        }
        Some(KeyChord { mods, key: key? })
    }

    /// Render the chord as macOS keycap glyphs in canonical order
    /// (⌃⌥⇧⌘ then the key), e.g. `KeyChord::char('s', Mods::cmd_shift())`
    /// → `"⇧⌘S"`.
    pub fn format_glyphs(self) -> String {
        let mut out = String::new();
        if self.mods.ctrl {
            out.push('⌃');
        }
        if self.mods.alt {
            out.push('⌥');
        }
        if self.mods.shift {
            out.push('⇧');
        }
        if self.mods.cmd {
            out.push('⌘');
        }
        match self.key {
            ChordKey::Char(c) => out.extend(c.to_uppercase()),
            ChordKey::Named(n) => out.push_str(n.glyph()),
        }
        out
    }

    /// Render the chord with `"+"`-separated word tokens — the inverse of
    /// [`KeyChord::parse`] (round-trips for every chord this module builds).
    pub fn format_tokens(self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.mods.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.mods.alt {
            parts.push("Alt".to_string());
        }
        if self.mods.shift {
            parts.push("Shift".to_string());
        }
        if self.mods.cmd {
            parts.push("Cmd".to_string());
        }
        match self.key {
            ChordKey::Char(c) => parts.push(c.to_ascii_uppercase().to_string()),
            ChordKey::Named(n) => parts.push(n.token().to_string()),
        }
        parts.join("+")
    }
}

fn apply_modifier(mods: &mut Mods, token: &str) -> Option<()> {
    match token.to_ascii_lowercase().as_str() {
        "cmd" | "command" | "super" | "win" | "meta" | "⌘" => mods.cmd = true,
        "ctrl" | "control" | "⌃" => mods.ctrl = true,
        "alt" | "opt" | "option" | "⌥" => mods.alt = true,
        "shift" | "⇧" => mods.shift = true,
        _ => return None,
    }
    Some(())
}

impl std::fmt::Display for KeyChord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.format_glyphs())
    }
}

// ===========================================================================
// iced bridge
// ===========================================================================

impl KeyChord {
    /// Build a chord from a live iced key event, or `None` for keys the
    /// registry never binds (modifier-only presses, exotic named keys). The
    /// `cmd` modifier follows iced's [`Modifiers::command`], which already maps
    /// to ⌘ on macOS and Ctrl elsewhere.
    pub fn from_iced(key: &iced::keyboard::Key, modifiers: iced::keyboard::Modifiers) -> Option<KeyChord> {
        use iced::keyboard::key::Named as N;
        use iced::keyboard::Key;

        let mods = Mods {
            // `command()` is the platform-correct accelerator modifier.
            cmd: modifiers.command(),
            // Only surface a raw Ctrl when it isn't already standing in for
            // the command modifier (avoids double-counting on Windows/Linux).
            ctrl: modifiers.control() && !modifiers.command(),
            alt: modifiers.alt(),
            shift: modifiers.shift(),
        };

        let chord_key = match key {
            Key::Character(c) => {
                let ch = c.chars().next()?;
                if ch.is_whitespace() {
                    return None;
                }
                ChordKey::Char(ch.to_ascii_lowercase())
            }
            Key::Named(named) => ChordKey::Named(match named {
                N::Enter => NamedKey::Enter,
                N::Escape => NamedKey::Escape,
                N::Space => NamedKey::Space,
                N::Tab => NamedKey::Tab,
                N::Backspace => NamedKey::Backspace,
                N::Delete => NamedKey::Delete,
                N::ArrowUp => NamedKey::ArrowUp,
                N::ArrowDown => NamedKey::ArrowDown,
                N::ArrowLeft => NamedKey::ArrowLeft,
                N::ArrowRight => NamedKey::ArrowRight,
                N::Home => NamedKey::Home,
                N::End => NamedKey::End,
                N::PageUp => NamedKey::PageUp,
                N::PageDown => NamedKey::PageDown,
                _ => return None,
            }),
            _ => return None,
        };

        Some(KeyChord {
            mods,
            key: chord_key,
        })
    }
}
