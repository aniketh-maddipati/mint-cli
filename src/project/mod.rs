use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

/// Stable identifier for a workspace, derived from `git remote` origin URL.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectId(pub String);

impl ProjectId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub struct ProjectRegistry;

impl ProjectRegistry {
    /// Resolve the current project from `cwd`'s git origin remote, falling back
    /// to a hash of the absolute path when not in a git repo.
    pub fn detect(cwd: &Path) -> ProjectId {
        detect_git_remote_id(cwd).unwrap_or_else(|| local_path_id(cwd))
    }

    pub fn cwd() -> Result<PathBuf> {
        std::env::current_dir().context("current working directory")
    }

    pub fn detect_current() -> Result<(ProjectId, PathBuf)> {
        let cwd = Self::cwd()?;
        Ok((Self::detect(&cwd), cwd))
    }

    /// Per-project data root: `~/.local/share/mint-cli/projects/{id}/`
    pub fn data_dir(id: &ProjectId) -> Result<PathBuf> {
        let dirs = ProjectDirs::from("dev", "mint", "mint-cli")
            .context("could not resolve data directory")?;
        Ok(dirs.data_dir().join("projects").join(id.as_str()))
    }
}

/// Normalize `git@github.com:user/repo.git` → `github.com/user/repo`.
fn normalize_remote_url(raw: &str) -> String {
    let s = raw.trim().trim_end_matches(".git");
    if let Some(rest) = s.strip_prefix("git@") {
        if let Some((host, path)) = rest.split_once(':') {
            return format!("{host}/{path}");
        }
    }
    if let Some(rest) = s.strip_prefix("https://") {
        return rest.trim_start_matches('/').to_string();
    }
    if let Some(rest) = s.strip_prefix("http://") {
        return rest.trim_start_matches('/').to_string();
    }
    s.replace(':', "/")
}

fn detect_git_remote_id(cwd: &Path) -> Option<ProjectId> {
    let output = Command::new("git")
        .args(["-C", cwd.to_str()?, "remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let id = normalize_remote_url(raw.trim());
    if id.is_empty() {
        return None;
    }
    Some(ProjectId(id))
}

fn local_path_id(cwd: &Path) -> ProjectId {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let abs = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let mut hasher = DefaultHasher::new();
    abs.hash(&mut hasher);
    ProjectId(format!("local/{:016x}", hasher.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ssh_remote() {
        assert_eq!(
            normalize_remote_url("git@github.com:acme/mint-cli.git"),
            "github.com/acme/mint-cli"
        );
    }

    #[test]
    fn normalize_https_remote() {
        assert_eq!(
            normalize_remote_url("https://github.com/acme/mint-cli.git"),
            "github.com/acme/mint-cli"
        );
    }

    #[test]
    fn local_path_id_is_stable() {
        let id1 = local_path_id(Path::new("/tmp/test"));
        let id2 = local_path_id(Path::new("/tmp/test"));
        assert_eq!(id1, id2);
        assert!(id1.as_str().starts_with("local/"));
    }
}
