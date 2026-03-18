use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use codex_app_server_protocol::CollabAgentTool;
use codex_app_server_protocol::CollabAgentToolCallStatus;
use codex_app_server_protocol::CommandExecutionStatus;
use codex_app_server_protocol::McpToolCallStatus;
use codex_app_server_protocol::PatchApplyStatus;
use codex_app_server_protocol::PatchChangeKind;
use codex_app_server_protocol::ThreadItem;
use codex_app_server_protocol::ThreadTokenUsage;
use codex_app_server_protocol::TurnStatus;
use codex_core::config::Config;
use codex_protocol::models::WebSearchAction;
use codex_protocol::protocol::SessionConfiguredEvent;
use serde_json::json;

use crate::event_processor::CodexStatus;
use crate::event_processor::EventProcessor;
use crate::event_processor::handle_last_message;
use crate::exec_events::AgentMessageItem;
use crate::exec_events::CollabAgentState;
use crate::exec_events::CollabAgentStatus;
use crate::exec_events::CollabTool;
use crate::exec_events::CollabToolCallItem;
use crate::exec_events::CollabToolCallStatus;
use crate::exec_events::CommandExecutionItem;
use crate::exec_events::CommandExecutionStatus as ExecCommandExecutionStatus;
use crate::exec_events::ErrorItem;
use crate::exec_events::FileChangeItem;
use crate::exec_events::FileUpdateChange;
use crate::exec_events::ItemCompletedEvent;
use crate::exec_events::ItemStartedEvent;
use crate::exec_events::ItemUpdatedEvent;
use crate::exec_events::McpToolCallItem;
use crate::exec_events::McpToolCallItemError;
use crate::exec_events::McpToolCallItemResult;
use crate::exec_events::McpToolCallStatus as ExecMcpToolCallStatus;
use crate::exec_events::PatchApplyStatus as ExecPatchApplyStatus;
use crate::exec_events::PatchChangeKind as ExecPatchChangeKind;
use crate::exec_events::ReasoningItem;
use crate::exec_events::ThreadErrorEvent;
use crate::exec_events::ThreadEvent;
use crate::exec_events::ThreadItem as ExecThreadItem;
use crate::exec_events::ThreadItemDetails;
use crate::exec_events::ThreadStartedEvent;
use crate::exec_events::TodoItem;
use crate::exec_events::TodoListItem;
use crate::exec_events::TurnCompletedEvent;
use crate::exec_events::TurnFailedEvent;
use crate::exec_events::TurnStartedEvent;
use crate::exec_events::Usage;
use crate::exec_events::WebSearchItem;
use crate::typed_exec_event::TypedExecEvent;

pub struct EventProcessorWithJsonOutput {
    last_message_path: Option<PathBuf>,
    next_item_id: AtomicU64,
    running_todo_list: Option<RunningTodoList>,
    last_total_token_usage: Option<ThreadTokenUsage>,
    last_critical_error: Option<ThreadErrorEvent>,
    final_message: Option<String>,
}

#[derive(Debug, Clone)]
struct RunningTodoList {
    item_id: String,
    items: Vec<TodoItem>,
}

#[derive(Debug, PartialEq)]
struct CollectedThreadEvents {
    events: Vec<ThreadEvent>,
    status: CodexStatus,
}

impl EventProcessorWithJsonOutput {
    pub fn new(last_message_path: Option<PathBuf>) -> Self {
        Self {
            last_message_path,
            next_item_id: AtomicU64::new(0),
            running_todo_list: None,
            last_total_token_usage: None,
            last_critical_error: None,
            final_message: None,
        }
    }

    fn next_item_id(&self) -> String {
        format!("item_{}", self.next_item_id.fetch_add(1, Ordering::SeqCst))
    }

    #[allow(clippy::print_stdout)]
    fn emit(&self, event: ThreadEvent) {
        println!(
            "{}",
            serde_json::to_string(&event).unwrap_or_else(|err| {
                json!({
                    "type": "error",
                    "message": format!("failed to serialize exec json event: {err}"),
                })
                .to_string()
            })
        );
    }

