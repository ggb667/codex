use super::roster::AgentConfig;
use super::roster::AgentConfigAgent;
use super::storage::agent_ipc_log_path_for;
use super::storage::append_chat_message_at;
use super::storage::append_json_line;
use super::storage::append_registry_heartbeat_at;
use super::storage::read_jsonl;
use super::storage::read_new_messages_at;
use super::*;
use chrono::Duration as ChronoDuration;
use tempfile::tempdir;

fn sample_identity() -> AgentIdentity {
    AgentIdentity {
        instance_id: "uuid-1".to_string(),
        agent_name: "TWILIGHT_SPARKLE".to_string(),
        agent_symbol: "✶".to_string(),
        agent_aliases: vec![
            "TWILIGHT_SPARKLE".to_string(),
            "Twilight Sparkle".to_string(),
            "Twilight".to_string(),
        ],
        mailbox_path: None,
        project_path: "/tmp/project".to_string(),
        git_branch: "agent/twi/main".to_string(),
        pid: 42,
    }
}

fn sample_roster() -> AgentConfig {
    AgentConfig {
        agent_id: "TWILIGHT_SPARKLE".to_string(),
        route_id: "CODEX:TWILIGHT_SPARKLE".to_string(),
        label: "Twilight Sparkle".to_string(),
        icon: "✶".to_string(),
        aliases: vec![
            "Twilight Sparkle".to_string(),
            "Twilight".to_string(),
            "CODEX:Twilight Sparkle".to_string(),
        ],
        project_root: "/tmp/codex".to_string(),
        branch_label: "main".to_string(),
        mailbox_path: "/tmp/codex/agent/team.coordination/twi.mailbox.md".to_string(),
        message_log_path: "/tmp/codex/agent/runtime/agent.chat.jsonl".to_string(),
        registry_path: "/tmp/codex/agent/runtime/agent.registry.jsonl".to_string(),
        global_singleton: false,
        agents: vec![
            AgentConfigAgent {
                agent_id: "TWILIGHT_SPARKLE".to_string(),
                route_id: "EVH:TWILIGHT_SPARKLE".to_string(),
                label: "Twilight Sparkle".to_string(),
                icon: "✶".to_string(),
                aliases: vec![
                    "Twilight Sparkle".to_string(),
                    "Twilight".to_string(),
                    "EVH:Twilight Sparkle".to_string(),
                ],
                project_root: "/tmp/evh".to_string(),
                mailbox_path: "/tmp/evh/agent/team.coordination/twi.mailbox.md".to_string(),
                message_log_path: "/tmp/evh/agent/runtime/agent.chat.jsonl".to_string(),
                registry_path: "/tmp/evh/agent/runtime/agent.registry.jsonl".to_string(),
                global_singleton: false,
            },
            AgentConfigAgent {
                agent_id: "GLOBAL_COORDINATOR".to_string(),
                route_id: "GLOBAL_COORDINATOR".to_string(),
                label: "Global Coordinator".to_string(),
                icon: "☀︎".to_string(),
                aliases: vec!["Global Coordinator".to_string(), "Coordinator".to_string()],
                project_root: "/tmp/foreign-agent-system".to_string(),
                mailbox_path:
                    "/tmp/foreign-agent-system/agent/team.coordination/global-coordinator.mailbox.md"
                        .to_string(),
                message_log_path: "/tmp/foreign-agent-system/agent/runtime/agent.chat.jsonl"
                    .to_string(),
                registry_path: "/tmp/foreign-agent-system/agent/runtime/agent.registry.jsonl"
                    .to_string(),
                global_singleton: true,
            },
        ],
    }
}

