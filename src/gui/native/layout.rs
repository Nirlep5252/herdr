//! Pixel layout for the native chrome and panes.
//!
//! Produces the rectangles for the sidebar, tab strip, toolbar, and each
//! terminal pane, plus the hit regions used for click handling. Pane rects are
//! derived proportionally from the server's `PaneLayoutSnapshot` (split tree),
//! so the relative tiling matches herdr regardless of the server's absolute
//! cell geometry.

use crate::gui::native::model::UiModel;

pub const SIDEBAR_W: i32 = 230;
pub const TABBAR_H: i32 = 38;
const PANE_GAP: i32 = 6;
pub const CAPTION_H: i32 = 22;
/// Height of two-line sidebar rows (workspaces and agents).
pub const ROW2_H: i32 = 44;
const TAB_W: i32 = 150;
const BTN_W: i32 = 36;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    pub fn inset(&self, d: i32) -> Rect {
        Rect::new(
            self.x + d,
            self.y + d,
            (self.w - 2 * d).max(0),
            (self.h - 2 * d).max(0),
        )
    }
}

#[derive(Debug, Clone)]
pub struct RowSlot {
    pub id: String,
    pub rect: Rect,
    pub focused: bool,
}

#[derive(Debug, Clone)]
pub struct TabSlot {
    pub tab_id: String,
    pub rect: Rect,
    pub close: Rect,
    pub focused: bool,
}

