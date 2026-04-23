pub mod supervisor {
    use std::io::{self, BufRead, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::{SystemTime, UNIX_EPOCH};

    const SOCKET_FILE: &str = "supervisor.sock";
    const AGENT_HOME_DIR: &str = ".afs";
    const REGISTRY_FILE: &str = "registry.tsv";

    pub fn run_foreground() -> io::Result<()> {
        let home = home()?;
        std::fs::create_dir_all(&home)?;

        let listener = bind_supervisor_socket(&home)?;
        let mut state = SupervisorState::new(home);

        loop {
            match listener.accept() {
                Ok((stream, _address)) => {
                    let _ = handle_client(stream, &mut state);
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

    struct SupervisorState {
        home: PathBuf,
        agents: Vec<RegisteredAgent>,
    }

    struct RegisteredAgent {
        identity: String,
        managed_dir: PathBuf,
        process: Child,
    }

    impl SupervisorState {
        fn new(home: PathBuf) -> Self {
            Self {
                home,
                agents: Vec::new(),
            }
        }
    }

    fn handle_client(mut stream: UnixStream, state: &mut SupervisorState) -> io::Result<()> {
        let mut reader = io::BufReader::new(stream.try_clone()?);
        let mut request = String::new();
        if reader.read_line(&mut request)? == 0 {
            return Ok(());
        }

        let request = request.trim_end_matches('\n').trim_end_matches('\r');
        let response = if let Some(prompt) = request.strip_prefix("ASK ") {
            let _ = prompt;
            Ok("ask handling not implemented yet\n".to_string())
        } else if let Some(path) = request.strip_prefix("INSTALL ") {
            state.install(Path::new(path))
        } else if request == "AGENTS" {
            state.agents()
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unknown supervisor request",
            ))
        };

        match response {
            Ok(body) => {
                stream.write_all(b"OK\n")?;
                stream.write_all(body.as_bytes())
            }
            Err(error) => {
                stream.write_all(b"ERR\n")?;
                writeln!(stream, "{error}")
            }
        }
    }

    impl SupervisorState {
        fn install(&mut self, path: &Path) -> io::Result<String> {
            let managed_dir = path.canonicalize()?;
            if !managed_dir.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "managed path must be a directory",
                ));
            }

            let agent_home = managed_dir.join(AGENT_HOME_DIR);
            let history_dir = agent_home.join("history");
            std::fs::create_dir_all(&history_dir)?;

            let identity_path = agent_home.join("identity");
            let was_already_managed = identity_path.exists();
            let identity = if identity_path.exists() {
                std::fs::read_to_string(&identity_path)?
            } else {
                let identity = new_agent_identity()?;
                std::fs::write(&identity_path, &identity)?;
                identity
            };

            let instructions_path = agent_home.join("instructions.md");
            if !instructions_path.exists() {
                std::fs::write(
                    instructions_path,
                    "# Agent Instructions\n\nManage this directory through AFS.\n",
                )?;
            }

            let baseline_path = history_dir.join("baseline.tsv");
            if !baseline_path.exists() {
                let baseline = history_baseline(&managed_dir)?;
                std::fs::write(baseline_path, baseline)?;
            }

            if !self
                .agents
                .iter()
                .any(|agent| agent.managed_dir == managed_dir)
            {
                let process = start_directory_agent_process(&managed_dir)?;
                self.agents.push(RegisteredAgent {
                    identity: identity.trim().to_string(),
                    managed_dir: managed_dir.clone(),
                    process,
                });
                self.write_registry()?;
            }

            let status = if was_already_managed {
                "already managed directory"
            } else {
                "installed managed directory"
            };

            Ok(format!(
                "{status} {}\nagent {}\n",
                managed_dir.display(),
                identity.trim()
            ))
        }

        fn agents(&mut self) -> io::Result<String> {
            if self.agents.is_empty() {
                return Ok("no agents registered\n".to_string());
            }

            let mut status = String::new();
            for agent in &mut self.agents {
                let health = match agent.process.try_wait()? {
                    Some(_) => "stopped",
                    None => "running",
                };
                status.push_str(&format!(
                    "{}\tagent={}\truntime=pi-rpc-stdio\thealth={health}\tindex=warming\treconciliation=idle\tqueue=0\n",
                    agent.managed_dir.display(),
                    agent.identity
                ));
            }
            Ok(status)
        }

        fn write_registry(&self) -> io::Result<()> {
            let mut registry = String::from("identity\tmanaged_dir\n");
            for agent in &self.agents {
                registry.push_str(&agent.identity);
                registry.push('\t');
                registry.push_str(&agent.managed_dir.to_string_lossy());
                registry.push('\n');
            }
            std::fs::write(self.home.join(REGISTRY_FILE), registry)
        }
    }

    fn start_directory_agent_process(managed_dir: &Path) -> io::Result<Child> {
        Command::new(std::env::current_exe()?)
            .arg("__directory-agent")
            .arg(managed_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }

    fn new_agent_identity() -> io::Result<String> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        Ok(format!("agent-{}-{nanos}\n", std::process::id()))
    }

    fn history_baseline(managed_dir: &Path) -> io::Result<String> {
        let mut files = Vec::new();
        collect_baseline_files(managed_dir, managed_dir, &mut files)?;
        files.sort();

        let mut baseline = String::from("type\tbaseline\n");
        for (relative_path, len) in files {
            baseline.push_str("file\t");
            baseline.push_str(&relative_path);
            baseline.push('\t');
            baseline.push_str(&len.to_string());
            baseline.push('\n');
        }
        Ok(baseline)
    }

    fn collect_baseline_files(
        managed_dir: &Path,
        current_dir: &Path,
        files: &mut Vec<(String, u64)>,
    ) -> io::Result<()> {
        for entry in std::fs::read_dir(current_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path == managed_dir.join(AGENT_HOME_DIR) {
                continue;
            }

            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                collect_baseline_files(managed_dir, &path, files)?;
            } else if metadata.is_file() {
                let relative_path = path
                    .strip_prefix(managed_dir)
                    .map_err(io::Error::other)?
                    .to_string_lossy()
                    .replace('\\', "/");
                files.push((relative_path, metadata.len()));
            }
        }
        Ok(())
    }
}

