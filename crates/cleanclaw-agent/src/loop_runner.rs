//! ReAct agent loop.
//!
//! Input → system prompt assembly → LLM call → tool dispatch (repeat)
//! → final assistant message. Streams `AgentEvent`s to the supplied hub
//! when one is wired.

use super::compact::{compact_in_place, save_compacted, should_compact, DEFAULT_TRIGGER_TOKENS};
use super::context::{ContextBuilder, IdentityFiles};
use super::event_hub::{AgentEvent, EventEnvelope, SharedEventHub, Usage};
use super::hooks::{HookPhase, HookRegistry};
use super::tool_recovery::TurnFailures;
use super::tools::{tool_definitions_message, ToolContext, ToolRegistry};
use cleanclaw_core::{CleanClawError, Result};
use cleanclaw_provider::{ChatRequest, Message, Provider, ToolCall};
use cleanclaw_skills::Skill;
use cleanclaw_store::Store;
use futures_util::StreamExt;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct TurnInput {
    pub user_text: String,
    pub channel: String,
    pub chat_id: String,
    pub session_key: String,
    pub user_id: String,
    pub owner_user_id: String,
    pub agent_id: String,
    pub is_admin: bool,
    /// Prior history (already in working-set form). Most recent first
    /// OR last — convention is oldest→newest, the loop appends the
    /// new user turn at the end.
    pub history: Vec<Message>,
    /// Optional attachments (image / file uploads from the chat
    /// surface). When non-empty, the new user turn carries
    /// `ContentPart::ImageBase64` parts so the LLM sees the media.
    //
    pub attachments: Vec<super::attachments::Attachment>,
}

pub struct AgentOutput {
    pub reply: String,
    pub finish_reason: String,
    pub usage: Usage,
    pub tool_calls: Vec<ToolCall>,
    pub iterations: u32,
}

pub struct Agent {
    pub agent_id: String,
    pub owner_user_id: String,
    pub display_name: String,
    pub model: String,
    pub system_prompt: String,
    pub tools: ToolRegistry,
    pub skills: Vec<Skill>,
    pub provider: Arc<dyn Provider>,
    pub max_iterations: u32,
    pub max_tokens: u32,
    pub temperature: f64,
    pub event_hub: Option<SharedEventHub>,
    pub store: Arc<dyn Store>,
    pub hooks: Option<Arc<HookRegistry>>,
    pub turn_failures: Arc<TurnFailures>,
    /// Per-tool runtime context that the agent loop spreads
    /// into every `ToolContext.extra` it builds. The chat
    /// service uses this to hand per-turn config (e.g. the
    /// web_search provider credentials) to tool impls without
    /// mutating the tool itself.
    pub tool_extras: parking_lot::RwLock<std::collections::HashMap<String, serde_json::Value>>,
}

impl Agent {
    /// Snapshot the tool_extras map so the agent loop can
    /// safely share an `Arc<HashMap>` with the tool dispatcher.
    /// Reads the `RwLock` once per turn.
    fn tool_extras_snapshot(&self) -> Arc<std::collections::HashMap<String, serde_json::Value>> {
        Arc::new(self.tool_extras.read().clone())
    }

