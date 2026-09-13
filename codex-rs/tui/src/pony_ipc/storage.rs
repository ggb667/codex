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
use std::process::Command;
use uuid::Uuid;

use super::BROADCAST_TARGET;
use super::PONY_CHAT_LOG_PATH_ENV;
use super::PONY_REGISTRY_LOG_PATH_ENV;
use super::PROJECT_ROOT_ENV;
use super::PonyChatEntry;
use super::PonyIdentity;
use super::PonyRegistryEntry;
use super::STALE_AFTER_SECS;
use super::UNKNOWN_BRANCH;
use super::agent_config_from_env;
use super::canonicalize_pony_name;
use super::non_empty_path;
use super::resolve_target_agent;
use super::roster::normalize_agent_name;
use super::roster::normalize_alias;
use super::same_project;

pub(super) fn append_registry_heartbeat_at(
    registry_path: &Path,
    lock_path: &Path,
    identity: &PonyIdentity,
) -> io::Result<()> {
    maybe_reset_stale_registry_log(registry_path, lock_path)?;
    append_json_line(registry_path, &identity.registry_entry())
}

pub(super) fn append_chat_message_at(
    chat_path: &Path,
    lock_path: &Path,
    identity: &PonyIdentity,
    target: &str,
    text: &str,
) -> io::Result<PonyChatEntry> {
    maybe_reset_stale_chat_log(chat_path, lock_path)?;
    let trimmed = text.trim();
    let (subject, body) = split_subject_and_body(trimmed);
    let entry = PonyChatEntry {
        id: Uuid::new_v4().to_string(),
        from_instance_id: identity.instance_id.clone(),
        from_pony_name: identity.pony_name.clone(),
        from_symbol: identity.pony_symbol.clone(),
        to: normalize_target(target),
        subject,
        body,
        created_at: Utc::now(),
    };
    append_json_line(chat_path, &entry)?;
    Ok(entry)
}

pub(super) fn read_live_registry_at(
    registry_path: &Path,
    lock_path: &Path,
) -> io::Result<Vec<PonyRegistryEntry>> {
    maybe_reset_stale_registry_log(registry_path, lock_path)?;
    let mut latest_by_uuid = HashMap::new();
    for entry in read_jsonl::<PonyRegistryEntry>(registry_path)? {
        if is_stale(entry.last_seen_at) {
            continue;
        }
        latest_by_uuid.insert(entry.uuid.clone(), entry);
    }
    let mut entries = latest_by_uuid.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.pony_name
            .cmp(&right.pony_name)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(entries)
}

pub(super) fn read_new_messages_at(
    chat_path: &Path,
    lock_path: &Path,
    identity: &PonyIdentity,
) -> io::Result<Vec<PonyChatEntry>> {
    maybe_reset_stale_chat_log(chat_path, lock_path)?;
    let mut latest_by_sender: HashMap<String, PonyChatEntry> = HashMap::new();
    for entry in read_jsonl::<PonyChatEntry>(chat_path)? {
        if is_stale(entry.created_at) {
            continue;
        }
        if entry.from_instance_id == identity.instance_id {
            continue;
        }
        if !target_matches(&entry.to, identity) {
            continue;
        }
        let sender = canonicalize_pony_name(&entry.from_pony_name);
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
    Ok(read_jsonl::<PonyRegistryEntry>(path)?
        .into_iter()
        .map(|entry| entry.last_seen_at)
        .max())
}

fn latest_chat_timestamp(path: &Path) -> io::Result<Option<DateTime<Utc>>> {
    Ok(read_jsonl::<PonyChatEntry>(path)?
        .into_iter()
        .map(|entry| entry.created_at)
        .max())
}

pub(super) fn append_json_line<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(io::Error::other(
            "missing parent directory for pony IPC log",
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

pub(super) fn git_branch_for_path(path: &Path) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD")
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if branch.is_empty() {
                UNKNOWN_BRANCH.to_string()
            } else {
                branch
            }
        }
        _ => UNKNOWN_BRANCH.to_string(),
    }
}

