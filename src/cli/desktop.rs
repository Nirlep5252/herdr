use crate::api::schema::{EmptyParams, Method, Request};

pub(super) fn run_desktop_command(args: &[String]) -> std::io::Result<i32> {
    let Some(subcommand) = args.first().map(|arg| arg.as_str()) else {
        print_desktop_help();
        return Ok(2);
    };

    match subcommand {
        "snapshot" if args.len() == 1 => desktop_snapshot(),
        "help" | "--help" | "-h" => {
            print_desktop_help();
            Ok(0)
        }
        _ => {
            print_desktop_help();
            Ok(2)
        }
    }
}

fn desktop_snapshot() -> std::io::Result<i32> {
    super::print_response(&super::send_request(&Request {
        id: "cli:desktop:snapshot".into(),
        method: Method::DesktopSnapshot(EmptyParams::default()),
    })?)
}

fn print_desktop_help() {
    eprintln!("herdr desktop commands:");
    eprintln!("  herdr desktop snapshot  print the Herdr Desktop shell model as JSON");
}
