//! Draws the native chrome and the terminal panes into the window buffer.
//!
//! Chrome (sidebar, tab strip, toolbar, pane captions) is custom-drawn with the
//! shared [`Canvas`] primitives so it looks like a desktop app rather than a
//! TUI. Each pane's live terminal content is composited into its content rect
//! via `render::render_frame_at`, with an optional selection highlight overlaid
//! on top. Colors follow a Catppuccin-Mocha palette so the app feels cohesive
//! with herdr's default theme.

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

/// A text selection to highlight within one pane.
#[derive(Debug, Clone)]
pub struct Highlight {
    pub terminal_id: String,
    pub start: (u16, u16),
    pub end: (u16, u16),
}

fn status_color(status: AgentStatus) -> Rgb {
    match status {
        AgentStatus::Blocked => RED,
        AgentStatus::Working => YELLOW,
        AgentStatus::Done => BLUE,
        AgentStatus::Idle => GREEN,
        AgentStatus::Unknown => OVERLAY,
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
    model: &UiModel,
    layout: &ViewLayout,
    panes: &PaneStreams,
    selection: Option<&Highlight>,
) {
    let mut canvas = Canvas::new(buffer, surface_w, surface_h);
    canvas.clear(BASE);

    draw_sidebar(&mut canvas, fonts, model, layout);
    draw_tabbar(&mut canvas, fonts, model, layout);
    draw_panes(&mut canvas, fonts, model, layout, panes, selection);
}

fn draw_sidebar(canvas: &mut Canvas, fonts: &mut FontSet, model: &UiModel, layout: &ViewLayout) {
    let sb = layout.sidebar;
    let cell_h = fonts.cell_height() as i32;
    let cell_w = fonts.cell_width() as i32;
    canvas.fill_rect(sb.x, sb.y, sb.w, sb.h, MANTLE);
    canvas.fill_rect(sb.x + sb.w - 1, sb.y, 1, sb.h, CRUST);

    let pad = 10;
    let label_max = sb.x + sb.w - pad;
    canvas.text(fonts, pad, 10, "SPACES", Style::Bold, OVERLAY, label_max);

    for row in &layout.workspace_rows {
        let Some(ws) = model.workspaces.iter().find(|w| w.workspace_id == row.id) else {
            continue;
        };
        if row.focused {
            canvas.fill_rect(row.rect.x, row.rect.y, row.rect.w, row.rect.h, SURFACE0);
            canvas.fill_rect(row.rect.x, row.rect.y, 3, row.rect.h, ACCENT);
        }
        let line1_y = row.rect.y + 6;
        let line2_y = line1_y + cell_h + 2;
        canvas.dot(
            row.rect.x + 14,
            line1_y + cell_h / 2,
            4,
            status_color(ws.agent_status),
        );
        let max_cells = ((row.rect.w - 30) / cell_w).max(1) as usize;
        let label = draw::truncate_cells(&ws.label, max_cells);
        let fg = if row.focused { TEXT } else { SUBTEXT };
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
                OVERLAY,
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
                OVERLAY,
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
            OVERLAY,
            label_max,
        );
    }
    for row in &layout.agent_rows {
        let Some(agent) = model.agents.iter().find(|a| a.terminal_id == row.id) else {
            continue;
        };
        if row.focused {
            canvas.fill_rect(row.rect.x, row.rect.y, row.rect.w, row.rect.h, SURFACE0);
            canvas.fill_rect(row.rect.x, row.rect.y, 3, row.rect.h, ACCENT);
        }
        let line1_y = row.rect.y + 6;
        let line2_y = line1_y + cell_h + 2;
        canvas.dot(
            row.rect.x + 14,
            line1_y + cell_h / 2,
            4,
            status_color(agent.agent_status),
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
        let fg = if row.focused { TEXT } else { SUBTEXT };
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
            status_color(agent.agent_status),
            label_max,
        );
    }

    let nw = layout.new_workspace;
    canvas.fill_rect(nw.x, nw.y, nw.w, nw.h, SURFACE0);
    let nw_y = nw.y + (nw.h - cell_h) / 2;
    canvas.text(
        fonts,
        nw.x + 10,
        nw_y,
        "+ new space",
        Style::Regular,
        TEXT,
        nw.x + nw.w,
    );
}

fn draw_tabbar(canvas: &mut Canvas, fonts: &mut FontSet, model: &UiModel, layout: &ViewLayout) {
    let tb = layout.tabbar;
    let cell_w = fonts.cell_width() as i32;
    let cell_h = fonts.cell_height() as i32;
    let text_y = |rect: Rect| rect.y + (rect.h - cell_h) / 2;
    canvas.fill_rect(tb.x, tb.y, tb.w, tb.h, CRUST);
    canvas.fill_rect(tb.x, tb.y + tb.h - 1, tb.w, 1, MANTLE);

    for tab in &layout.tabs {
        let bg = if tab.focused { BASE } else { SURFACE0 };
        canvas.fill_rect(tab.rect.x, tab.rect.y, tab.rect.w, tab.rect.h, bg);
        if tab.focused {
            canvas.fill_rect(tab.rect.x, tab.rect.y, tab.rect.w, 2, ACCENT);
        }
        if let Some(info) = model.tabs.iter().find(|t| t.tab_id == tab.tab_id) {
            let max_cells = ((tab.rect.w - 36) / cell_w).max(1) as usize;
            let label = draw::truncate_cells(&info.label, max_cells);
            let fg = if tab.focused { TEXT } else { SUBTEXT };
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
            SUBTEXT,
        );
    }

    let nt = layout.new_tab;
    canvas.fill_rect(nt.x, nt.y, nt.w, nt.h, SURFACE0);
    canvas.draw_glyph(
        fonts,
        '+',
        Style::Bold,
        nt.x + (nt.w - cell_w) / 2,
        text_y(nt),
        fonts.ascent(),
        TEXT,
    );

    draw_split_button(canvas, layout.btn_split_right, true);
    draw_split_button(canvas, layout.btn_split_down, false);
    let cp = layout.btn_close_pane;
    canvas.fill_rect(cp.x, cp.y, cp.w, cp.h, SURFACE0);
    canvas.draw_glyph(
        fonts,
        '×',
        Style::Bold,
        cp.x + (cp.w - cell_w) / 2,
        text_y(cp),
        fonts.ascent(),
        TEXT,
    );
}

fn draw_split_button(canvas: &mut Canvas, rect: Rect, vertical: bool) {
    canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, SURFACE0);
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
    selection: Option<&Highlight>,
) {
    let cell_w = fonts.cell_width() as i32;
    let cell_h = fonts.cell_height() as i32;
    for slot in &layout.panes {
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
            let max_cells = ((slot.rect.w - 30) / cell_w).max(1) as usize;
            let label = draw::truncate_cells(&caption_label(pane), max_cells);
            let y = slot.rect.y + (CAPTION_H - cell_h) / 2;
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

        if let Some(frame) = panes.frame(&slot.terminal_id) {
            render::render_frame_at(frame, fonts, canvas, slot.content.x, slot.content.y);
            if let Some(hl) = selection.filter(|h| h.terminal_id == slot.terminal_id) {
                draw_selection(
                    canvas,
                    slot.content,
                    frame.width,
                    frame.height,
                    cell_w,
                    cell_h,
                    hl,
                );
            }
        }

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

fn draw_selection(
    canvas: &mut Canvas,
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
        canvas.fill_rect_alpha(x, y, w, cell_h, ACCENT, 130);
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
