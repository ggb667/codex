use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use uuid::Uuid;

mod roster;
mod storage;

use roster::AgentConfig;
use roster::AgentConfigAgent;
use roster::agent_config_from_env;
use roster::display_agent_name as fallback_display_agent_name;
use roster::normalize_agent_name;
use storage::agent_mailbox_path;
use storage::append_chat_message_at;
use storage::append_json_line;
use storage::append_registry_heartbeat_at;
use storage::append_text_block;
use storage::cleanup_lock_path_for;
use storage::git_branch_for_path;
use storage::pony_chat_lock_path;
use storage::pony_chat_log_path;
use storage::pony_chat_log_path_for_target;
use storage::pony_registry_lock_path;
use storage::pony_registry_log_path;
use storage::read_jsonl;
use storage::read_live_registry_at;
use storage::read_new_messages_at;
use storage::receipt_ledger_path;

pub(crate) const AGENT_IPC_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(6);
pub(super) const STALE_AFTER_SECS: i64 = 60 * 60;
pub(super) const BROADCAST_TARGET: &str = "*";
const UNKNOWN_BRANCH: &str = "unknown";
pub(super) const PONY_CHAT_LOG_PATH_ENV: &str = "AGENIC_PONY_CHAT_LOG_PATH";
pub(super) const PONY_REGISTRY_LOG_PATH_ENV: &str = "AGENIC_PONY_REGISTRY_LOG_PATH";
pub(super) const PROJECT_ROOT_ENV: &str = "AGENIC_PROJECT_ROOT";
pub(super) const AGENT_CONFIG_ENV: &str = "CODEX_AGENT_CONFIG";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentSendCommand {
    List,
    Send {
        target: String,
        text: String,
        delivery_class: DeliveryClass,
    },
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DeliveryClass {
    #[default]
    Ephemeral,
    Durable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AgentIdentity {
    pub(crate) instance_id: String,
    #[serde(rename = "pony_name")]
    pub(crate) agent_name: String,
    #[serde(rename = "pony_symbol")]
    pub(crate) agent_symbol: String,
    #[serde(rename = "pony_aliases")]
    pub(crate) agent_aliases: Vec<String>,
    pub(crate) mailbox_path: Option<PathBuf>,
    pub(crate) project_path: String,
    pub(crate) git_branch: String,
    pub(crate) pid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AgentRegistryEntry {
    pub(crate) uuid: String,
    #[serde(rename = "pony_name")]
    pub(crate) agent_name: String,
    pub(crate) path: String,
    pub(crate) git_branch: String,
    pub(crate) pid: u32,
    pub(crate) last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AgentMessage {
    pub(crate) id: String,
    pub(crate) from_instance_id: String,
    #[serde(rename = "from_pony_name")]
    pub(crate) from_agent_name: String,
    pub(crate) from_symbol: String,
    pub(crate) to: String,
    pub(crate) subject: String,
    pub(crate) body: String,
    pub(crate) created_at: DateTime<Utc>,
    #[serde(default)]
    pub(crate) delivery_class: DeliveryClass,
}

impl AgentMessage {
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
            display_agent_name(&self.to),
            self.subject,
            body
        )
    }

    fn display_sender(&self) -> String {
        let pony = display_agent_name(&self.from_agent_name);
        if self.from_symbol.is_empty() {
            pony
        } else {
            format!("{} {}", self.from_symbol, pony)
        }
    }
}

impl AgentIdentity {
    fn registry_entry(&self) -> AgentRegistryEntry {
        AgentRegistryEntry {
            uuid: self.instance_id.clone(),
            agent_name: self.agent_name.clone(),
            path: self.project_path.clone(),
            git_branch: self.git_branch.clone(),
            pid: self.pid,
            last_seen_at: Utc::now(),
        }
    }
}

pub(crate) fn agent_identity_from_env(cwd: &Path) -> Option<AgentIdentity> {
    let raw_name = std::env::var("AGENIC_LAUNCH_PERSONALITY")
        .ok()
        .or_else(|| std::env::var("PERSONALITY").ok())?;
    let project_path = std::env::var("AGENIC_PROJECT_ROOT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| cwd.display().to_string());
    let git_branch = std::env::var("AGENIC_PROJECT_BRANCH")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| git_branch_for_path(Path::new(&project_path)));
    let roster = agent_config_from_env();
    let agent = roster
        .as_ref()
        .and_then(|roster| roster.current_agent(&raw_name, &project_path));
    let agent_name = agent
        .as_ref()
        .map(AgentConfigAgent::route)
        .unwrap_or_else(|| normalize_agent_name(&raw_name));
    let agent_symbol = agent
        .as_ref()
        .map(|agent| agent.icon.clone())
        .unwrap_or_default();
    let agent_aliases = agent
        .as_ref()
        .map(AgentConfigAgent::match_names)
        .unwrap_or_else(|| vec![agent_name.clone(), raw_name]);
    let mailbox_path = agent
        .as_ref()
        .and_then(|agent| non_empty_path(&agent.mailbox_path));

    Some(AgentIdentity {
        instance_id: Uuid::new_v4().to_string(),
        agent_name,
        agent_symbol,
        agent_aliases,
        mailbox_path,
        project_path,
        git_branch,
        pid: std::process::id(),
    })
}

pub(crate) fn parse_send_command(args: &str) -> Result<AgentSendCommand, String> {
    let roster = agent_config_from_env();
    parse_send_command_with_roster(args, roster.as_ref())
}

fn parse_send_command_with_roster(
    args: &str,
    roster: Option<&AgentConfig>,
) -> Result<AgentSendCommand, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Err(agent_usage().to_string());
    }
    if trimmed.eq_ignore_ascii_case("list") {
        return Ok(AgentSendCommand::List);
    }

    let Some((target, text)) = trimmed.split_once(char::is_whitespace) else {
        return Err(agent_usage().to_string());
    };
    let text = text.trim();
    if text.is_empty() {
        return Err(agent_usage().to_string());
    }

    let (delivery_class, target, text) = if target.eq_ignore_ascii_case("durable") {
        let Some((target, text)) = text.split_once(char::is_whitespace) else {
            return Err(agent_usage().to_string());
        };
        let text = text.trim();
        if text.is_empty() {
            return Err(agent_usage().to_string());
        }
        (DeliveryClass::Durable, target, text.to_string())
    } else {
        (DeliveryClass::Ephemeral, target, text.to_string())
    };

    let target = if target.eq_ignore_ascii_case("all") {
        BROADCAST_TARGET.to_string()
    } else {
        resolve_target_agent_with_roster(target, roster)?
    };

    Ok(AgentSendCommand::Send {
        target,
        text,
        delivery_class,
    })
}

