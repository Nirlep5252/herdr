//! Decode herdr's packed-`u32` wire colors into concrete RGB triples.
//!
//! The server encodes each cell color with [`crate::protocol`]'s scheme:
//! - tag `0x00`: a named ratatui color in the low byte (`0x00` = Reset,
//!   `0x01..=0x10` = the 16 ANSI names), where `Reset` defers to the
//!   client's default foreground/background.
//! - tag `0x01`: an indexed (xterm 256) color in the low byte.
//! - tag `0x02`: a 24-bit RGB color in the low three bytes.
//!
//! A real terminal lets the host decide what `Reset` and the named/indexed
//! colors look like. The GUI client *is* the terminal, so it owns that
//! palette here. The defaults track herdr's default Catppuccin theme so that
//! `Reset` cells blend with the rendered chrome.

/// RGB color as three 8-bit channels.
pub type Rgb = (u8, u8, u8);

/// Default foreground used for `Reset` foreground cells.
pub const DEFAULT_FG: Rgb = (205, 214, 244);
/// Default background used for `Reset` background cells.
pub const DEFAULT_BG: Rgb = (30, 30, 46);

/// The 16 ANSI colors (xterm-style defaults), indexed `0..=15`.
const ANSI_16: [Rgb; 16] = [
    (0, 0, 0),       // 0 black
    (205, 0, 0),     // 1 red
    (0, 205, 0),     // 2 green
    (205, 205, 0),   // 3 yellow
    (0, 0, 238),     // 4 blue
    (205, 0, 205),   // 5 magenta
    (0, 205, 205),   // 6 cyan
    (229, 229, 229), // 7 white / gray
    (127, 127, 127), // 8 bright black / dark gray
    (255, 0, 0),     // 9 bright red
    (0, 255, 0),     // 10 bright green
    (255, 255, 0),   // 11 bright yellow
    (92, 92, 255),   // 12 bright blue
    (255, 0, 255),   // 13 bright magenta
    (0, 255, 255),   // 14 bright cyan
    (255, 255, 255), // 15 bright white
];

/// One step of the 6-level xterm color cube.
fn cube_component(value: u8) -> u8 {
    if value == 0 {
        0
    } else {
        55 + value * 40
    }
}

/// Converts an xterm 256-color index to RGB.
fn indexed_to_rgb(index: u8) -> Rgb {
    match index {
        0..=15 => ANSI_16[index as usize],
        16..=231 => {
            let i = index - 16;
            let r = i / 36;
            let g = (i % 36) / 6;
            let b = i % 6;
            (cube_component(r), cube_component(g), cube_component(b))
        }
        _ => {
            // Grayscale ramp 232..=255 -> 8, 18, ... 238.
            let level = 8 + 10 * (index - 232);
            (level, level, level)
        }
    }
}

/// Decodes a packed wire color into RGB, falling back to `default` when the
/// color resolves to `Reset`.
pub fn decode(packed: u32, default: Rgb) -> Rgb {
    match packed >> 24 {
        0x00 => match (packed & 0xFF) as u8 {
            0x00 => default,
            named @ 0x01..=0x10 => ANSI_16[(named - 1) as usize],
            _ => default,
        },
        0x01 => indexed_to_rgb((packed & 0xFF) as u8),
        0x02 => {
            let r = ((packed >> 16) & 0xFF) as u8;
            let g = ((packed >> 8) & 0xFF) as u8;
            let b = (packed & 0xFF) as u8;
            (r, g, b)
        }
        _ => default,
    }
}

/// Packs an RGB triple into the `0x00RRGGBB` layout softbuffer expects.
pub fn pack_rgb((r, g, b): Rgb) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// Blends `fg` toward `bg` by half, used for the `DIM` modifier.
pub fn dim(fg: Rgb, bg: Rgb) -> Rgb {
    (
        ((fg.0 as u16 + bg.0 as u16) / 2) as u8,
        ((fg.1 as u16 + bg.1 as u16) / 2) as u8,
        ((fg.2 as u16 + bg.2 as u16) / 2) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_tag_decodes_directly() {
        let packed = 0x02_00_00_00 | (0x12 << 16) | (0x34 << 8) | 0x56;
        assert_eq!(decode(packed, DEFAULT_FG), (0x12, 0x34, 0x56));
    }

    #[test]
    fn reset_uses_default() {
        assert_eq!(decode(0x00_00_00_00, DEFAULT_BG), DEFAULT_BG);
        assert_eq!(decode(0x00_00_00_00, DEFAULT_FG), DEFAULT_FG);
    }

    #[test]
    fn named_colors_map_to_ansi16() {
        // 0x02 in the named slot is ratatui Red -> ANSI index 1.
        assert_eq!(decode(0x00_00_00_02, DEFAULT_FG), ANSI_16[1]);
        // 0x10 is White -> ANSI index 15.
        assert_eq!(decode(0x00_00_00_10, DEFAULT_FG), ANSI_16[15]);
    }

    #[test]
    fn indexed_cube_and_grayscale() {
        // 16 is the first cube entry -> (0,0,0).
        assert_eq!(decode(0x01_00_00_00 | 16, DEFAULT_FG), (0, 0, 0));
        // 231 is the last cube entry -> (255,255,255).
        assert_eq!(decode(0x01_00_00_00 | 231, DEFAULT_FG), (255, 255, 255));
        // 232 is the first grayscale step -> 8.
        assert_eq!(decode(0x01_00_00_00 | 232, DEFAULT_FG), (8, 8, 8));
    }

    #[test]
    fn pack_round_trips_channels() {
        assert_eq!(pack_rgb((0x12, 0x34, 0x56)), 0x123456);
    }
}
