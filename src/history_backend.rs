use crate::managed_subtree::AGENT_HOME_DIR;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const HISTORY_DIR: &str = "history";
const HISTORY_REPO_DIR: &str = "repo";

pub(crate) struct HistoryBackend {
    agent_home: PathBuf,
    lock: Mutex<()>,
}

#[derive(Debug)]
pub(crate) struct RecordedChange {
    pub(crate) history_entry: String,
    pub(crate) files: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct UndoOutcome {
    pub(crate) entry_id: String,
    pub(crate) files: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct HistoryEntry {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) summary: String,
    pub(crate) files: Vec<String>,
    pub(crate) undoable: bool,
    pub(crate) timestamp_secs: u64,
    pub(crate) origin: String,
}

impl HistoryBackend {
    pub(crate) fn open(agent_home: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(agent_home.join(HISTORY_DIR))?;
        Ok(Self {
            agent_home: agent_home.to_path_buf(),
            lock: Mutex::new(()),
        })
    }

    pub(crate) fn record_baseline(
        &self,
        managed_dir: &Path,
        nested_exclusions: &[String],
    ) -> io::Result<()> {
        let git_dir = self.git_dir();
        git_init_if_missing(&git_dir)?;
        if git_has_commits(&git_dir)? {
            return Ok(());
        }
        git_stage_and_commit(
            &git_dir,
            managed_dir,
            nested_exclusions,
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

    pub(crate) fn record_agent_change(
        &self,
        managed_dir: &Path,
        nested_exclusions: &[String],
    ) -> io::Result<Option<RecordedChange>> {
        self.record_snapshot_delta(managed_dir, nested_exclusions, "agent", "Agent change")
    }

    pub(crate) fn record_external_change(
        &self,
        managed_dir: &Path,
        nested_exclusions: &[String],
    ) -> io::Result<Option<RecordedChange>> {
        self.record_snapshot_delta(
            managed_dir,
            nested_exclusions,
            "external",
            "External change",
        )
    }

    pub(crate) fn merge_archived_child_history(
        &self,
        archived_child_home: &Path,
        parent_managed_dir: &Path,
        child_origin: &str,
    ) -> io::Result<()> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| io::Error::other("history lock poisoned"))?;
        let child_git_dir = archived_child_home.join(HISTORY_DIR).join(HISTORY_REPO_DIR);
        let mut commits = git_log_records(&child_git_dir)?;
        commits.reverse();

        let parent_git_dir = self.git_dir();
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

    pub(crate) fn record_ownership_event(
        &self,
        managed_dir: &Path,
        nested_exclusions: &[String],
        summary: &str,
    ) -> io::Result<()> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| io::Error::other("history lock poisoned"))?;
        let git_dir = self.git_dir();
        git_stage_work_tree(&git_dir, managed_dir, nested_exclusions)?;
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

    pub(crate) fn undo_latest(
        &self,
        managed_dir: &Path,
        nested_exclusions: &[String],
        requested_entry: &str,
        confirmed: bool,
    ) -> io::Result<UndoOutcome> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| io::Error::other("history lock poisoned"))?;
        let git_dir = self.git_dir();

        struct Candidate {
            entry_id: String,
            kind: String,
            summary: String,
            files: Vec<String>,
            representative_commit: String,
            latest_undoable: bool,
        }

        let mut commits = git_log_records(&git_dir)?;
        commits.reverse();

