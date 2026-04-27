pub mod supervisor {
    use notify::{RecommendedWatcher, RecursiveMode, Watcher};
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::{self, BufRead, Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread;
    use std::time::{Duration, Instant};
    use std::time::{SystemTime, UNIX_EPOCH};

    const SOCKET_FILE: &str = "supervisor.sock";
    const AGENT_HOME_DIR: &str = ".afs";
    const HISTORY_DIR: &str = "history";
    const HISTORY_REPO_DIR: &str = "repo";
    const REGISTRY_FILE: &str = "registry.tsv";
    const ARCHIVES_DIR: &str = "archives";
    const IGNORE_FILE: &str = "ignore";
    const PI_RUNTIME_ENV: &str = "AFS_PI_RUNTIME";
    const BROADCAST_REPLY_TIMEOUT_ENV: &str = "AFS_BROADCAST_REPLY_TIMEOUT_MS";
    const DEFAULT_BROADCAST_REPLY_TIMEOUT: Duration = Duration::from_secs(2);
    const SETTLE_WINDOW: Duration = Duration::from_millis(150);
    const MAX_TEXT_BYTES: usize = 256 * 1024;
    const INDEX_FINGERPRINT_BYTES: usize = 256;
    const INDEX_SETTLED_PATH_CAP: usize = 1024;
    const INDEX_WARM_DELAY_ENV: &str = "AFS_INDEX_WARM_DELAY_MS";

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
        stdout: ChildStdout,
        queued_tasks: usize,
        monitor: Option<DirectoryMonitor>,
        index: Arc<Mutex<DirectoryIndex>>,
    }

    struct DirectoryMonitor {
        stop: mpsc::Sender<()>,
        handle: thread::JoinHandle<()>,
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
        } else if let Some(payload) = request.strip_prefix("REMOVE\t") {
            let Some((path, flag)) = payload.split_once('\t') else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "REMOVE request is missing discard-history flag",
                ));
            };
            let discard_history = match flag {
                "discard" => true,
                "keep" => false,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "REMOVE request has unknown discard-history flag",
                    ));
                }
            };
            state.remove(Path::new(path), discard_history)
        } else if request == "AGENTS" {
            state.agents()
        } else if let Some(path) = request.strip_prefix("HISTORY ") {
            state.history(Path::new(path))
        } else if let Some(payload) = request.strip_prefix("UNDO\t") {
            let mut fields = payload.splitn(3, '\t');
            let Some(path) = fields.next() else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "UNDO request is missing path",
                ));
            };
            let Some(history_entry) = fields.next() else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "UNDO request is missing history entry",
                ));
            };
            let Some(confirmed) = fields.next() else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "UNDO request is missing confirmation mode",
                ));
            };
            state.undo(Path::new(path), history_entry, confirmed == "yes")
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

            if crate::config::Config::load(&self.home)?.is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "authentication required: run `afs login --provider claude|openai` before installing a managed directory",
                ));
            }

            let parent_agent_index = self.parent_agent_index(&managed_dir);

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

            ensure_ignore_file_seeded(&managed_dir, &agent_home)?;

            ensure_history_baseline_commit(&managed_dir, &agent_home)?;

            if !self
                .agents
                .iter()
                .any(|agent| agent.managed_dir == managed_dir)
            {
                self.start_registered_agent(managed_dir.clone(), agent_home, identity.trim())?;
                self.write_registry()?;
            }

            if !was_already_managed && let Some(parent_agent_index) = parent_agent_index {
                let parent = &self.agents[parent_agent_index];
                let affected = relative_managed_path(&parent.managed_dir, &managed_dir)?;
                record_ownership_event(
                    &parent.managed_dir,
                    &parent.agent_home,
                    &format!("Ownership split: {affected}"),
                )?;
                spawn_warm_task(
                    parent.managed_dir.clone(),
                    parent.agent_home.clone(),
                    parent.index.clone(),
                );
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

        fn remove(&mut self, path: &Path, discard_history: bool) -> io::Result<String> {
            let managed_dir = self.resolve_managed_dir_for_remove(path)?;
            let Some(agent_index) = self
                .agents
                .iter()
                .position(|agent| agent.managed_dir == managed_dir)
            else {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "managed directory is not installed",
                ));
            };

            if self.home.starts_with(&managed_dir) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "cannot remove a managed directory that contains the AFS supervisor home",
                ));
            }

            match self.parent_agent_index(&managed_dir) {
                Some(parent_agent_index) => self.remove_nested(
                    agent_index,
                    parent_agent_index,
                    managed_dir,
                    discard_history,
                ),
                None => self.remove_top_level(agent_index, managed_dir, discard_history),
            }
        }

        fn remove_nested(
            &mut self,
            agent_index: usize,
            parent_agent_index: usize,
            managed_dir: PathBuf,
            discard_history: bool,
        ) -> io::Result<String> {
            let parent_managed_dir = self.agents[parent_agent_index].managed_dir.clone();
            let parent_agent_home = self.agents[parent_agent_index].agent_home.clone();
            self.agents[parent_agent_index].stop_monitor()?;

            let removed_agent = self.agents.remove(agent_index);
            let removed_identity = removed_agent.identity.clone();
            let removed_agent_home = removed_agent.agent_home.clone();

            let outcome_result: io::Result<(PathBuf, RemoveOutcome)> = (|| {
                stop_and_persist_registry(removed_agent, self)?;

                let child_origin = relative_managed_path(&parent_managed_dir, &managed_dir)?;

                if !removed_agent_home.exists() {
                    record_ownership_event(
                        &parent_managed_dir,
                        &parent_agent_home,
                        &format!("Ownership merge: {child_origin} (home missing)"),
                    )?;
                    return Ok((removed_agent_home.clone(), RemoveOutcome::Missing));
                }

                if discard_history {
                    std::fs::remove_dir_all(&removed_agent_home)?;
                    record_ownership_event(
                        &parent_managed_dir,
                        &parent_agent_home,
                        &format!("Ownership merge: {child_origin} (history discarded)"),
                    )?;
                    return Ok((removed_agent_home.clone(), RemoveOutcome::Discarded));
                }

                let archived_agent_home = archive_agent_home(
                    &removed_agent_home,
                    &parent_agent_home.join(ARCHIVES_DIR),
                    &archive_safe_name(removed_identity.trim()),
                )?;
                record_ownership_event(
                    &parent_managed_dir,
                    &parent_agent_home,
                    &format!("Ownership merge: {child_origin}"),
                )?;
                merge_archived_child_history(
                    &archived_agent_home,
                    &parent_managed_dir,
                    &parent_agent_home,
                    &child_origin,
                )?;
                Ok((archived_agent_home, RemoveOutcome::Archived))
            })();

            let parent_restart_result = self
                .agents
                .iter_mut()
                .find(|agent| agent.managed_dir == parent_managed_dir)
                .map(RegisteredAgent::start_monitor)
                .transpose();

            let (home_path, outcome) = outcome_result?;
            if parent_restart_result?.is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "parent managed directory is no longer registered",
                ));
            }

            if let Some(parent) = self
                .agents
                .iter()
                .find(|agent| agent.managed_dir == parent_managed_dir)
            {
                spawn_warm_task(
                    parent.managed_dir.clone(),
                    parent.agent_home.clone(),
                    parent.index.clone(),
                );
            }

            Ok(format_remove_response(
                &managed_dir,
                &removed_identity,
                &home_path,
                outcome,
            ))
        }

        fn remove_top_level(
            &mut self,
            agent_index: usize,
            managed_dir: PathBuf,
            discard_history: bool,
        ) -> io::Result<String> {
            let removed_agent = self.agents.remove(agent_index);
            let removed_identity = removed_agent.identity.clone();
            let removed_agent_home = removed_agent.agent_home.clone();
            stop_and_persist_registry(removed_agent, self)?;

            let (home_path, outcome) = if !removed_agent_home.exists() {
                (removed_agent_home, RemoveOutcome::Missing)
            } else if discard_history {
                std::fs::remove_dir_all(&removed_agent_home)?;
                (removed_agent_home, RemoveOutcome::Discarded)
            } else {
                let last_component = managed_dir
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "managed".to_string());
                let archive_name =
                    archive_safe_name(&format!("{last_component}-{}", removed_identity.trim()));
                let archived_path = archive_agent_home(
                    &removed_agent_home,
                    &supervisor_archive_root(&self.home),
                    &archive_name,
                )?;
                (archived_path, RemoveOutcome::Archived)
            };

            Ok(format_remove_response(
                &managed_dir,
                &removed_identity,
                &home_path,
                outcome,
            ))
        }

        fn resolve_managed_dir_for_remove(&self, path: &Path) -> io::Result<PathBuf> {
            let absolute = if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir()?.join(path)
            };
            match absolute.canonicalize() {
                Ok(canonical) => Ok(canonical),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if let (Some(parent), Some(leaf)) = (absolute.parent(), absolute.file_name())
                        && let Ok(canonical_parent) = parent.canonicalize()
                    {
                        return Ok(canonical_parent.join(leaf));
                    }
                    Err(error)
                }
                Err(error) => Err(error),
            }
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
                let index_token = format_index_token(&agent.index);
                status.push_str(&format!(
                    "{}\tagent={}\truntime=pi-rpc-stdio\thealth={health}\tindex={index_token}\treconciliation=idle\tqueue={}\n",
                    agent.managed_dir.display(),
                    agent.identity,
                    agent.queued_tasks
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

        fn undo(
            &mut self,
            path: &Path,
            history_entry: &str,
            confirmed: bool,
        ) -> io::Result<String> {
            let requested_path = path.canonicalize()?;
            let Some(agent_index) = self.owning_agent_index(&requested_path) else {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "path is not managed",
                ));
            };

            let agent = &self.agents[agent_index];
            undo_history_entry(
                &agent.managed_dir,
                &agent.agent_home,
                history_entry,
                confirmed,
            )
        }

        fn ask(&mut self, cwd: &Path, prompt: &str) -> io::Result<String> {
            let Some(requested_path) = explicit_prompt_path(cwd, prompt)? else {
                return self.broadcast_ask(prompt);
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

            let answer = {
                let agent = &mut self.agents[agent_index];
                agent.ask(prompt, &requested_path)?
            };

            if let Some(request) = parse_delegate_request(&answer)? {
                let requests = self.collect_delegate_requests(agent_index, request)?;
                let reply_target = requests
                    .first()
                    .map(|request| request.reply_target)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "delegation request is missing")
                    })?;
                if requests
                    .iter()
                    .any(|request| request.reply_target != reply_target)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "delegated task batch must use one reply target",
                    ));
                }

                if reply_target == ReplyTarget::Supervisor {
                    let outcome =
                        self.perform_delegated_supervisor_tasks(agent_index, &requests)?;
                    let index_state = snapshot_index_state(&self.agents[agent_index].index);
                    return Ok(format_delegated_supervisor_response(
                        &outcome,
                        &requested_path,
                        &self.agents[agent_index].identity,
                        &index_state,
                    ));
                }

                if requests.len() != 1 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "delegator reply target supports one delegated task per request",
                    ));
                }

                let request = &requests[0];
                let reply = self.perform_delegated_task(agent_index, request)?;
                let delegator_answer = {
                    let agent = &mut self.agents[agent_index];
                    agent.deliver_delegated_reply(&reply)?
                };
                let index_state = snapshot_index_state(&self.agents[agent_index].index);
                return Ok(format_delegated_delegator_response(
                    &delegator_answer,
                    &reply,
                    &requested_path,
                    &self.agents[agent_index].identity,
                    &index_state,
                ));
            }

            let index_state = snapshot_index_state(&self.agents[agent_index].index);
            Ok(format_direct_ask_response(
                &answer,
                &requested_path,
                &self.agents[agent_index].identity,
                &index_state,
            ))
        }

        fn collect_delegate_requests(
            &mut self,
            agent_index: usize,
            first_request: DelegateRequest,
        ) -> io::Result<Vec<DelegateRequest>> {
            let mut requests = vec![first_request];
            let agent = &mut self.agents[agent_index];
            set_nonblocking(&agent.stdout, true)?;
            let mut buffer = Vec::new();
            let mut deadline = Instant::now() + Duration::from_millis(50);

            loop {
                let line = read_nonblocking_line(&mut agent.stdout, &mut buffer)?;
                match line {
                    Some(line) => {
                        let Some(request) = parse_delegate_request(&line)? else {
                            break;
                        };
                        requests.push(request);
                        deadline = Instant::now() + Duration::from_millis(50);
                    }
                    None if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    None => break,
                }
            }

            set_nonblocking(&agent.stdout, false)?;
            Ok(requests)
        }

        fn perform_delegated_task(
            &mut self,
            requester_index: usize,
            request: &DelegateRequest,
        ) -> io::Result<TaskReply> {
            let requester_identity = self.agents[requester_index].identity.clone();
            let Some(target_index) = self.delegated_target_agent_index(&request.target) else {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("delegated target is not managed: {}", request.target),
                ));
            };

            self.perform_delegated_task_at(requester_identity, target_index, request)
        }

        fn perform_delegated_supervisor_tasks(
            &mut self,
            requester_index: usize,
            requests: &[DelegateRequest],
        ) -> io::Result<DelegationOutcome> {
            let requester_identity = self.agents[requester_index].identity.clone();
            let mut target_indexes = Vec::new();
            let mut target_counts = BTreeMap::<usize, usize>::new();
            for request in requests {
                let Some(target_index) = self.delegated_target_agent_index(&request.target) else {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("delegated target is not managed: {}", request.target),
                    ));
                };
                target_indexes.push(target_index);
                *target_counts.entry(target_index).or_default() += 1;
            }

            let mut progress = Vec::new();
            for (target_index, count) in &target_counts {
                if *count > 1 {
                    let queued = count - 1;
                    self.agents[*target_index].queued_tasks = queued;
                    progress.push(format!(
                        "progress: queued task agent={} queue={queued}",
                        self.agents[*target_index].identity
                    ));
                }
            }

            let mut replies = Vec::new();
            for (request, target_index) in requests.iter().zip(target_indexes) {
                let reply = self.perform_delegated_task_at(
                    requester_identity.clone(),
                    target_index,
                    request,
                )?;
                replies.push(reply);
                if self.agents[target_index].queued_tasks > 0 {
                    self.agents[target_index].queued_tasks -= 1;
                    progress.push(format!(
                        "progress: started task agent={} queue={}",
                        self.agents[target_index].identity, self.agents[target_index].queued_tasks
                    ));
                }
            }

            Ok(DelegationOutcome { replies, progress })
        }

        fn perform_delegated_task_at(
            &mut self,
            requester_identity: String,
            target_index: usize,
            request: &DelegateRequest,
        ) -> io::Result<TaskReply> {
            let target = &mut self.agents[target_index];
            let raw_reply = target.task(
                &requester_identity,
                request.reply_target.as_protocol_field(),
                &request.prompt,
            )?;
            let mut reply = parse_task_reply(&raw_reply, target);
            if let Some(change) = record_agent_change(&target.managed_dir, &target.agent_home)? {
                reply.changed_files = change.files;
                reply.history_entries = vec![change.history_entry];
            }
            Ok(reply)
        }

        fn perform_delegated_task_with_deadline(
            &mut self,
            requester_index: usize,
            request: &DelegateRequest,
            deadline: Instant,
        ) -> io::Result<TaskReply> {
            let requester_identity = self.agents[requester_index].identity.clone();
            let Some(target_index) = self.delegated_target_agent_index(&request.target) else {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("delegated target is not managed: {}", request.target),
                ));
            };
            let target = &mut self.agents[target_index];
            let raw_reply = target.task_with_deadline(
                &requester_identity,
                request.reply_target.as_protocol_field(),
                &request.prompt,
                deadline,
            )?;
            let mut reply = parse_task_reply(&raw_reply, target);
            if let Some(change) = record_agent_change(&target.managed_dir, &target.agent_home)? {
                reply.changed_files = change.files;
                reply.history_entries = vec![change.history_entry];
            }
            Ok(reply)
        }

        fn delegated_target_agent_index(&self, target: &str) -> Option<usize> {
            if let Some(index) = self
                .agents
                .iter()
                .position(|agent| agent.identity == target.trim())
            {
                return Some(index);
            }

            let target_path = Path::new(target);
            let target_path = if target_path.is_absolute() {
                target_path.to_path_buf()
            } else {
                return None;
            };
            let target_path = target_path.canonicalize().ok()?;
            self.owning_agent_index(&target_path)
        }

        fn broadcast_ask(&mut self, prompt: &str) -> io::Result<String> {
            let timeout = broadcast_reply_timeout();
            let empty_outcome = CollaborationOutcome {
                answers: Vec::new(),
                progress: Vec::new(),
            };
            if self.agents.is_empty() {
                return Ok(format_broadcast_ask_response(&[], &empty_outcome, timeout));
            }

            for agent in &mut self.agents {
                agent.send_broadcast(prompt)?;
                set_nonblocking(&agent.stdout, true)?;
            }

            let deadline = Instant::now() + timeout;
            let mut pending = (0..self.agents.len()).collect::<BTreeSet<_>>();
            let mut buffers = vec![Vec::new(); self.agents.len()];
            let mut indexed_replies: Vec<(usize, BroadcastReply)> = Vec::new();

            while !pending.is_empty() && Instant::now() < deadline {
                let mut made_progress = false;
                for index in pending.iter().copied().collect::<Vec<_>>() {
                    let line = {
                        let agent = &mut self.agents[index];
                        read_nonblocking_line(&mut agent.stdout, &mut buffers[index])?
                    };

                    if let Some(line) = line {
                        pending.remove(&index);
                        if let Some(reply) = parse_broadcast_reply(&line, &self.agents[index]) {
                            indexed_replies.push((index, reply));
                        }
                        made_progress = true;
                    }
                }

                if !made_progress {
                    thread::sleep(Duration::from_millis(10));
                }
            }

            for agent in &self.agents {
                set_nonblocking(&agent.stdout, false)?;
            }

            let mut outcome = CollaborationOutcome {
                answers: Vec::new(),
                progress: Vec::new(),
            };

            if indexed_replies.len() >= 2 {
                let peers: Vec<(String, PathBuf)> = indexed_replies
                    .iter()
                    .map(|(_, reply)| (reply.agent_identity.clone(), reply.managed_dir.clone()))
                    .collect();
                let agent_indices: Vec<usize> =
                    indexed_replies.iter().map(|(index, _)| *index).collect();
                for agent_index in agent_indices {
                    let self_identity = self.agents[agent_index].identity.clone();
                    let peers_excluding_self: Vec<(String, PathBuf)> = peers
                        .iter()
                        .filter(|(identity, _)| identity != &self_identity)
                        .cloned()
                        .collect();
                    self.agents[agent_index].send_collaborate(prompt, &peers_excluding_self)?;
                    self.run_collaboration_turn(agent_index, &mut outcome)?;
                }
            }

            let replies: Vec<BroadcastReply> = indexed_replies
                .into_iter()
                .map(|(_, reply)| reply)
                .collect();
            Ok(format_broadcast_ask_response(&replies, &outcome, timeout))
        }

        fn run_collaboration_turn(
            &mut self,
            agent_index: usize,
            outcome: &mut CollaborationOutcome,
        ) -> io::Result<()> {
            set_nonblocking(&self.agents[agent_index].stdout, true)?;
            let mut buffer: Vec<u8> = Vec::new();
            let mut deadline = Instant::now() + broadcast_reply_timeout();
            let mut pending_line: Option<String> = None;

            loop {
                let line = if let Some(line) = pending_line.take() {
                    Some(line)
                } else {
                    read_nonblocking_line(&mut self.agents[agent_index].stdout, &mut buffer)?
                };

                if let Some(line) = line {
                    if let Some(req) = parse_delegate_request(&line)? {
                        set_nonblocking(&self.agents[agent_index].stdout, false)?;
                        let call_deadline = Instant::now() + broadcast_reply_timeout();
                        let reply = match self.perform_delegated_task_with_deadline(
                            agent_index,
                            &req,
                            call_deadline,
                        ) {
                            Ok(reply) => reply,
                            Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                                let identity = self.agents[agent_index].identity.clone();
                                outcome.progress.push(format!(
                                    "progress: collaboration delegation timeout agent={identity} target={}",
                                    req.target
                                ));
                                return Ok(());
                            }
                            Err(error) => return Err(error),
                        };
                        let next_line = if req.reply_target == ReplyTarget::Delegator {
                            let next_deadline = Instant::now() + broadcast_reply_timeout();
                            match self.agents[agent_index]
                                .deliver_delegated_reply_with_deadline(&reply, next_deadline)
                            {
                                Ok(line) => Some(line),
                                Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                                    let identity = self.agents[agent_index].identity.clone();
                                    outcome.answers.push(reply);
                                    outcome.progress.push(format!(
                                        "progress: collaboration timeout agent={identity}"
                                    ));
                                    return Ok(());
                                }
                                Err(error) => return Err(error),
                            }
                        } else {
                            None
                        };
                        outcome.answers.push(reply);
                        deadline = Instant::now() + broadcast_reply_timeout();
                        if let Some(next) = next_line {
                            pending_line = Some(next);
                        } else {
                            set_nonblocking(&self.agents[agent_index].stdout, true)?;
                        }
                        continue;
                    }

                    if line.starts_with("COLLABORATE_REPLY\t") {
                        let parsed = parse_collaborate_reply(&line, &self.agents[agent_index]);
                        outcome.answers.push(parsed);
                        set_nonblocking(&self.agents[agent_index].stdout, false)?;
                        return Ok(());
                    }

                    let identity = self.agents[agent_index].identity.clone();
                    outcome.answers.push(TaskReply {
                        agent_identity: identity,
                        answer: line.trim_end_matches(['\n', '\r']).to_string(),
                        changed_files: Vec::new(),
                        history_entries: Vec::new(),
                    });
                    set_nonblocking(&self.agents[agent_index].stdout, false)?;
                    return Ok(());
                }

                if Instant::now() >= deadline {
                    let identity = self.agents[agent_index].identity.clone();
                    outcome
                        .progress
                        .push(format!("progress: collaboration timeout agent={identity}"));
                    set_nonblocking(&self.agents[agent_index].stdout, false)?;
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(10));
            }
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

        fn parent_agent_index(&self, managed_dir: &Path) -> Option<usize> {
            self.agents
                .iter()
                .enumerate()
                .filter(|(_index, agent)| {
                    managed_dir != agent.managed_dir && managed_dir.starts_with(&agent.managed_dir)
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

            let registry = std::fs::read_to_string(&registry_path)?;
            let mut rebuilt = String::from("identity\tmanaged_dir\tagent_home\n");
            let mut registry_changed = false;
            for line in registry.lines().skip(1) {
                if line.is_empty() {
                    continue;
                }
                let fields = line.split('\t').collect::<Vec<_>>();
                if fields.len() != 3 {
                    // Preserve malformed rows verbatim so we don't silently
                    // delete unrecognized state when rewriting the registry.
                    rebuilt.push_str(line);
                    rebuilt.push('\n');
                    continue;
                }

                let identity = fields[0].to_string();
                let mut managed_dir = PathBuf::from(fields[1]);
                let mut agent_home = PathBuf::from(fields[2]);

                if !managed_dir.is_dir() {
                    match rediscover_managed_dir(&managed_dir, identity.trim())? {
                        Some(new_managed_dir) => {
                            managed_dir = new_managed_dir;
                            agent_home = managed_dir.join(AGENT_HOME_DIR);
                            registry_changed = true;
                        }
                        None => {
                            // Keep the unresolved row so it can recover later
                            // (for example, a managed directory on a drive
                            // that is currently unmounted).
                            rebuilt.push_str(line);
                            rebuilt.push('\n');
                            continue;
                        }
                    }
                }

                record_startup_reconciliation(&managed_dir, &agent_home)?;
                rebuilt.push_str(&identity);
                rebuilt.push('\t');
                rebuilt.push_str(&managed_dir.to_string_lossy());
                rebuilt.push('\t');
                rebuilt.push_str(&agent_home.to_string_lossy());
                rebuilt.push('\n');
                self.start_registered_agent(managed_dir, agent_home, &identity)?;
            }

            if registry_changed {
                std::fs::write(&registry_path, rebuilt)?;
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
                .ok_or_else(|| io::Error::other("AFS agent runtime stdin is unavailable"))?;
            let stdout = process
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("AFS agent runtime stdout is unavailable"))?;
            let index = Arc::new(Mutex::new(DirectoryIndex::new()));
            let monitor =
                start_directory_monitor(managed_dir.clone(), agent_home.clone(), index.clone())?;
            spawn_warm_task(managed_dir.clone(), agent_home.clone(), index.clone());
            self.agents.push(RegisteredAgent {
                identity: identity.to_string(),
                managed_dir,
                agent_home,
                process,
                stdin,
                stdout,
                queued_tasks: 0,
                monitor: Some(monitor),
                index,
            });
            Ok(())
        }
    }

    impl RegisteredAgent {
        fn stop(mut self) -> io::Result<()> {
            self.stop_monitor()?;
            if self.process.try_wait()?.is_none() {
                self.process.kill()?;
            }
            let _ = self.process.wait()?;
            Ok(())
        }

        fn stop_monitor(&mut self) -> io::Result<()> {
            if let Ok(guard) = self.index.lock() {
                guard.request_stop();
            }
            if let Some(monitor) = self.monitor.take() {
                monitor.stop()?;
            }
            Ok(())
        }

        fn start_monitor(&mut self) -> io::Result<()> {
            if self.monitor.is_none() {
                self.monitor = Some(start_directory_monitor(
                    self.managed_dir.clone(),
                    self.agent_home.clone(),
                    self.index.clone(),
                )?);
            }
            Ok(())
        }

        fn ask(&mut self, prompt: &str, requested_path: &Path) -> io::Result<String> {
            writeln!(self.stdin, "ASK")?;
            writeln!(self.stdin, "{}", requested_path.display())?;
            writeln!(self.stdin, "{prompt}")?;
            self.stdin.flush()?;

            read_blocking_line(&mut self.stdout)
        }

        fn send_broadcast(&mut self, prompt: &str) -> io::Result<()> {
            writeln!(self.stdin, "BROADCAST")?;
            writeln!(self.stdin, "{prompt}")?;
            self.stdin.flush()
        }

        fn send_collaborate(
            &mut self,
            prompt: &str,
            peers: &[(String, PathBuf)],
        ) -> io::Result<()> {
            writeln!(self.stdin, "COLLABORATE")?;
            writeln!(self.stdin, "{}", peers.len())?;
            for (identity, managed_dir) in peers {
                writeln!(self.stdin, "{}\t{}", identity, managed_dir.display())?;
            }
            writeln!(self.stdin, "{prompt}")?;
            self.stdin.flush()
        }

        fn task(
            &mut self,
            requester_identity: &str,
            reply_target: &str,
            prompt: &str,
        ) -> io::Result<String> {
            writeln!(self.stdin, "TASK")?;
            writeln!(self.stdin, "{requester_identity}")?;
            writeln!(self.stdin, "{reply_target}")?;
            writeln!(self.stdin, "{prompt}")?;
            self.stdin.flush()?;

            read_blocking_line(&mut self.stdout)
        }

        fn task_with_deadline(
            &mut self,
            requester_identity: &str,
            reply_target: &str,
            prompt: &str,
            deadline: Instant,
        ) -> io::Result<String> {
            writeln!(self.stdin, "TASK")?;
            writeln!(self.stdin, "{requester_identity}")?;
            writeln!(self.stdin, "{reply_target}")?;
            writeln!(self.stdin, "{prompt}")?;
            self.stdin.flush()?;

            read_line_with_deadline(&mut self.stdout, deadline)
        }

        fn deliver_delegated_reply(&mut self, reply: &TaskReply) -> io::Result<String> {
            writeln!(self.stdin, "DELEGATED_REPLY")?;
            writeln!(self.stdin, "{}", reply.agent_identity)?;
            writeln!(self.stdin, "{}", reply.answer)?;
            writeln!(self.stdin, "{}", report_list(&reply.changed_files))?;
            writeln!(self.stdin, "{}", report_list(&reply.history_entries))?;
            self.stdin.flush()?;

            read_blocking_line(&mut self.stdout)
        }

        fn deliver_delegated_reply_with_deadline(
            &mut self,
            reply: &TaskReply,
            deadline: Instant,
        ) -> io::Result<String> {
            writeln!(self.stdin, "DELEGATED_REPLY")?;
            writeln!(self.stdin, "{}", reply.agent_identity)?;
            writeln!(self.stdin, "{}", reply.answer)?;
            writeln!(self.stdin, "{}", report_list(&reply.changed_files))?;
            writeln!(self.stdin, "{}", report_list(&reply.history_entries))?;
            self.stdin.flush()?;

            read_line_with_deadline(&mut self.stdout, deadline)
        }
    }

    impl DirectoryMonitor {
        fn stop(self) -> io::Result<()> {
            let _ = self.stop.send(());
            self.handle
                .join()
                .map_err(|_| io::Error::other("directory monitor thread panicked"))
        }
    }

    struct BroadcastReply {
        agent_identity: String,
        managed_dir: PathBuf,
        relevance: String,
        reason: String,
        answer: String,
        file_references: Vec<String>,
    }

    struct DelegateRequest {
        target: String,
        reply_target: ReplyTarget,
        prompt: String,
    }

    #[derive(Clone, Copy, Eq, PartialEq)]
    enum ReplyTarget {
        Delegator,
        Supervisor,
    }

    impl ReplyTarget {
        fn as_protocol_field(self) -> &'static str {
            match self {
                Self::Delegator => "delegator",
                Self::Supervisor => "supervisor",
            }
        }
    }

    struct TaskReply {
        agent_identity: String,
        answer: String,
        changed_files: Vec<String>,
        history_entries: Vec<String>,
    }

    struct DelegationOutcome {
        replies: Vec<TaskReply>,
        progress: Vec<String>,
    }

    struct CollaborationOutcome {
        answers: Vec<TaskReply>,
        progress: Vec<String>,
    }

    struct RecordedChange {
        history_entry: String,
        files: Vec<String>,
    }

    #[derive(Clone, Debug)]
    enum IndexState {
        Warming {
            scanned: usize,
            total: Option<usize>,
        },
        Ready {
            files: usize,
            failed: usize,
        },
    }

    struct IndexEntry {
        // Length, fingerprint and extracted_text are persisted per file so
        // future broadcast collaboration (issue #17) can consume the local
        // index without re-reading content. They are intentionally unread in
        // this PR.
        #[allow(dead_code)]
        length: u64,
        #[allow(dead_code)]
        fingerprint: Vec<u8>,
        #[allow(dead_code)]
        extracted_text: Option<String>,
    }

    enum ReadOutcome {
        Indexed(IndexEntry),
        Skipped,
        Failed,
    }

    enum PathAction {
        Clear(String),
        Insert(String, IndexEntry),
        MarkFailed(String),
    }

    struct DirectoryIndex {
        state: IndexState,
        entries: BTreeMap<String, IndexEntry>,
        failed_paths: BTreeSet<String>,
        stop: AtomicBool,
        // Bumped at the start of every full warm scan. A scan stamps its own
        // generation and bails (without writing) at every checkpoint if a
        // newer scan has appeared, so concurrent rescans cannot let an older
        // scan overwrite a newer one's results.
        scan_generation: u64,
    }

    impl DirectoryIndex {
        fn new() -> Self {
            Self {
                state: IndexState::Warming {
                    scanned: 0,
                    total: None,
                },
                entries: BTreeMap::new(),
                failed_paths: BTreeSet::new(),
                stop: AtomicBool::new(false),
                scan_generation: 0,
            }
        }

        fn snapshot_state(&self) -> IndexState {
            self.state.clone()
        }

        fn request_stop(&self) {
            self.stop.store(true, Ordering::Relaxed);
        }

        fn apply_path_updates(&mut self, actions: Vec<PathAction>) {
            for action in actions {
                match action {
                    PathAction::Clear(relative) => {
                        self.entries.remove(&relative);
                        self.failed_paths.remove(&relative);
                    }
                    PathAction::Insert(relative, entry) => {
                        self.entries.insert(relative.clone(), entry);
                        self.failed_paths.remove(&relative);
                    }
                    PathAction::MarkFailed(relative) => {
                        self.entries.remove(&relative);
                        self.failed_paths.insert(relative);
                    }
                }
            }
            if let IndexState::Ready { files, failed } = &mut self.state {
                *files = self.entries.len();
                *failed = self.failed_paths.len();
            }
        }

        fn rescan_full(shared: &Arc<Mutex<DirectoryIndex>>, managed_dir: &Path, agent_home: &Path) {
            let nested = nested_managed_relative_paths(managed_dir).unwrap_or_default();
            let matcher = load_ignore_matcher(agent_home, managed_dir);
            let warm_delay = warm_delay_per_file();

            // Reserve a fresh generation, snapshot existing keys, reset state
            // to warming. We mutate guard.entries in place per file (preserving
            // any watcher-installed deltas that land during the scan); at the
            // end we only remove keys that existed before the scan and weren't
            // revisited.
            let (my_gen, pre_scan_keys, pre_scan_failed) = {
                let mut guard = match shared.lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                guard.scan_generation = guard.scan_generation.wrapping_add(1);
                guard.state = IndexState::Warming {
                    scanned: 0,
                    total: None,
                };
                guard.stop.store(false, Ordering::Relaxed);
                let my_gen = guard.scan_generation;
                let keys: BTreeSet<String> = guard.entries.keys().cloned().collect();
                let failed: BTreeSet<String> = guard.failed_paths.iter().cloned().collect();
                (my_gen, keys, failed)
            };

            let mut visited: BTreeSet<String> = BTreeSet::new();
            let mut scanned: usize = 0;

            let mut stack: Vec<PathBuf> = vec![managed_dir.to_path_buf()];
            while let Some(dir) = stack.pop() {
                match shared.lock() {
                    Ok(g) => {
                        if g.scan_generation != my_gen || g.stop.load(Ordering::Relaxed) {
                            return;
                        }
                    }
                    Err(_) => return,
                }

                let read_dir = match std::fs::read_dir(&dir) {
                    Ok(it) => it,
                    Err(_) => continue,
                };
                for entry in read_dir {
                    let entry = match entry {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    let path = entry.path();
                    let metadata = match std::fs::symlink_metadata(&path) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    if metadata.file_type().is_symlink() {
                        continue;
                    }

                    if metadata.is_dir() {
                        if !is_managed_content_path(managed_dir, &path)
                            || is_nested_managed_path_with_list(managed_dir, &path, &nested)
                        {
                            continue;
                        }
                        stack.push(path);
                        continue;
                    }

                    if !metadata.is_file() {
                        continue;
                    }
                    if !is_managed_content_path(managed_dir, &path)
                        || path_has_agent_home_component(managed_dir, &path)
                        || is_nested_managed_path_with_list(managed_dir, &path, &nested)
                    {
                        continue;
                    }
                    if matcher_matches(matcher.as_ref(), managed_dir, &path) {
                        continue;
                    }

                    let relative = match relative_managed_path(managed_dir, &path) {
                        Ok(r) => r,
                        Err(_) => continue,
                    };
                    let outcome = match read_index_entry(&path) {
                        Ok(o) => o,
                        Err(_) => continue,
                    };
                    let entry_value = match outcome {
                        ReadOutcome::Indexed(entry) => Some(entry),
                        ReadOutcome::Failed => None,
                        ReadOutcome::Skipped => continue,
                    };

                    scanned += 1;
                    if let Ok(mut guard) = shared.lock() {
                        if guard.scan_generation != my_gen || guard.stop.load(Ordering::Relaxed) {
                            return;
                        }
                        match entry_value {
                            Some(entry) => {
                                guard.entries.insert(relative.clone(), entry);
                                guard.failed_paths.remove(&relative);
                            }
                            None => {
                                guard.entries.remove(&relative);
                                guard.failed_paths.insert(relative.clone());
                            }
                        }
                        guard.state = IndexState::Warming {
                            scanned,
                            total: None,
                        };
                    } else {
                        return;
                    }
                    visited.insert(relative);

                    if let Some(delay) = warm_delay {
                        thread::sleep(delay);
                    }
                }
            }

            if let Ok(mut guard) = shared.lock() {
                if guard.scan_generation != my_gen || guard.stop.load(Ordering::Relaxed) {
                    return;
                }
                // Remove only keys that existed before the scan started AND
                // were not revisited. Keys NOT in pre_scan_keys are watcher
                // deltas that landed during the scan and must be preserved.
                let stale: Vec<String> = pre_scan_keys.difference(&visited).cloned().collect();
                for key in stale {
                    guard.entries.remove(&key);
                }
                let stale_failed: Vec<String> =
                    pre_scan_failed.difference(&visited).cloned().collect();
                for key in stale_failed {
                    guard.failed_paths.remove(&key);
                }
                let files = guard.entries.len();
                let failed = guard.failed_paths.len();
                guard.state = IndexState::Ready { files, failed };
            }
        }
    }

    fn spawn_warm_task(
        managed_dir: PathBuf,
        agent_home: PathBuf,
        index: Arc<Mutex<DirectoryIndex>>,
    ) {
        thread::spawn(move || {
            DirectoryIndex::rescan_full(&index, &managed_dir, &agent_home);
        });
    }

    fn warm_delay_per_file() -> Option<Duration> {
        std::env::var(INDEX_WARM_DELAY_ENV)
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|ms| *ms > 0)
            .map(Duration::from_millis)
    }

    fn matcher_matches(
        matcher: Option<&ignore::gitignore::Gitignore>,
        managed_dir: &Path,
        path: &Path,
    ) -> bool {
        let Some(matcher) = matcher else {
            return false;
        };
        let Ok(relative) = path.strip_prefix(managed_dir) else {
            return false;
        };
        matcher
            .matched_path_or_any_parents(relative, false)
            .is_ignore()
    }

    fn is_nested_managed_path_with_list(
        managed_dir: &Path,
        path: &Path,
        nested: &[String],
    ) -> bool {
        for relative in nested {
            let nested_root = managed_dir.join(relative);
            if path == nested_root || path.starts_with(&nested_root) {
                return true;
            }
        }
        false
    }

    fn classify_as_text(bytes: &[u8]) -> bool {
        let probe_end = bytes.len().min(INDEX_FINGERPRINT_BYTES);
        let probe = &bytes[..probe_end];
        if probe.contains(&0u8) {
            return false;
        }
        std::str::from_utf8(probe).is_ok()
    }

    const PDF_EXTRACT_BYTE_CAP: u64 = 50 * 1024 * 1024;

    fn read_index_entry(path: &Path) -> io::Result<ReadOutcome> {
        let mut file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(ReadOutcome::Skipped);
            }
            Err(error) => return Err(error),
        };
        let mut buffer = Vec::with_capacity(INDEX_FINGERPRINT_BYTES);
        let mut chunk = [0u8; INDEX_FINGERPRINT_BYTES];
        let mut total_read: u64 = 0;
        loop {
            let read = file.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            total_read += read as u64;
            if buffer.len() < INDEX_FINGERPRINT_BYTES {
                let take = (INDEX_FINGERPRINT_BYTES - buffer.len()).min(read);
                buffer.extend_from_slice(&chunk[..take]);
            }
            if total_read as usize >= MAX_TEXT_BYTES {
                break;
            }
        }
        if buffer.starts_with(b"%PDF-") {
            return Ok(read_pdf_entry(path, &buffer));
        }
        if !classify_as_text(&buffer) {
            return Ok(ReadOutcome::Skipped);
        }
        let length = std::fs::metadata(path)
            .map(|m| m.len())
            .unwrap_or(total_read);
        Ok(ReadOutcome::Indexed(IndexEntry {
            length,
            fingerprint: buffer,
            extracted_text: None,
        }))
    }

    fn read_pdf_entry(path: &Path, head: &[u8]) -> ReadOutcome {
        let length = match std::fs::metadata(path) {
            Ok(m) => m.len(),
            Err(_) => return ReadOutcome::Failed,
        };
        if length > PDF_EXTRACT_BYTE_CAP {
            return pdf_text_fallback(head, length);
        }
        let path_buf = path.to_path_buf();
        let extracted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pdf_extract::extract_text(&path_buf)
        }));
        match extracted {
            Ok(Ok(text)) => {
                let fingerprint_end = head.len().min(INDEX_FINGERPRINT_BYTES);
                let fingerprint = head[..fingerprint_end].to_vec();
                ReadOutcome::Indexed(IndexEntry {
                    length,
                    fingerprint,
                    extracted_text: Some(text),
                })
            }
            Ok(Err(_)) | Err(_) => pdf_text_fallback(head, length),
        }
    }

    // The %PDF- magic was present but PDF extraction is not viable (file
    // too large to parse, parse error, or panic). If the head buffer still
    // classifies as text, this is a plain-text file that incidentally starts
    // with the magic — index it as text rather than counting it as a broken
    // PDF.
    fn pdf_text_fallback(head: &[u8], length: u64) -> ReadOutcome {
        if classify_as_text(head) {
            let fingerprint_end = head.len().min(INDEX_FINGERPRINT_BYTES);
            let fingerprint = head[..fingerprint_end].to_vec();
            ReadOutcome::Indexed(IndexEntry {
                length,
                fingerprint,
                extracted_text: None,
            })
        } else {
            ReadOutcome::Failed
        }
    }

    fn snapshot_index_state(index: &Arc<Mutex<DirectoryIndex>>) -> IndexState {
        match index.lock() {
            Ok(guard) => guard.snapshot_state(),
            Err(_) => IndexState::Warming {
                scanned: 0,
                total: None,
            },
        }
    }

    fn format_index_token(index: &Arc<Mutex<DirectoryIndex>>) -> String {
        let snapshot = match index.lock() {
            Ok(guard) => guard.snapshot_state(),
            Err(_) => return "warming".to_string(),
        };
        match snapshot {
            IndexState::Warming {
                scanned: 0,
                total: None,
            } => "warming".to_string(),
            IndexState::Warming {
                scanned,
                total: None,
            } => format!("warming(scanned={scanned})"),
            IndexState::Warming {
                scanned,
                total: Some(total),
            } => format!("warming(scanned={scanned}/total={total})"),
            IndexState::Ready { files, failed: 0 } => format!("ready(files={files})"),
            IndexState::Ready { files, failed } => {
                format!("ready(files={files}, failed={failed})")
            }
        }
    }

    fn format_direct_ask_response(
        answer: &str,
        requested_path: &Path,
        agent_identity: &str,
        index_state: &IndexState,
    ) -> String {
        let mut response = String::new();
        response.push_str(answer);
        if !response.ends_with('\n') {
            response.push('\n');
        }
        response.push_str("references:\n");
        response.push_str(&format!("- {}\n", requested_path.display()));
        push_index_caveat(&mut response, index_state);
        response.push_str(&format!("participating_agents: {agent_identity}\n"));
        response.push_str("changed_files: none\n");
        response
    }

    fn push_index_caveat(response: &mut String, index_state: &IndexState) {
        match index_state {
            IndexState::Warming { .. } => {
                response.push_str("caveat: local index is warming; answer may be incomplete\n");
            }
            IndexState::Ready { failed, .. } if *failed > 0 => {
                response.push_str(&format!(
                    "caveat: local index could not extract {failed} file(s); answer may be incomplete\n"
                ));
            }
            IndexState::Ready { .. } => {}
        }
    }

    fn format_delegated_supervisor_response(
        outcome: &DelegationOutcome,
        requested_path: &Path,
        requester_identity: &str,
        index_state: &IndexState,
    ) -> String {
        let mut response = String::new();
        if let [reply] = outcome.replies.as_slice() {
            response.push_str(&reply.answer);
            if !response.ends_with('\n') {
                response.push('\n');
            }
        } else {
            response.push_str("answers:\n");
            for reply in &outcome.replies {
                response.push_str(&format!("- agent={}\n", reply.agent_identity));
                response.push_str(&format!("  {}\n", reply.answer));
            }
        }
        for progress in &outcome.progress {
            response.push_str(progress);
            response.push('\n');
        }
        response.push_str("references:\n");
        response.push_str(&format!("- {}\n", requested_path.display()));
        push_index_caveat(&mut response, index_state);
        let participating_agents = participating_agents(
            requester_identity,
            outcome
                .replies
                .iter()
                .map(|reply| reply.agent_identity.as_str()),
        );
        response.push_str(&format!(
            "participating_agents: {}\n",
            participating_agents.join(", ")
        ));
        let changed_files = outcome
            .replies
            .iter()
            .flat_map(|reply| reply.changed_files.iter().cloned())
            .collect::<Vec<_>>();
        let history_entries = outcome
            .replies
            .iter()
            .flat_map(|reply| reply.history_entries.iter().cloned())
            .collect::<Vec<_>>();
        response.push_str(&format!("changed_files: {}\n", report_list(&changed_files)));
        response.push_str(&format!(
            "history_entries: {}\n",
            report_list(&history_entries)
        ));
        response
    }

    fn format_delegated_delegator_response(
        answer: &str,
        reply: &TaskReply,
        requested_path: &Path,
        requester_identity: &str,
        index_state: &IndexState,
    ) -> String {
        let mut response = String::new();
        response.push_str(answer);
        if !response.ends_with('\n') {
            response.push('\n');
        }
        response.push_str("references:\n");
        response.push_str(&format!("- {}\n", requested_path.display()));
        push_index_caveat(&mut response, index_state);
        response.push_str(&format!(
            "participating_agents: {}, {}\n",
            requester_identity, reply.agent_identity
        ));
        response.push_str(&format!(
            "changed_files: {}\n",
            report_list(&reply.changed_files)
        ));
        response.push_str(&format!(
            "history_entries: {}\n",
            report_list(&reply.history_entries)
        ));
        response
    }

    fn participating_agents<'a>(
        requester_identity: &'a str,
        replies: impl Iterator<Item = &'a str>,
    ) -> Vec<&'a str> {
        let mut seen = BTreeSet::new();
        let mut agents = Vec::new();
        seen.insert(requester_identity);
        agents.push(requester_identity);
        for identity in replies {
            if seen.insert(identity) {
                agents.push(identity);
            }
        }
        agents
    }

    fn format_broadcast_ask_response(
        replies: &[BroadcastReply],
        outcome: &CollaborationOutcome,
        timeout: Duration,
    ) -> String {
        let mut response = String::new();
        if replies.is_empty() {
            response.push_str("no relevant agents replied before broadcast timeout\n");
        } else {
            response.push_str("answers:\n");
            for reply in replies {
                response.push_str(&format!(
                    "- agent={} managed_dir={} relevance={} reason={}\n",
                    reply.agent_identity,
                    reply.managed_dir.display(),
                    reply.relevance,
                    reply.reason
                ));
                response.push_str(&format!("  {}\n", reply.answer));
            }
        }

        if !outcome.answers.is_empty() {
            response.push_str("collaboration:\n");
            for answer in &outcome.answers {
                response.push_str(&format!("- agent={}\n", answer.agent_identity));
                response.push_str(&format!("  {}\n", answer.answer));
            }
        }
        for progress in &outcome.progress {
            response.push_str(progress);
            response.push('\n');
        }

        let references = replies
            .iter()
            .flat_map(|reply| reply.file_references.iter())
            .cloned()
            .collect::<BTreeSet<_>>();
        response.push_str("references:\n");
        if references.is_empty() {
            response.push_str("- none\n");
        } else {
            for reference in references {
                response.push_str(&format!("- {reference}\n"));
            }
        }

        let mut seen = BTreeSet::new();
        let mut participants: Vec<&str> = Vec::new();
        for reply in replies {
            if seen.insert(reply.agent_identity.as_str()) {
                participants.push(reply.agent_identity.as_str());
            }
        }
        for answer in &outcome.answers {
            if seen.insert(answer.agent_identity.as_str()) {
                participants.push(answer.agent_identity.as_str());
            }
        }
        response.push_str(&format!(
            "participating_agents: {}\n",
            if participants.is_empty() {
                "none".to_string()
            } else {
                participants.join(", ")
            }
        ));

        let aggregated_changed: Vec<String> = outcome
            .answers
            .iter()
            .flat_map(|answer| answer.changed_files.iter().cloned())
            .collect();
        let aggregated_history: Vec<String> = outcome
            .answers
            .iter()
            .flat_map(|answer| answer.history_entries.iter().cloned())
            .collect();
        response.push_str(&format!(
            "changed_files: {}\n",
            report_list(&aggregated_changed)
        ));
        response.push_str(&format!(
            "history_entries: {}\n",
            report_list(&aggregated_history)
        ));
        response.push_str(&format!("broadcast_timeout_ms: {}\n", timeout.as_millis()));
        response
    }

    fn parse_broadcast_reply(line: &str, agent: &RegisteredAgent) -> Option<BroadcastReply> {
        let mut fields = line.trim_end_matches('\r').splitn(4, '\t');
        let relevance = fields.next()?;
        if !matches!(relevance, "possible" | "strong") {
            return None;
        }

        let reason = fields.next()?.to_string();
        let answer = fields.next()?.to_string();
        let matcher = load_ignore_matcher(&agent.agent_home, &agent.managed_dir);
        let file_references = fields
            .next()
            .unwrap_or_default()
            .split(';')
            .filter_map(|reference| normalize_broadcast_reference(reference, &agent.managed_dir))
            .filter(|reference| !ignore_matches(&matcher, &agent.managed_dir, reference))
            .collect::<Vec<_>>();

        Some(BroadcastReply {
            agent_identity: agent.identity.clone(),
            managed_dir: agent.managed_dir.clone(),
            relevance: relevance.to_string(),
            reason,
            answer,
            file_references,
        })
    }

    fn parse_delegate_request(line: &str) -> io::Result<Option<DelegateRequest>> {
        let line = line.trim_end_matches(['\n', '\r']);
        let Some(payload) = line.strip_prefix("DELEGATE\t") else {
            return Ok(None);
        };

        let mut fields = payload.splitn(3, '\t');
        let Some(target) = fields.next().filter(|value| !value.is_empty()) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "delegation request is missing target",
            ));
        };
        let Some(reply_target) = fields.next() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "delegation request is missing reply target",
            ));
        };
        let Some(prompt) = fields.next() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "delegation request is missing prompt",
            ));
        };

        let reply_target = match reply_target {
            "delegator" => ReplyTarget::Delegator,
            "supervisor" => ReplyTarget::Supervisor,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "delegation reply target must be delegator or supervisor",
                ));
            }
        };

        Ok(Some(DelegateRequest {
            target: target.to_string(),
            reply_target,
            prompt: prompt.to_string(),
        }))
    }

    fn parse_task_reply(line: &str, agent: &RegisteredAgent) -> TaskReply {
        let line = line.trim_end_matches(['\n', '\r']);
        let Some(payload) = line.strip_prefix("TASK_REPLY\t") else {
            return TaskReply {
                agent_identity: agent.identity.clone(),
                answer: line.to_string(),
                changed_files: Vec::new(),
                history_entries: Vec::new(),
            };
        };

        let mut fields = payload.splitn(3, '\t');
        TaskReply {
            agent_identity: agent.identity.clone(),
            answer: fields.next().unwrap_or_default().to_string(),
            changed_files: parse_report_list(fields.next().unwrap_or_default()),
            history_entries: parse_report_list(fields.next().unwrap_or_default()),
        }
    }

    fn parse_collaborate_reply(line: &str, agent: &RegisteredAgent) -> TaskReply {
        let line = line.trim_end_matches(['\n', '\r']);
        let Some(payload) = line.strip_prefix("COLLABORATE_REPLY\t") else {
            return TaskReply {
                agent_identity: agent.identity.clone(),
                answer: line.to_string(),
                changed_files: Vec::new(),
                history_entries: Vec::new(),
            };
        };

        let mut fields = payload.splitn(3, '\t');
        TaskReply {
            agent_identity: agent.identity.clone(),
            answer: fields.next().unwrap_or_default().to_string(),
            changed_files: parse_report_list(fields.next().unwrap_or_default()),
            history_entries: parse_report_list(fields.next().unwrap_or_default()),
        }
    }

    fn parse_report_list(field: &str) -> Vec<String> {
        if field == "none" || field.trim().is_empty() {
            return Vec::new();
        }

        field
            .split(';')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    fn report_list(values: &[String]) -> String {
        if values.is_empty() {
            "none".to_string()
        } else {
            values.join(", ")
        }
    }

    fn load_ignore_matcher(
        agent_home: &Path,
        managed_dir: &Path,
    ) -> Option<ignore::gitignore::Gitignore> {
        let ignore_path = agent_home.join(IGNORE_FILE);
        if !ignore_path.is_file() {
            return None;
        }
        let mut builder = ignore::gitignore::GitignoreBuilder::new(managed_dir);
        if builder.add(&ignore_path).is_some() {
            return None;
        }
        builder.build().ok()
    }

    fn ignore_matches(
        matcher: &Option<ignore::gitignore::Gitignore>,
        managed_dir: &Path,
        normalized_reference: &str,
    ) -> bool {
        let Some(matcher) = matcher else {
            return false;
        };
        let reference_path = Path::new(normalized_reference);
        let Ok(relative) = reference_path.strip_prefix(managed_dir) else {
            return false;
        };
        matcher
            .matched_path_or_any_parents(relative, false)
            .is_ignore()
    }

    fn normalize_broadcast_reference(reference: &str, managed_dir: &Path) -> Option<String> {
        let reference = reference.trim();
        if reference.is_empty() {
            return None;
        }

        let path = Path::new(reference);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            managed_dir.join(path)
        };

        if matches!(path.try_exists(), Ok(true)) {
            let canonical = path.canonicalize().ok()?;
            return canonical
                .starts_with(managed_dir)
                .then(|| canonical.display().to_string());
        }

        path.starts_with(managed_dir)
            .then(|| path.display().to_string())
    }

    fn broadcast_reply_timeout() -> Duration {
        std::env::var(BROADCAST_REPLY_TIMEOUT_ENV)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|milliseconds| *milliseconds > 0)
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_BROADCAST_REPLY_TIMEOUT)
    }

    fn read_blocking_line(stdout: &mut ChildStdout) -> io::Result<String> {
        let mut line = Vec::new();
        let mut byte = [0_u8; 1];

        loop {
            match stdout.read(&mut byte) {
                Ok(0) if line.is_empty() => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "AFS agent runtime closed before answering",
                    ));
                }
                Ok(0) => return Ok(String::from_utf8_lossy(&line).to_string()),
                Ok(_) => {
                    line.push(byte[0]);
                    if byte[0] == b'\n' {
                        return Ok(String::from_utf8_lossy(&line).to_string());
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }

    fn read_line_with_deadline(stdout: &mut ChildStdout, deadline: Instant) -> io::Result<String> {
        set_nonblocking(stdout, true)?;
        let mut buffer = Vec::new();
        let result = loop {
            match read_nonblocking_line(stdout, &mut buffer) {
                Ok(Some(line)) => break Ok(line),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        break Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "AFS agent runtime did not reply within the deadline",
                        ));
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => break Err(error),
            }
        };
        // Best-effort restore of blocking mode; surface only the original error.
        let _ = set_nonblocking(stdout, false);
        result
    }

    fn read_nonblocking_line(
        stdout: &mut ChildStdout,
        buffer: &mut Vec<u8>,
    ) -> io::Result<Option<String>> {
        let mut byte = [0_u8; 1];

        loop {
            match stdout.read(&mut byte) {
                Ok(0) if buffer.is_empty() => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "AFS agent runtime closed before answering",
                    ));
                }
                Ok(0) => {
                    let line = String::from_utf8_lossy(buffer).to_string();
                    buffer.clear();
                    return Ok(Some(line));
                }
                Ok(_) if byte[0] == b'\n' => {
                    let line = String::from_utf8_lossy(buffer).to_string();
                    buffer.clear();
                    return Ok(Some(line));
                }
                Ok(_) => buffer.push(byte[0]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }

    fn set_nonblocking(stdout: &ChildStdout, nonblocking: bool) -> io::Result<()> {
        let flags = unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }

        let updated_flags = if nonblocking {
            flags | libc::O_NONBLOCK
        } else {
            flags & !libc::O_NONBLOCK
        };
        if unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_SETFL, updated_flags) } < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
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
            if !looks_like_explicit_path_token(token, path) {
                continue;
            }

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

    fn looks_like_explicit_path_token(token: &str, path: &Path) -> bool {
        path.is_absolute()
            || token == "."
            || token == ".."
            || token.starts_with("./")
            || token.starts_with("../")
            || token.contains('/')
            || path.extension().is_some()
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
        let afs_home = home()?;
        let config = crate::config::Config::load(&afs_home)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "AFS is not configured. Run `afs login --provider claude|openai` to authenticate.",
            )
        })?;
        let runtime = pi_runtime_command();
        let mut command = Command::new(&runtime);
        command
            .arg("--mode")
            .arg("rpc")
            .arg("--provider")
            .arg(config.provider.as_cli_str());
        if let Some(model) = config.model.as_deref() {
            command.arg("--model").arg(model);
        }
        command
            .current_dir(managed_dir)
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
                            "AFS agent runtime not found: {} (set {PI_RUNTIME_ENV})",
                            runtime.display()
                        ),
                    )
                } else {
                    error
                }
            })
    }

    pub fn pi_runtime_command() -> PathBuf {
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

    fn history_lock() -> &'static Mutex<()> {
        static HISTORY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        HISTORY_LOCK.get_or_init(|| Mutex::new(()))
    }

    fn ensure_ignore_file_seeded(managed_dir: &Path, agent_home: &Path) -> io::Result<()> {
        let ignore_path = agent_home.join(IGNORE_FILE);
        if ignore_path.exists() {
            return Ok(());
        }
        let mut contents =
            String::from("# AFS ignore policy - seeded from .gitignore at install.\n");
        match std::fs::read_to_string(managed_dir.join(".gitignore")) {
            Ok(gitignore) => contents.push_str(&gitignore),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        std::fs::write(ignore_path, contents)
    }

    fn ensure_history_baseline_commit(managed_dir: &Path, agent_home: &Path) -> io::Result<()> {
        let git_dir = history_repo_dir(agent_home);
        git_init_if_missing(&git_dir)?;
        if git_has_commits(&git_dir)? {
            return Ok(());
        }
        let nested = nested_managed_relative_paths(managed_dir)?;
        git_stage_and_commit(
            &git_dir,
            managed_dir,
            &nested,
            &GitCommitRequest {
                entry_id: &history_entry_id(),
                kind: "baseline",
                summary: "Install baseline",
                files: &[],
                undoable: false,
                undoes: None,
                origin: None,
            },
        )
    }

    fn nested_managed_relative_paths(managed_dir: &Path) -> io::Result<Vec<String>> {
        let mut results = Vec::new();
        collect_nested_managed_relative_paths(managed_dir, managed_dir, &mut results)?;
        Ok(results)
    }

    fn collect_nested_managed_relative_paths(
        managed_dir: &Path,
        current_dir: &Path,
        results: &mut Vec<String>,
    ) -> io::Result<()> {
        for entry in std::fs::read_dir(current_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path == managed_dir.join(AGENT_HOME_DIR) {
                continue;
            }
            let file_type = entry.file_type()?;
            if file_type.is_symlink() || !file_type.is_dir() {
                continue;
            }
            if is_nested_managed_root(managed_dir, &path) {
                results.push(relative_managed_path(managed_dir, &path)?);
                continue;
            }
            collect_nested_managed_relative_paths(managed_dir, &path, results)?;
        }
        Ok(())
    }

    fn start_directory_monitor(
        managed_dir: PathBuf,
        agent_home: PathBuf,
        index: Arc<Mutex<DirectoryIndex>>,
    ) -> io::Result<DirectoryMonitor> {
        let (ready_sender, ready_receiver) = mpsc::channel();
        let (stop_sender, stop_receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            if let Err(error) =
                monitor_directory(managed_dir, agent_home, index, ready_sender, stop_receiver)
            {
                eprintln!("directory monitor stopped: {error}");
            }
        });

        match ready_receiver.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => Ok(DirectoryMonitor {
                stop: stop_sender,
                handle,
            }),
            Ok(Err(message)) => Err(io::Error::other(message)),
            Err(error) => Err(io::Error::other(format!(
                "directory monitor did not start: {error}"
            ))),
        }
    }

    fn monitor_directory(
        managed_dir: PathBuf,
        agent_home: PathBuf,
        index: Arc<Mutex<DirectoryIndex>>,
        ready_sender: mpsc::Sender<Result<(), String>>,
        stop_receiver: mpsc::Receiver<()>,
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
            if stop_receiver.try_recv().is_ok() {
                return Ok(());
            }

            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(event)) => {
                    if event_affects_managed_subtree(&managed_dir, &event.paths) {
                        let mut settled = SettledPaths::new();
                        settled.absorb(&managed_dir, &event.paths);
                        wait_for_settled_events(&receiver, &managed_dir, &mut settled);
                        record_external_change(&managed_dir, &agent_home)?;
                        update_index_after_settle(&managed_dir, &agent_home, &index, settled);
                    }
                }
                Ok(Err(_error)) => {}
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }
    }

    struct SettledPaths {
        paths: BTreeSet<PathBuf>,
        saturated: bool,
    }

    impl SettledPaths {
        fn new() -> Self {
            Self {
                paths: BTreeSet::new(),
                saturated: false,
            }
        }

        fn absorb(&mut self, managed_dir: &Path, paths: &[PathBuf]) {
            for path in paths {
                if !is_managed_content_path(managed_dir, path) {
                    continue;
                }
                if self.paths.len() >= INDEX_SETTLED_PATH_CAP {
                    self.saturated = true;
                    return;
                }
                self.paths.insert(path.clone());
            }
        }
    }

    fn update_index_after_settle(
        managed_dir: &Path,
        agent_home: &Path,
        index: &Arc<Mutex<DirectoryIndex>>,
        settled: SettledPaths,
    ) {
        if settled.saturated {
            DirectoryIndex::rescan_full(index, managed_dir, agent_home);
            return;
        }
        let nested = nested_managed_relative_paths(managed_dir).unwrap_or_default();
        let matcher = load_ignore_matcher(agent_home, managed_dir);
        let paths_vec: Vec<PathBuf> = settled.paths.into_iter().collect();
        let actions = compute_path_updates(managed_dir, &paths_vec, matcher.as_ref(), &nested);
        if actions.is_empty() {
            return;
        }
        if let Ok(mut guard) = index.lock() {
            guard.apply_path_updates(actions);
        }
    }

    fn compute_path_updates(
        managed_dir: &Path,
        paths: &[PathBuf],
        matcher: Option<&ignore::gitignore::Gitignore>,
        nested: &[String],
    ) -> Vec<PathAction> {
        let mut actions = Vec::with_capacity(paths.len());
        for path in paths {
            if !is_managed_content_path(managed_dir, path)
                || path_has_agent_home_component(managed_dir, path)
                || is_nested_managed_path_with_list(managed_dir, path, nested)
            {
                continue;
            }
            let relative = match relative_managed_path(managed_dir, path) {
                Ok(r) => r,
                Err(_) => continue,
            };
            if matcher_matches(matcher, managed_dir, path) {
                actions.push(PathAction::Clear(relative));
                continue;
            }
            match read_index_entry(path) {
                Ok(ReadOutcome::Indexed(entry)) => {
                    actions.push(PathAction::Insert(relative, entry));
                }
                Ok(ReadOutcome::Skipped) | Err(_) => {
                    actions.push(PathAction::Clear(relative));
                }
                Ok(ReadOutcome::Failed) => {
                    actions.push(PathAction::MarkFailed(relative));
                }
            }
        }
        actions
    }

    fn wait_for_settled_events(
        receiver: &Receiver<notify::Result<notify::Event>>,
        managed_dir: &Path,
        settled: &mut SettledPaths,
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
                        settled.absorb(managed_dir, &event.paths);
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
        path != managed_dir
            && path.starts_with(managed_dir)
            && !path.starts_with(agent_home)
            && !path_has_agent_home_component(managed_dir, path)
            && !is_nested_managed_path(managed_dir, path)
    }

    fn record_external_change(managed_dir: &Path, agent_home: &Path) -> io::Result<()> {
        record_snapshot_delta(managed_dir, agent_home, "external", "External change").map(|_| ())
    }

    fn record_agent_change(
        managed_dir: &Path,
        agent_home: &Path,
    ) -> io::Result<Option<RecordedChange>> {
        record_snapshot_delta(managed_dir, agent_home, "agent", "Agent change")
    }

    fn record_startup_reconciliation(managed_dir: &Path, agent_home: &Path) -> io::Result<()> {
        std::fs::create_dir_all(agent_home.join(HISTORY_DIR))?;
        ensure_history_baseline_commit(managed_dir, agent_home)?;
        record_snapshot_delta(
            managed_dir,
            agent_home,
            "reconciliation",
            "Startup reconciliation",
        )
        .map(|_| ())
    }

    fn record_snapshot_delta(
        managed_dir: &Path,
        agent_home: &Path,
        kind: &str,
        summary_prefix: &str,
    ) -> io::Result<Option<RecordedChange>> {
        let _guard = history_lock()
            .lock()
            .map_err(|_| io::Error::other("history lock poisoned"))?;
        let git_dir = history_repo_dir(agent_home);
        let nested = nested_managed_relative_paths(managed_dir)?;
        git_stage_work_tree(&git_dir, managed_dir, &nested)?;
        let changed_files = git_staged_changes_vs_head(&git_dir)?;
        if changed_files.is_empty() {
            return Ok(None);
        }

        let entry_id = history_entry_id();
        let summary = history_summary(summary_prefix, &changed_files);
        git_commit_index(
            &git_dir,
            managed_dir,
            &GitCommitRequest {
                entry_id: &entry_id,
                kind,
                summary: &summary,
                files: &changed_files,
                undoable: true,
                undoes: None,
                origin: None,
            },
        )?;
        Ok(Some(RecordedChange {
            history_entry: entry_id,
            files: changed_files,
        }))
    }

    fn record_ownership_event(
        managed_dir: &Path,
        agent_home: &Path,
        summary: &str,
    ) -> io::Result<()> {
        let _guard = history_lock()
            .lock()
            .map_err(|_| io::Error::other("history lock poisoned"))?;
        let git_dir = history_repo_dir(agent_home);
        let nested = nested_managed_relative_paths(managed_dir)?;
        git_stage_work_tree(&git_dir, managed_dir, &nested)?;
        let changed_files = git_staged_changes_vs_head(&git_dir)?;
        let entry_id = history_entry_id();
        git_commit_index(
            &git_dir,
            managed_dir,
            &GitCommitRequest {
                entry_id: &entry_id,
                kind: "ownership",
                summary,
                files: &changed_files,
                undoable: false,
                undoes: None,
                origin: None,
            },
        )
    }

    fn merge_archived_child_history(
        archived_child_home: &Path,
        parent_managed_dir: &Path,
        parent_agent_home: &Path,
        child_origin: &str,
    ) -> io::Result<()> {
        let _guard = history_lock()
            .lock()
            .map_err(|_| io::Error::other("history lock poisoned"))?;
        let child_git_dir = history_repo_dir(archived_child_home);
        let mut commits = git_log_records(&child_git_dir)?;
        commits.reverse();

        let parent_git_dir = history_repo_dir(parent_agent_home);
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for record in commits {
            let Some(entry_id) = record.trailers.get("Afs-Entry-Id").cloned() else {
                continue;
            };
            if !seen.insert(entry_id.clone()) {
                continue;
            }
            let kind = record.trailers.get("Afs-Kind").cloned().unwrap_or_default();
            if kind == "baseline" {
                continue;
            }
            let original_summary = record
                .trailers
                .get("Afs-Summary")
                .cloned()
                .unwrap_or_default();
            let original_files: Vec<String> = record
                .trailers
                .get("Afs-Files")
                .map(|field| {
                    if field.is_empty() {
                        Vec::new()
                    } else {
                        field.split(", ").map(ToOwned::to_owned).collect()
                    }
                })
                .unwrap_or_default();
            let rewritten_files: Vec<String> = original_files
                .iter()
                .map(|file| prefix_child_path(child_origin, file))
                .collect();
            let rewritten_summary = if kind == "ownership" {
                rewrite_ownership_summary(&original_summary, child_origin)
            } else {
                rewrite_summary_paths(&original_summary, &original_files, &rewritten_files)
            };
            let existing_origin = record
                .trailers
                .get("Afs-Origin")
                .cloned()
                .unwrap_or_default();
            let chained_origin = chain_origins(child_origin, &existing_origin);
            git_commit_index(
                &parent_git_dir,
                parent_managed_dir,
                &GitCommitRequest {
                    entry_id: &entry_id,
                    kind: &kind,
                    summary: &rewritten_summary,
                    files: &rewritten_files,
                    undoable: false,
                    undoes: None,
                    origin: Some(&chained_origin),
                },
            )?;
        }
        Ok(())
    }

    fn prefix_child_path(origin: &str, file: &str) -> String {
        if file.is_empty() || origin.is_empty() {
            return file.to_string();
        }
        format!("{origin}/{file}")
    }

    fn chain_origins(outer: &str, inner: &str) -> String {
        match (outer.is_empty(), inner.is_empty()) {
            (true, _) => inner.to_string(),
            (_, true) => outer.to_string(),
            _ => format!("{outer}/{inner}"),
        }
    }

    fn rewrite_summary_paths(
        summary: &str,
        original_files: &[String],
        rewritten_files: &[String],
    ) -> String {
        let mut result = summary.to_string();
        for (original, rewritten) in original_files.iter().zip(rewritten_files.iter()) {
            if original.is_empty() || original == rewritten {
                continue;
            }
            result = result.replacen(original, rewritten, 1);
        }
        result
    }

    fn rewrite_ownership_summary(summary: &str, origin_prefix: &str) -> String {
        if origin_prefix.is_empty() {
            return summary.to_string();
        }
        match summary.split_once(": ") {
            Some((prefix, path)) if !path.is_empty() => {
                format!("{prefix}: {origin_prefix}/{path}")
            }
            _ => summary.to_string(),
        }
    }

    fn archive_agent_home(
        removed_agent_home: &Path,
        archive_root: &Path,
        archive_name: &str,
    ) -> io::Result<PathBuf> {
        std::fs::create_dir_all(archive_root)?;
        let full_name = format!("{archive_name}-{}", unix_timestamp_nanos());
        let archived_agent_home = archive_root.join(full_name);
        move_dir_across_filesystems(removed_agent_home, &archived_agent_home)?;
        Ok(archived_agent_home)
    }

    fn move_dir_across_filesystems(src: &Path, dst: &Path) -> io::Result<()> {
        match std::fs::rename(src, dst) {
            Ok(()) => Ok(()),
            Err(error) if is_cross_device_error(&error) => {
                copy_dir_recursively(src, dst)?;
                std::fs::remove_dir_all(src)
            }
            Err(error) => Err(error),
        }
    }

    fn is_cross_device_error(error: &io::Error) -> bool {
        error.raw_os_error() == Some(libc::EXDEV)
    }

    fn copy_dir_recursively(src: &Path, dst: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if file_type.is_dir() {
                copy_dir_recursively(&src_path, &dst_path)?;
            } else if file_type.is_symlink() {
                let target = std::fs::read_link(&src_path)?;
                std::os::unix::fs::symlink(target, &dst_path)?;
            } else {
                std::fs::copy(&src_path, &dst_path)?;
            }
        }
        Ok(())
    }

    fn archive_safe_name(value: &str) -> String {
        let sanitized = value
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '-'
                }
            })
            .collect::<String>();
        if sanitized.is_empty() {
            "agent".to_string()
        } else {
            sanitized
        }
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
        let git_dir = history_repo_dir(agent_home);
        let mut commits = git_log_records(&git_dir)?;
        commits.reverse();

        struct EntryState {
            representative: GitHistoryRecord,
            latest_undoable: String,
        }

        let mut by_id: BTreeMap<String, EntryState> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        for record in commits {
            let Some(entry_id) = record.trailers.get("Afs-Entry-Id") else {
                continue;
            };
            let kind = record
                .trailers
                .get("Afs-Kind")
                .map(String::as_str)
                .unwrap_or("");
            if kind == "baseline" {
                continue;
            }
            let undoable = record
                .trailers
                .get("Afs-Undoable")
                .cloned()
                .unwrap_or_else(|| "no".to_string());
            let entry_id = entry_id.clone();
            if let Some(state) = by_id.get_mut(&entry_id) {
                state.latest_undoable = undoable;
            } else {
                order.push(entry_id.clone());
                by_id.insert(
                    entry_id,
                    EntryState {
                        representative: record,
                        latest_undoable: undoable,
                    },
                );
            }
        }

        if order.is_empty() {
            return Ok("no history entries\n".to_string());
        }

        let mut output = String::new();
        for entry_id in order.iter().rev() {
            let state = &by_id[entry_id];
            let record = &state.representative;
            let kind = record.trailers.get("Afs-Kind").cloned().unwrap_or_default();
            let summary = record
                .trailers
                .get("Afs-Summary")
                .cloned()
                .unwrap_or_default();
            let file_count: usize = record
                .trailers
                .get("Afs-File-Count")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            let origin = record
                .trailers
                .get("Afs-Origin")
                .cloned()
                .unwrap_or_default();
            output.push_str(&format!(
                "entry={} timestamp={} type={} summary={} files={} undoable={} origin={}\n",
                entry_id,
                record.timestamp_secs,
                kind,
                summary,
                file_count,
                state.latest_undoable,
                origin,
            ));
        }
        Ok(output)
    }

    fn undoable_field(undoable: bool) -> &'static str {
        if undoable { "yes" } else { "no" }
    }

    fn undo_history_entry(
        managed_dir: &Path,
        agent_home: &Path,
        requested_entry: &str,
        confirmed: bool,
    ) -> io::Result<String> {
        let _guard = history_lock()
            .lock()
            .map_err(|_| io::Error::other("history lock poisoned"))?;
        let git_dir = history_repo_dir(agent_home);

        struct UndoCandidate {
            entry_id: String,
            kind: String,
            summary: String,
            files: Vec<String>,
            representative_commit: String,
            latest_undoable: bool,
        }

        let mut commits = git_log_records(&git_dir)?;
        commits.reverse();

        let mut by_id: BTreeMap<String, UndoCandidate> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        for record in commits {
            let Some(entry_id) = record.trailers.get("Afs-Entry-Id").cloned() else {
                continue;
            };
            let kind = record.trailers.get("Afs-Kind").cloned().unwrap_or_default();
            if kind == "baseline" {
                continue;
            }
            let summary = record
                .trailers
                .get("Afs-Summary")
                .cloned()
                .unwrap_or_default();
            let files = record
                .trailers
                .get("Afs-Files")
                .map(|field| {
                    if field.is_empty() {
                        Vec::new()
                    } else {
                        field.split(", ").map(ToOwned::to_owned).collect()
                    }
                })
                .unwrap_or_default();
            let undoable = record
                .trailers
                .get("Afs-Undoable")
                .map(|value| value == "yes")
                .unwrap_or(false);

            if let Some(existing) = by_id.get_mut(&entry_id) {
                existing.latest_undoable = undoable;
            } else {
                order.push(entry_id.clone());
                by_id.insert(
                    entry_id.clone(),
                    UndoCandidate {
                        entry_id,
                        kind,
                        summary,
                        files,
                        representative_commit: record.commit,
                        latest_undoable: undoable,
                    },
                );
            }
        }

        let Some(latest_id) = order
            .iter()
            .rev()
            .find(|id| by_id[id.as_str()].latest_undoable)
            .cloned()
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no undoable history entries",
            ));
        };

        let latest = by_id
            .remove(&latest_id)
            .expect("latest undoable id must exist in by_id");
        if latest.entry_id != requested_entry {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "only the latest undoable history entry can be undone: {}",
                    latest.entry_id
                ),
            ));
        }

        if matches!(latest.kind.as_str(), "external" | "reconciliation") && !confirmed {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "undoing an External Change requires --yes in scripted use or interactive confirmation",
            ));
        }

        let Some(parent_commit) = git_parent_commit(&git_dir, &latest.representative_commit)?
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "history entry is not undoable",
            ));
        };
        git_restore_tree(&git_dir, managed_dir, &parent_commit)?;

        let nested = nested_managed_relative_paths(managed_dir)?;
        let undo_entry_id = history_entry_id();
        let undo_summary = sanitize_field(&format!("Undo {}: {}", latest.entry_id, latest.summary));
        git_stage_and_commit(
            &git_dir,
            managed_dir,
            &nested,
            &GitCommitRequest {
                entry_id: &undo_entry_id,
                kind: "undo",
                summary: &undo_summary,
                files: &latest.files,
                undoable: false,
                undoes: Some(&latest.entry_id),
                origin: None,
            },
        )?;
        git_stage_and_commit(
            &git_dir,
            managed_dir,
            &nested,
            &GitCommitRequest {
                entry_id: &latest.entry_id,
                kind: &latest.kind,
                summary: &latest.summary,
                files: &latest.files,
                undoable: false,
                undoes: None,
                origin: None,
            },
        )?;

        Ok(format!(
            "undid history entry {}\nfiles={}\n",
            latest.entry_id,
            latest.files.len()
        ))
    }

    fn remove_managed_content(managed_dir: &Path) -> io::Result<()> {
        let agent_home = managed_dir.join(AGENT_HOME_DIR);
        for entry in std::fs::read_dir(managed_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path == agent_home {
                continue;
            }

            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                std::fs::remove_dir_all(path)?;
            } else {
                std::fs::remove_file(path)?;
            }
        }

        Ok(())
    }

    fn history_entry_id() -> String {
        let nanos = unix_timestamp_nanos();
        format!("history-{}-{nanos}", std::process::id())
    }

    fn unix_timestamp_nanos() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    }

    fn sanitize_field(value: &str) -> String {
        value.replace(['\t', '\n', '\r'], " ")
    }

    fn relative_managed_path(managed_dir: &Path, path: &Path) -> io::Result<String> {
        Ok(path
            .strip_prefix(managed_dir)
            .map_err(io::Error::other)?
            .to_string_lossy()
            .replace('\\', "/"))
    }

    fn is_nested_managed_root(managed_dir: &Path, path: &Path) -> bool {
        path != managed_dir
            && path.starts_with(managed_dir)
            && path.join(AGENT_HOME_DIR).join("identity").is_file()
    }

    fn rediscover_managed_dir(original: &Path, identity: &str) -> io::Result<Option<PathBuf>> {
        const MAX_NODES: usize = 4096;
        const MAX_DEPTH: usize = 8;
        const MAX_ANCESTOR_LEVELS: usize = 5;

        let mut search_root = original.parent();
        let mut scanned_levels = 0usize;
        while let Some(candidate_root) = search_root {
            if candidate_root.is_dir() {
                if let Some(found) =
                    scan_for_agent_identity(candidate_root, identity, MAX_DEPTH, MAX_NODES)?
                {
                    return Ok(Some(found));
                }
                scanned_levels += 1;
                if scanned_levels >= MAX_ANCESTOR_LEVELS {
                    return Ok(None);
                }
            }
            search_root = candidate_root.parent();
        }
        Ok(None)
    }

    fn scan_for_agent_identity(
        root: &Path,
        identity: &str,
        max_depth: usize,
        max_nodes: usize,
    ) -> io::Result<Option<PathBuf>> {
        let mut stack = vec![(root.to_path_buf(), 0usize)];
        let mut visited = 0usize;

        while let Some((dir, depth)) = stack.pop() {
            visited += 1;
            if visited > max_nodes {
                return Ok(None);
            }

            if directory_identity_matches(&dir, identity)? {
                return Ok(Some(dir));
            }

            if depth >= max_depth {
                continue;
            }

            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                // Stop enumerating this directory once the in-flight node
                // budget is exhausted; otherwise a single high-fanout
                // directory could allocate and stat far beyond max_nodes
                // before the next pop observes the cap.
                if visited + stack.len() >= max_nodes {
                    break;
                }
                let file_type = match entry.file_type() {
                    Ok(file_type) => file_type,
                    Err(_) => continue,
                };
                if !file_type.is_dir() || file_type.is_symlink() {
                    continue;
                }
                if entry.file_name() == AGENT_HOME_DIR {
                    continue;
                }
                stack.push((entry.path(), depth + 1));
            }
        }

        Ok(None)
    }

    fn directory_identity_matches(dir: &Path, identity: &str) -> io::Result<bool> {
        let identity_path = dir.join(AGENT_HOME_DIR).join("identity");
        if !identity_path.is_file() {
            return Ok(false);
        }
        match std::fs::read_to_string(identity_path) {
            Ok(content) => Ok(content.trim() == identity),
            Err(_) => Ok(false),
        }
    }

    fn is_nested_managed_path(managed_dir: &Path, path: &Path) -> bool {
        if !path.starts_with(managed_dir) {
            return false;
        }

        let mut current = path;
        loop {
            if is_nested_managed_root(managed_dir, current) {
                return true;
            }
            let Some(parent) = current.parent() else {
                return false;
            };
            if parent == current || !parent.starts_with(managed_dir) {
                return false;
            }
            current = parent;
        }
    }

    fn path_has_agent_home_component(managed_dir: &Path, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(managed_dir) else {
            return false;
        };

        relative
            .components()
            .any(|component| component.as_os_str() == AGENT_HOME_DIR)
    }

    fn history_repo_dir(agent_home: &Path) -> PathBuf {
        agent_home.join(HISTORY_DIR).join(HISTORY_REPO_DIR)
    }

    fn supervisor_archive_root(supervisor_home: &Path) -> PathBuf {
        supervisor_home.join(ARCHIVES_DIR)
    }

    enum RemoveOutcome {
        Archived,
        Discarded,
        Missing,
    }

    fn stop_and_persist_registry(
        agent: RegisteredAgent,
        state: &SupervisorState,
    ) -> io::Result<()> {
        let stop_err = agent.stop().err();
        let registry_err = state.write_registry().err();
        if let Some(err) = registry_err.or(stop_err) {
            return Err(err);
        }
        Ok(())
    }

    fn format_remove_response(
        managed_dir: &Path,
        identity: &str,
        home_path: &Path,
        outcome: RemoveOutcome,
    ) -> String {
        let home_label = match outcome {
            RemoveOutcome::Archived => "archived_agent_home",
            RemoveOutcome::Discarded => "discarded_agent_home",
            RemoveOutcome::Missing => "missing_agent_home",
        };
        format!(
            "removed managed directory {}\nagent {}\n{home_label} {}\n",
            managed_dir.display(),
            identity.trim(),
            home_path.display()
        )
    }

    fn git_base_command(git_dir: &Path, work_tree: Option<&Path>) -> Command {
        let mut cmd = Command::new("git");
        cmd.arg("-c")
            .arg("safe.directory=*")
            .arg("-c")
            .arg("user.email=afs@localhost")
            .arg("-c")
            .arg("user.name=AFS")
            .arg("-c")
            .arg("init.defaultBranch=afs")
            .arg("-c")
            .arg("commit.gpgsign=false")
            .arg(format!("--git-dir={}", git_dir.display()));
        if let Some(work_tree) = work_tree {
            cmd.arg(format!("--work-tree={}", work_tree.display()));
        }
        cmd.env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_AUTHOR_NAME")
            .env_remove("GIT_AUTHOR_EMAIL")
            .env_remove("GIT_COMMITTER_NAME")
            .env_remove("GIT_COMMITTER_EMAIL");
        cmd
    }

    fn git_init_if_missing(git_dir: &Path) -> io::Result<()> {
        if git_dir.join("HEAD").exists() {
            return Ok(());
        }
        if let Some(parent) = git_dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let output = Command::new("git")
            .arg("-c")
            .arg("init.defaultBranch=afs")
            .arg("init")
            .arg("--bare")
            .arg("--quiet")
            .arg(git_dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "git init failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(())
    }

    fn git_has_commits(git_dir: &Path) -> io::Result<bool> {
        let output = git_base_command(git_dir, None)
            .arg("rev-parse")
            .arg("--verify")
            .arg("--quiet")
            .arg("HEAD")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()?;
        Ok(output.status.success())
    }

    struct GitCommitRequest<'a> {
        entry_id: &'a str,
        kind: &'a str,
        summary: &'a str,
        files: &'a [String],
        undoable: bool,
        undoes: Option<&'a str>,
        origin: Option<&'a str>,
    }

    fn git_stage_work_tree(
        git_dir: &Path,
        work_tree: &Path,
        nested_exclusions: &[String],
    ) -> io::Result<()> {
        git_init_if_missing(git_dir)?;
        let mut add = git_base_command(git_dir, Some(work_tree));
        add.arg("add")
            .arg("--all")
            .arg("--force")
            .arg("--")
            .arg(".")
            .arg(format!(":(exclude,top){AGENT_HOME_DIR}"));
        for nested in nested_exclusions {
            add.arg(format!(":(exclude,top){nested}"));
        }
        let output = add.stdout(Stdio::null()).stderr(Stdio::piped()).output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "git add failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(())
    }

    fn git_staged_changes_vs_head(git_dir: &Path) -> io::Result<Vec<String>> {
        if !git_has_commits(git_dir)? {
            return Ok(Vec::new());
        }
        let output = git_base_command(git_dir, None)
            .arg("diff")
            .arg("--cached")
            .arg("--name-only")
            .arg("HEAD")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "git diff failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let mut files: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        files.sort();
        Ok(files)
    }

    fn git_commit_index(
        git_dir: &Path,
        work_tree: &Path,
        request: &GitCommitRequest<'_>,
    ) -> io::Result<()> {
        let message = git_commit_message(request);
        let mut commit = git_base_command(git_dir, Some(work_tree));
        commit
            .arg("commit")
            .arg("--allow-empty")
            .arg("--allow-empty-message")
            .arg("--quiet")
            .arg("-F")
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = commit.spawn()?;
        {
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| io::Error::other("failed to capture git commit stdin"))?;
            stdin.write_all(message.as_bytes())?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "git commit failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(())
    }

    fn git_stage_and_commit(
        git_dir: &Path,
        work_tree: &Path,
        nested_exclusions: &[String],
        request: &GitCommitRequest<'_>,
    ) -> io::Result<()> {
        git_stage_work_tree(git_dir, work_tree, nested_exclusions)?;
        git_commit_index(git_dir, work_tree, request)
    }

    fn git_commit_message(request: &GitCommitRequest<'_>) -> String {
        let mut message = sanitize_field(request.summary);
        message.push_str("\n\n");
        message.push_str(&format!(
            "Afs-Entry-Id: {}\n",
            sanitize_field(request.entry_id)
        ));
        message.push_str(&format!("Afs-Kind: {}\n", sanitize_field(request.kind)));
        message.push_str(&format!(
            "Afs-Summary: {}\n",
            sanitize_field(request.summary)
        ));
        message.push_str(&format!(
            "Afs-Undoable: {}\n",
            undoable_field(request.undoable)
        ));
        message.push_str(&format!("Afs-File-Count: {}\n", request.files.len()));
        message.push_str(&format!(
            "Afs-Files: {}\n",
            sanitize_field(&request.files.join(", "))
        ));
        if let Some(undoes) = request.undoes {
            message.push_str(&format!("Afs-Undoes: {}\n", sanitize_field(undoes)));
        }
        if let Some(origin) = request.origin {
            message.push_str(&format!("Afs-Origin: {}\n", sanitize_field(origin)));
        }
        message
    }

    #[derive(Clone)]
    struct GitHistoryRecord {
        commit: String,
        timestamp_secs: u64,
        trailers: BTreeMap<String, String>,
    }

    fn git_log_records(git_dir: &Path) -> io::Result<Vec<GitHistoryRecord>> {
        if !git_has_commits(git_dir)? {
            return Ok(Vec::new());
        }
        let output = git_base_command(git_dir, None)
            .arg("log")
            .arg("--format=--AFS-COMMIT--%n%H%n%ct%n%B%n--AFS-END--")
            .arg("HEAD")
            .stderr(Stdio::piped())
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "git log failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let mut records = Vec::new();
        for block in text.split("--AFS-COMMIT--\n").skip(1) {
            let Some((sha_line, rest)) = block.split_once('\n') else {
                continue;
            };
            let Some((ts_line, body_with_end)) = rest.split_once('\n') else {
                continue;
            };
            let Some((body, _)) = body_with_end.rsplit_once("--AFS-END--") else {
                continue;
            };
            let commit = sha_line.trim().to_string();
            let timestamp_secs = ts_line.trim().parse().unwrap_or(0);
            let mut trailers = BTreeMap::new();
            for line in body.lines() {
                if let Some((key, value)) = line.split_once(": ")
                    && key.starts_with("Afs-")
                {
                    trailers.insert(key.to_string(), value.to_string());
                }
            }
            records.push(GitHistoryRecord {
                commit,
                timestamp_secs,
                trailers,
            });
        }
        Ok(records)
    }

    fn git_parent_commit(git_dir: &Path, commit: &str) -> io::Result<Option<String>> {
        let output = git_base_command(git_dir, None)
            .arg("rev-parse")
            .arg("--verify")
            .arg("--quiet")
            .arg(format!("{commit}^"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()?;
        if !output.status.success() {
            return Ok(None);
        }
        let parent = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if parent.is_empty() {
            Ok(None)
        } else {
            Ok(Some(parent))
        }
    }

    fn git_restore_tree(git_dir: &Path, work_tree: &Path, commit: &str) -> io::Result<()> {
        remove_managed_content(work_tree)?;
        let output = git_base_command(git_dir, Some(work_tree))
            .arg("checkout")
            .arg("--force")
            .arg(commit)
            .arg("--")
            .arg(".")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "git checkout failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
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

    pub fn remove(path: &Path, discard_history: bool) -> Result<String, Error> {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map_err(Error::Io)?.join(path)
        };
        let flag = if discard_history { "discard" } else { "keep" };
        send_request(&format!("REMOVE\t{}\t{flag}", path.display()))
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

    pub fn undo(path: &Path, history_entry: &str, confirmed: bool) -> Result<String, Error> {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map_err(Error::Io)?.join(path)
        };
        let confirmation = if confirmed { "yes" } else { "no" };
        send_request(&format!(
            "UNDO\t{}\t{}\t{}",
            path.display(),
            history_entry,
            confirmation
        ))
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

pub mod login {
    use std::io::{self, IsTerminal};
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    use crate::config::{AuthMethod, Config, Provider};
    use crate::supervisor;

    pub const ALLOW_NO_TTY_ENV: &str = "AFS_LOGIN_ALLOW_NO_TTY";

    #[derive(Debug)]
    pub enum Error {
        MissingProvider,
        UnsupportedProvider(String),
        NoTty,
        Io(io::Error),
        AgentRuntimeNotFound(PathBuf),
        VerificationFailed(String),
    }

    impl std::fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Error::MissingProvider => {
                    write!(f, "specify `--provider claude` or `--provider openai`")
                }
                Error::UnsupportedProvider(name) => write!(
                    f,
                    "provider `{name}` is not supported in this version; use `claude` or `openai`"
                ),
                Error::NoTty => write!(f, "`afs login` must be run in an interactive terminal"),
                Error::AgentRuntimeNotFound(path) => write!(
                    f,
                    "AFS agent runtime not found: {} (set AFS_PI_RUNTIME)",
                    path.display()
                ),
                Error::VerificationFailed(message) => write!(f, "{message}"),
                Error::Io(error) => write!(f, "{error}"),
            }
        }
    }

    impl From<io::Error> for Error {
        fn from(error: io::Error) -> Self {
            Error::Io(error)
        }
    }

    pub fn run(provider_arg: Option<&str>) -> Result<String, Error> {
        let provider = match provider_arg {
            None => return Err(Error::MissingProvider),
            Some(raw) => {
                Provider::parse(raw).ok_or_else(|| Error::UnsupportedProvider(raw.to_string()))?
            }
        };

        if !tty_ok() {
            return Err(Error::NoTty);
        }

        let home_dir = user_home()?;
        let runtime = supervisor::pi_runtime_command();
        let status = Command::new(&runtime)
            .arg("--provider")
            .arg(provider.as_cli_str())
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    Error::AgentRuntimeNotFound(runtime.clone())
                } else {
                    Error::Io(error)
                }
            })?
            .wait()?;

        verify_auth(&home_dir, provider, status.success())?;

        let afs_home = supervisor::home()?;
        let config = Config {
            provider,
            model: None,
            auth_method: AuthMethod::Oauth,
            api_key_env: None,
        };
        config.save(&afs_home)?;

        Ok(format!(
            "authenticated with {}. run `afs install <path>` to activate a managed directory.\n",
            provider.as_cli_str()
        ))
    }

    fn tty_ok() -> bool {
        if std::env::var_os(ALLOW_NO_TTY_ENV).is_some() {
            return true;
        }
        io::stdin().is_terminal() && io::stdout().is_terminal()
    }

    fn user_home() -> io::Result<PathBuf> {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))
    }

    fn verify_auth(
        home_dir: &std::path::Path,
        provider: Provider,
        exit_ok: bool,
    ) -> Result<(), Error> {
        let auth_path = home_dir.join(".pi").join("agent").join("auth.json");
        let body = std::fs::read_to_string(&auth_path).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => Error::VerificationFailed(
                "authentication did not complete; please try again".to_string(),
            ),
            _ => Error::Io(error),
        })?;
        let value: serde_json::Value = serde_json::from_str(&body).map_err(|_| {
            Error::VerificationFailed("authentication store is corrupted".to_string())
        })?;
        let entry = value.get(provider.auth_json_key()).ok_or_else(|| {
            Error::VerificationFailed(
                "authentication did not complete; please try again".to_string(),
            )
        })?;
        if entry.get("type").is_none() {
            return Err(Error::VerificationFailed(
                "authentication did not complete; please try again".to_string(),
            ));
        }
        if !exit_ok {
            return Err(Error::VerificationFailed(
                "authentication did not complete; please try again".to_string(),
            ));
        }
        Ok(())
    }
}