    /// Run one ReAct turn.
    pub async fn run_turn(&self, input: TurnInput) -> Result<AgentOutput> {
        let mut messages = input.history.clone();
        let user_msg = if input.attachments.is_empty() {
            Message::user(input.user_text.clone())
        } else {
            super::attachments::user_message_with_attachments(&input.user_text, &input.attachments)
        };
        messages.push(user_msg);

        // Reset per-turn failure tracker so a fresh turn doesn't
        // inherit prior turns' record.
        self.turn_failures.reset();

        // Fire TurnStart hooks (best-effort).
        if let Some(hooks) = &self.hooks {
            hooks
                .fire(
                    HookPhase::TurnStart,
                    serde_json::json!({
                        "agent_id": self.agent_id,
                        "user_id": input.user_id,
                        "session_key": input.session_key,
                    }),
                )
                .await;
        }

        let mut iterations: u32 = 0;
        let mut total_usage = Usage::default();
        let mut all_tool_calls: Vec<ToolCall> = Vec::new();
        #[allow(unused_assignments)]
        let mut last_reply = String::new();
        #[allow(unused_assignments)]
        let mut last_finish = String::new();

        let tool_defs = self.tools.as_definitions();
        let _tools_section = tool_definitions_message(&tool_defs);

        loop {
            iterations += 1;
            if iterations > self.max_iterations.max(1) {
                return Err(CleanClawError::Upstream(format!(
                    "agent exceeded max_iterations ({})",
                    self.max_iterations
                )));
            }

            // Compact the working set if it has grown past the
            // token threshold. Cheap to run; short-circuits when
            // the list is already small.
            if let Some(keep) = should_compact(&messages, DEFAULT_TRIGGER_TOKENS) {
                messages = compact_in_place(messages, keep);
            }

            let mut req = ChatRequest {
                model: self.model.clone(),
                messages: std::iter::once(Message::system(&self.system_prompt))
                    .chain(messages.iter().cloned())
                    .collect(),
                tools: tool_defs.clone(),
                temperature: Some(self.temperature),
                max_tokens: Some(self.max_tokens),
                top_p: None,
                stop: vec![],
                stream: false,
                extra: Default::default(),
            };
            if let Some(prompt_mode) = self.prompt_mode() {
                req.extra
                    .insert("promptMode".into(), Value::String(prompt_mode));
            }

            let resp = self.provider.chat(&req).await.map_err(map_provider_err)?;
            accumulate_usage(&mut total_usage, &resp.usage);

            // Persist raw assistant for cache-hit replay.
            let assistant_message = resp.message.clone();
            messages.push(assistant_message.clone());

            if let Some(hub) = &self.event_hub {
                if !assistant_message.content.is_empty() {
                    hub.publish(EventEnvelope {
                        agent_id: self.agent_id.clone(),
                        user_id: input.user_id.clone(),
                        session_key: input.session_key.clone(),
                        event: AgentEvent::Content {
                            delta: assistant_message.content.clone(),
                        },
                    });
                }
                for tc in &assistant_message.tool_calls {
                    hub.publish(EventEnvelope {
                        agent_id: self.agent_id.clone(),
                        user_id: input.user_id.clone(),
                        session_key: input.session_key.clone(),
                        event: AgentEvent::ToolCall {
                            name: tc.name.clone(),
                            id: tc.id.clone(),
                            arguments: tc.arguments.clone(),
                        },
                    });
                }
            }

            last_reply = assistant_message.content.clone();
            last_finish = resp.finish_reason.clone();
            all_tool_calls.extend(assistant_message.tool_calls.iter().cloned());

            // If the provider didn't surface structured tool calls
            // but the content carries XML tool-call blocks (some
            // non-Anthropic models do this), recover them so the
            // dispatch loop still picks them up. The recovered
            // calls REPLACE the empty `tool_calls` field.
            let mut dispatch_calls = assistant_message.tool_calls.clone();
            if dispatch_calls.is_empty() {
                let recovered = super::tool_recovery::recover_tool_calls_from_text(
                    &assistant_message.content,
                    &assistant_message.tool_calls,
                );
                if !recovered.is_empty() {
                    for r in &recovered {
                        tracing::info!(
                            tool = %r.call.name,
                            source_len = r.source_text.len(),
                            "recovered tool call from XML in assistant content"
                        );
                    }
                    dispatch_calls = recovered.iter().map(|r| r.call.clone()).collect();
                    // Persist the synthesized calls back onto the
                    // message so the next iteration's history sees
                    // them.
                    let mut m = assistant_message.clone();
                    m.tool_calls = dispatch_calls.clone();
                    messages.pop();
                    messages.push(m);
                }
            }

            if dispatch_calls.is_empty() {
                break;
            }

            // Execute tools sequentially. The CleanClaw loop caps
            // parallelism at max_parallel_tool_calls; the simple
            // first-cut here just runs them serially.
            let ctx = ToolContext {
                agent_id: self.agent_id.clone(),
                owner_user_id: self.owner_user_id.clone(),
                chatter_user_id: input.user_id.clone(),
                channel: input.channel.clone(),
                chat_id: input.chat_id.clone(),
                account_id: input.user_id.clone(),
                session_key: input.session_key.clone(),
                project_id: String::new(),
                is_admin: input.is_admin,
                workspace_root: String::new(),
                extra: self.tool_extras_snapshot(),
            };

            for tc in &assistant_message.tool_calls {
                // ToolPreCall hook.
                if let Some(hooks) = &self.hooks {
                    hooks
                        .fire(
                            HookPhase::ToolPreCall,
                            serde_json::json!({
                                "agent_id": self.agent_id,
                                "tool": tc.name,
                                "arguments": tc.arguments,
                            }),
                        )
                        .await;
                }
                let started = std::time::Instant::now();
                let result = self
                    .tools
                    .dispatch(&ctx, &tc.name, tc.arguments.clone())
                    .await;
                let duration_ms = started.elapsed().as_millis() as u64;
                let (content, is_error) = match result {
                    Ok(v) => (
                        serde_json::to_string(&v).unwrap_or_else(|_| "null".into()),
                        false,
                    ),
                    Err(e) => {
                        let msg = e.to_string();
                        // Record for the turn-failure tracker so a
                        // retry of the same call doesn't repeat the
                        // same mistake blindly.
                        self.turn_failures.record(&tc.name, &tc.arguments, &msg);
                        (format!("[tool error: {msg}]"), true)
                    }
                };
                // ToolPostCall hook.
                if let Some(hooks) = &self.hooks {
                    hooks
                        .fire(
                            HookPhase::ToolPostCall,
                            serde_json::json!({
                                "agent_id": self.agent_id,
                                "tool": tc.name,
                                "is_error": is_error,
                                "duration_ms": duration_ms,
                            }),
                        )
                        .await;
                }
                if let Some(hub) = &self.event_hub {
                    hub.publish(EventEnvelope {
                        agent_id: self.agent_id.clone(),
                        user_id: input.user_id.clone(),
                        session_key: input.session_key.clone(),
                        event: AgentEvent::ToolResult {
                            id: tc.id.clone(),
                            content: content.clone(),
                            is_error,
                        },
                    });
                }
                messages.push(Message::tool_result(&tc.id, content));
            }
        }

        // Fire TurnEnd hook.
        if let Some(hooks) = &self.hooks {
            hooks
                .fire(
                    HookPhase::TurnEnd,
                    serde_json::json!({
                        "agent_id": self.agent_id,
                        "user_id": input.user_id,
                        "session_key": input.session_key,
                        "finish_reason": last_finish,
                        "iterations": iterations,
                        "usage": total_usage,
                    }),
                )
                .await;
        }

        // Final done event so listeners can flush.
        if let Some(hub) = &self.event_hub {
            hub.publish(EventEnvelope {
                agent_id: self.agent_id.clone(),
                user_id: input.user_id.clone(),
                session_key: input.session_key.clone(),
                event: AgentEvent::Done {
                    finish_reason: last_finish.clone(),
                    usage: Some(total_usage.clone()),
                },
            });
        }

        // Persist the compacted working set back to the store. The
        // session_messages archive is left alone — compaction is
        // purely a working-set optimization.
        if let Err(e) = save_compacted(
            &self.store,
            &input.user_id,
            &self.agent_id,
            &input.session_key,
            &messages,
        )
        .await
        {
            tracing::warn!(?e, "save_compacted failed");
        }

        Ok(AgentOutput {
            reply: last_reply,
            finish_reason: last_finish,
            usage: total_usage,
            tool_calls: all_tool_calls,
            iterations,
        })
    }

