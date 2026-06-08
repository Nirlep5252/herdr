//! Monospace font loading, metrics, and a glyph rasterization cache.
//!
//! The GUI renders a fixed cell grid, so it only needs a single monospace
//! family in four styles (regular, bold, italic, bold-italic). DejaVu Sans
//! Mono is bundled so rendering is deterministic and identical across
//! platforms regardless of installed system fonts.

use std::collections::HashMap;

use fontdue::{Font, FontSettings};

const REGULAR_TTF: &[u8] = include_bytes!("../../assets/fonts/DejaVuSansMono.ttf");
const BOLD_TTF: &[u8] = include_bytes!("../../assets/fonts/DejaVuSansMono-Bold.ttf");
const OBLIQUE_TTF: &[u8] = include_bytes!("../../assets/fonts/DejaVuSansMono-Oblique.ttf");
const BOLD_OBLIQUE_TTF: &[u8] = include_bytes!("../../assets/fonts/DejaVuSansMono-BoldOblique.ttf");

/// Font style selector, used to index into the loaded family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Style {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

impl Style {
    pub fn from_attrs(bold: bool, italic: bool) -> Self {
        match (bold, italic) {
            (false, false) => Style::Regular,
            (true, false) => Style::Bold,
            (false, true) => Style::Italic,
            (true, true) => Style::BoldItalic,
        }
    }

    fn index(self) -> usize {
        match self {
            Style::Regular => 0,
            Style::Bold => 1,
            Style::Italic => 2,
            Style::BoldItalic => 3,
        }
    }
}

/// A rasterized glyph: 8-bit coverage plus placement metrics.
#[derive(Clone)]
pub struct Glyph {
    /// Coverage bitmap width in pixels.
    pub width: usize,
    /// Coverage bitmap height in pixels.
    pub height: usize,
    /// Horizontal offset from the cell origin to the bitmap's left edge.
    pub xmin: i32,
    /// Vertical offset from the baseline to the bitmap's bottom edge.
    pub ymin: i32,
    /// Per-pixel coverage (`0..=255`), row-major, `width * height` long.
    pub coverage: Vec<u8>,
}

/// A monospace font family with cached glyph rasterizations.
pub struct FontSet {
    fonts: [Font; 4],
    px: f32,
    cell_w: usize,
    cell_h: usize,
    ascent: i32,
    cache: HashMap<(char, usize), Glyph>,
}

impl FontSet {
    /// Loads the bundled DejaVu Sans Mono family at the given pixel size.
    pub fn load(px: f32) -> Result<Self, String> {
        let load = |bytes: &[u8]| -> Result<Font, String> {
            Font::from_bytes(bytes, FontSettings::default())
                .map_err(|err| format!("failed to load bundled font: {err}"))
        };
        let fonts = [
            load(REGULAR_TTF)?,
            load(BOLD_TTF)?,
            load(OBLIQUE_TTF)?,
            load(BOLD_OBLIQUE_TTF)?,
        ];

        // Cell width comes from the advance of a representative glyph; the
        // family is monospace so every glyph shares the same advance.
        let regular = &fonts[0];
        let advance = regular.metrics('M', px).advance_width;
        let cell_w = advance.ceil().max(1.0) as usize;

        let line = regular
            .horizontal_line_metrics(px)
            .ok_or_else(|| "font is missing horizontal line metrics".to_string())?;
        let cell_h = (line.ascent - line.descent + line.line_gap).ceil().max(1.0) as usize;
        let ascent = line.ascent.ceil() as i32;

        Ok(Self {
            fonts,
            px,
            cell_w,
            cell_h,
            ascent,
            cache: HashMap::new(),
        })
    }

    pub fn cell_width(&self) -> usize {
        self.cell_w
    }

    pub fn cell_height(&self) -> usize {
        self.cell_h
    }

    /// Baseline offset (in pixels) from the top of a cell.
    pub fn ascent(&self) -> i32 {
        self.ascent
    }

    /// Rasterizes (or returns a cached) glyph for `ch` in the given style.
    pub fn glyph(&mut self, ch: char, style: Style) -> &Glyph {
        let key = (ch, style.index());
        if !self.cache.contains_key(&key) {
            let (metrics, coverage) = self.fonts[style.index()].rasterize(ch, self.px);
            let glyph = Glyph {
                width: metrics.width,
                height: metrics.height,
                xmin: metrics.xmin,
                ymin: metrics.ymin,
                coverage,
            };
            self.cache.insert(key, glyph);
        }
        self.cache
            .get(&key)
            .expect("glyph inserted above is present")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_family_and_reports_sane_metrics() {
        let fonts = FontSet::load(16.0).expect("bundled fonts load");
        assert!(fonts.cell_width() > 0);
        assert!(fonts.cell_height() > 0);
        assert!(fonts.ascent() > 0);
        assert!(fonts.ascent() as usize <= fonts.cell_height());
    }

    #[test]
    fn rasterizes_a_visible_glyph() {
        let mut fonts = FontSet::load(16.0).expect("bundled fonts load");
        let glyph = fonts.glyph('M', Style::Regular).clone();
        assert!(glyph.width > 0 && glyph.height > 0);
        assert!(
            glyph.coverage.iter().any(|&c| c > 0),
            "'M' should produce ink"
        );
    }

    #[test]
    fn space_has_no_ink() {
        let mut fonts = FontSet::load(16.0).expect("bundled fonts load");
        let glyph = fonts.glyph(' ', Style::Regular).clone();
        assert!(glyph.coverage.iter().all(|&c| c == 0));
    }
}
