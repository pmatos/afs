use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);

    match args.next().as_deref() {
        Some("login") => {
            let mut provider: Option<String> = None;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--provider" => provider = args.next(),
                    other if other.starts_with("--provider=") => {
                        provider = Some(other["--provider=".len()..].to_string());
                    }
                    _ => {
                        eprintln!("usage: afs login --provider <claude|openai>");
                        return ExitCode::FAILURE;
                    }
                }
            }

            match afs::login::run(provider.as_deref()) {
                Ok(message) => {
                    print!("{message}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            }
        }
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
        Some("remove") => {
            let Some(path) = args.next() else {
                eprintln!("usage: afs remove <path>");
                return ExitCode::FAILURE;
            };

            match afs::client::remove(std::path::Path::new(&path)) {
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
        Some("history") => {
            let Some(path) = args.next() else {
                eprintln!("usage: afs history <path>");
                return ExitCode::FAILURE;
            };

            match afs::client::history(std::path::Path::new(&path)) {
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
        Some("undo") => {
            let Some(path) = args.next() else {
                eprintln!("usage: afs undo <path> <history-entry> [--yes]");
                return ExitCode::FAILURE;
            };
            let Some(history_entry) = args.next() else {
                eprintln!("usage: afs undo <path> <history-entry> [--yes]");
                return ExitCode::FAILURE;
            };

            let confirmed = args.any(|argument| argument == "--yes" || argument == "-y")
                || confirm_interactive_undo(&history_entry);

            match afs::client::undo(std::path::Path::new(&path), &history_entry, confirmed) {
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
        _ => ExitCode::SUCCESS,
    }
}

fn confirm_interactive_undo(history_entry: &str) -> bool {
    if !io::stdin().is_terminal() {
        return false;
    }

    print!("Undo history entry {history_entry}? [y/N] ");
    let _ = io::stdout().flush();

    let mut answer = String::new();
    if io::stdin().read_line(&mut answer).is_err() {
        return false;
    }

    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}
