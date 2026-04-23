use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use std::os::unix::fs::FileTypeExt;
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
