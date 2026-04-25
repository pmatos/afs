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

fn start_daemon_with_pi_runtime_and_broadcast_timeout(
    afs_home: &std::path::Path,
    pi_runtime: &std::path::Path,
    timeout_ms: u64,
) -> Child {
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
    if [ -f "$AFS_AGENT_HOME/delegate-target" ]; then
      delegate_target="$(cat "$AFS_AGENT_HOME/delegate-target")"
      delegate_reply_target="supervisor"
      if [ -f "$AFS_AGENT_HOME/delegate-reply-target" ]; then
        delegate_reply_target="$(cat "$AFS_AGENT_HOME/delegate-reply-target")"
      fi
      delegate_prompt="$asked_prompt"
      if [ -f "$AFS_AGENT_HOME/delegate-prompt" ]; then
        delegate_prompt="$(cat "$AFS_AGENT_HOME/delegate-prompt")"
      fi
      printf 'DELEGATE\t%s\t%s\t%s\n' "$delegate_target" "$delegate_reply_target" "$delegate_prompt"
      if [ -f "$AFS_AGENT_HOME/delegate-second-prompt" ]; then
        delegate_second_prompt="$(cat "$AFS_AGENT_HOME/delegate-second-prompt")"
        printf 'DELEGATE\t%s\t%s\t%s\n' "$delegate_target" "$delegate_reply_target" "$delegate_second_prompt"
      fi
      if [ "$delegate_reply_target" = "delegator" ]; then
        IFS= read -r delegated_marker
        IFS= read -r delegated_agent
        IFS= read -r delegated_answer
        IFS= read -r delegated_changed_files
        IFS= read -r delegated_history_entries
        {
          printf 'agent=%s\n' "$delegated_agent"
          printf 'answer=%s\n' "$delegated_answer"
          printf 'changed_files=%s\n' "$delegated_changed_files"
          printf 'history_entries=%s\n' "$delegated_history_entries"
        } >> "$AFS_AGENT_HOME/delegated-reply-received"
        printf 'delegator %s used %s\n' "$AFS_AGENT_ID" "$delegated_answer"
      fi
    else
      printf 'agent %s answered about %s\n' "$AFS_AGENT_ID" "$asked_path"
    fi
  elif [ "$_line" = "BROADCAST" ]; then
    IFS= read -r asked_prompt
    printf 'prompt=%s\n' "$asked_prompt" >> "$AFS_AGENT_HOME/broadcast-received"
    if [ -f "$AFS_AGENT_HOME/broadcast-delay-seconds" ]; then
      sleep "$(cat "$AFS_AGENT_HOME/broadcast-delay-seconds")"
    fi
    if [ -f "$AFS_AGENT_HOME/broadcast-response" ]; then
      cat "$AFS_AGENT_HOME/broadcast-response"
    fi
  elif [ "$_line" = "TASK" ]; then
    IFS= read -r requester
    IFS= read -r reply_target
    IFS= read -r task_prompt
    {
      printf 'requester=%s\n' "$requester"
      printf 'reply_target=%s\n' "$reply_target"
      printf 'prompt=%s\n' "$task_prompt"
    } >> "$AFS_AGENT_HOME/task-received"
    if [ -f "$AFS_AGENT_HOME/task-delay-seconds" ]; then
      sleep "$(cat "$AFS_AGENT_HOME/task-delay-seconds")"
    fi
    if [ -f "$AFS_AGENT_HOME/task-write-file" ]; then
      task_write_file="$(cat "$AFS_AGENT_HOME/task-write-file")"
      task_write_content="delegated task content"
      if [ -f "$AFS_AGENT_HOME/task-write-content" ]; then
        task_write_content="$(cat "$AFS_AGENT_HOME/task-write-content")"
      fi
      printf '%s\n' "$task_write_content" > "$AFS_MANAGED_DIR/$task_write_file"
    fi
    if [ -f "$AFS_AGENT_HOME/task-response" ]; then
      task_answer="$(cat "$AFS_AGENT_HOME/task-response")"
    else
      task_answer="delegated answer for $task_prompt from $AFS_AGENT_ID"
    fi
    printf 'TASK_REPLY\t%s\tnone\tnone\n' "$task_answer"
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

    std::fs::remove_dir_all(&managed_dir).expect("test should wipe the managed directory off disk");

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
