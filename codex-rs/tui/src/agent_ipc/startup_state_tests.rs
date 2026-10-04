use super::*;
use pretty_assertions::assert_eq;

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
fn startup_read_rejects_wrong_schema_or_agent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.json");
    let mut state = sample();
    state.schema_version = 2;
    fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(
        read_for_startup(&path, "TEST_AGENT").unwrap_err().kind(),
        ErrorKind::InvalidData
    );

    state.schema_version = 1;
    fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(
        read_for_startup(&path, "OTHER_AGENT").unwrap_err().kind(),
        ErrorKind::InvalidData
    );
}

#[test]
fn startup_read_returns_the_complete_canonical_record() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.json");
    let state = sample();
    fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();

    assert_eq!(read_for_startup(&path, "TEST_AGENT").unwrap(), Some(state));
}
