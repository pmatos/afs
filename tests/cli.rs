use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use std::os::unix::fs::{FileTypeExt, PermissionsExt, symlink};
use std::os::unix::net::{UnixListener, UnixStream};

fn unique_afs_home(test_name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("afs-{test_name}-{}-{nanos}", std::process::id()))
}

fn supervisor_socket(afs_home: &std::path::Path) -> std::path::PathBuf {
    afs_home.join("supervisor.sock")
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn start_daemon(afs_home: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", afs_home)
        .arg("daemon")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("afs daemon should start")
}

fn write_default_config(afs_home: &std::path::Path) {
    std::fs::create_dir_all(afs_home).expect("test should create afs home");
    let path = afs_home.join("config.json");
    if path.exists() {
        return;
    }
    std::fs::write(&path, r#"{"provider":"claude","auth_method":"oauth"}"#)
        .expect("test should write default config");
}

fn write_config(afs_home: &std::path::Path, body: &str) {
    std::fs::create_dir_all(afs_home).expect("test should create afs home");
    std::fs::write(afs_home.join("config.json"), body).expect("test should write config");
}

fn fake_pi_login_runtime(test_name: &str, succeed: bool, provider: &str) -> std::path::PathBuf {
    let runtime_dir = unique_afs_home(test_name);
    std::fs::create_dir_all(&runtime_dir).expect("test should create fake login runtime directory");
    let runtime = runtime_dir.join("pi");
    let auth_key = match provider {
        "claude" => "anthropic",
        "openai" => "openai",
        "openai-codex" => "openai-codex",
        other => panic!("fake login runtime does not support provider {other}"),
    };
    let script = if succeed {
        format!(
            r#"#!/bin/sh
mkdir -p "$HOME/.pi/agent"
cat > "$HOME/.pi/agent/auth.json" <<'JSON'
{{"{key}":{{"type":"oauth","accessToken":"fake","refreshToken":"fake","expiresAt":9999999999999}}}}
JSON
exit 0
"#,
            key = auth_key
        )
    } else {
        String::from(
            r#"#!/bin/sh
exit 1
"#,
        )
    };
    std::fs::write(&runtime, script).expect("test should write fake login runtime");
    let mut permissions = std::fs::metadata(&runtime)
        .expect("test should read fake runtime metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&runtime, permissions)
        .expect("test should set fake runtime permissions");
    runtime
}

fn fake_pi_login_runtime_exits_zero_no_auth(test_name: &str) -> std::path::PathBuf {
    let runtime_dir = unique_afs_home(test_name);
    std::fs::create_dir_all(&runtime_dir).expect("test should create fake login runtime directory");
    let runtime = runtime_dir.join("pi");
    std::fs::write(&runtime, "#!/bin/sh\nexit 0\n").expect("test should write fake login runtime");
    let mut permissions = std::fs::metadata(&runtime)
        .expect("test should read fake runtime metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&runtime, permissions)
        .expect("test should set fake runtime permissions");
    runtime
}

fn run_afs_login(
    home_dir: &std::path::Path,
    afs_home: &std::path::Path,
    pi_runtime: &std::path::Path,
    extra_args: &[&str],
    allow_no_tty: bool,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_afs"));
    command
        .env("HOME", home_dir)
        .env("AFS_HOME", afs_home)
        .env("AFS_PI_RUNTIME", pi_runtime)
        .arg("login");
    if allow_no_tty {
        command.env("AFS_LOGIN_ALLOW_NO_TTY", "1");
    }
    for arg in extra_args {
        command.arg(arg);
    }
    command.output().expect("afs login should run")
}

fn start_daemon_with_pi_runtime(afs_home: &std::path::Path, pi_runtime: &std::path::Path) -> Child {
    write_default_config(afs_home);
    Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", afs_home)
        .env("AFS_PI_RUNTIME", pi_runtime)
        .arg("daemon")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("afs daemon should start")
}

fn start_daemon_with_pi_runtime_and_broadcast_timeout(
    afs_home: &std::path::Path,
    pi_runtime: &std::path::Path,
    timeout_ms: u64,
) -> Child {
    write_default_config(afs_home);
    Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", afs_home)
        .env("AFS_PI_RUNTIME", pi_runtime)
        .env("AFS_BROADCAST_REPLY_TIMEOUT_MS", timeout_ms.to_string())
        .arg("daemon")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("afs daemon should start")
}

fn start_daemon_with_index_warm_delay(
    afs_home: &std::path::Path,
    pi_runtime: &std::path::Path,
    delay_ms: u64,
) -> Child {
    write_default_config(afs_home);
    Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", afs_home)
        .env("AFS_PI_RUNTIME", pi_runtime)
        .env("AFS_INDEX_WARM_DELAY_MS", delay_ms.to_string())
        .arg("daemon")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("afs daemon should start")
}

fn start_daemon_with_reconciliation_delay(
    afs_home: &std::path::Path,
    pi_runtime: &std::path::Path,
    delay_ms: u64,
) -> Child {
    write_default_config(afs_home);
    Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", afs_home)
        .env("AFS_PI_RUNTIME", pi_runtime)
        .env("AFS_RECONCILIATION_DELAY_MS", delay_ms.to_string())
        .arg("daemon")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("afs daemon should start")
}

/// Path to the JSONL-conforming fake Pi binary built from
/// `src/bin/fake_pi.rs`. Tests pass this path to AFS via
/// `AFS_PI_RUNTIME`; per-test fixture files live under each agent's
/// `$AFS_AGENT_HOME` and drive the fake's reply behavior.
///
/// The `_test_name` parameter is kept for back-compat with the
/// previous shell-script fake; the binary is shared across all tests
/// because per-test state lives in the agent home, not the runtime.
fn fake_pi_runtime(_test_name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_fake_pi"))
}

#[allow(dead_code)]
fn write_relevance_reply(
    agent_home: &std::path::Path,
    relevance: &str,
    reason: &str,
    answer: &str,
    file_references: &[&str],
) {
    let body = serde_json::json!({
        "schema_version": 1,
        "relevance": relevance,
        "reason": reason,
        "answer": answer,
        "file_references": file_references,
        "changed_files": [],
        "history_entries": [],
        "delegates": [],
    });
    std::fs::write(
        agent_home.join("broadcast-response.json"),
        serde_json::to_string(&body).expect("relevance reply serializes"),
    )
    .expect("write broadcast-response.json");
}

#[allow(dead_code)]
fn write_collaborate_reply(
    agent_home: &std::path::Path,
    answer: &str,
    changed_files: &[&str],
    history_entries: &[&str],
) {
    let body = serde_json::json!({
        "schema_version": 1,
        "relevance": "strong",
        "reason": "fake collab",
        "answer": answer,
        "file_references": [],
        "changed_files": changed_files,
        "history_entries": history_entries,
        "delegates": [],
    });
    std::fs::write(
        agent_home.join("collaborate-response.json"),
        serde_json::to_string(&body).expect("collaborate reply serializes"),
    )
    .expect("write collaborate-response.json");
}

#[allow(dead_code)]
fn write_collaborate_template_reply(agent_home: &std::path::Path, answer_template: &str) {
    // The template uses __PEER_ANSWER__ which the fake does not yet
    // substitute (Pi has no streaming peer-answer semantics); tests
    // that previously relied on the substitution should write the
    // expected literal answer here.
    let body = serde_json::json!({
        "schema_version": 1,
        "relevance": "strong",
        "reason": "fake collab template",
        "answer": answer_template,
        "file_references": [],
        "changed_files": [],
        "history_entries": [],
        "delegates": [],
    });
    std::fs::write(
        agent_home.join("collaborate-response-template.json"),
        serde_json::to_string(&body).expect("template reply serializes"),
    )
    .expect("write collaborate-response-template.json");
}

#[allow(dead_code)]
fn write_task_reply(
    agent_home: &std::path::Path,
    answer: &str,
    changed_files: &[&str],
    history_entries: &[&str],
) {
    let body = serde_json::json!({
        "schema_version": 1,
        "relevance": "strong",
        "reason": "fake task",
        "answer": answer,
        "file_references": [],
        "changed_files": changed_files,
        "history_entries": history_entries,
        "delegates": [],
    });
    std::fs::write(
        agent_home.join("task-response.json"),
        serde_json::to_string(&body).expect("task reply serializes"),
    )
    .expect("write task-response.json");
}

fn stop_daemon(daemon: &mut Child) {
    if daemon
        .try_wait()
        .expect("daemon status should be readable")
        .is_none()
    {
        daemon.kill().expect("daemon should stop on test cleanup");
    }
    daemon.wait().expect("daemon cleanup should finish");
}

fn afs_history(afs_home: &std::path::Path, managed_dir: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", afs_home)
        .arg("history")
        .arg(managed_dir)
        .output()
        .expect("afs history should run")
}

fn history_entry_id(history_line: &str) -> &str {
    history_line
        .split_whitespace()
        .find_map(|field| field.strip_prefix("entry="))
        .expect("history output should include an entry id")
}

fn afs_ask(afs_home: &std::path::Path, prompt: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", afs_home)
        .arg("ask")
        .arg(prompt)
        .output()
        .expect("afs ask should run")
}

fn afs_ask_streamed(afs_home: &std::path::Path, prompt: &str) -> Vec<(Duration, String)> {
    use std::io::{BufRead, BufReader};

    let start = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", afs_home)
        .arg("ask")
        .arg(prompt)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("afs ask should spawn");

    let stdout = child.stdout.take().expect("afs ask should expose stdout");
    let reader = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut lines = Vec::new();
        let mut buffer = String::new();
        loop {
            buffer.clear();
            let n = reader
                .read_line(&mut buffer)
                .expect("stdout read should succeed");
            if n == 0 {
                break;
            }
            let trimmed = buffer.trim_end_matches(['\n', '\r']).to_string();
            lines.push((start.elapsed(), trimmed));
        }
        lines
    });

    let _ = child.wait().expect("afs ask should exit");
    reader.join().expect("stdout reader thread should join")
}

struct SpawnedAsk {
    child: Child,
    reader: std::thread::JoinHandle<Vec<(Duration, String)>>,
}

impl SpawnedAsk {
    fn finish(mut self) -> Vec<(Duration, String)> {
        let status = self.child.wait().expect("afs ask should exit");
        let lines = self
            .reader
            .join()
            .expect("stdout reader thread should join");
        assert!(
            status.success(),
            "afs ask should succeed; stdout:\n{lines:#?}"
        );
        lines
    }

    fn finish_with_timeout(mut self, timeout: Duration) -> Vec<(Duration, String)> {
        let start = Instant::now();
        let mut status = None;
        while start.elapsed() < timeout {
            status = self
                .child
                .try_wait()
                .expect("afs ask status should be readable");
            if status.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        if status.is_none() {
            self.child.kill().expect("hung afs ask should be killed");
            let _ = self.child.wait().expect("killed afs ask should exit");
        }

        let lines = self
            .reader
            .join()
            .expect("stdout reader thread should join");
        let status = status.expect("afs ask should exit before timeout");
        assert!(
            status.success(),
            "afs ask should succeed; stdout:\n{lines:#?}"
        );
        lines
    }
}

fn spawn_afs_ask_streamed(afs_home: &std::path::Path, prompt: &str) -> SpawnedAsk {
    use std::io::{BufRead, BufReader};

    let start = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", afs_home)
        .arg("ask")
        .arg(prompt)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("afs ask should spawn");

    let stdout = child.stdout.take().expect("afs ask should expose stdout");
    let reader = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut lines = Vec::new();
        let mut buffer = String::new();
        loop {
            buffer.clear();
            let n = reader
                .read_line(&mut buffer)
                .expect("stdout read should succeed");
            if n == 0 {
                break;
            }
            let trimmed = buffer.trim_end_matches(['\n', '\r']).to_string();
            lines.push((start.elapsed(), trimmed));
        }
        lines
    });

    SpawnedAsk { child, reader }
}

fn install_managed_dir(
    afs_home: &std::path::Path,
    managed_dir: &std::path::Path,
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", afs_home)
        .arg("install")
        .arg(managed_dir)
        .output()
        .expect("afs install should run")
}

fn remove_managed_dir(
    afs_home: &std::path::Path,
    managed_dir: &std::path::Path,
) -> std::process::Output {
    remove_managed_dir_with_flags(afs_home, managed_dir, &[])
}

fn remove_managed_dir_with_flags(
    afs_home: &std::path::Path,
    managed_dir: &std::path::Path,
    flags: &[&str],
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", afs_home)
        .arg("remove")
        .arg(managed_dir)
        .args(flags)
        .output()
        .expect("afs remove should run")
}

#[test]
fn ask_reports_daemon_not_running_when_supervisor_socket_is_unavailable() {
    let afs_home = unique_afs_home("ask-no-daemon");

    let output = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .args(["ask", "hello"])
        .output()
        .expect("afs ask should run");

    assert!(
        !output.status.success(),
        "afs ask should fail when no daemon is running"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "daemon is not running\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
}

#[test]
fn daemon_creates_supervisor_home_and_owns_socket_in_foreground() {
    let afs_home = unique_afs_home("daemon-startup");
    let socket_path = supervisor_socket(&afs_home);

    let mut daemon = start_daemon(&afs_home);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "afs daemon should create a Unix supervisor socket"
    );
    assert!(
        afs_home.is_dir(),
        "afs daemon should create the Supervisor Home"
    );
    assert!(
        daemon
            .try_wait()
            .expect("daemon status should be readable")
            .is_none(),
        "afs daemon should keep running in the foreground"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn undo_external_change_requires_yes_in_scripted_use() {
    let afs_home = unique_afs_home("undo-external-requires-yes");
    let managed_dir = unique_afs_home("managed-undo-external-requires-yes");
    let pi_runtime = fake_pi_runtime("undo-external-requires-yes-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let target_file = managed_dir.join("notes.txt");
    std::fs::write(&target_file, "before\n").expect("test should create managed file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let target_file = target_file
        .canonicalize()
        .expect("target file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("install")
        .arg(&managed_dir)
        .output()
        .expect("afs install should run");
    assert!(install.status.success(), "afs install should succeed");

    std::fs::write(&target_file, "after\n").expect("test should modify managed file");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let history = afs_history(&afs_home, &managed_dir);
            history.status.success()
                && String::from_utf8_lossy(&history.stdout).contains("type=external")
        }),
        "afs history should show the External Change"
    );

    let history = afs_history(&afs_home, &managed_dir);
    let stdout = String::from_utf8_lossy(&history.stdout);
    let entry = history_entry_id(stdout.lines().next().expect("history should have an entry"));
    let undo = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("undo")
        .arg(&managed_dir)
        .arg(entry)
        .output()
        .expect("afs undo should run");

    assert!(
        !undo.status.success(),
        "scripted undo of an External Change should require --yes"
    );
    assert_eq!(String::from_utf8_lossy(&undo.stdout), "");
    assert!(
        String::from_utf8_lossy(&undo.stderr).contains("requires --yes"),
        "afs undo should explain the scripted confirmation requirement"
    );
    assert_eq!(
        std::fs::read_to_string(&target_file).expect("target file should be readable"),
        "after\n",
        "failed undo should leave the filesystem unchanged"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn broad_ask_uses_configured_timeout_and_ignores_late_replies() {
    let afs_home = unique_afs_home("ask-broadcast-timeout");
    let fast_dir = unique_afs_home("ask-broadcast-fast");
    let slow_dir = unique_afs_home("ask-broadcast-slow");
    let pi_runtime = fake_pi_runtime("ask-broadcast-timeout-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&fast_dir).expect("test should create fast managed directory");
    std::fs::create_dir_all(&slow_dir).expect("test should create slow managed directory");
    let fast_file = fast_dir.join("workout.md");
    let slow_file = slow_dir.join("sleep.md");
    std::fs::write(&fast_file, "run workout\n").expect("test should create fast reference");
    std::fs::write(&slow_file, "late context\n").expect("test should create slow reference");
    let fast_dir = fast_dir
        .canonicalize()
        .expect("fast directory should canonicalize");
    let slow_dir = slow_dir
        .canonicalize()
        .expect("slow directory should canonicalize");
    let fast_file = fast_file
        .canonicalize()
        .expect("fast reference should canonicalize");
    let slow_file = slow_file
        .canonicalize()
        .expect("slow reference should canonicalize");
    let mut daemon =
        start_daemon_with_pi_runtime_and_broadcast_timeout(&afs_home, &pi_runtime, 100);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let fast_install = install_managed_dir(&afs_home, &fast_dir);
    assert!(
        fast_install.status.success(),
        "fast afs install should succeed"
    );
    let slow_install = install_managed_dir(&afs_home, &slow_dir);
    assert!(
        slow_install.status.success(),
        "slow afs install should succeed"
    );
    let fast_identity =
        std::fs::read_to_string(fast_dir.join(".afs/identity")).expect("fast identity exists");
    let slow_identity =
        std::fs::read_to_string(slow_dir.join(".afs/identity")).expect("slow identity exists");
    std::fs::write(
        fast_dir.join(".afs/broadcast-response"),
        format!(
            "possible\tworkout plan is here\tUse the fast workout context\t{}\n",
            fast_file.display()
        ),
    )
    .expect("test should configure fast broadcast response");
    std::fs::write(slow_dir.join(".afs/broadcast-delay-seconds"), "1")
        .expect("test should configure slow broadcast delay");
    std::fs::write(
        slow_dir.join(".afs/broadcast-response"),
        format!(
            "strong\tsleep context is here\tThis slow reply missed the timeout\t{}\n",
            slow_file.display()
        ),
    )
    .expect("test should configure slow broadcast response");

    let started = Instant::now();
    let ask = afs_ask(&afs_home, "what is the run workout today");
    let elapsed = started.elapsed();

    assert!(
        ask.status.success(),
        "afs ask should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&ask.stdout),
        String::from_utf8_lossy(&ask.stderr)
    );
    assert!(
        elapsed < Duration::from_millis(900),
        "afs ask should return after the configured broadcast timeout"
    );
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains("Use the fast workout context"),
        "afs ask should keep on-time replies"
    );
    assert!(
        stdout.contains(&format!("participating_agents: {}", fast_identity.trim())),
        "afs ask should report the on-time participant"
    );
    assert!(
        stdout.contains(&format!("- {}", fast_file.display())),
        "afs ask should include on-time File References"
    );
    assert!(
        !stdout.contains(slow_identity.trim()),
        "late agents should not be reported as participants"
    );
    assert!(
        !stdout.contains(&slow_file.display().to_string()),
        "late File References should not be included"
    );
    assert!(
        stdout.contains("broadcast_timeout_ms: 100"),
        "afs ask should report the configured broadcast timeout"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn undo_latest_external_change_with_yes_restores_file_and_records_reversal() {
    let afs_home = unique_afs_home("undo-external-with-yes");
    let managed_dir = unique_afs_home("managed-undo-external-with-yes");
    let pi_runtime = fake_pi_runtime("undo-external-with-yes-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let target_file = managed_dir.join("notes.txt");
    std::fs::write(&target_file, "before\n").expect("test should create managed file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let target_file = target_file
        .canonicalize()
        .expect("target file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("install")
        .arg(&managed_dir)
        .output()
        .expect("afs install should run");
    assert!(install.status.success(), "afs install should succeed");

    std::fs::write(&target_file, "after\n").expect("test should modify managed file");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let history = afs_history(&afs_home, &managed_dir);
            history.status.success()
                && String::from_utf8_lossy(&history.stdout).contains("type=external")
        }),
        "afs history should show the External Change"
    );

    let history = afs_history(&afs_home, &managed_dir);
    let stdout = String::from_utf8_lossy(&history.stdout);
    let entry = history_entry_id(stdout.lines().next().expect("history should have an entry"));
    let undo = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("undo")
        .arg(&managed_dir)
        .arg(entry)
        .arg("--yes")
        .output()
        .expect("afs undo should run");

    assert!(undo.status.success(), "afs undo --yes should succeed");
    assert_eq!(String::from_utf8_lossy(&undo.stderr), "");
    assert!(
        String::from_utf8_lossy(&undo.stdout).contains(&format!("undid history entry {entry}")),
        "afs undo should report the undone History Entry"
    );
    assert_eq!(
        std::fs::read_to_string(&target_file).expect("target file should be readable"),
        "before\n",
        "undo should restore the previous file contents"
    );

    assert!(
        wait_until(Duration::from_secs(3), || {
            let history = afs_history(&afs_home, &managed_dir);
            history.status.success()
                && String::from_utf8_lossy(&history.stdout).contains("type=undo")
        }),
        "afs history should show the Undo History Entry"
    );
    let history = afs_history(&afs_home, &managed_dir);
    assert!(history.status.success(), "afs history should succeed");
    let stdout = String::from_utf8_lossy(&history.stdout);
    let entries = stdout.lines().collect::<Vec<_>>();
    assert!(
        entries[0].contains("type=undo"),
        "undo should be the newest History Entry"
    );
    assert!(
        entries[0].contains(&format!("summary=Undo {entry}: External change: notes.txt")),
        "undo history should name the reversed entry"
    );
    assert!(
        entries[0].contains("undoable=no"),
        "undo entries should not be undoable"
    );
    assert!(
        entries
            .iter()
            .any(|line| line.contains(&format!("entry={entry}"))
                && line.contains("type=external")
                && line.contains("undoable=no")),
        "the reversed External Change should no longer be undoable"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn undo_rejects_non_latest_history_entry_without_changing_files() {
    let afs_home = unique_afs_home("undo-non-latest");
    let managed_dir = unique_afs_home("managed-undo-non-latest");
    let pi_runtime = fake_pi_runtime("undo-non-latest-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let first_file = managed_dir.join("first.txt");
    let second_file = managed_dir.join("second.txt");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("install")
        .arg(&managed_dir)
        .output()
        .expect("afs install should run");
    assert!(install.status.success(), "afs install should succeed");

    std::fs::write(&first_file, "first\n").expect("test should create first changed file");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let history = afs_history(&afs_home, &managed_dir);
            history.status.success()
                && String::from_utf8_lossy(&history.stdout).contains("first.txt")
        }),
        "first External Change should reach history"
    );

    std::fs::write(&second_file, "second\n").expect("test should create second changed file");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let history = afs_history(&afs_home, &managed_dir);
            let stdout = String::from_utf8_lossy(&history.stdout);
            history.status.success()
                && stdout.lines().count() == 2
                && stdout
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .contains("second.txt")
        }),
        "second External Change should become the newest history entry"
    );

    let history = afs_history(&afs_home, &managed_dir);
    let stdout = String::from_utf8_lossy(&history.stdout);
    let entries = stdout.lines().collect::<Vec<_>>();
    let older_entry = history_entry_id(entries[1]);
    let undo = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("undo")
        .arg(&managed_dir)
        .arg(older_entry)
        .arg("--yes")
        .output()
        .expect("afs undo should run");

    assert!(
        !undo.status.success(),
        "afs undo should reject non-latest History Entries"
    );
    assert_eq!(String::from_utf8_lossy(&undo.stdout), "");
    assert!(
        String::from_utf8_lossy(&undo.stderr).contains("only the latest undoable"),
        "afs undo should explain the latest-only restriction"
    );
    assert_eq!(
        std::fs::read_to_string(&first_file).expect("first file should be readable"),
        "first\n",
        "rejected undo should leave the older file as-is"
    );
    assert_eq!(
        std::fs::read_to_string(&second_file).expect("second file should be readable"),
        "second\n",
        "rejected undo should leave the latest file as-is"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn starting_second_daemon_fails_cleanly_when_first_daemon_is_live() {
    let afs_home = unique_afs_home("duplicate-daemon");
    let socket_path = supervisor_socket(&afs_home);
    let mut first_daemon = start_daemon(&afs_home);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "first daemon should create a Unix supervisor socket"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("daemon")
        .output()
        .expect("second afs daemon should run and fail");

    assert!(
        !output.status.success(),
        "second afs daemon should fail while the first daemon is live"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "supervisor daemon already running\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");

    stop_daemon(&mut first_daemon);
}

#[test]
fn daemon_replaces_stale_supervisor_socket_when_no_daemon_is_live() {
    let afs_home = unique_afs_home("stale-socket");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&afs_home).expect("test should create Supervisor Home");

    let stale_listener = UnixListener::bind(&socket_path).expect("test should create stale socket");
    drop(stale_listener);
    assert!(
        wait_until(Duration::from_secs(2), || UnixStream::connect(&socket_path)
            .is_err()),
        "test setup should leave a stale, unowned socket path"
    );

    let mut daemon = start_daemon(&afs_home);

    assert!(
        wait_until(Duration::from_secs(2), || UnixStream::connect(&socket_path)
            .is_ok()),
        "afs daemon should replace a stale socket with a live Supervisor Socket"
    );
    assert!(
        daemon
            .try_wait()
            .expect("daemon status should be readable")
            .is_none(),
        "afs daemon should keep running after replacing a stale socket"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn ask_connects_to_live_daemon_and_reports_no_broadcast_participants() {
    let afs_home = unique_afs_home("ask-live-daemon");
    let socket_path = supervisor_socket(&afs_home);
    let mut daemon = start_daemon(&afs_home);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before ask connects"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .args(["ask", "hello"])
        .output()
        .expect("afs ask should run");

    assert!(
        output.status.success(),
        "afs ask should reach the live daemon"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("no relevant agents replied before broadcast timeout"),
        "afs ask should explain that no broadcast replies were available"
    );
    assert!(
        stdout.contains("participating_agents: none"),
        "afs ask should report no Conversation Participants"
    );
    assert!(
        stdout.contains("references:\n- none"),
        "afs ask should report no File References"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");

    stop_daemon(&mut daemon);
}

#[test]
fn broad_ask_broadcasts_to_registered_agents_and_reports_relevant_references() {
    let afs_home = unique_afs_home("ask-broadcast-relevant");
    let health_dir = unique_afs_home("ask-broadcast-health");
    let recipes_dir = unique_afs_home("ask-broadcast-recipes");
    let pi_runtime = fake_pi_runtime("ask-broadcast-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&health_dir).expect("test should create health managed directory");
    std::fs::create_dir_all(&recipes_dir).expect("test should create recipes managed directory");
    let lab_file = health_dir.join("labs-2025.pdf");
    std::fs::write(&lab_file, "blood panel\n").expect("test should create referenced file");
    let health_dir = health_dir
        .canonicalize()
        .expect("health directory should canonicalize");
    let recipes_dir = recipes_dir
        .canonicalize()
        .expect("recipes directory should canonicalize");
    let lab_file = lab_file
        .canonicalize()
        .expect("referenced file should canonicalize");
    let mut daemon =
        start_daemon_with_pi_runtime_and_broadcast_timeout(&afs_home, &pi_runtime, 100);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let health_install = install_managed_dir(&afs_home, &health_dir);
    assert!(
        health_install.status.success(),
        "health afs install should succeed"
    );
    let recipes_install = install_managed_dir(&afs_home, &recipes_dir);
    assert!(
        recipes_install.status.success(),
        "recipes afs install should succeed"
    );
    let health_identity =
        std::fs::read_to_string(health_dir.join(".afs/identity")).expect("health identity exists");
    let recipes_identity = std::fs::read_to_string(recipes_dir.join(".afs/identity"))
        .expect("recipes identity exists");
    std::fs::write(
        health_dir.join(".afs/broadcast-response"),
        format!(
            "strong\tblood tests are in this managed directory\tFound the 2025 blood panel\t{}\n",
            lab_file.display()
        ),
    )
    .expect("test should configure relevant broadcast response");

    let ask = afs_ask(&afs_home, "find my last blood tests from 2025");

    assert!(
        ask.status.success(),
        "afs ask should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&ask.stdout),
        String::from_utf8_lossy(&ask.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&ask.stderr), "");
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains("Found the 2025 blood panel"),
        "afs ask should include the relevant agent answer"
    );
    assert!(
        stdout.contains(&format!("- {}", lab_file.display())),
        "afs ask should include returned File References"
    );
    assert!(
        stdout.contains(&format!("participating_agents: {}", health_identity.trim())),
        "afs ask should name the Directory Agent that participated"
    );
    assert!(
        !stdout.contains(recipes_identity.trim()),
        "silent irrelevant agents should not be reported as participants"
    );
    assert!(
        std::fs::read_to_string(health_dir.join(".afs/broadcast-received"))
            .expect("health agent should receive broadcast")
            .contains("find my last blood tests from 2025"),
        "relevant agent should receive the Broadcast Request"
    );
    assert!(
        std::fs::read_to_string(recipes_dir.join(".afs/broadcast-received"))
            .expect("recipes agent should receive broadcast")
            .contains("find my last blood tests from 2025"),
        "silent agent should still receive the Broadcast Request"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn broadcast_relevant_agents_collaborate_and_use_consulted_reply_in_final_synthesis() {
    let afs_home = unique_afs_home("ask-collab-criterion2");
    let recipes_dir = unique_afs_home("ask-collab-recipes");
    let workouts_dir = unique_afs_home("ask-collab-workouts");
    let pi_runtime = fake_pi_runtime("ask-collab-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&recipes_dir).expect("test should create recipes managed directory");
    std::fs::create_dir_all(&workouts_dir).expect("test should create workouts managed directory");
    let recipes_dir = recipes_dir
        .canonicalize()
        .expect("recipes directory should canonicalize");
    let workouts_dir = workouts_dir
        .canonicalize()
        .expect("workouts directory should canonicalize");
    let mut daemon =
        start_daemon_with_pi_runtime_and_broadcast_timeout(&afs_home, &pi_runtime, 200);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let recipes_install = install_managed_dir(&afs_home, &recipes_dir);
    assert!(
        recipes_install.status.success(),
        "recipes afs install should succeed"
    );
    let workouts_install = install_managed_dir(&afs_home, &workouts_dir);
    assert!(
        workouts_install.status.success(),
        "workouts afs install should succeed"
    );

    let recipes_identity = std::fs::read_to_string(recipes_dir.join(".afs/identity"))
        .expect("recipes identity exists");
    let workouts_identity = std::fs::read_to_string(workouts_dir.join(".afs/identity"))
        .expect("workouts identity exists");

    std::fs::write(
        recipes_dir.join(".afs/broadcast-response"),
        "strong\trecipes touch food and fitness\trecipes thinks the answer involves workouts\t\n",
    )
    .expect("test should configure recipes broadcast response");
    std::fs::write(
        workouts_dir.join(".afs/broadcast-response"),
        "strong\tworkouts indexes training plans\tworkouts has the daily routine\t\n",
    )
    .expect("test should configure workouts broadcast response");

    std::fs::write(
        recipes_dir.join(".afs/collaborate-delegate-target"),
        workouts_identity.trim(),
    )
    .expect("test should configure recipes delegate target");
    std::fs::write(
        recipes_dir.join(".afs/collaborate-response-template"),
        "COLLABORATE_REPLY\trecipes incorporated peer note: __PEER_ANSWER__\tnone\tnone\n",
    )
    .expect("test should configure recipes collaborate template");

    std::fs::write(
        workouts_dir.join(".afs/task-response"),
        "workouts says check workouts/run.md",
    )
    .expect("test should configure workouts task reply");

    let ask = afs_ask(&afs_home, "where are my recent fitness records?");

    assert!(
        ask.status.success(),
        "afs ask should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&ask.stdout),
        String::from_utf8_lossy(&ask.stderr)
    );
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains("recipes thinks the answer involves workouts"),
        "broadcast answers block should include recipes reply\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("workouts has the daily routine"),
        "broadcast answers block should include workouts reply\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("collaboration:"),
        "ask output should include the collaboration block\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("recipes incorporated peer note: workouts says check workouts/run.md"),
        "consulter's refined answer must contain the consultee's reply text\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains(recipes_identity.trim()),
        "participating_agents should list recipes\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains(workouts_identity.trim()),
        "participating_agents should list workouts\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("changed_files: none"),
        "no files were modified in this scenario\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("history_entries: none"),
        "no history entries were created in this scenario\nstdout:\n{stdout}"
    );
    assert!(
        recipes_dir.join(".afs/collaborate-received").is_file(),
        "recipes agent should have received a COLLABORATE message"
    );
    let delegated_log =
        std::fs::read_to_string(recipes_dir.join(".afs/collaborate-delegated-reply"))
            .expect("recipes agent should have received the delegated reply envelope");
    assert!(
        delegated_log.contains("answer=workouts says check workouts/run.md"),
        "supervisor must deliver the consultee's reply back to the consulter\nlog:\n{delegated_log}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn broadcast_collaboration_delegations_record_change_reports_in_history() {
    let afs_home = unique_afs_home("ask-collab-changes");
    let source_dir = unique_afs_home("ask-collab-changes-source");
    let peer_dir = unique_afs_home("ask-collab-changes-peer");
    let pi_runtime = fake_pi_runtime("ask-collab-changes-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&source_dir).expect("test should create source managed directory");
    std::fs::create_dir_all(&peer_dir).expect("test should create peer managed directory");
    let peer_file = peer_dir.join("handoff.md");
    std::fs::write(&peer_file, "before\n").expect("test should create peer file");
    let source_dir = source_dir
        .canonicalize()
        .expect("source directory should canonicalize");
    let peer_dir = peer_dir
        .canonicalize()
        .expect("peer directory should canonicalize");
    let peer_file = peer_file
        .canonicalize()
        .expect("peer file should canonicalize");
    let mut daemon =
        start_daemon_with_pi_runtime_and_broadcast_timeout(&afs_home, &pi_runtime, 200);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let source_install = install_managed_dir(&afs_home, &source_dir);
    assert!(
        source_install.status.success(),
        "source afs install should succeed"
    );
    let peer_install = install_managed_dir(&afs_home, &peer_dir);
    assert!(
        peer_install.status.success(),
        "peer afs install should succeed"
    );
    let peer_identity =
        std::fs::read_to_string(peer_dir.join(".afs/identity")).expect("peer identity exists");

    std::fs::write(
        source_dir.join(".afs/broadcast-response"),
        "strong\tsource owns coordination\tsource thinks peer should update handoff\t\n",
    )
    .expect("test should configure source broadcast response");
    std::fs::write(
        peer_dir.join(".afs/broadcast-response"),
        "possible\tpeer holds the handoff file\tpeer can update its handoff\t\n",
    )
    .expect("test should configure peer broadcast response");

    std::fs::write(
        source_dir.join(".afs/collaborate-delegate-target"),
        peer_identity.trim(),
    )
    .expect("test should configure source delegate target");
    std::fs::write(
        source_dir.join(".afs/collaborate-response-template"),
        "COLLABORATE_REPLY\tsource asked peer to update handoff\tnone\tnone\n",
    )
    .expect("test should configure source collaborate template");

    std::fs::write(peer_dir.join(".afs/task-write-file"), "handoff.md")
        .expect("test should configure peer write path");
    std::fs::write(
        peer_dir.join(".afs/task-write-content"),
        "after collaboration",
    )
    .expect("test should configure peer write content");
    std::fs::write(peer_dir.join(".afs/task-response"), "peer wrote handoff.md")
        .expect("test should configure peer task response");

    let ask = afs_ask(&afs_home, "coordinate the next handoff");

    assert!(
        ask.status.success(),
        "afs ask should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&ask.stdout),
        String::from_utf8_lossy(&ask.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&ask.stderr), "");
    assert_eq!(
        std::fs::read_to_string(&peer_file).expect("peer file should be readable"),
        "after collaboration\n",
        "consulted task should modify the peer Managed Subtree"
    );
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains("changed_files: handoff.md"),
        "afs ask should aggregate files changed by collaboration delegations\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("history_entries: history-"),
        "afs ask should report the resulting Agent Change History Entry\nstdout:\n{stdout}"
    );
    assert!(
        wait_until(Duration::from_secs(3), || {
            let history = afs_history(&afs_home, &peer_dir);
            let history_stdout = String::from_utf8_lossy(&history.stdout);
            history.status.success()
                && history_stdout.contains("type=agent")
                && history_stdout.contains("summary=Agent change: handoff.md")
        }),
        "consulted file modification should be recorded as an Agent Change"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn broadcast_collaboration_bounds_hung_consultee_with_per_call_timeout() {
    let afs_home = unique_afs_home("ask-collab-hung");
    let source_dir = unique_afs_home("ask-collab-hung-source");
    let peer_dir = unique_afs_home("ask-collab-hung-peer");
    let pi_runtime = fake_pi_runtime("ask-collab-hung-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&source_dir).expect("test should create source managed directory");
    std::fs::create_dir_all(&peer_dir).expect("test should create peer managed directory");
    let source_dir = source_dir
        .canonicalize()
        .expect("source directory should canonicalize");
    let peer_dir = peer_dir
        .canonicalize()
        .expect("peer directory should canonicalize");
    let mut daemon =
        start_daemon_with_pi_runtime_and_broadcast_timeout(&afs_home, &pi_runtime, 100);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let source_install = install_managed_dir(&afs_home, &source_dir);
    assert!(
        source_install.status.success(),
        "source afs install should succeed"
    );
    let peer_install = install_managed_dir(&afs_home, &peer_dir);
    assert!(
        peer_install.status.success(),
        "peer afs install should succeed"
    );
    let peer_identity =
        std::fs::read_to_string(peer_dir.join(".afs/identity")).expect("peer identity exists");

    std::fs::write(
        source_dir.join(".afs/broadcast-response"),
        "strong\tsource is relevant\tsource will delegate to peer\t\n",
    )
    .expect("test should configure source broadcast response");
    std::fs::write(
        peer_dir.join(".afs/broadcast-response"),
        "strong\tpeer is relevant\tpeer has data\t\n",
    )
    .expect("test should configure peer broadcast response");

    std::fs::write(
        source_dir.join(".afs/collaborate-delegate-target"),
        peer_identity.trim(),
    )
    .expect("test should configure source delegate target");
    // Peer hangs for far longer than the per-call deadline (100ms broadcast timeout).
    std::fs::write(peer_dir.join(".afs/task-delay-seconds"), "3")
        .expect("test should configure peer task delay");

    let started = Instant::now();
    let ask = afs_ask(&afs_home, "broad question that triggers collaboration");
    let elapsed = started.elapsed();

    assert!(
        ask.status.success(),
        "afs ask should still succeed when a consultee hangs\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&ask.stdout),
        String::from_utf8_lossy(&ask.stderr)
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "afs ask must return well before the 3s consultee delay; took {elapsed:?}"
    );
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains("progress: collaboration delegation timeout"),
        "ask output should report the per-call delegation timeout\nstdout:\n{stdout}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn broadcast_with_single_relevant_reply_skips_collaboration_phase() {
    let afs_home = unique_afs_home("ask-collab-skip");
    let lone_dir = unique_afs_home("ask-collab-skip-lone");
    let silent_dir = unique_afs_home("ask-collab-skip-silent");
    let pi_runtime = fake_pi_runtime("ask-collab-skip-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&lone_dir).expect("test should create lone managed directory");
    std::fs::create_dir_all(&silent_dir).expect("test should create silent managed directory");
    let lone_dir = lone_dir
        .canonicalize()
        .expect("lone directory should canonicalize");
    let silent_dir = silent_dir
        .canonicalize()
        .expect("silent directory should canonicalize");
    let mut daemon =
        start_daemon_with_pi_runtime_and_broadcast_timeout(&afs_home, &pi_runtime, 200);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let lone_install = install_managed_dir(&afs_home, &lone_dir);
    assert!(
        lone_install.status.success(),
        "lone afs install should succeed"
    );
    let silent_install = install_managed_dir(&afs_home, &silent_dir);
    assert!(
        silent_install.status.success(),
        "silent afs install should succeed"
    );
    let silent_identity =
        std::fs::read_to_string(silent_dir.join(".afs/identity")).expect("silent identity exists");

    std::fs::write(
        lone_dir.join(".afs/broadcast-response"),
        "strong\tlone is the only relevant agent\tlone has the answer\t\n",
    )
    .expect("test should configure lone broadcast response");
    // silent_dir replies with relevance=none so the supervisor discards it,
    // leaving exactly one relevant reply to consider.
    std::fs::write(
        silent_dir.join(".afs/broadcast-response"),
        "none\tnot relevant\t\t\n",
    )
    .expect("test should configure silent non-relevant response");

    // If collaboration ran on lone, this delegate target would be exercised.
    std::fs::write(
        lone_dir.join(".afs/collaborate-delegate-target"),
        silent_identity.trim(),
    )
    .expect("test should configure lone delegate target");

    let ask = afs_ask(&afs_home, "find the relevant context");

    assert!(
        ask.status.success(),
        "afs ask should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&ask.stdout),
        String::from_utf8_lossy(&ask.stderr)
    );
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains("lone has the answer"),
        "broadcast answer from the lone relevant agent should appear\nstdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("collaboration:"),
        "single-relevant broadcast must not emit a collaboration block\nstdout:\n{stdout}"
    );
    assert!(
        !lone_dir.join(".afs/collaborate-received").exists(),
        "lone agent must not receive COLLABORATE when no peer is relevant"
    );
    assert!(
        !silent_dir.join(".afs/collaborate-received").exists(),
        "silent agent must not receive COLLABORATE"
    );
    assert!(
        stdout.contains("changed_files: none"),
        "no files should be reported changed when collaboration is skipped\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("history_entries: none"),
        "no history entries when collaboration is skipped\nstdout:\n{stdout}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn ask_allows_agent_to_delegate_direct_task_reply_to_supervisor() {
    let afs_home = unique_afs_home("ask-delegate-to-supervisor");
    let source_dir = unique_afs_home("ask-delegate-source");
    let target_dir = unique_afs_home("ask-delegate-target");
    let pi_runtime = fake_pi_runtime("ask-delegate-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&source_dir).expect("test should create source managed directory");
    std::fs::create_dir_all(&target_dir).expect("test should create target managed directory");
    let source_file = source_dir.join("request.md");
    std::fs::write(&source_file, "needs delegated context\n")
        .expect("test should create source file");
    let source_dir = source_dir
        .canonicalize()
        .expect("source directory should canonicalize");
    let target_dir = target_dir
        .canonicalize()
        .expect("target directory should canonicalize");
    let source_file = source_file
        .canonicalize()
        .expect("source file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let source_install = install_managed_dir(&afs_home, &source_dir);
    assert!(
        source_install.status.success(),
        "source afs install should succeed"
    );
    let target_install = install_managed_dir(&afs_home, &target_dir);
    assert!(
        target_install.status.success(),
        "target afs install should succeed"
    );
    let source_identity =
        std::fs::read_to_string(source_dir.join(".afs/identity")).expect("source identity exists");
    let target_identity =
        std::fs::read_to_string(target_dir.join(".afs/identity")).expect("target identity exists");
    std::fs::write(
        source_dir.join(".afs/delegate-target"),
        target_dir.display().to_string(),
    )
    .expect("test should configure delegated target");
    std::fs::write(source_dir.join(".afs/delegate-reply-target"), "supervisor")
        .expect("test should configure delegated reply target");
    std::fs::write(
        source_dir.join(".afs/delegate-prompt"),
        "answer from target managed directory",
    )
    .expect("test should configure delegated prompt");
    std::fs::write(
        target_dir.join(".afs/task-response"),
        "target handled delegated work",
    )
    .expect("test should configure target task response");

    let ask = afs_ask(&afs_home, &format!("coordinate {}", source_file.display()));

    assert!(
        ask.status.success(),
        "afs ask should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&ask.stdout),
        String::from_utf8_lossy(&ask.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&ask.stderr), "");
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains("target handled delegated work"),
        "afs ask should include the delegated answer returned to the supervisor"
    );
    assert!(
        stdout.contains(&format!(
            "participating_agents: {}, {}",
            source_identity.trim(),
            target_identity.trim()
        )),
        "afs ask should report both the delegating and delegated Directory Agents"
    );
    assert!(
        stdout.contains("changed_files: none"),
        "afs ask should expose the delegated task Change Report"
    );
    let task_received = std::fs::read_to_string(target_dir.join(".afs/task-received"))
        .expect("target agent should receive delegated task");
    assert!(
        task_received.contains(&format!("requester={}", source_identity.trim())),
        "delegated task should identify the requesting Directory Agent"
    );
    assert!(
        task_received.contains("reply_target=supervisor"),
        "delegated task should preserve the requested Reply Target"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn ask_allows_agent_to_delegate_direct_task_reply_back_to_delegator() {
    let afs_home = unique_afs_home("ask-delegate-to-delegator");
    let source_dir = unique_afs_home("ask-delegator-source");
    let target_dir = unique_afs_home("ask-delegator-target");
    let pi_runtime = fake_pi_runtime("ask-delegator-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&source_dir).expect("test should create source managed directory");
    std::fs::create_dir_all(&target_dir).expect("test should create target managed directory");
    let source_file = source_dir.join("request.md");
    std::fs::write(&source_file, "needs delegated context\n")
        .expect("test should create source file");
    let source_dir = source_dir
        .canonicalize()
        .expect("source directory should canonicalize");
    let target_dir = target_dir
        .canonicalize()
        .expect("target directory should canonicalize");
    let source_file = source_file
        .canonicalize()
        .expect("source file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let source_install = install_managed_dir(&afs_home, &source_dir);
    assert!(
        source_install.status.success(),
        "source afs install should succeed"
    );
    let target_install = install_managed_dir(&afs_home, &target_dir);
    assert!(
        target_install.status.success(),
        "target afs install should succeed"
    );
    let source_identity =
        std::fs::read_to_string(source_dir.join(".afs/identity")).expect("source identity exists");
    let target_identity =
        std::fs::read_to_string(target_dir.join(".afs/identity")).expect("target identity exists");
    std::fs::write(
        source_dir.join(".afs/delegate-target"),
        target_dir.display().to_string(),
    )
    .expect("test should configure delegated target");
    std::fs::write(source_dir.join(".afs/delegate-reply-target"), "delegator")
        .expect("test should configure delegated reply target");
    std::fs::write(
        target_dir.join(".afs/task-response"),
        "target context for delegator",
    )
    .expect("test should configure target task response");

    let ask = afs_ask(&afs_home, &format!("coordinate {}", source_file.display()));

    assert!(
        ask.status.success(),
        "afs ask should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&ask.stdout),
        String::from_utf8_lossy(&ask.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&ask.stderr), "");
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains("delegator"),
        "afs ask should include the delegating agent's final answer"
    );
    assert!(
        stdout.contains("target context for delegator"),
        "delegating agent should receive and use the delegated answer"
    );
    assert!(
        stdout.contains(&format!(
            "participating_agents: {}, {}",
            source_identity.trim(),
            target_identity.trim()
        )),
        "afs ask should report both the delegating and delegated Directory Agents"
    );
    let delegated_reply = std::fs::read_to_string(source_dir.join(".afs/delegated-reply-received"))
        .expect("delegating agent should receive delegated reply");
    assert!(
        delegated_reply.contains(&format!("agent={}", target_identity.trim())),
        "delegated reply should identify the responding Directory Agent"
    );
    assert!(
        delegated_reply.contains("answer=target context for delegator"),
        "delegated reply should include the delegated task answer"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn ask_rejects_self_targeted_delegator_delegation_without_deadlocking() {
    let afs_home = unique_afs_home("ask-delegate-self-to-delegator");
    let managed_dir = unique_afs_home("ask-delegate-self-managed");
    let pi_runtime = fake_pi_runtime("ask-delegate-self-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let source_file = managed_dir.join("request.md");
    std::fs::write(&source_file, "needs delegated context\n")
        .expect("test should create source file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let source_file = source_file
        .canonicalize()
        .expect("source file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");
    let identity =
        std::fs::read_to_string(managed_dir.join(".afs/identity")).expect("identity exists");
    std::fs::write(managed_dir.join(".afs/delegate-target"), identity.trim())
        .expect("test should configure self delegation target");
    std::fs::write(managed_dir.join(".afs/delegate-reply-target"), "delegator")
        .expect("test should configure delegator reply target");

    let ask = spawn_afs_ask_streamed(&afs_home, &format!("coordinate {}", source_file.display()));
    let lines = ask.finish_with_timeout(Duration::from_secs(2));

    stop_daemon(&mut daemon);

    assert!(
        lines.iter().any(|(_, line)| {
            line == "progress: error delegated target cannot be the requesting agent when reply=delegator"
        }),
        "self-targeted delegator delegation should fail explicitly; lines:\n{lines:#?}"
    );
    assert!(
        !managed_dir.join(".afs/task-received").exists(),
        "the supervisor should not enqueue a task that would wait behind the active turn"
    );
}

#[test]
fn delegator_delegation_to_busy_target_cancels_queued_ticket() {
    let afs_home = unique_afs_home("ask-delegate-busy-target");
    let source_dir = unique_afs_home("ask-delegate-busy-source");
    let target_dir = unique_afs_home("ask-delegate-busy-target-dir");
    let pi_runtime = fake_pi_runtime("ask-delegate-busy-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&source_dir).expect("test should create source managed directory");
    std::fs::create_dir_all(&target_dir).expect("test should create target managed directory");
    let source_file = source_dir.join("request.md");
    let target_file = target_dir.join("target.md");
    std::fs::write(&source_file, "needs delegated context\n")
        .expect("test should create source file");
    std::fs::write(&target_file, "target context\n").expect("test should create target file");
    let source_dir = source_dir
        .canonicalize()
        .expect("source directory should canonicalize");
    let target_dir = target_dir
        .canonicalize()
        .expect("target directory should canonicalize");
    let source_file = source_file
        .canonicalize()
        .expect("source file should canonicalize");
    let target_file = target_file
        .canonicalize()
        .expect("target file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    assert!(
        install_managed_dir(&afs_home, &source_dir).status.success(),
        "source install should succeed"
    );
    assert!(
        install_managed_dir(&afs_home, &target_dir).status.success(),
        "target install should succeed"
    );
    std::fs::write(
        source_dir.join(".afs/delegate-target"),
        target_dir.display().to_string(),
    )
    .expect("test should configure target delegation");
    std::fs::write(source_dir.join(".afs/delegate-reply-target"), "delegator")
        .expect("test should configure delegator reply target");
    let delay_path = target_dir.join(".afs/ask-delay-seconds");
    std::fs::write(&delay_path, "2").expect("test should configure target ask delay");

    let blocking_prompt = format!("hold {}", target_file.display());
    let blocking = spawn_afs_ask_streamed(&afs_home, &blocking_prompt);
    assert!(
        wait_until(Duration::from_secs(1), || {
            std::fs::read_to_string(target_dir.join(".afs/ask-received"))
                .map(|log| log.contains(&format!("prompt={blocking_prompt}")))
                .unwrap_or(false)
        }),
        "target ask should hold the target agent before delegation starts"
    );
    std::fs::remove_file(&delay_path).expect("later target asks should not inherit the delay");

    let delegating =
        spawn_afs_ask_streamed(&afs_home, &format!("coordinate {}", source_file.display()));
    let delegating_lines = delegating.finish_with_timeout(Duration::from_secs(1));
    assert!(
        delegating_lines
            .iter()
            .any(|(_, line)| line == "progress: error delegated target is busy for reply=delegator"),
        "busy delegator-targeted delegation should fail explicitly; lines:\n{delegating_lines:#?}"
    );
    assert!(
        !target_dir.join(".afs/task-received").exists(),
        "busy target should not receive a queued TASK that cannot run immediately"
    );

    let _ = blocking.finish();
    let after_busy =
        spawn_afs_ask_streamed(&afs_home, &format!("after busy {}", target_file.display()));
    let after_busy_lines = after_busy.finish_with_timeout(Duration::from_secs(1));
    assert!(
        after_busy_lines
            .iter()
            .any(|(_, line)| line.contains("answered about")),
        "dropping the unstarted busy-target ticket must not wedge the target queue; lines:\n{after_busy_lines:#?}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn delegated_task_change_report_includes_agent_history_entry() {
    let afs_home = unique_afs_home("ask-delegate-change-report");
    let source_dir = unique_afs_home("ask-change-source");
    let target_dir = unique_afs_home("ask-change-target");
    let pi_runtime = fake_pi_runtime("ask-change-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&source_dir).expect("test should create source managed directory");
    std::fs::create_dir_all(&target_dir).expect("test should create target managed directory");
    let source_file = source_dir.join("request.md");
    let target_file = target_dir.join("handoff.md");
    std::fs::write(&source_file, "needs delegated edit\n").expect("test should create source file");
    std::fs::write(&target_file, "before\n").expect("test should create target file");
    let source_dir = source_dir
        .canonicalize()
        .expect("source directory should canonicalize");
    let target_dir = target_dir
        .canonicalize()
        .expect("target directory should canonicalize");
    let source_file = source_file
        .canonicalize()
        .expect("source file should canonicalize");
    let target_file = target_file
        .canonicalize()
        .expect("target file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let source_install = install_managed_dir(&afs_home, &source_dir);
    assert!(
        source_install.status.success(),
        "source afs install should succeed"
    );
    let target_install = install_managed_dir(&afs_home, &target_dir);
    assert!(
        target_install.status.success(),
        "target afs install should succeed"
    );
    std::fs::write(
        source_dir.join(".afs/delegate-target"),
        target_dir.display().to_string(),
    )
    .expect("test should configure delegated target");
    std::fs::write(source_dir.join(".afs/delegate-reply-target"), "supervisor")
        .expect("test should configure delegated reply target");
    std::fs::write(
        target_dir.join(".afs/task-response"),
        "target updated handoff",
    )
    .expect("test should configure target task response");
    std::fs::write(target_dir.join(".afs/task-write-file"), "handoff.md")
        .expect("test should configure target write path");
    std::fs::write(
        target_dir.join(".afs/task-write-content"),
        "after delegated task",
    )
    .expect("test should configure target write content");

    let ask = afs_ask(&afs_home, &format!("coordinate {}", source_file.display()));

    assert!(
        ask.status.success(),
        "afs ask should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&ask.stdout),
        String::from_utf8_lossy(&ask.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&ask.stderr), "");
    assert_eq!(
        std::fs::read_to_string(&target_file).expect("target file should be readable"),
        "after delegated task\n",
        "delegated task should modify the target Managed Subtree"
    );
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains("changed_files: handoff.md"),
        "afs ask should report files changed by delegated work"
    );
    assert!(
        stdout.contains("history_entries: history-"),
        "afs ask should report the resulting Agent Change History Entry"
    );
    assert!(
        wait_until(Duration::from_secs(3), || {
            let history = afs_history(&afs_home, &target_dir);
            let stdout = String::from_utf8_lossy(&history.stdout);
            history.status.success()
                && stdout.contains("type=agent")
                && stdout.contains("summary=Agent change: handoff.md")
        }),
        "delegated file modification should be recorded as an Agent Change"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn delegated_tasks_for_busy_agent_queue_fifo_and_report_progress() {
    let afs_home = unique_afs_home("ask-delegate-queue");
    let source_dir = unique_afs_home("ask-queue-source");
    let target_dir = unique_afs_home("ask-queue-target");
    let pi_runtime = fake_pi_runtime("ask-queue-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&source_dir).expect("test should create source managed directory");
    std::fs::create_dir_all(&target_dir).expect("test should create target managed directory");
    let source_file = source_dir.join("request.md");
    std::fs::write(&source_file, "needs two delegated tasks\n")
        .expect("test should create source file");
    let source_dir = source_dir
        .canonicalize()
        .expect("source directory should canonicalize");
    let target_dir = target_dir
        .canonicalize()
        .expect("target directory should canonicalize");
    let source_file = source_file
        .canonicalize()
        .expect("source file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let source_install = install_managed_dir(&afs_home, &source_dir);
    assert!(
        source_install.status.success(),
        "source afs install should succeed"
    );
    let target_install = install_managed_dir(&afs_home, &target_dir);
    assert!(
        target_install.status.success(),
        "target afs install should succeed"
    );
    let target_identity =
        std::fs::read_to_string(target_dir.join(".afs/identity")).expect("target identity exists");
    std::fs::write(
        source_dir.join(".afs/delegate-target"),
        target_dir.display().to_string(),
    )
    .expect("test should configure delegated target");
    std::fs::write(source_dir.join(".afs/delegate-reply-target"), "supervisor")
        .expect("test should configure delegated reply target");
    std::fs::write(
        source_dir.join(".afs/delegate-prompt"),
        "first delegated task",
    )
    .expect("test should configure first delegated prompt");
    std::fs::write(
        source_dir.join(".afs/delegate-second-prompt"),
        "second delegated task",
    )
    .expect("test should configure second delegated prompt");

    let ask = afs_ask(&afs_home, &format!("coordinate {}", source_file.display()));

    assert!(
        ask.status.success(),
        "afs ask should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&ask.stdout),
        String::from_utf8_lossy(&ask.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&ask.stderr), "");
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains(&format!(
            "progress: queued task agent={} queue=1",
            target_identity.trim()
        )),
        "afs ask should report when a delegated task waits in the target queue"
    );
    assert!(
        stdout.contains(&format!(
            "progress: started task agent={} queue=0",
            target_identity.trim()
        )),
        "afs ask should report when the target queue drains"
    );
    assert!(
        stdout.contains("delegated answer for first delegated task"),
        "first delegated task answer should be returned"
    );
    assert!(
        stdout.contains("delegated answer for second delegated task"),
        "second delegated task answer should be returned"
    );
    let task_received = std::fs::read_to_string(target_dir.join(".afs/task-received"))
        .expect("target agent should receive delegated tasks");
    let first_position = task_received
        .find("prompt=first delegated task")
        .expect("target should receive first task");
    let second_position = task_received
        .find("prompt=second delegated task")
        .expect("target should receive second task");
    assert!(
        first_position < second_position,
        "target agent should process delegated tasks FIFO"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn ask_routes_explicit_nested_managed_path_directly_to_deepest_owner() {
    let afs_home = unique_afs_home("ask-direct-managed-path");
    let parent_dir = unique_afs_home("ask-direct-parent");
    let pi_runtime = fake_pi_runtime("ask-direct-runtime");
    let socket_path = supervisor_socket(&afs_home);
    let child_dir = parent_dir.join("child");
    std::fs::create_dir_all(&child_dir).expect("test should create nested managed directory");
    let target_file = child_dir.join("notes.txt");
    std::fs::write(&target_file, "child notes\n").expect("test should create target file");
    let parent_dir = parent_dir
        .canonicalize()
        .expect("parent managed directory should canonicalize");
    let child_dir = child_dir
        .canonicalize()
        .expect("child managed directory should canonicalize");
    let target_file = target_file
        .canonicalize()
        .expect("target file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let parent_install = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("install")
        .arg(&parent_dir)
        .output()
        .expect("parent afs install should run");
    assert!(
        parent_install.status.success(),
        "parent afs install should succeed"
    );
    let child_install = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("install")
        .arg(&child_dir)
        .output()
        .expect("child afs install should run");
    assert!(
        child_install.status.success(),
        "child afs install should succeed"
    );

    let ask = afs_ask(&afs_home, &format!("summarize {}", target_file.display()));

    assert!(ask.status.success(), "afs ask should succeed");
    assert_eq!(String::from_utf8_lossy(&ask.stderr), "");
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains(&format!("answered about {}", target_file.display())),
        "afs ask should return the owning agent response"
    );
    assert!(
        wait_until(Duration::from_secs(2), || child_dir
            .join(".afs/ask-received")
            .is_file()),
        "deepest owning agent should receive the ask"
    );
    assert!(
        std::fs::read_to_string(child_dir.join(".afs/ask-received"))
            .expect("owning agent ask marker should be readable")
            .contains(&target_file.display().to_string()),
        "owning agent should receive the explicit path"
    );
    assert!(
        !parent_dir.join(".afs/ask-received").exists(),
        "ancestor agent should not receive a direct path ask owned by a nested agent"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn installing_nested_managed_directory_splits_parent_history_ownership() {
    let afs_home = unique_afs_home("nested-install-split");
    let parent_dir = unique_afs_home("nested-install-parent");
    let pi_runtime = fake_pi_runtime("nested-install-runtime");
    let socket_path = supervisor_socket(&afs_home);
    let child_dir = parent_dir.join("child");
    std::fs::create_dir_all(&child_dir).expect("test should create child directory");
    let child_file = child_dir.join("notes.txt");
    std::fs::write(&child_file, "before split\n").expect("test should create child file");
    let parent_dir = parent_dir
        .canonicalize()
        .expect("parent managed directory should canonicalize");
    let child_dir = child_dir
        .canonicalize()
        .expect("child managed directory should canonicalize");
    let child_file = child_file
        .canonicalize()
        .expect("child file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let parent_install = install_managed_dir(&afs_home, &parent_dir);
    assert!(
        parent_install.status.success(),
        "parent afs install should succeed"
    );
    let child_install = install_managed_dir(&afs_home, &child_dir);
    assert!(
        child_install.status.success(),
        "child afs install should succeed"
    );

    let parent_history = afs_history(&afs_home, &parent_dir);
    assert!(
        parent_history.status.success(),
        "parent afs history should succeed"
    );
    let parent_stdout = String::from_utf8_lossy(&parent_history.stdout);
    assert!(
        parent_stdout.contains("type=ownership"),
        "parent history should record the ownership split"
    );
    assert!(
        parent_stdout.contains("summary=Ownership split: child"),
        "parent ownership history should name the nested child"
    );

    std::fs::write(&child_file, "after split\n").expect("test should modify child file");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let child_history = afs_history(&afs_home, &child_dir);
            child_history.status.success()
                && String::from_utf8_lossy(&child_history.stdout).contains("External change")
        }),
        "child history should record child edits after the split"
    );

    let parent_history = afs_history(&afs_home, &parent_dir);
    assert!(
        parent_history.status.success(),
        "parent afs history should still succeed"
    );
    let parent_stdout = String::from_utf8_lossy(&parent_history.stdout);
    assert!(
        !parent_stdout.contains("External change: child/notes.txt"),
        "parent history should not record child-owned edits after the split"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn removing_nested_managed_directory_merges_history_and_archives_agent_home() {
    let afs_home = unique_afs_home("nested-remove-merge");
    let parent_dir = unique_afs_home("nested-remove-parent");
    let pi_runtime = fake_pi_runtime("nested-remove-runtime");
    let socket_path = supervisor_socket(&afs_home);
    let child_dir = parent_dir.join("child");
    std::fs::create_dir_all(&child_dir).expect("test should create child directory");
    let child_file = child_dir.join("notes.txt");
    std::fs::write(&child_file, "before child history\n").expect("test should create child file");
    let parent_dir = parent_dir
        .canonicalize()
        .expect("parent managed directory should canonicalize");
    let child_dir = child_dir
        .canonicalize()
        .expect("child managed directory should canonicalize");
    let child_file = child_file
        .canonicalize()
        .expect("child file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let parent_install = install_managed_dir(&afs_home, &parent_dir);
    assert!(
        parent_install.status.success(),
        "parent afs install should succeed"
    );
    let child_install = install_managed_dir(&afs_home, &child_dir);
    assert!(
        child_install.status.success(),
        "child afs install should succeed"
    );
    let child_identity =
        std::fs::read_to_string(child_dir.join(".afs/identity")).expect("child identity exists");

    std::fs::write(&child_file, "child history before removal\n")
        .expect("test should modify child file before removal");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let child_history = afs_history(&afs_home, &child_dir);
            child_history.status.success()
                && String::from_utf8_lossy(&child_history.stdout)
                    .contains("summary=External change: notes.txt")
        }),
        "child history should contain a local entry before removal"
    );

    let remove = remove_managed_dir(&afs_home, &child_dir);
    assert!(
        remove.status.success(),
        "afs remove should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&remove.stdout),
        String::from_utf8_lossy(&remove.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&remove.stderr), "");
    let remove_stdout = String::from_utf8_lossy(&remove.stdout);
    assert!(
        remove_stdout.contains("removed managed directory"),
        "afs remove should report the removed child"
    );
    assert!(
        remove_stdout.contains("archived_agent_home "),
        "afs remove should report the archive location"
    );

    assert!(
        !child_dir.join(".afs").exists(),
        "removed child Agent Home should be moved out of the child subtree"
    );
    let archive_root = parent_dir.join(".afs/archives");
    let archived_home = std::fs::read_dir(&archive_root)
        .expect("parent archive root should exist")
        .map(|entry| entry.expect("archive entry should be readable").path())
        .find(|path| path.join("identity").is_file())
        .expect("parent should archive the removed child Agent Home");
    assert_eq!(
        std::fs::read_to_string(archived_home.join("identity"))
            .expect("archived identity should be readable"),
        child_identity,
        "archive should preserve the child identity"
    );
    let archived_log = Command::new("git")
        .arg("-c")
        .arg("safe.directory=*")
        .arg(format!(
            "--git-dir={}",
            archived_home.join("history/repo").display()
        ))
        .arg("log")
        .arg("--format=%B")
        .output()
        .expect("git log on archived history should run");
    assert!(
        archived_log.status.success(),
        "archived history should be a readable git repo: {}",
        String::from_utf8_lossy(&archived_log.stderr)
    );
    assert!(
        String::from_utf8_lossy(&archived_log.stdout).contains("External change: notes.txt"),
        "archive should preserve the child history"
    );

    let parent_history = afs_history(&afs_home, &parent_dir);
    assert!(
        parent_history.status.success(),
        "parent afs history should succeed"
    );
    let parent_stdout = String::from_utf8_lossy(&parent_history.stdout);
    assert!(
        parent_stdout.contains("type=ownership"),
        "parent history should include ownership entries"
    );
    assert!(
        parent_stdout.contains("summary=Ownership merge: child"),
        "parent history should record the child merge"
    );

    std::fs::write(&child_file, "parent owns child content now\n")
        .expect("test should modify child content after removal");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let parent_history = afs_history(&afs_home, &parent_dir);
            parent_history.status.success()
                && String::from_utf8_lossy(&parent_history.stdout)
                    .contains("summary=External change: child/notes.txt")
        }),
        "parent history should record child-path edits after removal"
    );

    let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("agents")
        .output()
        .expect("afs agents should run");
    assert!(agents.status.success(), "afs agents should succeed");
    let agents_stdout = String::from_utf8_lossy(&agents.stdout);
    assert!(
        agents_stdout.contains(&parent_dir.display().to_string()),
        "parent agent should remain registered"
    );
    assert!(
        !agents_stdout.contains(&child_dir.display().to_string()),
        "removed child agent should leave the registry"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn removing_nested_managed_directory_surfaces_child_history_in_parent() {
    let afs_home = unique_afs_home("nested-remove-visibility");
    let parent_dir = unique_afs_home("nested-remove-visibility-parent");
    let pi_runtime = fake_pi_runtime("nested-remove-visibility-runtime");
    let socket_path = supervisor_socket(&afs_home);
    let child_dir = parent_dir.join("child");
    std::fs::create_dir_all(&child_dir).expect("test should create child directory");
    let child_file = child_dir.join("notes.txt");
    std::fs::write(&child_file, "initial child content\n").expect("test should create child file");
    let parent_dir = parent_dir
        .canonicalize()
        .expect("parent managed directory should canonicalize");
    let child_dir = child_dir
        .canonicalize()
        .expect("child managed directory should canonicalize");
    let child_file = child_file
        .canonicalize()
        .expect("child file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    assert!(
        install_managed_dir(&afs_home, &parent_dir).status.success(),
        "parent afs install should succeed"
    );
    assert!(
        install_managed_dir(&afs_home, &child_dir).status.success(),
        "child afs install should succeed"
    );

    std::fs::write(&child_file, "child edit pre-removal\n")
        .expect("test should write child file before removal");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let child_history = afs_history(&afs_home, &child_dir);
            child_history.status.success()
                && String::from_utf8_lossy(&child_history.stdout)
                    .contains("summary=External change: notes.txt")
        }),
        "child history should contain an external change entry before removal"
    );

    let remove = remove_managed_dir(&afs_home, &child_dir);
    assert!(
        remove.status.success(),
        "afs remove should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&remove.stdout),
        String::from_utf8_lossy(&remove.stderr)
    );

    let parent_history = afs_history(&afs_home, &parent_dir);
    assert!(
        parent_history.status.success(),
        "afs history on parent should succeed"
    );
    let parent_stdout = String::from_utf8_lossy(&parent_history.stdout);
    let child_entry_line = parent_stdout
        .lines()
        .find(|line| line.contains("summary=External change: child/notes.txt"))
        .unwrap_or_else(|| panic!(
            "parent history should surface the child's external change rewritten to a parent-relative path; got:\n{parent_stdout}"
        ));
    assert!(
        child_entry_line.contains("origin=child"),
        "merged child entry should carry an origin=child provenance marker; got:\n{child_entry_line}"
    );

    let ownership_line = parent_stdout
        .lines()
        .find(|line| line.contains("summary=Ownership merge: child"))
        .expect("parent history should still include the ownership-merge marker");
    assert!(
        ownership_line.contains("origin=") && !ownership_line.contains("origin=child"),
        "local parent-authored entries should not carry a child origin; got:\n{ownership_line}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn transitive_removal_chains_origin_and_rewrites_ownership_summaries() {
    let afs_home = unique_afs_home("nested-remove-transitive");
    let grandparent_dir = unique_afs_home("nested-remove-transitive-gp");
    let pi_runtime = fake_pi_runtime("nested-remove-transitive-runtime");
    let socket_path = supervisor_socket(&afs_home);
    let child_dir = grandparent_dir.join("child");
    let grandchild_dir = child_dir.join("grandchild");
    std::fs::create_dir_all(&grandchild_dir).expect("test should create nested layout");
    let grandchild_file = grandchild_dir.join("notes.txt");
    std::fs::write(&grandchild_file, "initial grandchild content\n")
        .expect("test should create grandchild file");
    let grandparent_dir = grandparent_dir
        .canonicalize()
        .expect("grandparent should canonicalize");
    let child_dir = child_dir.canonicalize().expect("child should canonicalize");
    let grandchild_dir = grandchild_dir
        .canonicalize()
        .expect("grandchild should canonicalize");
    let grandchild_file = grandchild_file
        .canonicalize()
        .expect("grandchild file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    assert!(
        install_managed_dir(&afs_home, &grandparent_dir)
            .status
            .success(),
        "grandparent install should succeed"
    );
    assert!(
        install_managed_dir(&afs_home, &child_dir).status.success(),
        "child install should succeed"
    );
    assert!(
        install_managed_dir(&afs_home, &grandchild_dir)
            .status
            .success(),
        "grandchild install should succeed"
    );

    std::fs::write(&grandchild_file, "grandchild edit pre-removal\n")
        .expect("test should edit grandchild file");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let history = afs_history(&afs_home, &grandchild_dir);
            history.status.success()
                && String::from_utf8_lossy(&history.stdout)
                    .contains("summary=External change: notes.txt")
        }),
        "grandchild history should record the external change before removal"
    );

    let remove_grandchild = remove_managed_dir(&afs_home, &grandchild_dir);
    assert!(
        remove_grandchild.status.success(),
        "afs remove grandchild should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&remove_grandchild.stdout),
        String::from_utf8_lossy(&remove_grandchild.stderr)
    );

    let remove_child = remove_managed_dir(&afs_home, &child_dir);
    assert!(
        remove_child.status.success(),
        "afs remove child should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&remove_child.stdout),
        String::from_utf8_lossy(&remove_child.stderr)
    );

    let grandparent_history = afs_history(&afs_home, &grandparent_dir);
    assert!(
        grandparent_history.status.success(),
        "afs history on grandparent should succeed"
    );
    let stdout = String::from_utf8_lossy(&grandparent_history.stdout);

    let external_line = stdout
        .lines()
        .find(|line| line.contains("summary=External change: child/grandchild/notes.txt"))
        .unwrap_or_else(|| {
            panic!("grandparent history should surface the grandchild external change rewritten to child/grandchild/notes.txt; got:\n{stdout}")
        });
    assert!(
        external_line.contains("origin=child/grandchild"),
        "transitively-merged entries should chain their origin provenance; got:\n{external_line}"
    );

    assert!(
        stdout
            .lines()
            .any(|line| line.contains("summary=Ownership merge: child/grandchild")),
        "grandparent history should rewrite the merged ownership summary to child/grandchild; got:\n{stdout}"
    );
    assert!(
        stdout
            .lines()
            .any(|line| line.contains("summary=Ownership split: child/grandchild")),
        "grandparent history should rewrite the merged ownership-split summary to child/grandchild; got:\n{stdout}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn removing_top_level_managed_directory_archives_agent_home_under_supervisor_home() {
    let afs_home = unique_afs_home("top-level-remove-archive");
    let managed_dir = unique_afs_home("top-level-remove-archive-managed");
    let pi_runtime = fake_pi_runtime("top-level-remove-archive-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let target_file = managed_dir.join("notes.txt");
    std::fs::write(&target_file, "before remove\n").expect("test should create target file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let target_file = target_file
        .canonicalize()
        .expect("target file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    assert!(
        install_managed_dir(&afs_home, &managed_dir)
            .status
            .success(),
        "top-level afs install should succeed"
    );
    let identity =
        std::fs::read_to_string(managed_dir.join(".afs/identity")).expect("identity exists");

    std::fs::write(&target_file, "after external change\n")
        .expect("test should modify file before removal");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let history = afs_history(&afs_home, &managed_dir);
            history.status.success()
                && String::from_utf8_lossy(&history.stdout)
                    .contains("summary=External change: notes.txt")
        }),
        "managed directory should have a live external-change entry before removal"
    );

    let remove = remove_managed_dir(&afs_home, &managed_dir);
    assert!(
        remove.status.success(),
        "afs remove should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&remove.stdout),
        String::from_utf8_lossy(&remove.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&remove.stderr), "");
    let remove_stdout = String::from_utf8_lossy(&remove.stdout);
    assert!(
        remove_stdout.contains("removed managed directory"),
        "afs remove should report the removed directory; got:\n{remove_stdout}"
    );
    assert!(
        remove_stdout.contains("archived_agent_home "),
        "afs remove should report the archive location; got:\n{remove_stdout}"
    );

    assert!(
        !managed_dir.join(".afs").exists(),
        "removed Agent Home should move out of the managed directory"
    );
    let archive_root = afs_home.join("archives");
    let archived_home = std::fs::read_dir(&archive_root)
        .expect("supervisor archive root should exist")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .find(|path| {
            std::fs::read_to_string(path.join("identity"))
                .map(|archived_identity| archived_identity == identity)
                .unwrap_or(false)
        })
        .expect("supervisor archive should contain the removed Agent Home");
    let archived_log = Command::new("git")
        .arg("-c")
        .arg("safe.directory=*")
        .arg(format!(
            "--git-dir={}",
            archived_home.join("history/repo").display()
        ))
        .arg("log")
        .arg("--format=%B")
        .output()
        .expect("git log on archived history should run");
    assert!(
        archived_log.status.success(),
        "archived history should be a readable git repo: {}",
        String::from_utf8_lossy(&archived_log.stderr)
    );
    assert!(
        String::from_utf8_lossy(&archived_log.stdout).contains("External change: notes.txt"),
        "archive should preserve the pre-removal history"
    );

    let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("agents")
        .output()
        .expect("afs agents should run");
    assert!(agents.status.success(), "afs agents should succeed");
    assert_eq!(
        String::from_utf8_lossy(&agents.stdout),
        "no agents registered\n",
        "afs agents should report no registrations after top-level remove"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn removing_top_level_managed_directory_with_discard_history_deletes_agent_home() {
    let afs_home = unique_afs_home("top-level-remove-discard");
    let managed_dir = unique_afs_home("top-level-remove-discard-managed");
    let pi_runtime = fake_pi_runtime("top-level-remove-discard-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    assert!(
        install_managed_dir(&afs_home, &managed_dir)
            .status
            .success(),
        "top-level afs install should succeed"
    );

    let remove = remove_managed_dir_with_flags(&afs_home, &managed_dir, &["--discard-history"]);
    assert!(
        remove.status.success(),
        "afs remove --discard-history should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&remove.stdout),
        String::from_utf8_lossy(&remove.stderr)
    );
    let remove_stdout = String::from_utf8_lossy(&remove.stdout);
    assert!(
        remove_stdout.contains("discarded_agent_home "),
        "afs remove --discard-history should report a discarded_agent_home line; got:\n{remove_stdout}"
    );
    assert!(
        !remove_stdout.contains("archived_agent_home "),
        "afs remove --discard-history should not report an archive location; got:\n{remove_stdout}"
    );

    assert!(
        !managed_dir.join(".afs").exists(),
        "managed Agent Home should be deleted on --discard-history"
    );
    let archive_root = afs_home.join("archives");
    let archive_is_empty = !archive_root.exists()
        || std::fs::read_dir(&archive_root)
            .expect("archive root readable")
            .next()
            .is_none();
    assert!(
        archive_is_empty,
        "supervisor archive root should remain empty when --discard-history is set"
    );

    let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("agents")
        .output()
        .expect("afs agents should run");
    assert_eq!(
        String::from_utf8_lossy(&agents.stdout),
        "no agents registered\n",
        "afs agents should reflect the removal even with --discard-history"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn removing_nested_managed_directory_with_discard_history_skips_archive_and_merge() {
    let afs_home = unique_afs_home("nested-remove-discard");
    let parent_dir = unique_afs_home("nested-remove-discard-parent");
    let pi_runtime = fake_pi_runtime("nested-remove-discard-runtime");
    let socket_path = supervisor_socket(&afs_home);
    let child_dir = parent_dir.join("child");
    std::fs::create_dir_all(&child_dir).expect("test should create child directory");
    let child_file = child_dir.join("notes.txt");
    std::fs::write(&child_file, "child content\n").expect("test should create child file");
    let parent_dir = parent_dir
        .canonicalize()
        .expect("parent managed directory should canonicalize");
    let child_dir = child_dir
        .canonicalize()
        .expect("child managed directory should canonicalize");
    let child_file = child_file
        .canonicalize()
        .expect("child file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    assert!(
        install_managed_dir(&afs_home, &parent_dir).status.success(),
        "parent afs install should succeed"
    );
    assert!(
        install_managed_dir(&afs_home, &child_dir).status.success(),
        "child afs install should succeed"
    );

    std::fs::write(&child_file, "child edit pre-removal\n")
        .expect("test should modify child before removal");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let child_history = afs_history(&afs_home, &child_dir);
            child_history.status.success()
                && String::from_utf8_lossy(&child_history.stdout)
                    .contains("summary=External change: notes.txt")
        }),
        "child history should record an external change before removal"
    );

    let remove = remove_managed_dir_with_flags(&afs_home, &child_dir, &["--discard-history"]);
    assert!(
        remove.status.success(),
        "nested afs remove --discard-history should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&remove.stdout),
        String::from_utf8_lossy(&remove.stderr)
    );
    let remove_stdout = String::from_utf8_lossy(&remove.stdout);
    assert!(
        remove_stdout.contains("discarded_agent_home "),
        "nested --discard-history should report a discarded_agent_home; got:\n{remove_stdout}"
    );
    assert!(
        !remove_stdout.contains("archived_agent_home "),
        "nested --discard-history should not report an archive location; got:\n{remove_stdout}"
    );

    assert!(
        !child_dir.join(".afs").exists(),
        "child Agent Home should be deleted on --discard-history"
    );
    let parent_archive_root = parent_dir.join(".afs/archives");
    let archive_is_empty = !parent_archive_root.exists()
        || std::fs::read_dir(&parent_archive_root)
            .expect("parent archive root readable")
            .next()
            .is_none();
    assert!(
        archive_is_empty,
        "parent archive root should remain empty when --discard-history is set"
    );

    let parent_history = afs_history(&afs_home, &parent_dir);
    assert!(
        parent_history.status.success(),
        "afs history on parent should succeed"
    );
    let parent_stdout = String::from_utf8_lossy(&parent_history.stdout);
    assert!(
        parent_stdout.contains("Ownership merge: child (history discarded)"),
        "parent history should record that child history was discarded; got:\n{parent_stdout}"
    );
    assert!(
        !parent_stdout.contains("External change: child/notes.txt"),
        "parent history should NOT carry merged child entries after --discard-history; got:\n{parent_stdout}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn removing_top_level_managed_directory_that_no_longer_exists_unregisters_cleanly() {
    let afs_home = unique_afs_home("top-level-remove-missing");
    let managed_dir = unique_afs_home("top-level-remove-missing-managed");
    let pi_runtime = fake_pi_runtime("top-level-remove-missing-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    assert!(
        install_managed_dir(&afs_home, &managed_dir)
            .status
            .success(),
        "top-level afs install should succeed"
    );

    remove_dir_all_retry(&managed_dir);

    let remove = remove_managed_dir(&afs_home, &managed_dir);
    assert!(
        remove.status.success(),
        "afs remove on a missing directory should succeed without --discard-history\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&remove.stdout),
        String::from_utf8_lossy(&remove.stderr)
    );
    let remove_stdout = String::from_utf8_lossy(&remove.stdout);
    assert!(
        remove_stdout.contains("missing_agent_home "),
        "afs remove on a missing directory should report missing_agent_home; got:\n{remove_stdout}"
    );
    assert!(
        !remove_stdout.contains("archived_agent_home "),
        "afs remove on a missing directory should not claim to have archived; got:\n{remove_stdout}"
    );

    let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("agents")
        .output()
        .expect("afs agents should run");
    assert_eq!(
        String::from_utf8_lossy(&agents.stdout),
        "no agents registered\n",
        "afs agents should report no registrations after removing a missing directory"
    );

    let registry_path = afs_home.join("registry.tsv");
    let registry_has_live_entry = std::fs::read_to_string(&registry_path)
        .map(|contents| contents.lines().skip(1).any(|line| !line.trim().is_empty()))
        .unwrap_or(false);
    assert!(
        !registry_has_live_entry,
        "registry should not keep a stale entry for the removed directory"
    );

    let archive_root = afs_home.join("archives");
    let archive_is_empty = !archive_root.exists()
        || std::fs::read_dir(&archive_root)
            .expect("archive root readable")
            .next()
            .is_none();
    assert!(
        archive_is_empty,
        "supervisor archive root should remain empty when the Agent Home was already missing"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn removing_top_level_managed_directory_that_contains_supervisor_home_is_rejected() {
    let managed_dir = unique_afs_home("top-level-remove-supervisor-inside");
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let afs_home = managed_dir.join(".afs-supervisor");
    let pi_runtime = fake_pi_runtime("top-level-remove-supervisor-inside-runtime");
    let socket_path = supervisor_socket(&afs_home);
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    assert!(
        install_managed_dir(&afs_home, &managed_dir)
            .status
            .success(),
        "top-level afs install should succeed"
    );

    let remove = remove_managed_dir(&afs_home, &managed_dir);
    assert!(
        !remove.status.success(),
        "afs remove should fail when the supervisor home is inside the managed directory"
    );
    let stderr = String::from_utf8_lossy(&remove.stderr);
    assert!(
        stderr.contains("cannot remove a managed directory that contains the AFS supervisor home"),
        "afs remove should explain the supervisor-home collision; got stderr:\n{stderr}"
    );

    let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("agents")
        .output()
        .expect("afs agents should run");
    assert!(
        String::from_utf8_lossy(&agents.stdout).contains(&managed_dir.display().to_string()),
        "managed directory should remain registered after rejected removal"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn ask_answer_for_managed_path_includes_file_reference() {
    let afs_home = unique_afs_home("ask-reference");
    let managed_dir = unique_afs_home("ask-reference-managed");
    let pi_runtime = fake_pi_runtime("ask-reference-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let target_file = managed_dir.join("notes.txt");
    std::fs::write(&target_file, "notes for direct ask\n").expect("test should create target file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let target_file = target_file
        .canonicalize()
        .expect("target file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("install")
        .arg(&managed_dir)
        .output()
        .expect("afs install should run");
    assert!(install.status.success(), "afs install should succeed");

    let ask = afs_ask(&afs_home, &format!("summarize {}", target_file.display()));

    assert!(ask.status.success(), "afs ask should succeed");
    assert_eq!(String::from_utf8_lossy(&ask.stderr), "");
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains("references:\n"),
        "afs ask should include a file-reference section"
    );
    assert!(
        stdout.contains(&format!("- {}", target_file.display())),
        "afs ask should reference the explicit managed path"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn ask_reports_unmanaged_explicit_path_without_contacting_agents() {
    let afs_home = unique_afs_home("ask-unmanaged-path");
    let managed_dir = unique_afs_home("ask-unmanaged-managed");
    let unmanaged_dir = unique_afs_home("ask-unmanaged-outside");
    let pi_runtime = fake_pi_runtime("ask-unmanaged-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::create_dir_all(&unmanaged_dir).expect("test should create unmanaged directory");
    let unmanaged_file = unmanaged_dir.join("outside.txt");
    std::fs::write(&unmanaged_file, "outside afs\n").expect("test should create unmanaged file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let unmanaged_file = unmanaged_file
        .canonicalize()
        .expect("unmanaged file should canonicalize");
    let unmanaged_parent = unmanaged_file
        .parent()
        .expect("unmanaged file should have a parent")
        .to_path_buf();
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("install")
        .arg(&managed_dir)
        .output()
        .expect("afs install should run");
    assert!(install.status.success(), "afs install should succeed");

    let ask = afs_ask(
        &afs_home,
        &format!("summarize {}", unmanaged_file.display()),
    );

    assert!(
        !ask.status.success(),
        "afs ask should fail for an unmanaged explicit path"
    );
    assert_eq!(String::from_utf8_lossy(&ask.stdout), "");
    let stderr = String::from_utf8_lossy(&ask.stderr);
    assert!(
        stderr.contains(&format!(
            "path is not managed: {}",
            unmanaged_file.display()
        )),
        "afs ask should identify the unmanaged path"
    );
    assert!(
        stderr.contains(&format!("afs install {}", unmanaged_parent.display())),
        "afs ask should suggest installing the path parent"
    );
    assert!(
        !managed_dir.join(".afs/ask-received").exists(),
        "managed agents should not inspect unmanaged explicit paths"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn install_creates_agent_home_and_history_baseline_through_live_supervisor() {
    let afs_home = unique_afs_home("install-agent-home");
    let managed_dir = unique_afs_home("managed-dir");
    let pi_runtime = fake_pi_runtime("install-agent-home-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(managed_dir.join("notes.txt"), "hello afs\n")
        .expect("test should create managed file");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("install")
        .arg(&managed_dir)
        .output()
        .expect("afs install should run");

    assert!(output.status.success(), "afs install should succeed");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("installed managed directory"),
        "afs install should report a new Managed Directory"
    );
    assert!(managed_dir.join(".afs/identity").is_file());
    assert!(managed_dir.join(".afs/instructions.md").is_file());
    assert!(managed_dir.join(".afs/history/repo/HEAD").is_file());
    assert!(
        managed_dir
            .join(".afs/history/repo/refs/heads/afs")
            .is_file()
    );
    assert!(
        wait_until(Duration::from_secs(2), || {
            std::fs::read_to_string(managed_dir.join(".afs/runtime-started"))
                .map(|content| content.contains("rpc=stdio"))
                .unwrap_or(false)
        }),
        "afs install should start the configured Pi Agent Runtime in stdio RPC mode"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn install_is_idempotent_for_an_already_managed_directory() {
    let afs_home = unique_afs_home("install-idempotent");
    let managed_dir = unique_afs_home("managed-idempotent");
    let pi_runtime = fake_pi_runtime("install-idempotent-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(managed_dir.join("notes.txt"), "hello afs\n")
        .expect("test should create managed file");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let first = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("install")
        .arg(&managed_dir)
        .output()
        .expect("first afs install should run");

    assert!(first.status.success(), "first afs install should succeed");
    let identity_after_first =
        std::fs::read_to_string(managed_dir.join(".afs/identity")).expect("identity should exist");
    let baseline_commit_after_first =
        std::fs::read_to_string(managed_dir.join(".afs/history/repo/refs/heads/afs"))
            .expect("baseline commit ref should exist");

    let second = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("install")
        .arg(&managed_dir)
        .output()
        .expect("second afs install should run");

    assert!(second.status.success(), "second afs install should succeed");
    assert_eq!(String::from_utf8_lossy(&second.stderr), "");
    assert!(
        String::from_utf8_lossy(&second.stdout).contains("already managed directory"),
        "second install should report the existing Managed Directory"
    );
    assert_eq!(
        std::fs::read_to_string(managed_dir.join(".afs/identity"))
            .expect("identity should still exist"),
        identity_after_first
    );
    assert_eq!(
        std::fs::read_to_string(managed_dir.join(".afs/history/repo/refs/heads/afs"))
            .expect("baseline commit ref should still exist"),
        baseline_commit_after_first,
        "idempotent re-install should not add new history commits"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn agents_lists_installed_directory_with_live_runtime_status() {
    let afs_home = unique_afs_home("agents-live-status");
    let managed_dir = unique_afs_home("managed-live-status");
    let pi_runtime = fake_pi_runtime("agents-live-status-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("install")
        .arg(&managed_dir)
        .output()
        .expect("afs install should run");
    assert!(install.status.success(), "afs install should succeed");

    assert!(
        wait_until(Duration::from_secs(3), || {
            let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
                .env("AFS_HOME", &afs_home)
                .arg("agents")
                .output()
                .expect("afs agents should run");
            agents.status.success()
                && String::from_utf8_lossy(&agents.stdout).contains("index=ready(files=0)")
        }),
        "afs agents should report index=ready(files=0) for an empty managed directory"
    );

    let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("agents")
        .output()
        .expect("afs agents should run");

    assert!(agents.status.success(), "afs agents should succeed");
    assert_eq!(String::from_utf8_lossy(&agents.stderr), "");
    let stdout = String::from_utf8_lossy(&agents.stdout);
    assert!(
        stdout.contains(&managed_dir.display().to_string()),
        "afs agents should show the Managed Directory path"
    );
    assert!(
        stdout.contains("health=running"),
        "afs agents should show basic live health"
    );
    assert!(
        stdout.contains("reconciliation=idle"),
        "afs agents should show reconciliation state"
    );
    assert!(
        stdout.contains("queue=0"),
        "afs agents should show Task Queue length"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn history_shows_live_external_change_recorded_from_filesystem_events() {
    let afs_home = unique_afs_home("history-live-external-change");
    let managed_dir = unique_afs_home("managed-live-external-change");
    let pi_runtime = fake_pi_runtime("history-live-external-change-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(managed_dir.join("notes.txt"), "hello afs\n")
        .expect("test should create managed file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("install")
        .arg(&managed_dir)
        .output()
        .expect("afs install should run");
    assert!(install.status.success(), "afs install should succeed");

    std::fs::write(managed_dir.join("notes.txt"), "hello afs\nnow tracked\n")
        .expect("test should modify managed file");

    assert!(
        wait_until(Duration::from_secs(3), || {
            let history = afs_history(&afs_home, &managed_dir);
            history.status.success()
                && String::from_utf8_lossy(&history.stdout).contains("type=external")
        }),
        "afs history should show the live External Change"
    );

    let history = afs_history(&afs_home, &managed_dir);
    assert!(history.status.success(), "afs history should succeed");
    assert_eq!(String::from_utf8_lossy(&history.stderr), "");
    let stdout = String::from_utf8_lossy(&history.stdout);
    assert!(
        stdout
            .lines()
            .next()
            .unwrap_or_default()
            .contains("timestamp="),
        "afs history should show a timestamp"
    );
    assert!(
        stdout.contains("type=external"),
        "afs history should show the History Entry type"
    );
    assert!(
        stdout.contains("summary=External change: notes.txt"),
        "afs history should show a short summary with the affected path"
    );
    assert!(
        stdout.contains("files=1"),
        "afs history should show the affected file count"
    );
    assert!(
        stdout.contains("undoable=yes"),
        "afs history should show current undoability"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn history_normalizes_editor_atomic_save_burst_to_final_file_change() {
    let afs_home = unique_afs_home("history-editor-save");
    let managed_dir = unique_afs_home("managed-editor-save");
    let pi_runtime = fake_pi_runtime("history-editor-save-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(managed_dir.join("notes.txt"), "before\n")
        .expect("test should create managed file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("install")
        .arg(&managed_dir)
        .output()
        .expect("afs install should run");
    assert!(install.status.success(), "afs install should succeed");

    let temp_file = managed_dir.join(".notes.txt.swp");
    std::fs::write(&temp_file, "after\n").expect("editor temp file should be written");
    std::fs::rename(&temp_file, managed_dir.join("notes.txt"))
        .expect("editor save should atomically replace final file");

    assert!(
        wait_until(Duration::from_secs(3), || {
            let history = afs_history(&afs_home, &managed_dir);
            history.status.success()
                && String::from_utf8_lossy(&history.stdout).contains("type=external")
        }),
        "afs history should show the normalized External Change"
    );

    let history = afs_history(&afs_home, &managed_dir);
    assert!(history.status.success(), "afs history should succeed");
    let stdout = String::from_utf8_lossy(&history.stdout);
    let entries = stdout.lines().collect::<Vec<_>>();
    assert_eq!(
        entries.len(),
        1,
        "editor save burst should become one History Entry"
    );
    assert!(
        entries[0].contains("summary=External change: notes.txt"),
        "history should describe the final saved path"
    );
    assert!(
        !entries[0].contains(".notes.txt.swp"),
        "history should not expose the removed editor temp file"
    );
    assert!(
        entries[0].contains("files=1"),
        "history should count only the final file"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn restart_reconciliation_records_missed_changes_as_one_history_batch() {
    let afs_home = unique_afs_home("history-reconciliation");
    let managed_dir = unique_afs_home("managed-reconciliation");
    let pi_runtime = fake_pi_runtime("history-reconciliation-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(managed_dir.join("notes.txt"), "before\n")
        .expect("test should create managed file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("install")
        .arg(&managed_dir)
        .output()
        .expect("afs install should run");
    assert!(install.status.success(), "afs install should succeed");

    stop_daemon(&mut daemon);

    std::fs::write(managed_dir.join("offline-a.txt"), "created while stopped\n")
        .expect("test should create first offline change");
    std::fs::write(
        managed_dir.join("offline-b.txt"),
        "also created while stopped\n",
    )
    .expect("test should create second offline change");

    let mut restarted_daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(3), || {
            let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
                .env("AFS_HOME", &afs_home)
                .arg("agents")
                .output()
                .expect("afs agents should run");
            agents.status.success()
                && String::from_utf8_lossy(&agents.stdout)
                    .contains("reconciliation=complete(changed_files=2)")
        }),
        "restarted agent should report completed Startup Reconciliation"
    );

    let history = afs_history(&afs_home, &managed_dir);
    assert!(history.status.success(), "afs history should succeed");
    assert_eq!(String::from_utf8_lossy(&history.stderr), "");
    let stdout = String::from_utf8_lossy(&history.stdout);
    let entries = stdout.lines().collect::<Vec<_>>();
    assert_eq!(
        entries.len(),
        1,
        "missed offline changes should become one reconciliation History Entry"
    );
    assert!(
        entries[0].contains("type=reconciliation"),
        "history should identify the Startup Reconciliation entry"
    );
    assert!(
        entries[0].contains("summary=Startup reconciliation: 2 files changed"),
        "history should summarize the reconciliation batch"
    );
    assert!(
        entries[0].contains("files=2"),
        "history should show the batch affected file count"
    );
    assert!(
        entries[0].contains("undoable=yes"),
        "history should show reconciliation undoability"
    );

    stop_daemon(&mut restarted_daemon);
}

#[test]
fn agents_reports_reconciliation_running_then_complete_after_restart() {
    let afs_home = unique_afs_home("reconciliation-status");
    let managed_dir = unique_afs_home("reconciliation-status-managed");
    let pi_runtime = fake_pi_runtime("reconciliation-status-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(managed_dir.join("notes.txt"), "before\n")
        .expect("test should create managed file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    stop_daemon(&mut daemon);

    std::fs::write(managed_dir.join("offline.txt"), "created while stopped\n")
        .expect("test should create offline change");

    let mut restarted_daemon = start_daemon_with_reconciliation_delay(&afs_home, &pi_runtime, 1000);
    await_socket(&socket_path);

    let mut last_agents = String::new();
    let saw_running = wait_until(Duration::from_secs(2), || {
        let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
            .env("AFS_HOME", &afs_home)
            .arg("agents")
            .output()
            .expect("afs agents should run");
        if !agents.status.success() {
            return false;
        }
        last_agents = String::from_utf8_lossy(&agents.stdout).to_string();
        last_agents.contains("reconciliation=running")
    });
    assert!(
        saw_running,
        "afs agents should expose startup reconciliation while it is in progress; last output:\n{last_agents}"
    );

    let saw_complete = wait_until(Duration::from_secs(5), || {
        let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
            .env("AFS_HOME", &afs_home)
            .arg("agents")
            .output()
            .expect("afs agents should run");
        if !agents.status.success() {
            return false;
        }
        last_agents = String::from_utf8_lossy(&agents.stdout).to_string();
        last_agents.contains("reconciliation=complete(changed_files=1)")
    });
    assert!(
        saw_complete,
        "afs agents should report completed startup reconciliation with changed file count; last output:\n{last_agents}"
    );

    stop_daemon(&mut restarted_daemon);
}

#[test]
fn direct_ask_warns_when_startup_reconciliation_is_running() {
    let afs_home = unique_afs_home("ask-reconciliation-caveat");
    let managed_dir = unique_afs_home("ask-reconciliation-caveat-managed");
    let pi_runtime = fake_pi_runtime("ask-reconciliation-caveat-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let target = managed_dir.join("notes.txt");
    std::fs::write(&target, "before\n").expect("test should create managed file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let target = target.canonicalize().expect("target should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    stop_daemon(&mut daemon);

    std::fs::write(&target, "changed while stopped\n")
        .expect("test should change the managed file offline");

    let mut restarted_daemon = start_daemon_with_reconciliation_delay(&afs_home, &pi_runtime, 1000);
    await_socket(&socket_path);
    let agents = await_index_token(&afs_home, "reconciliation=running", Duration::from_secs(2));
    assert!(
        agents.contains("reconciliation=running"),
        "test should observe reconciliation running before ask; got:\n{agents}"
    );

    let ask = afs_ask(&afs_home, &format!("summarize {}", target.display()));
    assert!(
        ask.status.success(),
        "afs ask should succeed during reconciliation"
    );
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains("caveat: startup reconciliation is running; answer may be incomplete"),
        "afs ask should explain that reconciliation is still running; got:\n{stdout}"
    );

    stop_daemon(&mut restarted_daemon);
}

#[test]
fn live_edit_during_startup_reconciliation_is_recorded_as_external_change() {
    let afs_home = unique_afs_home("reconciliation-live-edit");
    let managed_dir = unique_afs_home("reconciliation-live-edit-managed");
    let pi_runtime = fake_pi_runtime("reconciliation-live-edit-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(managed_dir.join("baseline.txt"), "before\n")
        .expect("test should create baseline file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    stop_daemon(&mut daemon);

    std::fs::write(managed_dir.join("offline.txt"), "changed while stopped\n")
        .expect("test should create offline change");

    let mut restarted_daemon = start_daemon_with_reconciliation_delay(&afs_home, &pi_runtime, 1000);
    await_socket(&socket_path);
    await_index_token(&afs_home, "reconciliation=running", Duration::from_secs(2));

    std::fs::write(managed_dir.join("live.txt"), "changed after restart\n")
        .expect("test should create live edit during reconciliation");

    assert!(
        wait_until(Duration::from_secs(5), || {
            let history = afs_history(&afs_home, &managed_dir);
            if !history.status.success() {
                return false;
            }
            let stdout = String::from_utf8_lossy(&history.stdout);
            stdout.contains("type=external")
                && stdout.contains("summary=External change: live.txt")
                && stdout.contains("type=reconciliation")
                && stdout.contains("summary=Startup reconciliation: offline.txt")
        }),
        "live edit during startup reconciliation should remain an External Change"
    );

    stop_daemon(&mut restarted_daemon);
}

#[test]
fn remove_cancels_pending_startup_reconciliation_without_recreating_agent_home() {
    let afs_home = unique_afs_home("reconciliation-remove");
    let managed_dir = unique_afs_home("reconciliation-remove-managed");
    let pi_runtime = fake_pi_runtime("reconciliation-remove-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(managed_dir.join("baseline.txt"), "before\n")
        .expect("test should create baseline file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let agent_home = managed_dir.join(".afs");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    stop_daemon(&mut daemon);

    std::fs::write(managed_dir.join("offline.txt"), "changed while stopped\n")
        .expect("test should create offline change");

    let mut restarted_daemon = start_daemon_with_reconciliation_delay(&afs_home, &pi_runtime, 1000);
    await_socket(&socket_path);
    await_index_token(&afs_home, "reconciliation=running", Duration::from_secs(2));

    let remove = remove_managed_dir_with_flags(&afs_home, &managed_dir, &["--discard-history"]);
    assert!(
        remove.status.success(),
        "afs remove --discard-history should succeed while reconciliation is running\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&remove.stdout),
        String::from_utf8_lossy(&remove.stderr)
    );

    std::thread::sleep(Duration::from_millis(1200));

    assert!(
        !agent_home.exists(),
        "cancelled reconciliation must not recreate the removed Agent Home"
    );

    stop_daemon(&mut restarted_daemon);
}

#[test]
fn history_lists_newest_entries_first() {
    let afs_home = unique_afs_home("history-newest-first");
    let managed_dir = unique_afs_home("managed-newest-first");
    let pi_runtime = fake_pi_runtime("history-newest-first-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("install")
        .arg(&managed_dir)
        .output()
        .expect("afs install should run");
    assert!(install.status.success(), "afs install should succeed");

    std::fs::write(managed_dir.join("first.txt"), "first\n")
        .expect("test should create first managed change");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let history = afs_history(&afs_home, &managed_dir);
            history.status.success()
                && String::from_utf8_lossy(&history.stdout).contains("first.txt")
        }),
        "first External Change should reach history"
    );

    std::fs::write(managed_dir.join("second.txt"), "second\n")
        .expect("test should create second managed change");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let history = afs_history(&afs_home, &managed_dir);
            let stdout = String::from_utf8_lossy(&history.stdout);
            history.status.success()
                && stdout.lines().count() == 2
                && stdout
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .contains("second.txt")
        }),
        "newest External Change should appear first"
    );

    let history = afs_history(&afs_home, &managed_dir);
    let stdout = String::from_utf8_lossy(&history.stdout);
    let entries = stdout.lines().collect::<Vec<_>>();
    assert!(
        entries[0].contains("second.txt"),
        "newest history entry should be first"
    );
    assert!(
        entries[1].contains("first.txt"),
        "older history entry should follow"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn history_backend_does_not_touch_surrounding_project_git_repository() {
    let afs_home = unique_afs_home("history-isolation");
    let project_dir = unique_afs_home("history-isolation-project");
    let pi_runtime = fake_pi_runtime("history-isolation-runtime");
    let socket_path = supervisor_socket(&afs_home);

    std::fs::create_dir_all(&project_dir).expect("test should create project directory");
    let init = Command::new("git")
        .arg("-c")
        .arg("init.defaultBranch=main")
        .arg("init")
        .arg("--quiet")
        .arg(&project_dir)
        .output()
        .expect("git init should run");
    assert!(init.status.success(), "surrounding git init should succeed");
    let committer = [
        ("GIT_AUTHOR_NAME", "Test"),
        ("GIT_AUTHOR_EMAIL", "test@example.com"),
        ("GIT_COMMITTER_NAME", "Test"),
        ("GIT_COMMITTER_EMAIL", "test@example.com"),
    ];
    std::fs::write(project_dir.join("README.md"), "project\n")
        .expect("test should create project file");
    let add = Command::new("git")
        .current_dir(&project_dir)
        .arg("add")
        .arg("README.md")
        .output()
        .expect("git add should run");
    assert!(add.status.success(), "project git add should succeed");
    let initial_commit = Command::new("git")
        .current_dir(&project_dir)
        .envs(committer)
        .arg("commit")
        .arg("--quiet")
        .arg("-m")
        .arg("initial")
        .output()
        .expect("git commit should run");
    assert!(
        initial_commit.status.success(),
        "project git commit should succeed"
    );

    let managed_dir = project_dir.join("managed");
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(managed_dir.join("notes.txt"), "before\n")
        .expect("test should create managed file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");

    let project_git_head_before = std::fs::read_to_string(project_dir.join(".git/refs/heads/main"))
        .expect("surrounding repo should have main branch");
    let project_config_before = std::fs::read_to_string(project_dir.join(".git/config"))
        .expect("surrounding repo should have config");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");
    std::fs::write(managed_dir.join("notes.txt"), "after\n")
        .expect("test should trigger external change");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let history = afs_history(&afs_home, &managed_dir);
            history.status.success()
                && String::from_utf8_lossy(&history.stdout).contains("type=external")
        }),
        "afs history should record the external change"
    );
    let history = afs_history(&afs_home, &managed_dir);
    let stdout = String::from_utf8_lossy(&history.stdout);
    let entry = history_entry_id(stdout.lines().next().expect("history should have an entry"));
    let undo = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("undo")
        .arg(&managed_dir)
        .arg(entry)
        .arg("--yes")
        .output()
        .expect("afs undo should run");
    assert!(undo.status.success(), "afs undo --yes should succeed");

    let project_git_head_after = std::fs::read_to_string(project_dir.join(".git/refs/heads/main"))
        .expect("surrounding repo main branch should still exist");
    assert_eq!(
        project_git_head_before, project_git_head_after,
        "AFS must not create commits in the surrounding project git repository"
    );
    let project_config_after = std::fs::read_to_string(project_dir.join(".git/config"))
        .expect("surrounding repo config should still exist");
    assert_eq!(
        project_config_before, project_config_after,
        "AFS must not modify the surrounding project git config"
    );
    let status = Command::new("git")
        .current_dir(&project_dir)
        .arg("status")
        .arg("--porcelain")
        .output()
        .expect("git status should run");
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(
        !status_stdout.contains(" managed/notes.txt"),
        "surrounding project should not see undo-managed file as modified:\n{status_stdout}"
    );
    assert!(
        managed_dir.join(".afs/history/repo/HEAD").is_file(),
        "AFS should keep its own history repo inside the Agent Home"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn gitignored_files_in_managed_dir_are_still_tracked_and_restored_on_undo() {
    let afs_home = unique_afs_home("gitignore-tracked");
    let managed_dir = unique_afs_home("managed-gitignore-tracked");
    let pi_runtime = fake_pi_runtime("gitignore-tracked-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(managed_dir.join(".gitignore"), "ignored.log\n")
        .expect("test should create .gitignore");
    let ignored_path = managed_dir.join("ignored.log");
    std::fs::write(&ignored_path, "before\n").expect("test should create ignored file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let ignored_path = ignored_path
        .canonicalize()
        .expect("ignored file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    std::fs::write(&ignored_path, "after\n").expect("test should modify ignored file");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let history = afs_history(&afs_home, &managed_dir);
            history.status.success()
                && String::from_utf8_lossy(&history.stdout).contains("ignored.log")
        }),
        "AFS history should record changes to .gitignored files"
    );

    let history = afs_history(&afs_home, &managed_dir);
    let stdout = String::from_utf8_lossy(&history.stdout);
    let entry = history_entry_id(stdout.lines().next().expect("history should have an entry"));
    let undo = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("undo")
        .arg(&managed_dir)
        .arg(entry)
        .arg("--yes")
        .output()
        .expect("afs undo should run");
    assert!(
        undo.status.success(),
        "afs undo --yes should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&undo.stdout),
        String::from_utf8_lossy(&undo.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&ignored_path).expect("ignored file should still be readable"),
        "before\n",
        "undo should restore .gitignored file contents captured at baseline"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn install_without_config_fails_with_authentication_required_message() {
    let afs_home = unique_afs_home("install-no-config");
    let mut daemon = start_daemon(&afs_home);
    assert!(
        wait_until(Duration::from_secs(2), || supervisor_socket(&afs_home)
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "afs daemon should create a Unix supervisor socket"
    );

    let managed_dir = unique_afs_home("install-no-config-target");
    std::fs::create_dir_all(&managed_dir).expect("test should create target directory");

    let output = install_managed_dir(&afs_home, &managed_dir);
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        !output.status.success(),
        "afs install should fail without config. stderr={stderr}"
    );
    assert!(
        stderr.contains("authentication required"),
        "stderr should mention authentication. got: {stderr}"
    );
    assert!(
        stderr.contains("afs login"),
        "stderr should direct the user to `afs login`. got: {stderr}"
    );
    for token in ["Pi", "pi ", "pi,", "pi."] {
        assert!(
            !stderr.contains(token),
            "stderr should not leak the word pi. got: {stderr}"
        );
    }
    assert!(
        !managed_dir.join(".afs").exists(),
        "install failure should not leave a partial .afs/ directory"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn install_with_valid_config_spawns_agent_runtime_with_provider_and_model() {
    let afs_home = unique_afs_home("install-with-config");
    let pi_runtime = fake_pi_runtime("install-with-config-runtime");

    write_config(
        &afs_home,
        r#"{"provider":"claude","model":"claude-sonnet-4-6","auth_method":"oauth"}"#,
    );

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || supervisor_socket(&afs_home)
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "afs daemon should create a Unix supervisor socket"
    );

    let managed_dir = unique_afs_home("install-with-config-target");
    std::fs::create_dir_all(&managed_dir).expect("test should create target directory");

    let output = install_managed_dir(&afs_home, &managed_dir);
    assert!(
        output.status.success(),
        "afs install should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let managed_dir_canonical = managed_dir
        .canonicalize()
        .expect("canonicalize managed dir");
    let spawn_observed = managed_dir_canonical.join(".afs").join("spawn-observed");
    assert!(
        wait_until(Duration::from_secs(2), || std::fs::read_to_string(
            &spawn_observed
        )
        .map(|body| body.contains("done=1"))
        .unwrap_or(false)),
        "fake agent runtime should record its spawn observations"
    );
    let observed =
        std::fs::read_to_string(&spawn_observed).expect("test should read spawn observations");
    assert!(
        observed.contains("arg=--mode") && observed.contains("arg=rpc"),
        "runtime should be spawned in RPC mode. got:\n{observed}"
    );
    assert!(
        observed.contains("arg=--provider") && observed.contains("arg=claude"),
        "runtime should receive --provider claude. got:\n{observed}"
    );
    assert!(
        observed.contains("arg=--model") && observed.contains("arg=claude-sonnet-4-6"),
        "runtime should receive --model from config. got:\n{observed}"
    );
    assert!(
        observed.contains("arg=-e") && observed.contains("afs_reply.ts"),
        "runtime should receive -e <path-to-afs_reply.ts>. got:\n{observed}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn install_with_config_without_model_omits_model_flag() {
    let afs_home = unique_afs_home("install-config-no-model");
    let pi_runtime = fake_pi_runtime("install-config-no-model-runtime");

    write_config(&afs_home, r#"{"provider":"openai","auth_method":"oauth"}"#);

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || supervisor_socket(&afs_home)
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "afs daemon should create a Unix supervisor socket"
    );

    let managed_dir = unique_afs_home("install-config-no-model-target");
    std::fs::create_dir_all(&managed_dir).expect("test should create target directory");

    let output = install_managed_dir(&afs_home, &managed_dir);
    assert!(
        output.status.success(),
        "afs install should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let managed_dir_canonical = managed_dir
        .canonicalize()
        .expect("canonicalize managed dir");
    let spawn_observed = managed_dir_canonical.join(".afs").join("spawn-observed");
    assert!(
        wait_until(Duration::from_secs(2), || std::fs::read_to_string(
            &spawn_observed
        )
        .map(|body| body.contains("done=1"))
        .unwrap_or(false)),
        "fake agent runtime should record its spawn observations"
    );
    let observed =
        std::fs::read_to_string(&spawn_observed).expect("test should read spawn observations");
    assert!(
        observed.contains("arg=--provider") && observed.contains("arg=openai"),
        "runtime should receive --provider openai. got:\n{observed}"
    );
    assert!(
        !observed.contains("arg=--model"),
        "runtime should not receive --model when unset in config. got:\n{observed}"
    );
    assert!(
        observed.contains("arg=-e") && observed.contains("afs_reply.ts"),
        "runtime should receive -e <path-to-afs_reply.ts>. got:\n{observed}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn install_forwards_runtime_provider_id_to_pi() {
    let afs_home = unique_afs_home("install-runtime-provider-id");
    let pi_runtime = fake_pi_runtime("install-runtime-provider-id-runtime");

    write_config(
        &afs_home,
        r#"{"provider":"openai","auth_method":"oauth","runtime_provider_id":"openai-codex"}"#,
    );

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || supervisor_socket(&afs_home)
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "afs daemon should create a Unix supervisor socket"
    );

    let managed_dir = unique_afs_home("install-runtime-provider-id-target");
    std::fs::create_dir_all(&managed_dir).expect("test should create target directory");

    let output = install_managed_dir(&afs_home, &managed_dir);
    assert!(
        output.status.success(),
        "afs install should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let managed_dir_canonical = managed_dir
        .canonicalize()
        .expect("canonicalize managed dir");
    let spawn_observed = managed_dir_canonical.join(".afs").join("spawn-observed");
    assert!(
        wait_until(Duration::from_secs(2), || std::fs::read_to_string(
            &spawn_observed
        )
        .map(|body| body.contains("done=1"))
        .unwrap_or(false)),
        "fake agent runtime should record its spawn observations"
    );
    let observed =
        std::fs::read_to_string(&spawn_observed).expect("test should read spawn observations");
    assert!(
        observed.contains("arg=--provider") && observed.contains("arg=openai-codex"),
        "runtime should receive --provider openai-codex when runtime_provider_id is set. got:\n{observed}"
    );
    assert!(
        !observed.contains("arg=openai\n"),
        "runtime should not receive bare --provider openai when runtime_provider_id overrides it. got:\n{observed}"
    );
    assert!(
        observed.contains("arg=-e") && observed.contains("afs_reply.ts"),
        "runtime should receive -e <path-to-afs_reply.ts>. got:\n{observed}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn install_with_api_key_config_forwards_named_env_var_to_runtime() {
    let afs_home = unique_afs_home("install-api-key");
    let runtime_dir = unique_afs_home("install-api-key-runtime");
    std::fs::create_dir_all(&runtime_dir).expect("test should create fake runtime dir");
    let pi_runtime = runtime_dir.join("pi");
    // Re-use the shared fake script via a helper function body inlined:
    std::fs::write(
        &pi_runtime,
        r#"#!/bin/sh
{
  printf 'env_ANTHROPIC_API_KEY=%s\n' "${ANTHROPIC_API_KEY-}"
  for arg in "$@"; do
    printf 'arg=%s\n' "$arg"
  done
  printf 'done=1\n'
} > "$AFS_AGENT_HOME/spawn-observed"
# Keep the child alive to mimic an agent; the test only inspects spawn-observed.
cat >/dev/null
"#,
    )
    .expect("test should write fake runtime");
    let mut permissions = std::fs::metadata(&pi_runtime)
        .expect("test should read runtime metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&pi_runtime, permissions)
        .expect("test should set runtime permissions");

    write_config(
        &afs_home,
        r#"{"provider":"claude","auth_method":"api_key","api_key_env":"ANTHROPIC_API_KEY"}"#,
    );

    let mut daemon = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .env("AFS_PI_RUNTIME", &pi_runtime)
        .env("ANTHROPIC_API_KEY", "fake-key-for-test")
        .arg("daemon")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("afs daemon should start");
    assert!(
        wait_until(Duration::from_secs(2), || supervisor_socket(&afs_home)
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "afs daemon should create a Unix supervisor socket"
    );

    let managed_dir = unique_afs_home("install-api-key-target");
    std::fs::create_dir_all(&managed_dir).expect("test should create target directory");

    let output = install_managed_dir(&afs_home, &managed_dir);
    assert!(
        output.status.success(),
        "afs install should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let managed_dir_canonical = managed_dir
        .canonicalize()
        .expect("canonicalize managed dir");
    let spawn_observed = managed_dir_canonical.join(".afs").join("spawn-observed");
    assert!(
        wait_until(Duration::from_secs(2), || std::fs::read_to_string(
            &spawn_observed
        )
        .map(|body| body.contains("done=1"))
        .unwrap_or(false)),
        "fake agent runtime should record its spawn observations"
    );
    let observed =
        std::fs::read_to_string(&spawn_observed).expect("test should read spawn observations");
    assert!(
        observed.contains("env_ANTHROPIC_API_KEY=fake-key-for-test"),
        "runtime should receive the api_key_env value. got:\n{observed}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn login_writes_config_after_runtime_populates_auth_json() {
    let afs_home = unique_afs_home("login-success");
    let home_dir = unique_afs_home("login-success-home");
    std::fs::create_dir_all(&home_dir).expect("test should create home dir");
    let pi_runtime = fake_pi_login_runtime("login-success-runtime", true, "claude");

    let output = run_afs_login(
        &home_dir,
        &afs_home,
        &pi_runtime,
        &["--provider", "claude"],
        true,
    );
    assert!(
        output.status.success(),
        "afs login should succeed. stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let config_body = std::fs::read_to_string(afs_home.join("config.json"))
        .expect("config.json should exist after login");
    assert!(
        config_body.contains("\"provider\": \"claude\""),
        "config should record provider. got: {config_body}"
    );
    assert!(
        config_body.contains("\"auth_method\": \"oauth\""),
        "config should record oauth auth_method. got: {config_body}"
    );
    assert!(
        config_body.contains("\"runtime_provider_id\": \"anthropic\""),
        "config should record matched Pi auth.json key. got: {config_body}"
    );
    assert!(
        !config_body.contains("\"model\""),
        "login should not pin a model. got: {config_body}"
    );
}

#[test]
fn login_openai_accepts_codex_oauth_auth_key() {
    let afs_home = unique_afs_home("login-openai-codex");
    let home_dir = unique_afs_home("login-openai-codex-home");
    std::fs::create_dir_all(&home_dir).expect("test should create home dir");
    let pi_runtime = fake_pi_login_runtime("login-openai-codex-runtime", true, "openai-codex");

    let output = run_afs_login(
        &home_dir,
        &afs_home,
        &pi_runtime,
        &["--provider", "openai"],
        true,
    );
    assert!(
        output.status.success(),
        "afs login --provider openai should accept openai-codex auth.json key. stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let config_body = std::fs::read_to_string(afs_home.join("config.json"))
        .expect("config.json should exist after login");
    assert!(
        config_body.contains("\"provider\": \"openai\""),
        "config should record user-facing provider as openai. got: {config_body}"
    );
    assert!(
        config_body.contains("\"runtime_provider_id\": \"openai-codex\""),
        "config should record matched Pi auth.json key. got: {config_body}"
    );
}

#[test]
fn login_fails_when_runtime_exits_without_populating_auth_json() {
    let afs_home = unique_afs_home("login-no-auth");
    let home_dir = unique_afs_home("login-no-auth-home");
    std::fs::create_dir_all(&home_dir).expect("test should create home dir");
    let pi_runtime = fake_pi_login_runtime_exits_zero_no_auth("login-no-auth-runtime");

    let output = run_afs_login(
        &home_dir,
        &afs_home,
        &pi_runtime,
        &["--provider", "claude"],
        true,
    );
    assert!(
        !output.status.success(),
        "afs login should fail when auth was not written"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("authentication did not complete"),
        "stderr should explain verification failure. got: {stderr}"
    );
    assert!(
        !afs_home.join("config.json").exists(),
        "config.json should not be written on login failure"
    );
}

#[test]
fn login_fails_when_runtime_exits_nonzero() {
    let afs_home = unique_afs_home("login-runtime-failed");
    let home_dir = unique_afs_home("login-runtime-failed-home");
    std::fs::create_dir_all(&home_dir).expect("test should create home dir");
    let pi_runtime = fake_pi_login_runtime("login-runtime-failed-runtime", false, "claude");

    let output = run_afs_login(
        &home_dir,
        &afs_home,
        &pi_runtime,
        &["--provider", "claude"],
        true,
    );
    assert!(
        !output.status.success(),
        "afs login should fail when runtime exits non-zero"
    );
    assert!(
        !afs_home.join("config.json").exists(),
        "config.json should not be written on runtime failure"
    );
}

#[test]
fn login_rejects_missing_provider_argument() {
    let afs_home = unique_afs_home("login-missing-provider");
    let home_dir = unique_afs_home("login-missing-provider-home");
    std::fs::create_dir_all(&home_dir).expect("test should create home dir");
    let pi_runtime = fake_pi_login_runtime("login-missing-provider-runtime", true, "claude");

    let output = run_afs_login(&home_dir, &afs_home, &pi_runtime, &[], true);
    assert!(
        !output.status.success(),
        "afs login without --provider should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("claude") && stderr.contains("openai"),
        "stderr should name supported providers. got: {stderr}"
    );
}

#[test]
fn login_rejects_unsupported_provider() {
    let afs_home = unique_afs_home("login-bad-provider");
    let home_dir = unique_afs_home("login-bad-provider-home");
    std::fs::create_dir_all(&home_dir).expect("test should create home dir");
    let pi_runtime = fake_pi_login_runtime("login-bad-provider-runtime", true, "claude");

    let output = run_afs_login(
        &home_dir,
        &afs_home,
        &pi_runtime,
        &["--provider", "gemini"],
        true,
    );
    assert!(
        !output.status.success(),
        "afs login with unsupported provider should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("gemini") || stderr.contains("not supported"),
        "stderr should explain unsupported provider. got: {stderr}"
    );
    assert!(
        !stderr.to_lowercase().contains(" pi "),
        "stderr should not leak pi branding. got: {stderr}"
    );
}

#[test]
fn login_requires_interactive_terminal() {
    let afs_home = unique_afs_home("login-no-tty");
    let home_dir = unique_afs_home("login-no-tty-home");
    std::fs::create_dir_all(&home_dir).expect("test should create home dir");
    let pi_runtime = fake_pi_login_runtime("login-no-tty-runtime", true, "claude");

    let output = run_afs_login(
        &home_dir,
        &afs_home,
        &pi_runtime,
        &["--provider", "claude"],
        false,
    );
    assert!(
        !output.status.success(),
        "afs login should refuse without a terminal"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("interactive terminal"),
        "stderr should mention the tty requirement. got: {stderr}"
    );
}

#[test]
fn install_with_api_key_config_missing_api_key_env_field_fails_cleanly() {
    let afs_home = unique_afs_home("install-api-key-missing-env");
    let pi_runtime = fake_pi_runtime("install-api-key-missing-env-runtime");

    // Write a broken config: api_key auth_method without api_key_env field.
    write_config(
        &afs_home,
        r#"{"provider":"claude","auth_method":"api_key"}"#,
    );

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || supervisor_socket(&afs_home)
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "afs daemon should create a Unix supervisor socket"
    );

    let managed_dir = unique_afs_home("install-api-key-missing-env-target");
    std::fs::create_dir_all(&managed_dir).expect("test should create target directory");

    let output = install_managed_dir(&afs_home, &managed_dir);
    assert!(
        !output.status.success(),
        "afs install with broken config should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("api_key_env"),
        "stderr should mention the missing field. got: {stderr}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn ask_routes_explicit_symlink_path_to_owning_nested_child() {
    let afs_home = unique_afs_home("ask-symlink-nested-child");
    let parent_dir = unique_afs_home("ask-symlink-parent");
    let pi_runtime = fake_pi_runtime("ask-symlink-runtime");
    let socket_path = supervisor_socket(&afs_home);
    let child_dir = parent_dir.join("child");
    std::fs::create_dir_all(&child_dir).expect("test should create nested managed directory");
    let target_file = child_dir.join("notes.txt");
    std::fs::write(&target_file, "child notes\n").expect("test should create target file");
    let parent_dir = parent_dir
        .canonicalize()
        .expect("parent managed directory should canonicalize");
    let child_dir = child_dir
        .canonicalize()
        .expect("child managed directory should canonicalize");
    let target_file = target_file
        .canonicalize()
        .expect("target file should canonicalize");
    let link_path = parent_dir.join("link-to-child-notes");
    symlink(&target_file, &link_path).expect("test should create symlink into nested child");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let parent_install = install_managed_dir(&afs_home, &parent_dir);
    assert!(
        parent_install.status.success(),
        "parent install should succeed\nstderr:\n{}",
        String::from_utf8_lossy(&parent_install.stderr)
    );
    let child_install = install_managed_dir(&afs_home, &child_dir);
    assert!(
        child_install.status.success(),
        "child install should succeed\nstderr:\n{}",
        String::from_utf8_lossy(&child_install.stderr)
    );

    let ask = afs_ask(&afs_home, &format!("summarize {}", link_path.display()));
    assert!(
        ask.status.success(),
        "afs ask via symlink should succeed\nstderr:\n{}",
        String::from_utf8_lossy(&ask.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&ask.stderr), "");

    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains(&format!("answered about {}", target_file.display())),
        "afs ask should route to the canonical target owner\nstdout:\n{}",
        stdout
    );
    assert!(
        wait_until(Duration::from_secs(2), || child_dir
            .join(".afs/ask-received")
            .is_file()),
        "nested child should receive the ask for the symlink target"
    );
    assert!(
        std::fs::read_to_string(child_dir.join(".afs/ask-received"))
            .expect("child ask marker should be readable")
            .contains(&target_file.display().to_string()),
        "child should receive the canonical target path, not the symlink path"
    );
    assert!(
        !parent_dir.join(".afs/ask-received").exists(),
        "parent should not receive ask when symlink resolves into nested child"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn ask_reports_symlink_to_external_target_as_unmanaged() {
    let afs_home = unique_afs_home("ask-symlink-external");
    let managed_dir = unique_afs_home("ask-symlink-external-managed");
    let external_dir = unique_afs_home("ask-symlink-external-target");
    let pi_runtime = fake_pi_runtime("ask-symlink-external-runtime");
    let socket_path = supervisor_socket(&afs_home);

    std::fs::create_dir_all(&managed_dir).expect("test should create managed dir");
    std::fs::create_dir_all(&external_dir).expect("test should create external dir");
    let external_secret = external_dir.join("secret.txt");
    std::fs::write(&external_secret, "secret\n").expect("test should create external target");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed dir should canonicalize");
    let external_secret = external_secret
        .canonicalize()
        .expect("external secret should canonicalize");
    let link_path = managed_dir.join("link-to-external");
    symlink(&external_secret, &link_path).expect("test should create symlink to external target");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(
        install.status.success(),
        "managed install should succeed\nstderr:\n{}",
        String::from_utf8_lossy(&install.stderr)
    );

    let ask = afs_ask(&afs_home, &format!("inspect {}", link_path.display()));
    assert!(
        !ask.status.success(),
        "afs ask should fail when symlink target escapes managed dir"
    );
    let stderr = String::from_utf8_lossy(&ask.stderr);
    assert!(
        stderr.contains("path is not managed:"),
        "stderr should report unmanaged canonical target\nstderr:\n{}",
        stderr
    );
    assert!(
        stderr.contains(&external_secret.display().to_string()),
        "stderr should name the canonical external target\nstderr:\n{}",
        stderr
    );
    assert!(
        !managed_dir.join(".afs/ask-received").exists(),
        "managed agent should not receive ask routed through external symlink"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn install_via_symlinked_path_stores_canonical_registry_entry() {
    let afs_home = unique_afs_home("install-symlink-alias");
    let real_dir = unique_afs_home("install-symlink-real");
    let alias_parent = unique_afs_home("install-symlink-aliases");
    let pi_runtime = fake_pi_runtime("install-symlink-runtime");
    let socket_path = supervisor_socket(&afs_home);

    std::fs::create_dir_all(&real_dir).expect("test should create real dir");
    std::fs::create_dir_all(&alias_parent).expect("test should create alias parent");
    let real_dir = real_dir
        .canonicalize()
        .expect("real dir should canonicalize");
    let alias_path = alias_parent.join("alias");
    symlink(&real_dir, &alias_path).expect("test should create alias symlink");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let alias_install = install_managed_dir(&afs_home, &alias_path);
    assert!(
        alias_install.status.success(),
        "install via alias should succeed\nstderr:\n{}",
        String::from_utf8_lossy(&alias_install.stderr)
    );

    let registry = std::fs::read_to_string(afs_home.join("registry.tsv"))
        .expect("registry should be readable");
    let entries: Vec<&str> = registry
        .lines()
        .skip(1)
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "registry should record the alias install as a single canonical entry\nregistry:\n{}",
        registry
    );
    let fields: Vec<&str> = entries[0].split('\t').collect();
    assert_eq!(
        fields.get(1).copied(),
        Some(real_dir.to_string_lossy().as_ref()),
        "registry should store canonical managed_dir, not the alias path\nregistry:\n{}",
        registry
    );

    let duplicate = install_managed_dir(&afs_home, &real_dir);
    assert!(
        duplicate.status.success(),
        "installing the canonical path after the alias should succeed"
    );
    assert!(
        String::from_utf8_lossy(&duplicate.stdout).contains("already managed directory"),
        "second install resolving to the same canonical path should report already managed"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn install_survives_self_referential_symlink_loop_inside_managed_dir() {
    let afs_home = unique_afs_home("install-symlink-loop");
    let managed_dir = unique_afs_home("install-symlink-loop-dir");
    let pi_runtime = fake_pi_runtime("install-symlink-loop-runtime");
    let socket_path = supervisor_socket(&afs_home);

    std::fs::create_dir_all(&managed_dir).expect("test should create managed dir");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed dir should canonicalize");
    let loop_path = managed_dir.join("loop");
    symlink(&loop_path, &loop_path).expect("test should create self-referential symlink");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(
        install.status.success(),
        "install should not fail when managed dir contains a symlink loop\nstderr:\n{}",
        String::from_utf8_lossy(&install.stderr)
    );

    let history = afs_history(&afs_home, &managed_dir);
    assert!(
        history.status.success(),
        "history should succeed for managed dir with symlink loop\nstderr:\n{}",
        String::from_utf8_lossy(&history.stderr)
    );

    let change_path = managed_dir.join("notes.txt");
    std::fs::write(&change_path, "hello\n").expect("test should write post-install change");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let out = afs_history(&afs_home, &managed_dir);
            out.status.success() && String::from_utf8_lossy(&out.stdout).contains("External change")
        }),
        "external change should be recorded despite symlink loop in managed dir"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn broadcast_reply_filters_out_reference_symlink_escaping_managed_dir() {
    let afs_home = unique_afs_home("broadcast-symlink-escape");
    let managed_dir = unique_afs_home("broadcast-symlink-managed");
    let external_dir = unique_afs_home("broadcast-symlink-external");
    let pi_runtime = fake_pi_runtime("broadcast-symlink-runtime");
    let socket_path = supervisor_socket(&afs_home);

    std::fs::create_dir_all(&managed_dir).expect("test should create managed dir");
    std::fs::create_dir_all(&external_dir).expect("test should create external dir");
    let sensitive_file = external_dir.join("sensitive.txt");
    std::fs::write(&sensitive_file, "top secret\n").expect("test should create external secret");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed dir should canonicalize");
    let sensitive_file = sensitive_file
        .canonicalize()
        .expect("sensitive file should canonicalize");
    let benign_file = managed_dir.join("benign.txt");
    std::fs::write(&benign_file, "public\n").expect("test should create benign file");
    let benign_file = benign_file
        .canonicalize()
        .expect("benign file should canonicalize");
    let escape_link = managed_dir.join("escape");
    symlink(&sensitive_file, &escape_link).expect("test should create escape symlink");

    let mut daemon =
        start_daemon_with_pi_runtime_and_broadcast_timeout(&afs_home, &pi_runtime, 200);
    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(
        install.status.success(),
        "managed install should succeed\nstderr:\n{}",
        String::from_utf8_lossy(&install.stderr)
    );

    std::fs::write(
        managed_dir.join(".afs/broadcast-response"),
        format!(
            "strong\tfound potentially sensitive content\tI reviewed the files\t{};{}\n",
            escape_link.display(),
            benign_file.display()
        ),
    )
    .expect("test should write broadcast response fixture");

    let ask = afs_ask(&afs_home, "scan for sensitive content");
    assert!(
        ask.status.success(),
        "afs ask should succeed\nstderr:\n{}",
        String::from_utf8_lossy(&ask.stderr)
    );
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains(&format!("- {}", benign_file.display())),
        "broadcast response should include the benign in-tree reference\nstdout:\n{}",
        stdout
    );
    assert!(
        !stdout.contains(&sensitive_file.display().to_string()),
        "broadcast response should not leak the canonical external target\nstdout:\n{}",
        stdout
    );
    assert!(
        !stdout.contains(&escape_link.display().to_string()),
        "broadcast response should filter the escape symlink path\nstdout:\n{}",
        stdout
    );

    stop_daemon(&mut daemon);
}

#[test]
fn install_creates_afs_ignore_file_in_agent_home() {
    let afs_home = unique_afs_home("ignore-file-created");
    let managed_dir = unique_afs_home("managed-ignore-file-created");
    let pi_runtime = fake_pi_runtime("ignore-file-created-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    let ignore_path = managed_dir.join(".afs/ignore");
    assert!(
        ignore_path.is_file(),
        "afs install should create an ignore policy file inside the Agent Home"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn install_seeds_afs_ignore_from_gitignore_when_present() {
    let afs_home = unique_afs_home("ignore-seed-from-gitignore");
    let managed_dir = unique_afs_home("managed-ignore-seed-from-gitignore");
    let pi_runtime = fake_pi_runtime("ignore-seed-from-gitignore-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(managed_dir.join(".gitignore"), "build/\n*.log\n")
        .expect("test should create .gitignore");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    let seeded = std::fs::read_to_string(managed_dir.join(".afs/ignore"))
        .expect("afs install should seed an ignore policy file");
    assert!(
        seeded.contains("build/"),
        "seeded ignore policy should include the .gitignore directory pattern, got:\n{seeded}"
    );
    assert!(
        seeded.contains("*.log"),
        "seeded ignore policy should include the .gitignore glob pattern, got:\n{seeded}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn explicit_ask_on_ignored_path_routes_to_owner_without_falling_through_to_broadcast() {
    let afs_home = unique_afs_home("ignore-explicit-ask");
    let managed_dir = unique_afs_home("managed-ignore-explicit-ask");
    let pi_runtime = fake_pi_runtime("ignore-explicit-ask-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(managed_dir.join(".gitignore"), "ignored.log\n")
        .expect("test should create .gitignore");
    let ignored_path = managed_dir.join("ignored.log");
    std::fs::write(&ignored_path, "before\n").expect("test should create ignored file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let ignored_path = ignored_path
        .canonicalize()
        .expect("ignored file should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    let ask = afs_ask(&afs_home, &ignored_path.display().to_string());
    assert!(
        ask.status.success(),
        "afs ask on an ignored path should still succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&ask.stdout),
        String::from_utf8_lossy(&ask.stderr)
    );
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains("answered about "),
        "afs ask on an ignored path should route to the owning agent, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("no relevant agents replied"),
        "afs ask on an ignored path must not fall through to broadcast, got:\n{stdout}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn broadcast_discovery_filters_file_references_matching_ignore_policy() {
    let afs_home = unique_afs_home("ignore-broadcast-filter");
    let managed_dir = unique_afs_home("managed-ignore-broadcast-filter");
    let pi_runtime = fake_pi_runtime("ignore-broadcast-filter-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(managed_dir.join("secrets"))
        .expect("test should create secrets subdirectory");
    std::fs::write(managed_dir.join(".gitignore"), "secrets/\n")
        .expect("test should create .gitignore");
    let readme_path = managed_dir.join("README.md");
    let vault_path = managed_dir.join("secrets/vault.txt");
    std::fs::write(&readme_path, "project readme\n").expect("test should create README");
    std::fs::write(&vault_path, "sensitive\n").expect("test should create secrets file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let readme_path = readme_path
        .canonicalize()
        .expect("readme should canonicalize");
    let vault_path = vault_path
        .canonicalize()
        .expect("vault should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    std::fs::write(
        managed_dir.join(".afs/broadcast-response"),
        format!(
            "possible\tcontains useful references\tHere are the project files\t{};{}\n",
            readme_path.display(),
            vault_path.display()
        ),
    )
    .expect("test should configure broadcast response");

    let ask = afs_ask(&afs_home, "what does the project contain");
    assert!(
        ask.status.success(),
        "afs ask should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&ask.stdout),
        String::from_utf8_lossy(&ask.stderr)
    );
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        stdout.contains(&format!("- {}", readme_path.display())),
        "broadcast references should include non-ignored files, got:\n{stdout}"
    );
    assert!(
        !stdout.contains(&vault_path.display().to_string()),
        "broadcast references should filter out files matched by the ignore policy, got:\n{stdout}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn install_preserves_existing_afs_ignore_file() {
    let afs_home = unique_afs_home("ignore-idempotent");
    let managed_dir = unique_afs_home("managed-ignore-idempotent");
    let pi_runtime = fake_pi_runtime("ignore-idempotent-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(managed_dir.join(".afs"))
        .expect("test should create Agent Home directory");
    std::fs::write(managed_dir.join(".gitignore"), "seeded.log\n")
        .expect("test should create .gitignore");
    let preserved = "# custom AFS ignore policy\ncustom-pattern\n";
    std::fs::write(managed_dir.join(".afs/ignore"), preserved)
        .expect("test should create a custom ignore policy");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);

    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    let after = std::fs::read_to_string(managed_dir.join(".afs/ignore"))
        .expect("afs install should leave the existing ignore policy in place");
    assert_eq!(
        after, preserved,
        "afs install must not overwrite a pre-existing ignore policy"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn supervisor_rediscovers_moved_managed_directory_after_restart() {
    let afs_home = unique_afs_home("move-rediscover");
    let workspace = unique_afs_home("move-rediscover-workspace");
    let original_dir = workspace.join("project");
    let pi_runtime = fake_pi_runtime("move-rediscover-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&original_dir).expect("test should create managed directory");
    std::fs::write(original_dir.join("notes.txt"), "before move\n")
        .expect("test should create managed file");
    let original_dir = original_dir
        .canonicalize()
        .expect("original directory should canonicalize");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );

    let install = install_managed_dir(&afs_home, &original_dir);
    assert!(install.status.success(), "afs install should succeed");
    let identity = std::fs::read_to_string(original_dir.join(".afs/identity"))
        .expect("identity should be readable")
        .trim()
        .to_string();

    stop_daemon(&mut daemon);

    let workspace = workspace
        .canonicalize()
        .expect("workspace should canonicalize");
    let moved_dir = workspace.join("project-renamed");
    std::fs::rename(&original_dir, &moved_dir).expect("test should move managed directory");
    let moved_dir = moved_dir
        .canonicalize()
        .expect("moved directory should canonicalize");

    let mut restarted_daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "restarted daemon should re-create the Supervisor Socket"
    );

    assert!(
        wait_until(Duration::from_secs(3), || {
            let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
                .env("AFS_HOME", &afs_home)
                .arg("agents")
                .output()
                .expect("afs agents should run");
            let stdout = String::from_utf8_lossy(&agents.stdout);
            agents.status.success()
                && stdout.contains(&moved_dir.display().to_string())
                && stdout.contains(&identity)
        }),
        "afs agents should report the moved managed directory under its preserved identity"
    );

    let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("agents")
        .output()
        .expect("afs agents should run");
    let stdout = String::from_utf8_lossy(&agents.stdout);
    assert!(
        !stdout.contains(&format!("{}\t", original_dir.display())),
        "afs agents should not report the original path as managed after rediscovery"
    );

    let history = afs_history(&afs_home, &moved_dir);
    assert!(
        history.status.success(),
        "afs history on the moved path should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&history.stdout),
        String::from_utf8_lossy(&history.stderr)
    );

    let stale = afs_history(&afs_home, &original_dir);
    assert!(
        !stale.status.success(),
        "afs history on the original path should fail after the directory moved"
    );

    stop_daemon(&mut restarted_daemon);
}

#[test]
fn rediscovery_skips_candidate_with_unreadable_identity_file() {
    let afs_home = unique_afs_home("move-rediscover-bad-identity");
    let workspace = unique_afs_home("move-rediscover-bad-identity-workspace");
    let original_dir = workspace.join("project");
    let pi_runtime = fake_pi_runtime("move-rediscover-bad-identity-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&original_dir).expect("test should create managed directory");
    std::fs::write(original_dir.join("notes.txt"), "before move\n")
        .expect("test should create managed file");
    let original_dir = original_dir
        .canonicalize()
        .expect("original directory should canonicalize");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );
    let install = install_managed_dir(&afs_home, &original_dir);
    assert!(install.status.success(), "afs install should succeed");
    let identity = std::fs::read_to_string(original_dir.join(".afs/identity"))
        .expect("identity should be readable")
        .trim()
        .to_string();
    stop_daemon(&mut daemon);

    let workspace = workspace
        .canonicalize()
        .expect("workspace should canonicalize");
    let moved_dir = workspace.join("project-renamed");
    std::fs::rename(&original_dir, &moved_dir).expect("test should move managed directory");
    let moved_dir = moved_dir
        .canonicalize()
        .expect("moved directory should canonicalize");

    // Place an unrelated `.afs/identity` at the search root with invalid UTF-8
    // so reading it would fail. The supervisor should treat this candidate as a
    // non-match and continue scanning rather than abort startup.
    let unrelated_agent_home = workspace.join(".afs");
    std::fs::create_dir_all(&unrelated_agent_home)
        .expect("test should create unrelated agent home");
    std::fs::write(unrelated_agent_home.join("identity"), b"\xff\xfe\xfd")
        .expect("test should write invalid-utf8 identity bytes");

    let mut restarted_daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "restarted daemon should re-create the Supervisor Socket"
    );

    assert!(
        wait_until(Duration::from_secs(3), || {
            let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
                .env("AFS_HOME", &afs_home)
                .arg("agents")
                .output()
                .expect("afs agents should run");
            let stdout = String::from_utf8_lossy(&agents.stdout);
            agents.status.success()
                && stdout.contains(&moved_dir.display().to_string())
                && stdout.contains(&identity)
        }),
        "afs agents should report the moved managed directory even when an unrelated identity file in the scan tree is unreadable"
    );

    stop_daemon(&mut restarted_daemon);
}

#[test]
fn rediscovery_rewrite_preserves_unresolved_registry_rows() {
    let afs_home = unique_afs_home("move-rediscover-preserve");
    let movable_workspace = unique_afs_home("move-rediscover-preserve-movable");
    let movable_dir = movable_workspace.join("project");
    let unmounted_workspace = unique_afs_home("move-rediscover-preserve-unmounted");
    let unmounted_dir = unmounted_workspace.join("project");
    let pi_runtime = fake_pi_runtime("move-rediscover-preserve-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&movable_dir).expect("test should create movable directory");
    std::fs::create_dir_all(&unmounted_dir).expect("test should create unmounted directory");
    let movable_dir = movable_dir
        .canonicalize()
        .expect("movable directory should canonicalize");
    let unmounted_dir = unmounted_dir
        .canonicalize()
        .expect("unmounted directory should canonicalize");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );
    assert!(
        install_managed_dir(&afs_home, &movable_dir)
            .status
            .success(),
        "movable afs install should succeed"
    );
    assert!(
        install_managed_dir(&afs_home, &unmounted_dir)
            .status
            .success(),
        "unmounted afs install should succeed"
    );
    let unmounted_identity = std::fs::read_to_string(unmounted_dir.join(".afs/identity"))
        .expect("unmounted identity should be readable")
        .trim()
        .to_string();
    let unmounted_registry_line = format!(
        "{}\t{}\t{}",
        unmounted_identity,
        unmounted_dir.display(),
        unmounted_dir.join(".afs").display()
    );
    stop_daemon(&mut daemon);

    let movable_workspace = movable_workspace
        .canonicalize()
        .expect("movable workspace should canonicalize");
    let moved_dir = movable_workspace.join("project-renamed");
    std::fs::rename(&movable_dir, &moved_dir).expect("test should move movable directory");

    // Simulate the unmounted-drive scenario: the entire ancestor disappears,
    // so the supervisor cannot rediscover this entry on this restart.
    std::fs::remove_dir_all(&unmounted_workspace)
        .expect("test should remove unmounted workspace entirely");

    let mut restarted_daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "restarted daemon should re-create the Supervisor Socket"
    );

    let moved_dir = moved_dir
        .canonicalize()
        .expect("moved directory should canonicalize");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
                .env("AFS_HOME", &afs_home)
                .arg("agents")
                .output()
                .expect("afs agents should run");
            agents.status.success()
                && String::from_utf8_lossy(&agents.stdout)
                    .contains(&moved_dir.display().to_string())
        }),
        "afs agents should report the rediscovered movable directory"
    );

    let registry = std::fs::read_to_string(afs_home.join("registry.tsv"))
        .expect("registry should be readable after restart");
    assert!(
        registry.lines().any(|line| line == unmounted_registry_line),
        "registry should preserve the unresolved row for the temporarily missing managed directory; got:\n{registry}"
    );

    stop_daemon(&mut restarted_daemon);
}

#[test]
fn rediscovery_finds_move_to_sibling_ancestor_directory() {
    let afs_home = unique_afs_home("move-rediscover-sibling");
    let workspace = unique_afs_home("move-rediscover-sibling-workspace");
    let work_dir = workspace.join("work");
    let archive_dir = workspace.join("archive");
    let original_dir = work_dir.join("project");
    let pi_runtime = fake_pi_runtime("move-rediscover-sibling-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&original_dir).expect("test should create managed directory");
    std::fs::create_dir_all(&archive_dir).expect("test should create archive directory");
    std::fs::write(original_dir.join("notes.txt"), "before move\n")
        .expect("test should create managed file");
    let original_dir = original_dir
        .canonicalize()
        .expect("original directory should canonicalize");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before install connects"
    );
    let install = install_managed_dir(&afs_home, &original_dir);
    assert!(install.status.success(), "afs install should succeed");
    let identity = std::fs::read_to_string(original_dir.join(".afs/identity"))
        .expect("identity should be readable")
        .trim()
        .to_string();
    stop_daemon(&mut daemon);

    // Move the managed directory out of its registered parent into a sibling
    // ancestor: /<workspace>/work/project -> /<workspace>/archive/project. The
    // registered parent (/<workspace>/work) still exists, so rediscovery must
    // keep ascending past it to find the new location.
    let archive_dir = archive_dir
        .canonicalize()
        .expect("archive directory should canonicalize");
    let moved_dir = archive_dir.join("project");
    std::fs::rename(&original_dir, &moved_dir)
        .expect("test should move managed directory to sibling-ancestor location");
    let moved_dir = moved_dir
        .canonicalize()
        .expect("moved directory should canonicalize");

    let mut restarted_daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "restarted daemon should re-create the Supervisor Socket"
    );

    assert!(
        wait_until(Duration::from_secs(3), || {
            let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
                .env("AFS_HOME", &afs_home)
                .arg("agents")
                .output()
                .expect("afs agents should run");
            let stdout = String::from_utf8_lossy(&agents.stdout);
            agents.status.success()
                && stdout.contains(&moved_dir.display().to_string())
                && stdout.contains(&identity)
        }),
        "afs agents should report the directory rediscovered under a sibling ancestor"
    );

    stop_daemon(&mut restarted_daemon);
}

#[test]
fn broadcast_progress_streams_before_final_body() {
    let afs_home = unique_afs_home("ask-stream-broadcast");
    let fast_dir = unique_afs_home("ask-stream-broadcast-fast");
    let slow_dir = unique_afs_home("ask-stream-broadcast-slow");
    let pi_runtime = fake_pi_runtime("ask-stream-broadcast-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&fast_dir).expect("test should create fast managed directory");
    std::fs::create_dir_all(&slow_dir).expect("test should create slow managed directory");
    let fast_file = fast_dir.join("notes.md");
    std::fs::write(&fast_file, "fast context\n").expect("test should create fast reference");
    let fast_dir = fast_dir
        .canonicalize()
        .expect("fast directory should canonicalize");
    let slow_dir = slow_dir
        .canonicalize()
        .expect("slow directory should canonicalize");
    let fast_file = fast_file
        .canonicalize()
        .expect("fast reference should canonicalize");

    let mut daemon =
        start_daemon_with_pi_runtime_and_broadcast_timeout(&afs_home, &pi_runtime, 3000);
    await_socket(&socket_path);

    let fast_install = install_managed_dir(&afs_home, &fast_dir);
    assert!(fast_install.status.success(), "fast install should succeed");
    let slow_install = install_managed_dir(&afs_home, &slow_dir);
    assert!(slow_install.status.success(), "slow install should succeed");

    let fast_identity =
        std::fs::read_to_string(fast_dir.join(".afs/identity")).expect("fast identity exists");
    std::fs::write(
        fast_dir.join(".afs/broadcast-response"),
        format!("possible\tfast reply\tfast wins\t{}\n", fast_file.display()),
    )
    .expect("test should configure fast broadcast response");
    std::fs::write(slow_dir.join(".afs/broadcast-delay-seconds"), "2")
        .expect("test should configure slow broadcast delay");
    std::fs::write(
        slow_dir.join(".afs/broadcast-response"),
        "strong\tslow reply\tslow wins\t\n",
    )
    .expect("test should configure slow broadcast response");

    let lines = afs_ask_streamed(&afs_home, "what context is available");

    let waiting = lines
        .iter()
        .find(|(_, line)| line.starts_with("progress: broadcast waiting"))
        .expect("streamed output should include broadcast waiting progress");
    let fast_reply = lines
        .iter()
        .find(|(_, line)| {
            line.starts_with(&format!(
                "progress: broadcast reply agent={}",
                fast_identity.trim()
            ))
        })
        .expect("streamed output should include the fast agent's broadcast reply progress");
    let body = lines
        .iter()
        .find(|(_, line)| line == "answers:")
        .expect("streamed output should include the answers body line");

    assert!(
        waiting.0 + Duration::from_millis(1500) <= body.0,
        "broadcast waiting progress must be flushed before the slow agent replies; \
         waiting at {:?}, body at {:?}",
        waiting.0,
        body.0,
    );
    assert!(
        fast_reply.0 < body.0,
        "fast broadcast reply progress must be flushed before the body; \
         fast_reply at {:?}, body at {:?}",
        fast_reply.0,
        body.0,
    );

    stop_daemon(&mut daemon);
}

#[test]
fn delegated_supervisor_progress_streams_before_final_body() {
    let afs_home = unique_afs_home("ask-stream-delegate");
    let source_dir = unique_afs_home("ask-stream-delegate-source");
    let target_dir = unique_afs_home("ask-stream-delegate-target");
    let pi_runtime = fake_pi_runtime("ask-stream-delegate-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&source_dir).expect("test should create source dir");
    std::fs::create_dir_all(&target_dir).expect("test should create target dir");
    let source_file = source_dir.join("request.md");
    std::fs::write(&source_file, "needs delegated work\n").expect("test should create source file");
    let source_dir = source_dir
        .canonicalize()
        .expect("source dir should canonicalize");
    let target_dir = target_dir
        .canonicalize()
        .expect("target dir should canonicalize");
    let source_file = source_file
        .canonicalize()
        .expect("source file should canonicalize");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    assert!(
        install_managed_dir(&afs_home, &source_dir).status.success(),
        "source install should succeed"
    );
    assert!(
        install_managed_dir(&afs_home, &target_dir).status.success(),
        "target install should succeed"
    );

    let source_identity =
        std::fs::read_to_string(source_dir.join(".afs/identity")).expect("source identity exists");
    let target_identity =
        std::fs::read_to_string(target_dir.join(".afs/identity")).expect("target identity exists");

    std::fs::write(
        source_dir.join(".afs/delegate-target"),
        target_dir.display().to_string(),
    )
    .expect("test should configure delegated target");
    std::fs::write(source_dir.join(".afs/delegate-reply-target"), "supervisor")
        .expect("test should configure reply target");
    std::fs::write(source_dir.join(".afs/delegate-prompt"), "first task")
        .expect("test should configure first delegated prompt");
    std::fs::write(
        source_dir.join(".afs/delegate-second-prompt"),
        "second task",
    )
    .expect("test should configure second delegated prompt");
    std::fs::write(target_dir.join(".afs/task-delay-seconds"), "1")
        .expect("test should configure target task delay");

    let lines = afs_ask_streamed(&afs_home, &format!("coordinate {}", source_file.display()));

    let route_delegated = lines
        .iter()
        .find(|(_, line)| {
            line == &format!("progress: route=delegated from={}", source_identity.trim())
        })
        .expect("streamed output should include route=delegated progress");
    let queued = lines
        .iter()
        .find(|(_, line)| {
            line == &format!(
                "progress: queued task agent={} queue=1",
                target_identity.trim()
            )
        })
        .expect("streamed output should include queued task progress");
    let delegating = lines
        .iter()
        .find(|(_, line)| {
            line == &format!(
                "progress: delegating from={} to={} reply=supervisor",
                source_identity.trim(),
                target_identity.trim()
            )
        })
        .expect("streamed output should include delegating progress");
    let task_complete = lines
        .iter()
        .find(|(_, line)| {
            line == &format!(
                "progress: task complete agent={} changed_files=0",
                target_identity.trim()
            )
        })
        .expect("streamed output should include task complete progress");
    let started = lines
        .iter()
        .find(|(_, line)| {
            line == &format!(
                "progress: started task agent={} queue=0",
                target_identity.trim()
            )
        })
        .expect("streamed output should include started task progress");
    let body = lines
        .iter()
        .find(|(_, line)| line.starts_with("references:"))
        .expect("streamed output should include the references body line");

    assert!(
        route_delegated.0 < body.0
            && queued.0 < body.0
            && delegating.0 < body.0
            && started.0 < body.0
            && task_complete.0 < body.0,
        "all delegation progress lines must arrive before the final body in line order"
    );
    assert!(
        delegating.0 + Duration::from_millis(500) <= body.0,
        "delegating progress must be flushed at least 500ms before the body; \
         delegating at {:?}, body at {:?}",
        delegating.0,
        body.0,
    );
    assert!(
        queued.0 + Duration::from_millis(500) <= body.0,
        "queued task progress must be flushed at least 500ms before the body; \
         queued at {:?}, body at {:?}",
        queued.0,
        body.0,
    );

    stop_daemon(&mut daemon);
}

#[test]
fn direct_ask_streams_route_direct_progress() {
    let afs_home = unique_afs_home("ask-stream-direct");
    let managed_dir = unique_afs_home("ask-stream-direct-managed");
    let pi_runtime = fake_pi_runtime("ask-stream-direct-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed dir");
    let managed_file = managed_dir.join("notes.md");
    std::fs::write(&managed_file, "direct context\n").expect("test should create managed file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed dir should canonicalize");
    let managed_file = managed_file
        .canonicalize()
        .expect("managed file should canonicalize");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    assert!(
        install_managed_dir(&afs_home, &managed_dir)
            .status
            .success(),
        "install should succeed"
    );
    let identity =
        std::fs::read_to_string(managed_dir.join(".afs/identity")).expect("identity exists");
    std::fs::write(managed_dir.join(".afs/ask-delay-seconds"), "1")
        .expect("test should configure ask delay");

    let lines = afs_ask_streamed(&afs_home, &format!("summarize {}", managed_file.display()));

    let route_direct = lines
        .iter()
        .find(|(_, line)| line == &format!("progress: route=direct agent={}", identity.trim()))
        .expect("streamed output should include route=direct progress");
    let answer = lines
        .iter()
        .find(|(_, line)| line.starts_with(&format!("agent {} answered about", identity.trim())))
        .expect("streamed output should include the agent answer");

    assert!(
        route_direct.0 + Duration::from_millis(500) <= answer.0,
        "route=direct progress must be flushed at least 500ms before the answer; \
         route_direct at {:?}, answer at {:?}",
        route_direct.0,
        answer.0,
    );
    let any_broadcast = lines.iter().any(|(_, line)| line.contains("broadcast"));
    assert!(
        !any_broadcast,
        "direct ask should not emit broadcast progress; got lines:\n{:#?}",
        lines
    );
    let any_delegated = lines.iter().any(|(_, line)| line.contains("delegating"));
    assert!(
        !any_delegated,
        "direct ask without delegation should not emit delegation progress; got lines:\n{:#?}",
        lines
    );

    stop_daemon(&mut daemon);
}

#[test]
fn concurrent_direct_asks_to_same_agent_queue_fifo_and_report_status() {
    let afs_home = unique_afs_home("ask-concurrent-direct");
    let managed_dir = unique_afs_home("ask-concurrent-direct-managed");
    let pi_runtime = fake_pi_runtime("ask-concurrent-direct-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed dir");
    let managed_file = managed_dir.join("notes.md");
    std::fs::write(&managed_file, "direct context\n").expect("test should create managed file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed dir should canonicalize");
    let managed_file = managed_file
        .canonicalize()
        .expect("managed file should canonicalize");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    assert!(
        install_managed_dir(&afs_home, &managed_dir)
            .status
            .success(),
        "install should succeed"
    );
    let identity =
        std::fs::read_to_string(managed_dir.join(".afs/identity")).expect("identity exists");
    let delay_path = managed_dir.join(".afs/ask-delay-seconds");
    std::fs::write(&delay_path, "2").expect("test should configure initial ask delay");

    let first_prompt = format!("first direct {}", managed_file.display());
    let second_prompt = format!("second direct {}", managed_file.display());
    let first = spawn_afs_ask_streamed(&afs_home, &first_prompt);
    assert!(
        wait_until(Duration::from_secs(1), || {
            std::fs::read_to_string(managed_dir.join(".afs/ask-received"))
                .map(|log| log.contains(&format!("prompt={first_prompt}")))
                .unwrap_or(false)
        }),
        "first ask should reach the agent runtime before the second ask is spawned"
    );
    std::fs::remove_file(&delay_path).expect("second ask should not inherit the startup delay");

    let second = spawn_afs_ask_streamed(&afs_home, &second_prompt);
    let mut last_agents = String::new();
    let saw_queue = wait_until(Duration::from_secs(3), || {
        let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
            .env("AFS_HOME", &afs_home)
            .arg("agents")
            .output()
            .expect("afs agents should run");
        if !agents.status.success() {
            return false;
        }
        last_agents = String::from_utf8_lossy(&agents.stdout).to_string();
        last_agents.contains("active=true") && last_agents.contains("queue=1")
    });

    let first_lines = first.finish();
    let second_lines = second.finish();
    let task_log = std::fs::read_to_string(managed_dir.join(".afs/ask-received"))
        .expect("agent runtime should record both asks");
    let final_agents = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("agents")
        .output()
        .expect("afs agents should run");
    assert!(
        final_agents.status.success(),
        "final afs agents should succeed"
    );
    let final_agents_stdout = String::from_utf8_lossy(&final_agents.stdout);
    stop_daemon(&mut daemon);

    assert!(
        saw_queue,
        "afs agents should expose active work and the queued second ask while the first ask is active; last output:\n{last_agents}"
    );
    assert!(
        final_agents_stdout.contains("active=false") && final_agents_stdout.contains("queue=0"),
        "afs agents should show no active or queued work after both asks finish; got:\n{final_agents_stdout}"
    );
    let queued_index = second_lines
        .iter()
        .position(|(_, line)| {
            line == &format!("progress: queued task agent={} queue=1", identity.trim())
        })
        .expect("second ask should stream queued progress");
    let started_index = second_lines
        .iter()
        .position(|(_, line)| {
            line == &format!("progress: started task agent={} queue=0", identity.trim())
        })
        .expect("second ask should stream started progress when the first ask releases the agent");
    let answer_index = second_lines
        .iter()
        .position(|(_, line)| {
            line.starts_with(&format!("agent {} answered about", identity.trim()))
        })
        .expect("second ask should return the direct answer");
    assert!(
        queued_index < started_index && started_index < answer_index,
        "second ask progress should show queued, then started, then answer; lines:\n{second_lines:#?}"
    );
    let first_position = task_log
        .find(&format!("prompt={first_prompt}"))
        .expect("runtime should record first ask");
    let second_position = task_log
        .find(&format!("prompt={second_prompt}"))
        .expect("runtime should record second ask");
    assert!(
        first_position < second_position,
        "agent runtime should receive concurrent direct asks FIFO; log:\n{task_log}"
    );
    assert!(
        first_lines
            .iter()
            .any(|(_, line)| line.starts_with(&format!("agent {} answered about", identity.trim()))),
        "first ask should also complete normally; lines:\n{first_lines:#?}"
    );
}

fn remove_dir_all_retry(path: &std::path::Path) {
    // The directory monitor inside the daemon races with rmdir: while the test
    // is removing files, the monitor can still be writing AFS history entries
    // for the same paths. Retry on ENOTEMPTY/EBUSY for a short budget rather
    // than failing on the race window.
    const ENOTEMPTY: i32 = 39;
    const EBUSY: i32 = 16;
    let deadline = Instant::now() + Duration::from_millis(2_000);
    loop {
        match std::fs::remove_dir_all(path) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error)
                if matches!(error.raw_os_error(), Some(ENOTEMPTY) | Some(EBUSY))
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => panic!("test should wipe the managed directory off disk: {error:?}"),
        }
    }
}

fn await_socket(socket_path: &std::path::Path) {
    assert!(
        wait_until(Duration::from_secs(2), || socket_path
            .metadata()
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)),
        "daemon should create the Supervisor Socket before clients connect"
    );
}

fn await_index_token(afs_home: &std::path::Path, expected: &str, timeout: Duration) -> String {
    let mut last_stdout = String::new();
    let satisfied = wait_until(timeout, || {
        let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
            .env("AFS_HOME", afs_home)
            .arg("agents")
            .output()
            .expect("afs agents should run");
        if !agents.status.success() {
            return false;
        }
        last_stdout = String::from_utf8_lossy(&agents.stdout).to_string();
        last_stdout.contains(expected)
    });
    assert!(
        satisfied,
        "afs agents should report {expected}; last output:\n{last_stdout}"
    );
    last_stdout
}

#[test]
fn agents_reports_ready_index_for_text_files() {
    let afs_home = unique_afs_home("index-ready-text");
    let managed_dir = unique_afs_home("index-ready-text-managed");
    let pi_runtime = fake_pi_runtime("index-ready-text-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(managed_dir.join("a.txt"), "hello a\n")
        .expect("test should create first text file");
    std::fs::write(managed_dir.join("b.txt"), "hello b\n")
        .expect("test should create second text file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    await_index_token(&afs_home, "index=ready(files=2)", Duration::from_secs(3));

    stop_daemon(&mut daemon);
}

#[test]
fn agents_reports_warming_index_during_initial_scan() {
    let afs_home = unique_afs_home("index-warming");
    let managed_dir = unique_afs_home("index-warming-managed");
    let pi_runtime = fake_pi_runtime("index-warming-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    for n in 0..5 {
        std::fs::write(
            managed_dir.join(format!("file-{n}.txt")),
            format!("text-{n}\n"),
        )
        .expect("test should create warming fixture file");
    }
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_index_warm_delay(&afs_home, &pi_runtime, 100);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    let agents = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("agents")
        .output()
        .expect("afs agents should run");
    assert!(agents.status.success(), "afs agents should succeed");
    let stdout = String::from_utf8_lossy(&agents.stdout);
    assert!(
        stdout.contains("index=warming"),
        "afs agents should report warming index during the slowed initial scan; got:\n{stdout}"
    );

    await_index_token(&afs_home, "index=ready(files=5)", Duration::from_secs(5));

    stop_daemon(&mut daemon);
}

#[test]
fn index_updates_after_external_text_change() {
    let afs_home = unique_afs_home("index-external-update");
    let managed_dir = unique_afs_home("index-external-update-managed");
    let pi_runtime = fake_pi_runtime("index-external-update-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    await_index_token(&afs_home, "index=ready(files=0)", Duration::from_secs(3));

    std::fs::write(managed_dir.join("notes.txt"), "fresh notes\n")
        .expect("test should create external text file");

    await_index_token(&afs_home, "index=ready(files=1)", Duration::from_secs(5));

    stop_daemon(&mut daemon);
}

#[test]
fn index_excludes_files_per_ignore_policy() {
    let afs_home = unique_afs_home("index-ignore");
    let managed_dir = unique_afs_home("index-ignore-managed");
    let pi_runtime = fake_pi_runtime("index-ignore-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(managed_dir.join(".gitignore"), ".gitignore\nsecret.txt\n")
        .expect("test should seed .gitignore");
    std::fs::write(managed_dir.join("secret.txt"), "do not index\n")
        .expect("test should write ignored file");
    std::fs::write(managed_dir.join("notes.txt"), "indexed\n")
        .expect("test should write indexed file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    await_index_token(&afs_home, "index=ready(files=1)", Duration::from_secs(3));

    stop_daemon(&mut daemon);
}

#[test]
fn index_skips_binary_files() {
    let afs_home = unique_afs_home("index-binary-skip");
    let managed_dir = unique_afs_home("index-binary-skip-managed");
    let pi_runtime = fake_pi_runtime("index-binary-skip-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(managed_dir.join("notes.txt"), "human readable\n")
        .expect("test should write text file");
    std::fs::write(managed_dir.join("blob.bin"), [0u8, 1, 2, 3, 0, 4, 5])
        .expect("test should write binary file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    await_index_token(&afs_home, "index=ready(files=1)", Duration::from_secs(3));

    stop_daemon(&mut daemon);
}

#[test]
fn index_excludes_nested_managed_directory() {
    let afs_home = unique_afs_home("index-nested");
    let parent_dir = unique_afs_home("index-nested-parent");
    let pi_runtime = fake_pi_runtime("index-nested-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(parent_dir.join("child")).expect("test should create nested layout");
    std::fs::write(parent_dir.join("p1.txt"), "parent 1\n")
        .expect("test should write parent file 1");
    std::fs::write(parent_dir.join("p2.txt"), "parent 2\n")
        .expect("test should write parent file 2");
    std::fs::write(parent_dir.join("child").join("c1.txt"), "child 1\n")
        .expect("test should write child file");
    let parent_dir = parent_dir
        .canonicalize()
        .expect("parent directory should canonicalize");
    let child_dir = parent_dir.join("child");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    let install_parent = install_managed_dir(&afs_home, &parent_dir);
    assert!(
        install_parent.status.success(),
        "afs install (parent) should succeed"
    );
    await_index_token(&afs_home, "index=ready(files=3)", Duration::from_secs(3));

    let install_child = install_managed_dir(&afs_home, &child_dir);
    assert!(
        install_child.status.success(),
        "afs install (child) should succeed"
    );

    let stdout = await_index_token(&afs_home, "index=ready(files=2)", Duration::from_secs(5));
    assert!(
        stdout.contains("index=ready(files=1)"),
        "child managed directory should report a single indexed file; got:\n{stdout}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn direct_ask_omits_warming_caveat_when_index_ready() {
    let afs_home = unique_afs_home("ask-no-warming");
    let managed_dir = unique_afs_home("ask-no-warming-managed");
    let pi_runtime = fake_pi_runtime("ask-no-warming-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let target = managed_dir.join("notes.txt");
    std::fs::write(&target, "ready content\n").expect("test should write target file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let target = target.canonicalize().expect("target should canonicalize");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    await_index_token(&afs_home, "index=ready(files=1)", Duration::from_secs(3));

    let ask = afs_ask(&afs_home, &format!("summarize {}", target.display()));
    assert!(ask.status.success(), "afs ask should succeed");
    let stdout = String::from_utf8_lossy(&ask.stdout);
    assert!(
        !stdout.contains("caveat: local index is warming"),
        "afs ask should omit the warming caveat once the local index is ready; got:\n{stdout}"
    );

    stop_daemon(&mut daemon);
}

const BLOOD_PANEL_PDF: &[u8] = include_bytes!("fixtures/blood-panel.pdf");

#[test]
fn pdf_file_is_indexed_and_counted_among_ready_files() {
    let afs_home = unique_afs_home("index-pdf");
    let managed_dir = unique_afs_home("index-pdf-managed");
    let pi_runtime = fake_pi_runtime("index-pdf-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(managed_dir.join("notes.txt"), "human readable\n")
        .expect("test should write text file");
    std::fs::write(managed_dir.join("blood-panel.pdf"), BLOOD_PANEL_PDF)
        .expect("test should write pdf file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    let stdout = await_index_token(&afs_home, "index=ready(files=2)", Duration::from_secs(10));
    assert!(
        !stdout.contains("failed="),
        "ready index should not surface a failed= count for a well-formed pdf; got:\n{stdout}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn binary_file_is_tracked_in_history_and_restored_through_undo() {
    let afs_home = unique_afs_home("undo-binary");
    let managed_dir = unique_afs_home("undo-binary-managed");
    let pi_runtime = fake_pi_runtime("undo-binary-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let target_file = managed_dir.join("payload.bin");
    let original: [u8; 8] = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01, 0x02, 0xFF];
    std::fs::write(&target_file, original).expect("test should create binary managed file");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let target_file = target_file
        .canonicalize()
        .expect("target file should canonicalize");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    let mutated: [u8; 6] = [0xCA, 0xFE, 0xBA, 0xBE, 0x00, 0x42];
    std::fs::write(&target_file, mutated).expect("test should mutate binary file");
    assert!(
        wait_until(Duration::from_secs(3), || {
            let history = afs_history(&afs_home, &managed_dir);
            history.status.success()
                && String::from_utf8_lossy(&history.stdout).contains("type=external")
        }),
        "afs history should record the binary External Change"
    );

    let history = afs_history(&afs_home, &managed_dir);
    let stdout = String::from_utf8_lossy(&history.stdout);
    let entry = history_entry_id(stdout.lines().next().expect("history should have an entry"));
    let undo = Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", &afs_home)
        .arg("undo")
        .arg(&managed_dir)
        .arg(entry)
        .arg("--yes")
        .output()
        .expect("afs undo should run");

    assert!(undo.status.success(), "afs undo --yes should succeed");
    assert_eq!(
        std::fs::read(&target_file).expect("target file should be readable"),
        original.to_vec(),
        "undo should restore the exact binary bytes without UTF-8 assumptions"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn corrupted_pdf_degrades_gracefully_with_failure_caveat() {
    let afs_home = unique_afs_home("index-pdf-corrupt");
    let managed_dir = unique_afs_home("index-pdf-corrupt-managed");
    let pi_runtime = fake_pi_runtime("index-pdf-corrupt-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    let target = managed_dir.join("broken.pdf");
    let mut bytes = b"%PDF-1.4\n".to_vec();
    bytes.extend_from_slice(
        b"this is not a real pdf body, just garbage that pdf-extract cannot parse\n",
    );
    bytes.extend_from_slice(&[0xFF, 0xFE, 0x00, 0x01, 0x02]);
    std::fs::write(&target, &bytes).expect("test should write corrupted pdf");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let target = target.canonicalize().expect("target should canonicalize");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    let stdout = await_index_token(
        &afs_home,
        "index=incomplete(files=0, failed=1)",
        Duration::from_secs(10),
    );
    assert!(
        stdout.contains("failed=1"),
        "agents should surface failed=1 for the corrupted pdf; got:\n{stdout}"
    );

    let ask = afs_ask(&afs_home, &format!("summarize {}", target.display()));
    assert!(ask.status.success(), "afs ask should still succeed");
    let answer = String::from_utf8_lossy(&ask.stdout);
    assert!(
        answer.contains("caveat: local index could not extract 1 file(s)"),
        "afs ask should surface an honest extraction-failure caveat; got:\n{answer}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn text_file_with_pdf_magic_prefix_falls_back_to_text_index() {
    let afs_home = unique_afs_home("index-pdf-magic-text");
    let managed_dir = unique_afs_home("index-pdf-magic-text-managed");
    let pi_runtime = fake_pi_runtime("index-pdf-magic-text-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(&managed_dir).expect("test should create managed directory");
    std::fs::write(
        managed_dir.join("pdf-spec-notes.md"),
        "%PDF-1.4 is the PDF major version. This file is plain markdown text \
         describing the magic bytes; it is not itself a real PDF and pdf-extract \
         should not be able to parse it.\n",
    )
    .expect("test should write text file with pdf magic prefix");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    let stdout = await_index_token(&afs_home, "index=ready(files=1)", Duration::from_secs(10));
    assert!(
        !stdout.contains("failed="),
        "text file that incidentally starts with %PDF- should not be counted as a failed extraction; got:\n{stdout}"
    );

    stop_daemon(&mut daemon);
}

/// Real-Pi wire-format canary. Skips by default; runs only when the
/// caller sets `AFS_REAL_PI_SMOKE=1` AND a `pi` binary is on PATH
/// AND the caller has already configured Pi credentials (via
/// `pi login` or equivalent). Spawns AFS against the real
/// `pi --mode rpc` and exercises one `afs ask` end-to-end so a
/// future Pi release that drifts from the JSONL JSON-RPC schema
/// AFS targets surfaces here instead of in user-visible silent
/// failure (the bug that produced issue #40).
///
/// Run with:
///   AFS_REAL_PI_SMOKE=1 cargo test -- --ignored real_pi_smoke
#[test]
#[ignore]
fn real_pi_smoke_ask_returns_afs_reply() {
    if std::env::var("AFS_REAL_PI_SMOKE").ok().as_deref() != Some("1") {
        eprintln!(
            "skipping real-Pi smoke: set AFS_REAL_PI_SMOKE=1 and ensure `pi` is on PATH and authenticated"
        );
        return;
    }
    let pi_path = match Command::new("which").arg("pi").output() {
        Ok(output) if output.status.success() => {
            std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        _ => {
            eprintln!("skipping real-Pi smoke: `pi` not found on PATH");
            return;
        }
    };

    let afs_home = unique_afs_home("real-pi-smoke");
    let socket_path = supervisor_socket(&afs_home);
    write_config(&afs_home, r#"{"provider":"claude","auth_method":"oauth"}"#);
    let managed_dir = unique_afs_home("real-pi-smoke-managed");
    std::fs::create_dir_all(&managed_dir).expect("create managed dir");
    std::fs::write(managed_dir.join("hello.txt"), "smoke test\n").expect("seed managed file");
    let managed_dir = managed_dir.canonicalize().expect("canonicalize");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_path);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(
        install.status.success(),
        "afs install should succeed against real Pi\nstderr:\n{}",
        String::from_utf8_lossy(&install.stderr)
    );

    let ask = afs_ask(&afs_home, "what is in hello.txt?");
    let stdout = String::from_utf8_lossy(&ask.stdout);
    let stderr = String::from_utf8_lossy(&ask.stderr);

    // We accept any of: a structured reply (afs_reply was called),
    // a graceful "agent finished without afs_reply" diagnostic, or
    // a Pi-side parse error. What we DON'T accept is a hang or a
    // panic. The point of this test is to flush wire-format drift.
    assert!(
        ask.status.success()
            || stderr.contains("did not reply")
            || stderr.contains("agent finished"),
        "real-Pi ask either succeeded or surfaced a diagnostic; instead got\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    stop_daemon(&mut daemon);
}

#[test]
fn directory_agent_stderr_is_captured_to_runtime_log() {
    let afs_home = unique_afs_home("runtime-log-stderr");
    let managed_dir = unique_afs_home("runtime-log-stderr-managed");
    let pi_runtime = fake_pi_runtime("runtime-log-stderr-runtime");
    let socket_path = supervisor_socket(&afs_home);
    std::fs::create_dir_all(managed_dir.join(".afs"))
        .expect("test should create managed directory and agent home");
    let managed_dir = managed_dir
        .canonicalize()
        .expect("managed directory should canonicalize");
    let stderr_sigil = "fake-pi-stderr-banner-12345";
    std::fs::write(
        managed_dir.join(".afs/runtime-stderr"),
        format!("{stderr_sigil}\n"),
    )
    .expect("test should seed runtime-stderr before install");

    let mut daemon = start_daemon_with_pi_runtime(&afs_home, &pi_runtime);
    await_socket(&socket_path);

    let install = install_managed_dir(&afs_home, &managed_dir);
    assert!(install.status.success(), "afs install should succeed");

    let log_path = managed_dir.join(".afs/runtime.log");
    let captured = wait_until(Duration::from_secs(5), || {
        std::fs::read_to_string(&log_path)
            .map(|contents| contents.contains(stderr_sigil))
            .unwrap_or(false)
    });
    let log_contents = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        captured,
        "Pi runtime stderr should be appended to <agent-home>/runtime.log; got:\n{log_contents}"
    );

    stop_daemon(&mut daemon);
}
