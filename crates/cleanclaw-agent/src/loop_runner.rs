//! ReAct agent loop.
//!
//! Input → system prompt assembly → LLM call → tool dispatch (repeat)
//! → final assistant message. Streams `AgentEvent`s to the supplied hub
//! when one is wired.
//!
//! # Why "ReAct"?
//!
//! The loop implements the Reason+Act pattern: each turn, the model
//! either emits a final answer (Reason→Answer) or a tool call (Reason→Act).
//! After a tool call we feed the result back as a `tool` role message and
//! loop, giving the model a chance to reason about the result before
//! acting again. We bound the loop with `max_iterations` so a runaway
//! agent can't spin forever.
//!
//! # Two execution paths
//!
//! * `run_turn` — blocking. Provider returns a single `ChatResponse`
//!   per call; the loop drives tool execution and the next LLM
//!   call in a tight series.
//! * `run_turn_stream` — SSE/streaming. Provider returns a
//!   `StreamEvent` stream; we coalesce deltas into a single
//!   assistant message and emit `Content` / `Thinking` /
//!   `ToolCall` events to the hub in real time.
//!
//! Both paths share the same loop skeleton (compact → request →
//! dispatch tools → maybe continue), so a bug fix in one lands
//! in the other by copy-paste. We tolerate the duplication to
//! keep the streaming path allocation-free of pre-emptive
//! buffering.
//!
//! # Telemetry surface
//!
//! * `event_hub`: optional `SharedEventHub` that subscribers
//!   (chat service, TUI, log shipper) listen to. The loop
//!   publishes `Content` / `Thinking` / `ToolCall` / `ToolResult`
//!   / `Done` events without blocking the model call.
//! * `hooks`: optional `HookRegistry` that fires
//!   `TurnStart` / `ToolPreCall` / `ToolPostCall` / `TurnEnd`
//!   events for operator-defined side effects (logging,
//!   billing, audit). Hooks are best-effort; their failures
//!   never abort a turn.
//! * `turn_failures`: tracks repeated tool errors within a
//!   turn so callers can short-circuit identical mistakes
//!   (the provider loop may retry by accident).
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

/// Per-turn input envelope.
///
/// Carries everything the loop needs that is *not* part of the
/// persistent agent config (model, system prompt, tool list, ...).
/// `TurnInput` is constructed by the chat surface on every inbound
/// message and consumed once.
///
/// Fields:
/// * `user_text` — the raw inbound text from the user (after
///   the surface has stripped @-mentions, command prefixes, etc.).
/// * `channel` / `chat_id` — routing identifiers used by tools
///   that need to call back to the chat surface (e.g. send a
///   follow-up image, reply in a thread).
/// * `session_key` — unique-per-conversation key used for
///   persistence (the working set is keyed by
///   `(user_id, agent_id, session_key)` in the store).
/// * `user_id` / `owner_user_id` — the *chatter* (who sent
///   the message) and the *owner* of the agent (whose
///   configuration this agent runs under). They differ in
///   shared-agent scenarios where a manager runs an agent
///   they don't own.
/// * `agent_id` — which agent (under the owner) is being
///   invoked.
/// * `is_admin` — admin flag passed through to tools that
///   gate privileged operations.
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
    /// Prior history (already in working-set form).
    ///
    /// The chat service is responsible for hydrating this from
    /// the session store before calling `run_turn`. Convention
    /// is **oldest → newest**; the loop appends the new user
    /// turn at the end so the request to the provider is in
    /// natural order. Mixing the two orderings would not crash
    /// the loop but would confuse the model.
    /// new user turn at the end.
    pub history: Vec<Message>,
    /// Optional attachments (image / file uploads from the chat
    /// surface). When non-empty, the new user turn carries
    /// Optional attachments (image / file uploads from the
    /// chat surface). When non-empty, the new user turn
    /// carries `ContentPart::ImageBase64` parts so the LLM
    /// sees the media.
    ///
    /// The transformation into a `Message` is done by
    /// `super::attachments::user_message_with_attachments`;
    /// the loop never inspects the attachment shape itself.
    /// `ContentPart::ImageBase64` parts so the LLM sees the media.
    //
    pub attachments: Vec<super::attachments::Attachment>,
}

