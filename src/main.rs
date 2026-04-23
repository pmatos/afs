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
            Err(afs::client::Error::DaemonNotRunning) => {
                eprintln!("daemon is not running");
                ExitCode::FAILURE
            }
            Err(afs::client::Error::Supervisor(message)) => {
                eprintln!("{message}");
                ExitCode::FAILURE
            }
            Err(afs::client::Error::Io(error)) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Some("install") => {
            let Some(path) = args.next() else {
                eprintln!("usage: afs install <path>");
                return ExitCode::FAILURE;
            };

            match afs::client::install(std::path::Path::new(&path)) {
                Ok(response) => {
                    print!("{response}");
                    ExitCode::SUCCESS
                }
                Err(afs::client::Error::DaemonNotRunning) => {
                    eprintln!("daemon is not running");
                    ExitCode::FAILURE
                }
                Err(afs::client::Error::Supervisor(message)) => {
                    eprintln!("{message}");
                    ExitCode::FAILURE
                }
                Err(afs::client::Error::Io(error)) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("agents") => match afs::client::agents() {
            Ok(response) => {
                print!("{response}");
                ExitCode::SUCCESS
            }
            Err(afs::client::Error::DaemonNotRunning) => {
                eprintln!("daemon is not running");
                ExitCode::FAILURE
            }
            Err(afs::client::Error::Supervisor(message)) => {
                eprintln!("{message}");
                ExitCode::FAILURE
            }
            Err(afs::client::Error::Io(error)) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        _ => ExitCode::SUCCESS,
    }
}