#[test]
fn parse_send_command_supports_list_direct_and_broadcast() {
    assert_eq!(
        parse_send_command_with_roster("list", /*roster*/ None).unwrap(),
        AgentSendCommand::List
    );
    assert_eq!(
        parse_send_command_with_roster("PINKIE_PIE hello there", /*roster*/ None).unwrap(),
        AgentSendCommand::Send {
            target: "PINKIE_PIE".to_string(),
            text: "hello there".to_string(),
            delivery_class: DeliveryClass::Ephemeral,
        }
    );
    assert_eq!(
        parse_send_command_with_roster("all status check", /*roster*/ None).unwrap(),
        AgentSendCommand::Send {
            target: "*".to_string(),
            text: "status check".to_string(),
            delivery_class: DeliveryClass::Ephemeral,
        }
    );
    assert_eq!(
        parse_send_command_with_roster("durable PINKIE_PIE keep this", /*roster*/ None).unwrap(),
        AgentSendCommand::Send {
            target: "PINKIE_PIE".to_string(),
            text: "keep this".to_string(),
            delivery_class: DeliveryClass::Durable,
        }
    );
}

#[test]
fn roster_keeps_unqualified_ambiguous_aliases_local() {
    assert_eq!(
        sample_roster().resolve_route("Twilight").unwrap(),
        "CODEX:TWILIGHT_SPARKLE"
    );
}

#[test]
fn roster_allows_explicit_qualified_cross_repo_targets() {
    assert_eq!(
        sample_roster()
            .resolve_route("EVH:Twilight Sparkle")
            .unwrap(),
        "EVH:TWILIGHT_SPARKLE"
    );
}

#[test]
fn roster_selects_target_message_log_for_qualified_cross_repo_target() {
    let target = sample_roster()
        .resolve_agent("EVH:Twilight Sparkle")
        .unwrap();
    assert_eq!(
        non_empty_path(&target.message_log_path).unwrap(),
        PathBuf::from("/tmp/evh/agent/runtime/agent.chat.jsonl")
    );
}

#[test]
fn roster_uses_configured_global_singleton() {
    assert_eq!(
        sample_roster().resolve_route("Coordinator").unwrap(),
        "GLOBAL_COORDINATOR"
    );
}

#[test]
fn roster_does_not_treat_non_singleton_as_global() {
    let mut roster = sample_roster();
    roster.agents[1].global_singleton = false;

    let err = roster.resolve_route("Coordinator").unwrap_err();
    assert!(err.contains("Ambiguous agent 'Coordinator'"));
}

#[test]
fn roster_rejects_unknown_target() {
    let err = sample_roster().resolve_route("discord").unwrap_err();
    assert!(err.contains("Unknown agent 'discord'"));
}

#[test]
fn read_new_messages_keeps_only_latest_message_per_sender() {
    let temp = tempdir().unwrap();
    let chat_path = temp.path().join("chat.jsonl");
    let lock_path = temp.path().join("chat.lock");
    let identity = sample_identity();
    let older_from_pinkie = AgentMessage {
        id: "msg-1".to_string(),
        from_instance_id: "uuid-2".to_string(),
        from_agent_name: "PINKIE_PIE".to_string(),
        from_symbol: "🎈".to_string(),
        to: "TWILIGHT_SPARKLE".to_string(),
        subject: "older waiting note".to_string(),
        body: String::new(),
        created_at: Utc::now() - ChronoDuration::seconds(5),
        delivery_class: DeliveryClass::Ephemeral,
    };
    let fresh_from_pinkie = AgentMessage {
        id: "msg-2".to_string(),
        from_instance_id: "uuid-2".to_string(),
        from_agent_name: "PINKIE_PIE".to_string(),
        from_symbol: "🎈".to_string(),
        to: "TWILIGHT_SPARKLE".to_string(),
        subject: "latest waiting note".to_string(),
        body: String::new(),
        created_at: Utc::now(),
        delivery_class: DeliveryClass::Ephemeral,
    };
    let stale = AgentMessage {
        id: "msg-3".to_string(),
        from_instance_id: "uuid-3".to_string(),
        from_agent_name: "APPLEJACK".to_string(),
        from_symbol: "🍎".to_string(),
        to: "TWILIGHT_SPARKLE".to_string(),
        subject: "old message".to_string(),
        body: String::new(),
        created_at: Utc::now() - ChronoDuration::hours(2),
        delivery_class: DeliveryClass::Ephemeral,
    };
    let own = AgentMessage {
        id: "msg-4".to_string(),
        from_instance_id: identity.instance_id.clone(),
        from_agent_name: identity.agent_name.clone(),
        from_symbol: "✶".to_string(),
        to: "TWILIGHT_SPARKLE".to_string(),
        subject: "self".to_string(),
        body: String::new(),
        created_at: Utc::now(),
        delivery_class: DeliveryClass::Ephemeral,
    };
    let fresh_from_dash = AgentMessage {
        id: "msg-5".to_string(),
        from_instance_id: "uuid-5".to_string(),
        from_agent_name: "RAINBOW_DASH".to_string(),
        from_symbol: "⚡".to_string(),
        to: "TWILIGHT_SPARKLE".to_string(),
        subject: "dash status".to_string(),
        body: String::new(),
        created_at: Utc::now() - ChronoDuration::seconds(1),
        delivery_class: DeliveryClass::Ephemeral,
    };
    append_json_line(&chat_path, &older_from_pinkie).unwrap();
    append_json_line(&chat_path, &fresh_from_pinkie).unwrap();
    append_json_line(&chat_path, &stale).unwrap();
    append_json_line(&chat_path, &own).unwrap();
    append_json_line(&chat_path, &fresh_from_dash).unwrap();

    let first = read_new_messages_at(&chat_path, &lock_path, &identity).unwrap();
    assert_eq!(
        first,
        vec![fresh_from_dash.clone(), fresh_from_pinkie.clone()]
    );

    let second = read_new_messages_at(&chat_path, &lock_path, &identity).unwrap();
    assert_eq!(second, vec![fresh_from_dash, fresh_from_pinkie]);
}