    fn usage_from_last_total(&self) -> Usage {
        let Some(usage) = self.last_total_token_usage.as_ref() else {
            return Usage::default();
        };
        Usage {
            input_tokens: usage.total.input_tokens,
            cached_input_tokens: usage.total.cached_input_tokens,
            output_tokens: usage.total.output_tokens,
        }
    }

    fn map_todo_items(plan: &[codex_app_server_protocol::TurnPlanStep]) -> Vec<TodoItem> {
        plan.iter()
            .map(|step| TodoItem {
                text: step.step.clone(),
                completed: matches!(
                    step.status,
                    codex_app_server_protocol::TurnPlanStepStatus::Completed
                ),
            })
            .collect()
    }

    fn map_item(item: ThreadItem) -> Option<ExecThreadItem> {
        let id = item.id().to_string();
        let details = match item {
            ThreadItem::AgentMessage { text, .. } => {
                ThreadItemDetails::AgentMessage(AgentMessageItem { text })
            }
            ThreadItem::Reasoning {
                summary, content, ..
            } => {
                let text = if content.is_empty() {
                    summary.join("\n")
                } else {
                    content.join("\n")
                };
                ThreadItemDetails::Reasoning(ReasoningItem { text })
            }
            ThreadItem::CommandExecution {
                command,
                aggregated_output,
                exit_code,
                status,
                ..
            } => ThreadItemDetails::CommandExecution(CommandExecutionItem {
                command,
                aggregated_output: aggregated_output.unwrap_or_default(),
                exit_code,
                status: match status {
                    CommandExecutionStatus::InProgress => ExecCommandExecutionStatus::InProgress,
                    CommandExecutionStatus::Completed => ExecCommandExecutionStatus::Completed,
                    CommandExecutionStatus::Failed => ExecCommandExecutionStatus::Failed,
                    CommandExecutionStatus::Declined => ExecCommandExecutionStatus::Declined,
                },
            }),
            ThreadItem::FileChange {
                changes, status, ..
            } => ThreadItemDetails::FileChange(FileChangeItem {
                changes: changes
                    .into_iter()
                    .map(|change| FileUpdateChange {
                        path: change.path,
                        kind: match change.kind {
                            PatchChangeKind::Add => ExecPatchChangeKind::Add,
                            PatchChangeKind::Delete => ExecPatchChangeKind::Delete,
                            PatchChangeKind::Update { .. } => ExecPatchChangeKind::Update,
                        },
                    })
                    .collect(),
                status: match status {
                    PatchApplyStatus::InProgress => ExecPatchApplyStatus::InProgress,
                    PatchApplyStatus::Completed => ExecPatchApplyStatus::Completed,
                    PatchApplyStatus::Failed | PatchApplyStatus::Declined => {
                        ExecPatchApplyStatus::Failed
                    }
                },
            }),
            ThreadItem::McpToolCall {
                server,
                tool,
                status,
                arguments,
                result,
                error,
                ..
            } => ThreadItemDetails::McpToolCall(McpToolCallItem {
                server,
                tool,
                status: match status {
                    McpToolCallStatus::InProgress => ExecMcpToolCallStatus::InProgress,
                    McpToolCallStatus::Completed => ExecMcpToolCallStatus::Completed,
                    McpToolCallStatus::Failed => ExecMcpToolCallStatus::Failed,
                },
                arguments,
                result: result.map(|result| McpToolCallItemResult {
                    content: result.content,
                    structured_content: result.structured_content,
                }),
                error: error.map(|error| McpToolCallItemError {
                    message: error.message,
                }),
            }),
            ThreadItem::CollabAgentToolCall {
                tool,
                sender_thread_id,
                receiver_thread_ids,
                prompt,
                agents_states,
                status,
                ..
            } => ThreadItemDetails::CollabToolCall(CollabToolCallItem {
                tool: match tool {
                    CollabAgentTool::SpawnAgent => CollabTool::SpawnAgent,
                    CollabAgentTool::SendInput => CollabTool::SendInput,
                    CollabAgentTool::ResumeAgent => CollabTool::Wait,
                    CollabAgentTool::Wait => CollabTool::Wait,
                    CollabAgentTool::CloseAgent => CollabTool::CloseAgent,
                },
                sender_thread_id,
                receiver_thread_ids,
                prompt,
                agents_states: agents_states
                    .into_iter()
                    .map(|(thread_id, state)| {
                        (
                            thread_id,
                            CollabAgentState {
                                status: match state.status {
                                    codex_app_server_protocol::CollabAgentStatus::PendingInit => {
                                        CollabAgentStatus::PendingInit
                                    }
                                    codex_app_server_protocol::CollabAgentStatus::Running => {
                                        CollabAgentStatus::Running
                                    }
                                    codex_app_server_protocol::CollabAgentStatus::Interrupted => {
                                        CollabAgentStatus::Interrupted
                                    }
                                    codex_app_server_protocol::CollabAgentStatus::Completed => {
                                        CollabAgentStatus::Completed
                                    }
                                    codex_app_server_protocol::CollabAgentStatus::Errored => {
                                        CollabAgentStatus::Errored
                                    }
                                    codex_app_server_protocol::CollabAgentStatus::Shutdown => {
                                        CollabAgentStatus::Shutdown
                                    }
                                    codex_app_server_protocol::CollabAgentStatus::NotFound => {
                                        CollabAgentStatus::NotFound
                                    }
                                },
                                message: state.message,
                            },
                        )
                    })
                    .collect(),
                status: match status {
                    CollabAgentToolCallStatus::InProgress => CollabToolCallStatus::InProgress,
                    CollabAgentToolCallStatus::Completed => CollabToolCallStatus::Completed,
                    CollabAgentToolCallStatus::Failed => CollabToolCallStatus::Failed,
                },
            }),
            ThreadItem::WebSearch { query, action, .. } => {
                ThreadItemDetails::WebSearch(WebSearchItem {
                    id: id.clone(),
                    query,
                    action: match action {
                        Some(action) => serde_json::from_value(
                            serde_json::to_value(action).unwrap_or_else(|_| json!("other")),
                        )
                        .unwrap_or(WebSearchAction::Other),
                        None => WebSearchAction::Other,
                    },
                })
            }
            _ => return None,
        };

        Some(ExecThreadItem { id, details })
    }

