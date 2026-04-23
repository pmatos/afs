use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);

    match args.next().as_deref() {
        Some("daemon") => match afs::supervisor::run_foreground() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Some("ask") => match afs::client::ask(&args.collect::<Vec<_>>().join(" ")) {
            Ok(response) => {
                print!("{response}");
                ExitCode::SUCCESS
            }
            Err(afs::client::AskError::DaemonNotRunning) => {
                eprintln!("daemon is not running");
                ExitCode::FAILURE
            }
            Err(afs::client::AskError::Io(error)) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        _ => ExitCode::SUCCESS,
    }
}