    /// Streaming variant — surfaces deltas to the hub in real time
    /// AND drives the same multi-step tool-call loop the blocking
    /// path uses. Falls through to the blocking path if the provider
    /// errors mid-stream.
    pub async fn run_turn_stream(&self, input: TurnInput) -> Result<AgentOutput> {
        use cleanclaw_provider::StreamEvent;
        let mut messages = input.history.clone();
        let user_msg = if input.attachments.is_empty() {
            Message::user(input.user_text.clone())
        } else {
            super::attachments::user_message_with_attachments(&input.user_text, &input.attachments)
        };
        messages.push(user_msg);
        self.turn_failures.reset();

        // Fire TurnStart hooks.
        if let Some(hooks) = &self.hooks {
            hooks
                .fire(
                    HookPhase::TurnStart,
                    serde_json::json!({
                        "agent_id": self.agent_id,
                        "user_id": input.user_id,
                        "session_key": input.session_key,
                    }),
                )
                .await;
        }

        let mut iterations: u32 = 0;
        let mut total_usage = Usage::default();
        let mut all_tool_calls: Vec<ToolCall> = Vec::new();
        #[allow(unused_assignments)]
        let mut last_reply = String::new();
        #[allow(unused_assignments)]
        let mut last_finish = String::new();
        let tool_defs = self.tools.as_definitions();

        loop {
            iterations += 1;
            if iterations > self.max_iterations.max(1) {
                return Err(CleanClawError::Upstream(format!(
                    "agent exceeded max_iterations ({})",
                    self.max_iterations
                )));
            }

            if let Some(keep) = should_compact(&messages, DEFAULT_TRIGGER_TOKENS) {
                messages = compact_in_place(messages, keep);
            }

            let mut req = ChatRequest {
                model: self.model.clone(),
                messages: std::iter::once(Message::system(&self.system_prompt))
                    .chain(messages.iter().cloned())
                    .collect(),
                tools: tool_defs.clone(),
                temperature: Some(self.temperature),
                max_tokens: Some(self.max_tokens),
                top_p: None,
                stop: vec![],
                stream: true,
                extra: Default::default(),
            };
            if let Some(prompt_mode) = self.prompt_mode() {
                req.extra
                    .insert("promptMode".into(), Value::String(prompt_mode));
            }

            // Build the request, then drain the stream. We accumulate
            // a `Message` incrementally so the next loop iteration (if
            // any) sees the same assistant text the user just saw.
            let stream = match self.provider.chat_stream(&req).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(?e, "chat_stream failed, falling back to blocking");
                    let out = self.run_turn(input).await?;
                    return Ok(out);
                }
            };

