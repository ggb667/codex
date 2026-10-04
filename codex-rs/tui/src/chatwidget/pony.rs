use super::*;
use crate::agent_ipc;

fn idle_drain_error_message(err: &std::io::Error) -> String {
    format!("Idle deferred because inbound agent messages could not be checked: {err}")
}

impl ChatWidget {
    pub(super) fn maybe_start_agent_ipc(&mut self) {
        let Some(identity) = agent_ipc::agent_identity_from_env(self.config.cwd.as_ref()) else {
            return;
        };
        if let Err(err) = agent_ipc::read_startup_state(&identity) {
            tracing::warn!(error = %err, "failed to read canonical agent startup state");
        }
        if let Err(err) = agent_ipc::append_registry_heartbeat(&identity) {
            tracing::debug!(error = %err, "failed to write initial pony IPC heartbeat");
        }
        self.agent_ipc_task = Some(Self::spawn_agent_ipc_task(
            self.app_event_tx.clone(),
            identity.clone(),
        ));
        self.agent_ipc_identity = Some(identity);
        self.agent_ipc_drain = Some(agent_ipc::InboundMessageDrain::default());
    }

    fn spawn_agent_ipc_task(
        app_event_tx: AppEventSender,
        identity: AgentIdentity,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                if let Err(err) = agent_ipc::append_registry_heartbeat(&identity) {
                    tracing::debug!(error = %err, "failed to refresh pony IPC heartbeat");
                }
                app_event_tx.send(AppEvent::DrainAgentMessages);
                tokio::time::sleep(agent_ipc::AGENT_IPC_POLL_INTERVAL).await;
            }
        })
    }

    pub(crate) fn drain_agent_messages(&mut self) -> std::io::Result<agent_ipc::DrainReport> {
        let Some(identity) = self.agent_ipc_identity.clone() else {
            return Ok(agent_ipc::DrainReport::default());
        };
        let Some(mut drain) = self.agent_ipc_drain.take() else {
            return Ok(agent_ipc::DrainReport::default());
        };
        let result = agent_ipc::drain_new_messages(
            &mut drain,
            &identity,
            |message| agent_ipc::append_incoming_message_to_mailbox(&identity, message),
            |message| self.queue_or_buffer_agent_message(message),
        );
        self.agent_ipc_drain = Some(drain);
        result
    }

    pub(super) fn drain_agent_messages_before_idle(&mut self, message: &mut String) {
        if !agent_ipc::requests_idle_transition(message) {
            return;
        }
        match self.drain_agent_messages() {
            Ok(report) if report.delivered > 0 => {
                agent_ipc::suppress_idle_transition(message);
                self.agent_ipc_idle_deferred = true;
            }
            Ok(_) => {}
            Err(err) => {
                agent_ipc::suppress_idle_transition(message);
                self.agent_ipc_idle_deferred = true;
                self.add_error_message(idle_drain_error_message(&err));
            }
        }
    }

    pub(crate) fn queue_or_buffer_agent_message(&mut self, message: AgentMessage) {
        self.pending_agent_messages.push_back(message);
        self.try_deliver_pending_agent_messages();
    }

    pub(crate) fn try_deliver_pending_agent_messages(&mut self) {
        while let Some(message) = self.pending_agent_messages.pop_front() {
            self.submit_user_message(message.prompt_text().into());
        }
    }

    pub(crate) fn handle_agent_send(
        &mut self,
        target: String,
        text: String,
        delivery_class: agent_ipc::DeliveryClass,
    ) {
        let Some(identity) = self.agent_ipc_identity.as_ref() else {
            self.add_error_message(
                "Pony IPC is unavailable because this Codex session has no pony identity."
                    .to_string(),
            );
            return;
        };
        match agent_ipc::append_chat_message(identity, &target, &text, delivery_class) {
            Ok(_entry) => {
                let recipient = if target == "*" {
                    "all ponies".to_string()
                } else {
                    agent_ipc::display_agent_name(&target)
                };
                self.add_info_message(format!("Sent pony message to {recipient}."), Some(text));
            }
            Err(err) => {
                self.add_error_message(format!("Failed to send pony message: {err}"));
            }
        }
    }

    pub(crate) fn handle_agent_list_active(&mut self) {
        match agent_ipc::read_live_registry() {
            Ok(entries) if entries.is_empty() => {
                self.add_info_message(
                    "No live pony Codex sessions found.".to_string(),
                    Some(
                        "Live sessions heartbeat into the temp registry every 6 seconds."
                            .to_string(),
                    ),
                );
            }
            Ok(entries) => {
                let mut lines = vec![Line::from("Live pony Codex sessions:")];
                for entry in entries {
                    lines.push(Line::from(format!(
                        "- {} [{}] {}",
                        agent_ipc::display_agent_name(&entry.agent_name),
                        entry.git_branch,
                        entry.path,
                    )));
                }
                self.add_plain_history_lines(lines);
            }
            Err(err) => {
                self.add_error_message(format!("Failed to read pony registry: {err}"));
            }
        }
    }
}

#[cfg(test)]
#[path = "pony_tests.rs"]
mod tests;
