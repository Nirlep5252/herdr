//! Translate winit input events into herdr [`ClientInputEvent`]s.
//!
//! The server reuses the same input pipeline as the terminal client, so the
//! goal here is to produce the crossterm-shaped key/mouse events the server
//! already understands. Modifier bytes use crossterm's `KeyModifiers` bit
//! layout, and shifted letters are reported as the uppercase character plus a
//! `SHIFT` modifier (matching the server's legacy convention).

use crossterm::event::KeyModifiers;
use winit::event::{ElementState, MouseButton};
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::protocol::{ClientInputEvent, ClientKeyCode, ClientKeyKind, ClientMouseButton};

/// Crossterm modifier bits for the current keyboard modifier state.
pub fn modifier_bits(state: ModifiersState) -> u8 {
    let mut mods = KeyModifiers::empty();
    if state.shift_key() {
        mods |= KeyModifiers::SHIFT;
    }
    if state.control_key() {
        mods |= KeyModifiers::CONTROL;
    }
    if state.alt_key() {
        mods |= KeyModifiers::ALT;
    }
    if state.super_key() {
        mods |= KeyModifiers::SUPER;
    }
    mods.bits()
}

fn non_shift_bits(state: ModifiersState) -> u8 {
    let mut mods = KeyModifiers::empty();
    if state.control_key() {
        mods |= KeyModifiers::CONTROL;
    }
    if state.alt_key() {
        mods |= KeyModifiers::ALT;
    }
    if state.super_key() {
        mods |= KeyModifiers::SUPER;
    }
    mods.bits()
}

/// Translates a winit logical key into a client key event.
///
/// Returns `None` for keys that have no terminal meaning (dead keys, bare
/// modifier presses, unidentified keys).
pub fn translate_key(
    logical: &Key,
    state: ModifiersState,
    element: ElementState,
    repeat: bool,
) -> Option<ClientInputEvent> {
    // The terminal pipeline only expects key-down/repeat events, matching how
    // a real terminal delivers input. Releases are dropped to avoid double
    // processing.
    if element != ElementState::Pressed {
        return None;
    }
    let kind = if repeat {
        ClientKeyKind::Repeat
    } else {
        ClientKeyKind::Press
    };

    match logical {
        Key::Named(named) => {
            let shift = state.shift_key();
            let code = match named {
                NamedKey::Enter => ClientKeyCode::Enter,
                NamedKey::Backspace => ClientKeyCode::Backspace,
                NamedKey::Tab if shift => ClientKeyCode::BackTab,
                NamedKey::Tab => ClientKeyCode::Tab,
                NamedKey::Space => ClientKeyCode::Char(' '),
                NamedKey::Escape => ClientKeyCode::Esc,
                NamedKey::ArrowLeft => ClientKeyCode::Left,
                NamedKey::ArrowRight => ClientKeyCode::Right,
                NamedKey::ArrowUp => ClientKeyCode::Up,
                NamedKey::ArrowDown => ClientKeyCode::Down,
                NamedKey::Home => ClientKeyCode::Home,
                NamedKey::End => ClientKeyCode::End,
                NamedKey::PageUp => ClientKeyCode::PageUp,
                NamedKey::PageDown => ClientKeyCode::PageDown,
                NamedKey::Delete => ClientKeyCode::Delete,
                NamedKey::Insert => ClientKeyCode::Insert,
                NamedKey::F1 => ClientKeyCode::F(1),
                NamedKey::F2 => ClientKeyCode::F(2),
                NamedKey::F3 => ClientKeyCode::F(3),
                NamedKey::F4 => ClientKeyCode::F(4),
                NamedKey::F5 => ClientKeyCode::F(5),
                NamedKey::F6 => ClientKeyCode::F(6),
                NamedKey::F7 => ClientKeyCode::F(7),
                NamedKey::F8 => ClientKeyCode::F(8),
                NamedKey::F9 => ClientKeyCode::F(9),
                NamedKey::F10 => ClientKeyCode::F(10),
                NamedKey::F11 => ClientKeyCode::F(11),
                NamedKey::F12 => ClientKeyCode::F(12),
                _ => return None,
            };
            Some(ClientInputEvent::Key {
                code,
                modifiers: modifier_bits(state),
                kind,
            })
        }
        Key::Character(text) => {
            let ch = text.chars().next()?;
            // Shift is already folded into the character. Keep an explicit
            // SHIFT only for alphabetic letters, matching the server's
            // uppercase-letter-plus-SHIFT convention; symbols carry their
            // shifted form in the character itself.
            let mut bits = non_shift_bits(state);
            if state.shift_key() && ch.is_ascii_alphabetic() {
                bits |= KeyModifiers::SHIFT.bits();
            }
            Some(ClientInputEvent::Key {
                code: ClientKeyCode::Char(ch),
                modifiers: bits,
                kind,
            })
        }
        _ => None,
    }
}

