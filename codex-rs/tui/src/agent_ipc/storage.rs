use chrono::DateTime;
use chrono::Duration as ChronoDuration;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use uuid::Uuid;

use super::AgentIdentity;
use super::AgentMessage;
use super::AgentRegistryEntry;
use super::BROADCAST_TARGET;
use super::DeliveryClass;
use super::STALE_AFTER_SECS;
use super::agent_config_from_env;
use super::canonicalize_agent_name;
use super::non_empty_path;
use super::resolve_target_agent;
use super::roster::normalize_agent_name;
use super::roster::normalize_alias;
use super::same_project;

pub(super) fn append_registry_heartbeat_at(
    registry_path: &Path,
    lock_path: &Path,
    identity: &AgentIdentity,
) -> io::Result<()> {
    maybe_reset_stale_registry_log(registry_path, lock_path)?;
    append_json_line(registry_path, &identity.registry_entry())
}

pub(super) fn append_chat_message_at(
    chat_path: &Path,
    lock_path: &Path,
    identity: &AgentIdentity,
    target: &str,
    text: &str,
    delivery_class: DeliveryClass,
) -> io::Result<AgentMessage> {
    if delivery_class == DeliveryClass::Ephemeral {
        maybe_reset_stale_chat_log(chat_path, lock_path)?;
    }
    let trimmed = text.trim();
    let (subject, body) = split_subject_and_body(trimmed);
    let entry = AgentMessage {
        id: Uuid::new_v4().to_string(),
        from_instance_id: identity.instance_id.clone(),
        from_agent_name: identity.agent_name.clone(),
        from_symbol: identity.agent_symbol.clone(),
        to: normalize_target(target),
        subject,
        body,
        created_at: Utc::now(),
        delivery_class,
    };
    append_json_line(chat_path, &entry)?;
    Ok(entry)
}

pub(super) fn read_live_registry_at(
    registry_path: &Path,
    lock_path: &Path,
) -> io::Result<Vec<AgentRegistryEntry>> {
    maybe_reset_stale_registry_log(registry_path, lock_path)?;
    let mut latest_by_uuid = HashMap::new();
    for entry in read_jsonl::<AgentRegistryEntry>(registry_path)? {
        if is_stale(entry.last_seen_at) {
            continue;
        }
        latest_by_uuid.insert(entry.uuid.clone(), entry);
    }
    let mut entries = latest_by_uuid.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.agent_name
            .cmp(&right.agent_name)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(entries)
}

pub(super) fn read_new_messages_at(
    chat_path: &Path,
    lock_path: &Path,
    identity: &AgentIdentity,
) -> io::Result<Vec<AgentMessage>> {
    maybe_reset_stale_chat_log(chat_path, lock_path)?;
    let mut latest_by_sender: HashMap<String, AgentMessage> = HashMap::new();
    for entry in read_jsonl::<AgentMessage>(chat_path)? {
        if entry.delivery_class == DeliveryClass::Ephemeral && is_stale(entry.created_at) {
            continue;
        }
        if entry.from_instance_id == identity.instance_id {
            continue;
        }
        if !target_matches(&entry.to, identity) {
            continue;
        }
        let sender = canonicalize_agent_name(&entry.from_agent_name);
        match latest_by_sender.get(&sender) {
            Some(existing) if existing.created_at >= entry.created_at => {}
            _ => {
                latest_by_sender.insert(sender, entry);
            }
        }
    }
    let mut messages = latest_by_sender.into_values().collect::<Vec<_>>();
    messages.sort_by_key(|left| left.created_at);
    Ok(messages)
}

fn maybe_reset_stale_registry_log(registry_path: &Path, lock_path: &Path) -> io::Result<()> {
    maybe_reset_stale_log(registry_path, lock_path, latest_registry_timestamp)
}

