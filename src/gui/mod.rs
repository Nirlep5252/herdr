//! Native desktop GUI client for herdr (`herdr gui`).
//!
//! The GUI is a second client to the same headless server the terminal client
//! uses. It speaks the existing binary client protocol, requests full semantic
//! frames, and renders them in a native window with a CPU framebuffer
//! (`winit` + `softbuffer`) so no GPU is required. Agent terminal panes are
//! drawn exactly as the server composited them — the GUI never reinterprets
//! pane content.

use std::io;

#[cfg(feature = "gui")]
mod color;
#[cfg(feature = "gui")]
mod connection;
#[cfg(feature = "gui")]
mod draw;
#[cfg(feature = "gui")]
mod font;
#[cfg(feature = "gui")]
mod input;
#[cfg(feature = "gui")]
mod mirror;
#[cfg(feature = "gui")]
mod native;
#[cfg(feature = "gui")]
mod render;

/// Parsed `herdr gui` invocation.
#[derive(Debug, Default)]
struct GuiArgs {
    snapshot: Option<std::path::PathBuf>,
    mirror: bool,
    cols: u16,
    rows: u16,
}

/// Entry point for the `herdr gui` subcommand.
pub fn run(args: &[String]) -> io::Result<()> {
    let parsed = match parse_args(args) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };

    #[cfg(not(feature = "gui"))]
    {
        let _ = parsed;
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this build was compiled without the `gui` feature; rebuild with --features gui",
        ));
    }

    #[cfg(feature = "gui")]
    {
        crate::logging::init_file_logging("herdr-gui.log");
        connection::ensure_server_running()?;
        match parsed.snapshot {
            Some(path) => snapshot::capture(&path, parsed.cols, parsed.rows),
            None if parsed.mirror => mirror::run(),
            None => native::run(),
        }
    }
}

fn parse_args(args: &[String]) -> Result<GuiArgs, String> {
    let mut parsed = GuiArgs {
        cols: 120,
        rows: 34,
        ..GuiArgs::default()
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--snapshot" => {
                let path = args.get(i + 1).ok_or_else(|| {
                    "usage: herdr gui --snapshot <path.png> [--cols N --rows N]".to_string()
                })?;
                parsed.snapshot = Some(std::path::PathBuf::from(path));
                i += 2;
            }
            "--mirror" => {
                parsed.mirror = true;
                i += 1;
            }
            "--cols" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "--cols needs a value".to_string())?;
                parsed.cols = value
                    .parse()
                    .map_err(|_| "--cols must be a number".to_string())?;
                i += 2;
            }
            "--rows" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "--rows needs a value".to_string())?;
                parsed.rows = value
                    .parse()
                    .map_err(|_| "--rows must be a number".to_string())?;
                i += 2;
            }
            "help" | "--help" | "-h" => {
                println!("herdr gui — native desktop client");
                println!();
                println!("Usage: herdr gui                     native desktop UI (default)");
                println!("       herdr gui --mirror            faithful TUI mirror renderer");
                println!("       herdr gui --snapshot <path.png> [--cols N] [--rows N]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown gui option: {other}")),
        }
    }
    Ok(parsed)
}

/// Headless snapshot: connect, capture the first frame, render it to a PNG.
///
/// This exercises the full protocol + render path without opening a window,
/// which makes it usable for automated/offscreen verification.
#[cfg(feature = "gui")]
mod snapshot {
    use std::io::{self, BufWriter};
    use std::path::Path;
    use std::time::{Duration, Instant};

    use crate::protocol::ServerMessage;

    use super::connection::{self, Geometry};
    use super::font::FontSet;
    use super::render;

    const FONT_PX: f32 = 16.0;
    const FRAME_DEADLINE: Duration = Duration::from_secs(10);

    pub fn capture(path: &Path, cols: u16, rows: u16) -> io::Result<()> {
        let mut fonts = FontSet::load(FONT_PX).map_err(io::Error::other)?;
        let geometry = Geometry {
            cols,
            rows,
            cell_width_px: fonts.cell_width() as u32,
            cell_height_px: fonts.cell_height() as u32,
        };

        let mut stream = connection::connect(geometry)?;
        connection::set_recv_timeout_best_effort(&stream, Some(Duration::from_millis(500)))?;

        let deadline = Instant::now() + FRAME_DEADLINE;
        let frame = loop {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "no frame received from server",
                ));
            }
            match connection::read(&mut stream) {
                Ok(ServerMessage::Frame(frame)) => break frame,
                Ok(_) => continue,
                Err(err)
                    if err.kind() == io::ErrorKind::WouldBlock
                        || err.kind() == io::ErrorKind::TimedOut =>
                {
                    continue
                }
                Err(err) => return Err(err),
            }
        };

        let surface_w = cols as usize * fonts.cell_width();
        let surface_h = rows as usize * fonts.cell_height();
        let mut buffer = vec![0u32; surface_w * surface_h];
        render::render_frame(&frame, &mut fonts, &mut buffer, surface_w, surface_h);

        write_png(path, &buffer, surface_w as u32, surface_h as u32)?;
        eprintln!(
            "herdr gui: wrote {}x{} snapshot to {}",
            surface_w,
            surface_h,
            path.display()
        );
        Ok(())
    }

    fn write_png(path: &Path, buffer: &[u32], width: u32, height: u32) -> io::Result<()> {
        let mut rgb = Vec::with_capacity(buffer.len() * 3);
        for &pixel in buffer {
            let (r, g, b) = (
                ((pixel >> 16) & 0xFF) as u8,
                ((pixel >> 8) & 0xFF) as u8,
                (pixel & 0xFF) as u8,
            );
            rgb.extend_from_slice(&[r, g, b]);
        }
        let file = std::fs::File::create(path)?;
        let writer = BufWriter::new(file);
        let mut encoder = png::Encoder::new(writer, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut png_writer = encoder
            .write_header()
            .map_err(|err| io::Error::other(format!("png header: {err}")))?;
        png_writer
            .write_image_data(&rgb)
            .map_err(|err| io::Error::other(format!("png data: {err}")))?;
        Ok(())
    }
}