pub(crate) fn append_registry_heartbeat(identity: &AgentIdentity) -> io::Result<()> {
    let registry_path = pony_registry_log_path();
    let lock_path = pony_registry_lock_path();
    append_registry_heartbeat_at(&registry_path, &lock_path, identity)
}

pub(crate) fn append_chat_message(
    identity: &AgentIdentity,
    target: &str,
    text: &str,
    delivery_class: DeliveryClass,
) -> io::Result<AgentMessage> {
    let chat_path = if delivery_class == DeliveryClass::Durable {
        pony_chat_log_path_for_target(target)
    } else {
        pony_chat_log_path()
    };
    let lock_path = cleanup_lock_path_for(&chat_path, "pony.chat.cleanup.lock");
    append_chat_message_at(
        &chat_path,
        &lock_path,
        identity,
        target,
        text,
        delivery_class,
    )
}

pub(crate) fn read_live_registry() -> io::Result<Vec<AgentRegistryEntry>> {
    let registry_path = pony_registry_log_path();
    let lock_path = pony_registry_lock_path();
    read_live_registry_at(&registry_path, &lock_path)
}

pub(crate) fn read_new_messages(identity: &AgentIdentity) -> io::Result<Vec<AgentMessage>> {
    let chat_path = pony_chat_log_path();
    let lock_path = pony_chat_lock_path();
    read_new_messages_at(&chat_path, &lock_path, identity)
}

pub(crate) fn receipt_recorded(identity: &AgentIdentity, id: &str) -> io::Result<bool> {
    Ok(
        read_jsonl::<String>(&receipt_ledger_path(&identity.agent_name))?
            .iter()
            .any(|seen| seen == id),
    )
}

pub(crate) fn record_receipt(identity: &AgentIdentity, id: &str) -> io::Result<()> {
    append_json_line(&receipt_ledger_path(&identity.agent_name), &id.to_string())
}

pub(crate) fn append_incoming_message_to_mailbox(
    identity: &AgentIdentity,
    message: &AgentMessage,
) -> io::Result<()> {
    let project_root = Path::new(&identity.project_path);
    let mailbox_path = identity.mailbox_path.clone().map_or_else(
        || agent_mailbox_path(project_root, &identity.agent_name),
        Ok,
    )?;
    append_text_block(&mailbox_path, &message.mailbox_markdown())
}

pub(crate) fn display_agent_name(name: &str) -> String {
    agent_config_from_env()
        .and_then(|roster| roster.resolve_display_name(name))
        .unwrap_or_else(|| fallback_display_agent_name(name))
}

pub(crate) fn canonicalize_agent_name(name: &str) -> String {
    agent_config_from_env()
        .and_then(|roster| roster.resolve_route(name).ok())
        .unwrap_or_else(|| normalize_agent_name(name))
}

fn resolve_target_agent(name: &str) -> Result<String, String> {
    let roster = agent_config_from_env();
    resolve_target_agent_with_roster(name, roster.as_ref())
}

fn resolve_target_agent_with_roster(
    name: &str,
    roster: Option<&AgentConfig>,
) -> Result<String, String> {
    if let Some(roster) = roster {
        roster.resolve_route(name)
    } else {
        Ok(normalize_agent_name(name))
    }
}

fn agent_usage() -> &'static str {
    "Usage: /tell list | /tell [durable] <agent-name|all> <message>"
}

fn non_empty_path(value: &str) -> Option<PathBuf> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
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

fn same_project(left: &str, right: &str) -> bool {
    roster::normalize_alias(left) == roster::normalize_alias(right)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