            let mut full_content = String::new();
            let mut full_thinking = String::new();
            let mut pending_tool_calls: Vec<ToolCall> = Vec::new();
            let mut stream_usage: Option<cleanclaw_provider::Usage> = None;
            let mut stream_finish = String::new();

            tokio::pin!(stream);
            while let Some(ev) = stream.next().await {
                let ev = match ev {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!(?e, "stream error, falling back to blocking");
                        let out = self.run_turn(input).await?;
                        return Ok(out);
                    }
                };
                match ev {
                    StreamEvent::ContentDelta { delta } => {
                        full_content.push_str(&delta);
                        if let Some(hub) = &self.event_hub {
                            hub.publish(EventEnvelope {
                                agent_id: self.agent_id.clone(),
                                user_id: input.user_id.clone(),
                                session_key: input.session_key.clone(),
                                event: AgentEvent::Content { delta },
                            });
                        }
                    }
                    StreamEvent::ThinkingDelta { delta } => {
                        full_thinking.push_str(&delta);
                        if let Some(hub) = &self.event_hub {
                            hub.publish(EventEnvelope {
                                agent_id: self.agent_id.clone(),
                                user_id: input.user_id.clone(),
                                session_key: input.session_key.clone(),
                                event: AgentEvent::Thinking { delta },
                            });
                        }
                    }
                    StreamEvent::ToolCallDelta {
                        index,
                        id,
                        name,
                        arguments_delta,
                    } => {
                        while pending_tool_calls.len() <= index {
                            pending_tool_calls.push(ToolCall {
                                id: String::new(),
                                name: String::new(),
                                arguments: Value::Null,
                            });
                        }
                        let tc = &mut pending_tool_calls[index];
                        if let Some(id) = id {
                            tc.id = id;
                        }
                        if let Some(name) = name {
                            tc.name = name;
                        }
                        if let Some(args) = arguments_delta {
                            // Coalesce: a single text deltas is the
                            // JSON of the tool arguments. Stash the
                            // raw text; parse on Done.
                            if let Value::String(ref mut buf) = tc.arguments {
                                buf.push_str(&args);
                            } else {
                                tc.arguments = Value::String(args);
                            }
                        }
                    }
                    StreamEvent::Done {
                        finish_reason,
                        usage,
                    } => {
                        stream_finish = finish_reason;
                        stream_usage = usage;
                    }
                    StreamEvent::Error { message } => {
                        tracing::warn!(?message, "stream reported error, falling back");
                        let out = self.run_turn(input).await?;
                        return Ok(out);
                    }
                }
            }

            // Promote coalesced text into a parsed JSON Value for each
            // tool call. Falls back to Null when the JSON is malformed
            // — the tool's own validation will surface a clearer error.
            for tc in &mut pending_tool_calls {
                if let Value::String(raw) = &tc.arguments {
                    tc.arguments = serde_json::from_str(raw).unwrap_or(Value::String(raw.clone()));
                }
            }

            accumulate_usage(&mut total_usage, &stream_usage.clone().unwrap_or_default());
            // Add the cached-token deltas if the provider surfaced them.
            if let Some(u) = &stream_usage {
                total_usage.cache_read_tokens += u.cache_read_tokens;
                total_usage.cache_creation_tokens += u.cache_creation_tokens;
            }

            // Persist the full assistant message back into the working
            // set so the next iteration can re-feed it.
            let mut assistant_message =
                Message::assistant_with_thinking(&full_content, &full_thinking);
            assistant_message.tool_calls = pending_tool_calls.clone();
            messages.push(assistant_message.clone());

            for tc in &pending_tool_calls {
                if let Some(hub) = &self.event_hub {
                    hub.publish(EventEnvelope {
                        agent_id: self.agent_id.clone(),
                        user_id: input.user_id.clone(),
                        session_key: input.session_key.clone(),
                        event: AgentEvent::ToolCall {
                            name: tc.name.clone(),
                            id: tc.id.clone(),
                            arguments: tc.arguments.clone(),
                        },
                    });
                }
            }

            last_reply = full_content.clone();
            last_finish = stream_finish.clone();
            all_tool_calls.extend(pending_tool_calls.iter().cloned());

            if pending_tool_calls.is_empty() {
                break;
            }

            // Same sequential tool dispatch as the blocking path.
            let ctx = ToolContext {
                agent_id: self.agent_id.clone(),
                owner_user_id: self.owner_user_id.clone(),
                chatter_user_id: input.user_id.clone(),
                channel: input.channel.clone(),
                chat_id: input.chat_id.clone(),
                account_id: input.user_id.clone(),
                session_key: input.session_key.clone(),
                project_id: String::new(),
                is_admin: input.is_admin,
                workspace_root: String::new(),
                extra: self.tool_extras_snapshot(),
            };

            for tc in &pending_tool_calls {
                if let Some(hooks) = &self.hooks {
                    hooks
                        .fire(
                            HookPhase::ToolPreCall,
                            serde_json::json!({
                                "agent_id": self.agent_id,
                                "tool": tc.name,
                                "arguments": tc.arguments,
                            }),
                        )
                        .await;
                }
                let started = std::time::Instant::now();
                let result = self
                    .tools
                    .dispatch(&ctx, &tc.name, tc.arguments.clone())
                    .await;
                let duration_ms = started.elapsed().as_millis() as u64;
                let (content, is_error) = match result {
                    Ok(v) => (
                        serde_json::to_string(&v).unwrap_or_else(|_| "null".into()),
                        false,
                    ),
                    Err(e) => {
                        let msg = e.to_string();
                        self.turn_failures.record(&tc.name, &tc.arguments, &msg);
                        (format!("[tool error: {msg}]"), true)
                    }
                };
                if let Some(hooks) = &self.hooks {
                    hooks
                        .fire(
                            HookPhase::ToolPostCall,
                            serde_json::json!({
                                "agent_id": self.agent_id,
                                "tool": tc.name,
                                "is_error": is_error,
                                "duration_ms": duration_ms,
                            }),
                        )
                        .await;
                }
                if let Some(hub) = &self.event_hub {
                    hub.publish(EventEnvelope {
                        agent_id: self.agent_id.clone(),
                        user_id: input.user_id.clone(),
                        session_key: input.session_key.clone(),
                        event: AgentEvent::ToolResult {
                            id: tc.id.clone(),
                            content: content.clone(),
                            is_error,
                        },
                    });
                }
                messages.push(Message::tool_result(&tc.id, content));
            }
        }

        if let Some(hooks) = &self.hooks {
            hooks
                .fire(
                    HookPhase::TurnEnd,
                    serde_json::json!({
                        "agent_id": self.agent_id,
                        "user_id": input.user_id,
                        "session_key": input.session_key,
                        "finish_reason": last_finish,
                        "iterations": iterations,
                        "usage": total_usage,
                    }),
                )
                .await;
        }
        if let Some(hub) = &self.event_hub {
            hub.publish(EventEnvelope {
                agent_id: self.agent_id.clone(),
                user_id: input.user_id.clone(),
                session_key: input.session_key.clone(),
                event: AgentEvent::Done {
                    finish_reason: last_finish.clone(),
                    usage: Some(total_usage.clone()),
                },
            });
        }
        if let Err(e) = save_compacted(
            &self.store,
            &input.user_id,
            &self.agent_id,
            &input.session_key,
            &messages,
        )
        .await
        {
            tracing::warn!(?e, "save_compacted failed");
        }

        Ok(AgentOutput {
            reply: last_reply,
            finish_reason: last_finish,
            usage: total_usage,
            tool_calls: all_tool_calls,
            iterations,
        })
    }

    fn prompt_mode(&self) -> Option<String> {
        // Stub: full implementation reads from agent.json override.
        None
    }
}

