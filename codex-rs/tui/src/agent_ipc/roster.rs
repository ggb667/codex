use serde::Deserialize;
use std::collections::HashMap;
use std::fs;

use super::AGENT_CONFIG_ENV;
use super::non_empty_path;
use super::pony_usage;
use super::same_project;
use super::title_case_word;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentConfig {
    pub(super) agent_id: String,
    pub(super) route_id: String,
    pub(super) label: String,
    pub(super) icon: String,
    #[serde(default)]
    pub(super) aliases: Vec<String>,
    pub(super) project_root: String,
    #[serde(default)]
    pub(super) mailbox_path: String,
    #[serde(default)]
    pub(super) message_log_path: String,
    #[serde(default)]
    pub(super) registry_path: String,
    #[serde(default)]
    pub(super) global_singleton: bool,
    #[serde(default)]
    pub(super) agents: Vec<AgentConfigAgent>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentConfigAgent {
    pub(super) agent_id: String,
    pub(super) route_id: String,
    pub(super) label: String,
    pub(super) icon: String,
    #[serde(default)]
    pub(super) aliases: Vec<String>,
    pub(super) project_root: String,
    #[serde(default)]
    pub(super) mailbox_path: String,
    #[serde(default)]
    pub(super) message_log_path: String,
    #[serde(default)]
    pub(super) registry_path: String,
    #[serde(default)]
    pub(super) global_singleton: bool,
}

impl AgentConfig {
    pub(super) fn current_agent(
        &self,
        raw_name: &str,
        project_path: &str,
    ) -> Option<AgentConfigAgent> {
        let normalized_project = normalize_alias(project_path);
        self.candidates().into_iter().find(|agent| {
            agent.matches(raw_name) && normalize_alias(&agent.project_root) == normalized_project
        })
    }

    pub(super) fn resolve_route(&self, name: &str) -> Result<String, String> {
        self.resolve_agent(name).map(|agent| agent.route())
    }

    pub(super) fn resolve_agent(&self, name: &str) -> Result<AgentConfigAgent, String> {
        let raw = name.trim();
        let matches = self.matching_agents(raw);
        if matches.is_empty() {
            return Err(format!("Unknown pony '{}'. {}", raw, pony_usage()));
        }

        let unique = unique_agents_by_route(matches);
        if raw.contains(':') {
            return unique_agent_or_ambiguous(raw, unique);
        }

        let local = unique
            .iter()
            .filter(|agent| same_project(&agent.project_root, &self.project_root))
            .cloned()
            .collect::<Vec<_>>();
        if !local.is_empty() {
            return unique_agent_or_ambiguous(raw, local);
        }

        if unique.len() == 1 && unique[0].is_global_singleton() {
            return Ok(unique[0].clone());
        }

        Err(format!(
            "Ambiguous pony '{raw}'. Use a disambiguated alias such as <project>:<name>."
        ))
    }

    pub(super) fn resolve_display_name(&self, name: &str) -> Option<String> {
        let matches = self.matching_agents(name);
        let unique = unique_agents_by_route(matches);
        unique
            .iter()
            .find(|agent| same_project(&agent.project_root, &self.project_root))
            .or_else(|| unique.iter().find(|agent| agent.is_global_singleton()))
            .or_else(|| unique.first())
            .map(|agent| agent.label.clone())
    }

    pub(super) fn target_matches_agent(&self, target: &str, agent_name: &str) -> bool {
        let Ok(target_route) = self.resolve_route(target) else {
            return false;
        };
        let matches = self.matching_agents(agent_name);
        unique_agents_by_route(matches)
            .iter()
            .any(|agent| normalize_alias(&agent.route()) == normalize_alias(&target_route))
    }

    pub(super) fn matching_agents(&self, name: &str) -> Vec<AgentConfigAgent> {
        self.candidates()
            .into_iter()
            .filter(|agent| agent.matches(name))
            .collect()
    }

    fn candidates(&self) -> Vec<AgentConfigAgent> {
        let mut agents = Vec::with_capacity(self.agents.len() + 1);
        agents.push(AgentConfigAgent {
            agent_id: self.agent_id.clone(),
            route_id: self.route_id.clone(),
            label: self.label.clone(),
            icon: self.icon.clone(),
            aliases: self.aliases.clone(),
            project_root: self.project_root.clone(),
            mailbox_path: self.mailbox_path.clone(),
            message_log_path: self.message_log_path.clone(),
            registry_path: self.registry_path.clone(),
            global_singleton: self.global_singleton,
        });
        agents.extend(self.agents.clone());
        agents
    }
}

impl AgentConfigAgent {
    pub(super) fn route(&self) -> String {
        if self.route_id.trim().is_empty() {
            self.agent_id.clone()
        } else {
            self.route_id.clone()
        }
    }

    fn matches(&self, name: &str) -> bool {
        let needle = normalize_alias(name);
        self.match_names()
            .iter()
            .any(|alias| normalize_alias(alias) == needle)
    }

    pub(super) fn match_names(&self) -> Vec<String> {
        let mut names = vec![self.agent_id.clone(), self.route()];
        names.extend(self.aliases.clone());
        names
    }

    fn is_global_singleton(&self) -> bool {
        self.global_singleton
            || self.agent_id == "PRINCESS_CELESTIA_SOL_INVICTUS"
            || self.route_id == "PRINCESS_CELESTIA_SOL_INVICTUS"
    }
}

pub(super) fn agent_config_from_env() -> Option<AgentConfig> {
    let path = std::env::var(AGENT_CONFIG_ENV).ok()?;
    let path = non_empty_path(&path)?;
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub(super) fn normalize_alias(name: &str) -> String {
    name.trim().to_lowercase()
}

pub(super) fn normalize_agent_name(name: &str) -> String {
    name.trim().to_ascii_uppercase().replace([' ', '-'], "_")
}

pub(super) fn display_agent_name(name: &str) -> String {
    let name = name.rsplit_once(':').map_or(name, |(_, name)| name);
    normalize_agent_name(name)
        .split('_')
        .filter(|part| !part.is_empty())
        .map(title_case_word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn unique_agent_or_ambiguous(
    raw: &str,
    agents: Vec<AgentConfigAgent>,
) -> Result<AgentConfigAgent, String> {
    if agents.len() == 1 {
        Ok(agents[0].clone())
    } else {
        Err(format!(
            "Ambiguous pony '{raw}'. Use a disambiguated alias such as <project>:<name>."
        ))
    }
}

fn unique_agents_by_route(agents: Vec<AgentConfigAgent>) -> Vec<AgentConfigAgent> {
    let mut unique = HashMap::new();
    for agent in agents {
        unique.entry(agent.route()).or_insert(agent);
    }
    unique.into_values().collect()
}
