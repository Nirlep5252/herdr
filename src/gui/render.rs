//! Rasterize a [`FrameData`] cell grid into an RGB pixel buffer.
//!
//! The output uses softbuffer's `0x00RRGGBB` layout. `render_frame` clears the
//! whole buffer and draws the grid at the origin (used by the faithful mirror
//! client and the snapshot path). `render_frame_at` draws a single terminal's
//! grid into a sub-rectangle without clearing, used by the native UI to place
//! each pane's live content inside its widget rect.

use ratatui::style::Modifier;

use crate::protocol::FrameData;

use super::color::{self, DEFAULT_BG, DEFAULT_FG};
use super::draw::Canvas;
use super::font::{FontSet, Style};

/// Renders `frame` into a full-surface buffer, clearing it first.
pub fn render_frame(
    frame: &FrameData,
    fonts: &mut FontSet,
    buffer: &mut [u32],
    surface_w: usize,
    surface_h: usize,
) {
    let mut canvas = Canvas::new(buffer, surface_w, surface_h);
    canvas.clear(DEFAULT_BG);
    draw_frame(frame, fonts, &mut canvas, 0, 0);
}

/// Draws `frame` into `canvas` with its top-left cell at pixel `(ox, oy)`.
/// Does not clear; callers paint the surrounding chrome first.
pub fn render_frame_at(
    frame: &FrameData,
    fonts: &mut FontSet,
    canvas: &mut Canvas,
    ox: i32,
    oy: i32,
) {
    draw_frame(frame, fonts, canvas, ox, oy);
}

fn draw_frame(frame: &FrameData, fonts: &mut FontSet, canvas: &mut Canvas, ox: i32, oy: i32) {
    let cell_w = fonts.cell_width() as i32;
    let cell_h = fonts.cell_height() as i32;
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

            let x0 = ox + col as i32 * cell_w;
            let y0 = oy + row as i32 * cell_h;
            canvas.fill_rect(x0, y0, cell_w, cell_h, bg);

            let ch = cell.symbol.chars().next().unwrap_or(' ');
            if !modifier.contains(Modifier::HIDDEN) {
                let style = Style::from_attrs(
                    modifier.contains(Modifier::BOLD),
                    modifier.contains(Modifier::ITALIC),
                );
                canvas.draw_glyph(fonts, ch, style, x0, y0, ascent, fg);
            }

            if modifier.contains(Modifier::UNDERLINED) {
                let y = y0 + (ascent + 1).min(cell_h - 1);
                canvas.fill_rect(x0, y, cell_w, 1, fg);
            }
            if modifier.contains(Modifier::CROSSED_OUT) {
                canvas.fill_rect(x0, y0 + cell_h / 2, cell_w, 1, fg);
            }
        }
    }

    draw_cursor(frame, fonts, canvas, ox, oy);
}

fn draw_cursor(frame: &FrameData, fonts: &mut FontSet, canvas: &mut Canvas, ox: i32, oy: i32) {
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

    let cell_w = fonts.cell_width() as i32;
    let cell_h = fonts.cell_height() as i32;
    let ascent = fonts.ascent();
    let x0 = ox + col as i32 * cell_w;
    let y0 = oy + row as i32 * cell_h;

    let fg = color::decode(cell.fg, DEFAULT_FG);
    let bg = color::decode(cell.bg, DEFAULT_BG);

    match cursor.shape {
        3 | 4 => {
            let h = (cell_h / 8).max(1);
            canvas.fill_rect(x0, y0 + cell_h - h, cell_w, h, fg);
        }
        5 | 6 => {
            let w = (cell_w / 6).max(1);
            canvas.fill_rect(x0, y0, w, cell_h, fg);
        }
        _ => {
            canvas.fill_rect(x0, y0, cell_w, cell_h, fg);
            let ch = cell.symbol.chars().next().unwrap_or(' ');
            let modifier = Modifier::from_bits_truncate(cell.modifier);
            let style = Style::from_attrs(
                modifier.contains(Modifier::BOLD),
                modifier.contains(Modifier::ITALIC),
            );
            canvas.draw_glyph(fonts, ch, style, x0, y0, ascent, bg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::color::Rgb;
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
    }

    #[test]
    fn renders_at_offset_into_larger_buffer() {
        let mut fonts = FontSet::load(16.0).expect("fonts load");
        let (cw, ch) = (fonts.cell_width(), fonts.cell_height());
        let frame = FrameData {
            cells: vec![rgb_cell(" ", (0, 0, 0), (0, 255, 0))],
            width: 1,
            height: 1,
            cursor: None,
            hyperlinks: vec![],
            graphics: vec![],
        };
        let surf_w = cw * 4;
        let surf_h = ch * 4;
        let mut buf = vec![0u32; surf_w * surf_h];
        {
            let mut canvas = Canvas::new(&mut buf, surf_w, surf_h);
            render_frame_at(&frame, &mut fonts, &mut canvas, cw as i32, ch as i32);
        }
        // Green cell drawn at (cw, ch); origin should be untouched (still 0).
        assert_eq!(buf[0], 0);
        assert_eq!(buf[ch * surf_w + cw], color::pack_rgb((0, 255, 0)));
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
        assert!(buf.iter().all(|&p| p == color::pack_rgb((255, 255, 255))));
    }
}
