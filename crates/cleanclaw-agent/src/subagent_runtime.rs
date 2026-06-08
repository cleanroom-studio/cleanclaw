//! Sub-agent runtime.
//!
//! When the agent calls `spawn_subagent(target_agent_id, task)`, the
//! gateway routes the task to the target agent's own chat service,
//! running a fresh `run_turn` with the task as the user message.
//! The result is the final assistant reply, returned to the caller.

use super::loop_runner::Agent;
use super::tools::subagent::SubAgentSpawner;
use cleanclaw_core::Result;
use cleanclaw_provider::Message;
use std::sync::Arc;

/// Default sub-agent spawner. Holds a reference to the parent
/// `ChatService` (which owns the agent cache + provider pool) and
/// asks it to drive the target agent.
pub struct DefaultSubAgentSpawner {
    pub driver: Arc<dyn SubAgentDriver>,
}

#[async_trait::async_trait]
pub trait SubAgentDriver: Send + Sync {
    /// Run `target_agent_id` with `task` as the user message. Returns
    /// the agent's final reply.
    async fn drive(&self, target_agent_id: &str, task: &str) -> Result<String>;
}

#[async_trait::async_trait]
impl SubAgentSpawner for DefaultSubAgentSpawner {
    fn spawn_subagent(
        &self,
        _parent_agent_id: &str,
        target_agent_id: &str,
        task: &str,
    ) -> Result<String> {
        // The agent loop drives the tool from a synchronous context
        // (the loop's `run_turn` polls the tool future but the tool
        // itself runs as `async fn`). To bridge sync→async we use
        // `block_on`. The runtime-handle dance avoids the
        // re-entrancy case where the caller is already inside an
        // async runtime (an integration test, for example): in
        // that case `block_on` would deadlock, so we spawn a
        // dedicated worker thread that owns a fresh current-thread
        // runtime. The downside is one extra thread per call;
        // for sub-agent fan-out the call rate is low so the cost
        // is acceptable.
        let driver = self.driver.clone();
        let target = target_agent_id.to_string();
        let task = task.to_string();
        let join = std::thread::spawn(move || -> Result<String> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| {
                    cleanclaw_core::CleanClawError::Internal(format!("subagent: rt build {e}"))
                })?;
            let target_for_rt = target.clone();
            let task_for_rt = task.clone();
            rt.block_on(async move { driver.drive(&target_for_rt, &task_for_rt).await })
        });
        join.join().map_err(|e| {
            cleanclaw_core::CleanClawError::Internal(format!("subagent: thread join: {e:?}"))
        })?
    }
}

/// A `SubAgentDriver` that just runs a stored `Agent`. Useful for
/// tests and for a single-agent setup; the real gateway uses the
/// `ChatService` so sub-agents inherit the full provider / event
/// pipeline.
pub struct StaticAgentDriver {
    pub agent: Arc<Agent>,
    pub user_id: String,
}

#[async_trait::async_trait]
impl SubAgentDriver for StaticAgentDriver {
    async fn drive(&self, target_agent_id: &str, task: &str) -> Result<String> {
        // The StaticAgentDriver only knows about a single agent. We
        // verify the target matches and otherwise surface a not-found
        // error so the parent agent retries against a different one.
        if target_agent_id != self.agent.agent_id {
            return Err(cleanclaw_core::CleanClawError::NotFound(format!(
                "subagent: agent {target_agent_id} not registered"
            )));
        }
        let input = super::loop_runner::TurnInput {
            user_text: task.to_string(),
            channel: "subagent".into(),
            chat_id: format!("subagent-{}", self.agent.agent_id),
            session_key: format!("subagent-{}", self.agent.agent_id),
            user_id: self.user_id.clone(),
            owner_user_id: self.user_id.clone(),
            agent_id: self.agent.agent_id.clone(),
            is_admin: true,
            history: Vec::new(),
            attachments: Vec::new(),
        };
        let out = self.agent.run_turn(input).await?;
        Ok(out.reply)
    }
}

