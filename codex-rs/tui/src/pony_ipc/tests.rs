use super::roster::AgentConfig;
use super::roster::AgentConfigAgent;
use super::storage::append_chat_message_at;
use super::storage::append_json_line;
use super::storage::append_registry_heartbeat_at;
use super::storage::pony_ipc_log_path_for;
use super::storage::read_jsonl;
use super::storage::read_new_messages_at;
use super::*;
use chrono::Duration as ChronoDuration;
use tempfile::tempdir;

fn sample_identity() -> PonyIdentity {
    PonyIdentity {
        instance_id: "uuid-1".to_string(),
        pony_name: "TWILIGHT_SPARKLE".to_string(),
        pony_symbol: "✶".to_string(),
        pony_aliases: vec![
            "TWILIGHT_SPARKLE".to_string(),
            "Twilight Sparkle".to_string(),
            "Twilight".to_string(),
        ],
        mailbox_path: None,
        project_path: "/tmp/project".to_string(),
        git_branch: "pony/twi/main".to_string(),
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
        mailbox_path: "/tmp/codex/pony/team.coordination/twi.mailbox.md".to_string(),
        message_log_path: "/tmp/codex/pony/runtime/pony.chat.jsonl".to_string(),
        registry_path: "/tmp/codex/pony/runtime/pony.registry.jsonl".to_string(),
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
                mailbox_path: "/tmp/evh/pony/team.coordination/twi.mailbox.md".to_string(),
                message_log_path: "/tmp/evh/pony/runtime/pony.chat.jsonl".to_string(),
                registry_path: "/tmp/evh/pony/runtime/pony.registry.jsonl".to_string(),
                global_singleton: false,
            },
            AgentConfigAgent {
                agent_id: "PRINCESS_CELESTIA_SOL_INVICTUS".to_string(),
                route_id: "PRINCESS_CELESTIA_SOL_INVICTUS".to_string(),
                label: "Princess Celestia Sol Invictus".to_string(),
                icon: "☀︎".to_string(),
                aliases: vec![
                    "Princess Celestia Sol Invictus".to_string(),
                    "Celestia".to_string(),
                ],
                project_root: "/tmp/agenic-pony-system".to_string(),
                mailbox_path: "/tmp/agenic-pony-system/pony/team.coordination/celestia.mailbox.md"
                    .to_string(),
                message_log_path: "/tmp/agenic-pony-system/pony/runtime/pony.chat.jsonl"
                    .to_string(),
                registry_path: "/tmp/agenic-pony-system/pony/runtime/pony.registry.jsonl"
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
        PonySendCommand::List
    );
    assert_eq!(
        parse_send_command_with_roster("PINKIE_PIE hello there", /*roster*/ None).unwrap(),
        PonySendCommand::Send {
            target: "PINKIE_PIE".to_string(),
            text: "hello there".to_string(),
        }
    );
    assert_eq!(
        parse_send_command_with_roster("all status check", /*roster*/ None).unwrap(),
        PonySendCommand::Send {
            target: "*".to_string(),
            text: "status check".to_string(),
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
        PathBuf::from("/tmp/evh/pony/runtime/pony.chat.jsonl")
    );
}

#[test]
fn roster_preserves_celestia_as_singleton() {
    assert_eq!(
        sample_roster().resolve_route("Celestia").unwrap(),
        "PRINCESS_CELESTIA_SOL_INVICTUS"
    );
}

#[test]
fn roster_rejects_unknown_target() {
    let err = sample_roster().resolve_route("discord").unwrap_err();
    assert!(err.contains("Unknown pony 'discord'"));
}

#[test]
fn read_new_messages_keeps_only_latest_message_per_sender() {
    let temp = tempdir().unwrap();
    let chat_path = temp.path().join("chat.jsonl");
    let lock_path = temp.path().join("chat.lock");
    let identity = sample_identity();
    let older_from_pinkie = PonyChatEntry {
        id: "msg-1".to_string(),
        from_instance_id: "uuid-2".to_string(),
        from_pony_name: "PINKIE_PIE".to_string(),
        from_symbol: "🎈".to_string(),
        to: "TWILIGHT_SPARKLE".to_string(),
        subject: "older waiting note".to_string(),
        body: String::new(),
        created_at: Utc::now() - ChronoDuration::seconds(5),
    };
    let fresh_from_pinkie = PonyChatEntry {
        id: "msg-2".to_string(),
        from_instance_id: "uuid-2".to_string(),
        from_pony_name: "PINKIE_PIE".to_string(),
        from_symbol: "🎈".to_string(),
        to: "TWILIGHT_SPARKLE".to_string(),
        subject: "latest waiting note".to_string(),
        body: String::new(),
        created_at: Utc::now(),
    };
    let stale = PonyChatEntry {
        id: "msg-3".to_string(),
        from_instance_id: "uuid-3".to_string(),
        from_pony_name: "APPLEJACK".to_string(),
        from_symbol: "🍎".to_string(),
        to: "TWILIGHT_SPARKLE".to_string(),
        subject: "old message".to_string(),
        body: String::new(),
        created_at: Utc::now() - ChronoDuration::hours(2),
    };
    let own = PonyChatEntry {
        id: "msg-4".to_string(),
        from_instance_id: identity.instance_id.clone(),
        from_pony_name: identity.pony_name.clone(),
        from_symbol: "✶".to_string(),
        to: "TWILIGHT_SPARKLE".to_string(),
        subject: "self".to_string(),
        body: String::new(),
        created_at: Utc::now(),
    };
    let fresh_from_dash = PonyChatEntry {
        id: "msg-5".to_string(),
        from_instance_id: "uuid-5".to_string(),
        from_pony_name: "RAINBOW_DASH".to_string(),
        from_symbol: "⚡".to_string(),
        to: "TWILIGHT_SPARKLE".to_string(),
        subject: "dash status".to_string(),
        body: String::new(),
        created_at: Utc::now() - ChronoDuration::seconds(1),
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
fn parse_send_command_splits_subject_and_body() {
    let entry = append_chat_message_at(
        Path::new("/tmp/chat.jsonl"),
        Path::new("/tmp/chat.lock"),
        &sample_identity(),
        "TWILIGHT_SPARKLE",
        "databases should use RDS. Please update the schema tonight.",
    )
    .unwrap();
    assert_eq!(entry.subject, "databases should use RDS");
    assert_eq!(entry.body, ". Please update the schema tonight.");
    assert_eq!(entry.from_symbol, "✶");
}

#[test]
fn mailbox_markdown_uses_sender_symbol() {
    let entry = PonyChatEntry {
        id: "msg-4".to_string(),
        from_instance_id: "uuid-4".to_string(),
        from_pony_name: "APPLEJACK".to_string(),
        from_symbol: "🍎".to_string(),
        to: "TWILIGHT_SPARKLE".to_string(),
        subject: "databases should use RDS".to_string(),
        body: " Please update the schema tonight.".to_string(),
        created_at: Utc::now(),
    };
    let rendered = entry.mailbox_markdown();
    assert!(rendered.contains("FROM: 🍎 Applejack"));
    assert!(rendered.contains("SUBJECT: databases should use RDS"));
    assert!(rendered.contains("Please update the schema tonight."));
}

#[test]
fn stale_registry_log_is_removed_before_next_heartbeat() {
    let temp = tempdir().unwrap();
    let registry_path = temp.path().join("registry.jsonl");
    let lock_path = temp.path().join("registry.lock");
    let stale_entry = PonyRegistryEntry {
        uuid: "old".to_string(),
        pony_name: "PINKIE_PIE".to_string(),
        path: "/tmp/old".to_string(),
        git_branch: "old-branch".to_string(),
        pid: 9,
        last_seen_at: Utc::now() - ChronoDuration::hours(2),
    };
    append_json_line(&registry_path, &stale_entry).unwrap();

    append_registry_heartbeat_at(&registry_path, &lock_path, &sample_identity()).unwrap();
    let entries = read_jsonl::<PonyRegistryEntry>(&registry_path).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].pony_name, "TWILIGHT_SPARKLE");
}
#[test]
fn ipc_log_path_defaults_to_project_runtime_when_project_root_is_set() {
    assert_eq!(
        pony_ipc_log_path_for(
            /*explicit_path*/ None,
            /*config_path*/ None,
            Some("/tmp/project"),
            Some(Path::new("/tmp/other")),
            "pony.chat.jsonl",
            "codex-pony-chat.jsonl",
        ),
        PathBuf::from("/tmp/project/pony/runtime/pony.chat.jsonl")
    );
}

#[test]
fn ipc_log_path_ignores_explicit_path_from_another_project() {
    assert_eq!(
        pony_ipc_log_path_for(
            Some("/tmp/source/pony/runtime/pony.chat.jsonl"),
            /*config_path*/ None,
            Some("/tmp/codex"),
            Some(Path::new("/tmp/other")),
            "pony.chat.jsonl",
            "codex-pony-chat.jsonl",
        ),
        PathBuf::from("/tmp/codex/pony/runtime/pony.chat.jsonl")
    );
}

#[test]
fn ipc_log_path_accepts_explicit_path_under_project_root() {
    assert_eq!(
        pony_ipc_log_path_for(
            Some("/tmp/codex/pony/runtime/custom.chat.jsonl"),
            /*config_path*/ None,
            Some("/tmp/codex"),
            Some(Path::new("/tmp/other")),
            "pony.chat.jsonl",
            "codex-pony-chat.jsonl",
        ),
        PathBuf::from("/tmp/codex/pony/runtime/custom.chat.jsonl")
    );
}

#[test]
fn ipc_log_path_uses_current_dir_before_legacy_tmp_fallback() {
    assert_eq!(
        pony_ipc_log_path_for(
            /*explicit_path*/ None,
            /*config_path*/ None,
            /*project_root*/ None,
            Some(Path::new("/tmp/cwd")),
            "pony.chat.jsonl",
            "codex-pony-chat.jsonl",
        ),
        PathBuf::from("/tmp/cwd/pony/runtime/pony.chat.jsonl")
    );
}

#[test]
fn ipc_log_path_uses_config_path_under_project_root() {
    assert_eq!(
        pony_ipc_log_path_for(
            /*explicit_path*/ None,
            Some(Path::new("/tmp/project/pony/runtime/config.chat.jsonl")),
            Some("/tmp/project"),
            Some(Path::new("/tmp/other")),
            "pony.chat.jsonl",
            "codex-pony-chat.jsonl",
        ),
        PathBuf::from("/tmp/project/pony/runtime/config.chat.jsonl")
    );
}
