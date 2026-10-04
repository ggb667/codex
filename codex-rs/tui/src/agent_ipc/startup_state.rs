//! Canonical startup state for managed agent sessions.
//!
//! Assignment is durable authority; workspace observation is disposable
//! runtime evidence.  Keeping them in separate records prevents git
//! preflight from accidentally changing the task an agent was assigned.

use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::fs::File;
use std::io;
use std::io::ErrorKind;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Assignment {
    pub(crate) task: String,
    pub(crate) status: String,
    pub(crate) next: String,
    pub(crate) blocker: Option<String>,
    #[serde(default)]
    pub(crate) approvals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct WorkspaceObservation {
    pub(crate) branch: String,
    pub(crate) worktree: PathBuf,
    pub(crate) push_status: String,
    pub(crate) observed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct CanonicalAgentState {
    #[serde(rename = "schemaVersion")]
    pub(crate) schema_version: u32,
    #[serde(rename = "agentId")]
    pub(crate) agent_id: String,
    pub(crate) revision: u64,
    #[serde(rename = "updatedAt")]
    pub(crate) updated_at: String,
    #[serde(rename = "updatedBy")]
    pub(crate) updated_by: String,
    pub(crate) assignment: Assignment,
    #[serde(rename = "workspaceObservation")]
    pub(crate) workspace_observation: WorkspaceObservation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GeneratedViewStamp {
    #[serde(rename = "sourceRevision")]
    pub(crate) source_revision: u64,
    #[serde(rename = "generatedAt")]
    pub(crate) generated_at: String,
}

pub(crate) fn read(path: &Path) -> io::Result<Option<CanonicalAgentState>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| io::Error::new(ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn read_for_startup(path: &Path) -> io::Result<Option<CanonicalAgentState>> {
    read(path)
}

pub(crate) fn compare_and_swap(
    path: &Path,
    expected_revision: Option<u64>,
    mut next: CanonicalAgentState,
) -> io::Result<CanonicalAgentState> {
    let lock_path = path.with_extension("json.lock");
    let _lock = LockFile::acquire(&lock_path)?;
    let current = read(path)?;
    let current_revision = current.as_ref().map(|state| state.revision);
    if current_revision != expected_revision {
        return Err(io::Error::new(
            ErrorKind::WouldBlock,
            format!(
                "startup state revision changed: expected {expected_revision:?}, got {current_revision:?}"
            ),
        ));
    }
    next.revision = expected_revision.map_or(1, |revision| revision + 1);
    next.updated_at = now();
    let bytes = serde_json::to_vec_pretty(&next).map_err(io::Error::other)?;
    atomic_replace(path, &bytes)?;
    Ok(next)
}

pub(crate) fn view_stamp(state: &CanonicalAgentState) -> GeneratedViewStamp {
    GeneratedViewStamp {
        source_revision: state.revision,
        generated_at: now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CanonicalAgentState {
        CanonicalAgentState {
            schema_version: 1,
            agent_id: "TEST_AGENT".to_string(),
            revision: 0,
            updated_at: String::new(),
            updated_by: "TEST_AGENT".to_string(),
            assignment: Assignment {
                task: "test".to_string(),
                status: "ready".to_string(),
                next: "run".to_string(),
                blocker: None,
                approvals: vec![],
            },
            workspace_observation: WorkspaceObservation {
                branch: "main".to_string(),
                worktree: PathBuf::from("/tmp/test"),
                push_status: "clean".to_string(),
                observed_at: String::new(),
            },
        }
    }

    #[test]
    fn compare_and_swap_stamps_revision_and_rejects_stale_writer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let first = compare_and_swap(&path, None, sample()).unwrap();
        assert_eq!(first.revision, 1);
        let second = compare_and_swap(&path, Some(1), first.clone()).unwrap();
        assert_eq!(second.revision, 2);
        assert_eq!(read(&path).unwrap(), Some(second.clone()));
        assert_eq!(view_stamp(&second).source_revision, 2);
        assert_eq!(
            compare_and_swap(&path, Some(1), sample())
                .unwrap_err()
                .kind(),
            ErrorKind::WouldBlock
        );
    }
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("canonical state path has no parent"))?;
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::other("canonical state path has no filename"))?;
    let temp_path = parent.join(format!(
        ".{}.tmp-{}",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    {
        let mut file = File::create(&temp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(temp_path, path)
}

fn now() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_secs()
        .to_string()
}

struct LockFile {
    path: PathBuf,
}

impl LockFile {
    fn acquire(path: &Path) -> io::Result<Self> {
        match File::options().write(true).create_new(true).open(path) {
            Ok(_) => Ok(Self {
                path: path.to_path_buf(),
            }),
            Err(error) => Err(error),
        }
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