fn accumulate_usage(into: &mut Usage, from: &cleanclaw_provider::Usage) {
    into.input_tokens += from.input_tokens;
    into.output_tokens += from.output_tokens;
    into.cache_read_tokens += from.cache_read_tokens;
    into.cache_creation_tokens += from.cache_creation_tokens;
}

fn map_provider_err(e: cleanclaw_provider::ProviderError) -> CleanClawError {
    use cleanclaw_provider::ProviderError::*;
    match e {
        Auth(m) => CleanClawError::Upstream(format!("auth: {m}")),
        RateLimited => CleanClawError::RateLimited,
        Http(m) => CleanClawError::Upstream(format!("http: {m}")),
        Upstream(m) => CleanClawError::Upstream(m),
        Decode(m) => CleanClawError::Upstream(format!("decode: {m}")),
        Config(m) => CleanClawError::Internal(format!("provider config: {m}")),
    }
}

// ---- Manager -------------------------------------------------------------

/// Per-user lazy-loaded agent cache. Mirrors
/// .
pub struct AgentManager {
    agents: Mutex<std::collections::HashMap<String, Arc<Agent>>>,
}

impl AgentManager {
    pub fn new() -> Self {
        Self {
            agents: Mutex::new(Default::default()),
        }
    }

    pub async fn put(&self, id: &str, a: Arc<Agent>) {
        self.agents.lock().await.insert(id.to_string(), a);
    }

