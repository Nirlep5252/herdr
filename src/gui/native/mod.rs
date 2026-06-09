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
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::api::schema::SplitDirection;
use crate::protocol::{ClientInputEvent, ClientMouseButton, ClientMouseKind, FrameData};

use super::font::FontSet;
use super::input;
use layout::ViewLayout;
use model::UiModel;
use panes::PaneStreams;
use worker::Cmd;

const DEFAULT_WIDTH: f64 = 1200.0;
const DEFAULT_HEIGHT: f64 = 780.0;
const FONT_PX: f32 = 15.0;

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
    let fonts = FontSet::load(FONT_PX).map_err(io::Error::other)?;
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|err| io::Error::other(format!("failed to build event loop: {err}")))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let cmd_tx = worker::spawn(proxy.clone());

    let mut app = NativeApp::new(fonts, proxy, cmd_tx);
    event_loop
        .run_app(&mut app)
        .map_err(|err| io::Error::other(format!("event loop error: {err}")))?;
    app.into_result()
}

struct NativeApp {
    fonts: FontSet,
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
    // A single long-lived clipboard handle. On X11 the process must keep
    // owning the selection for the contents to persist, so we must not
    // create/drop a handle per copy.
    clipboard: Option<arboard::Clipboard>,
    // Last text we copied, used as a paste fallback when the system clipboard
    // is unreadable (e.g. headless X servers without a clipboard manager).
    last_copied: Option<String>,
    error: Option<io::Error>,
}

impl NativeApp {
    fn new(fonts: FontSet, proxy: EventLoopProxy<UserEvent>, cmd_tx: Sender<Cmd>) -> Self {
        Self {
            fonts,
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

    fn reconcile_panes(&mut self) {
        let Some(panes) = self.panes.as_mut() else {
            return;
        };
        let cell_w = self.fonts.cell_width() as i32;
        let cell_h = self.fonts.cell_height() as i32;
        // Open/resize connections for panes in the active tab.
        for slot in &self.layout.panes {
            if slot.terminal_id.is_empty() {
                continue;
            }
            let cols = (slot.content.w / cell_w).max(1) as u16;
            let rows = (slot.content.h / cell_h).max(1) as u16;
            panes.ensure(&slot.terminal_id, cols, rows);
        }
        // Keep connections for every pane in every workspace so switching tabs
        // or spaces does not tear down and re-attach (which caused blank panes),
        // only dropping terminals that left the server entirely.
        panes.retain_visible(&self.model.retained_terminal_ids());
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
            model,
            layout,
            panes,
            surface_size,
            selection,
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
            model,
            layout,
            panes,
            highlight.as_ref(),
        );
        if let Err(err) = buffer.present() {
            warn!(error = %err, "failed to present buffer");
        }
        window.pre_present_notify();
    }

    fn focused_terminal(&self) -> Option<String> {
        self.model.focused_terminal_id().map(|s| s.to_string())
    }

    /// Forwards a mouse event to whichever pane is under the cursor.
    fn forward_mouse_to_pane(&mut self, kind: ClientMouseKind) {
        let (px, py) = (self.cursor_px.0 as i32, self.cursor_px.1 as i32);
        let cell_w = self.fonts.cell_width() as i32;
        let cell_h = self.fonts.cell_height() as i32;
        let mods = input::modifier_bits(self.mods);
        let mut target: Option<(String, u16, u16)> = None;
        for slot in &self.layout.panes {
            if slot.content.contains(px, py) && !slot.terminal_id.is_empty() {
                let col = ((px - slot.content.x) / cell_w).max(0) as u16;
                let row = ((py - slot.content.y) / cell_h).max(0) as u16;
                target = Some((slot.terminal_id.clone(), col, row));
                break;
            }
        }
        if let (Some((terminal_id, column, row)), Some(panes)) = (target, self.panes.as_mut()) {
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
        if layout.btn_split_right.contains(x, y) {
            self.split_focused(SplitDirection::Right);
            return true;
        }
        if layout.btn_split_down.contains(x, y) {
            self.split_focused(SplitDirection::Down);
            return true;
        }
        if layout.btn_close_pane.contains(x, y) {
            if let Some(pane_id) = self.model.focused_pane_id.clone() {
                self.send_cmd(Cmd::ClosePane(pane_id));
            }
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

    /// Maps a pixel position to a pane's terminal id and clamped cell.
    fn pane_cell_at(&self, x: i32, y: i32) -> Option<(String, u16, u16)> {
        let cell_w = self.fonts.cell_width() as i32;
        let cell_h = self.fonts.cell_height() as i32;
        let panes = self.panes.as_ref()?;
        for slot in &self.layout.panes {
            if slot.terminal_id.is_empty() || !slot.content.contains(x, y) {
                continue;
            }
            let (max_cols, max_rows) = panes
                .frame(&slot.terminal_id)
                .map(|f| (f.width, f.height))
                .unwrap_or((
                    (slot.content.w / cell_w).max(1) as u16,
                    (slot.content.h / cell_h).max(1) as u16,
                ));
            let col =
                (((x - slot.content.x) / cell_w).max(0) as u16).min(max_cols.saturating_sub(1));
            let row =
                (((y - slot.content.y) / cell_h).max(0) as u16).min(max_rows.saturating_sub(1));
            return Some((slot.terminal_id.clone(), col, row));
        }
        None
    }

    fn begin_selection(&mut self, x: i32, y: i32) {
        if let Some((terminal_id, col, row)) = self.pane_cell_at(x, y) {
            self.selection = Some(Selection {
                terminal_id,
                anchor: (col, row),
                head: (col, row),
            });
            self.selecting = true;
            self.request_redraw();
        } else {
            self.clear_selection();
        }
    }

    fn update_selection(&mut self) {
        let (x, y) = (self.cursor_px.0 as i32, self.cursor_px.1 as i32);
        let Some((terminal_id, col, row)) = self.pane_cell_at(x, y) else {
            return;
        };
        if let Some(sel) = self.selection.as_mut() {
            if sel.terminal_id == terminal_id {
                sel.head = (col, row);
                self.request_redraw();
            }
        }
    }

    fn finish_selection(&mut self) {
        if !self.selecting {
            return;
        }
        self.selecting = false;
        match self.selection.as_ref() {
            Some(sel) if sel.anchor == sel.head => {
                // A plain click, not a drag: clear the (empty) selection.
                self.clear_selection();
            }
            Some(_) => self.copy_selection(),
            None => {}
        }
    }

    fn clear_selection(&mut self) {
        if self.selection.take().is_some() {
            self.request_redraw();
        }
        self.selecting = false;
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
        self.panes = Some(PaneStreams::new(
            self.proxy.clone(),
            self.fonts.cell_width() as u32,
            self.fonts.cell_height() as u32,
        ));
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
                if self.selecting {
                    self.update_selection();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(button) = input::mouse_button(button) else {
                    return;
                };
                let (x, y) = (self.cursor_px.0 as i32, self.cursor_px.1 as i32);
                match (button, state) {
                    (ClientMouseButton::Left, ElementState::Pressed) => {
                        if self.handle_chrome_click(x, y) {
                            self.clear_selection();
                            return;
                        }
                        self.focus_pane_at(x, y);
                        self.begin_selection(x, y);
                    }
                    (ClientMouseButton::Left, ElementState::Released) => {
                        self.finish_selection();
                    }
                    (ClientMouseButton::Right, ElementState::Pressed) => {
                        self.paste_into_focused();
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
