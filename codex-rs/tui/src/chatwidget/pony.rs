use super::*;
use crate::agent_ipc;

impl ChatWidget {
    pub(super) fn maybe_start_agent_ipc(&mut self) {
        let Some(identity) = agent_ipc::agent_identity_from_env(self.config.cwd.as_ref()) else {
            return;
        };
        if let Err(err) = agent_ipc::append_registry_heartbeat(&identity) {
            tracing::debug!(error = %err, "failed to write initial pony IPC heartbeat");
        }
        self.agent_ipc_task = Some(Self::spawn_agent_ipc_task(
            self.app_event_tx.clone(),
            identity.clone(),
        ));
        self.agent_ipc_identity = Some(identity);
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
                match agent_ipc::read_new_messages(&identity) {
                    Ok(messages) => {
                        for message in messages {
                            if agent_ipc::receipt_recorded(&identity, &message.id).unwrap_or(false)
                            {
                                continue;
                            }
                            if let Err(err) =
                                agent_ipc::append_incoming_message_to_mailbox(&identity, &message)
                            {
                                tracing::debug!(error = %err, from = %message.from_agent_name, "failed to append pony letter to mailbox");
                                continue;
                            }
                            if let Err(err) = agent_ipc::record_receipt(&identity, &message.id) {
                                tracing::debug!(error = %err, "failed to record pony IPC receipt");
                                continue;
                            }
                            app_event_tx.send(AppEvent::AgentMessageReceived(message));
                        }
                    }
                    Err(err) => {
                        tracing::debug!(error = %err, "failed to read pony IPC messages");
                    }
                }
                tokio::time::sleep(agent_ipc::AGENT_IPC_POLL_INTERVAL).await;
            }
        })
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