    pub async fn get(&self, id: &str) -> Option<Arc<Agent>> {
        self.agents.lock().await.get(id).cloned()
    }

    pub async fn invalidate(&self, id: &str) {
        self.agents.lock().await.remove(id);
    }

    pub async fn all(&self) -> Vec<Arc<Agent>> {
        self.agents.lock().await.values().cloned().collect()
    }
}

impl Default for AgentManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Builder -------------------------------------------------------------

pub struct AgentBuilder {
    pub agent_id: String,
    pub owner_user_id: String,
    pub display_name: String,
    pub model: String,
    pub provider: Arc<dyn Provider>,
    pub store: Arc<dyn Store>,
    pub max_iterations: u32,
    pub max_tokens: u32,
    pub temperature: f64,
    pub skills: Vec<Skill>,
    pub tools: ToolRegistry,
    pub event_hub: Option<SharedEventHub>,
    pub identity: IdentityFiles,
    pub extra_system: String,
    pub hooks: Option<Arc<HookRegistry>>,
}

impl AgentBuilder {
    pub fn new(
        agent_id: impl Into<String>,
        owner_user_id: impl Into<String>,
        model: impl Into<String>,
        provider: Arc<dyn Provider>,
        store: Arc<dyn Store>,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            owner_user_id: owner_user_id.into(),
            display_name: String::new(),
            model: model.into(),
            provider,
            store,
            max_iterations: 12,
            max_tokens: 4096,
            temperature: 0.7,
            skills: Vec::new(),
            tools: ToolRegistry::new(),
            event_hub: None,
            identity: IdentityFiles::empty(),
            extra_system: String::new(),
            hooks: None,
        }
    }

    pub fn display_name(mut self, n: impl Into<String>) -> Self {
        self.display_name = n.into();
        self
    }
    pub fn max_iterations(mut self, n: u32) -> Self {
        self.max_iterations = n;
        self
    }
    pub fn max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = n;
        self
    }
    pub fn temperature(mut self, t: f64) -> Self {
        self.temperature = t;
        self
    }
    pub fn skills(mut self, s: Vec<Skill>) -> Self {
        self.skills = s;
        self
    }
    pub fn tools(mut self, t: ToolRegistry) -> Self {
        self.tools = t;
        self
    }
    pub fn event_hub(mut self, h: SharedEventHub) -> Self {
        self.event_hub = Some(h);
        self
    }
    pub fn identity(mut self, id: IdentityFiles) -> Self {
        self.identity = id;
        self
    }
    pub fn extra_system(mut self, s: impl Into<String>) -> Self {
        self.extra_system = s.into();
        self
    }
    pub fn hooks(mut self, h: Arc<HookRegistry>) -> Self {
        self.hooks = Some(h);
        self
    }

    pub fn build(self) -> Agent {
        let cb = ContextBuilder::new();
        let tool_defs = self.tools.as_definitions();
        let tools_section = tool_definitions_message(&tool_defs);
        let mut sys = cb.build(&self.identity, &self.skills, &tools_section);
        if !self.extra_system.is_empty() {
            sys.push_str("\n\n# Extra\n\n");
            sys.push_str(&self.extra_system);
        }
        Agent {
            agent_id: self.agent_id,
            owner_user_id: self.owner_user_id,
            display_name: self.display_name,
            model: self.model,
            system_prompt: sys,
            tools: self.tools,
            skills: self.skills,
            provider: self.provider,
            max_iterations: self.max_iterations,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            event_hub: self.event_hub,
            store: self.store,
            hooks: self.hooks,
            turn_failures: Arc::new(TurnFailures::new()),
            tool_extras: parking_lot::RwLock::new(std::collections::HashMap::new()),
        }
    }
}

