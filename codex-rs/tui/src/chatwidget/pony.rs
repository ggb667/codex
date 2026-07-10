use super::*;
use crate::pony_ipc;

impl ChatWidget {
    pub(super) fn maybe_start_pony_ipc(&mut self) {
        let Some(identity) = pony_ipc::pony_identity_from_env(self.config.cwd.as_ref()) else {
            return;
        };
        if let Err(err) = pony_ipc::append_registry_heartbeat(&identity) {
            tracing::debug!(error = %err, "failed to write initial pony IPC heartbeat");
        }
        self.pony_ipc_task = Some(Self::spawn_pony_ipc_task(
            self.app_event_tx.clone(),
            identity.clone(),
        ));
        self.pony_ipc_identity = Some(identity);
    }

    fn spawn_pony_ipc_task(app_event_tx: AppEventSender, identity: PonyIdentity) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut seen_ids = HashSet::new();
            loop {
                if let Err(err) = pony_ipc::append_registry_heartbeat(&identity) {
                    tracing::debug!(error = %err, "failed to refresh pony IPC heartbeat");
                }
                match pony_ipc::read_new_messages(&identity) {
                    Ok(messages) => {
                        for message in messages {
                            if !seen_ids.insert(message.id.clone()) {
                                continue;
                            }
                            if let Err(err) =
                                pony_ipc::append_incoming_message_to_mailbox(&identity, &message)
                            {
                                tracing::debug!(error = %err, from = %message.from_pony_name, "failed to append pony letter to mailbox");
                                seen_ids.remove(&message.id);
                                continue;
                            }
                            app_event_tx.send(AppEvent::PonyMessageReceived(message));
                        }
                    }
                    Err(err) => {
                        tracing::debug!(error = %err, "failed to read pony IPC messages");
                    }
                }
                tokio::time::sleep(pony_ipc::PONY_IPC_POLL_INTERVAL).await;
            }
        })
    }

    pub(crate) fn queue_or_buffer_pony_message(&mut self, message: PonyChatEntry) {
        self.pending_pony_messages.push_back(message);
        self.try_deliver_pending_pony_messages();
    }

    pub(crate) fn try_deliver_pending_pony_messages(&mut self) {
        while let Some(message) = self.pending_pony_messages.pop_front() {
            self.submit_user_message(message.prompt_text().into());
        }
    }

    pub(crate) fn handle_pony_send(&mut self, target: String, text: String) {
        let Some(identity) = self.pony_ipc_identity.as_ref() else {
            self.add_error_message(
                "Pony IPC is unavailable because this Codex session has no pony identity."
                    .to_string(),
            );
            return;
        };
        match pony_ipc::append_chat_message(identity, &target, &text) {
            Ok(_entry) => {
                let recipient = if target == "*" {
                    "all ponies".to_string()
                } else {
                    pony_ipc::display_pony_name(&target)
                };
                self.add_info_message(format!("Sent pony message to {recipient}."), Some(text));
            }
            Err(err) => {
                self.add_error_message(format!("Failed to send pony message: {err}"));
            }
        }
    }

    pub(crate) fn handle_pony_list_active(&mut self) {
        match pony_ipc::read_live_registry() {
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
                        pony_ipc::display_pony_name(&entry.pony_name),
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
