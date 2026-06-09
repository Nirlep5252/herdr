//! Settings overlay for the native GUI client.

use crate::app::state::THEME_NAMES;
use crate::config::{config_path, upsert_section_value, Config, GuiConfig};
use std::fs;

use super::super::draw::{self, Canvas};
use super::super::font::{FontSet, Style};
use super::super::theme::ChromePalette;
use super::layout::Rect;

pub const FONT_SIZE_CHOICES: &[f32] = &[12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 20.0, 22.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    Theme,
    FontSize,
}

impl SettingsSection {}

#[derive(Debug, Clone)]
pub struct SettingsOverlay {
    pub open: bool,
    pub section: SettingsSection,
    pub selected: usize,
    pub draft_theme: String,
    pub draft_font_size: f32,
    pub saved_theme: String,
    pub saved_font_size: f32,
}

impl SettingsOverlay {
    pub fn from_config(config: &Config) -> Self {
        let theme = config
            .theme
            .name
            .clone()
            .unwrap_or_else(|| "catppuccin".to_string());
        let font_size = config.gui.font_size.clamp(10.0, 28.0);
        Self {
            open: false,
            section: SettingsSection::Theme,
            selected: theme_index(&theme),
            draft_theme: theme.clone(),
            draft_font_size: font_size,
            saved_theme: theme,
            saved_font_size: font_size,
        }
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.draft_theme != self.saved_theme || self.draft_font_size != self.saved_font_size
    }

    pub fn item_count(&self) -> usize {
        match self.section {
            SettingsSection::Theme => THEME_NAMES.len(),
            SettingsSection::FontSize => FONT_SIZE_CHOICES.len(),
        }
    }

    pub fn move_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_next(&mut self) {
        if self.item_count() > 0 {
            self.selected = (self.selected + 1).min(self.item_count() - 1);
        }
    }

    pub fn select_tab(&mut self, section: SettingsSection) {
        self.section = section;
        self.selected = match section {
            SettingsSection::Theme => theme_index(&self.draft_theme),
            SettingsSection::FontSize => font_size_index(self.draft_font_size),
        };
    }

    pub fn apply_selection(&mut self) {
        match self.section {
            SettingsSection::Theme => {
                if let Some(name) = THEME_NAMES.get(self.selected) {
                    self.draft_theme = (*name).to_string();
                }
            }
            SettingsSection::FontSize => {
                if let Some(size) = FONT_SIZE_CHOICES.get(self.selected) {
                    self.draft_font_size = *size;
                }
            }
        }
    }

    pub fn save(&mut self) -> bool {
        self.apply_selection();
        let theme = self.draft_theme.clone();
        let font_size = self.draft_font_size;
        if !write_settings(&theme, font_size) {
            return false;
        }
        self.saved_theme = theme;
        self.saved_font_size = font_size;
        true
    }

    pub fn cancel(&mut self) {
        self.draft_theme = self.saved_theme.clone();
        self.draft_font_size = self.saved_font_size;
        self.selected = match self.section {
            SettingsSection::Theme => theme_index(&self.saved_theme),
            SettingsSection::FontSize => font_size_index(self.saved_font_size),
        };
        self.open = false;
    }

    pub fn close_saved(&mut self) {
        self.saved_theme = self.draft_theme.clone();
        self.saved_font_size = self.draft_font_size;
        self.open = false;
    }
}

pub fn load_gui_config() -> (Config, GuiConfig, ChromePalette) {
    let loaded = Config::load();
    let config = loaded.config;
    let gui = config.gui;
    let palette = ChromePalette::from_config(&config);
    (config, gui, palette)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_overlay_tracks_unsaved_changes() {
        let mut settings = SettingsOverlay::from_config(&Config::default());
        assert!(!settings.has_unsaved_changes());
        settings.draft_theme = "dracula".to_string();
        assert!(settings.has_unsaved_changes());
    }
}

