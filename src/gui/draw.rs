//! Low-level CPU drawing primitives shared by the native chrome and the
//! terminal-pane renderer.
//!
//! A [`Canvas`] borrows a softbuffer-style `0x00RRGGBB` pixel buffer and offers
//! clipped fills, alpha blending, and glyph/text drawing on top of whatever is
//! already in the buffer. Keeping these primitives in one place lets the
//! native widgets and the cell renderer share the same font cache and blending.

use super::color::{self, Rgb};
use super::font::{FontSet, Style};

/// A mutable view over an RGB pixel buffer with clipped drawing helpers.
pub struct Canvas<'a> {
    buf: &'a mut [u32],
    pub width: usize,
    pub height: usize,
}

impl<'a> Canvas<'a> {
    pub fn new(buf: &'a mut [u32], width: usize, height: usize) -> Self {
        Self { buf, width, height }
    }

    pub fn clear(&mut self, rgb: Rgb) {
        let packed = color::pack_rgb(rgb);
        for pixel in self.buf.iter_mut() {
            *pixel = packed;
        }
    }

    #[inline]
    pub fn put(&mut self, x: i32, y: i32, packed: u32) {
        if x < 0 || y < 0 {
            return;
        }
        let (x, y) = (x as usize, y as usize);
        if x >= self.width || y >= self.height {
            return;
        }
        self.buf[y * self.width + x] = packed;
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: Rgb) {
        if w <= 0 || h <= 0 {
            return;
        }
        let packed = color::pack_rgb(rgb);
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = ((x + w).max(0) as usize).min(self.width);
        let y1 = ((y + h).max(0) as usize).min(self.height);
        for py in y0..y1 {
            let row = py * self.width;
            for px in x0..x1 {
                self.buf[row + px] = packed;
            }
        }
    }

    /// Draws a 1px-thick rectangle outline.
    pub fn stroke_rect(&mut self, x: i32, y: i32, w: i32, h: i32, thickness: i32, rgb: Rgb) {
        if w <= 0 || h <= 0 {
            return;
        }
        let t = thickness.max(1);
        self.fill_rect(x, y, w, t, rgb);
        self.fill_rect(x, y + h - t, w, t, rgb);
        self.fill_rect(x, y, t, h, rgb);
        self.fill_rect(x + w - t, y, t, h, rgb);
    }

    /// Blends `rgb` over the existing pixel at `(x, y)` with coverage `cov`.
    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, rgb: Rgb, cov: u8) {
        if cov == 0 || x < 0 || y < 0 {
            return;
        }
        let (xu, yu) = (x as usize, y as usize);
        if xu >= self.width || yu >= self.height {
            return;
        }
        let idx = yu * self.width + xu;
        let existing = self.buf[idx];
        let bg = (
            ((existing >> 16) & 0xFF) as u8,
            ((existing >> 8) & 0xFF) as u8,
            (existing & 0xFF) as u8,
        );
        let a = cov as u32;
        let inv = 255 - a;
        let mix = |f: u8, b: u8| -> u8 { ((f as u32 * a + b as u32 * inv) / 255) as u8 };
        self.buf[idx] = color::pack_rgb((mix(rgb.0, bg.0), mix(rgb.1, bg.1), mix(rgb.2, bg.2)));
    }

    /// Draws a single glyph with its top-left cell origin at `(cell_x, cell_y)`,
    /// blended over the existing buffer contents.
    pub fn draw_glyph(
        &mut self,
        fonts: &mut FontSet,
        ch: char,
        style: Style,
        cell_x: i32,
        cell_y: i32,
        ascent: i32,
        fg: Rgb,
    ) {
        if ch == ' ' || ch == '\0' {
            return;
        }
        let glyph = fonts.glyph(ch, style).clone();
        if glyph.width == 0 || glyph.height == 0 {
            return;
        }
        let top = cell_y + ascent - glyph.ymin - glyph.height as i32;
        let left = cell_x + glyph.xmin;
        for by in 0..glyph.height {
            for bx in 0..glyph.width {
                let cov = glyph.coverage[by * glyph.width + bx];
                if cov != 0 {
                    self.blend(left + bx as i32, top + by as i32, fg, cov);
                }
            }
        }
    }

    /// Draws a left-aligned text run starting at pixel `(x, y_top)`, advancing
    /// one cell width per character. Stops before exceeding `max_x`.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        fonts: &mut FontSet,
        x: i32,
        y_top: i32,
        text: &str,
        style: Style,
        fg: Rgb,
        max_x: i32,
    ) -> i32 {
        let cell_w = fonts.cell_width() as i32;
        let ascent = fonts.ascent();
        let mut pen = x;
        for ch in text.chars() {
            if pen + cell_w > max_x {
                break;
            }
            self.draw_glyph(fonts, ch, style, pen, y_top, ascent, fg);
            pen += cell_w;
        }
        pen
    }

    /// Draws a small filled status dot (used for agent state in the sidebar).
    pub fn dot(&mut self, cx: i32, cy: i32, radius: i32, rgb: Rgb) {
        let r2 = radius * radius;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy <= r2 {
                    self.put(cx + dx, cy + dy, color::pack_rgb(rgb));
                }
            }
        }
    }
}

/// Truncates `text` with an ellipsis so it fits within `max_cells` columns.
pub fn truncate_cells(text: &str, max_cells: usize) -> String {
    let count = text.chars().count();
    if count <= max_cells {
        return text.to_string();
    }
    if max_cells <= 1 {
        return text.chars().take(max_cells).collect();
    }
    let mut out: String = text.chars().take(max_cells - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_rect_clips_to_bounds() {
        let mut buf = vec![0u32; 4 * 4];
        let mut canvas = Canvas::new(&mut buf, 4, 4);
        canvas.fill_rect(2, 2, 10, 10, (255, 0, 0));
        // Only the bottom-right 2x2 should be red.
        assert_eq!(buf[3 * 4 + 3], color::pack_rgb((255, 0, 0)));
        assert_eq!(buf[0], 0);
    }

    #[test]
    fn truncate_adds_ellipsis() {
        assert_eq!(truncate_cells("hello", 10), "hello");
        assert_eq!(truncate_cells("hello world", 5), "hell…");
    }

    #[test]
    fn text_advances_and_clips() {
        let mut fonts = FontSet::load(16.0).expect("fonts");
        let cw = fonts.cell_width() as i32;
        let mut buf = vec![0u32; 200 * 40];
        let mut canvas = Canvas::new(&mut buf, 200, 40);
        let end = canvas.text(&mut fonts, 0, 0, "AB", Style::Regular, (255, 255, 255), 200);
        assert_eq!(end, cw * 2);
    }
}