    fn thread_started_event(session_configured: &SessionConfiguredEvent) -> ThreadEvent {
        ThreadEvent::ThreadStarted(ThreadStartedEvent {
            thread_id: session_configured.session_id.to_string(),
        })
    }

    fn collect_thread_events(&mut self, event: TypedExecEvent) -> CollectedThreadEvents {
        let mut events = Vec::new();
        let status = match event {
            TypedExecEvent::Warning(message) => {
                events.push(ThreadEvent::ItemCompleted(ItemCompletedEvent {
                    item: ExecThreadItem {
                        id: self.next_item_id(),
                        details: ThreadItemDetails::Error(ErrorItem { message }),
                    },
                }));
                CodexStatus::Running
            }
            TypedExecEvent::ConfigWarning(notification) => {
                let message = match notification.details {
                    Some(details) if !details.is_empty() => {
                        format!("{} ({details})", notification.summary)
                    }
                    _ => notification.summary,
                };
                events.push(ThreadEvent::ItemCompleted(ItemCompletedEvent {
                    item: ExecThreadItem {
                        id: self.next_item_id(),
                        details: ThreadItemDetails::Error(ErrorItem { message }),
                    },
                }));
                CodexStatus::Running
            }
            TypedExecEvent::Error(notification) => {
                let message = match notification.error.additional_details {
                    Some(details) if !details.is_empty() => {
                        format!("{} ({details})", notification.error.message)
                    }
                    _ => notification.error.message,
                };
                let error = ThreadErrorEvent { message };
                self.last_critical_error = Some(error.clone());
                events.push(ThreadEvent::Error(error));
                CodexStatus::Running
            }
            TypedExecEvent::DeprecationNotice(notification) => {
                let message = match notification.details {
                    Some(details) if !details.is_empty() => {
                        format!("{} ({details})", notification.summary)
                    }
                    _ => notification.summary,
                };
                events.push(ThreadEvent::ItemCompleted(ItemCompletedEvent {
                    item: ExecThreadItem {
                        id: self.next_item_id(),
                        details: ThreadItemDetails::Error(ErrorItem { message }),
                    },
                }));
                CodexStatus::Running
            }
            TypedExecEvent::HookStarted(_) | TypedExecEvent::HookCompleted(_) => {
                CodexStatus::Running
            }
            TypedExecEvent::ItemStarted(notification) => {
                if let Some(item) = Self::map_item(notification.item) {
                    events.push(ThreadEvent::ItemStarted(ItemStartedEvent { item }));
                }
                CodexStatus::Running
            }
            TypedExecEvent::ItemCompleted(notification) => {
                if let Some(item) = Self::map_item(notification.item) {
                    if let ThreadItemDetails::AgentMessage(AgentMessageItem { text }) =
                        &item.details
                    {
                        self.final_message = Some(text.clone());
                    }
                    events.push(ThreadEvent::ItemCompleted(ItemCompletedEvent { item }));
                }
                CodexStatus::Running
            }
            TypedExecEvent::ModelRerouted(notification) => {
                events.push(ThreadEvent::ItemCompleted(ItemCompletedEvent {
                    item: ExecThreadItem {
                        id: self.next_item_id(),
                        details: ThreadItemDetails::Error(ErrorItem {
                            message: format!(
                                "model rerouted: {} -> {} ({:?})",
                                notification.from_model, notification.to_model, notification.reason
                            ),
                        }),
                    },
                }));
                CodexStatus::Running
            }
            TypedExecEvent::ThreadTokenUsageUpdated(notification) => {
                self.last_total_token_usage = Some(notification.token_usage);
                CodexStatus::Running
            }
            TypedExecEvent::TurnCompleted(notification) => {
                if let Some(running) = self.running_todo_list.take() {
                    events.push(ThreadEvent::ItemCompleted(ItemCompletedEvent {
                        item: ExecThreadItem {
                            id: running.item_id,
                            details: ThreadItemDetails::TodoList(TodoListItem {
                                items: running.items,
                            }),
                        },
                    }));
                }
                match notification.turn.status {
                    TurnStatus::Completed => {
                        events.push(ThreadEvent::TurnCompleted(TurnCompletedEvent {
                            usage: self.usage_from_last_total(),
                        }));
                        CodexStatus::InitiateShutdown
                    }
                    TurnStatus::Failed => {
                        let error = notification
                            .turn
                            .error
                            .map(|error| ThreadErrorEvent {
                                message: match error.additional_details {
                                    Some(details) if !details.is_empty() => {
                                        format!("{} ({details})", error.message)
                                    }
                                    _ => error.message,
                                },
                            })
                            .or_else(|| self.last_critical_error.clone())
                            .unwrap_or_else(|| ThreadErrorEvent {
                                message: "turn failed".to_string(),
                            });
                        events.push(ThreadEvent::TurnFailed(TurnFailedEvent { error }));
                        CodexStatus::InitiateShutdown
                    }
                    TurnStatus::Interrupted => CodexStatus::InitiateShutdown,
                    TurnStatus::InProgress => CodexStatus::Running,
                }
            }
            TypedExecEvent::TurnDiffUpdated(_) => CodexStatus::Running,
            TypedExecEvent::TurnPlanUpdated(notification) => {
                let items = Self::map_todo_items(&notification.plan);
                if let Some(running) = self.running_todo_list.as_mut() {
                    running.items = items.clone();
                    let item_id = running.item_id.clone();
                    events.push(ThreadEvent::ItemUpdated(ItemUpdatedEvent {
                        item: ExecThreadItem {
                            id: item_id,
                            details: ThreadItemDetails::TodoList(TodoListItem { items }),
                        },
                    }));
                } else {
                    let item_id = self.next_item_id();
                    self.running_todo_list = Some(RunningTodoList {
                        item_id: item_id.clone(),
                        items: items.clone(),
                    });
                    events.push(ThreadEvent::ItemStarted(ItemStartedEvent {
                        item: ExecThreadItem {
                            id: item_id,
                            details: ThreadItemDetails::TodoList(TodoListItem { items }),
                        },
                    }));
                }
                CodexStatus::Running
            }
            TypedExecEvent::TurnStarted => {
                events.push(ThreadEvent::TurnStarted(TurnStartedEvent {}));
                CodexStatus::Running
            }
        };

        CollectedThreadEvents { events, status }
    }
}

