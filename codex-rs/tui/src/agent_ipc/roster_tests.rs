use super::*;

fn parse_config(launch_policy: &str) -> AgentConfig {
    serde_json::from_str(&format!(
        r#"{{
            "agentId": "PROJECT:AGENT",
            "routeId": "PROJECT:AGENT",
            "label": "Agent",
            "icon": "A",
            "projectRoot": "/project",
            "branchLabel": "main",
            "launchPolicy": {launch_policy}
        }}"#
    ))
    .unwrap()
}

#[test]
fn only_schema_one_paused_workers_require_the_launch_gate() {
    let paused_worker =
        parse_config(r#"{"schemaVersion": 1, "role": "worker", "startMode": "paused"}"#);
    let active_worker =
        parse_config(r#"{"schemaVersion": 1, "role": "worker", "startMode": "active"}"#);
    let coordinator =
        parse_config(r#"{"schemaVersion": 1, "role": "coordinator", "startMode": "paused"}"#);

    assert!(paused_worker.starts_paused());
    assert!(!active_worker.starts_paused());
    assert!(!coordinator.starts_paused());
}

#[test]
fn legacy_config_without_a_launch_policy_starts_active() {
    let config: AgentConfig = serde_json::from_str(
        r#"{
            "agentId": "PROJECT:AGENT",
            "routeId": "PROJECT:AGENT",
            "label": "Agent",
            "icon": "A",
            "projectRoot": "/project",
            "branchLabel": "main"
        }"#,
    )
    .unwrap();

    assert!(!config.starts_paused());
}