#[test]
fn read_new_messages_keeps_stale_durable_messages() {
    let temp = tempdir().unwrap();
    let chat_path = temp.path().join("chat.jsonl");
    let lock_path = temp.path().join("chat.lock");
    let identity = sample_identity();
    let durable = AgentMessage {
        id: "msg-durable".to_string(),
        from_instance_id: "uuid-6".to_string(),
        from_agent_name: "PINKIE_PIE".to_string(),
        from_symbol: "🎈".to_string(),
        to: "TWILIGHT_SPARKLE".to_string(),
        subject: "durable note".to_string(),
        body: String::new(),
        created_at: Utc::now() - ChronoDuration::hours(2),
        delivery_class: DeliveryClass::Durable,
    };
    append_json_line(&chat_path, &durable).unwrap();

    assert_eq!(
        read_new_messages_at(&chat_path, &lock_path, &identity).unwrap(),
        vec![durable]
    );
}

#[test]
fn parse_send_command_splits_subject_and_body() {
    let entry = append_chat_message_at(
        Path::new("/tmp/chat.jsonl"),
        Path::new("/tmp/chat.lock"),
        &sample_identity(),
        "TWILIGHT_SPARKLE",
        "databases should use RDS. Please update the schema tonight.",
        DeliveryClass::Ephemeral,
    )
    .unwrap();
    assert_eq!(entry.subject, "databases should use RDS");
    assert_eq!(entry.body, ". Please update the schema tonight.");
    assert_eq!(entry.from_symbol, "✶");
}

#[test]
fn mailbox_markdown_uses_sender_symbol() {
    let entry = AgentMessage {
        id: "msg-4".to_string(),
        from_instance_id: "uuid-4".to_string(),
        from_agent_name: "APPLEJACK".to_string(),
        from_symbol: "🍎".to_string(),
        to: "TWILIGHT_SPARKLE".to_string(),
        subject: "databases should use RDS".to_string(),
        body: " Please update the schema tonight.".to_string(),
        created_at: Utc::now(),
        delivery_class: DeliveryClass::Ephemeral,
    };
    let rendered = entry.mailbox_markdown();
    assert!(rendered.contains("FROM: 🍎 Applejack"));
    assert!(rendered.contains("SUBJECT: databases should use RDS"));
    assert!(rendered.contains("Please update the schema tonight."));
}