impl EventProcessor for EventProcessorWithJsonOutput {
    fn print_config_summary(
        &mut self,
        _: &Config,
        _: &str,
        session_configured: &SessionConfiguredEvent,
    ) {
        self.emit(Self::thread_started_event(session_configured));
    }

    fn process_event(&mut self, event: TypedExecEvent) -> CodexStatus {
        let collected = self.collect_thread_events(event);
        for event in collected.events {
            self.emit(event);
        }
        collected.status
    }

    fn print_final_output(&mut self) {
        if let Some(path) = self.last_message_path.as_deref() {
            handle_last_message(self.final_message.as_deref(), path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CollectedThreadEvents;
    use super::EventProcessorWithJsonOutput;
    use crate::event_processor::CodexStatus;
    use crate::exec_events::CommandExecutionItem;
    use crate::exec_events::CommandExecutionStatus;
    use crate::exec_events::ItemCompletedEvent;
    use crate::exec_events::ItemStartedEvent;
    use crate::exec_events::ItemUpdatedEvent;
    use crate::exec_events::ThreadErrorEvent;
    use crate::exec_events::ThreadEvent;
    use crate::exec_events::ThreadItem as ExecThreadItem;
    use crate::exec_events::ThreadItemDetails;
    use crate::exec_events::ThreadStartedEvent;
    use crate::exec_events::TodoItem;
    use crate::exec_events::TodoListItem;
    use crate::exec_events::TurnCompletedEvent;
    use crate::exec_events::TurnStartedEvent;
    use crate::exec_events::Usage;
    use crate::exec_events::WebSearchItem;
    use crate::typed_exec_event::TypedExecEvent;
    use codex_app_server_protocol::CommandAction;
    use codex_app_server_protocol::CommandExecutionStatus as ApiCommandExecutionStatus;
    use codex_app_server_protocol::ErrorNotification;
    use codex_app_server_protocol::ItemCompletedNotification;
    use codex_app_server_protocol::ItemStartedNotification;
    use codex_app_server_protocol::ThreadItem;
    use codex_app_server_protocol::ThreadTokenUsage;
    use codex_app_server_protocol::TokenUsageBreakdown;
    use codex_app_server_protocol::Turn;
    use codex_app_server_protocol::TurnCompletedNotification;
    use codex_app_server_protocol::TurnError;
    use codex_app_server_protocol::TurnPlanStep;
    use codex_app_server_protocol::TurnPlanStepStatus;
    use codex_app_server_protocol::TurnPlanUpdatedNotification;
    use codex_app_server_protocol::TurnStatus;
    use codex_app_server_protocol::WebSearchAction as ApiWebSearchAction;
    use codex_protocol::ThreadId;
    use codex_protocol::models::WebSearchAction;
    use codex_protocol::protocol::AskForApproval;
    use codex_protocol::protocol::SandboxPolicy;
    use codex_protocol::protocol::SessionConfiguredEvent;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    #[test]
    fn map_todo_items_preserves_text_and_completion_state() {
        let items = EventProcessorWithJsonOutput::map_todo_items(&[
            TurnPlanStep {
                step: "inspect bootstrap".to_string(),
                status: TurnPlanStepStatus::InProgress,
            },
            TurnPlanStep {
                step: "drop legacy notifications".to_string(),
                status: TurnPlanStepStatus::Completed,
            },
        ]);

        assert_eq!(
            items,
            vec![
                TodoItem {
                    text: "inspect bootstrap".to_string(),
                    completed: false,
                },
                TodoItem {
                    text: "drop legacy notifications".to_string(),
                    completed: true,
                },
            ]
        );
    }

    #[test]
    fn session_configured_produces_thread_started_event() {
        let session_configured = SessionConfiguredEvent {
            session_id: ThreadId::from_string("67e55044-10b1-426f-9247-bb680e5fe0c8")
                .expect("thread id should parse"),
            forked_from_id: None,
            thread_name: None,
            model: "codex-mini-latest".to_string(),
            model_provider_id: "test-provider".to_string(),
            service_tier: None,
            approval_policy: AskForApproval::Never,
            approvals_reviewer: codex_protocol::config_types::ApprovalsReviewer::User,
            sandbox_policy: SandboxPolicy::new_read_only_policy(),
            cwd: PathBuf::from("/tmp/project"),
            reasoning_effort: None,
            history_log_id: 0,
            history_entry_count: 0,
            initial_messages: None,
            network_proxy: None,
            rollout_path: None,
        };

        assert_eq!(
            EventProcessorWithJsonOutput::thread_started_event(&session_configured),
            ThreadEvent::ThreadStarted(ThreadStartedEvent {
                thread_id: "67e55044-10b1-426f-9247-bb680e5fe0c8".to_string(),
            })
        );
    }

    #[test]
    fn turn_started_emits_turn_started_event() {
        let mut processor = EventProcessorWithJsonOutput::new(None);

        let collected = processor.collect_thread_events(TypedExecEvent::TurnStarted);

        assert_eq!(
            collected,
            CollectedThreadEvents {
                events: vec![ThreadEvent::TurnStarted(TurnStartedEvent {})],
                status: CodexStatus::Running,
            }
        );
    }

    #[test]
    fn command_execution_started_and_completed_translate_to_thread_events() {
        let mut processor = EventProcessorWithJsonOutput::new(None);
        let command_item = ThreadItem::CommandExecution {
            id: "cmd-1".to_string(),
            command: "ls".to_string(),
            cwd: PathBuf::from("/tmp/project"),
            process_id: Some("123".to_string()),
            status: ApiCommandExecutionStatus::InProgress,
            command_actions: Vec::<CommandAction>::new(),
            aggregated_output: None,
            exit_code: None,
            duration_ms: None,
        };

        let started =
            processor.collect_thread_events(TypedExecEvent::ItemStarted(ItemStartedNotification {
                item: command_item,
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
            }));
        assert_eq!(
            started,
            CollectedThreadEvents {
                events: vec![ThreadEvent::ItemStarted(ItemStartedEvent {
                    item: ExecThreadItem {
                        id: "cmd-1".to_string(),
                        details: ThreadItemDetails::CommandExecution(CommandExecutionItem {
                            command: "ls".to_string(),
                            aggregated_output: String::new(),
                            exit_code: None,
                            status: CommandExecutionStatus::InProgress,
                        }),
                    },
                })],
                status: CodexStatus::Running,
            }
        );

        let completed = processor.collect_thread_events(TypedExecEvent::ItemCompleted(
            ItemCompletedNotification {
                item: ThreadItem::CommandExecution {
                    id: "cmd-1".to_string(),
                    command: "ls".to_string(),
                    cwd: PathBuf::from("/tmp/project"),
                    process_id: Some("123".to_string()),
                    status: ApiCommandExecutionStatus::Completed,
                    command_actions: Vec::<CommandAction>::new(),
                    aggregated_output: Some("a.txt\n".to_string()),
                    exit_code: Some(0),
                    duration_ms: Some(3),
                },
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
            },
        ));
        assert_eq!(
            completed,
            CollectedThreadEvents {
                events: vec![ThreadEvent::ItemCompleted(ItemCompletedEvent {
                    item: ExecThreadItem {
                        id: "cmd-1".to_string(),
                        details: ThreadItemDetails::CommandExecution(CommandExecutionItem {
                            command: "ls".to_string(),
                            aggregated_output: "a.txt\n".to_string(),
                            exit_code: Some(0),
                            status: CommandExecutionStatus::Completed,
                        }),
                    },
                })],
                status: CodexStatus::Running,
            }
        );
    }

    #[test]
    fn web_search_completion_preserves_query_and_action() {
        let mut processor = EventProcessorWithJsonOutput::new(None);

        let collected = processor.collect_thread_events(TypedExecEvent::ItemCompleted(
            ItemCompletedNotification {
                item: ThreadItem::WebSearch {
                    id: "search-1".to_string(),
                    query: "rust async await".to_string(),
                    action: Some(ApiWebSearchAction::Search {
                        query: Some("rust async await".to_string()),
                        queries: None,
                    }),
                },
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
            },
        ));

        assert_eq!(
            collected,
            CollectedThreadEvents {
                events: vec![ThreadEvent::ItemCompleted(ItemCompletedEvent {
                    item: ExecThreadItem {
                        id: "search-1".to_string(),
                        details: ThreadItemDetails::WebSearch(WebSearchItem {
                            id: "search-1".to_string(),
                            query: "rust async await".to_string(),
                            action: WebSearchAction::Search {
                                query: Some("rust async await".to_string()),
                                queries: None,
                            },
                        }),
                    },
                })],
                status: CodexStatus::Running,
            }
        );
    }

    #[test]
    fn plan_update_emits_started_then_updated_then_completed() {
        let mut processor = EventProcessorWithJsonOutput::new(None);

        let started = processor.collect_thread_events(TypedExecEvent::TurnPlanUpdated(
            TurnPlanUpdatedNotification {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                explanation: None,
                plan: vec![
                    TurnPlanStep {
                        step: "step one".to_string(),
                        status: TurnPlanStepStatus::Pending,
                    },
                    TurnPlanStep {
                        step: "step two".to_string(),
                        status: TurnPlanStepStatus::InProgress,
                    },
                ],
            },
        ));
        assert_eq!(
            started,
            CollectedThreadEvents {
                events: vec![ThreadEvent::ItemStarted(ItemStartedEvent {
                    item: ExecThreadItem {
                        id: "item_0".to_string(),
                        details: ThreadItemDetails::TodoList(TodoListItem {
                            items: vec![
                                TodoItem {
                                    text: "step one".to_string(),
                                    completed: false,
                                },
                                TodoItem {
                                    text: "step two".to_string(),
                                    completed: false,
                                },
                            ],
                        }),
                    },
                })],
                status: CodexStatus::Running,
            }
        );

        let updated = processor.collect_thread_events(TypedExecEvent::TurnPlanUpdated(
            TurnPlanUpdatedNotification {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                explanation: None,
                plan: vec![
                    TurnPlanStep {
                        step: "step one".to_string(),
                        status: TurnPlanStepStatus::Completed,
                    },
                    TurnPlanStep {
                        step: "step two".to_string(),
                        status: TurnPlanStepStatus::InProgress,
                    },
                ],
            },
        ));
        assert_eq!(
            updated,
            CollectedThreadEvents {
                events: vec![ThreadEvent::ItemUpdated(ItemUpdatedEvent {
                    item: ExecThreadItem {
                        id: "item_0".to_string(),
                        details: ThreadItemDetails::TodoList(TodoListItem {
                            items: vec![
                                TodoItem {
                                    text: "step one".to_string(),
                                    completed: true,
                                },
                                TodoItem {
                                    text: "step two".to_string(),
                                    completed: false,
                                },
                            ],
                        }),
                    },
                })],
                status: CodexStatus::Running,
            }
        );

        let completed = processor.collect_thread_events(TypedExecEvent::TurnCompleted(
            TurnCompletedNotification {
                thread_id: "thread-1".to_string(),
                turn: Turn {
                    id: "turn-1".to_string(),
                    items: Vec::new(),
                    status: TurnStatus::Completed,
                    error: None,
                },
            },
        ));
        assert_eq!(
            completed,
            CollectedThreadEvents {
                events: vec![
                    ThreadEvent::ItemCompleted(ItemCompletedEvent {
                        item: ExecThreadItem {
                            id: "item_0".to_string(),
                            details: ThreadItemDetails::TodoList(TodoListItem {
                                items: vec![
                                    TodoItem {
                                        text: "step one".to_string(),
                                        completed: true,
                                    },
                                    TodoItem {
                                        text: "step two".to_string(),
                                        completed: false,
                                    },
                                ],
                            }),
                        },
                    }),
                    ThreadEvent::TurnCompleted(TurnCompletedEvent {
                        usage: Usage::default(),
                    }),
                ],
                status: CodexStatus::InitiateShutdown,
            }
        );
    }

    #[test]
    fn token_usage_update_is_emitted_on_turn_completion() {
        let mut processor = EventProcessorWithJsonOutput::new(None);

        let usage_update =
            processor.collect_thread_events(TypedExecEvent::ThreadTokenUsageUpdated(
                codex_app_server_protocol::ThreadTokenUsageUpdatedNotification {
                    thread_id: "thread-1".to_string(),
                    turn_id: "turn-1".to_string(),
                    token_usage: ThreadTokenUsage {
                        total: TokenUsageBreakdown {
                            total_tokens: 42,
                            input_tokens: 10,
                            cached_input_tokens: 3,
                            output_tokens: 29,
                            reasoning_output_tokens: 7,
                        },
                        last: TokenUsageBreakdown {
                            total_tokens: 42,
                            input_tokens: 10,
                            cached_input_tokens: 3,
                            output_tokens: 29,
                            reasoning_output_tokens: 7,
                        },
                        model_context_window: Some(128_000),
                    },
                },
            ));
        assert_eq!(
            usage_update,
            CollectedThreadEvents {
                events: Vec::new(),
                status: CodexStatus::Running,
            }
        );

        let completed = processor.collect_thread_events(TypedExecEvent::TurnCompleted(
            TurnCompletedNotification {
                thread_id: "thread-1".to_string(),
                turn: Turn {
                    id: "turn-1".to_string(),
                    items: Vec::new(),
                    status: TurnStatus::Completed,
                    error: None,
                },
            },
        ));
        assert_eq!(
            completed,
            CollectedThreadEvents {
                events: vec![ThreadEvent::TurnCompleted(TurnCompletedEvent {
                    usage: Usage {
                        input_tokens: 10,
                        cached_input_tokens: 3,
                        output_tokens: 29,
                    },
                })],
                status: CodexStatus::InitiateShutdown,
            }
        );
    }

    #[test]
    fn turn_failure_prefers_structured_error_message() {
        let mut processor = EventProcessorWithJsonOutput::new(None);

        let error = processor.collect_thread_events(TypedExecEvent::Error(ErrorNotification {
            error: TurnError {
                message: "backend failed".to_string(),
                codex_error_info: None,
                additional_details: Some("request id abc".to_string()),
            },
            will_retry: false,
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
        }));
        assert_eq!(
            error,
            CollectedThreadEvents {
                events: vec![ThreadEvent::Error(ThreadErrorEvent {
                    message: "backend failed (request id abc)".to_string(),
                })],
                status: CodexStatus::Running,
            }
        );

        let failed = processor.collect_thread_events(TypedExecEvent::TurnCompleted(
            TurnCompletedNotification {
                thread_id: "thread-1".to_string(),
                turn: Turn {
                    id: "turn-1".to_string(),
                    items: Vec::new(),
                    status: TurnStatus::Failed,
                    error: None,
                },
            },
        ));
        assert_eq!(
            failed,
            CollectedThreadEvents {
                events: vec![ThreadEvent::TurnFailed(
                    crate::exec_events::TurnFailedEvent {
                        error: ThreadErrorEvent {
                            message: "backend failed (request id abc)".to_string(),
                        },
                    }
                )],
                status: CodexStatus::InitiateShutdown,
            }
        );
    }

    #[test]
    fn model_reroute_surfaces_as_error_item() {
        let mut processor = EventProcessorWithJsonOutput::new(None);

        let collected = processor.collect_thread_events(TypedExecEvent::ModelRerouted(
            codex_app_server_protocol::ModelReroutedNotification {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                from_model: "gpt-5".to_string(),
                to_model: "gpt-5-mini".to_string(),
                reason: codex_app_server_protocol::ModelRerouteReason::HighRiskCyberActivity,
            },
        ));

        assert_eq!(collected.status, CodexStatus::Running);
        assert_eq!(collected.events.len(), 1);
        let ThreadEvent::ItemCompleted(ItemCompletedEvent { item }) = &collected.events[0] else {
            panic!("expected ItemCompleted");
        };
        assert_eq!(item.id, "item_0");
        assert_eq!(
            item.details,
            ThreadItemDetails::Error(crate::exec_events::ErrorItem {
                message: "model rerouted: gpt-5 -> gpt-5-mini (HighRiskCyberActivity)".to_string(),
            })
        );
    }
}