/// Per-turn output envelope.
///
/// Returned to the chat service so it can render the final
/// reply, log the usage, and surface tool-call records to
/// subscribers (e.g. an audit dashboard).
pub struct AgentOutput {
    /// Final assistant text. For the streaming path this is
    /// the coalesced full text; for the blocking path it is
    /// the content of the last assistant message (which is
    /// the one that did not request any tool call).
    pub reply: String,
    /// Provider-reported finish reason. Values:
    ///   * `"stop"` — model emitted EOS without tool calls.
    ///   * `"tool_calls"` — model requested at least one tool.
    ///   * `"length"` — hit `max_tokens` cap before finishing.
    ///   * `"content_filter"` — model refused for safety reasons.
    ///
    /// The loop respects this only loosely: it actually
    /// breaks the iteration when `tool_calls` is empty, not
    /// when `finish_reason == "stop"`, because some models
    /// keep `finish_reason == "tool_calls"` even when the
    /// tool_calls field is empty.
    pub finish_reason: String,
    /// Accumulated token usage across all iterations of this
    /// turn. Includes input / output / cache read / cache
    /// creation tokens. The chat service uses this for
    /// per-user billing and per-agent cost dashboards.
    pub usage: Usage,
    /// Every tool call the model emitted during the turn, in
    /// order. Useful for "what did the agent do?" summaries
    /// and for replaying a turn offline.
    pub tool_calls: Vec<ToolCall>,
    /// Number of LLM round-trips the loop made to complete
    /// this turn. A simple Q&A is `1`; an agent that calls
    /// two tools and then answers is `3` (call 1, dispatch
    /// tool, call 2, dispatch tool, call 3, final answer).
    pub iterations: u32,
}

/// The agent itself — everything the loop needs that is *not*
/// per-turn.
///
/// An `Agent` is built once per (agent_id, owner_user_id)
/// pair and cached in `AgentManager` so the chat service can
/// serve many turns without rebuilding the system prompt and
/// tool list each time. The loop is read-only against this
/// struct (only `tool_extras` is mutable, behind a parking_lot
/// lock to keep contention low).
pub struct Agent {
    /// Stable id (matches the agent_id in the registry / store).
    pub agent_id: String,
    /// User that *owns* this agent (the account that wrote
    /// the agent config). Differs from the chatter's
    /// `user_id` in shared-agent scenarios.
    pub owner_user_id: String,
    /// Human-readable name. Used in tool error messages and
    /// in the events hub so subscribers can label a stream.
    pub display_name: String,
    /// Model identifier passed to the provider. Format is
    /// provider-specific (e.g. `"claude-opus-4-7"` for
    /// Anthropic, `"gpt-5"` for OpenAI). The provider
    /// implementation chooses how to interpret it.
    pub model: String,
    /// Pre-rendered system prompt. Built once by
    /// `AgentBuilder::build` from identity files, skills,
    /// tools, and any `extra_system` the operator appended.
    /// The loop never edits this — it's the source of truth
    /// for who the model thinks it is.
    pub system_prompt: String,
    /// Tool registry — every tool the agent can call. The
    /// `as_definitions()` projection is sent to the provider
    /// as the `tools` field of each `ChatRequest`.
    pub tools: ToolRegistry,
    /// Skills attached to this agent. The system prompt
    /// embeds their descriptions; the loop does not invoke
    /// skills directly (the provider may surface them as
    /// tool calls).
    pub skills: Vec<Skill>,
    /// LLM provider. `dyn Provider` so we can swap Anthropic
    /// / OpenAI / local / canned-test providers without
    /// touching the loop. Wrapped in `Arc` because the
    /// provider is shared with other agents.
    pub provider: Arc<dyn Provider>,
    /// Hard cap on the number of LLM round-trips per turn.
    /// Default 12 in `AgentBuilder::new`. Hit the cap → the
    /// loop returns `CleanClawError::Upstream` with a clear
    /// message; the chat service can choose to surface this
    /// as "agent got stuck, try again" to the user.
    pub max_iterations: u32,
    /// `max_tokens` for each provider call (NOT the turn as
    /// a whole — each iteration gets its own budget).
    pub max_tokens: u32,
    /// Sampling temperature. 0.0 = deterministic, 2.0 =
    /// chaotic. Default 0.7 in the builder.
    pub temperature: f64,
    /// Optional event hub. When `Some`, the loop publishes
    /// `Content` / `Thinking` / `ToolCall` / `ToolResult` /
    /// `Done` events. When `None`, the loop runs silently
    /// and the only output is the returned `AgentOutput`.
    pub event_hub: Option<SharedEventHub>,
    /// Workspace store. Used by the loop to *save* the
    /// compacted working set at the end of each turn (so
    /// the next turn can re-hydrate it cheaply). The loop
    /// does not *read* the store — the chat service does
    /// that before constructing `TurnInput`.
    pub store: Arc<dyn Store>,
    /// Optional hook registry. When `Some`, the loop fires
    /// `TurnStart` / `ToolPreCall` / `ToolPostCall` /
    /// `TurnEnd` events. Hooks are best-effort — their
    /// errors are logged but do not abort the turn.
    pub hooks: Option<Arc<HookRegistry>>,
    /// Per-turn failure tracker (shared between iterations
    /// of the same turn). The loop records each tool error
    /// here; the recovery layer (see `super::tool_recovery`)
    /// reads the tracker to decide whether to short-circuit
    /// a repeating failure.
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
    /// Run one ReAct turn using the **blocking** provider path.
    ///
    /// Pipeline (one iteration of the outer loop):
    /// 1. Start with `input.history`.
    /// 2. Append the new user turn (text only, or with
    ///    attachments flattened to a multi-part message).
    /// 3. Reset the per-turn failure tracker.
    /// 4. Fire `TurnStart` hook.
    /// 5. **Loop body** (see below).
    /// 6. Fire `TurnEnd` hook, publish `Done` event, save the
    ///    compacted working set back to the store.
    ///
    /// **Loop body:**
    /// 1. Increment iteration counter; bail with `Upstream`
    ///    error if it exceeds `max_iterations`.
    /// 2. Compact the working set if it grew past the token
    ///    threshold (see `should_compact`).
    /// 3. Build a `ChatRequest` with system prompt + working
    ///    set + tool definitions + sampling params.
    /// 4. Call `provider.chat`; map provider errors to the
    ///    crate-wide error type.
    /// 5. Append the assistant message to the working set,
    ///    publish events for content / tool calls.
    /// 6. If the assistant emitted no tool calls → break.
    /// 7. Otherwise, dispatch each tool sequentially (see the
    ///    in-line notes for the hooks + event flow).
    ///
    /// Why a *blocking* path at all when we have a streaming
    /// path? Two reasons:
    /// 1. Tests and offline replays don't need streaming; the
    ///    blocking path is easier to reason about.
    /// 2. The streaming path *falls through* to this path on
    ///    any mid-stream error, so a regression here is
    ///    automatically a regression in the streaming path.
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
    /// Streaming variant — surfaces deltas to the hub in real
    /// time AND drives the same multi-step tool-call loop the
    /// blocking path uses.
    ///
    /// The structure mirrors `run_turn` exactly: the only
    /// difference is that the provider call is replaced with a
    /// stream of `StreamEvent`s which we coalesce into a
    /// single `Message`. The dispatch path is identical.
    ///
    /// Failure mode: any error during the stream (provider
    /// error, network error, parse error, an explicit
    /// `StreamEvent::Error`) falls through to the blocking
    /// path. This means a streaming-enabled agent still
    /// works when the provider loses its SSE channel — the
    /// user just doesn't see live deltas.
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