pub mod client {
    use std::io::{self, Read, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;

    use crate::supervisor;

    #[derive(Debug)]
    pub enum Error {
        DaemonNotRunning,
        Supervisor(String),
        Io(io::Error),
    }

    pub fn ask(prompt: &str) -> Result<String, Error> {
        send_request(&format!("ASK {prompt}"))
    }

    pub fn install(path: &Path) -> Result<String, Error> {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map_err(Error::Io)?.join(path)
        };
        send_request(&format!("INSTALL {}", path.display()))
    }

    pub fn agents() -> Result<String, Error> {
        send_request("AGENTS")
    }

    fn send_request(request: &str) -> Result<String, Error> {
        let home = supervisor::home().map_err(Error::Io)?;
        let socket_path = supervisor::socket_path(&home);
        let mut stream = UnixStream::connect(socket_path).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => Error::DaemonNotRunning,
            _ => Error::Io(error),
        })?;

        writeln!(stream, "{request}").map_err(Error::Io)?;

        let mut response = String::new();
        stream.read_to_string(&mut response).map_err(Error::Io)?;

        if let Some(body) = response.strip_prefix("OK\n") {
            return Ok(body.to_string());
        }
        if let Some(message) = response.strip_prefix("ERR\n") {
            return Err(Error::Supervisor(message.trim_end().to_string()));
        }

        Err(Error::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            "supervisor returned an invalid response",
        )))
    }
}

pub mod directory_agent {
    use std::io::{self, Read};

    pub fn run_stdio() -> io::Result<()> {
        let mut stdin = io::stdin().lock();
        let mut buffer = [0_u8; 1024];
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) => return Ok(()),
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }
}
