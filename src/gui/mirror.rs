//! winit + softbuffer event loop for the GUI client.
//!
//! Owns the native window, presents rendered frames with a CPU framebuffer
//! (no GPU required), and forwards input back to the server. Server messages
//! arrive on a background reader thread and are delivered to the loop via an
//! [`winit::event_loop::EventLoopProxy`].

use std::io;
use std::num::NonZeroU32;
use std::rc::Rc;

use interprocess::TryClone as _;
use tracing::{info, warn};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::ipc::LocalStream;
use crate::protocol::{
    ClientInputEvent, ClientMessage, ClientMouseButton, ClientMouseKind, NotifyKind, ServerMessage,
};

use super::connection::{self, Geometry};
use super::font::FontSet;
use super::input;
use super::render;

const DEFAULT_WIDTH: f64 = 1100.0;
const DEFAULT_HEIGHT: f64 = 720.0;
const FONT_PX: f32 = 16.0;

/// Messages delivered into the winit loop from the background reader thread.
#[derive(Debug)]
enum UserEvent {
    Server(ServerMessage),
    Disconnected,
}

/// Runs the GUI client event loop. Assumes a server is already reachable.
pub fn run() -> io::Result<()> {
    let fonts = FontSet::load(FONT_PX).map_err(io::Error::other)?;

    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|err| io::Error::other(format!("failed to build event loop: {err}")))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();

    let mut app = GuiApp::new(fonts, proxy);
    event_loop
        .run_app(&mut app)
        .map_err(|err| io::Error::other(format!("event loop error: {err}")))?;

    app.into_result()
}

struct GuiApp {
    fonts: FontSet,
    proxy: EventLoopProxy<UserEvent>,
    // Surface must be declared before `window` so it is dropped first.
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    window: Option<Rc<Window>>,
    write_stream: Option<LocalStream>,
    frame: Option<crate::protocol::FrameData>,
    geometry: Geometry,
    surface_size: (u32, u32),
    mods: ModifiersState,
    cursor_px: (f64, f64),
    buttons: Vec<ClientMouseButton>,
    error: Option<io::Error>,
}

