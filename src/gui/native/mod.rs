//! Native-chrome GUI: a desktop app shell around real terminal panes.
//!
//! Chrome (sidebar, tabs, toolbar) is driven by the JSON API via a background
//! [`worker`] thread and drawn as native widgets. Each visible pane streams its
//! own terminal content over a client-protocol attach connection ([`panes`]).
//! This module owns the winit window/event loop and routes input: chrome
//! clicks become API commands, and keyboard/mouse over a pane are forwarded to
//! that pane's terminal.

mod layout;
mod model;
mod panes;
mod settings;
mod view;
mod worker;

use std::io;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use tracing::{info, warn};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::api::schema::SplitDirection;
use crate::config::Config;
use crate::protocol::{ClientInputEvent, ClientMouseButton, ClientMouseKind, FrameData};

use super::font::{FontCache, FontSet};
use super::input;
use super::theme::ChromePalette;
use layout::ViewLayout;
use model::UiModel;
use panes::PaneStreams;
use settings::{load_gui_config, SettingsHit, SettingsOverlay, SettingsSection};
use worker::Cmd;

const DEFAULT_WIDTH: f64 = 1200.0;
const DEFAULT_HEIGHT: f64 = 780.0;
const PANE_ZOOM_MIN: f32 = 0.5;
const PANE_ZOOM_MAX: f32 = 2.5;
const PANE_ZOOM_STEP: f32 = 0.1;
const SELECTION_DRAG_THRESHOLD_PX: i32 = 4;

/// Events delivered into the winit loop from background threads.
#[derive(Debug)]
pub enum UserEvent {
    Model(Box<UiModel>),
    PaneFrame {
        terminal_id: String,
        epoch: u64,
        frame: FrameData,
    },
    PaneClosed {
        terminal_id: String,
        epoch: u64,
    },
    ApiError(String),
}

/// An in-progress or completed text selection within a pane.
#[derive(Debug, Clone)]
struct Selection {
    terminal_id: String,
    anchor: (u16, u16),
    head: (u16, u16),
}

/// Runs the native GUI. Assumes a server is reachable.
pub fn run() -> io::Result<()> {
    let (config, gui, chrome) = load_gui_config();
    let base_font_px = gui.validated_font_size();
    let fonts = FontSet::load(base_font_px).map_err(io::Error::other)?;
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|err| io::Error::other(format!("failed to build event loop: {err}")))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let cmd_tx = worker::spawn(proxy.clone());

    let mut app = NativeApp::new(fonts, base_font_px, chrome, config, proxy, cmd_tx);
    event_loop
        .run_app(&mut app)
        .map_err(|err| io::Error::other(format!("event loop error: {err}")))?;
    app.into_result()
}

struct NativeApp {
    fonts: FontSet,
    font_cache: FontCache,
    base_font_px: f32,
    chrome: ChromePalette,
    settings: SettingsOverlay,
    pane_zoom: std::collections::HashMap<String, f32>,
    proxy: EventLoopProxy<UserEvent>,
    cmd_tx: Sender<Cmd>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    window: Option<Rc<Window>>,
    panes: Option<PaneStreams>,
    model: UiModel,
    layout: ViewLayout,
    surface_size: (u32, u32),
    mods: ModifiersState,
    cursor_px: (f64, f64),
    selection: Option<Selection>,
    selecting: bool,
    left_button_down: bool,
    selection_dragging: bool,
    press_origin: Option<(i32, i32)>,
    mouse_buttons: Vec<ClientMouseButton>,
    clipboard: Option<arboard::Clipboard>,
    last_copied: Option<String>,
    error: Option<io::Error>,
}

impl NativeApp {
    fn new(
        fonts: FontSet,
        base_font_px: f32,
        chrome: ChromePalette,
        config: Config,
        proxy: EventLoopProxy<UserEvent>,
        cmd_tx: Sender<Cmd>,
    ) -> Self {
        Self {
            fonts,
            font_cache: FontCache::default(),
            base_font_px,
            chrome,
            settings: SettingsOverlay::from_config(&config),
            pane_zoom: std::collections::HashMap::new(),
            proxy,
            cmd_tx,
            surface: None,
            window: None,
            panes: None,
            model: UiModel::default(),
            layout: ViewLayout::default(),
            surface_size: (0, 0),
            mods: ModifiersState::empty(),
            cursor_px: (0.0, 0.0),
            selection: None,
            selecting: false,
            left_button_down: false,
            selection_dragging: false,
            press_origin: None,
            mouse_buttons: Vec::new(),
            clipboard: None,
            last_copied: None,
            error: None,
        }
    }

