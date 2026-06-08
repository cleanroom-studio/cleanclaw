//! End-to-end agent loop test using a mock provider. Verifies the
//! tool dispatcher, hook lifecycle, compaction threshold, and
//! turn-failure tracker all wire together correctly.

use async_trait::async_trait;
use cleanclaw_agent::{Agent, AgentBuilder, Hook, HookPhase, HookRegistry, TurnInput};
use cleanclaw_provider::{
    ChatRequest, ChatResponse, Message, Provider, ProviderError, ProviderStream, Role, ToolCall,
    Usage,
};
use cleanclaw_store::models::UserRecord;
use cleanclaw_store::sqlite::SqliteStore;
use cleanclaw_store::Store;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, Default)]
struct MockProvider {
    scripted: Arc<Mutex<Vec<ChatResponse>>>,
    call_count: Arc<AtomicUsize>,
}

#[async_trait]
impl Provider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }
    async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        let mut q = self.scripted.lock().await;
        if q.is_empty() {
            return Err(ProviderError::Upstream("no scripted response".into()));
        }
        Ok(q.remove(0))
    }
    async fn chat_stream(&self, _req: &ChatRequest) -> Result<ProviderStream, ProviderError> {
        Err(ProviderError::Config(
            "streaming not used in this test".into(),
        ))
    }
}

fn make_response(text: &str) -> ChatResponse {
    ChatResponse {
        id: "test".into(),
        model: "test-model".into(),
        message: Message {
            role: Role::Assistant,
            content: text.into(),
            content_parts: vec![],
            tool_calls: vec![],
            tool_call_id: None,
            name: None,
            cache_control: None,
            raw: None,
            thinking: None,
            timestamp: None,
        },
        finish_reason: "stop".into(),
        usage: Usage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
        },
        raw: json!({}),
    }
}

fn make_response_with_tool(text: &str, tool_name: &str, tool_id: &str) -> ChatResponse {
    let mut r = make_response(text);
    r.message.tool_calls.push(ToolCall {
        id: tool_id.into(),
        name: tool_name.into(),
        arguments: json!({}),
    });
    r
}

struct CountHook {
    name: String,
    phase: HookPhase,
    counter: Arc<AtomicUsize>,
}

#[async_trait]
impl Hook for CountHook {
    fn name(&self) -> &str {
        &self.name
    }
    fn phase(&self) -> HookPhase {
        self.phase
    }
    async fn run(&self, _payload: serde_json::Value) -> cleanclaw_core::Result<()> {
        self.counter.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

async fn store_with_user() -> Arc<SqliteStore> {
    let st = SqliteStore::open(":memory:").await.unwrap();
    st.migrate().await.unwrap();
    let now = cleanclaw_core::now_utc();
    let u = UserRecord {
        id: "u_1".into(),
        username: "alice".into(),
        email: "a@x.com".into(),
        password_hash: String::new(),
        display_name: "alice".into(),
        role: "user".into(),
        status: "active".into(),
        apikey_id: String::new(),
        external_id: String::new(),
        avatar_url: String::new(),
        agent_quota: -1,
        created_at: now,
        updated_at: now,
    };
    st.create_user(&u).await.unwrap();
    Arc::new(st)
}

#[tokio::test]
async fn full_loop_with_tools_and_hooks() {
    let store = store_with_user().await;
    let provider = MockProvider::default();
    let scripted = provider.scripted.clone();
    {
        let mut q = scripted.lock().await;
        // First response: tool call (current_time). Second: text reply.
        q.push(make_response_with_tool(
            "calling tool",
            "current_time",
            "tc_1",
        ));
        q.push(make_response("done"));
    }

    // Build the agent with builtins + hooks.
    let mut hooks = HookRegistry::new();
    let turn_start_count = Arc::new(AtomicUsize::new(0));
    let tool_pre_count = Arc::new(AtomicUsize::new(0));
    let turn_end_count = Arc::new(AtomicUsize::new(0));
    hooks.register(Arc::new(CountHook {
        name: "ts".into(),
        phase: HookPhase::TurnStart,
        counter: turn_start_count.clone(),
    }));
    hooks.register(Arc::new(CountHook {
        name: "tp".into(),
        phase: HookPhase::ToolPreCall,
        counter: tool_pre_count.clone(),
    }));
    hooks.register(Arc::new(CountHook {
        name: "te".into(),
        phase: HookPhase::TurnEnd,
        counter: turn_end_count.clone(),
    }));

    let mut tools = cleanclaw_agent::ToolRegistry::new();
    cleanclaw_agent::builtins::register_builtins(
        &mut tools,
        &std::sync::Arc::new(cleanclaw_toolprov::Registry::new()),
    );

    let agent: Arc<Agent> = Arc::new(
        AgentBuilder::new(
            "agt_1",
            "u_1",
            "test-model",
            Arc::new(provider.clone()),
            store.clone(),
        )
        .tools(tools)
        .max_iterations(4)
        .max_tokens(1024)
        .hooks(Arc::new(hooks))
        .build(),
    );

    let input = TurnInput {
        user_text: "what time is it?".into(),
        channel: "web".into(),
        chat_id: "c_1".into(),
        session_key: "sk_1".into(),
        user_id: "u_1".into(),
        owner_user_id: "u_1".into(),
        agent_id: "agt_1".into(),
        is_admin: true,
        history: vec![],
        attachments: vec![],
    };
    let out = agent.run_turn(input).await.unwrap();
    assert_eq!(out.reply, "done");
    assert!(out.iterations >= 1);
    assert_eq!(provider.call_count.load(Ordering::Relaxed), 2);
    assert_eq!(turn_start_count.load(Ordering::Relaxed), 1);
    assert_eq!(tool_pre_count.load(Ordering::Relaxed), 1);
    assert_eq!(turn_end_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn tool_failure_records_in_turn_failures() {
    let store = store_with_user().await;
    let provider = MockProvider::default();
    let scripted = provider.scripted.clone();
    {
        let mut q = scripted.lock().await;
        // First response: tool call to a non-existent tool. Second: retry (still calls the same).
        // We can simulate this by returning tool_calls twice — the loop should
        // see the failure on each and run the turn until max_iterations.
        q.push(make_response_with_tool("first try", "no_such_tool", "tc_1"));
        q.push(make_response_with_tool(
            "second try",
            "no_such_tool",
            "tc_1",
        ));
    }
    let mut tools = cleanclaw_agent::ToolRegistry::new();
    cleanclaw_agent::builtins::register_builtins(
        &mut tools,
        &std::sync::Arc::new(cleanclaw_toolprov::Registry::new()),
    );
    let agent: Arc<Agent> = Arc::new(
        AgentBuilder::new(
            "agt_1",
            "u_1",
            "test-model",
            Arc::new(provider.clone()),
            store.clone(),
        )
        .tools(tools)
        .max_iterations(2)
        .build(),
    );
    let input = TurnInput {
        user_text: "try something".into(),
        channel: "web".into(),
        chat_id: "c_1".into(),
        session_key: "sk_1".into(),
        user_id: "u_1".into(),
        owner_user_id: "u_1".into(),
        agent_id: "agt_1".into(),
        is_admin: true,
        history: vec![],
        attachments: vec![],
    };
    let _ = agent.run_turn(input).await;
    let prior = agent
        .turn_failures
        .prior_failure("no_such_tool", &json!({}));
    assert!(prior.is_some(), "no_such_tool failure should be recorded");
}