impl DefaultSubAgentSpawner {
    pub fn from_agent(agent: Arc<Agent>, user_id: String) -> Self {
        Self {
            driver: Arc::new(StaticAgentDriver { agent, user_id }),
        }
    }
}

#[allow(dead_code)]
fn _unused_message_check(m: &Message) -> &str {
    m.content.as_str()
}

#[cfg(test)]
mod tests {
    //! P6-3: integration tests for the sub-agent driver.
    //!
    //! These tests wire the `spawn_subagent` tool into a parent
    //! agent whose provider is a canned mock, then drive a target
    //! agent with a separate canned mock and assert that the
    //! sub-agent's reply is returned through the tool call back
    //! into the parent loop's final reply.

    use super::*;
    use crate::loop_runner::{Agent, AgentBuilder, TurnInput};
    use crate::tools::{Tool, ToolContext, ToolRegistry};
    use async_trait::async_trait;
    use cleanclaw_provider::{
        ChatResponse, Message, Provider, ProviderError, ProviderStream, StreamEvent, Usage,
    };
    use cleanclaw_store::Store;
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// Mirrors the CannedProvider in `loop_runner` tests, but
    /// also tracks how many `chat_stream` calls were issued so we
    /// can verify the target agent was actually invoked through
    /// its streaming path.
    struct CannedProvider {
        responses: Mutex<Vec<ChatResponse>>,
        stream_calls: AtomicUsize,
    }