        let mut by_id: BTreeMap<String, Candidate> = BTreeMap::new();
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
                    Candidate {
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

        let undo_entry_id = history_entry_id();
        let undo_summary = sanitize_field(&format!("Undo {}: {}", latest.entry_id, latest.summary));
        git_stage_and_commit(
            &git_dir,
            managed_dir,
            nested_exclusions,
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
            nested_exclusions,
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

        Ok(UndoOutcome {
            entry_id: latest.entry_id,
            files: latest.files,
        })
    }

    pub(crate) fn record_reconciliation(
        &self,
        managed_dir: &Path,
        files: &[String],
    ) -> io::Result<Option<RecordedChange>> {
        if files.is_empty() {
            return Ok(None);
        }
        let _guard = self
            .lock
            .lock()
            .map_err(|_| io::Error::other("history lock poisoned"))?;
        let git_dir = self.git_dir();
        git_stage_paths(&git_dir, managed_dir, files)?;
        let changed_files = git_staged_changes_vs_head(&git_dir)?;
        if changed_files.is_empty() {
            return Ok(None);
        }
        let entry_id = history_entry_id();
        let summary = history_summary("Startup reconciliation", &changed_files);
        git_commit_index(
            &git_dir,
            managed_dir,
            &GitCommitRequest {
                entry_id: &entry_id,
                kind: "reconciliation",
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

    pub(crate) fn pending_external_files(
        &self,
        managed_dir: &Path,
        nested_exclusions: &[String],
        cutoff: SystemTime,
    ) -> io::Result<Vec<String>> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| io::Error::other("history lock poisoned"))?;
        let git_dir = self.git_dir();
        git_stage_work_tree(&git_dir, managed_dir, nested_exclusions)?;
        let changed_files = git_staged_changes_vs_head(&git_dir)?;
        let pending: Vec<String> = changed_files
            .into_iter()
            .filter(|file| changed_file_existed_before(managed_dir, file, cutoff))
            .collect();
        git_reset_index(&git_dir, managed_dir)?;
        Ok(pending)
    }

    pub(crate) fn entries(&self) -> io::Result<Vec<HistoryEntry>> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| io::Error::other("history lock poisoned"))?;
        let git_dir = self.git_dir();
        let mut records = git_log_records(&git_dir)?;
        records.reverse();

        let mut by_id: BTreeMap<String, HistoryEntry> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        for record in records {
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
            let origin = record
                .trailers
                .get("Afs-Origin")
                .cloned()
                .unwrap_or_default();

            if let Some(existing) = by_id.get_mut(&entry_id) {
                existing.undoable = undoable;
            } else {
                order.push(entry_id.clone());
                by_id.insert(
                    entry_id.clone(),
                    HistoryEntry {
                        id: entry_id,
                        kind,
                        summary,
                        files,
                        undoable,
                        timestamp_secs: record.timestamp_secs,
                        origin,
                    },
                );
            }
        }

        let entries: Vec<HistoryEntry> = order
            .into_iter()
            .rev()
            .map(|id| by_id.remove(&id).expect("entry must be present"))
            .collect();
        Ok(entries)
    }

    fn record_snapshot_delta(
        &self,
        managed_dir: &Path,
        nested_exclusions: &[String],
        kind: &str,
        summary_prefix: &str,
    ) -> io::Result<Option<RecordedChange>> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| io::Error::other("history lock poisoned"))?;
        let git_dir = self.git_dir();
        git_stage_work_tree(&git_dir, managed_dir, nested_exclusions)?;
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

    fn git_dir(&self) -> PathBuf {
        self.agent_home.join(HISTORY_DIR).join(HISTORY_REPO_DIR)
    }
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

