use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::*;
use crate::context::TargetInfo;
use crate::error::{AgentError, AgentResult, ModelError};
use crate::model::{
    FinishReason, ModelCapabilities, ModelEvent, ModelEventStream, ModelRequest, ReasoningReplay,
    TokenUsage, ToolSpec, WireMessage,
};
use crate::policy::{ApprovalMode, ShellDialect, assess_command};
use crate::toolkit::{
    ProgressSender, ToolAssessment, ToolInvocation, ToolOutcome, ToolSetOptions, tool_class,
    tool_specs,
};
use crate::tools::AgentToolCancellation;
use crate::transcript::{ItemKind, ToolDisplay, ToolResultStatus};

type Script = Result<Vec<ModelEvent>, ModelError>;

struct ScriptedModel {
    scripts: Mutex<VecDeque<Script>>,
    requests: Mutex<Vec<ModelRequest>>,
}

impl ScriptedModel {
    fn new(scripts: Vec<Script>) -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::new(scripts.into()),
            requests: Mutex::new(Vec::new()),
        })
    }
}

impl ModelClient for ScriptedModel {
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            vision: false,
            parallel_tool_calls: true,
            reasoning_replay: ReasoningReplay::CurrentTurn,
            context_window: 128_000,
        }
    }

    fn stream(&self, request: ModelRequest) -> BoxFuture<'_, AgentResult<ModelEventStream>> {
        self.requests.lock().unwrap().push(request);
        let script = self
            .scripts
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(text("(script exhausted)")));
        Box::pin(async move {
            match script {
                Ok(events) => {
                    let stream: ModelEventStream =
                        Box::pin(futures::stream::iter(events.into_iter().map(Ok)));
                    Ok(stream)
                }
                Err(error) => Err(AgentError::Model(error)),
            }
        })
    }
}

fn text(value: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta(value.into()),
        ModelEvent::Usage(TokenUsage {
            input_tokens: 10,
            output_tokens: 2,
            cached_input_tokens: 0,
        }),
        ModelEvent::Finish(FinishReason::Stop),
    ]
}

fn call(id: &str, name: &str, args: Value) -> Vec<ModelEvent> {
    vec![
        ModelEvent::ReasoningDelta("thinking".into()),
        ModelEvent::ToolCallStart {
            index: 0,
            id: id.into(),
            name: name.into(),
        },
        ModelEvent::ToolCallArgsDelta {
            index: 0,
            delta: args.to_string(),
        },
        ModelEvent::Finish(FinishReason::ToolCalls),
    ]
}

#[derive(Default)]
struct FakeTools {
    executed: Mutex<Vec<String>>,
    /// Block shell execution until cancelled.
    hang: bool,
}

impl ToolRuntime for FakeTools {
    fn specs(&self, options: &ToolSetOptions) -> Vec<ToolSpec> {
        tool_specs(options)
    }

    fn assess<'a>(
        &'a self,
        call: &'a ToolInvocation,
    ) -> BoxFuture<'a, Result<ToolAssessment, String>> {
        Box::pin(async move {
            let class = tool_class(&call.name).ok_or("unknown")?;
            let command = call.arguments["command"].as_str().unwrap_or_default();
            let assessment = if command.is_empty() {
                assess_command("cat x", ShellDialect::Posix)
            } else {
                assess_command(command, ShellDialect::Posix)
            };
            Ok(ToolAssessment {
                class,
                target: "web".into(),
                risk: assessment.risk,
                reason: assessment.reason,
                grant_prefix: assessment.grant_prefix,
                summary: command.to_string(),
                editable: Some(command.to_string()),
            })
        })
    }

    fn apply_edit(&self, call: &ToolInvocation, edited: &str) -> Value {
        let mut arguments = call.arguments.clone();
        arguments["command"] = Value::String(edited.into());
        arguments
    }

    fn execute(
        &self,
        call: ToolInvocation,
        cancel: AgentToolCancellation,
        _progress: ProgressSender,
    ) -> BoxFuture<'static, ToolOutcome> {
        let label = call.arguments["command"]
            .as_str()
            .or(call.arguments["path"].as_str())
            .unwrap_or_default()
            .to_string();
        self.executed.lock().unwrap().push(label.clone());
        let hang = self.hang;
        Box::pin(async move {
            if hang {
                cancel.cancelled().await;
                return ToolOutcome::cancelled();
            }
            ToolOutcome::ok(format!("ran {label}"), ToolDisplay::None)
        })
    }
}