    /// Resolve the per-agent `promptMode` override (e.g.
    /// `"thinking"` for Anthropic).
    ///
    /// Stub: full implementation reads from the
    /// agent.json override map. Today every agent runs in
    /// the default mode, so the loop adds the extra `extra`
    /// bag only when this returns `Some`.
    fn prompt_mode(&self) -> Option<String> {
        // Stub: full implementation reads from agent.json override.
        None
    }
}

/// Sum a `cleanclaw_provider::Usage` into a running
/// `Usage` total. Used by both the blocking and streaming
/// paths to track cumulative token spend across iterations.
/// Cache read / creation tokens are summed in `accumulate_usage`
/// AND also added explicitly by the streaming path (which
/// gets a `Usage` from `StreamEvent::Done` separately).
/// The duplicate addition is intentional: `accumulate_usage`
/// handles the common case, and the streaming path covers
/// the case where the provider only surfaces cache counts
/// in the final `Done` event.
fn accumulate_usage(into: &mut Usage, from: &cleanclaw_provider::Usage) {
    into.input_tokens += from.input_tokens;
    into.output_tokens += from.output_tokens;
    into.cache_read_tokens += from.cache_read_tokens;
    into.cache_creation_tokens += from.cache_creation_tokens;
}

/// Map a `cleanclaw_provider::ProviderError` into the
/// crate-wide `CleanClawError`.
///
/// The translation is 1:1 with one wrinkle: `RateLimited`
/// becomes `CleanClawError::RateLimited` (a first-class
/// variant) so the chat service can implement a dedicated
/// back-off / retry strategy for it. All other variants
/// collapse into `Upstream(_)` or `Internal(_)`.
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
/// Per-user lazy-loaded agent cache.
///
/// Mirrors the agent-cache pattern used in the Go daemon.
/// The chat service holds a single `AgentManager` for the
/// lifetime of the process; on the first turn for a given
/// agent_id it calls `get_or_build` (typically in a
/// `manager.rs` layer above this file) to populate the
/// cache. Subsequent turns reuse the cached `Arc<Agent>`
/// and avoid re-paying the system-prompt build cost.
///
/// Concurrency: a single `tokio::sync::Mutex` guards the
/// map. Operations are O(1) and the critical section is a
/// few HashMap lookups, so contention is negligible. A
/// `DashMap` would buy us nothing at this scale.
pub struct AgentManager {
    agents: Mutex<std::collections::HashMap<String, Arc<Agent>>>,
}

