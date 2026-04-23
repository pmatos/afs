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
  :
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