fn maybe_reset_stale_chat_log(chat_path: &Path, lock_path: &Path) -> io::Result<()> {
    maybe_reset_stale_log(chat_path, lock_path, latest_chat_timestamp)
}

fn maybe_reset_stale_log<F>(
    log_path: &Path,
    lock_path: &Path,
    latest_timestamp: F,
) -> io::Result<()>
where
    F: Fn(&Path) -> io::Result<Option<DateTime<Utc>>>,
{
    let Some(latest) = latest_timestamp(log_path)? else {
        return Ok(());
    };
    if !is_stale(latest) {
        return Ok(());
    }
    let Some(_lock) = CleanupLock::try_acquire(lock_path)? else {
        return Ok(());
    };
    let Some(rechecked) = latest_timestamp(log_path)? else {
        return Ok(());
    };
    if is_stale(rechecked) {
        remove_file_if_exists(log_path)?;
    }
    Ok(())
}

fn latest_registry_timestamp(path: &Path) -> io::Result<Option<DateTime<Utc>>> {
    Ok(read_jsonl::<AgentRegistryEntry>(path)?
        .into_iter()
        .map(|entry| entry.last_seen_at)
        .max())
}

fn latest_chat_timestamp(path: &Path) -> io::Result<Option<DateTime<Utc>>> {
    let mut latest = None;
    for entry in read_jsonl::<AgentMessage>(path)? {
        if entry.delivery_class == DeliveryClass::Durable {
            return Ok(None);
        }
        latest = latest.max(Some(entry.created_at));
    }
    Ok(latest)
}

pub(super) fn append_json_line<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(io::Error::other(
            "missing parent directory for agent IPC log",
        ));
    };
    fs::create_dir_all(parent)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, value).map_err(io::Error::other)?;
    file.write_all(b"\n")
}

pub(super) fn read_jsonl<T>(path: &Path) -> io::Result<Vec<T>>
where
    T: for<'de> Deserialize<'de>,
{
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut values = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str(trimmed) else {
            continue;
        };
        values.push(value);
    }
    Ok(values)
}

pub(super) fn agent_registry_log_path() -> PathBuf {
    let config_path =
        agent_config_from_env().and_then(|config| non_empty_path(&config.registry_path));
    agent_ipc_log_path_with_config(
        config_path.as_deref(),
        "agent.registry.jsonl",
        "codex-agent-registry.jsonl",
    )
}

pub(super) fn agent_registry_lock_path() -> PathBuf {
    cleanup_lock_path_for(&agent_registry_log_path(), "agent.registry.cleanup.lock")
}

pub(super) fn agent_chat_log_path() -> PathBuf {
    let config_path =
        agent_config_from_env().and_then(|config| non_empty_path(&config.message_log_path));
    agent_ipc_log_path_with_config(
        config_path.as_deref(),
        "agent.chat.jsonl",
        "codex-agent-chat.jsonl",
    )
}

pub(super) fn agent_chat_log_path_for_target(target: &str) -> PathBuf {
    if target != BROADCAST_TARGET
        && !target.eq_ignore_ascii_case("all")
        && let Some(path) = agent_config_from_env()
            .and_then(|config| config.resolve_agent(target).ok())
            .and_then(|agent| non_empty_path(&agent.message_log_path))
    {
        return path;
    }

    agent_chat_log_path()
}

pub(super) fn agent_chat_lock_path() -> PathBuf {
    cleanup_lock_path_for(&agent_chat_log_path(), "agent.chat.cleanup.lock")
}

pub(super) fn receipt_ledger_path(agent_name: &str) -> PathBuf {
    let file_name = format!(
        "agent.receipts-{}.jsonl",
        normalize_agent_name(agent_name).to_ascii_lowercase()
    );
    agent_chat_log_path().with_file_name(file_name)
}

