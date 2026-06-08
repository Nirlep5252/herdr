//! Client-protocol connection management for the GUI.
//!
//! The GUI is just another client to the headless herdr server: it reuses the
//! same auto-detect/spawn behavior and the same binary client protocol as the
//! terminal thin client. This module owns connecting, the Hello/Welcome
//! handshake, and message framing helpers; rendering and input live elsewhere.

use std::io;
use std::time::Duration;

use interprocess::local_socket::traits::Stream as _;

use crate::ipc::{self, LocalStream};
use crate::protocol::{
    self, ClientKeybindings, ClientLaunchMode, ClientMessage, RenderEncoding, ServerMessage,
    MAX_GRAPHICS_FRAME_SIZE, PROTOCOL_VERSION,
};
use crate::server::autodetect;
use crate::server::socket_paths::client_socket_path;

const SERVER_READY_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Viewport geometry reported to the server during handshake and on resize.
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub cols: u16,
    pub rows: u16,
    pub cell_width_px: u32,
    pub cell_height_px: u32,
}

/// Ensures a herdr server is listening, spawning one if necessary.
pub fn ensure_server_running() -> io::Result<()> {
    let socket_path = client_socket_path();
    if autodetect::is_server_listening() {
        return Ok(());
    }
    autodetect::spawn_server_daemon()?;
    autodetect::wait_for_server_socket(&socket_path, SERVER_READY_TIMEOUT)
}

/// Connects to the server's client socket and performs the handshake,
/// requesting full semantic frames for an app client.
pub fn connect(geometry: Geometry) -> io::Result<LocalStream> {
    let socket_path = client_socket_path();
    let mut stream = ipc::connect_local_stream(&socket_path)?;
    stream.set_nonblocking(false)?;

    let hello = ClientMessage::Hello {
        version: PROTOCOL_VERSION,
        cols: geometry.cols,
        rows: geometry.rows,
        cell_width_px: geometry.cell_width_px,
        cell_height_px: geometry.cell_height_px,
        requested_encoding: RenderEncoding::SemanticFrame,
        keybindings: ClientKeybindings::Server,
        launch_mode: ClientLaunchMode::App,
    };
    write(&mut stream, &hello)?;

    // Named pipes (Windows) reject recv timeouts; the server sends Welcome
    // immediately, so a plain blocking read is fine there.
    set_recv_timeout_best_effort(&stream, Some(HANDSHAKE_TIMEOUT))?;
    let welcome: ServerMessage = read(&mut stream)?;
    set_recv_timeout_best_effort(&stream, None)?;

    match welcome {
        ServerMessage::Welcome {
            encoding, error, ..
        } => {
            if let Some(error) = error {
                return Err(io::Error::other(format!("server rejected client: {error}")));
            }
            if encoding != RenderEncoding::SemanticFrame {
                return Err(io::Error::other(
                    "server did not grant semantic-frame encoding",
                ));
            }
            Ok(stream)
        }
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected Welcome, got {other:?}"),
        )),
    }
}

/// Sets a receive timeout, treating "unsupported" (Windows named pipes) as a
/// no-op so callers can rely on blocking reads on every platform.
pub fn set_recv_timeout_best_effort(
    stream: &LocalStream,
    timeout: Option<Duration>,
) -> io::Result<()> {
    match stream.set_recv_timeout(timeout) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::Unsupported => Ok(()),
        Err(err) => Err(err),
    }
}

/// Writes a client message to the server.
pub fn write(stream: &mut LocalStream, msg: &ClientMessage) -> io::Result<()> {
    protocol::write_message(stream, msg).map_err(framing_to_io)
}

/// Reads a single server message (blocking).
pub fn read(stream: &mut LocalStream) -> io::Result<ServerMessage> {
    protocol::read_message(stream, MAX_GRAPHICS_FRAME_SIZE).map_err(framing_to_io)
}

fn framing_to_io(err: protocol::FramingError) -> io::Error {
    match err {
        protocol::FramingError::Io(io_err) => io_err,
        other => io::Error::other(other.to_string()),
    }
}
