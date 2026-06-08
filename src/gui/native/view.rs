//! Draws the native chrome and the terminal panes into the window buffer.
//!
//! Chrome (sidebar, tab strip, toolbar, pane captions) is custom-drawn with the
//! shared [`Canvas`] primitives so it looks like a desktop app rather than a
//! TUI. Each pane's live terminal content is composited into its content rect
//! via `render::render_frame_at`. Colors follow a Catppuccin-Mocha palette so
//! the app feels cohesive with herdr's default theme.

use crate::api::schema::{AgentStatus, PaneInfo};

use super::super::color::Rgb;
use super::super::draw::{self, Canvas};
use super::super::font::{FontSet, Style};
use super::super::render;
use super::layout::{Rect, ViewLayout, CAPTION_H};
use super::model::UiModel;
use super::panes::PaneStreams;

const BASE: Rgb = (30, 30, 46);
const MANTLE: Rgb = (24, 24, 37);
const CRUST: Rgb = (17, 17, 27);
const SURFACE0: Rgb = (49, 50, 68);
const SURFACE1: Rgb = (69, 71, 90);
const TEXT: Rgb = (205, 214, 244);
const SUBTEXT: Rgb = (166, 173, 200);
const OVERLAY: Rgb = (108, 112, 134);
const ACCENT: Rgb = (137, 180, 250);

const RED: Rgb = (243, 139, 168);
const YELLOW: Rgb = (249, 226, 175);
const BLUE: Rgb = (137, 180, 250);
const GREEN: Rgb = (166, 227, 161);

fn status_color(status: AgentStatus) -> Rgb {
    match status {
        AgentStatus::Blocked => RED,
        AgentStatus::Working => YELLOW,
        AgentStatus::Done => BLUE,
        AgentStatus::Idle => GREEN,
        AgentStatus::Unknown => OVERLAY,
    }
}

fn row_text_y(rect: Rect, fonts: &FontSet) -> i32 {
    rect.y + (rect.h - fonts.cell_height() as i32) / 2
}

/// Renders the entire native UI into `buffer`.
pub fn render(
    buffer: &mut [u32],
    surface_w: usize,
    surface_h: usize,
    fonts: &mut FontSet,
    model: &UiModel,
    layout: &ViewLayout,
    panes: &PaneStreams,
) {
    let mut canvas = Canvas::new(buffer, surface_w, surface_h);
    canvas.clear(BASE);

    draw_sidebar(&mut canvas, fonts, model, layout);
    draw_tabbar(&mut canvas, fonts, model, layout);
    draw_panes(&mut canvas, fonts, model, layout, panes);
}

fn draw_sidebar(canvas: &mut Canvas, fonts: &mut FontSet, model: &UiModel, layout: &ViewLayout) {
    let sb = layout.sidebar;
    canvas.fill_rect(sb.x, sb.y, sb.w, sb.h, MANTLE);
    canvas.fill_rect(sb.x + sb.w - 1, sb.y, 1, sb.h, CRUST);

    let pad = 10;
    let label_max = sb.x + sb.w - pad;
    canvas.text(fonts, pad, 10, "SPACES", Style::Bold, OVERLAY, label_max);

    for row in &layout.workspace_rows {
        if row.focused {
            canvas.fill_rect(row.rect.x, row.rect.y, row.rect.w, row.rect.h, SURFACE0);
            canvas.fill_rect(row.rect.x, row.rect.y, 3, row.rect.h, ACCENT);
        }
        if let Some(ws) = model.workspaces.iter().find(|w| w.workspace_id == row.id) {
            canvas.dot(
                row.rect.x + 14,
                row.rect.y + row.rect.h / 2,
                4,
                status_color(ws.agent_status),
            );
            let label = draw::truncate_cells(
                &ws.label,
                ((row.rect.w - 30) / fonts.cell_width() as i32).max(1) as usize,
            );
            let fg = if row.focused { TEXT } else { SUBTEXT };
            canvas.text(
                fonts,
                row.rect.x + 26,
                row_text_y(row.rect, fonts),
                &label,
                Style::Regular,
                fg,
                label_max,
            );
        }
    }

    if !layout.agent_rows.is_empty() {
        let header_y = layout.agent_rows[0].rect.y - 26;
        canvas.text(
            fonts,
            pad,
            header_y,
            "AGENTS",
            Style::Bold,
            OVERLAY,
            label_max,
        );
    }
    for row in &layout.agent_rows {
        if row.focused {
            canvas.fill_rect(row.rect.x, row.rect.y, row.rect.w, row.rect.h, SURFACE0);
            canvas.fill_rect(row.rect.x, row.rect.y, 3, row.rect.h, ACCENT);
        }
        if let Some(agent) = model.agents.iter().find(|a| a.terminal_id == row.id) {
            canvas.dot(
                row.rect.x + 14,
                row.rect.y + row.rect.h / 2,
                4,
                status_color(agent.agent_status),
            );
            let name = agent
                .name
                .as_deref()
                .or(agent.display_agent.as_deref())
                .or(agent.agent.as_deref())
                .unwrap_or("agent");
            let label = draw::truncate_cells(
                name,
                ((row.rect.w - 30) / fonts.cell_width() as i32).max(1) as usize,
            );
            let fg = if row.focused { TEXT } else { SUBTEXT };
            canvas.text(
                fonts,
                row.rect.x + 26,
                row_text_y(row.rect, fonts),
                &label,
                Style::Regular,
                fg,
                label_max,
            );
        }
    }

    // New-workspace button.
    let nw = layout.new_workspace;
    canvas.fill_rect(nw.x, nw.y, nw.w, nw.h, SURFACE0);
    canvas.text(
        fonts,
        nw.x + 10,
        row_text_y(nw, fonts),
        "+ new space",
        Style::Regular,
        TEXT,
        nw.x + nw.w,
    );
}