#[derive(Debug, Clone)]
pub struct PaneSlot {
    pub pane_id: String,
    pub terminal_id: String,
    pub rect: Rect,
    pub content: Rect,
    pub focused: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ViewLayout {
    pub sidebar: Rect,
    pub tabbar: Rect,
    pub workspace_rows: Vec<RowSlot>,
    pub agent_rows: Vec<RowSlot>,
    pub new_workspace: Rect,
    pub tabs: Vec<TabSlot>,
    pub new_tab: Rect,
    pub btn_split_right: Rect,
    pub btn_split_down: Rect,
    pub btn_settings: Rect,
    pub btn_close_pane: Rect,
    pub panes: Vec<PaneSlot>,
}

impl Default for Rect {
    fn default() -> Self {
        Rect::new(0, 0, 0, 0)
    }
}

pub fn compute(model: &UiModel, surface_w: i32, surface_h: i32) -> ViewLayout {
    let sidebar = Rect::new(0, 0, SIDEBAR_W, surface_h);
    let tabbar = Rect::new(SIDEBAR_W, 0, surface_w - SIDEBAR_W, TABBAR_H);
    let content = Rect::new(
        SIDEBAR_W,
        TABBAR_H,
        surface_w - SIDEBAR_W,
        surface_h - TABBAR_H,
    );

    let mut layout = ViewLayout {
        sidebar,
        tabbar,
        ..ViewLayout::default()
    };

    layout_sidebar(&mut layout, model, surface_h);
    layout_tabbar(&mut layout, model);
    layout_panes(&mut layout, model, content);
    layout
}

fn layout_sidebar(layout: &mut ViewLayout, model: &UiModel, surface_h: i32) {
    let pad = 10;
    let row_w = SIDEBAR_W - pad * 2;
    let mut y = 34; // below "spaces" header

    for ws in &model.workspaces {
        layout.workspace_rows.push(RowSlot {
            id: ws.workspace_id.clone(),
            rect: Rect::new(pad, y, row_w, ROW2_H),
            focused: Some(&ws.workspace_id) == model.active_workspace_id.as_ref(),
        });
        y += ROW2_H + 2;
    }

    // Agents section header sits ~30px down; rows follow.
    y += 36;
    for agent in &model.agents {
        let focused = Some(&agent.pane_id) == model.focused_pane_id.as_ref();
        layout.agent_rows.push(RowSlot {
            id: agent.terminal_id.clone(),
            rect: Rect::new(pad, y, row_w, ROW2_H),
            focused,
        });
        y += ROW2_H + 2;
    }

    layout.new_workspace = Rect::new(pad, surface_h - 38, row_w, 28);
}

fn layout_tabbar(layout: &mut ViewLayout, model: &UiModel) {
    let mut x = SIDEBAR_W + 8;
    let tab_h = TABBAR_H - 10;
    let tab_y = 5;
    let right = layout.tabbar.x + layout.tabbar.w - 8;
    let toolbar_left = right - BTN_W * 4 - 8;
    for tab in &model.tabs {
        if x + TAB_W > toolbar_left {
            break;
        }
        let rect = Rect::new(x, tab_y, TAB_W, tab_h);
        let close = Rect::new(x + TAB_W - 22, tab_y + (tab_h - 16) / 2, 16, 16);
        layout.tabs.push(TabSlot {
            tab_id: tab.tab_id.clone(),
            rect,
            close,
            focused: Some(&tab.tab_id) == model.active_tab_id.as_ref(),
        });
        x += TAB_W + 4;
    }
    layout.new_tab = Rect::new(x, tab_y, tab_h, tab_h);
    if layout.new_tab.x + layout.new_tab.w > toolbar_left {
        layout.new_tab = Rect::new(0, 0, 0, 0);
    }

    // Right-aligned pane toolbar.
    layout.btn_close_pane = Rect::new(right - BTN_W, tab_y, BTN_W - 4, tab_h);
    layout.btn_settings = Rect::new(right - BTN_W * 2, tab_y, BTN_W - 4, tab_h);
    layout.btn_split_down = Rect::new(right - BTN_W * 3, tab_y, BTN_W - 4, tab_h);
    layout.btn_split_right = Rect::new(right - BTN_W * 4, tab_y, BTN_W - 4, tab_h);
}

fn layout_panes(layout: &mut ViewLayout, model: &UiModel, content: Rect) {
    let panes = model.active_panes();
    if panes.is_empty() {
        return;
    }

    // Map terminal ids by pane id for content streaming.
    let terminal_for = |pane_id: &str| -> Option<String> {
        panes
            .iter()
            .find(|p| p.pane_id == pane_id)
            .map(|p| p.terminal_id.clone())
    };

    let inner = content.inset(PANE_GAP);

    if let Some(snapshot) = &model.layout {
        let area = snapshot.area;
        if area.width > 0 && area.height > 0 && !snapshot.panes.is_empty() {
            for p in &snapshot.panes {
                let fx = (p.rect.x.saturating_sub(area.x)) as f32 / area.width as f32;
                let fy = (p.rect.y.saturating_sub(area.y)) as f32 / area.height as f32;
                let fw = p.rect.width as f32 / area.width as f32;
                let fh = p.rect.height as f32 / area.height as f32;
                let rect = Rect::new(
                    inner.x + (fx * inner.w as f32).round() as i32,
                    inner.y + (fy * inner.h as f32).round() as i32,
                    (fw * inner.w as f32).round() as i32 - PANE_GAP,
                    (fh * inner.h as f32).round() as i32 - PANE_GAP,
                );
                push_pane(
                    layout,
                    &p.pane_id,
                    terminal_for(&p.pane_id).unwrap_or_default(),
                    rect,
                    p.focused || Some(&p.pane_id) == model.focused_pane_id.as_ref(),
                );
            }
            return;
        }
    }

    // Fallback: stack panes evenly when no usable layout snapshot exists.
    let n = panes.len() as i32;
    let cell_h = inner.h / n;
    for (i, pane) in panes.iter().enumerate() {
        let rect = Rect::new(
            inner.x,
            inner.y + i as i32 * cell_h,
            inner.w,
            cell_h - PANE_GAP,
        );
        push_pane(
            layout,
            &pane.pane_id,
            pane.terminal_id.clone(),
            rect,
            Some(&pane.pane_id) == model.focused_pane_id.as_ref(),
        );
    }
}

fn push_pane(
    layout: &mut ViewLayout,
    pane_id: &str,
    terminal_id: String,
    rect: Rect,
    focused: bool,
) {
    if rect.w <= 0 || rect.h <= CAPTION_H {
        return;
    }
    let content = Rect::new(
        rect.x + 1,
        rect.y + CAPTION_H,
        rect.w - 2,
        rect.h - CAPTION_H - 1,
    );
    layout.panes.push(PaneSlot {
        pane_id: pane_id.to_string(),
        terminal_id,
        rect,
        content,
        focused,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_is_half_open() {
        let r = Rect::new(10, 10, 20, 20);
        assert!(r.contains(10, 10));
        assert!(r.contains(29, 29));
        assert!(!r.contains(30, 30));
        assert!(!r.contains(9, 10));
    }

    #[test]
    fn empty_model_yields_chrome_but_no_panes() {
        let model = UiModel::default();
        let layout = compute(&model, 1000, 700);
        assert_eq!(layout.sidebar.w, SIDEBAR_W);
        assert_eq!(layout.tabbar.x, SIDEBAR_W);
        assert!(layout.panes.is_empty());
    }
}