impl GuiApp {
    fn new(fonts: FontSet, proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            fonts,
            proxy,
            surface: None,
            window: None,
            write_stream: None,
            frame: None,
            geometry: Geometry {
                cols: 80,
                rows: 24,
                cell_width_px: 0,
                cell_height_px: 0,
            },
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

    fn cols_rows(&self, width: u32, height: u32) -> (u16, u16) {
        let cw = self.fonts.cell_width().max(1) as u32;
        let ch = self.fonts.cell_height().max(1) as u32;
        let cols = (width / cw).max(1).min(u16::MAX as u32) as u16;
        let rows = (height / ch).max(1).min(u16::MAX as u32) as u16;
        (cols, rows)
    }

    /// Sends a single input event to the server, recording fatal write errors.
    fn send_input(&mut self, event: ClientInputEvent) {
        self.send(ClientMessage::InputEvents {
            events: vec![event],
        });
    }

    fn send(&mut self, msg: ClientMessage) {
        if let Some(stream) = self.write_stream.as_mut() {
            if let Err(err) = connection::write(stream, &msg) {
                warn!(error = %err, "failed to send to server");
                self.error = Some(err);
            }
        }
    }

    fn redraw(&mut self) {
        let (Some(surface), Some(window)) = (self.surface.as_mut(), self.window.as_ref()) else {
            return;
        };
        let (w, h) = self.surface_size;
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
        if let Some(frame) = self.frame.as_ref() {
            render::render_frame(frame, &mut self.fonts, &mut buffer, w as usize, h as usize);
        } else {
            let bg = super::color::pack_rgb(super::color::DEFAULT_BG);
            for pixel in buffer.iter_mut() {
                *pixel = bg;
            }
        }
        if let Err(err) = buffer.present() {
            warn!(error = %err, "failed to present buffer");
        }
        window.pre_present_notify();
    }

    fn resize_surface(&mut self, width: u32, height: u32) {
        self.surface_size = (width, height);
        let (cols, rows) = self.cols_rows(width, height);
        if (cols, rows) != (self.geometry.cols, self.geometry.rows) {
            self.geometry.cols = cols;
            self.geometry.rows = rows;
            self.send(ClientMessage::Resize {
                cols,
                rows,
                cell_width_px: self.geometry.cell_width_px,
                cell_height_px: self.geometry.cell_height_px,
            });
        }
    }
}

impl ApplicationHandler<UserEvent> for GuiApp {
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
                self.error = Some(io::Error::other(format!(
                    "failed to create softbuffer context: {err}"
                )));
                event_loop.exit();
                return;
            }
        };
        let surface = match softbuffer::Surface::new(&context, window.clone()) {
            Ok(surface) => surface,
            Err(err) => {
                self.error = Some(io::Error::other(format!(
                    "failed to create softbuffer surface: {err}"
                )));
                event_loop.exit();
                return;
            }
        };

        let size = window.inner_size();
        let (cols, rows) = self.cols_rows(size.width, size.height);
        self.geometry = Geometry {
            cols,
            rows,
            cell_width_px: self.fonts.cell_width() as u32,
            cell_height_px: self.fonts.cell_height() as u32,
        };
        self.surface_size = (size.width, size.height);

        match connection::connect(self.geometry) {
            Ok(stream) => {
                match stream.try_clone() {
                    Ok(read_half) => spawn_reader(read_half, self.proxy.clone()),
                    Err(err) => {
                        self.error = Some(io::Error::other(format!(
                            "failed to clone server connection: {err}"
                        )));
                        event_loop.exit();
                        return;
                    }
                }
                self.write_stream = Some(stream);
                info!(cols, rows, "gui connected to server");
            }
            Err(err) => {
                self.error = Some(err);
                event_loop.exit();
                return;
            }
        }

        self.surface = Some(surface);
        self.window = Some(window);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Server(message) => {
                self.handle_server_message(message);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            UserEvent::Disconnected => {
                info!("server disconnected");
                event_loop.exit();
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
                self.send(ClientMessage::Detach);
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                self.resize_surface(size.width, size.height);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::ModifiersChanged(modifiers) => {
                self.mods = modifiers.state();
            }
            WindowEvent::Focused(focused) => {
                let event = if focused {
                    ClientInputEvent::FocusGained
                } else {
                    ClientInputEvent::FocusLost
                };
                self.send_input(event);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(input) =
                    input::translate_key(&event.logical_key, self.mods, event.state, event.repeat)
                {
                    self.send_input(input);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_px = (position.x, position.y);
                let (col, row) = self.cursor_cell();
                let kind = match self.buttons.first() {
                    Some(button) => ClientMouseKind::Drag(*button),
                    None => ClientMouseKind::Moved,
                };
                self.send_mouse(kind, col, row);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(button) = input::mouse_button(button) else {
                    return;
                };
                let (col, row) = self.cursor_cell();
                match state {
                    ElementState::Pressed => {
                        if !self.buttons.contains(&button) {
                            self.buttons.push(button);
                        }
                        self.send_mouse(ClientMouseKind::Down(button), col, row);
                    }
                    ElementState::Released => {
                        self.buttons.retain(|b| *b != button);
                        self.send_mouse(ClientMouseKind::Up(button), col, row);
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (col, row) = self.cursor_cell();
                for kind in wheel_kinds(delta) {
                    self.send_mouse(kind, col, row);
                }
            }
            _ => {}
        }
    }
}

impl GuiApp {
    fn cursor_cell(&self) -> (u16, u16) {
        input::pixel_to_cell(
            self.cursor_px.0,
            self.cursor_px.1,
            self.fonts.cell_width(),
            self.fonts.cell_height(),
            self.geometry.cols,
            self.geometry.rows,
        )
    }

    fn send_mouse(&mut self, kind: ClientMouseKind, column: u16, row: u16) {
        let modifiers = input::modifier_bits(self.mods);
        self.send_input(ClientInputEvent::Mouse {
            kind,
            column,
            row,
            modifiers,
        });
    }

    fn handle_server_message(&mut self, message: ServerMessage) {
        match message {
            ServerMessage::Frame(frame) => {
                self.frame = Some(frame);
            }
            ServerMessage::ServerShutdown { reason } => {
                info!(reason, "server shutting down");
                self.proxy.send_event(UserEvent::Disconnected).ok();
            }
            ServerMessage::Notify { kind, message, .. } => {
                if matches!(kind, NotifyKind::Toast | NotifyKind::SystemToast) {
                    info!(%message, "server toast");
                }
            }
            // Clipboard, graphics, and terminal-ansi handling are tracked for a
            // later pass; semantic-frame app clients do not receive Terminal.
            _ => {}
        }
    }
}

fn wheel_kinds(delta: MouseScrollDelta) -> Vec<ClientMouseKind> {
    let mut kinds = Vec::new();
    let (dx, dy) = match delta {
        MouseScrollDelta::LineDelta(x, y) => (x as f64, y as f64),
        MouseScrollDelta::PixelDelta(pos) => (pos.x / 16.0, pos.y / 16.0),
    };
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

fn spawn_reader(mut stream: LocalStream, proxy: EventLoopProxy<UserEvent>) {
    std::thread::Builder::new()
        .name("herdr-gui-reader".to_string())
        .spawn(move || loop {
            match connection::read(&mut stream) {
                Ok(message) => {
                    if proxy.send_event(UserEvent::Server(message)).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    warn!(error = %err, "server reader stopped");
                    proxy.send_event(UserEvent::Disconnected).ok();
                    break;
                }
            }
        })
        .expect("spawn gui reader thread");
}