    /// Lazily creates and returns the shared clipboard handle.
    fn clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(clipboard) => self.clipboard = Some(clipboard),
                Err(err) => {
                    warn!(error = %err, "clipboard unavailable");
                    return None;
                }
            }
        }
        self.clipboard.as_mut()
    }

    fn into_result(self) -> io::Result<()> {
        match self.error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn send_cmd(&self, cmd: Cmd) {
        if let Err(err) = self.cmd_tx.send(cmd) {
            warn!(error = %err, "failed to queue gui command");
        }
    }

    fn recompute(&mut self) {
        let (w, h) = self.surface_size;
        if w == 0 || h == 0 {
            return;
        }
        self.layout = layout::compute(&self.model, w as i32, h as i32);
        self.reconcile_panes();
    }

    fn pane_zoom_for(&self, terminal_id: &str) -> f32 {
        self.pane_zoom
            .get(terminal_id)
            .copied()
            .unwrap_or(1.0)
            .clamp(PANE_ZOOM_MIN, PANE_ZOOM_MAX)
    }

    fn pane_cell_metrics(&mut self, terminal_id: &str) -> Option<(i32, i32, u32, u32)> {
        let zoom = self.pane_zoom_for(terminal_id);
        let pane_px = self.base_font_px * zoom;
        let pane_fonts = self.font_cache.get(pane_px).ok()?;
        let cell_w = pane_fonts.cell_width() as i32;
        let cell_h = pane_fonts.cell_height() as i32;
        Some((cell_w, cell_h, cell_w as u32, cell_h as u32))
    }

    fn reconcile_panes(&mut self) {
        let pane_targets: Vec<_> = self
            .layout
            .panes
            .iter()
            .filter(|slot| !slot.terminal_id.is_empty())
            .map(|slot| (slot.terminal_id.clone(), slot.content))
            .collect();
        let mut slots = Vec::with_capacity(pane_targets.len());
        for (terminal_id, content) in pane_targets {
            let metrics = self.pane_cell_metrics(&terminal_id);
            slots.push((terminal_id, content, metrics));
        }
        let retained = self.model.retained_terminal_ids();
        let Some(panes) = self.panes.as_mut() else {
            return;
        };
        for (terminal_id, content, metrics) in slots {
            let Some((cell_w, cell_h, cell_w_u, cell_h_u)) = metrics else {
                continue;
            };
            let cols = (content.w / cell_w).max(1) as u16;
            let rows = (content.h / cell_h).max(1) as u16;
            panes.ensure(&terminal_id, cols, rows, cell_w_u, cell_h_u);
        }
        panes.retain_visible(&retained);
    }

    fn is_visible(&self, terminal_id: &str) -> bool {
        self.layout
            .panes
            .iter()
            .any(|slot| slot.terminal_id == terminal_id)
    }

    fn request_redraw(&self) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn redraw(&mut self) {
        let Self {
            surface,
            window,
            fonts,
            font_cache,
            base_font_px,
            chrome,
            model,
            layout,
            panes,
            surface_size,
            selection,
            pane_zoom,
            settings,
            ..
        } = self;
        let (Some(surface), Some(window), Some(panes)) =
            (surface.as_mut(), window.as_ref(), panes.as_ref())
        else {
            return;
        };
        let highlight = selection.as_ref().map(|sel| view::Highlight {
            terminal_id: sel.terminal_id.clone(),
            start: sel.anchor,
            end: sel.head,
        });
        let (w, h) = *surface_size;
        let (Some(nw), Some(nh)) = (NonZeroU32::new(w), NonZeroU32::new(h)) else {
            return;
        };
        if let Err(err) = surface.resize(nw, nh) {
            warn!(error = %err, "surface resize failed");
            return;
        }
        let mut buffer = match surface.buffer_mut() {
            Ok(buffer) => buffer,
            Err(err) => {
                warn!(error = %err, "failed to acquire surface buffer");
                return;
            }
        };
        view::render(
            &mut buffer,
            w as usize,
            h as usize,
            fonts,
            font_cache,
            *base_font_px,
            chrome,
            model,
            layout,
            panes,
            pane_zoom,
            highlight.as_ref(),
            settings,
        );
        if let Err(err) = buffer.present() {
            warn!(error = %err, "failed to present buffer");
        }
        window.pre_present_notify();
    }

    fn focused_terminal(&self) -> Option<String> {
        self.model.focused_terminal_id().map(|s| s.to_string())
    }

    fn hit_inflated(rect: layout::Rect, x: i32, y: i32, pad: i32) -> bool {
        layout::Rect::new(
            rect.x - pad,
            rect.y - pad,
            rect.w + pad * 2,
            rect.h + pad * 2,
        )
        .contains(x, y)
    }

    fn pane_at_pixel(&self, x: i32, y: i32) -> Option<(String, i32, i32)> {
        for slot in &self.layout.panes {
            if slot.terminal_id.is_empty() || !slot.content.contains(x, y) {
                continue;
            }
            return Some((slot.terminal_id.clone(), slot.content.x, slot.content.y));
        }
        None
    }

    fn cell_at_pixel(&mut self, x: i32, y: i32) -> Option<(String, u16, u16)> {
        let (terminal_id, content_x, content_y) = self.pane_at_pixel(x, y)?;
        let (cell_w, cell_h, _, _) = self.pane_cell_metrics(&terminal_id)?;
        let panes = self.panes.as_ref()?;
        let slot = self
            .layout
            .panes
            .iter()
            .find(|slot| slot.terminal_id == terminal_id)?;
        let (max_cols, max_rows) = panes
            .frame(&terminal_id)
            .map(|f| (f.width, f.height))
            .unwrap_or((
                (slot.content.w / cell_w).max(1) as u16,
                (slot.content.h / cell_h).max(1) as u16,
            ));
        let col = (((x - content_x) / cell_w).max(0) as u16).min(max_cols.saturating_sub(1));
        let row = (((y - content_y) / cell_h).max(0) as u16).min(max_rows.saturating_sub(1));
        Some((terminal_id, col, row))
    }

    /// Forwards a mouse event to whichever pane is under the cursor.
    fn forward_mouse_to_pane(&mut self, kind: ClientMouseKind) {
        let (px, py) = (self.cursor_px.0 as i32, self.cursor_px.1 as i32);
        let mods = input::modifier_bits(self.mods);
        let Some((terminal_id, column, row)) = self.cell_at_pixel(px, py) else {
            return;
        };
        if let Some(panes) = self.panes.as_mut() {
            panes.send_input(
                &terminal_id,
                vec![ClientInputEvent::Mouse {
                    kind,
                    column,
                    row,
                    modifiers: mods,
                }],
            );
        }
    }

    /// Handles a left click on chrome. Returns true if a control was hit.
    fn handle_chrome_click(&mut self, x: i32, y: i32) -> bool {
        let layout = self.layout.clone();
        for row in &layout.workspace_rows {
            if row.rect.contains(x, y) {
                self.send_cmd(Cmd::FocusWorkspace(row.id.clone()));
                return true;
            }
        }
        for row in &layout.agent_rows {
            if row.rect.contains(x, y) {
                self.send_cmd(Cmd::FocusTarget(row.id.clone()));
                return true;
            }
        }
        if layout.new_workspace.contains(x, y) {
            self.send_cmd(Cmd::NewWorkspace);
            return true;
        }
        // Toolbar buttons before tabs so wide tab strips cannot steal clicks.
        if Self::hit_inflated(layout.btn_settings, x, y, 4) {
            self.open_settings();
            return true;
        }
        if Self::hit_inflated(layout.btn_close_pane, x, y, 4) {
            if let Some(pane_id) = self.model.focused_pane_id.clone() {
                self.send_cmd(Cmd::ClosePane(pane_id));
            }
            return true;
        }
        if Self::hit_inflated(layout.btn_split_down, x, y, 4) {
            self.split_focused(SplitDirection::Down);
            return true;
        }
        if Self::hit_inflated(layout.btn_split_right, x, y, 4) {
            self.split_focused(SplitDirection::Right);
            return true;
        }
        for tab in &layout.tabs {
            if tab.close.contains(x, y) {
                self.send_cmd(Cmd::CloseTab(tab.tab_id.clone()));
                return true;
            }
            if tab.rect.contains(x, y) {
                self.send_cmd(Cmd::FocusTab(tab.tab_id.clone()));
                return true;
            }
        }
        if layout.new_tab.contains(x, y) {
            self.send_cmd(Cmd::NewTab);
            return true;
        }
        false
    }

    fn split_focused(&self, dir: SplitDirection) {
        if let Some(pane_id) = self.model.focused_pane_id.clone() {
            self.send_cmd(Cmd::SplitPane { pane_id, dir });
        }
    }

    /// Focuses the pane under the cursor, if any.
    fn focus_pane_at(&mut self, x: i32, y: i32) {
        let mut target = None;
        for slot in &self.layout.panes {
            if slot.rect.contains(x, y) && !slot.terminal_id.is_empty() {
                target = Some(slot.terminal_id.clone());
                break;
            }
        }
        if let Some(terminal_id) = target {
            self.send_cmd(Cmd::FocusTarget(terminal_id));
        }
    }

    fn adjust_focused_pane_zoom(&mut self, factor: f32) {
        let Some(terminal_id) = self.focused_terminal() else {
            return;
        };
        let current = self.pane_zoom_for(&terminal_id);
        let next = (current * factor).clamp(PANE_ZOOM_MIN, PANE_ZOOM_MAX);
        if (next - current).abs() < f32::EPSILON {
            return;
        }
        self.pane_zoom.insert(terminal_id, next);
        self.recompute();
        self.request_redraw();
    }

    fn apply_gui_settings(&mut self) {
        self.base_font_px = self.settings.draft_font_size;
        self.font_cache.clear();
        if let Ok(fonts) = FontSet::load(self.base_font_px) {
            self.fonts = fonts;
        }
        self.chrome = ChromePalette::from_config(&Config {
            theme: crate::config::ThemeConfig {
                name: Some(self.settings.draft_theme.clone()),
                custom: None,
            },
            gui: crate::config::GuiConfig {
                font_size: self.settings.draft_font_size,
            },
            ..Config::default()
        });
        self.recompute();
        self.request_redraw();
    }

    fn handle_settings_click(&mut self, x: i32, y: i32) -> bool {
        if !self.settings.open {
            return false;
        }
        let (w, h) = (self.surface_size.0 as i32, self.surface_size.1 as i32);
        match settings::hit_test(x, y, w, h) {
            Some(SettingsHit::Close) => {
                if self.settings.has_unsaved_changes() {
                    self.settings.cancel();
                } else {
                    self.settings.open = false;
                }
            }
            Some(SettingsHit::Apply) => {
                if self.settings.save() {
                    self.settings.close_saved();
                    self.apply_gui_settings();
                }
            }
            Some(SettingsHit::Tab(section)) => self.settings.select_tab(section),
            Some(SettingsHit::ListItem(idx)) => {
                self.settings.selected = idx;
                self.settings.apply_selection();
            }
            Some(SettingsHit::Dismiss) => {
                if self.settings.has_unsaved_changes() {
                    self.settings.cancel();
                } else {
                    self.settings.open = false;
                }
            }
            None => {}
        }
        self.request_redraw();
        true
    }

    fn handle_settings_key(&mut self, key: &Key) -> bool {
        if !self.settings.open {
            return false;
        }
        match key {
            Key::Named(NamedKey::Escape) => {
                if self.settings.has_unsaved_changes() {
                    self.settings.cancel();
                } else {
                    self.settings.open = false;
                }
            }
            Key::Named(NamedKey::Enter) => {
                if self.settings.save() {
                    self.settings.close_saved();
                    self.apply_gui_settings();
                }
            }
            Key::Named(NamedKey::ArrowUp) => self.settings.move_prev(),
            Key::Named(NamedKey::ArrowDown) => self.settings.move_next(),
            Key::Named(NamedKey::ArrowLeft) => self.settings.select_tab(SettingsSection::Theme),
            Key::Named(NamedKey::ArrowRight) => self.settings.select_tab(SettingsSection::FontSize),
            Key::Character(text) if text == "1" => self.settings.select_tab(SettingsSection::Theme),
            Key::Character(text) if text == "2" => {
                self.settings.select_tab(SettingsSection::FontSize)
            }
            _ => return false,
        }
        self.request_redraw();
        true
    }

    fn try_handle_zoom_shortcut(&mut self, key: &Key) -> bool {
        if !self.mods.control_key() || self.settings.open {
            return false;
        }
        match key {
            Key::Character(text) if text == "=" || text == "+" => {
                self.adjust_focused_pane_zoom(1.0 + PANE_ZOOM_STEP);
                true
            }
            Key::Character(text) if text == "-" => {
                self.adjust_focused_pane_zoom(1.0 - PANE_ZOOM_STEP);
                true
            }
            _ => false,
        }
    }

    fn try_open_settings_shortcut(&mut self, key: &Key) -> bool {
        if self.mods.control_key() && matches!(key, Key::Character(text) if text == ",") {
            self.open_settings();
            return true;
        }
        false
    }

    fn open_settings(&mut self) {
        self.settings.open = true;
        self.settings.select_tab(SettingsSection::Theme);
        self.request_redraw();
    }

    fn begin_selection(&mut self, x: i32, y: i32) {
        if let Some((terminal_id, col, row)) = self.cell_at_pixel(x, y) {
            self.selection = Some(Selection {
                terminal_id,
                anchor: (col, row),
                head: (col, row),
            });
            self.selecting = true;
            self.selection_dragging = true;
            self.request_redraw();
        } else {
            self.clear_selection();
        }
    }

    fn update_selection(&mut self) {
        let (x, y) = (self.cursor_px.0 as i32, self.cursor_px.1 as i32);
        let Some((terminal_id, col, row)) = self.cell_at_pixel(x, y) else {
            return;
        };
        if let Some(sel) = self.selection.as_mut() {
            if sel.terminal_id == terminal_id {
                sel.head = (col, row);
                self.request_redraw();
            }
        }
    }

    fn maybe_begin_selection_drag(&mut self) {
        if self.selection_dragging || !self.left_button_down {
            return;
        }
        let (x, y) = (self.cursor_px.0 as i32, self.cursor_px.1 as i32);
        let Some((ox, oy)) = self.press_origin else {
            return;
        };
        if (x - ox).abs() + (y - oy).abs() < SELECTION_DRAG_THRESHOLD_PX {
            return;
        }
        self.begin_selection(ox, oy);
        self.update_selection();
    }

    fn finish_selection(&mut self) {
        if !self.selecting {
            return;
        }
        self.selecting = false;
        self.selection_dragging = false;
        match self.selection.as_ref() {
            Some(sel) if sel.anchor == sel.head => self.clear_selection(),
            Some(_) => self.copy_selection(),
            None => {}
        }
    }

    fn clear_selection(&mut self) {
        if self.selection.take().is_some() {
            self.request_redraw();
        }
        self.selecting = false;
        self.selection_dragging = false;
    }

    fn copy_selection(&mut self) {
        let Some(sel) = self.selection.clone() else {
            return;
        };
        let text = {
            let Some(panes) = self.panes.as_ref() else {
                return;
            };
            let Some(frame) = panes.frame(&sel.terminal_id) else {
                return;
            };
            selection_text(frame, sel.anchor, sel.head)
        };
        if text.trim().is_empty() {
            return;
        }
        let len = text.len();
        self.last_copied = Some(text.clone());
        match self.clipboard().map(|cb| cb.set_text(text)) {
            Some(Ok(())) => info!(len, "copied selection to clipboard"),
            Some(Err(err)) => warn!(error = %err, "failed to set clipboard"),
            None => {}
        }
    }

    fn paste_into_focused(&mut self) {
        let Some(terminal_id) = self.focused_terminal() else {
            return;
        };
        let system = self
            .clipboard()
            .and_then(|cb| cb.get_text().ok())
            .filter(|text| !text.is_empty());
        let text = match system.or_else(|| self.last_copied.clone()) {
            Some(text) if !text.is_empty() => text,
            _ => return,
        };
        if let Some(panes) = self.panes.as_mut() {
            panes.send_bytes(&terminal_id, text.into_bytes());
        }
    }
}

