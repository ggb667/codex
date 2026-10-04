//! Canonical startup state for managed agent sessions.
//!
//! Assignment is durable authority; workspace observation is disposable
//! runtime evidence.  Keeping them in separate records prevents git
//! preflight from accidentally changing the task an agent was assigned.

use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::io;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

const SUPPORTED_SCHEMA_VERSION: u32 = 1;

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

pub(crate) fn read(path: &Path) -> io::Result<Option<CanonicalAgentState>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| io::Error::new(ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn read_for_startup(
    path: &Path,
    expected_agent_id: &str,
) -> io::Result<Option<CanonicalAgentState>> {
    let Some(state) = read(path)? else {
        return Ok(None);
    };
    validate_state(&state, expected_agent_id)?;
    Ok(Some(state))
}

fn validate_state(state: &CanonicalAgentState, expected_agent_id: &str) -> io::Result<()> {
    if state.schema_version != SUPPORTED_SCHEMA_VERSION {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "unsupported canonical agent state schema version {}",
                state.schema_version
            ),
        ));
    }
    if state.agent_id != expected_agent_id {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "canonical agent state belongs to {}, expected {expected_agent_id}",
                state.agent_id
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "startup_state_tests.rs"]
mod tests;
