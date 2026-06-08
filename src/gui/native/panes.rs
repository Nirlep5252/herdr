//! Per-pane terminal content streams.
//!
//! Each visible pane is backed by its own client-protocol connection that
//! attaches to one terminal and receives `SemanticFrame`s for just that
//! terminal's grid. Frames arrive on background reader threads and are
//! delivered to the winit loop, which stores them here for rendering. The set
//! of live connections is reconciled against the model's visible panes.

use std::collections::HashMap;
use std::collections::HashSet;

use interprocess::TryClone as _;
use tracing::warn;
use winit::event_loop::EventLoopProxy;

use crate::ipc::LocalStream;
use crate::protocol::{ClientInputEvent, ClientMessage, FrameData, ServerMessage};

use super::super::connection::{self, Geometry};
use super::UserEvent;

struct PaneConn {
    write: LocalStream,
    frame: Option<FrameData>,
    cols: u16,
    rows: u16,
    epoch: u64,
}

/// Manages one attach connection per visible terminal.
pub struct PaneStreams {
    proxy: EventLoopProxy<UserEvent>,
    cell_w: u32,
    cell_h: u32,
    conns: HashMap<String, PaneConn>,
    next_epoch: u64,
}

impl PaneStreams {
    pub fn new(proxy: EventLoopProxy<UserEvent>, cell_w: u32, cell_h: u32) -> Self {
        Self {
            proxy,
            cell_w,
            cell_h,
            conns: HashMap::new(),
            next_epoch: 1,
        }
    }

    /// Opens a connection for `terminal_id` if needed, or resizes an existing
    /// one when its grid dimensions change.
    pub fn ensure(&mut self, terminal_id: &str, cols: u16, rows: u16) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        if let Some(conn) = self.conns.get_mut(terminal_id) {
            if conn.cols != cols || conn.rows != rows {
                conn.cols = cols;
                conn.rows = rows;
                let msg = ClientMessage::Resize {
                    cols,
                    rows,
                    cell_width_px: self.cell_w,
                    cell_height_px: self.cell_h,
                };
                if let Err(err) = connection::write(&mut conn.write, &msg) {
                    warn!(terminal_id, error = %err, "pane resize failed");
                }
            }
            return;
        }

        let geometry = Geometry {
            cols,
            rows,
            cell_width_px: self.cell_w,
            cell_height_px: self.cell_h,
        };
        let epoch = self.next_epoch;
        self.next_epoch += 1;
        match connection::connect_attach(geometry, terminal_id) {
            Ok(stream) => match stream.try_clone() {
                Ok(read_half) => {
                    spawn_reader(
                        read_half,
                        terminal_id.to_string(),
                        epoch,
                        self.proxy.clone(),
                    );
                    self.conns.insert(
                        terminal_id.to_string(),
                        PaneConn {
                            write: stream,
                            frame: None,
                            cols,
                            rows,
                            epoch,
                        },
                    );
                }
                Err(err) => warn!(terminal_id, error = %err, "clone attach connection failed"),
            },
            Err(err) => warn!(terminal_id, error = %err, "attach to terminal failed"),
        }
    }

    /// Drops connections for terminals not in `keep`.
    pub fn retain_visible(&mut self, keep: &HashSet<String>) {
        self.conns
            .retain(|terminal_id, _| keep.contains(terminal_id));
    }

    /// Stores a frame only if it came from the current connection epoch.
    pub fn set_frame(&mut self, terminal_id: &str, epoch: u64, frame: FrameData) {
        if let Some(conn) = self.conns.get_mut(terminal_id) {
            if conn.epoch == epoch {
                conn.frame = Some(frame);
            }
        }
    }

    /// Removes a connection only if the closing report matches the current
    /// epoch, so a stale close from a previously dropped connection cannot tear
    /// down a freshly reopened one.
    pub fn close_if_current(&mut self, terminal_id: &str, epoch: u64) {
        if self
            .conns
            .get(terminal_id)
            .is_some_and(|conn| conn.epoch == epoch)
        {
            self.conns.remove(terminal_id);
        }
    }

    pub fn frame(&self, terminal_id: &str) -> Option<&FrameData> {
        self.conns
            .get(terminal_id)
            .and_then(|conn| conn.frame.as_ref())
    }

    /// Sends input events to a specific terminal.
    pub fn send_input(&mut self, terminal_id: &str, events: Vec<ClientInputEvent>) {
        if events.is_empty() {
            return;
        }
        if let Some(conn) = self.conns.get_mut(terminal_id) {
            let msg = ClientMessage::InputEvents { events };
            if let Err(err) = connection::write(&mut conn.write, &msg) {
                warn!(terminal_id, error = %err, "pane input send failed");
            }
        }
    }
}

fn spawn_reader(
    mut stream: LocalStream,
    terminal_id: String,
    epoch: u64,
    proxy: EventLoopProxy<UserEvent>,
) {
    std::thread::Builder::new()
        .name(format!("herdr-gui-pane-{terminal_id}"))
        .spawn(move || loop {
            match connection::read(&mut stream) {
                Ok(ServerMessage::Frame(frame)) => {
                    let event = UserEvent::PaneFrame {
                        terminal_id: terminal_id.clone(),
                        epoch,
                        frame,
                    };
                    if proxy.send_event(event).is_err() {
                        break;
                    }
                }
                Ok(ServerMessage::ServerShutdown { .. }) => {
                    proxy
                        .send_event(UserEvent::PaneClosed {
                            terminal_id: terminal_id.clone(),
                            epoch,
                        })
                        .ok();
                    break;
                }
                Ok(_) => {}
                Err(err) => {
                    warn!(terminal_id, error = %err, "pane stream ended");
                    proxy
                        .send_event(UserEvent::PaneClosed {
                            terminal_id: terminal_id.clone(),
                            epoch,
                        })
                        .ok();
                    break;
                }
            }
        })
        .expect("spawn pane reader thread");
}