/// Extracts the text covered by a selection from a rendered frame.
fn selection_text(frame: &FrameData, anchor: (u16, u16), head: (u16, u16)) -> String {
    let (start, end) = view::ordered(anchor, head);
    let width = frame.width as usize;
    let mut lines: Vec<String> = Vec::new();
    for row in start.1..=end.1.min(frame.height.saturating_sub(1)) {
        let col_lo = if row == start.1 { start.0 } else { 0 };
        let col_hi = if row == end.1 {
            end.0
        } else {
            frame.width.saturating_sub(1)
        };
        let mut line = String::new();
        for col in col_lo..=col_hi {
            let idx = row as usize * width + col as usize;
            if let Some(cell) = frame.cells.get(idx) {
                line.push_str(&cell.symbol);
            }
        }
        lines.push(line.trim_end().to_string());
    }
    lines.join("\n")
}

impl ApplicationHandler<UserEvent> for NativeApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("herdr")
            .with_inner_size(LogicalSize::new(DEFAULT_WIDTH, DEFAULT_HEIGHT));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Rc::new(window),
            Err(err) => {
                self.error = Some(io::Error::other(format!("failed to create window: {err}")));
                event_loop.exit();
                return;
            }
        };
        let context = match softbuffer::Context::new(window.clone()) {
            Ok(context) => context,
            Err(err) => {
                self.error = Some(io::Error::other(format!("softbuffer context: {err}")));
                event_loop.exit();
                return;
            }
        };
        let surface = match softbuffer::Surface::new(&context, window.clone()) {
            Ok(surface) => surface,
            Err(err) => {
                self.error = Some(io::Error::other(format!("softbuffer surface: {err}")));
                event_loop.exit();
                return;
            }
        };

        let size = window.inner_size();
        self.surface_size = (size.width, size.height);
        self.panes = Some(PaneStreams::new(self.proxy.clone()));
        self.surface = Some(surface);
        self.window = Some(window);
        info!("native gui window created");
        self.recompute();
        self.request_redraw();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Model(model) => {
                self.model = *model;
                self.recompute();
                self.request_redraw();
            }
            UserEvent::PaneFrame {
                terminal_id,
                epoch,
                frame,
            } => {
                if let Some(panes) = self.panes.as_mut() {
                    panes.set_frame(&terminal_id, epoch, frame);
                }
                // Only repaint when the updated terminal is on the active tab.
                if self.is_visible(&terminal_id) {
                    self.request_redraw();
                }
            }
            UserEvent::PaneClosed { terminal_id, epoch } => {
                if let Some(panes) = self.panes.as_mut() {
                    panes.close_if_current(&terminal_id, epoch);
                }
                self.request_redraw();
            }
            UserEvent::ApiError(message) => {
                warn!(%message, "api error");
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.send_cmd(Cmd::Shutdown);
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                self.surface_size = (size.width, size.height);
                self.recompute();
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::ModifiersChanged(modifiers) => self.mods = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                if self.handle_settings_key(&event.logical_key) {
                    return;
                }
                if self.try_open_settings_shortcut(&event.logical_key) {
                    return;
                }
                if self.try_handle_zoom_shortcut(&event.logical_key) {
                    return;
                }
                if self.settings.open {
                    return;
                }
                if let Some(input) =
                    input::translate_key(&event.logical_key, self.mods, event.state, event.repeat)
                {
                    if let (Some(terminal_id), Some(panes)) =
                        (self.focused_terminal(), self.panes.as_mut())
                    {
                        panes.send_input(&terminal_id, vec![input]);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_px = (position.x, position.y);
                if self.settings.open {
                    return;
                }
                if self.selecting {
                    self.update_selection();
                } else if self.left_button_down {
                    self.maybe_begin_selection_drag();
                    if !self.selection_dragging {
                        self.forward_mouse_to_pane(ClientMouseKind::Drag(ClientMouseButton::Left));
                    }
                } else if !self.mouse_buttons.is_empty() {
                    self.forward_mouse_to_pane(ClientMouseKind::Moved);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(button) = input::mouse_button(button) else {
                    return;
                };
                let (x, y) = (self.cursor_px.0 as i32, self.cursor_px.1 as i32);
                if self.handle_settings_click(x, y) {
                    return;
                }
                if self.settings.open {
                    return;
                }
                match (button, state) {
                    (ClientMouseButton::Left, ElementState::Pressed) => {
                        if self.handle_chrome_click(x, y) {
                            self.clear_selection();
                            return;
                        }
                        if !self.pane_at_pixel(x, y).is_some() {
                            return;
                        }
                        self.focus_pane_at(x, y);
                        self.left_button_down = true;
                        self.press_origin = Some((x, y));
                        if !self.mouse_buttons.contains(&button) {
                            self.mouse_buttons.push(button);
                        }
                        self.forward_mouse_to_pane(ClientMouseKind::Down(button));
                    }
                    (ClientMouseButton::Left, ElementState::Released) => {
                        self.left_button_down = false;
                        self.press_origin = None;
                        self.mouse_buttons.retain(|b| *b != button);
                        self.forward_mouse_to_pane(ClientMouseKind::Up(button));
                        self.finish_selection();
                    }
                    (ClientMouseButton::Right, ElementState::Pressed) => {
                        self.paste_into_focused();
                    }
                    (ClientMouseButton::Middle, ElementState::Pressed) => {
                        if !self.mouse_buttons.contains(&button) {
                            self.mouse_buttons.push(button);
                        }
                        self.forward_mouse_to_pane(ClientMouseKind::Down(button));
                    }
                    (ClientMouseButton::Middle, ElementState::Released) => {
                        self.mouse_buttons.retain(|b| *b != button);
                        self.forward_mouse_to_pane(ClientMouseKind::Up(button));
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                for kind in wheel_kinds(delta) {
                    self.forward_mouse_to_pane(kind);
                }
            }
            _ => {}
        }
    }
}

fn wheel_kinds(delta: MouseScrollDelta) -> Vec<ClientMouseKind> {
    let (dx, dy) = match delta {
        MouseScrollDelta::LineDelta(x, y) => (x as f64, y as f64),
        MouseScrollDelta::PixelDelta(pos) => (pos.x / 16.0, pos.y / 16.0),
    };
    let mut kinds = Vec::new();
    if dy > 0.0 {
        kinds.push(ClientMouseKind::ScrollUp);
    } else if dy < 0.0 {
        kinds.push(ClientMouseKind::ScrollDown);
    }
    if dx > 0.0 {
        kinds.push(ClientMouseKind::ScrollRight);
    } else if dx < 0.0 {
        kinds.push(ClientMouseKind::ScrollLeft);
    }
    kinds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::CellData;

    fn frame_from(lines: &[&str]) -> FrameData {
        let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16;
        let height = lines.len() as u16;
        let mut cells = Vec::new();
        for line in lines {
            let chars: Vec<char> = line.chars().collect();
            for col in 0..width as usize {
                let symbol = chars.get(col).copied().unwrap_or(' ').to_string();
                cells.push(CellData {
                    symbol,
                    fg: 0,
                    bg: 0,
                    modifier: 0,
                    skip: false,
                    hyperlink: None,
                });
            }
        }
        FrameData {
            cells,
            width,
            height,
            cursor: None,
            hyperlinks: vec![],
            graphics: vec![],
        }
    }

    #[test]
    fn selection_single_line_trims_trailing_space() {
        let frame = frame_from(&["hello world   ", "second line"]);
        // Select "hello world" on row 0 (cols 0..=10).
        let text = selection_text(&frame, (0, 0), (10, 0));
        assert_eq!(text, "hello world");
    }

    #[test]
    fn selection_multi_line_joins_with_newline() {
        let frame = frame_from(&["abcdef", "ghijkl"]);
        // From (2,0) to (3,1): "cdef" + "\n" + "ghij".
        let text = selection_text(&frame, (2, 0), (3, 1));
        assert_eq!(text, "cdef\nghij");
    }

    #[test]
    fn selection_handles_reversed_endpoints() {
        let frame = frame_from(&["abcdef"]);
        let forward = selection_text(&frame, (1, 0), (3, 0));
        let backward = selection_text(&frame, (3, 0), (1, 0));
        assert_eq!(forward, "bcd");
        assert_eq!(backward, "bcd");
    }
}