fn write_settings(theme: &str, font_size: f32) -> bool {
    let path = config_path();
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    let content = fs::read_to_string(&path).unwrap_or_default();
    let content = upsert_section_value(&content, "theme", "name", &format!("\"{theme}\""));
    let content = upsert_section_value(
        &content,
        "gui",
        "font_size",
        &format!("{:.1}", font_size.clamp(10.0, 28.0)),
    );
    fs::write(path, content).is_ok()
}

fn theme_index(name: &str) -> usize {
    THEME_NAMES
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(name))
        .unwrap_or(0)
}

fn font_size_index(size: f32) -> usize {
    FONT_SIZE_CHOICES
        .iter()
        .position(|candidate| (*candidate - size).abs() < 0.01)
        .unwrap_or(3)
}

#[derive(Debug, Clone, Copy)]
pub struct SettingsLayout {
    pub rect: Rect,
    pub close: Rect,
    pub apply: Rect,
    pub tab_theme: Rect,
    pub tab_font: Rect,
    pub list: Rect,
}

pub fn layout(surface_w: i32, surface_h: i32) -> SettingsLayout {
    let popup_w = 520.min(surface_w - 40).max(320);
    let popup_h = 360.min(surface_h - 40).max(240);
    let x = (surface_w - popup_w) / 2;
    let y = (surface_h - popup_h) / 2;
    let rect = Rect::new(x, y, popup_w, popup_h);
    let pad = 16;
    let footer_h = 34;
    let header_h = 72;
    let close = Rect::new(
        rect.x + rect.w - pad - 72,
        rect.y + rect.h - footer_h,
        72,
        24,
    );
    let apply = Rect::new(close.x - 8 - 72, close.y, 72, 24);
    let tab_y = rect.y + 40;
    let tab_w = (popup_w - pad * 2 - 8) / 2;
    SettingsLayout {
        rect,
        close,
        apply,
        tab_theme: Rect::new(rect.x + pad, tab_y, tab_w, 24),
        tab_font: Rect::new(rect.x + pad + tab_w + 8, tab_y, tab_w, 24),
        list: Rect::new(
            rect.x + pad,
            rect.y + header_h,
            rect.w - pad * 2,
            rect.h - header_h - footer_h - pad,
        ),
    }
}

pub fn render(
    canvas: &mut Canvas,
    fonts: &mut FontSet,
    palette: &ChromePalette,
    settings: &SettingsOverlay,
    surface_w: i32,
    surface_h: i32,
) {
    canvas.fill_rect_alpha(0, 0, surface_w, surface_h, (0, 0, 0), 140);
    let layout = layout(surface_w, surface_h);
    let popup = layout.rect;
    canvas.fill_rect(popup.x, popup.y, popup.w, popup.h, palette.mantle);
    canvas.stroke_rect(popup.x, popup.y, popup.w, popup.h, 2, palette.accent);

    let cell_h = fonts.cell_height() as i32;
    canvas.text(
        fonts,
        popup.x + 16,
        popup.y + 12,
        "settings",
        Style::Bold,
        palette.text,
        popup.x + popup.w,
    );

    draw_tab(
        canvas,
        fonts,
        palette,
        layout.tab_theme,
        "theme",
        settings.section == SettingsSection::Theme,
        cell_h,
    );
    draw_tab(
        canvas,
        fonts,
        palette,
        layout.tab_font,
        "font size",
        settings.section == SettingsSection::FontSize,
        cell_h,
    );

    let list = layout.list;
    let row_h = cell_h + 8;
    let visible_rows = (list.h / row_h).max(1) as usize;
    let scroll = settings
        .selected
        .saturating_sub(visible_rows.saturating_sub(1));
    match settings.section {
        SettingsSection::Theme => {
            for (idx, name) in THEME_NAMES.iter().enumerate().skip(scroll) {
                if idx >= scroll + visible_rows {
                    break;
                }
                let row_y = list.y + ((idx - scroll) as i32) * row_h;
                let selected = idx == settings.selected;
                draw_choice_row(
                    canvas,
                    fonts,
                    palette,
                    list.x,
                    row_y,
                    list.w,
                    row_h,
                    name,
                    selected,
                    settings.draft_theme == *name,
                );
            }
        }
        SettingsSection::FontSize => {
            for (idx, size) in FONT_SIZE_CHOICES.iter().enumerate().skip(scroll) {
                if idx >= scroll + visible_rows {
                    break;
                }
                let row_y = list.y + ((idx - scroll) as i32) * row_h;
                let label = format!("{size:.0}px");
                let selected = idx == settings.selected;
                let active = (*size - settings.draft_font_size).abs() < 0.01;
                draw_choice_row(
                    canvas, fonts, palette, list.x, row_y, list.w, row_h, &label, selected, active,
                );
            }
        }
    }

    draw_button(
        canvas,
        fonts,
        palette,
        layout.apply,
        "apply",
        settings.has_unsaved_changes(),
    );
    draw_button(canvas, fonts, palette, layout.close, "close", true);
}