    impl CannedProvider {
        fn new(responses: Vec<ChatResponse>) -> Self {
            Self {
                responses: Mutex::new(responses),
                stream_calls: AtomicUsize::new(0),
            }
        }
        #[allow(dead_code)]
        fn stream_call_count(&self) -> usize {
            self.stream_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl Provider for CannedProvider {
        fn name(&self) -> &str {
            "canned-subagent-test"
        }
        async fn chat(
            &self,
            _req: &cleanclaw_provider::ChatRequest,
        ) -> std::result::Result<ChatResponse, ProviderError> {
            let mut g = self.responses.lock().unwrap();
            Ok(g.remove(0))
        }
        async fn chat_stream(
            &self,
            _req: &cleanclaw_provider::ChatRequest,
        ) -> std::result::Result<ProviderStream, ProviderError> {
            self.stream_calls.fetch_add(1, Ordering::SeqCst);
            let mut g = self.responses.lock().unwrap();
            let resp = if g.is_empty() {
                ChatResponse {
                    id: "x".into(),
                    model: "m".into(),
                    message: Message::assistant("(none)"),
                    finish_reason: "stop".into(),
                    usage: Usage::default(),
                    raw: Value::Null,
                }
            } else {
                g.remove(0)
            };
            let s = async_stream::stream! {
                let parts: Vec<&str> = resp.message.content.split_whitespace().collect();
                for (i, chunk) in parts.iter().enumerate() {
                    let delta = if i + 1 < parts.len() {
                        format!("{chunk} ")
                    } else {
                        chunk.to_string()
                    };
                    yield Ok::<_, ProviderError>(StreamEvent::ContentDelta { delta });
                }
                for tc in &resp.message.tool_calls {
                    yield Ok::<_, ProviderError>(StreamEvent::ToolCallDelta {
                        index: 0,
                        id: Some(tc.id.clone()),
                        name: Some(tc.name.clone()),
                        arguments_delta: Some(
                            serde_json::to_string(&tc.arguments).unwrap_or_default(),
                        ),
                    });
                }
                yield Ok::<_, ProviderError>(StreamEvent::Done {
                    finish_reason: resp.finish_reason.clone(),
                    usage: Some(resp.usage.clone()),
                });
            };
            Ok(Box::pin(s))
        }
    }

    fn make_agent_with_provider(
        agent_id: &str,
        p: Arc<dyn Provider>,
        store: Arc<dyn Store>,
    ) -> Arc<Agent> {
        Arc::new(AgentBuilder::new(agent_id, "u1", "m", p, store).build())
    }

    fn input_for(agent_id: &str) -> TurnInput {
        TurnInput {
            user_text: "run".into(),
            channel: "test".into(),
            chat_id: "c1".into(),
            session_key: "s1".into(),
            user_id: "u1".into(),
            owner_user_id: "u1".into(),
            agent_id: agent_id.into(),
            is_admin: false,
            history: vec![],
            attachments: vec![],
        }
    }

    /// A no-op `Tool` we can register alongside the spawner so
    /// the registry isn't empty.
    struct NoopTool;
    #[async_trait]
    impl Tool for NoopTool {
        fn name(&self) -> &str {
            "noop"
        }
        fn description(&self) -> &str {
            "no-op"
        }
        fn parameters(&self) -> Value {
            json!({"type": "object", "properties": {}})
        }
        async fn call(&self, _ctx: &ToolContext, _args: Value) -> cleanclaw_core::Result<Value> {
            Ok(json!({"ok": true}))
        }
    }

    async fn fresh_store_async() -> Arc<dyn Store> {
        let s = cleanclaw_store::sqlite::SqliteStore::open(":memory:")
            .await
            .unwrap();
        s.migrate().await.unwrap();
        Arc::new(s)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn static_driver_dispatches_task_to_target_agent() {
        // Build a target agent that returns "hello from target".
        let target_p = Arc::new(CannedProvider::new(vec![ChatResponse {
            id: "r1".into(),
            model: "m".into(),
            message: Message::assistant("hello from target"),
            finish_reason: "stop".into(),
            usage: Usage::default(),
            raw: Value::Null,
        }])) as Arc<dyn Provider>;
        let target = make_agent_with_provider("a_target", target_p, fresh_store_async().await);

        let driver = StaticAgentDriver {
            agent: target,
            user_id: "u1".into(),
        };
        let reply = driver.drive("a_target", "ping").await.unwrap();
        assert_eq!(reply, "hello from target");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn static_driver_errors_on_wrong_target_id() {
        // A driver that only knows about "a1" should reject any
        // other target. The loop will see a NotFound and fall
        // back or surface the error.
        let p = Arc::new(CannedProvider::new(vec![])) as Arc<dyn Provider>;
        let agent = make_agent_with_provider("a1", p, fresh_store_async().await);
        let driver = StaticAgentDriver {
            agent,
            user_id: "u1".into(),
        };
        let err = driver.drive("unknown", "ping").await.unwrap_err();
        match err {
            cleanclaw_core::CleanClawError::NotFound(_) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_subagent_tool_rejects_self_spawn() {
        // The tool must refuse to let an agent spawn itself.
        let p = Arc::new(CannedProvider::new(vec![])) as Arc<dyn Provider>;
        let agent = make_agent_with_provider("a1", p, fresh_store_async().await);
        let spawner = DefaultSubAgentSpawner::from_agent(agent, "u1".into());
        let tool = crate::tools::subagent::SpawnSubAgentTool {
            spawner: Arc::new(spawner),
            caller_agent_id: "a1".into(),
        };
        let err = tool
            .call(
                &ToolContext::default(),
                json!({"agent_id": "a1", "task": "ping"}),
            )
            .await;
        assert!(matches!(
            err,
            Err(cleanclaw_core::CleanClawError::InvalidArgument(_))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_subagent_tool_rejects_empty_task() {
        let p = Arc::new(CannedProvider::new(vec![])) as Arc<dyn Provider>;
        let agent = make_agent_with_provider("a_target", p, fresh_store_async().await);
        let spawner = DefaultSubAgentSpawner::from_agent(agent, "u1".into());
        let tool = crate::tools::subagent::SpawnSubAgentTool {
            spawner: Arc::new(spawner),
            caller_agent_id: "a1".into(),
        };
        let err = tool
            .call(
                &ToolContext::default(),
                json!({"agent_id": "a_target", "task": ""}),
            )
            .await;
        assert!(matches!(
            err,
            Err(cleanclaw_core::CleanClawError::InvalidArgument(_))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_subagent_tool_routes_to_target_and_returns_reply() {
        // Build a target that always replies "sub says hi".
        let p = Arc::new(CannedProvider::new(vec![ChatResponse {
            id: "r1".into(),
            model: "m".into(),
            message: Message::assistant("sub says hi"),
            finish_reason: "stop".into(),
            usage: Usage::default(),
            raw: Value::Null,
        }])) as Arc<dyn Provider>;
        let target = make_agent_with_provider("a_target", p, fresh_store_async().await);
        let spawner = DefaultSubAgentSpawner::from_agent(target, "u1".into());
        let tool = crate::tools::subagent::SpawnSubAgentTool {
            spawner: Arc::new(spawner),
            caller_agent_id: "a_parent".into(),
        };
        let v = tool
            .call(
                &ToolContext::default(),
                json!({"agent_id": "a_target", "task": "what do you say?"}),
            )
            .await
            .unwrap();
        let agent_id = v.get("agent_id").and_then(|x| x.as_str()).unwrap();
        let reply = v.get("reply").and_then(|x| x.as_str()).unwrap();
        assert_eq!(agent_id, "a_target");
        assert_eq!(reply, "sub says hi");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn full_loop_with_subagent_tool_call_returns_combined_reply() {
        // End-to-end: parent agent emits a spawn_subagent tool
        // call; on the next iteration the target's reply is
        // returned and the parent produces a final text reply.
        let p_parent = Arc::new(CannedProvider::new(vec![
            // First response: tool call to spawn_subagent.
            ChatResponse {
                id: "r1".into(),
                model: "m".into(),
                message: Message {
                    role: cleanclaw_provider::Role::Assistant,
                    content: String::new(),
                    content_parts: vec![],
                    tool_calls: vec![cleanclaw_provider::ToolCall {
                        id: "tc1".into(),
                        name: "spawn_subagent".into(),
                        arguments: json!({"agent_id": "a_target", "task": "do work"}),
                    }],
                    tool_call_id: None,
                    name: None,
                    cache_control: None,
                    raw: None,
                    thinking: None,
                    timestamp: None,
                },
                finish_reason: "tool_calls".into(),
                usage: Usage::default(),
                raw: Value::Null,
            },
            // Second response: final text after the tool result.
            ChatResponse {
                id: "r2".into(),
                model: "m".into(),
                message: Message::assistant("all done"),
                finish_reason: "stop".into(),
                usage: Usage::default(),
                raw: Value::Null,
            },
        ])) as Arc<dyn Provider>;

        // Target agent: always returns "sub result".
        let p_target = Arc::new(CannedProvider::new(vec![ChatResponse {
            id: "r_sub".into(),
            model: "m".into(),
            message: Message::assistant("sub result"),
            finish_reason: "stop".into(),
            usage: Usage::default(),
            raw: Value::Null,
        }])) as Arc<dyn Provider>;
        let target = make_agent_with_provider("a_target", p_target, fresh_store_async().await);
        let spawner: Arc<dyn crate::tools::subagent::SubAgentSpawner> =
            Arc::new(DefaultSubAgentSpawner::from_agent(target, "u1".into()));
        let tool = Arc::new(crate::tools::subagent::SpawnSubAgentTool {
            spawner,
            caller_agent_id: "a_parent".into(),
        });

        let mut reg = ToolRegistry::new();
        reg.register(tool);
        reg.register(Arc::new(NoopTool));

        // Build the parent agent with the tool registry wired in.
        let parent = AgentBuilder::new("a_parent", "u1", "m", p_parent, fresh_store_async().await)
            .tools(reg)
            .build();
        let out = parent.run_turn(input_for("a_parent")).await.unwrap();
        assert_eq!(out.reply, "all done");
        assert_eq!(out.iterations, 2);
        // The tool call should have been recorded.
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].name, "spawn_subagent");
    }
}
