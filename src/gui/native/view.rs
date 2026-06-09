//! Draws the native chrome and the terminal panes into the window buffer.

use std::collections::HashMap;

use crate::api::schema::{AgentStatus, PaneInfo};

use super::super::color::Rgb;
use super::super::draw::{self, Canvas};
use super::super::font::{FontCache, FontSet, Style};
use super::super::render;
use super::super::theme::ChromePalette;
use super::layout::{Rect, ViewLayout, CAPTION_H};
use super::model::UiModel;
use super::panes::PaneStreams;
use super::settings::{self, SettingsOverlay};

/// A text selection to highlight within one pane.
#[derive(Debug, Clone)]
pub struct Highlight {
    pub terminal_id: String,
    pub start: (u16, u16),
    pub end: (u16, u16),
}

fn status_color(palette: &ChromePalette, status: AgentStatus) -> Rgb {
    match status {
        AgentStatus::Blocked => palette.red,
        AgentStatus::Working => palette.yellow,
        AgentStatus::Done => palette.blue,
        AgentStatus::Idle => palette.green,
        AgentStatus::Unknown => palette.overlay,
    }
}

fn status_text(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Blocked => "blocked",
        AgentStatus::Working => "working",
        AgentStatus::Done => "done",
        AgentStatus::Idle => "idle",
        AgentStatus::Unknown => "idle",
    }
}

/// Renders the entire native UI into `buffer`.
#[allow(clippy::too_many_arguments)]
pub fn render(
    buffer: &mut [u32],
    surface_w: usize,
    surface_h: usize,
    fonts: &mut FontSet,
    font_cache: &mut FontCache,
    base_font_px: f32,
    palette: &ChromePalette,
    model: &UiModel,
    layout: &ViewLayout,
    panes: &PaneStreams,
    pane_zoom: &HashMap<String, f32>,
    selection: Option<&Highlight>,
    settings: &SettingsOverlay,
) {
    let mut canvas = Canvas::new(buffer, surface_w, surface_h);
    canvas.clear(palette.base);

    draw_sidebar(&mut canvas, fonts, palette, model, layout);
    draw_tabbar(&mut canvas, fonts, palette, model, layout);
    draw_panes(
        &mut canvas,
        fonts,
        font_cache,
        base_font_px,
        palette,
        model,
        layout,
        panes,
        pane_zoom,
        selection,
    );

    if settings.open {
        settings::render(
            &mut canvas,
            fonts,
            palette,
            settings,
            surface_w as i32,
            surface_h as i32,
        );
    }
}