impl AgentManager {
    /// Build an empty manager. The chat service calls this
    /// once at boot.
    pub fn new() -> Self {
        Self {
            agents: Mutex::new(Default::default()),
        }
    }

    /// Insert or replace an agent by id. Used by the
    /// manager layer to populate the cache after
    /// `AgentBuilder::build`.
    pub async fn put(&self, id: &str, a: Arc<Agent>) {
        self.agents.lock().await.insert(id.to_string(), a);
    }

    /// Look up an agent by id. Returns `None` if the
    /// manager has never seen it. The caller is
    /// responsible for the `get_or_build` semantics
    /// (this method is a pure cache read).
    pub async fn get(&self, id: &str) -> Option<Arc<Agent>> {
        self.agents.lock().await.get(id).cloned()
    }

    /// Drop an agent from the cache. Called when an
    /// agent's config is updated at runtime so the next
    /// turn rebuilds the `Agent` from scratch.
    pub async fn invalidate(&self, id: &str) {
        self.agents.lock().await.remove(id);
    }

    /// Snapshot every cached agent. Used by ops
    /// commands ("list all running agents") and by the
    /// `/admin/agents` endpoint.
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

/// Builder for `Agent`.
///
/// The builder is the **only** supported way to construct
/// an `Agent` — it centralises the system-prompt assembly
/// (identity + skills + tools + extra_system) so callers
/// can't forget to wire one of the pieces.
///
/// Usage:
/// ```ignore
/// AgentBuilder::new(agent_id, owner_user_id, model, provider, store)
///     .display_name("Ops Bot")
///     .max_iterations(20)
///     .tools(my_tool_registry)
///     .skills(my_skills)
///     .identity(identity_files)
///     .build()
/// ```
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
    /// Create a builder with sensible defaults and the
    /// mandatory fields.
    ///
    /// `agent_id`, `owner_user_id`, `model`, `provider`,
    /// `store` are required and have no default. The
    /// remaining fields are populated by the per-field
    /// builder methods below.
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

    /// Set the human-readable name (used in tool error
    /// messages and event labels).
    pub fn display_name(mut self, n: impl Into<String>) -> Self {
        self.display_name = n.into();
        self
    }
    /// Override the per-turn iteration cap (default 12).
    pub fn max_iterations(mut self, n: u32) -> Self {
        self.max_iterations = n;
        self
    }
    /// Override the per-call `max_tokens` (default 4096).
    pub fn max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = n;
        self
    }
    /// Override the sampling temperature (default 0.7).
    pub fn temperature(mut self, t: f64) -> Self {
        self.temperature = t;
        self
    }
    /// Attach a list of skills. The system prompt will
    /// embed their descriptions; the loop never invokes
    /// them directly.
    pub fn skills(mut self, s: Vec<Skill>) -> Self {
        self.skills = s;
        self
    }
    /// Attach a tool registry. Tools listed here are the
    /// only ones the model can call during this turn.
    pub fn tools(mut self, t: ToolRegistry) -> Self {
        self.tools = t;
        self
    }
    /// Attach an event hub. Once attached, the loop
    /// publishes `Content` / `Thinking` / `ToolCall` /
    /// `ToolResult` / `Done` events.
    pub fn event_hub(mut self, h: SharedEventHub) -> Self {
        self.event_hub = Some(h);
        self
    }
    /// Attach identity files (AGENTS.md, IDENTITY.md,
    /// USER.md, ...). The system prompt embeds them at
    /// the top, before the skill list.
    pub fn identity(mut self, id: IdentityFiles) -> Self {
        self.identity = id;
        self
    }
    /// Append a free-form `Extra` section to the system
    /// prompt. Use sparingly — anything in this section
    /// applies to *every* turn, including ones where the
    /// user just asked a factual question.
    pub fn extra_system(mut self, s: impl Into<String>) -> Self {
        self.extra_system = s.into();
        self
    }
    /// Attach a hook registry. Once attached, the loop
    /// fires `TurnStart` / `ToolPreCall` / `ToolPostCall` /
    /// `TurnEnd` events.
    pub fn hooks(mut self, h: Arc<HookRegistry>) -> Self {
        self.hooks = Some(h);
        self
    }

    /// Build the `Agent`.
    ///
    /// This is where the system prompt is assembled. The
    /// `ContextBuilder` (see `super::context`) takes the
    /// identity files, skills, and the rendered tool list
    /// and returns a single string. We append the
    /// `extra_system` (if any) under an `Extra` heading
    /// so the model can tell operator-supplied content
    /// from the agent's own identity.
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
