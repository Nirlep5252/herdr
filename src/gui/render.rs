//! Pure rasterization of a [`FrameData`] cell grid into an RGB pixel buffer.
//!
//! The output buffer uses softbuffer's `0x00RRGGBB` layout so it can be
//! presented to a window directly or encoded to a PNG for headless tests.
//! This module performs no I/O and never touches the windowing system, so it
//! is fully testable without a display.

use ratatui::style::Modifier;

use crate::protocol::FrameData;

use super::color::{self, Rgb, DEFAULT_BG, DEFAULT_FG};
use super::font::{FontSet, Style};

/// Renders `frame` into `buffer` (`surface_w * surface_h` pixels, row-major).
///
/// Any surface area not covered by the cell grid is filled with the default
/// background so window sizes that are not an exact multiple of the cell size
/// still look clean.
pub fn render_frame(
    frame: &FrameData,
    fonts: &mut FontSet,
    buffer: &mut [u32],
    surface_w: usize,
    surface_h: usize,
) {
    let bg_fill = color::pack_rgb(DEFAULT_BG);
    for pixel in buffer.iter_mut() {
        *pixel = bg_fill;
    }

    let cell_w = fonts.cell_width();
    let cell_h = fonts.cell_height();
    let ascent = fonts.ascent();

    let width = frame.width as usize;
    let height = frame.height as usize;

    for row in 0..height {
        for col in 0..width {
            let idx = row * width + col;
            let Some(cell) = frame.cells.get(idx) else {
                continue;
            };

            let modifier = Modifier::from_bits_truncate(cell.modifier);
            let mut fg = color::decode(cell.fg, DEFAULT_FG);
            let mut bg = color::decode(cell.bg, DEFAULT_BG);

            if modifier.contains(Modifier::REVERSED) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if modifier.contains(Modifier::DIM) {
                fg = color::dim(fg, bg);
            }
            if modifier.contains(Modifier::HIDDEN) {
                fg = bg;
            }

            let x0 = col * cell_w;
            let y0 = row * cell_h;
            fill_rect(buffer, surface_w, surface_h, x0, y0, cell_w, cell_h, bg);

            let ch = cell.symbol.chars().next().unwrap_or(' ');
            if ch != ' ' && ch != '\0' && !modifier.contains(Modifier::HIDDEN) {
                let style = Style::from_attrs(
                    modifier.contains(Modifier::BOLD),
                    modifier.contains(Modifier::ITALIC),
                );
                draw_glyph(
                    buffer, surface_w, surface_h, fonts, ch, style, x0, y0, ascent, fg, bg,
                );
            }

            if modifier.contains(Modifier::UNDERLINED) {
                let y = y0 + (ascent as usize + 1).min(cell_h.saturating_sub(1));
                fill_rect(buffer, surface_w, surface_h, x0, y, cell_w, 1, fg);
            }
            if modifier.contains(Modifier::CROSSED_OUT) {
                let y = y0 + cell_h / 2;
                fill_rect(buffer, surface_w, surface_h, x0, y, cell_w, 1, fg);
            }
        }
    }

    draw_cursor(frame, fonts, buffer, surface_w, surface_h);
}