fn draw_sidebar(
    canvas: &mut Canvas,
    fonts: &mut FontSet,
    palette: &ChromePalette,
    model: &UiModel,
    layout: &ViewLayout,
) {
    let sb = layout.sidebar;
    let cell_h = fonts.cell_height() as i32;
    let cell_w = fonts.cell_width() as i32;
    canvas.fill_rect(sb.x, sb.y, sb.w, sb.h, palette.mantle);
    canvas.fill_rect(sb.x + sb.w - 1, sb.y, 1, sb.h, palette.crust);

    let pad = 10;
    let label_max = sb.x + sb.w - pad;
    canvas.text(
        fonts,
        pad,
        10,
        "SPACES",
        Style::Bold,
        palette.overlay,
        label_max,
    );

    for row in &layout.workspace_rows {
        let Some(ws) = model.workspaces.iter().find(|w| w.workspace_id == row.id) else {
            continue;
        };
        if row.focused {
            canvas.fill_rect(
                row.rect.x,
                row.rect.y,
                row.rect.w,
                row.rect.h,
                palette.surface0,
            );
            canvas.fill_rect(row.rect.x, row.rect.y, 3, row.rect.h, palette.accent);
        }
        let line1_y = row.rect.y + 6;
        let line2_y = line1_y + cell_h + 2;
        canvas.dot(
            row.rect.x + 14,
            line1_y + cell_h / 2,
            4,
            status_color(palette, ws.agent_status),
        );
        let max_cells = ((row.rect.w - 30) / cell_w).max(1) as usize;
        let label = draw::truncate_cells(&ws.label, max_cells);
        let fg = if row.focused {
            palette.text
        } else {
            palette.subtext
        };
        canvas.text(
            fonts,
            row.rect.x + 26,
            line1_y,
            &label,
            Style::Bold,
            fg,
            label_max,
        );
        if let Some(branch) = ws.branch.as_deref().filter(|b| !b.is_empty()) {
            let sub = draw::truncate_cells(branch, max_cells);
            canvas.text(
                fonts,
                row.rect.x + 26,
                line2_y,
                &sub,
                Style::Regular,
                palette.overlay,
                label_max,
            );
        } else if let Some(worktree) = ws.worktree.as_ref() {
            let sub = draw::truncate_cells(&worktree.repo_name, max_cells);
            canvas.text(
                fonts,
                row.rect.x + 26,
                line2_y,
                &sub,
                Style::Regular,
                palette.overlay,
                label_max,
            );
        }
    }

    if !layout.agent_rows.is_empty() {
        let header_y = layout.agent_rows[0].rect.y - 24;
        canvas.text(
            fonts,
            pad,
            header_y,
            "AGENTS",
            Style::Bold,
            palette.overlay,
            label_max,
        );
    }
    for row in &layout.agent_rows {
        let Some(agent) = model.agents.iter().find(|a| a.terminal_id == row.id) else {
            continue;
        };
        if row.focused {
            canvas.fill_rect(
                row.rect.x,
                row.rect.y,
                row.rect.w,
                row.rect.h,
                palette.surface0,
            );
            canvas.fill_rect(row.rect.x, row.rect.y, 3, row.rect.h, palette.accent);
        }
        let line1_y = row.rect.y + 6;
        let line2_y = line1_y + cell_h + 2;
        canvas.dot(
            row.rect.x + 14,
            line1_y + cell_h / 2,
            4,
            status_color(palette, agent.agent_status),
        );

        let ws_label = model
            .workspaces
            .iter()
            .find(|w| w.workspace_id == agent.workspace_id);
        let title = match ws_label {
            Some(ws) => format!("{} · {}", ws.label, ws.number),
            None => agent.workspace_id.clone(),
        };
        let max_cells = ((row.rect.w - 30) / cell_w).max(1) as usize;
        let fg = if row.focused {
            palette.text
        } else {
            palette.subtext
        };
        canvas.text(
            fonts,
            row.rect.x + 26,
            line1_y,
            &draw::truncate_cells(&title, max_cells),
            Style::Bold,
            fg,
            label_max,
        );

        let agent_label = agent
            .name
            .as_deref()
            .or(agent.display_agent.as_deref())
            .or(agent.agent.as_deref())
            .unwrap_or("agent");
        let detail = format!("{} · {}", status_text(agent.agent_status), agent_label);
        canvas.text(
            fonts,
            row.rect.x + 26,
            line2_y,
            &draw::truncate_cells(&detail, max_cells),
            Style::Regular,
            status_color(palette, agent.agent_status),
            label_max,
        );
    }

    let nw = layout.new_workspace;
    canvas.fill_rect(nw.x, nw.y, nw.w, nw.h, palette.surface0);
    let nw_y = nw.y + (nw.h - cell_h) / 2;
    canvas.text(
        fonts,
        nw.x + 10,
        nw_y,
        "+ new space",
        Style::Regular,
        palette.text,
        nw.x + nw.w,
    );
}

