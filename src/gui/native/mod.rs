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

use std::collections::HashSet;
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
        frame: FrameData,
    },
    PaneClosed {
        terminal_id: String,
    },
    ApiError(String),
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
    buttons: Vec<ClientMouseButton>,
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
            buttons: Vec::new(),
            error: None,
        }
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
        let mut visible = HashSet::new();
        for slot in &self.layout.panes {
            if slot.terminal_id.is_empty() {
                continue;
            }
            let cols = (slot.content.w / cell_w).max(1) as u16;
            let rows = (slot.content.h / cell_h).max(1) as u16;
            visible.insert(slot.terminal_id.clone());
            panes.ensure(&slot.terminal_id, cols, rows);
        }
        panes.retain_visible(&visible);
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
            ..
        } = self;
        let (Some(surface), Some(window), Some(panes)) =
            (surface.as_mut(), window.as_ref(), panes.as_ref())
        else {
            return;
        };
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
            UserEvent::PaneFrame { terminal_id, frame } => {
                if let Some(panes) = self.panes.as_mut() {
                    panes.set_frame(&terminal_id, frame);
                }
                self.request_redraw();
            }
            UserEvent::PaneClosed { terminal_id } => {
                if let Some(panes) = self.panes.as_mut() {
                    panes.remove(&terminal_id);
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
                if let Some(button) = self.buttons.first().copied() {
                    self.forward_mouse_to_pane(ClientMouseKind::Drag(button));
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(button) = input::mouse_button(button) else {
                    return;
                };
                let (x, y) = (self.cursor_px.0 as i32, self.cursor_px.1 as i32);
                match state {
                    ElementState::Pressed => {
                        if !self.buttons.contains(&button) {
                            self.buttons.push(button);
                        }
                        if button == ClientMouseButton::Left && self.handle_chrome_click(x, y) {
                            return;
                        }
                        self.focus_pane_at(x, y);
                        self.forward_mouse_to_pane(ClientMouseKind::Down(button));
                    }
                    ElementState::Released => {
                        self.buttons.retain(|b| *b != button);
                        self.forward_mouse_to_pane(ClientMouseKind::Up(button));
                    }
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
