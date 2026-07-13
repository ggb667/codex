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

pub(crate) const PONY_IPC_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(6);
const STALE_AFTER_SECS: i64 = 60 * 60;
const BROADCAST_TARGET: &str = "*";
const UNKNOWN_BRANCH: &str = "unknown";
const PONY_CHAT_LOG_PATH_ENV: &str = "AGENIC_PONY_CHAT_LOG_PATH";
const PONY_REGISTRY_LOG_PATH_ENV: &str = "AGENIC_PONY_REGISTRY_LOG_PATH";
const PROJECT_ROOT_ENV: &str = "AGENIC_PROJECT_ROOT";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PonySendCommand {
    List,
    Send { target: String, text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PonyIdentity {
    pub(crate) instance_id: String,
    pub(crate) pony_name: String,
    pub(crate) project_path: String,
    pub(crate) git_branch: String,
    pub(crate) pid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PonyRegistryEntry {
    pub(crate) uuid: String,
    pub(crate) pony_name: String,
    pub(crate) path: String,
    pub(crate) git_branch: String,
    pub(crate) pid: u32,
    pub(crate) last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PonyChatEntry {
    pub(crate) id: String,
    pub(crate) from_instance_id: String,
    pub(crate) from_pony_name: String,
    pub(crate) from_symbol: String,
    pub(crate) to: String,
    pub(crate) subject: String,
    pub(crate) body: String,
    pub(crate) created_at: DateTime<Utc>,
}

impl PonyChatEntry {
    pub(crate) fn prompt_text(&self) -> String {
        let sender = self.display_sender();
        if self.body.is_empty() {
            format!("{sender} letter\nSubject: {}", self.subject)
        } else {
            format!(
                "{sender} letter\nSubject: {}\nBody:\n{}",
                self.subject, self.body
            )
        }
    }

    pub(crate) fn mailbox_markdown(&self) -> String {
        let body = if self.body.is_empty() {
            "_empty_".to_string()
        } else {
            self.body.clone()
        };
        format!(
            "## {}\n- FROM: {}\n- TO: {}\n- SUBJECT: {}\n- BODY:\n```text\n{}\n```\n\n",
            self.created_at.to_rfc3339(),
            self.display_sender(),
            display_pony_name(&self.to),
            self.subject,
            body
        )
    }

    fn display_sender(&self) -> String {
        let pony = display_pony_name(&self.from_pony_name);
        if self.from_symbol.is_empty() {
            pony
        } else {
            format!("{} {}", self.from_symbol, pony)
        }
    }
}

impl PonyIdentity {
    fn registry_entry(&self) -> PonyRegistryEntry {
        PonyRegistryEntry {
            uuid: self.instance_id.clone(),
            pony_name: self.pony_name.clone(),
            path: self.project_path.clone(),
            git_branch: self.git_branch.clone(),
            pid: self.pid,
            last_seen_at: Utc::now(),
        }
    }
}

pub(crate) fn pony_identity_from_env(cwd: &Path) -> Option<PonyIdentity> {
    let raw_name = std::env::var("AGENIC_LAUNCH_PERSONALITY")
        .ok()
        .or_else(|| std::env::var("PERSONALITY").ok())?;
    let pony_name = canonicalize_pony_name(&raw_name);
    let project_path = std::env::var("AGENIC_PROJECT_ROOT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| cwd.display().to_string());
    let git_branch = std::env::var("AGENIC_PROJECT_BRANCH")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| git_branch_for_path(Path::new(&project_path)));

    Some(PonyIdentity {
        instance_id: Uuid::new_v4().to_string(),
        pony_name,
        project_path,
        git_branch,
        pid: std::process::id(),
    })
}

pub(crate) fn parse_send_command(args: &str) -> Result<PonySendCommand, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Err(pony_usage().to_string());
    }
    if trimmed.eq_ignore_ascii_case("list") {
        return Ok(PonySendCommand::List);
    }

    let Some((target, text)) = trimmed.split_once(char::is_whitespace) else {
        return Err(pony_usage().to_string());
    };
    let text = text.trim();
    if text.is_empty() {
        return Err(pony_usage().to_string());
    }

    let target = if target.eq_ignore_ascii_case("all") {
        BROADCAST_TARGET.to_string()
    } else {
        canonicalize_target_pony_name(target)?
    };

    Ok(PonySendCommand::Send {
        target,
        text: text.to_string(),
    })
}

pub(crate) fn append_registry_heartbeat(identity: &PonyIdentity) -> io::Result<()> {
    let registry_path = pony_registry_log_path();
    let lock_path = pony_registry_lock_path();
    append_registry_heartbeat_at(&registry_path, &lock_path, identity)
}

pub(crate) fn append_chat_message(
    identity: &PonyIdentity,
    target: &str,
    text: &str,
) -> io::Result<PonyChatEntry> {
    let chat_path = pony_chat_log_path();
    let lock_path = pony_chat_lock_path();
    append_chat_message_at(&chat_path, &lock_path, identity, target, text)
}

pub(crate) fn read_live_registry() -> io::Result<Vec<PonyRegistryEntry>> {
    let registry_path = pony_registry_log_path();
    let lock_path = pony_registry_lock_path();
    read_live_registry_at(&registry_path, &lock_path)
}

pub(crate) fn read_new_messages(identity: &PonyIdentity) -> io::Result<Vec<PonyChatEntry>> {
    let chat_path = pony_chat_log_path();
    let lock_path = pony_chat_lock_path();
    read_new_messages_at(&chat_path, &lock_path, identity)
}

pub(crate) fn append_incoming_message_to_mailbox(
    identity: &PonyIdentity,
    message: &PonyChatEntry,
) -> io::Result<()> {
    let project_root = Path::new(&identity.project_path);
    let mailbox_path = pony_mailbox_path(project_root, &identity.pony_name);
    append_text_block(&mailbox_path, &message.mailbox_markdown())
}

pub(crate) fn display_pony_name(name: &str) -> String {
    canonicalize_pony_name(name)
        .split('_')
        .filter(|part| !part.is_empty())
        .map(title_case_word)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn canonicalize_pony_name(name: &str) -> String {
    let normalized = name.trim().to_ascii_uppercase().replace([' ', '-'], "_");
    match normalized.as_str() {
        "TIA"
        | "CELESTIA"
        | "PRINCESS"
        | "CELLY"
        | "SUNBUTT"
        | "PRINCESS_CELESTIA_SOL_INVICTUS" => "PRINCESS_CELESTIA_SOL_INVICTUS".to_string(),
        "TWI" | "TWILIGHT" | "TWILIGHT_SPARKLE" => "TWILIGHT_SPARKLE".to_string(),
        "AJ" | "APPLEJACK" => "APPLEJACK".to_string(),
        "FS" | "FLUTTERSHY" | "SHY" | "FLUTTERS" => "FLUTTERSHY".to_string(),
        "PINKIE" | "PINKIE_PIE" => "PINKIE_PIE".to_string(),
        "RARITY" | "RARES" => "RARITY".to_string(),
        "RD" | "RAINBOW" | "RAINBOW_DASH" | "DASH" => "RAINBOW_DASH".to_string(),
        "SPIKE" => "SPIKE".to_string(),
        other => other.to_string(),
    }
}

fn canonicalize_target_pony_name(name: &str) -> Result<String, String> {
    let canonical = canonicalize_pony_name(name);
    match canonical.as_str() {
        "PRINCESS_CELESTIA_SOL_INVICTUS"
        | "TWILIGHT_SPARKLE"
        | "APPLEJACK"
        | "FLUTTERSHY"
        | "PINKIE_PIE"
        | "RARITY"
        | "RAINBOW_DASH"
        | "SPIKE" => Ok(canonical),
        _ => Err(format!(
            "Unknown pony '{}'. Usage: /tell list | /tell <pony-name|all> <message>",
            name.trim()
        )),
    }
}

fn pony_usage() -> &'static str {
    "Usage: /tell list | /tell <pony-name|all> <message>"
}

fn append_registry_heartbeat_at(
    registry_path: &Path,
    lock_path: &Path,
    identity: &PonyIdentity,
) -> io::Result<()> {
    maybe_reset_stale_registry_log(registry_path, lock_path)?;
    append_json_line(registry_path, &identity.registry_entry())
}

fn append_chat_message_at(
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
        from_symbol: pony_symbol_for_name(&identity.pony_name)
            .unwrap_or("")
            .to_string(),
        to: normalize_target(target),
        subject,
        body,
        created_at: Utc::now(),
    };
    append_json_line(chat_path, &entry)?;
    Ok(entry)
}

fn read_live_registry_at(
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

fn read_new_messages_at(
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
        if !target_matches(&entry.to, &identity.pony_name) {
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
    messages.sort_by(|left, right| left.created_at.cmp(&right.created_at));
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

fn append_json_line<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
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

fn read_jsonl<T>(path: &Path) -> io::Result<Vec<T>>
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

fn git_branch_for_path(path: &Path) -> String {
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

fn pony_registry_log_path() -> PathBuf {
    pony_ipc_log_path(
        PONY_REGISTRY_LOG_PATH_ENV,
        "pony.registry.jsonl",
        "codex-pony-registry.jsonl",
    )
}

fn pony_registry_lock_path() -> PathBuf {
    cleanup_lock_path_for(&pony_registry_log_path(), "pony.registry.cleanup.lock")
}

fn pony_chat_log_path() -> PathBuf {
    pony_ipc_log_path(
        PONY_CHAT_LOG_PATH_ENV,
        "pony.chat.jsonl",
        "codex-pony-chat.jsonl",
    )
}

fn pony_chat_lock_path() -> PathBuf {
    cleanup_lock_path_for(&pony_chat_log_path(), "pony.chat.cleanup.lock")
}

fn pony_ipc_log_path(env_name: &str, project_file_name: &str, legacy_file_name: &str) -> PathBuf {
    let explicit_path = std::env::var(env_name).ok();
    let project_root = std::env::var(PROJECT_ROOT_ENV).ok();
    let current_dir = std::env::current_dir().ok();
    pony_ipc_log_path_for(
        explicit_path.as_deref(),
        project_root.as_deref(),
        current_dir.as_deref(),
        project_file_name,
        legacy_file_name,
    )
}

fn pony_ipc_log_path_for(
    explicit_path: Option<&str>,
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

    if let Some(root) = project_root {
        return project_runtime_path(&root, project_file_name);
    }

    if let Some(cwd) = current_dir {
        return project_runtime_path(cwd, project_file_name);
    }

    std::env::temp_dir().join(legacy_file_name)
}

fn non_empty_path(value: &str) -> Option<PathBuf> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

fn project_runtime_path(project_root: &Path, file_name: &str) -> PathBuf {
    project_root.join("pony/runtime").join(file_name)
}

fn cleanup_lock_path_for(log_path: &Path, lock_file_name: &str) -> PathBuf {
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

fn pony_symbol_for_name(name: &str) -> Option<&'static str> {
    match canonicalize_pony_name(name).as_str() {
        "PRINCESS_CELESTIA_SOL_INVICTUS" => Some("☀︎"),
        "TWILIGHT_SPARKLE" => Some("✶"),
        "APPLEJACK" => Some("🍎"),
        "FLUTTERSHY" => Some("🦋"),
        "PINKIE_PIE" => Some("🎈"),
        "RARITY" => Some("💎"),
        "RAINBOW_DASH" => Some("⚡"),
        "SPIKE" => Some("🐲"),
        _ => None,
    }
}

fn pony_mailbox_path(project_root: &Path, pony_name: &str) -> PathBuf {
    project_root
        .join("pony/team.coordination")
        .join(format!("{}.mailbox.md", pony_mailbox_stem(pony_name)))
}

fn pony_mailbox_stem(pony_name: &str) -> &'static str {
    match canonicalize_pony_name(pony_name).as_str() {
        "PRINCESS_CELESTIA_SOL_INVICTUS" => "celestia",
        "TWILIGHT_SPARKLE" => "twi",
        "APPLEJACK" => "aj",
        "FLUTTERSHY" => "fs",
        "PINKIE_PIE" => "pinkie",
        "RARITY" => "rarity",
        "RAINBOW_DASH" => "rd",
        "SPIKE" => "spike",
        _ => "unknown",
    }
}

fn append_text_block(path: &Path, block: &str) -> io::Result<()> {
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
        canonicalize_pony_name(target)
    }
}

fn target_matches(target: &str, pony_name: &str) -> bool {
    target == BROADCAST_TARGET
        || canonicalize_pony_name(target) == canonicalize_pony_name(pony_name)
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

fn title_case_word(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut rendered = first.to_uppercase().collect::<String>();
    rendered.push_str(&chars.as_str().to_ascii_lowercase());
    rendered
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_identity() -> PonyIdentity {
        PonyIdentity {
            instance_id: "uuid-1".to_string(),
            pony_name: "TWILIGHT_SPARKLE".to_string(),
            project_path: "/tmp/project".to_string(),
            git_branch: "pony/twi/main".to_string(),
            pid: 42,
        }
    }

    #[test]
    fn parse_send_command_supports_list_direct_and_broadcast() {
        assert_eq!(parse_send_command("list").unwrap(), PonySendCommand::List);
        assert_eq!(
            parse_send_command("pinkie hello there").unwrap(),
            PonySendCommand::Send {
                target: "PINKIE_PIE".to_string(),
                text: "hello there".to_string(),
            }
        );
        assert_eq!(
            parse_send_command("all status check").unwrap(),
            PonySendCommand::Send {
                target: "*".to_string(),
                text: "status check".to_string(),
            }
        );
        assert_eq!(
            parse_send_command("princess status report").unwrap(),
            PonySendCommand::Send {
                target: "PRINCESS_CELESTIA_SOL_INVICTUS".to_string(),
                text: "status report".to_string(),
            }
        );
        assert_eq!(
            parse_send_command("rd clear the sky").unwrap(),
            PonySendCommand::Send {
                target: "RAINBOW_DASH".to_string(),
                text: "clear the sky".to_string(),
            }
        );
    }

    #[test]
    fn parse_send_command_rejects_unknown_target() {
        let err = parse_send_command("discord status check").unwrap_err();
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
                /*project_root*/ None,
                Some(Path::new("/tmp/cwd")),
                "pony.chat.jsonl",
                "codex-pony-chat.jsonl",
            ),
            PathBuf::from("/tmp/cwd/pony/runtime/pony.chat.jsonl")
        );
    }
}
