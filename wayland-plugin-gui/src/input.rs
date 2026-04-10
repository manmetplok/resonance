//! Translate SCTK pointer / keyboard events into `egui::Event` values.

use egui::{Modifiers, PointerButton, Pos2, Vec2};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym, Modifiers as SctkModifiers};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind};

/// Incremental input state owned by the window thread.
#[derive(Default)]
pub struct InputState {
    pointer_pos: Pos2,
    modifiers: Modifiers,
    scale: f32,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            pointer_pos: Pos2::ZERO,
            modifiers: Modifiers::default(),
            scale: 1.0,
        }
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale.max(0.1);
    }

    /// Convert a SCTK pointer event into zero or more egui events.
    pub fn process_pointer(&mut self, event: &PointerEvent, out: &mut Vec<egui::Event>) {
        let (sx, sy) = event.position;
        // SCTK gives coordinates in *surface-local logical* pixels. egui wants
        // the same logical points (we feed egui a pixels-per-point separately).
        self.pointer_pos = Pos2::new(sx as f32, sy as f32);

        match event.kind {
            PointerEventKind::Enter { .. } => {
                out.push(egui::Event::PointerMoved(self.pointer_pos));
            }
            PointerEventKind::Leave { .. } => {
                out.push(egui::Event::PointerGone);
            }
            PointerEventKind::Motion { .. } => {
                out.push(egui::Event::PointerMoved(self.pointer_pos));
            }
            PointerEventKind::Press { button, .. } => {
                if let Some(btn) = map_pointer_button(button) {
                    out.push(egui::Event::PointerButton {
                        pos: self.pointer_pos,
                        button: btn,
                        pressed: true,
                        modifiers: self.modifiers,
                    });
                }
            }
            PointerEventKind::Release { button, .. } => {
                if let Some(btn) = map_pointer_button(button) {
                    out.push(egui::Event::PointerButton {
                        pos: self.pointer_pos,
                        button: btn,
                        pressed: false,
                        modifiers: self.modifiers,
                    });
                }
            }
            PointerEventKind::Axis {
                horizontal,
                vertical,
                ..
            } => {
                // SCTK gives line-based deltas via `discrete` and pixel deltas
                // via `absolute`. egui wants scroll in points; we treat line
                // deltas as ~50 px each, absolute deltas as pixels.
                let mut dx = horizontal.absolute as f32;
                let mut dy = vertical.absolute as f32;
                if horizontal.discrete != 0 && dx == 0.0 {
                    dx = horizontal.discrete as f32 * 50.0;
                }
                if vertical.discrete != 0 && dy == 0.0 {
                    dy = vertical.discrete as f32 * 50.0;
                }
                // egui's convention: positive = scroll up / right.
                let delta = Vec2::new(-dx, -dy);
                if delta.x != 0.0 || delta.y != 0.0 {
                    out.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta,
                        phase: egui::TouchPhase::Move,
                        modifiers: self.modifiers,
                    });
                }
            }
        }
    }

    pub fn set_modifiers(&mut self, modifiers: SctkModifiers) {
        self.modifiers = Modifiers {
            alt: modifiers.alt,
            ctrl: modifiers.ctrl,
            shift: modifiers.shift,
            mac_cmd: false,
            command: modifiers.ctrl,
        };
    }

    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    pub fn process_key(&mut self, event: &KeyEvent, pressed: bool, out: &mut Vec<egui::Event>) {
        if let Some(key) = map_keysym(event.keysym) {
            out.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: self.modifiers,
            });
        }
        if pressed {
            if let Some(text) = event.utf8.as_ref() {
                if !text.is_empty() && !self.modifiers.ctrl && !self.modifiers.alt {
                    // Only generate Text for printable input.
                    if text.chars().all(|c| !c.is_control()) {
                        out.push(egui::Event::Text(text.clone()));
                    }
                }
            }
        }
    }
}

fn map_pointer_button(raw: u32) -> Option<PointerButton> {
    // Linux input event codes (from <linux/input-event-codes.h>).
    match raw {
        0x110 => Some(PointerButton::Primary),   // BTN_LEFT
        0x111 => Some(PointerButton::Secondary), // BTN_RIGHT
        0x112 => Some(PointerButton::Middle),    // BTN_MIDDLE
        0x113 => Some(PointerButton::Extra1),    // BTN_SIDE
        0x114 => Some(PointerButton::Extra2),    // BTN_EXTRA
        _ => None,
    }
}

fn map_keysym(sym: Keysym) -> Option<egui::Key> {
    use egui::Key;
    // xkbcommon Keysym values via constants re-exported by SCTK.
    Some(match sym {
        Keysym::Return | Keysym::KP_Enter => Key::Enter,
        Keysym::Escape => Key::Escape,
        Keysym::Tab => Key::Tab,
        Keysym::BackSpace => Key::Backspace,
        Keysym::Delete => Key::Delete,
        Keysym::Insert => Key::Insert,
        Keysym::Left => Key::ArrowLeft,
        Keysym::Right => Key::ArrowRight,
        Keysym::Up => Key::ArrowUp,
        Keysym::Down => Key::ArrowDown,
        Keysym::Home => Key::Home,
        Keysym::End => Key::End,
        Keysym::Page_Up => Key::PageUp,
        Keysym::Page_Down => Key::PageDown,
        Keysym::space => Key::Space,
        Keysym::a | Keysym::A => Key::A,
        Keysym::b | Keysym::B => Key::B,
        Keysym::c | Keysym::C => Key::C,
        Keysym::d | Keysym::D => Key::D,
        Keysym::e | Keysym::E => Key::E,
        Keysym::f | Keysym::F => Key::F,
        Keysym::g | Keysym::G => Key::G,
        Keysym::h | Keysym::H => Key::H,
        Keysym::i | Keysym::I => Key::I,
        Keysym::j | Keysym::J => Key::J,
        Keysym::k | Keysym::K => Key::K,
        Keysym::l | Keysym::L => Key::L,
        Keysym::m | Keysym::M => Key::M,
        Keysym::n | Keysym::N => Key::N,
        Keysym::o | Keysym::O => Key::O,
        Keysym::p | Keysym::P => Key::P,
        Keysym::q | Keysym::Q => Key::Q,
        Keysym::r | Keysym::R => Key::R,
        Keysym::s | Keysym::S => Key::S,
        Keysym::t | Keysym::T => Key::T,
        Keysym::u | Keysym::U => Key::U,
        Keysym::v | Keysym::V => Key::V,
        Keysym::w | Keysym::W => Key::W,
        Keysym::x | Keysym::X => Key::X,
        Keysym::y | Keysym::Y => Key::Y,
        Keysym::z | Keysym::Z => Key::Z,
        Keysym::_0 | Keysym::KP_0 => Key::Num0,
        Keysym::_1 | Keysym::KP_1 => Key::Num1,
        Keysym::_2 | Keysym::KP_2 => Key::Num2,
        Keysym::_3 | Keysym::KP_3 => Key::Num3,
        Keysym::_4 | Keysym::KP_4 => Key::Num4,
        Keysym::_5 | Keysym::KP_5 => Key::Num5,
        Keysym::_6 | Keysym::KP_6 => Key::Num6,
        Keysym::_7 | Keysym::KP_7 => Key::Num7,
        Keysym::_8 | Keysym::KP_8 => Key::Num8,
        Keysym::_9 | Keysym::KP_9 => Key::Num9,
        _ => return None,
    })
}