#[test]
fn agent_wire_format_writes_generic_keys_and_reads_legacy_keys() {
    let message = AgentMessage {
        id: "msg-wire".to_string(),
        from_instance_id: "uuid-wire".to_string(),
        from_agent_name: "AGENT_ONE".to_string(),
        from_symbol: "A".to_string(),
        to: "AGENT_TWO".to_string(),
        subject: "status".to_string(),
        body: String::new(),
        created_at: Utc::now(),
        delivery_class: DeliveryClass::Ephemeral,
    };
    let serialized = serde_json::to_value(&message).unwrap();
    assert_eq!(serialized["from_agent_name"], "AGENT_ONE");
    assert!(serialized.get("from_pony_name").is_none());

    let mut legacy = serialized;
    let legacy_name = legacy
        .as_object_mut()
        .unwrap()
        .remove("from_agent_name")
        .unwrap();
    legacy["from_pony_name"] = legacy_name;
    let parsed: AgentMessage = serde_json::from_value(legacy).unwrap();
    assert_eq!(parsed.from_agent_name, "AGENT_ONE");
}

#[test]
fn stale_registry_log_is_removed_before_next_heartbeat() {
    let temp = tempdir().unwrap();
    let registry_path = temp.path().join("registry.jsonl");
    let lock_path = temp.path().join("registry.lock");
    let stale_entry = AgentRegistryEntry {
        uuid: "old".to_string(),
        agent_name: "PINKIE_PIE".to_string(),
        path: "/tmp/old".to_string(),
        git_branch: "old-branch".to_string(),
        pid: 9,
        last_seen_at: Utc::now() - ChronoDuration::hours(2),
    };
    append_json_line(&registry_path, &stale_entry).unwrap();

    append_registry_heartbeat_at(&registry_path, &lock_path, &sample_identity()).unwrap();
    let entries = read_jsonl::<AgentRegistryEntry>(&registry_path).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].agent_name, "TWILIGHT_SPARKLE");
}
#[test]
fn ipc_log_path_defaults_to_project_runtime_when_project_root_is_set() {
    assert_eq!(
        agent_ipc_log_path_for(
            /*config_path*/ None,
            Some(Path::new("/tmp/project")),
            "agent.chat.jsonl",
            "codex-agent-chat.jsonl",
        ),
        PathBuf::from("/tmp/project/.codex/agent-ipc/agent.chat.jsonl")
    );
}

#[test]
fn ipc_log_path_uses_neutral_current_directory_fallback() {
    assert_eq!(
        agent_ipc_log_path_for(
            /*config_path*/ None,
            Some(Path::new("/tmp/other")),
            "agent.chat.jsonl",
            "codex-agent-chat.jsonl",
        ),
        PathBuf::from("/tmp/other/.codex/agent-ipc/agent.chat.jsonl")
    );
}

#[test]
fn ipc_log_path_uses_config_path() {
    assert_eq!(
        agent_ipc_log_path_for(
            Some(Path::new("/tmp/config/agent.chat.jsonl")),
            Some(Path::new("/tmp/other")),
            "agent.chat.jsonl",
            "codex-agent-chat.jsonl",
        ),
        PathBuf::from("/tmp/config/agent.chat.jsonl")
    );
}

#[test]
fn ipc_log_path_uses_current_dir_before_legacy_tmp_fallback() {
    assert_eq!(
        agent_ipc_log_path_for(
            /*config_path*/ None,
            Some(Path::new("/tmp/cwd")),
            "agent.chat.jsonl",
            "codex-agent-chat.jsonl",
        ),
        PathBuf::from("/tmp/cwd/.codex/agent-ipc/agent.chat.jsonl")
    );
}

#[test]
fn ipc_log_path_uses_config_path_under_project_root() {
    assert_eq!(
        agent_ipc_log_path_for(
            Some(Path::new("/tmp/project/.config/agent.chat.jsonl")),
            Some(Path::new("/tmp/other")),
            "agent.chat.jsonl",
            "codex-agent-chat.jsonl",
        ),
        PathBuf::from("/tmp/project/.config/agent.chat.jsonl")
    );
}
