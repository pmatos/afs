pub mod supervisor {
    use std::io::{self, BufRead, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};

    const SOCKET_FILE: &str = "supervisor.sock";

    pub fn run_foreground() -> io::Result<()> {
        let home = home()?;
        std::fs::create_dir_all(&home)?;

        let listener = bind_supervisor_socket(&home)?;

        loop {
            match listener.accept() {
                Ok((stream, _address)) => {
                    let _ = handle_client(stream);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }

    pub fn home() -> io::Result<PathBuf> {
        if let Some(home) = std::env::var_os("AFS_HOME") {
            return Ok(home.into());
        }

        let Some(home) = std::env::var_os("HOME") else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "HOME is not set"));
        };

        Ok(PathBuf::from(home).join(".afs"))
    }

    pub fn socket_path(home: &Path) -> PathBuf {
        home.join(SOCKET_FILE)
    }

    fn bind_supervisor_socket(home: &Path) -> io::Result<UnixListener> {
        let socket_path = socket_path(home);
        if socket_path.exists() {
            if UnixStream::connect(&socket_path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "supervisor daemon already running",
                ));
            }
            std::fs::remove_file(&socket_path)?;
        }

        UnixListener::bind(socket_path)
    }

    fn handle_client(mut stream: UnixStream) -> io::Result<()> {
        let mut reader = io::BufReader::new(stream.try_clone()?);
        let mut request = String::new();
        if reader.read_line(&mut request)? == 0 {
            return Ok(());
        }

        stream.write_all(b"ask handling not implemented yet\n")
    }
}

pub mod client {
    use std::io::{self, Read, Write};
    use std::os::unix::net::UnixStream;

    use crate::supervisor;

    #[derive(Debug)]
    pub enum AskError {
        DaemonNotRunning,
        Io(io::Error),
    }

    pub fn ask(prompt: &str) -> Result<String, AskError> {
        let home = supervisor::home().map_err(AskError::Io)?;
        let socket_path = supervisor::socket_path(&home);
        let mut stream = UnixStream::connect(socket_path).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
                AskError::DaemonNotRunning
            }
            _ => AskError::Io(error),
        })?;

        writeln!(stream, "{prompt}").map_err(AskError::Io)?;

        let mut response = String::new();
        stream.read_to_string(&mut response).map_err(AskError::Io)?;
        Ok(response)
    }
}