struct Harness {
    handle: AgentHandle,
    events: mpsc::UnboundedReceiver<AgentEvent>,
    model: Arc<ScriptedModel>,
    tools: Arc<FakeTools>,
    seen: Vec<Item>,
}

fn start(scripts: Vec<Script>, mode: ApprovalMode, tools: FakeTools) -> Harness {
    let model = ScriptedModel::new(scripts);
    let tools = Arc::new(tools);
    let limits = TurnLimits {
        max_retries: 2,
        ..TurnLimits::default()
    };
    let (handle, events) = spawn_session(
        AgentSessionConfig {
            session_id: "s".into(),
            approval: ApprovalSettings {
                mode,
                auto_approve_dangerous: false,
            },
            limits,
            targets: vec![TargetInfo {
                name: "web".into(),
                is_default: true,
                ..Default::default()
            }],
            web_search: false,
            terminal_available: false,
        },
        Vec::new(),
        AgentDeps {
            model: model.clone(),
            tools: tools.clone(),
            store: Arc::new(NullSink),
        },
    );
    Harness {
        handle,
        events,
        model,
        tools,
        seen: Vec::new(),
    }
}

fn submit(text: &str) -> AgentCommand {
    AgentCommand::Submit {
        text: text.into(),
        attachments: vec![],
        targets: vec![],
        context: None,
    }
}

impl Harness {
    async fn next(&mut self) -> AgentEvent {
        let event = tokio::time::timeout(Duration::from_secs(20), self.events.recv())
            .await
            .expect("timed out waiting for engine event")
            .expect("event channel closed");
        if let AgentEvent::ItemCommitted(item) = &event {
            self.seen.push(item.clone());
        }
        event
    }

    /// All committed items seen so far, once the turn finishes.
    async fn finish(&mut self) -> (Vec<Item>, TurnEndReason) {
        loop {
            if let AgentEvent::TurnFinished { reason, .. } = self.next().await {
                return (std::mem::take(&mut self.seen), reason);
            }
        }
    }

    async fn until_approval(&mut self) -> ApprovalRequest {
        loop {
            if let AgentEvent::ApprovalRequested(request) = self.next().await {
                return request;
            }
        }
    }
}