#[cfg(test)]
mod loop_tests {
    //! Tests for the streaming run_turn_stream path. Uses a tiny
    //! canned `Stream` Provider that emits a fixed sequence of
    //! `StreamEvent`s so the loop's coalescing + tool-dispatch
    //! behavior can be verified offline.

    use super::*;
    use async_stream::stream;
    use cleanclaw_provider::{
        ChatRequest, ChatResponse, Provider, ProviderError, ProviderStream, StreamEvent, Usage,
    };
    use cleanclaw_store::Store;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A canned provider that returns a fresh response on each
    /// `chat` call. The chat history is captured so tests can
    /// inspect how the loop appended tool results.
    struct CannedProvider {
        responses: std::sync::Mutex<Vec<ChatResponse>>,
        stream_calls: AtomicUsize,
    }

    impl CannedProvider {
        fn new(responses: Vec<ChatResponse>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses),
                stream_calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl Provider for CannedProvider {
        fn name(&self) -> &str {
            "canned"
        }
        async fn chat(
            &self,
            _req: &ChatRequest,
        ) -> std::result::Result<ChatResponse, ProviderError> {
            let mut g = self.responses.lock().unwrap();
            if g.is_empty() {
                Ok(ChatResponse {
                    id: "x".into(),
                    model: "test".into(),
                    message: Message::assistant("(none)"),
                    finish_reason: "stop".into(),
                    usage: Usage::default(),
                    raw: Value::Null,
                })
            } else {
                Ok(g.remove(0))
            }
        }
        async fn chat_stream(
            &self,
            _req: &ChatRequest,
        ) -> std::result::Result<ProviderStream, ProviderError> {
            self.stream_calls.fetch_add(1, Ordering::SeqCst);
            // Build a fresh response off the front of the queue and
            // project it into a stream of StreamEvents.
            let mut g = self.responses.lock().unwrap();
            let resp = if g.is_empty() {
                ChatResponse {
                    id: "x".into(),
                    model: "test".into(),
                    message: Message::assistant("(none)"),
                    finish_reason: "stop".into(),
                    usage: Usage::default(),
                    raw: Value::Null,
                }
            } else {
                g.remove(0)
            };
            let s = stream! {
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

    fn make_agent(p: Arc<dyn Provider>, store: Arc<dyn Store>) -> Agent {
        AgentBuilder::new("a1", "u1", "test-model", p, store).build()
    }

    fn input() -> TurnInput {
        TurnInput {
            user_text: "hello".into(),
            channel: "test".into(),
            chat_id: "c1".into(),
            session_key: "s1".into(),
            user_id: "u1".into(),
            owner_user_id: "u1".into(),
            agent_id: "a1".into(),
            is_admin: false,
            history: vec![],
            attachments: vec![],
        }
    }

    #[tokio::test]
    async fn stream_path_emits_text_reply_without_tool_calls() {
        let canned = ChatResponse {
            id: "r1".into(),
            model: "m".into(),
            message: Message::assistant("hi there"),
            finish_reason: "stop".into(),
            usage: Usage {
                input_tokens: 5,
                output_tokens: 2,
                ..Default::default()
            },
            raw: Value::Null,
        };
        let p = Arc::new(CannedProvider::new(vec![canned]));
        let store = cleanclaw_store::sqlite::SqliteStore::open(":memory:")
            .await
            .unwrap();
        let store: Arc<dyn Store> = Arc::new(store);
        store.migrate().await.unwrap();
        let agent = make_agent(p.clone(), store);
        let out = agent.run_turn_stream(input()).await.unwrap();
        assert_eq!(out.reply, "hi there");
        assert_eq!(out.finish_reason, "stop");
        assert_eq!(out.iterations, 1);
        assert_eq!(out.usage.output_tokens, 2);
        assert!(out.tool_calls.is_empty());
    }

    #[tokio::test]
    async fn stream_path_coalesces_text_deltas_into_final_message() {
        // The CannedProvider splits the assistant text on whitespace
        // and emits each token as a separate ContentDelta. The loop
        // must reconstruct the full text into the `reply` field.
        let canned = ChatResponse {
            id: "r1".into(),
            model: "m".into(),
            message: Message::assistant("alpha beta gamma"),
            finish_reason: "stop".into(),
            usage: Usage::default(),
            raw: Value::Null,
        };
        let p = Arc::new(CannedProvider::new(vec![canned]));
        let store = cleanclaw_store::sqlite::SqliteStore::open(":memory:")
            .await
            .unwrap();
        let store: Arc<dyn Store> = Arc::new(store);
        store.migrate().await.unwrap();
        let agent = make_agent(p.clone(), store);
        let out = agent.run_turn_stream(input()).await.unwrap();
        assert_eq!(out.reply, "alpha beta gamma");
    }

    #[tokio::test]
    async fn stream_path_handles_tool_call_then_final_text() {
        // First response: a tool call. Second response: a text reply
        // after the tool result. The loop should iterate twice.
        let tool_response = ChatResponse {
            id: "r1".into(),
            model: "m".into(),
            message: Message {
                role: cleanclaw_provider::Role::Assistant,
                content: String::new(),
                content_parts: vec![],
                tool_calls: vec![ToolCall {
                    id: "tc1".into(),
                    name: "echo".into(),
                    arguments: serde_json::json!({ "text": "hi" }),
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
        };
        let final_response = ChatResponse {
            id: "r2".into(),
            model: "m".into(),
            message: Message::assistant("done"),
            finish_reason: "stop".into(),
            usage: Usage::default(),
            raw: Value::Null,
        };
        let p = Arc::new(CannedProvider::new(vec![tool_response, final_response]));
        let store = cleanclaw_store::sqlite::SqliteStore::open(":memory:")
            .await
            .unwrap();
        let store: Arc<dyn Store> = Arc::new(store);
        store.migrate().await.unwrap();

        // Register a minimal "echo" tool.
        use crate::tools::{Tool, ToolRegistry};
        struct EchoTool;
        #[async_trait::async_trait]
        impl Tool for EchoTool {
            fn name(&self) -> &str {
                "echo"
            }
            fn description(&self) -> &str {
                "echo back the input"
            }
            fn parameters(&self) -> Value {
                serde_json::json!({
                    "type": "object",
                    "properties": { "text": { "type": "string" } }
                })
            }
            async fn call(&self, _ctx: &ToolContext, args: Value) -> cleanclaw_core::Result<Value> {
                Ok(serde_json::json!({ "echo": args["text"] }))
            }
        }
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(EchoTool));
        let agent = AgentBuilder::new("a1", "u1", "m", p.clone(), store)
            .tools(reg)
            .build();
        let out = agent.run_turn_stream(input()).await.unwrap();
        assert_eq!(out.reply, "done");
        assert_eq!(out.iterations, 2);
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].name, "echo");
    }

    #[tokio::test]
    async fn stream_path_returns_done_event_on_final_iteration() {
        use crate::event_hub::{EventHub, SharedEventHub};
        let canned = ChatResponse {
            id: "r1".into(),
            model: "m".into(),
            message: Message::assistant("ok"),
            finish_reason: "stop".into(),
            usage: Usage::default(),
            raw: Value::Null,
        };
        let p = Arc::new(CannedProvider::new(vec![canned]));
        let store = cleanclaw_store::sqlite::SqliteStore::open(":memory:")
            .await
            .unwrap();
        let store: Arc<dyn Store> = Arc::new(store);
        store.migrate().await.unwrap();
        let hub: SharedEventHub = Arc::new(EventHub::new(16));
        let mut rx = hub.subscribe();
        let agent = AgentBuilder::new("a1", "u1", "m", p, store)
            .event_hub(hub)
            .build();
        let _ = agent.run_turn_stream(input()).await.unwrap();
        // Drain the broadcast channel: we expect at least a Content
        // event for "ok" and a final Done event.
        let mut got_done = false;
        let mut got_content = false;
        while let Ok(env) = rx.try_recv() {
            match env.event {
                AgentEvent::Done { .. } => got_done = true,
                AgentEvent::Content { .. } => got_content = true,
                _ => {}
            }
        }
        assert!(got_content, "no Content event published");
        assert!(got_done, "no Done event published");
    }

    #[test]
    fn message_assistant_with_thinking_round_trip() {
        let m = Message::assistant_with_thinking("hi", "thinking");
        assert_eq!(m.content, "hi");
        assert_eq!(m.thinking.as_deref(), Some("thinking"));
        let m2 = Message::assistant_with_thinking("hi", "");
        assert!(m2.thinking.is_none());
    }
}