fn draw_tabbar(
    canvas: &mut Canvas,
    fonts: &mut FontSet,
    palette: &ChromePalette,
    model: &UiModel,
    layout: &ViewLayout,
) {
    let tb = layout.tabbar;
    let cell_w = fonts.cell_width() as i32;
    let cell_h = fonts.cell_height() as i32;
    let text_y = |rect: Rect| rect.y + (rect.h - cell_h) / 2;
    canvas.fill_rect(tb.x, tb.y, tb.w, tb.h, palette.crust);
    canvas.fill_rect(tb.x, tb.y + tb.h - 1, tb.w, 1, palette.mantle);

    for tab in &layout.tabs {
        let bg = if tab.focused {
            palette.base
        } else {
            palette.surface0
        };
        canvas.fill_rect(tab.rect.x, tab.rect.y, tab.rect.w, tab.rect.h, bg);
        if tab.focused {
            canvas.fill_rect(tab.rect.x, tab.rect.y, tab.rect.w, 2, palette.accent);
        }
        if let Some(info) = model.tabs.iter().find(|t| t.tab_id == tab.tab_id) {
            let max_cells = ((tab.rect.w - 36) / cell_w).max(1) as usize;
            let label = draw::truncate_cells(&info.label, max_cells);
            let fg = if tab.focused {
                palette.text
            } else {
                palette.subtext
            };
            canvas.text(
                fonts,
                tab.rect.x + 10,
                text_y(tab.rect),
                &label,
                Style::Regular,
                fg,
                tab.close.x,
            );
        }
        canvas.draw_glyph(
            fonts,
            '×',
            Style::Regular,
            tab.close.x,
            tab.close.y,
            fonts.ascent(),
            palette.subtext,
        );
    }

    let nt = layout.new_tab;
    canvas.fill_rect(nt.x, nt.y, nt.w, nt.h, palette.surface0);
    canvas.draw_glyph(
        fonts,
        '+',
        Style::Bold,
        nt.x + (nt.w - cell_w) / 2,
        text_y(nt),
        fonts.ascent(),
        palette.text,
    );

    draw_split_button(canvas, palette, layout.btn_split_right, true);
    draw_split_button(canvas, palette, layout.btn_split_down, false);
    draw_settings_button(canvas, fonts, palette, layout.btn_settings);
    let cp = layout.btn_close_pane;
    canvas.fill_rect(cp.x, cp.y, cp.w, cp.h, palette.surface0);
    canvas.draw_glyph(
        fonts,
        '×',
        Style::Bold,
        cp.x + (cp.w - cell_w) / 2,
        text_y(cp),
        fonts.ascent(),
        palette.text,
    );
}

fn draw_split_button(canvas: &mut Canvas, palette: &ChromePalette, rect: Rect, vertical: bool) {
    canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, palette.surface0);
    let icon = Rect::new(rect.x + rect.w / 2 - 7, rect.y + rect.h / 2 - 6, 14, 12);
    canvas.stroke_rect(icon.x, icon.y, icon.w, icon.h, 1, palette.text);
    if vertical {
        canvas.fill_rect(icon.x + icon.w / 2, icon.y, 1, icon.h, palette.text);
    } else {
        canvas.fill_rect(icon.x, icon.y + icon.h / 2, icon.w, 1, palette.text);
    }
}

fn draw_settings_button(
    canvas: &mut Canvas,
    fonts: &mut FontSet,
    palette: &ChromePalette,
    rect: Rect,
) {
    canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, palette.surface0);
    let cx = rect.x + rect.w / 2;
    let cy = rect.y + rect.h / 2;
    canvas.stroke_rect(cx - 6, cy - 6, 12, 12, 1, palette.text);
    for angle in [0.0_f64, 1.25, 2.5, 3.75, 5.0] {
        let (sin, cos) = angle.sin_cos();
        let x = cx + (sin * 7.0) as i32;
        let y = cy - (cos * 7.0) as i32;
        canvas.fill_rect(x - 1, y - 1, 3, 3, palette.text);
    }
    canvas.fill_rect(cx - 2, cy - 2, 4, 4, palette.text);
    let _ = (fonts, cx, cy);
}

fn caption_label(pane: &PaneInfo) -> String {
    pane.label
        .clone()
        .or_else(|| pane.display_agent.clone())
        .or_else(|| pane.agent.clone())
        .or_else(|| pane.title.clone())
        .unwrap_or_else(|| "shell".to_string())
}