pub mod config {
    use std::io;
    use std::path::{Path, PathBuf};

    use serde::{Deserialize, Serialize};

    pub const CONFIG_FILE: &str = "config.json";

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum AuthMethod {
        Oauth,
        ApiKey,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Provider {
        Claude,
        Openai,
    }

    impl Provider {
        pub fn as_cli_str(&self) -> &'static str {
            match self {
                Provider::Claude => "claude",
                Provider::Openai => "openai",
            }
        }

        pub fn auth_json_key(&self) -> &'static str {
            match self {
                Provider::Claude => "anthropic",
                Provider::Openai => "openai",
            }
        }

        pub fn parse(input: &str) -> Option<Self> {
            match input {
                "claude" => Some(Provider::Claude),
                "openai" => Some(Provider::Openai),
                _ => None,
            }
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Config {
        pub provider: Provider,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub model: Option<String>,
        pub auth_method: AuthMethod,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub api_key_env: Option<String>,
    }

    impl Config {
        pub fn load(afs_home: &Path) -> io::Result<Option<Config>> {
            let path = config_path(afs_home);
            let body = match std::fs::read_to_string(&path) {
                Ok(body) => body,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            };
            let config: Config = serde_json::from_str(&body).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} is not valid AFS config: {error}", path.display()),
                )
            })?;
            config.validate().map_err(|message| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} is not valid AFS config: {message}", path.display()),
                )
            })?;
            Ok(Some(config))
        }

        pub fn save(&self, afs_home: &Path) -> io::Result<()> {
            self.validate()
                .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
            std::fs::create_dir_all(afs_home)?;
            let body = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
            std::fs::write(config_path(afs_home), body)
        }

        fn validate(&self) -> Result<(), String> {
            if matches!(self.auth_method, AuthMethod::ApiKey) && self.api_key_env.is_none() {
                return Err("auth_method \"api_key\" requires api_key_env".to_string());
            }
            Ok(())
        }
    }

    pub fn config_path(afs_home: &Path) -> PathBuf {
        afs_home.join(CONFIG_FILE)
    }
}
