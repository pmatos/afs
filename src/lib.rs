pub mod supervisor {
    use notify::{RecommendedWatcher, RecursiveMode, Watcher};
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs::OpenOptions;
    use std::io::{self, BufRead, Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
    use std::sync::{Mutex, OnceLock};
    use std::thread;
    use std::time::{Duration, Instant};
    use std::time::{SystemTime, UNIX_EPOCH};

    const SOCKET_FILE: &str = "supervisor.sock";
    const AGENT_HOME_DIR: &str = ".afs";
    const HISTORY_DIR: &str = "history";
    const BASELINE_FILE: &str = "baseline.tsv";
    const SNAPSHOT_FILE: &str = "snapshot.tsv";
    const ENTRIES_FILE: &str = "entries.tsv";
    const REGISTRY_FILE: &str = "registry.tsv";
    const PI_RUNTIME_ENV: &str = "AFS_PI_RUNTIME";
    const SETTLE_WINDOW: Duration = Duration::from_millis(150);

    pub fn run_foreground() -> io::Result<()> {
        let home = home()?;
        std::fs::create_dir_all(&home)?;

        let listener = bind_supervisor_socket(&home)?;
        let mut state = SupervisorState::new(home)?;

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
        agent_home: PathBuf,
        process: Child,
        stdin: ChildStdin,
        stdout: io::BufReader<ChildStdout>,
        _monitor: thread::JoinHandle<()>,
    }

    impl SupervisorState {
        fn new(home: PathBuf) -> io::Result<Self> {
            let mut state = Self {
                home,
                agents: Vec::new(),
            };
            state.load_registry()?;
            Ok(state)
        }
    }

    fn handle_client(mut stream: UnixStream, state: &mut SupervisorState) -> io::Result<()> {
        let mut reader = io::BufReader::new(stream.try_clone()?);
        let mut request = String::new();
        if reader.read_line(&mut request)? == 0 {
            return Ok(());
        }

        let request = request.trim_end_matches('\n').trim_end_matches('\r');
        let response = if let Some(payload) = request.strip_prefix("ASK\t") {
            let Some((cwd, prompt)) = payload.split_once('\t') else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "ASK request is missing current directory",
                ));
            };
            state.ask(Path::new(cwd), prompt)
        } else if let Some(path) = request.strip_prefix("INSTALL ") {
            state.install(Path::new(path))
        } else if request == "AGENTS" {
            state.agents()
        } else if let Some(path) = request.strip_prefix("HISTORY ") {
            state.history(Path::new(path))
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
            let history_dir = agent_home.join(HISTORY_DIR);
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

            let baseline_path = history_dir.join(BASELINE_FILE);
            if !baseline_path.exists() {
                let baseline = history_baseline(&managed_dir)?;
                std::fs::write(baseline_path, baseline)?;
            }
            ensure_history_snapshot(&managed_dir, &agent_home)?;

            if !self
                .agents
                .iter()
                .any(|agent| agent.managed_dir == managed_dir)
            {
                self.start_registered_agent(managed_dir.clone(), agent_home, identity.trim())?;
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

        fn history(&self, path: &Path) -> io::Result<String> {
            let requested_path = path.canonicalize()?;
            let Some(agent_index) = self.owning_agent_index(&requested_path) else {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "path is not managed",
                ));
            };

            format_history(&self.agents[agent_index].agent_home)
        }

        fn ask(&mut self, cwd: &Path, prompt: &str) -> io::Result<String> {
            let Some(requested_path) = explicit_prompt_path(cwd, prompt)? else {
                return Ok("ask handling not implemented yet\n".to_string());
            };
            let Some(agent_index) = self.owning_agent_index(&requested_path) else {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "path is not managed: {}. Run afs install {} or afs install a suitable parent directory.",
                        requested_path.display(),
                        install_suggestion_path(&requested_path).display()
                    ),
                ));
            };

            let agent = &mut self.agents[agent_index];
            let answer = agent.ask(prompt, &requested_path)?;
            Ok(format_direct_ask_response(
                &answer,
                &requested_path,
                &agent.identity,
            ))
        }

        fn owning_agent_index(&self, requested_path: &Path) -> Option<usize> {
            self.agents
                .iter()
                .enumerate()
                .filter(|(_index, agent)| {
                    requested_path == agent.managed_dir
                        || requested_path.starts_with(&agent.managed_dir)
                })
                .max_by_key(|(_index, agent)| agent.managed_dir.components().count())
                .map(|(index, _agent)| index)
        }

        fn write_registry(&self) -> io::Result<()> {
            let mut registry = String::from("identity\tmanaged_dir\tagent_home\n");
            for agent in &self.agents {
                registry.push_str(&agent.identity);
                registry.push('\t');
                registry.push_str(&agent.managed_dir.to_string_lossy());
                registry.push('\t');
                registry.push_str(&agent.agent_home.to_string_lossy());
                registry.push('\n');
            }
            std::fs::write(self.home.join(REGISTRY_FILE), registry)
        }

        fn load_registry(&mut self) -> io::Result<()> {
            let registry_path = self.home.join(REGISTRY_FILE);
            if !registry_path.exists() {
                return Ok(());
            }

            let registry = std::fs::read_to_string(registry_path)?;
            for line in registry.lines().skip(1) {
                let fields = line.split('\t').collect::<Vec<_>>();
                if fields.len() != 3 {
                    continue;
                }

                let identity = fields[0].to_string();
                let managed_dir = PathBuf::from(fields[1]);
                let agent_home = PathBuf::from(fields[2]);
                if !managed_dir.is_dir() {
                    continue;
                }

                record_startup_reconciliation(&managed_dir, &agent_home)?;
                self.start_registered_agent(managed_dir, agent_home, &identity)?;
            }

            Ok(())
        }

        fn start_registered_agent(
            &mut self,
            managed_dir: PathBuf,
            agent_home: PathBuf,
            identity: &str,
        ) -> io::Result<()> {
            let mut process = start_directory_agent_process(&managed_dir, &agent_home, identity)?;
            let stdin = process
                .stdin
                .take()
                .ok_or_else(|| io::Error::other("Pi Agent Runtime stdin is unavailable"))?;
            let stdout = process
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("Pi Agent Runtime stdout is unavailable"))?;
            let monitor = start_directory_monitor(managed_dir.clone(), agent_home.clone())?;
            self.agents.push(RegisteredAgent {
                identity: identity.to_string(),
                managed_dir,
                agent_home,
                process,
                stdin,
                stdout: io::BufReader::new(stdout),
                _monitor: monitor,
            });
            Ok(())
        }
    }

    impl RegisteredAgent {
        fn ask(&mut self, prompt: &str, requested_path: &Path) -> io::Result<String> {
            writeln!(self.stdin, "ASK")?;
            writeln!(self.stdin, "{}", requested_path.display())?;
            writeln!(self.stdin, "{prompt}")?;
            self.stdin.flush()?;

            let mut answer = String::new();
            self.stdout.read_line(&mut answer)?;
            if answer.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Pi Agent Runtime closed before answering",
                ));
            }
            Ok(answer)
        }
    }

    fn format_direct_ask_response(
        answer: &str,
        requested_path: &Path,
        agent_identity: &str,
    ) -> String {
        let mut response = String::new();
        response.push_str(answer);
        if !response.ends_with('\n') {
            response.push('\n');
        }
        response.push_str("references:\n");
        response.push_str(&format!("- {}\n", requested_path.display()));
        response.push_str("caveat: local index is warming; answer may be incomplete\n");
        response.push_str(&format!("participating_agents: {agent_identity}\n"));
        response.push_str("changed_files: none\n");
        response
    }

    fn explicit_prompt_path(cwd: &Path, prompt: &str) -> io::Result<Option<PathBuf>> {
        for token in prompt.split_whitespace() {
            let token = token.trim_matches(|character: char| {
                matches!(
                    character,
                    '"' | '\''
                        | '`'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '<'
                        | '>'
                        | ','
                        | ':'
                        | ';'
                        | '!'
                        | '?'
                )
            });
            if token.is_empty() {
                continue;
            }

            let path = Path::new(token);
            let candidate = if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            };
            if candidate.exists() {
                return candidate.canonicalize().map(Some);
            }
        }

        Ok(None)
    }

    fn install_suggestion_path(path: &Path) -> &Path {
        if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        }
    }

    fn start_directory_agent_process(
        managed_dir: &Path,
        agent_home: &Path,
        identity: &str,
    ) -> io::Result<Child> {
        let runtime = pi_runtime_command();
        Command::new(&runtime)
            .arg("agent")
            .arg("--rpc")
            .arg("stdio")
            .arg("--managed-dir")
            .arg(managed_dir)
            .arg("--agent-home")
            .arg(agent_home)
            .arg("--identity")
            .arg(identity)
            .env("AFS_AGENT_ID", identity)
            .env("AFS_AGENT_HOME", agent_home)
            .env("AFS_MANAGED_DIR", managed_dir)
            .env("AFS_AGENT_RPC", "stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        format!(
                            "Pi Agent Runtime command not found: {} (set {PI_RUNTIME_ENV})",
                            runtime.display()
                        ),
                    )
                } else {
                    error
                }
            })
    }

    fn pi_runtime_command() -> PathBuf {
        std::env::var_os(PI_RUNTIME_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("pi"))
    }

    fn new_agent_identity() -> io::Result<String> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        Ok(format!("agent-{}-{nanos}\n", std::process::id()))
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct SnapshotEntry {
        len: u64,
        hash: u64,
    }

    type Snapshot = BTreeMap<String, SnapshotEntry>;

    fn history_lock() -> &'static Mutex<()> {
        static HISTORY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        HISTORY_LOCK.get_or_init(|| Mutex::new(()))
    }

    fn ensure_history_snapshot(managed_dir: &Path, agent_home: &Path) -> io::Result<()> {
        let snapshot_path = agent_home.join(HISTORY_DIR).join(SNAPSHOT_FILE);
        if snapshot_path.exists() {
            return Ok(());
        }

        let snapshot = collect_file_snapshot(managed_dir)?;
        write_snapshot(&snapshot_path, &snapshot)
    }

    fn start_directory_monitor(
        managed_dir: PathBuf,
        agent_home: PathBuf,
    ) -> io::Result<thread::JoinHandle<()>> {
        let (ready_sender, ready_receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            if let Err(error) = monitor_directory(managed_dir, agent_home, ready_sender) {
                eprintln!("directory monitor stopped: {error}");
            }
        });

        match ready_receiver.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => Ok(handle),
            Ok(Err(message)) => Err(io::Error::other(message)),
            Err(error) => Err(io::Error::other(format!(
                "directory monitor did not start: {error}"
            ))),
        }
    }

    fn monitor_directory(
        managed_dir: PathBuf,
        agent_home: PathBuf,
        ready_sender: mpsc::Sender<Result<(), String>>,
    ) -> io::Result<()> {
        let (sender, receiver) = mpsc::channel();
        let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
        })
        .map_err(io::Error::other)?;
        if let Err(error) = watcher
            .watch(&managed_dir, RecursiveMode::Recursive)
            .map_err(io::Error::other)
        {
            let _ = ready_sender.send(Err(error.to_string()));
            return Err(error);
        }
        let _ = ready_sender.send(Ok(()));

        loop {
            match receiver.recv() {
                Ok(Ok(event)) => {
                    if event_affects_managed_subtree(&managed_dir, &event.paths) {
                        wait_for_settled_events(&receiver, &managed_dir);
                        record_external_change(&managed_dir, &agent_home)?;
                    }
                }
                Ok(Err(_error)) => {}
                Err(_closed) => return Ok(()),
            }
        }
    }

    fn wait_for_settled_events(
        receiver: &Receiver<notify::Result<notify::Event>>,
        managed_dir: &Path,
    ) {
        let mut deadline = Instant::now() + SETTLE_WINDOW;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return;
            }

            match receiver.recv_timeout(deadline - now) {
                Ok(Ok(event)) => {
                    if event_affects_managed_subtree(managed_dir, &event.paths) {
                        deadline = Instant::now() + SETTLE_WINDOW;
                    }
                }
                Ok(Err(_error)) => {}
                Err(RecvTimeoutError::Timeout) => return,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn event_affects_managed_subtree(managed_dir: &Path, paths: &[PathBuf]) -> bool {
        paths
            .iter()
            .any(|path| is_managed_content_path(managed_dir, path))
    }

    fn is_managed_content_path(managed_dir: &Path, path: &Path) -> bool {
        let agent_home = managed_dir.join(AGENT_HOME_DIR);
        path != managed_dir && path.starts_with(managed_dir) && !path.starts_with(agent_home)
    }

    fn record_external_change(managed_dir: &Path, agent_home: &Path) -> io::Result<()> {
        record_snapshot_delta(managed_dir, agent_home, "external", "External change")
    }

    fn record_startup_reconciliation(managed_dir: &Path, agent_home: &Path) -> io::Result<()> {
        std::fs::create_dir_all(agent_home.join(HISTORY_DIR))?;
        let snapshot_path = agent_home.join(HISTORY_DIR).join(SNAPSHOT_FILE);
        if !snapshot_path.exists() {
            let snapshot = collect_file_snapshot(managed_dir)?;
            return write_snapshot(&snapshot_path, &snapshot);
        }

        record_snapshot_delta(
            managed_dir,
            agent_home,
            "reconciliation",
            "Startup reconciliation",
        )
    }

    fn record_snapshot_delta(
        managed_dir: &Path,
        agent_home: &Path,
        kind: &str,
        summary_prefix: &str,
    ) -> io::Result<()> {
        let _guard = history_lock()
            .lock()
            .map_err(|_| io::Error::other("history lock poisoned"))?;
        let snapshot_path = agent_home.join(HISTORY_DIR).join(SNAPSHOT_FILE);
        let previous = read_snapshot(&snapshot_path)?;
        let current = collect_file_snapshot(managed_dir)?;
        let changed_files = changed_files(&previous, &current);
        if changed_files.is_empty() {
            return Ok(());
        }

        append_history_entry(agent_home, kind, summary_prefix, &changed_files)?;
        write_snapshot(&snapshot_path, &current)
    }

    fn changed_files(previous: &Snapshot, current: &Snapshot) -> Vec<String> {
        let mut paths = BTreeSet::new();
        paths.extend(previous.keys().cloned());
        paths.extend(current.keys().cloned());
        paths
            .into_iter()
            .filter(|path| previous.get(path) != current.get(path))
            .collect()
    }

    fn append_history_entry(
        agent_home: &Path,
        kind: &str,
        summary_prefix: &str,
        files: &[String],
    ) -> io::Result<()> {
        let history_dir = agent_home.join(HISTORY_DIR);
        std::fs::create_dir_all(&history_dir)?;
        let entries_path = history_dir.join(ENTRIES_FILE);
        let mut entries = OpenOptions::new()
            .create(true)
            .append(true)
            .open(entries_path)?;
        let timestamp = unix_timestamp();
        let summary = history_summary(summary_prefix, files);
        writeln!(
            entries,
            "{}\t{}\t{}\t{}\t{}\tyes\t{}",
            history_entry_id(),
            timestamp,
            kind,
            sanitize_field(&summary),
            files.len(),
            sanitize_field(&files.join(", "))
        )
    }

    fn history_summary(prefix: &str, files: &[String]) -> String {
        match files {
            [] => format!("{prefix}: no files changed"),
            [file] => format!("{prefix}: {file}"),
            _ => format!("{prefix}: {} files changed", files.len()),
        }
    }

    fn format_history(agent_home: &Path) -> io::Result<String> {
        let _guard = history_lock()
            .lock()
            .map_err(|_| io::Error::other("history lock poisoned"))?;
        let entries_path = agent_home.join(HISTORY_DIR).join(ENTRIES_FILE);
        if !entries_path.exists() {
            return Ok("no history entries\n".to_string());
        }

        let mut entries = std::fs::read_to_string(entries_path)?
            .lines()
            .filter_map(parse_history_entry)
            .collect::<Vec<_>>();
        entries.reverse();

        if entries.is_empty() {
            return Ok("no history entries\n".to_string());
        }

        let mut output = String::new();
        for entry in entries {
            output.push_str(&format!(
                "timestamp={} type={} summary={} files={} undoable={}\n",
                entry.timestamp, entry.kind, entry.summary, entry.file_count, entry.undoable
            ));
        }
        Ok(output)
    }

    struct HistoryEntry {
        timestamp: String,
        kind: String,
        summary: String,
        file_count: usize,
        undoable: String,
    }

    fn parse_history_entry(line: &str) -> Option<HistoryEntry> {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 7 {
            return None;
        }

        Some(HistoryEntry {
            timestamp: fields[1].to_string(),
            kind: fields[2].to_string(),
            summary: fields[3].to_string(),
            file_count: fields[4].parse().ok()?,
            undoable: fields[5].to_string(),
        })
    }

    fn unix_timestamp() -> String {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs().to_string())
            .unwrap_or_else(|_| "0".to_string())
    }

    fn history_entry_id() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        format!("history-{}-{nanos}", std::process::id())
    }

    fn sanitize_field(value: &str) -> String {
        value.replace(['\t', '\n', '\r'], " ")
    }

    fn collect_file_snapshot(managed_dir: &Path) -> io::Result<Snapshot> {
        let mut snapshot = BTreeMap::new();
        collect_snapshot_files(managed_dir, managed_dir, &mut snapshot)?;
        Ok(snapshot)
    }

    fn collect_snapshot_files(
        managed_dir: &Path,
        current_dir: &Path,
        snapshot: &mut Snapshot,
    ) -> io::Result<()> {
        for entry in std::fs::read_dir(current_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path == managed_dir.join(AGENT_HOME_DIR) {
                continue;
            }

            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                collect_snapshot_files(managed_dir, &path, snapshot)?;
            } else if metadata.is_file() {
                let relative_path = relative_managed_path(managed_dir, &path)?;
                snapshot.insert(
                    relative_path,
                    SnapshotEntry {
                        len: metadata.len(),
                        hash: file_hash(&path)?,
                    },
                );
            }
        }
        Ok(())
    }

    fn relative_managed_path(managed_dir: &Path, path: &Path) -> io::Result<String> {
        Ok(path
            .strip_prefix(managed_dir)
            .map_err(io::Error::other)?
            .to_string_lossy()
            .replace('\\', "/"))
    }

    fn file_hash(path: &Path) -> io::Result<u64> {
        const FNV_OFFSET: u64 = 0xcbf29ce484222325;
        const FNV_PRIME: u64 = 0x100000001b3;

        let mut file = std::fs::File::open(path)?;
        let mut hash = FNV_OFFSET;
        let mut buffer = [0; 8192];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                return Ok(hash);
            }

            for byte in &buffer[..read] {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(FNV_PRIME);
            }
        }
    }

    fn read_snapshot(snapshot_path: &Path) -> io::Result<Snapshot> {
        if !snapshot_path.exists() {
            return Ok(BTreeMap::new());
        }

        let mut snapshot = BTreeMap::new();
        for line in std::fs::read_to_string(snapshot_path)?.lines().skip(1) {
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.len() != 3 {
                continue;
            }

            let Some(len) = fields[1].parse().ok() else {
                continue;
            };
            let Some(hash) = fields[2].parse().ok() else {
                continue;
            };

            snapshot.insert(fields[0].to_string(), SnapshotEntry { len, hash });
        }

        Ok(snapshot)
    }

    fn write_snapshot(snapshot_path: &Path, snapshot: &Snapshot) -> io::Result<()> {
        if let Some(parent) = snapshot_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut output = String::from("path\tlen\thash\n");
        for (path, entry) in snapshot {
            output.push_str(path);
            output.push('\t');
            output.push_str(&entry.len.to_string());
            output.push('\t');
            output.push_str(&entry.hash.to_string());
            output.push('\n');
        }

        std::fs::write(snapshot_path, output)
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
        let cwd = std::env::current_dir().map_err(Error::Io)?;
        send_request(&format!("ASK\t{}\t{prompt}", cwd.display()))
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

    pub fn history(path: &Path) -> Result<String, Error> {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map_err(Error::Io)?.join(path)
        };
        send_request(&format!("HISTORY {}", path.display()))
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