pub(super) fn pony_registry_log_path() -> PathBuf {
    let config_path =
        agent_config_from_env().and_then(|config| non_empty_path(&config.registry_path));
    pony_ipc_log_path_with_config(
        PONY_REGISTRY_LOG_PATH_ENV,
        config_path.as_deref(),
        "pony.registry.jsonl",
        "codex-pony-registry.jsonl",
    )
}

pub(super) fn pony_registry_lock_path() -> PathBuf {
    cleanup_lock_path_for(&pony_registry_log_path(), "pony.registry.cleanup.lock")
}

pub(super) fn pony_chat_log_path() -> PathBuf {
    let config_path =
        agent_config_from_env().and_then(|config| non_empty_path(&config.message_log_path));
    pony_ipc_log_path_with_config(
        PONY_CHAT_LOG_PATH_ENV,
        config_path.as_deref(),
        "pony.chat.jsonl",
        "codex-pony-chat.jsonl",
    )
}

pub(super) fn pony_chat_log_path_for_target(target: &str) -> PathBuf {
    if target != BROADCAST_TARGET
        && !target.eq_ignore_ascii_case("all")
        && let Some(path) = agent_config_from_env()
            .and_then(|config| config.resolve_agent(target).ok())
            .and_then(|agent| non_empty_path(&agent.message_log_path))
    {
        return path;
    }

    pony_chat_log_path()
}

pub(super) fn pony_chat_lock_path() -> PathBuf {
    cleanup_lock_path_for(&pony_chat_log_path(), "pony.chat.cleanup.lock")
}

fn pony_ipc_log_path_with_config(
    env_name: &str,
    config_path: Option<&Path>,
    project_file_name: &str,
    legacy_file_name: &str,
) -> PathBuf {
    let explicit_path = std::env::var(env_name).ok();
    let project_root = std::env::var(PROJECT_ROOT_ENV).ok();
    let current_dir = std::env::current_dir().ok();
    pony_ipc_log_path_for(
        explicit_path.as_deref(),
        config_path,
        project_root.as_deref(),
        current_dir.as_deref(),
        project_file_name,
        legacy_file_name,
    )
}

pub(super) fn pony_ipc_log_path_for(
    explicit_path: Option<&str>,
    config_path: Option<&Path>,
    project_root: Option<&str>,
    current_dir: Option<&Path>,
    project_file_name: &str,
    legacy_file_name: &str,
) -> PathBuf {
    let project_root = project_root.and_then(non_empty_path);
    if let Some(path) = explicit_path.and_then(non_empty_path)
        && project_root
            .as_ref()
            .is_none_or(|root| path.starts_with(root))
    {
        return path;
    }

    if let Some(path) = config_path
        && project_root
            .as_ref()
            .is_none_or(|root| path.starts_with(root))
    {
        return path.to_path_buf();
    }

    if let Some(root) = project_root {
        return project_runtime_path(&root, project_file_name);
    }

    if let Some(cwd) = current_dir {
        return project_runtime_path(cwd, project_file_name);
    }

    std::env::temp_dir().join(legacy_file_name)
}

fn project_runtime_path(project_root: &Path, file_name: &str) -> PathBuf {
    project_root.join("pony/runtime").join(file_name)
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

pub(super) fn pony_mailbox_path(project_root: &Path, pony_name: &str) -> PathBuf {
    if let Some(roster) = agent_config_from_env()
        && let Some(path) = roster
            .matching_agents(pony_name)
            .into_iter()
            .find(|agent| same_project(&agent.project_root, &roster.project_root))
            .and_then(|agent| non_empty_path(&agent.mailbox_path))
    {
        return path;
    }

    project_root
        .join("pony/team.coordination")
        .join(format!("{}.mailbox.md", pony_mailbox_stem(pony_name)))
}

fn pony_mailbox_stem(pony_name: &str) -> String {
    normalize_agent_name(pony_name)
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

fn target_matches(target: &str, identity: &PonyIdentity) -> bool {
    target == BROADCAST_TARGET
        || normalize_alias(target) == normalize_alias(&identity.pony_name)
        || identity
            .pony_aliases
            .iter()
            .any(|alias| normalize_alias(target) == normalize_alias(alias))
        || agent_config_from_env()
            .is_some_and(|roster| roster.target_matches_agent(target, &identity.pony_name))
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