fn results(items: &[Item]) -> Vec<(ToolResultStatus, String)> {
    items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::ToolResult {
                status, model_text, ..
            } => Some((status.clone(), model_text.clone())),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn plain_answer_completes() {
    let mut h = start(
        vec![Ok(text("hello"))],
        ApprovalMode::Standard,
        FakeTools::default(),
    );
    h.handle.send(submit("hi"));
    let (items, reason) = h.finish().await;
    assert_eq!(reason, TurnEndReason::Completed);
    let kinds: Vec<_> = items.iter().map(|item| item.kind_tag()).collect();
    assert_eq!(kinds, vec!["user", "assistant", "usage"]);
}

#[tokio::test]
async fn read_only_tool_runs_without_approval_and_loop_continues() {
    let mut h = start(
        vec![
            Ok(call("c1", "shell", json!({"command": "uptime"}))),
            Ok(text("load is fine")),
        ],
        ApprovalMode::Cautious,
        FakeTools::default(),
    );
    h.handle.send(submit("check load"));
    let (items, reason) = h.finish().await;
    assert_eq!(reason, TurnEndReason::Completed);
    assert_eq!(
        results(&items),
        vec![(ToolResultStatus::Ok, "ran uptime".into())]
    );
    // Second request replays the call, its result and current-turn reasoning.
    let requests = h.model.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(matches!(
        requests[1].messages.last(),
        Some(WireMessage::ToolResult { call_id, .. }) if call_id == "c1"
    ));
}

#[tokio::test]
async fn approval_is_awaited_inside_the_loop() {
    let mut h = start(
        vec![
            Ok(call(
                "c1",
                "shell",
                json!({"command": "systemctl restart nginx"}),
            )),
            Ok(text("restarted")),
        ],
        ApprovalMode::Standard,
        FakeTools::default(),
    );
    h.handle.send(submit("restart nginx"));
    let request = h.until_approval().await;
    assert_eq!(request.call_id, "c1");
    assert_eq!(request.grant_prefix.as_deref(), Some("systemctl restart"));
    assert!(h.tools.executed.lock().unwrap().is_empty());
    h.handle.send(AgentCommand::Decide {
        call_id: "c1".into(),
        decision: ApprovalDecision::ApproveEdited {
            edited: "systemctl reload nginx".into(),
        },
    });
    let (items, reason) = h.finish().await;
    assert_eq!(reason, TurnEndReason::Completed);
    assert_eq!(
        *h.tools.executed.lock().unwrap(),
        vec!["systemctl reload nginx".to_string()]
    );
    assert_eq!(results(&items)[0].0, ToolResultStatus::Ok);
}

#[tokio::test]
async fn denial_is_reported_to_the_model_and_grants_skip_later_prompts() {
    let mut h = start(
        vec![
            Ok(call(
                "c1",
                "shell",
                json!({"command": "systemctl restart nginx"}),
            )),
            Ok(call(
                "c2",
                "shell",
                json!({"command": "systemctl restart nginx"}),
            )),
            Ok(call(
                "c3",
                "shell",
                json!({"command": "systemctl restart nginx"}),
            )),
            Ok(text("done")),
        ],
        ApprovalMode::Standard,
        FakeTools::default(),
    );
    h.handle.send(submit("restart"));
    h.until_approval().await;
    h.handle.send(AgentCommand::Decide {
        call_id: "c1".into(),
        decision: ApprovalDecision::Deny {
            reason: Some("not now".into()),
        },
    });
    h.until_approval().await;
    h.handle.send(AgentCommand::Decide {
        call_id: "c2".into(),
        decision: ApprovalDecision::ApproveAndGrant,
    });
    // c3 is covered by the grant: no third approval.
    let (items, reason) = h.finish().await;
    assert_eq!(reason, TurnEndReason::Completed);
    let results = results(&items);
    assert_eq!(
        results[0],
        (
            ToolResultStatus::Denied {
                by_user: true,
                reason: Some("not now".into())
            },
            "User denied this call. Reason: not now".into()
        )
    );
    assert_eq!(results[1].0, ToolResultStatus::Ok);
    assert_eq!(results[2].0, ToolResultStatus::Ok);
}

#[tokio::test]
async fn interrupt_while_waiting_cancels_and_repairs() {
    let mut h = start(
        vec![Ok(call("c1", "shell", json!({"command": "rm -rf build"})))],
        ApprovalMode::Standard,
        FakeTools::default(),
    );
    h.handle.send(submit("clean"));
    h.until_approval().await;
    h.handle.send(AgentCommand::Interrupt);
    let (items, reason) = h.finish().await;
    assert_eq!(reason, TurnEndReason::Interrupted);
    assert_eq!(results(&items)[0].0, ToolResultStatus::Cancelled);
    assert!(h.tools.executed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn interrupt_cancels_running_tool() {
    let mut h = start(
        vec![Ok(call(
            "c1",
            "shell",
            json!({"command": "tail -f /var/log/syslog"}),
        ))],
        ApprovalMode::Standard,
        FakeTools {
            hang: true,
            ..Default::default()
        },
    );
    h.handle.send(submit("watch"));
    loop {
        if let AgentEvent::Status(SessionStatus::RunningTools) = h.next().await {
            break;
        }
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    h.handle.send(AgentCommand::Interrupt);
    let (items, reason) = h.finish().await;
    assert_eq!(reason, TurnEndReason::Interrupted);
    assert_eq!(results(&items)[0].0, ToolResultStatus::Cancelled);
}

#[tokio::test(start_paused = true)]
async fn transient_errors_are_retried() {
    let error = ModelError {
        status: Some(503),
        message: "overloaded".into(),
        retryable: true,
        context_overflow: false,
    };
    let mut h = start(
        vec![Err(error), Ok(text("ok"))],
        ApprovalMode::Standard,
        FakeTools::default(),
    );
    h.handle.send(submit("hi"));
    let (_, reason) = h.finish().await;
    assert_eq!(reason, TurnEndReason::Completed);
    assert_eq!(h.model.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn fatal_errors_fail_the_turn_with_a_notice() {
    let error = ModelError {
        status: Some(401),
        message: "bad key".into(),
        retryable: false,
        context_overflow: false,
    };
    let mut h = start(
        vec![Err(error)],
        ApprovalMode::Standard,
        FakeTools::default(),
    );
    h.handle.send(submit("hi"));
    let (items, reason) = h.finish().await;
    assert_eq!(reason, TurnEndReason::Failed);
    assert!(items.iter().any(|item| matches!(
        &item.kind,
        ItemKind::Notice { text, .. } if text.contains("bad key")
    )));
}

#[tokio::test]
async fn repeated_invalid_calls_abort_after_three() {
    let bad = || Ok(call("x", "shell", json!({"cmd": "ls"})));
    let mut h = start(
        vec![bad(), bad(), bad(), Ok(text("never"))],
        ApprovalMode::Standard,
        FakeTools::default(),
    );
    h.handle.send(submit("go"));
    let (items, reason) = h.finish().await;
    assert_eq!(reason, TurnEndReason::Failed);
    assert_eq!(results(&items).len(), 3);
    assert!(
        results(&items)
            .iter()
            .all(|(status, text)| *status == ToolResultStatus::Failed
                && text.contains("Invalid arguments"))
    );
}

#[tokio::test]
async fn read_only_mode_denies_changes_and_ask_user_waits_for_answer() {
    let mut h = start(
        vec![
            Ok(call("c1", "shell", json!({"command": "touch /tmp/x"}))),
            Ok(call(
                "c2",
                "ask_user",
                json!({"question": "Which host?", "choices": ["a", "b"]}),
            )),
            Ok(text("thanks")),
        ],
        ApprovalMode::ReadOnly,
        FakeTools::default(),
    );
    h.handle.send(submit("go"));
    loop {
        if let AgentEvent::QuestionAsked {
            call_id, options, ..
        } = h.next().await
        {
            assert_eq!(call_id, "c2");
            assert_eq!(options, vec!["a".to_string(), "b".to_string()]);
            break;
        }
    }
    h.handle.send(AgentCommand::Answer {
        call_id: "c2".into(),
        answer: "a".into(),
    });
    let (items, reason) = h.finish().await;
    assert_eq!(reason, TurnEndReason::Completed);
    let results = results(&items);
    assert!(matches!(
        results[0].0,
        ToolResultStatus::Denied { by_user: false, .. }
    ));
    assert_eq!(
        results[1],
        (ToolResultStatus::Ok, "User answered: a".into())
    );
    assert!(h.tools.executed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn steer_is_injected_before_next_request() {
    let mut h = start(
        vec![
            Ok(call("c1", "ask_user", json!({"question": "?"}))),
            Ok(text("ok")),
        ],
        ApprovalMode::Standard,
        FakeTools::default(),
    );
    h.handle.send(submit("go"));
    loop {
        if let AgentEvent::QuestionAsked { .. } = h.next().await {
            break;
        }
    }
    h.handle.send(AgentCommand::Steer {
        text: "also check disk".into(),
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    h.handle.send(AgentCommand::Answer {
        call_id: "c1".into(),
        answer: "yes".into(),
    });
    let (items, _) = h.finish().await;
    let users: Vec<_> = items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::UserMessage { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(users, vec!["go".to_string(), "also check disk".to_string()]);
}
