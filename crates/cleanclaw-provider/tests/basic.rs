use cleanclaw_provider::{ChatRequest, Message, Role, ToolDefinition};
use serde_json::json;

#[test]
fn message_constructors() {
    let s = Message::system("you are helpful");
    assert_eq!(s.role, Role::System);
    assert!(s.tool_calls.is_empty());

    let u = Message::user("hi");
    assert_eq!(u.role, Role::User);

    let t = Message::tool_result("call_1", "ok");
    assert_eq!(t.role, Role::Tool);
    assert_eq!(t.tool_call_id.as_deref(), Some("call_1"));
}

#[test]
fn chat_request_serialization_round_trip() {
    let req = ChatRequest {
        model: "gpt-4o-mini".into(),
        messages: vec![Message::system("sys"), Message::user("hi")],
        tools: vec![ToolDefinition {
            name: "echo".into(),
            description: "Echo a string".into(),
            parameters: json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"]
            }),
        }],
        temperature: Some(0.5),
        max_tokens: Some(1024),
        top_p: None,
        stop: vec![],
        stream: false,
        extra: Default::default(),
    };
    let s = serde_json::to_string(&req).unwrap();
    assert!(s.contains("gpt-4o-mini"));
    assert!(s.contains("echo"));
    assert!(s.contains("\"temperature\":0.5"));
}

#[test]
fn factory_builds_openai_for_unknown_type() {
    let cfg = cleanclaw_config::ProviderConfig {
        api_key: "sk-test".into(),
        api_base: "https://api.openai.com/v1".into(),
        ..Default::default()
    };
    let p = cleanclaw_provider::build_provider("openai", &cfg).unwrap();
    assert_eq!(p.name(), "openai");

    let cfg2 = cleanclaw_config::ProviderConfig {
        api_key: "sk-test".into(),
        api_base: "https://api.example.com/v1".into(),
        api_type: "openai".into(),
        ..Default::default()
    };
    let p2 = cleanclaw_provider::build_provider("whatever", &cfg2).unwrap();
    assert_eq!(p2.name(), "openai");
}

#[test]
fn factory_builds_anthropic() {
    let cfg = cleanclaw_config::ProviderConfig {
        api_key: "sk-test".into(),
        api_type: "anthropic".into(),
        ..Default::default()
    };
    let p = cleanclaw_provider::build_provider("anthropic", &cfg).unwrap();
    assert_eq!(p.name(), "anthropic");
}

#[test]
fn factory_resolves_env_var_api_key() {
    std::env::set_var("CLEANCLAW_TEST_KEY", "from-env");
    let cfg = cleanclaw_config::ProviderConfig {
        api_key: "$CLEANCLAW_TEST_KEY".into(),
        ..Default::default()
    };
    let p = cleanclaw_provider::build_provider("openai", &cfg).unwrap();
    assert_eq!(p.name(), "openai");
    std::env::remove_var("CLEANCLAW_TEST_KEY");
}