fn history_summary(prefix: &str, files: &[String]) -> String {
    match files {
        [] => format!("{prefix}: no files changed"),
        [file] => format!("{prefix}: {file}"),
        _ => format!("{prefix}: {} files changed", files.len()),
    }
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

fn git_stage_paths(git_dir: &Path, work_tree: &Path, paths: &[String]) -> io::Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut add = git_base_command(git_dir, Some(work_tree));
    add.arg("add").arg("--all").arg("--force").arg("--");
    for path in paths {
        add.arg(path);
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

fn git_reset_index(git_dir: &Path, work_tree: &Path) -> io::Result<()> {
    let output = git_base_command(git_dir, Some(work_tree))
        .arg("reset")
        .arg("--quiet")
        .arg("HEAD")
        .arg("--")
        .arg(".")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "git reset failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn changed_file_existed_before(managed_dir: &Path, relative: &str, cutoff: SystemTime) -> bool {
    let path = managed_dir.join(relative);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return true,
        Err(_) => return false,
    };
    metadata
        .modified()
        .map(|mtime| mtime <= cutoff)
        .unwrap_or(false)
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

fn undoable_field(undoable: bool) -> &'static str {
    if undoable { "yes" } else { "no" }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn unique_tempdir(name: &str) -> PathBuf {
        let nanos = unix_timestamp_nanos();
        let path =
            std::env::temp_dir().join(format!("afs-history-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).expect("tempdir should be creatable");
        path
    }

    fn read_head_message(git_dir: &Path) -> String {
        let output = Command::new("git")
            .arg(format!("--git-dir={}", git_dir.display()))
            .arg("log")
            .arg("--format=%B")
            .arg("-1")
            .output()
            .expect("git log should run");
        assert!(
            output.status.success(),
            "git log failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn commit_count(git_dir: &Path) -> u32 {
        let output = Command::new("git")
            .arg(format!("--git-dir={}", git_dir.display()))
            .arg("rev-list")
            .arg("--count")
            .arg("HEAD")
            .output()
            .expect("git rev-list should run");
        assert!(
            output.status.success(),
            "git rev-list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .expect("count should parse")
    }

    #[test]
    fn record_baseline_creates_one_baseline_commit() {
        let workspace = unique_tempdir("baseline");
        let managed_dir = workspace.join("project");
        let agent_home = managed_dir.join(".afs");
        std::fs::create_dir_all(&managed_dir).unwrap();
        std::fs::create_dir_all(&agent_home).unwrap();

        let backend = HistoryBackend::open(&agent_home).expect("open should succeed");
        backend
            .record_baseline(&managed_dir, &[])
            .expect("baseline should succeed");

        let git_dir = backend.git_dir();
        let message = read_head_message(&git_dir);
        assert!(
            message.contains("Afs-Kind: baseline"),
            "missing Afs-Kind=baseline in: {message}"
        );
        assert!(
            message.contains("Afs-Undoable: no"),
            "missing Afs-Undoable=no in: {message}"
        );

        backend
            .record_baseline(&managed_dir, &[])
            .expect("baseline should be idempotent");
        assert_eq!(
            commit_count(&git_dir),
            1,
            "baseline must be idempotent (one commit)"
        );

        std::fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn record_agent_change_creates_undoable_agent_commit() {
        let workspace = unique_tempdir("agent-change");
        let managed_dir = workspace.join("project");
        let agent_home = managed_dir.join(".afs");
        std::fs::create_dir_all(&managed_dir).unwrap();
        std::fs::create_dir_all(&agent_home).unwrap();

        let backend = HistoryBackend::open(&agent_home).expect("open");
        backend
            .record_baseline(&managed_dir, &[])
            .expect("baseline");

        std::fs::write(managed_dir.join("notes.txt"), "v1").unwrap();
        let recorded = backend
            .record_agent_change(&managed_dir, &[])
            .expect("record_agent_change should succeed")
            .expect("expected Some(RecordedChange)");
        assert_eq!(recorded.files, vec!["notes.txt".to_string()]);
        assert!(!recorded.history_entry.is_empty());

        let message = read_head_message(&backend.git_dir());
        assert!(message.contains("Afs-Kind: agent"), "{message}");
        assert!(message.contains("Afs-Undoable: yes"), "{message}");
        assert!(message.contains("Afs-Files: notes.txt"), "{message}");

        let again = backend
            .record_agent_change(&managed_dir, &[])
            .expect("idempotent record");
        assert!(
            again.is_none(),
            "expected None when nothing changed, got {again:?}"
        );

        std::fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn entries_returns_newest_first_excluding_baseline() {
        let workspace = unique_tempdir("entries");
        let managed_dir = workspace.join("project");
        let agent_home = managed_dir.join(".afs");
        std::fs::create_dir_all(&managed_dir).unwrap();
        std::fs::create_dir_all(&agent_home).unwrap();

        let backend = HistoryBackend::open(&agent_home).unwrap();
        backend.record_baseline(&managed_dir, &[]).unwrap();

        std::fs::write(managed_dir.join("alpha.txt"), "a").unwrap();
        let a = backend
            .record_agent_change(&managed_dir, &[])
            .unwrap()
            .expect("Some(A)");
        std::fs::write(managed_dir.join("beta.txt"), "b").unwrap();
        let b = backend
            .record_external_change(&managed_dir, &[])
            .unwrap()
            .expect("Some(B)");

        let entries = backend.entries().expect("entries should succeed");
        assert_eq!(entries.len(), 2, "baseline should be excluded");

        assert_eq!(entries[0].id, b.history_entry);
        assert_eq!(entries[0].kind, "external");
        assert_eq!(entries[0].files, vec!["beta.txt".to_string()]);
        assert!(entries[0].undoable);
        assert!(entries[0].timestamp_secs > 0);

        assert_eq!(entries[1].id, a.history_entry);
        assert_eq!(entries[1].kind, "agent");
        assert_eq!(entries[1].files, vec!["alpha.txt".to_string()]);
        assert!(entries[1].undoable);
        assert!(entries[1].summary.contains("Agent change"));

        std::fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn merge_chains_origin_through_two_levels() {
        let workspace = unique_tempdir("merge-chain");

        // grandchild backend: one external change to notes.txt, no origin.
        let gc_managed = workspace.join("gc");
        let gc_home = gc_managed.join(".afs");
        std::fs::create_dir_all(&gc_managed).unwrap();
        std::fs::create_dir_all(&gc_home).unwrap();
        std::fs::write(gc_managed.join("seed.txt"), "s").unwrap();
        let gc = HistoryBackend::open(&gc_home).unwrap();
        gc.record_baseline(&gc_managed, &[]).unwrap();
        std::fs::write(gc_managed.join("notes.txt"), "edit").unwrap();
        let e_gc = gc
            .record_external_change(&gc_managed, &[])
            .unwrap()
            .expect("gc external");

        // child backend: gets grandchild's history merged in with origin "grandchild".
        let child_managed = workspace.join("child");
        let child_home = child_managed.join(".afs");
        std::fs::create_dir_all(&child_managed).unwrap();
        std::fs::create_dir_all(&child_home).unwrap();
        std::fs::write(child_managed.join("seed.txt"), "s").unwrap();
        let child = HistoryBackend::open(&child_home).unwrap();
        child.record_baseline(&child_managed, &[]).unwrap();
        child
            .merge_archived_child_history(&gc_home, &child_managed, "grandchild")
            .unwrap();

        // grandparent backend: merges child's history with origin "child".
        let gp_managed = workspace.join("gp");
        let gp_home = gp_managed.join(".afs");
        std::fs::create_dir_all(&gp_managed).unwrap();
        std::fs::create_dir_all(&gp_home).unwrap();
        std::fs::write(gp_managed.join("seed.txt"), "s").unwrap();
        let gp = HistoryBackend::open(&gp_home).unwrap();
        gp.record_baseline(&gp_managed, &[]).unwrap();
        gp.merge_archived_child_history(&child_home, &gp_managed, "child")
            .unwrap();

        let entries = gp.entries().unwrap();
        let target = entries
            .iter()
            .find(|e| e.id == e_gc.history_entry)
            .expect("grandparent should contain the original external entry id");
        assert_eq!(target.kind, "external");
        assert_eq!(
            target.origin, "child/grandchild",
            "transitive merge should chain origins; entries={entries:?}"
        );

        std::fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn merge_archived_child_history_replays_with_rewritten_paths_and_origin() {
        let workspace = unique_tempdir("merge");

        let child_managed = workspace.join("child");
        let child_home = child_managed.join(".afs");
        std::fs::create_dir_all(&child_managed).unwrap();
        std::fs::create_dir_all(&child_home).unwrap();
        std::fs::write(child_managed.join("seed.txt"), "s").unwrap();
        let child_backend = HistoryBackend::open(&child_home).unwrap();
        child_backend.record_baseline(&child_managed, &[]).unwrap();
        std::fs::create_dir_all(child_managed.join("src")).unwrap();
        std::fs::write(child_managed.join("src/lib.rs"), "// child code").unwrap();
        let c = child_backend
            .record_agent_change(&child_managed, &[])
            .unwrap()
            .expect("c");

        let parent_managed = workspace.join("parent");
        let parent_home = parent_managed.join(".afs");
        std::fs::create_dir_all(&parent_managed).unwrap();
        std::fs::create_dir_all(&parent_home).unwrap();
        std::fs::write(parent_managed.join("seed.txt"), "p").unwrap();
        let parent_backend = HistoryBackend::open(&parent_home).unwrap();
        parent_backend
            .record_baseline(&parent_managed, &[])
            .unwrap();

        parent_backend
            .merge_archived_child_history(&child_home, &parent_managed, "app")
            .expect("merge should succeed");

        let entries = parent_backend.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, c.history_entry);
        assert_eq!(entries[0].kind, "agent");
        assert_eq!(entries[0].files, vec!["app/src/lib.rs".to_string()]);
        assert_eq!(entries[0].origin, "app");
        assert!(
            !entries[0].undoable,
            "merged entries are non-undoable in parent"
        );

        std::fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn record_ownership_event_creates_non_undoable_entry() {
        let workspace = unique_tempdir("ownership");
        let managed_dir = workspace.join("project");
        let agent_home = managed_dir.join(".afs");
        std::fs::create_dir_all(&managed_dir).unwrap();
        std::fs::create_dir_all(&agent_home).unwrap();

        std::fs::write(managed_dir.join("seed.txt"), "seed").unwrap();
        let backend = HistoryBackend::open(&agent_home).unwrap();
        backend.record_baseline(&managed_dir, &[]).unwrap();

        backend
            .record_ownership_event(&managed_dir, &[], "Ownership split: child")
            .expect("ownership event should commit");

        let entries = backend.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, "ownership");
        assert_eq!(entries[0].summary, "Ownership split: child");
        assert!(!entries[0].undoable);

        let err = backend
            .undo_latest(&managed_dir, &[], "anything", false)
            .expect_err("ownership entry must not be undoable");
        assert!(err.to_string().contains("no undoable history entries"));

        std::fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn undo_latest_rejects_external_change_without_confirmation() {
        let workspace = unique_tempdir("undo-external");
        let managed_dir = workspace.join("project");
        let agent_home = managed_dir.join(".afs");
        std::fs::create_dir_all(&managed_dir).unwrap();
        std::fs::create_dir_all(&agent_home).unwrap();

        std::fs::write(managed_dir.join("seed.txt"), "seed").unwrap();
        let backend = HistoryBackend::open(&agent_home).unwrap();
        backend.record_baseline(&managed_dir, &[]).unwrap();

        std::fs::write(managed_dir.join("draft.md"), "x").unwrap();
        let e = backend
            .record_external_change(&managed_dir, &[])
            .unwrap()
            .expect("e");

        let err = backend
            .undo_latest(&managed_dir, &[], &e.history_entry, false)
            .expect_err("unconfirmed undo of External Change must error");
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(err.to_string().contains("requires --yes"), "{err}");

        backend
            .undo_latest(&managed_dir, &[], &e.history_entry, true)
            .expect("confirmed undo should succeed");
        assert!(!managed_dir.join("draft.md").exists());
        assert_eq!(
            std::fs::read_to_string(managed_dir.join("seed.txt")).unwrap(),
            "seed"
        );

        std::fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn undo_latest_rejects_when_entry_is_not_latest_applicable() {
        let workspace = unique_tempdir("undo-not-latest");
        let managed_dir = workspace.join("project");
        let agent_home = managed_dir.join(".afs");
        std::fs::create_dir_all(&managed_dir).unwrap();
        std::fs::create_dir_all(&agent_home).unwrap();

        let backend = HistoryBackend::open(&agent_home).unwrap();
        backend.record_baseline(&managed_dir, &[]).unwrap();

        let no_entries = backend
            .undo_latest(&managed_dir, &[], "history-bogus-0", false)
            .expect_err("undo with no entries must error");
        assert_eq!(no_entries.kind(), io::ErrorKind::InvalidInput);
        assert!(
            no_entries
                .to_string()
                .contains("no undoable history entries")
        );

        std::fs::write(managed_dir.join("a.txt"), "1").unwrap();
        let a = backend
            .record_agent_change(&managed_dir, &[])
            .unwrap()
            .expect("a");
        std::fs::write(managed_dir.join("b.txt"), "2").unwrap();
        let b = backend
            .record_agent_change(&managed_dir, &[])
            .unwrap()
            .expect("b");

        let err = backend
            .undo_latest(&managed_dir, &[], &a.history_entry, false)
            .expect_err("undoing A while B is latest must error");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains(&b.history_entry),
            "error should name latest undoable entry: {err}"
        );

        std::fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn undo_latest_reverts_agent_change_and_marks_entry_non_undoable() {
        let workspace = unique_tempdir("undo-agent");
        let managed_dir = workspace.join("project");
        let agent_home = managed_dir.join(".afs");
        std::fs::create_dir_all(&managed_dir).unwrap();
        std::fs::create_dir_all(&agent_home).unwrap();

        let backend = HistoryBackend::open(&agent_home).unwrap();
        backend.record_baseline(&managed_dir, &[]).unwrap();

        let notes = managed_dir.join("notes.txt");
        std::fs::write(&notes, "v1").unwrap();
        let a = backend
            .record_agent_change(&managed_dir, &[])
            .unwrap()
            .expect("a");
        std::fs::write(&notes, "v2").unwrap();
        let b = backend
            .record_agent_change(&managed_dir, &[])
            .unwrap()
            .expect("b");

        let outcome = backend
            .undo_latest(&managed_dir, &[], &b.history_entry, false)
            .expect("undo should succeed");
        assert_eq!(outcome.entry_id, b.history_entry);
        assert_eq!(outcome.files, vec!["notes.txt".to_string()]);

        assert_eq!(std::fs::read_to_string(&notes).unwrap(), "v1");

        let entries = backend.entries().unwrap();
        assert_eq!(entries.len(), 3, "expected undo + B (non-undoable) + A");
        assert_eq!(entries[0].kind, "undo");
        assert!(!entries[0].undoable);
        assert_eq!(entries[1].id, b.history_entry);
        assert!(!entries[1].undoable, "B should now be non-undoable");
        assert_eq!(entries[2].id, a.history_entry);
        assert!(entries[2].undoable);

        std::fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn record_reconciliation_commits_batch_external_change() {
        let workspace = unique_tempdir("recon");
        let managed_dir = workspace.join("project");
        let agent_home = managed_dir.join(".afs");
        std::fs::create_dir_all(&managed_dir).unwrap();
        std::fs::create_dir_all(&agent_home).unwrap();

        let backend = HistoryBackend::open(&agent_home).unwrap();
        backend.record_baseline(&managed_dir, &[]).unwrap();

        std::fs::write(managed_dir.join("foo.txt"), "f").unwrap();
        std::fs::write(managed_dir.join("bar.txt"), "b").unwrap();

        let recorded = backend
            .record_reconciliation(
                &managed_dir,
                &["foo.txt".to_string(), "bar.txt".to_string()],
            )
            .expect("record_reconciliation should succeed")
            .expect("Some");
        let mut files = recorded.files;
        files.sort();
        assert_eq!(files, vec!["bar.txt".to_string(), "foo.txt".to_string()]);

        let message = read_head_message(&backend.git_dir());
        assert!(message.contains("Afs-Kind: reconciliation"), "{message}");
        assert!(message.contains("Afs-Undoable: yes"), "{message}");

        let count_after_recon = commit_count(&backend.git_dir());
        let empty = backend
            .record_reconciliation(&managed_dir, &[])
            .expect("empty input should be Ok");
        assert!(empty.is_none());
        assert_eq!(
            commit_count(&backend.git_dir()),
            count_after_recon,
            "empty reconciliation must not commit"
        );

        std::fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn pending_external_files_honours_cutoff_and_resets_index() {
        use std::fs::{File, FileTimes};
        use std::time::Duration;

        let workspace = unique_tempdir("pending");
        let managed_dir = workspace.join("project");
        let agent_home = managed_dir.join(".afs");
        std::fs::create_dir_all(&managed_dir).unwrap();
        std::fs::create_dir_all(&agent_home).unwrap();

        let backend = HistoryBackend::open(&agent_home).unwrap();
        backend.record_baseline(&managed_dir, &[]).unwrap();

        let now = SystemTime::now();
        let earlier = now - Duration::from_secs(10);
        let cutoff = now - Duration::from_secs(5);

        let old_path = managed_dir.join("old.txt");
        std::fs::write(&old_path, "old").unwrap();
        File::options()
            .write(true)
            .open(&old_path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(earlier))
            .unwrap();

        let new_path = managed_dir.join("new.txt");
        std::fs::write(&new_path, "new").unwrap();
        File::options()
            .write(true)
            .open(&new_path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(now))
            .unwrap();

        let pending = backend
            .pending_external_files(&managed_dir, &[], cutoff)
            .expect("pending_external_files should succeed");
        assert_eq!(pending, vec!["old.txt".to_string()]);

        assert_eq!(
            backend.entries().unwrap().len(),
            0,
            "pending should not commit"
        );

        let recorded = backend
            .record_agent_change(&managed_dir, &[])
            .unwrap()
            .expect("Some");
        let mut files = recorded.files;
        files.sort();
        assert_eq!(files, vec!["new.txt".to_string(), "old.txt".to_string()]);

        std::fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn record_external_change_emits_external_kind_undoable() {
        let workspace = unique_tempdir("external");
        let managed_dir = workspace.join("project");
        let agent_home = managed_dir.join(".afs");
        std::fs::create_dir_all(&managed_dir).unwrap();
        std::fs::create_dir_all(&agent_home).unwrap();

        let backend = HistoryBackend::open(&agent_home).expect("open");
        backend
            .record_baseline(&managed_dir, &[])
            .expect("baseline");

        std::fs::write(managed_dir.join("draft.md"), "hello").unwrap();
        let recorded = backend
            .record_external_change(&managed_dir, &[])
            .expect("record_external_change should succeed")
            .expect("expected Some(RecordedChange)");
        assert_eq!(recorded.files, vec!["draft.md".to_string()]);

        let message = read_head_message(&backend.git_dir());
        assert!(message.contains("Afs-Kind: external"), "{message}");
        assert!(message.contains("Afs-Undoable: yes"), "{message}");
        assert!(message.contains("Afs-Files: draft.md"), "{message}");

        std::fs::remove_dir_all(&workspace).ok();
    }
}
