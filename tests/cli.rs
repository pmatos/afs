use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use std::os::unix::fs::{FileTypeExt, PermissionsExt};
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

fn start_daemon_with_pi_runtime(afs_home: &std::path::Path, pi_runtime: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", afs_home)
        .env("AFS_PI_RUNTIME", pi_runtime)
        .arg("daemon")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("afs daemon should start")
}

fn fake_pi_runtime(test_name: &str) -> std::path::PathBuf {
    let runtime_dir = unique_afs_home(test_name);
    std::fs::create_dir_all(&runtime_dir).expect("test should create fake runtime directory");
    let runtime = runtime_dir.join("pi");
    std::fs::write(
        &runtime,
        r#"#!/bin/sh
{
  printf 'identity=%s\n' "$AFS_AGENT_ID"
  printf 'managed_dir=%s\n' "$AFS_MANAGED_DIR"
  printf 'rpc=%s\n' "$AFS_AGENT_RPC"
} > "$AFS_AGENT_HOME/runtime-started"
while IFS= read -r _line; do
  if [ "$_line" = "ASK" ]; then
    IFS= read -r asked_path
    IFS= read -r asked_prompt
    {
      printf 'path=%s\n' "$asked_path"
      printf 'prompt=%s\n' "$asked_prompt"
    } >> "$AFS_AGENT_HOME/ask-received"
    printf 'agent %s answered about %s\n' "$AFS_AGENT_ID" "$asked_path"
  fi
done
"#,
    )
    .expect("test should create fake Pi runtime");
    let mut permissions = std::fs::metadata(&runtime)
        .expect("fake Pi runtime should exist")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&runtime, permissions).expect("fake Pi runtime should be executable");
    runtime
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

fn afs_ask(afs_home: &std::path::Path, prompt: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_afs"))
        .env("AFS_HOME", afs_home)
        .arg("ask")
        .arg(prompt)
        .output()
        .expect("afs ask should run")
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
fn ask_connects_to_live_daemon_and_prints_stub_response() {
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
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "ask handling not implemented yet\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");

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
fn ask_answer_for_managed_path_includes_file_reference_and_index_caveat() {
    let afs_home = unique_afs_home("ask-reference-caveat");
    let managed_dir = unique_afs_home("ask-reference-caveat-managed");
    let pi_runtime = fake_pi_runtime("ask-reference-caveat-runtime");
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
    assert!(
        stdout.contains("caveat: local index is warming; answer may be incomplete"),
        "afs ask should disclose incomplete local index coverage"
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
    assert!(managed_dir.join(".afs/history/baseline.tsv").is_file());
    assert!(
        wait_until(Duration::from_secs(2), || managed_dir
            .join(".afs/runtime-started")
            .is_file()),
        "afs install should start the configured Pi Agent Runtime"
    );
    assert!(
        std::fs::read_to_string(managed_dir.join(".afs/runtime-started"))
            .expect("runtime marker should be readable")
            .contains("rpc=stdio"),
        "Pi Agent Runtime should be started in stdio RPC mode"
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
    let baseline_after_first =
        std::fs::read_to_string(managed_dir.join(".afs/history/baseline.tsv"))
            .expect("baseline should exist");

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
        std::fs::read_to_string(managed_dir.join(".afs/history/baseline.tsv"))
            .expect("baseline should still exist"),
        baseline_after_first
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
        stdout.contains("index=warming"),
        "afs agents should show Index Status"
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
                && String::from_utf8_lossy(&agents.stdout).contains("reconciliation=idle")
        }),
        "restarted agent should finish Startup Reconciliation before reporting idle"
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