fn draw_tabbar(canvas: &mut Canvas, fonts: &mut FontSet, model: &UiModel, layout: &ViewLayout) {
    let tb = layout.tabbar;
    canvas.fill_rect(tb.x, tb.y, tb.w, tb.h, CRUST);
    canvas.fill_rect(tb.x, tb.y + tb.h - 1, tb.w, 1, MANTLE);

    for tab in &layout.tabs {
        let bg = if tab.focused { BASE } else { SURFACE0 };
        canvas.fill_rect(tab.rect.x, tab.rect.y, tab.rect.w, tab.rect.h, bg);
        if tab.focused {
            canvas.fill_rect(tab.rect.x, tab.rect.y, tab.rect.w, 2, ACCENT);
        }
        if let Some(info) = model.tabs.iter().find(|t| t.tab_id == tab.tab_id) {
            let max_cells = ((tab.rect.w - 36) / fonts.cell_width() as i32).max(1) as usize;
            let label = draw::truncate_cells(&info.label, max_cells);
            let fg = if tab.focused { TEXT } else { SUBTEXT };
            canvas.text(
                fonts,
                tab.rect.x + 10,
                row_text_y(tab.rect, fonts),
                &label,
                Style::Regular,
                fg,
                tab.close.x,
            );
        }
        // Close glyph.
        canvas.draw_glyph(
            fonts,
            '×',
            Style::Regular,
            tab.close.x,
            tab.close.y,
            fonts.ascent(),
            SUBTEXT,
        );
    }

    // New-tab button.
    let nt = layout.new_tab;
    canvas.fill_rect(nt.x, nt.y, nt.w, nt.h, SURFACE0);
    canvas.draw_glyph(
        fonts,
        '+',
        Style::Bold,
        nt.x + (nt.w - fonts.cell_width() as i32) / 2,
        row_text_y(nt, fonts),
        fonts.ascent(),
        TEXT,
    );

    // Toolbar buttons (act on focused pane).
    draw_split_button(canvas, layout.btn_split_right, true);
    draw_split_button(canvas, layout.btn_split_down, false);
    let cp = layout.btn_close_pane;
    canvas.fill_rect(cp.x, cp.y, cp.w, cp.h, SURFACE0);
    canvas.draw_glyph(
        fonts,
        '×',
        Style::Bold,
        cp.x + (cp.w - fonts.cell_width() as i32) / 2,
        row_text_y(cp, fonts),
        fonts.ascent(),
        TEXT,
    );
}

fn draw_split_button(canvas: &mut Canvas, rect: Rect, vertical: bool) {
    canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, SURFACE0);
    // Draw a small icon: a bordered box divided into two halves.
    let icon = Rect::new(rect.x + rect.w / 2 - 7, rect.y + rect.h / 2 - 6, 14, 12);
    canvas.stroke_rect(icon.x, icon.y, icon.w, icon.h, 1, TEXT);
    if vertical {
        canvas.fill_rect(icon.x + icon.w / 2, icon.y, 1, icon.h, TEXT);
    } else {
        canvas.fill_rect(icon.x, icon.y + icon.h / 2, icon.w, 1, TEXT);
    }
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
    fonts: &mut FontSet,
    model: &UiModel,
    layout: &ViewLayout,
    panes: &PaneStreams,
) {
    for slot in &layout.panes {
        // Pane background + caption.
        canvas.fill_rect(
            slot.rect.x,
            slot.rect.y,
            slot.rect.w,
            CAPTION_H,
            if slot.focused { SURFACE1 } else { SURFACE0 },
        );
        canvas.fill_rect(
            slot.content.x,
            slot.content.y,
            slot.content.w,
            slot.content.h,
            BASE,
        );

        if let Some(pane) = model.panes.iter().find(|p| p.pane_id == slot.pane_id) {
            canvas.dot(
                slot.rect.x + 12,
                slot.rect.y + CAPTION_H / 2,
                4,
                status_color(pane.agent_status),
            );
            let label = caption_label(pane);
            let max_cells = ((slot.rect.w - 30) / fonts.cell_width() as i32).max(1) as usize;
            let label = draw::truncate_cells(&label, max_cells);
            let y = slot.rect.y + (CAPTION_H - fonts.cell_height() as i32) / 2;
            let fg = if slot.focused { TEXT } else { SUBTEXT };
            canvas.text(
                fonts,
                slot.rect.x + 24,
                y,
                &label,
                Style::Bold,
                fg,
                slot.rect.x + slot.rect.w,
            );
        }

        // Live terminal content.
        if let Some(frame) = panes.frame(&slot.terminal_id) {
            render::render_frame_at(frame, fonts, canvas, slot.content.x, slot.content.y);
        }

        // Focus border.
        let border = if slot.focused { ACCENT } else { SURFACE0 };
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
