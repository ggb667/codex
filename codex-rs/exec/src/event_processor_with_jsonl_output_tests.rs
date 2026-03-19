use std::path::PathBuf;

use codex_app_server_protocol::CommandAction;
use codex_app_server_protocol::CommandExecutionSource;
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
        source: CommandExecutionSource::UserShell,
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

    let completed =
        processor.collect_thread_events(TypedExecEvent::ItemCompleted(ItemCompletedNotification {
            item: ThreadItem::CommandExecution {
                id: "cmd-1".to_string(),
                command: "ls".to_string(),
                cwd: PathBuf::from("/tmp/project"),
                process_id: Some("123".to_string()),
                source: CommandExecutionSource::UserShell,
                status: ApiCommandExecutionStatus::Completed,
                command_actions: Vec::<CommandAction>::new(),
                aggregated_output: Some("a.txt\n".to_string()),
                exit_code: Some(0),
                duration_ms: Some(3),
            },
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
        }));
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

    let collected =
        processor.collect_thread_events(TypedExecEvent::ItemCompleted(ItemCompletedNotification {
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
        }));

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

    let completed =
        processor.collect_thread_events(TypedExecEvent::TurnCompleted(TurnCompletedNotification {
            thread_id: "thread-1".to_string(),
            turn: Turn {
                id: "turn-1".to_string(),
                items: Vec::new(),
                status: TurnStatus::Completed,
                error: None,
            },
        }));
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

    let usage_update = processor.collect_thread_events(TypedExecEvent::ThreadTokenUsageUpdated(
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

    let completed =
        processor.collect_thread_events(TypedExecEvent::TurnCompleted(TurnCompletedNotification {
            thread_id: "thread-1".to_string(),
            turn: Turn {
                id: "turn-1".to_string(),
                items: Vec::new(),
                status: TurnStatus::Completed,
                error: None,
            },
        }));
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
fn turn_completion_recovers_final_message_from_turn_items() {
    let mut processor = EventProcessorWithJsonOutput::new(None);

    let completed =
        processor.collect_thread_events(TypedExecEvent::TurnCompleted(TurnCompletedNotification {
            thread_id: "thread-1".to_string(),
            turn: Turn {
                id: "turn-1".to_string(),
                items: vec![ThreadItem::AgentMessage {
                    id: "msg-1".to_string(),
                    text: "final answer".to_string(),
                    phase: None,
                    memory_citation: None,
                }],
                status: TurnStatus::Completed,
                error: None,
            },
        }));

    assert_eq!(
        completed,
        CollectedThreadEvents {
            events: vec![ThreadEvent::TurnCompleted(TurnCompletedEvent {
                usage: Usage::default(),
            })],
            status: CodexStatus::InitiateShutdown,
        }
    );
    assert_eq!(processor.final_message.as_deref(), Some("final answer"));
}

#[test]
fn turn_completion_falls_back_to_final_plan_text() {
    let mut processor = EventProcessorWithJsonOutput::new(None);

    let completed =
        processor.collect_thread_events(TypedExecEvent::TurnCompleted(TurnCompletedNotification {
            thread_id: "thread-1".to_string(),
            turn: Turn {
                id: "turn-1".to_string(),
                items: vec![ThreadItem::Plan {
                    id: "plan-1".to_string(),
                    text: "ship the typed adapter".to_string(),
                }],
                status: TurnStatus::Completed,
                error: None,
            },
        }));

    assert_eq!(
        completed,
        CollectedThreadEvents {
            events: vec![ThreadEvent::TurnCompleted(TurnCompletedEvent {
                usage: Usage::default(),
            })],
            status: CodexStatus::InitiateShutdown,
        }
    );
    assert_eq!(
        processor.final_message.as_deref(),
        Some("ship the typed adapter")
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

    let failed =
        processor.collect_thread_events(TypedExecEvent::TurnCompleted(TurnCompletedNotification {
            thread_id: "thread-1".to_string(),
            turn: Turn {
                id: "turn-1".to_string(),
                items: Vec::new(),
                status: TurnStatus::Failed,
                error: None,
            },
        }));
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