fn draw_panes(
    canvas: &mut Canvas,
    chrome_fonts: &mut FontSet,
    font_cache: &mut FontCache,
    base_font_px: f32,
    palette: &ChromePalette,
    model: &UiModel,
    layout: &ViewLayout,
    panes: &PaneStreams,
    pane_zoom: &HashMap<String, f32>,
    selection: Option<&Highlight>,
) {
    let chrome_cell_w = chrome_fonts.cell_width() as i32;
    let chrome_cell_h = chrome_fonts.cell_height() as i32;
    for slot in &layout.panes {
        canvas.fill_rect(
            slot.rect.x,
            slot.rect.y,
            slot.rect.w,
            CAPTION_H,
            if slot.focused {
                palette.surface1
            } else {
                palette.surface0
            },
        );
        canvas.fill_rect(
            slot.content.x,
            slot.content.y,
            slot.content.w,
            slot.content.h,
            palette.base,
        );

        if let Some(pane) = model.panes.iter().find(|p| p.pane_id == slot.pane_id) {
            canvas.dot(
                slot.rect.x + 12,
                slot.rect.y + CAPTION_H / 2,
                4,
                status_color(palette, pane.agent_status),
            );
            let max_cells = ((slot.rect.w - 30) / chrome_cell_w).max(1) as usize;
            let label = draw::truncate_cells(&caption_label(pane), max_cells);
            let y = slot.rect.y + (CAPTION_H - chrome_cell_h) / 2;
            let fg = if slot.focused {
                palette.text
            } else {
                palette.subtext
            };
            canvas.text(
                chrome_fonts,
                slot.rect.x + 24,
                y,
                &label,
                Style::Bold,
                fg,
                slot.rect.x + slot.rect.w,
            );
        }

        let zoom = pane_zoom
            .get(&slot.terminal_id)
            .copied()
            .unwrap_or(1.0)
            .clamp(0.5, 2.5);
        let pane_px = base_font_px * zoom;
        if let Ok(pane_fonts) = font_cache.get(pane_px) {
            let cell_w = pane_fonts.cell_width() as i32;
            let cell_h = pane_fonts.cell_height() as i32;
            if let Some(frame) = panes.frame(&slot.terminal_id) {
                render::render_frame_at(frame, pane_fonts, canvas, slot.content.x, slot.content.y);
                if let Some(hl) = selection.filter(|h| h.terminal_id == slot.terminal_id) {
                    draw_selection(
                        canvas,
                        palette,
                        slot.content,
                        frame.width,
                        frame.height,
                        cell_w,
                        cell_h,
                        hl,
                    );
                }
            }
            if zoom != 1.0 {
                let zoom_label = format!("{:.0}%", zoom * 100.0);
                canvas.text(
                    chrome_fonts,
                    slot.content.x + slot.content.w - 48,
                    slot.content.y + 4,
                    &zoom_label,
                    Style::Regular,
                    palette.overlay,
                    slot.content.x + slot.content.w,
                );
            }
        }

        let border = if slot.focused {
            palette.accent
        } else {
            palette.surface0
        };
        let thickness = if slot.focused { 2 } else { 1 };
        canvas.stroke_rect(
            slot.rect.x,
            slot.rect.y,
            slot.rect.w,
            slot.rect.h,
            thickness,
            border,
        );
    }
}

fn draw_selection(
    canvas: &mut Canvas,
    palette: &ChromePalette,
    content: Rect,
    width: u16,
    height: u16,
    cell_w: i32,
    cell_h: i32,
    hl: &Highlight,
) {
    let (start, end) = ordered(hl.start, hl.end);
    for row in start.1..=end.1 {
        if row >= height {
            break;
        }
        let col_lo = if row == start.1 { start.0 } else { 0 };
        let col_hi = if row == end.1 {
            end.0
        } else {
            width.saturating_sub(1)
        };
        if col_hi < col_lo {
            continue;
        }
        let x = content.x + col_lo as i32 * cell_w;
        let w = (col_hi - col_lo + 1) as i32 * cell_w;
        let y = content.y + row as i32 * cell_h;
        canvas.fill_rect_alpha(x, y, w, cell_h, palette.accent, 130);
    }
}

/// Orders two cell coordinates into reading order (start <= end).
pub fn ordered(a: (u16, u16), b: (u16, u16)) -> ((u16, u16), (u16, u16)) {
    if (a.1, a.0) <= (b.1, b.0) {
        (a, b)
    } else {
        (b, a)
    }
}
