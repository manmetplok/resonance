//! Group identity colour palette for track grouping (epic #36, doc #200).
//!
//! A track group brackets related lanes under one header with a colour
//! *identity* — a swatch in the group header and a coloured rail down the left
//! edge of every member's header cell. The palette is deliberately small and
//! used for identity only, never semantics.
//!
//! This module is the single source of truth for the *base* RGB of each palette
//! entry. The dark-lavender-tuned wash (14% alpha) and line (40% alpha)
//! variants the prototype calls for are render-time treatments and live in the
//! app's `theme.rs` as canonical tokens (doc #200, decision 2) — they are
//! derived from these base colours, not re-spelled.

use serde::{Deserialize, Serialize};
use std::fmt;

/// An opaque RGB colour, stored packed as `0xRRGGBB`.
///
/// `resonance-common` sits below the app in the dependency graph and cannot
/// reference `iced::Color`, so the palette is expressed as plain RGB; the view
/// layer converts to its own colour type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GroupColor(pub u32);

impl GroupColor {
    /// Pack RGB components into a colour.
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        GroupColor(((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
    }

    /// Red component.
    pub const fn r(self) -> u8 {
        ((self.0 >> 16) & 0xFF) as u8
    }

    /// Green component.
    pub const fn g(self) -> u8 {
        ((self.0 >> 8) & 0xFF) as u8
    }

    /// Blue component.
    pub const fn b(self) -> u8 {
        (self.0 & 0xFF) as u8
    }
}

impl fmt::Display for GroupColor {
    /// CSS-style hex, e.g. `#c98f5f`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:06x}", self.0 & 0xFF_FFFF)
    }
}

/// The fixed identity palette assigned to track groups on creation (cycling,
/// user-recolourable). Muted jewel tones tuned to the dark lavender backdrop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum GroupIdentityColor {
    /// Muted amber. (`#c98f5f`)
    #[default]
    Drum,
    /// Muted rose. (`#c97b9c`)
    Vocal,
    /// Muted teal. (`#6fb6b0`)
    Keys,
    /// Muted periwinkle. (`#7d86c9`)
    Guitar,
}

impl GroupIdentityColor {
    /// Base RGB for this identity. Matches the prototype palette
    /// (`design/track-grouping-folder-tracks/index.html`).
    pub const fn color(self) -> GroupColor {
        match self {
            GroupIdentityColor::Drum => GroupColor::new(0xc9, 0x8f, 0x5f),
            GroupIdentityColor::Vocal => GroupColor::new(0xc9, 0x7b, 0x9c),
            GroupIdentityColor::Keys => GroupColor::new(0x6f, 0xb6, 0xb0),
            GroupIdentityColor::Guitar => GroupColor::new(0x7d, 0x86, 0xc9),
        }
    }

    /// Every palette entry, in cycle order.
    pub const fn all() -> &'static [GroupIdentityColor] {
        &[
            GroupIdentityColor::Drum,
            GroupIdentityColor::Vocal,
            GroupIdentityColor::Keys,
            GroupIdentityColor::Guitar,
        ]
    }

    /// The next entry, wrapping around — used to pick a fresh swatch as groups
    /// are created.
    pub const fn next(self) -> GroupIdentityColor {
        match self {
            GroupIdentityColor::Drum => GroupIdentityColor::Vocal,
            GroupIdentityColor::Vocal => GroupIdentityColor::Keys,
            GroupIdentityColor::Keys => GroupIdentityColor::Guitar,
            GroupIdentityColor::Guitar => GroupIdentityColor::Drum,
        }
    }
}

impl fmt::Display for GroupIdentityColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            GroupIdentityColor::Drum => "Drum",
            GroupIdentityColor::Vocal => "Vocal",
            GroupIdentityColor::Keys => "Keys",
            GroupIdentityColor::Guitar => "Guitar",
        };
        f.write_str(name)
    }
}

impl From<GroupIdentityColor> for GroupColor {
    fn from(identity: GroupIdentityColor) -> Self {
        identity.color()
    }
}
