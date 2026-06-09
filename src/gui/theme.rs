//! Chrome color palette for the native GUI, derived from Herdr theme config.

use ratatui::style::Color;

use crate::app::state::Palette;
use crate::config::Config;

use super::color::Rgb;

/// RGB colors for native chrome widgets (sidebar, tabs, modals).
#[derive(Debug, Clone)]
pub struct ChromePalette {
    pub base: Rgb,
    pub mantle: Rgb,
    pub crust: Rgb,
    pub surface0: Rgb,
    pub surface1: Rgb,
    pub text: Rgb,
    pub subtext: Rgb,
    pub overlay: Rgb,
    pub accent: Rgb,
    pub red: Rgb,
    pub yellow: Rgb,
    pub blue: Rgb,
    pub green: Rgb,
}

impl ChromePalette {
    pub fn from_config(config: &Config) -> Self {
        let name = config.theme.name.as_deref().unwrap_or("catppuccin");
        let palette = Palette::from_name(name).unwrap_or_else(Palette::catppuccin);
        let palette = match config.theme.custom.as_ref() {
            Some(custom) => palette.with_overrides(custom),
            None => palette,
        };
        Self::from_palette(&palette)
    }

    pub fn from_palette(palette: &Palette) -> Self {
        Self {
            base: color_to_rgb(palette.surface_dim),
            mantle: color_to_rgb(palette.panel_bg),
            crust: color_to_rgb(darker(palette.panel_bg)),
            surface0: color_to_rgb(palette.surface0),
            surface1: color_to_rgb(palette.surface1),
            text: color_to_rgb(palette.text),
            subtext: color_to_rgb(palette.subtext0),
            overlay: color_to_rgb(palette.overlay0),
            accent: color_to_rgb(palette.accent),
            red: color_to_rgb(palette.red),
            yellow: color_to_rgb(palette.yellow),
            blue: color_to_rgb(palette.blue),
            green: color_to_rgb(palette.green),
        }
    }
}

fn color_to_rgb(color: Color) -> Rgb {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::Red => (205, 49, 49),
        Color::Green => (13, 188, 121),
        Color::Yellow => (229, 229, 16),
        Color::Blue => (36, 114, 200),
        Color::Magenta => (188, 63, 188),
        Color::Cyan => (17, 168, 205),
        Color::Gray => (128, 128, 128),
        Color::DarkGray => (95, 95, 95),
        Color::LightRed => (241, 76, 76),
        Color::LightGreen => (35, 209, 139),
        Color::LightYellow => (245, 245, 67),
        Color::LightBlue => (59, 142, 234),
        Color::LightMagenta => (210, 84, 210),
        Color::LightCyan => (41, 184, 219),
        Color::White => (229, 229, 229),
        Color::Indexed(_) | Color::Reset => (205, 214, 244),
    }
}

fn darker(color: Color) -> Color {
    let (r, g, b) = color_to_rgb(color);
    Color::Rgb(
        (r as u16 * 8 / 10).min(255) as u8,
        (g as u16 * 8 / 10).min(255) as u8,
        (b as u16 * 8 / 10).min(255) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_palette_from_default_config() {
        let palette = ChromePalette::from_config(&Config::default());
        assert_eq!(palette.accent, (137, 180, 250));
    }
}