/// Maps a winit mouse button to the protocol button.
pub fn mouse_button(button: MouseButton) -> Option<ClientMouseButton> {
    match button {
        MouseButton::Left => Some(ClientMouseButton::Left),
        MouseButton::Right => Some(ClientMouseButton::Right),
        MouseButton::Middle => Some(ClientMouseButton::Middle),
        _ => None,
    }
}

/// Converts pixel coordinates into a clamped (column, row) cell address.
pub fn pixel_to_cell(
    x: f64,
    y: f64,
    cell_w: usize,
    cell_h: usize,
    cols: u16,
    rows: u16,
) -> (u16, u16) {
    if cell_w == 0 || cell_h == 0 || cols == 0 || rows == 0 {
        return (0, 0);
    }
    let col = (x as usize / cell_w).min(cols as usize - 1) as u16;
    let row = (y as usize / cell_h).min(rows as usize - 1) as u16;
    (col, row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifted_letter_reports_uppercase_with_shift() {
        let mut state = ModifiersState::empty();
        state |= ModifiersState::SHIFT;
        let event = translate_key(
            &Key::Character("N".into()),
            state,
            ElementState::Pressed,
            false,
        )
        .expect("event");
        match event {
            ClientInputEvent::Key {
                code, modifiers, ..
            } => {
                assert_eq!(code, ClientKeyCode::Char('N'));
                assert_eq!(
                    modifiers & KeyModifiers::SHIFT.bits(),
                    KeyModifiers::SHIFT.bits()
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn ctrl_b_carries_control_modifier() {
        let mut state = ModifiersState::empty();
        state |= ModifiersState::CONTROL;
        let event = translate_key(
            &Key::Character("b".into()),
            state,
            ElementState::Pressed,
            false,
        )
        .expect("event");
        match event {
            ClientInputEvent::Key {
                code, modifiers, ..
            } => {
                assert_eq!(code, ClientKeyCode::Char('b'));
                assert_eq!(
                    modifiers & KeyModifiers::CONTROL.bits(),
                    KeyModifiers::CONTROL.bits()
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn releases_are_dropped() {
        assert!(translate_key(
            &Key::Character("a".into()),
            ModifiersState::empty(),
            ElementState::Released,
            false,
        )
        .is_none());
    }

    #[test]
    fn shift_tab_becomes_backtab() {
        let mut state = ModifiersState::empty();
        state |= ModifiersState::SHIFT;
        let event = translate_key(
            &Key::Named(NamedKey::Tab),
            state,
            ElementState::Pressed,
            false,
        )
        .expect("event");
        match event {
            ClientInputEvent::Key { code, .. } => assert_eq!(code, ClientKeyCode::BackTab),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn pixel_to_cell_clamps() {
        assert_eq!(pixel_to_cell(0.0, 0.0, 8, 16, 80, 24), (0, 0));
        assert_eq!(pixel_to_cell(8.0, 16.0, 8, 16, 80, 24), (1, 1));
        assert_eq!(pixel_to_cell(100000.0, 100000.0, 8, 16, 80, 24), (79, 23));
    }
}