fn agent_ipc_log_path_with_config(
    config_path: Option<&Path>,
    project_file_name: &str,
    legacy_file_name: &str,
) -> PathBuf {
    let current_dir = std::env::current_dir().ok();
    agent_ipc_log_path_for(
        config_path,
        current_dir.as_deref(),
        project_file_name,
        legacy_file_name,
    )
}

pub(super) fn agent_ipc_log_path_for(
    config_path: Option<&Path>,
    current_dir: Option<&Path>,
    project_file_name: &str,
    legacy_file_name: &str,
) -> PathBuf {
    if let Some(path) = config_path {
        return path.to_path_buf();
    }
    if let Some(cwd) = current_dir {
        return project_runtime_path(cwd, project_file_name);
    }
    std::env::temp_dir().join(legacy_file_name)
}

fn project_runtime_path(project_root: &Path, file_name: &str) -> PathBuf {
    project_root.join(".codex/agent-ipc").join(file_name)
}

pub(super) fn cleanup_lock_path_for(log_path: &Path, lock_file_name: &str) -> PathBuf {
    log_path.with_file_name(lock_file_name)
}

fn split_subject_and_body(text: &str) -> (String, String) {
    let mut split_at = text.len();
    let mut chars_seen = 0usize;
    for (idx, ch) in text.char_indices() {
        if matches!(ch, '.' | '!' | '?' | '\n') {
            split_at = idx;
            break;
        }
        chars_seen += 1;
        if chars_seen == 25 {
            split_at = idx + ch.len_utf8();
            break;
        }
    }
    let subject = text[..split_at].trim_end().to_string();
    let body = text[split_at..].to_string();
    (subject, body)
}

pub(super) fn agent_mailbox_path(project_root: &Path, agent_name: &str) -> io::Result<PathBuf> {
    if let Some(roster) = agent_config_from_env() {
        return roster
            .matching_agents(agent_name)
            .into_iter()
            .find(|agent| same_project(&agent.project_root, &roster.project_root))
            .and_then(|agent| non_empty_path(&agent.mailbox_path))
            .ok_or_else(|| io::Error::other("managed agent has no configured mailbox path"));
    }

    Ok(project_root
        .join(".codex/agent-ipc")
        .join(format!("{}.mailbox.md", agent_mailbox_stem(agent_name))))
}

fn agent_mailbox_stem(agent_name: &str) -> String {
    normalize_agent_name(agent_name)
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub(super) fn append_text_block(path: &Path, block: &str) -> io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(io::Error::other("missing parent directory for mailbox"));
    };
    fs::create_dir_all(parent)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(block.as_bytes())
}

fn normalize_target(target: &str) -> String {
    if target == BROADCAST_TARGET || target.eq_ignore_ascii_case("all") {
        BROADCAST_TARGET.to_string()
    } else {
        resolve_target_agent(target).unwrap_or_else(|_| normalize_agent_name(target))
    }
}

fn target_matches(target: &str, identity: &AgentIdentity) -> bool {
    target == BROADCAST_TARGET
        || normalize_alias(target) == normalize_alias(&identity.agent_name)
        || identity
            .agent_aliases
            .iter()
            .any(|alias| normalize_alias(target) == normalize_alias(alias))
        || agent_config_from_env()
            .is_some_and(|roster| roster.target_matches_agent(target, &identity.agent_name))
}

fn is_stale(timestamp: DateTime<Utc>) -> bool {
    Utc::now().signed_duration_since(timestamp) > ChronoDuration::seconds(STALE_AFTER_SECS)
}

fn remove_file_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

struct CleanupLock {
    path: PathBuf,
}

impl CleanupLock {
    fn try_acquire(path: &Path) -> io::Result<Option<Self>> {
        match OpenOptions::new().create_new(true).write(true).open(path) {
            Ok(mut file) => {
                let _ = writeln!(file, "{}", std::process::id());
                Ok(Some(Self {
                    path: path.to_path_buf(),
                }))
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(None),
            Err(err) => Err(err),
        }
    }
}

impl Drop for CleanupLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