fn draw_tab(
    canvas: &mut Canvas,
    fonts: &mut FontSet,
    palette: &ChromePalette,
    rect: Rect,
    label: &str,
    active: bool,
    cell_h: i32,
) {
    let bg = if active {
        palette.surface1
    } else {
        palette.surface0
    };
    let fg = if active {
        palette.text
    } else {
        palette.subtext
    };
    canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, bg);
    if active {
        canvas.fill_rect(rect.x, rect.y, rect.w, 2, palette.accent);
    }
    canvas.text(
        fonts,
        rect.x + 10,
        rect.y + (rect.h - cell_h) / 2,
        label,
        Style::Bold,
        fg,
        rect.x + rect.w,
    );
}

fn draw_choice_row(
    canvas: &mut Canvas,
    fonts: &mut FontSet,
    palette: &ChromePalette,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    label: &str,
    selected: bool,
    active: bool,
) {
    if selected {
        canvas.fill_rect(x, y, w, h, palette.surface0);
        canvas.fill_rect(x, y, 3, h, palette.accent);
    }
    let fg = if active {
        palette.accent
    } else if selected {
        palette.text
    } else {
        palette.subtext
    };
    canvas.text(
        fonts,
        x + 12,
        y + (h - fonts.cell_height() as i32) / 2,
        &draw::truncate_cells(
            label,
            ((w - 24) / fonts.cell_width() as i32).max(1) as usize,
        ),
        if active { Style::Bold } else { Style::Regular },
        fg,
        x + w,
    );
}

fn draw_button(
    canvas: &mut Canvas,
    fonts: &mut FontSet,
    palette: &ChromePalette,
    rect: Rect,
    label: &str,
    enabled: bool,
) {
    let bg = if enabled {
        palette.surface1
    } else {
        palette.surface0
    };
    let fg = if enabled {
        palette.text
    } else {
        palette.overlay
    };
    canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, bg);
    canvas.text(
        fonts,
        rect.x + 10,
        rect.y + (rect.h - fonts.cell_height() as i32) / 2,
        label,
        Style::Regular,
        fg,
        rect.x + rect.w,
    );
}

pub fn hit_test(x: i32, y: i32, surface_w: i32, surface_h: i32) -> Option<SettingsHit> {
    let layout = layout(surface_w, surface_h);
    if layout.close.contains(x, y) {
        return Some(SettingsHit::Close);
    }
    if layout.apply.contains(x, y) {
        return Some(SettingsHit::Apply);
    }
    if layout.tab_theme.contains(x, y) {
        return Some(SettingsHit::Tab(SettingsSection::Theme));
    }
    if layout.tab_font.contains(x, y) {
        return Some(SettingsHit::Tab(SettingsSection::FontSize));
    }
    let list = layout.list;
    if list.contains(x, y) {
        let row_h = 24;
        let idx = ((y - list.y) / row_h).max(0) as usize;
        return Some(SettingsHit::ListItem(idx));
    }
    if !layout.rect.contains(x, y) {
        return Some(SettingsHit::Dismiss);
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsHit {
    Close,
    Apply,
    Tab(SettingsSection),
    ListItem(usize),
    Dismiss,
}