fn draw_cursor(
    frame: &FrameData,
    fonts: &mut FontSet,
    buffer: &mut [u32],
    surface_w: usize,
    surface_h: usize,
) {
    let Some(cursor) = frame.cursor.as_ref() else {
        return;
    };
    if !cursor.visible {
        return;
    }
    let width = frame.width as usize;
    let (col, row) = (cursor.x as usize, cursor.y as usize);
    let idx = row * width + col;
    let Some(cell) = frame.cells.get(idx) else {
        return;
    };

    let cell_w = fonts.cell_width();
    let cell_h = fonts.cell_height();
    let ascent = fonts.ascent();
    let x0 = col * cell_w;
    let y0 = row * cell_h;

    let fg = color::decode(cell.fg, DEFAULT_FG);
    let bg = color::decode(cell.bg, DEFAULT_BG);

    // DECSCUSR: 3/4 underline, 5/6 bar, everything else block.
    match cursor.shape {
        3 | 4 => {
            let h = (cell_h / 8).max(1);
            fill_rect(
                buffer,
                surface_w,
                surface_h,
                x0,
                y0 + cell_h - h,
                cell_w,
                h,
                fg,
            );
        }
        5 | 6 => {
            let w = (cell_w / 6).max(1);
            fill_rect(buffer, surface_w, surface_h, x0, y0, w, cell_h, fg);
        }
        _ => {
            // Block: paint the cell with the foreground color and redraw the
            // glyph in the background color (classic inverted-block cursor).
            fill_rect(buffer, surface_w, surface_h, x0, y0, cell_w, cell_h, fg);
            let ch = cell.symbol.chars().next().unwrap_or(' ');
            if ch != ' ' && ch != '\0' {
                let modifier = Modifier::from_bits_truncate(cell.modifier);
                let style = Style::from_attrs(
                    modifier.contains(Modifier::BOLD),
                    modifier.contains(Modifier::ITALIC),
                );
                draw_glyph(
                    buffer, surface_w, surface_h, fonts, ch, style, x0, y0, ascent, bg, fg,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_glyph(
    buffer: &mut [u32],
    surface_w: usize,
    surface_h: usize,
    fonts: &mut FontSet,
    ch: char,
    style: Style,
    cell_x: usize,
    cell_y: usize,
    ascent: i32,
    fg: Rgb,
    bg: Rgb,
) {
    let glyph = fonts.glyph(ch, style).clone();
    if glyph.width == 0 || glyph.height == 0 {
        return;
    }

    let top = cell_y as i32 + ascent - glyph.ymin - glyph.height as i32;
    let left = cell_x as i32 + glyph.xmin;

    for by in 0..glyph.height {
        for bx in 0..glyph.width {
            let coverage = glyph.coverage[by * glyph.width + bx];
            if coverage == 0 {
                continue;
            }
            let px = left + bx as i32;
            let py = top + by as i32;
            if px < 0 || py < 0 {
                continue;
            }
            let (px, py) = (px as usize, py as usize);
            if px >= surface_w || py >= surface_h {
                continue;
            }
            let blended = blend(bg, fg, coverage);
            buffer[py * surface_w + px] = color::pack_rgb(blended);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_rect(
    buffer: &mut [u32],
    surface_w: usize,
    surface_h: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    color: Rgb,
) {
    let packed = color::pack_rgb(color);
    let x_end = (x + w).min(surface_w);
    let y_end = (y + h).min(surface_h);
    for py in y..y_end {
        let row_start = py * surface_w;
        for px in x..x_end {
            buffer[row_start + px] = packed;
        }
    }
}

/// Alpha-blends `fg` over `bg` with coverage `a` (`0..=255`).
fn blend(bg: Rgb, fg: Rgb, a: u8) -> Rgb {
    let a = a as u32;
    let inv = 255 - a;
    let mix = |f: u8, b: u8| -> u8 { ((f as u32 * a + b as u32 * inv) / 255) as u8 };
    (mix(fg.0, bg.0), mix(fg.1, bg.1), mix(fg.2, bg.2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{CellData, CursorState};

    fn rgb_cell(symbol: &str, fg: Rgb, bg: Rgb) -> CellData {
        CellData {
            symbol: symbol.to_string(),
            fg: 0x02_00_00_00 | ((fg.0 as u32) << 16) | ((fg.1 as u32) << 8) | (fg.2 as u32),
            bg: 0x02_00_00_00 | ((bg.0 as u32) << 16) | ((bg.1 as u32) << 8) | (bg.2 as u32),
            modifier: 0,
            skip: false,
            hyperlink: None,
        }
    }

    #[test]
    fn fills_cell_background() {
        let mut fonts = FontSet::load(16.0).expect("fonts load");
        let (cw, ch) = (fonts.cell_width(), fonts.cell_height());
        let frame = FrameData {
            cells: vec![rgb_cell(" ", (255, 255, 255), (255, 0, 0))],
            width: 1,
            height: 1,
            cursor: None,
            hyperlinks: vec![],
            graphics: vec![],
        };
        let mut buf = vec![0u32; cw * ch];
        render_frame(&frame, &mut fonts, &mut buf, cw, ch);
        // A space cell should be entirely its red background.
        assert!(buf.iter().all(|&p| p == color::pack_rgb((255, 0, 0))));
    }

    #[test]
    fn glyph_produces_foreground_ink() {
        let mut fonts = FontSet::load(16.0).expect("fonts load");
        let (cw, ch) = (fonts.cell_width(), fonts.cell_height());
        let frame = FrameData {
            cells: vec![rgb_cell("M", (255, 255, 255), (0, 0, 0))],
            width: 1,
            height: 1,
            cursor: None,
            hyperlinks: vec![],
            graphics: vec![],
        };
        let mut buf = vec![0u32; cw * ch];
        render_frame(&frame, &mut fonts, &mut buf, cw, ch);
        let white = color::pack_rgb((255, 255, 255));
        assert!(buf.contains(&white), "letter M should paint white ink");
        assert!(
            buf.contains(&color::pack_rgb((0, 0, 0))),
            "background should remain black around the glyph"
        );
    }

    #[test]
    fn block_cursor_inverts_cell() {
        let mut fonts = FontSet::load(16.0).expect("fonts load");
        let (cw, ch) = (fonts.cell_width(), fonts.cell_height());
        let frame = FrameData {
            cells: vec![rgb_cell(" ", (255, 255, 255), (0, 0, 0))],
            width: 1,
            height: 1,
            cursor: Some(CursorState {
                x: 0,
                y: 0,
                visible: true,
                shape: 2,
            }),
            hyperlinks: vec![],
            graphics: vec![],
        };
        let mut buf = vec![0u32; cw * ch];
        render_frame(&frame, &mut fonts, &mut buf, cw, ch);
        // Block cursor fills the cell with the foreground (white).
        assert!(buf.iter().all(|&p| p == color::pack_rgb((255, 255, 255))));
    }
}
