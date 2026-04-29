use std::io;
use std::path::{Path, PathBuf};

pub(crate) const AGENT_HOME_DIR: &str = ".afs";
pub(crate) const IGNORE_FILE: &str = "ignore";

pub(crate) struct ManagedSubtree {
    managed_dir: PathBuf,
    agent_home: PathBuf,
    ignore_matcher: Option<ignore::gitignore::Gitignore>,
}

impl ManagedSubtree {
    pub(crate) fn new(managed_dir: &Path, agent_home: &Path) -> Self {
        Self {
            managed_dir: managed_dir.to_path_buf(),
            agent_home: agent_home.to_path_buf(),
            ignore_matcher: load_ignore_matcher(agent_home, managed_dir),
        }
    }

    pub(crate) fn relative_path(&self, path: &Path) -> io::Result<String> {
        relative_path(&self.managed_dir, path)
    }

    pub(crate) fn nested_managed_relative_paths(&self) -> io::Result<Vec<String>> {
        let mut results = Vec::new();
        collect_nested_managed_relative_paths(&self.managed_dir, &self.managed_dir, &mut results)?;
        Ok(results)
    }

    pub(crate) fn is_content_path(&self, path: &Path) -> bool {
        self.is_base_content_path(path) && !is_nested_managed_path(&self.managed_dir, path)
    }

    pub(crate) fn is_content_path_with_nested(&self, path: &Path, nested: &[String]) -> bool {
        self.is_content_path(path)
            && !is_nested_managed_path_with_list(&self.managed_dir, path, nested)
    }

    pub(crate) fn is_ignored(&self, path: &Path) -> bool {
        matcher_matches(self.ignore_matcher.as_ref(), &self.managed_dir, path)
    }

    pub(crate) fn file_reference(&self, reference: &str) -> Option<String> {
        let normalized = self.normalize_file_reference(reference)?;
        (!self.is_ignored(Path::new(&normalized))).then_some(normalized)
    }

    fn is_base_content_path(&self, path: &Path) -> bool {
        path != self.managed_dir
            && path.starts_with(&self.managed_dir)
            && !path.starts_with(&self.agent_home)
            && !path_has_agent_home_component(&self.managed_dir, path)
    }

    fn normalize_file_reference(&self, reference: &str) -> Option<String> {
        let reference = reference.trim();
        if reference.is_empty() {
            return None;
        }

        let path = Path::new(reference);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.managed_dir.join(path)
        };

        if matches!(path.try_exists(), Ok(true)) {
            let canonical = path.canonicalize().ok()?;
            return canonical
                .starts_with(&self.managed_dir)
                .then(|| canonical.display().to_string());
        }

        path.starts_with(&self.managed_dir)
            .then(|| path.display().to_string())
    }
}

pub(crate) fn relative_path(managed_dir: &Path, path: &Path) -> io::Result<String> {
    Ok(path
        .strip_prefix(managed_dir)
        .map_err(io::Error::other)?
        .to_string_lossy()
        .replace('\\', "/"))
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
            results.push(relative_path(managed_dir, &path)?);
            continue;
        }
        collect_nested_managed_relative_paths(managed_dir, &path, results)?;
    }
    Ok(())
}

fn is_nested_managed_root(managed_dir: &Path, path: &Path) -> bool {
    path != managed_dir
        && path.starts_with(managed_dir)
        && path.join(AGENT_HOME_DIR).join("identity").is_file()
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

fn is_nested_managed_path_with_list(managed_dir: &Path, path: &Path, nested: &[String]) -> bool {
    for relative in nested {
        let nested_root = managed_dir.join(relative);
        if path == nested_root || path.starts_with(&nested_root) {
            return true;
        }
    }
    false
}

fn path_has_agent_home_component(managed_dir: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(managed_dir) else {
        return false;
    };

    relative
        .components()
        .any(|component| component.as_os_str() == AGENT_HOME_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "afs-managed-subtree-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn content_path_excludes_agent_home_and_nested_managed_directories() {
        let managed_dir = unique_dir("content");
        let agent_home = managed_dir.join(AGENT_HOME_DIR);
        let nested_dir = managed_dir.join("child");
        std::fs::create_dir_all(agent_home.join("history")).expect("create agent home");
        std::fs::create_dir_all(nested_dir.join(AGENT_HOME_DIR)).expect("create nested home");
        std::fs::write(
            nested_dir.join(AGENT_HOME_DIR).join("identity"),
            "agent-child",
        )
        .expect("write nested identity");
        std::fs::write(managed_dir.join("owned.txt"), "owned").expect("write owned file");

        let subtree = ManagedSubtree::new(&managed_dir, &agent_home);

        assert!(subtree.is_content_path(&managed_dir.join("owned.txt")));
        assert!(!subtree.is_content_path(&agent_home.join("history")));
        assert!(!subtree.is_content_path(&nested_dir.join("owned-by-child.txt")));

        let _ = std::fs::remove_dir_all(managed_dir);
    }

    #[test]
    fn file_reference_filters_ignored_paths_and_symlink_escape() {
        let managed_dir = unique_dir("reference");
        let agent_home = managed_dir.join(AGENT_HOME_DIR);
        let outside_dir = unique_dir("outside");
        std::fs::create_dir_all(&agent_home).expect("create agent home");
        std::fs::create_dir_all(managed_dir.join("secrets")).expect("create secrets dir");
        std::fs::create_dir_all(&outside_dir).expect("create outside dir");
        std::fs::write(agent_home.join(IGNORE_FILE), "secrets/\n").expect("write ignore file");
        std::fs::write(managed_dir.join("README.md"), "readme").expect("write readme");
        std::fs::write(managed_dir.join("secrets/token.txt"), "token").expect("write secret");
        std::fs::write(outside_dir.join("escape.txt"), "escape").expect("write outside");
        symlink(outside_dir.join("escape.txt"), managed_dir.join("escape"))
            .expect("create escape symlink");

        let subtree = ManagedSubtree::new(&managed_dir, &agent_home);

        assert!(subtree.file_reference("README.md").is_some());
        assert!(subtree.file_reference("secrets/token.txt").is_none());
        assert!(subtree.file_reference("escape").is_none());

        let _ = std::fs::remove_dir_all(managed_dir);
        let _ = std::fs::remove_dir_all(outside_dir);
    }
}
