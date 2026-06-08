//! End-to-end smoke test for the SQLite store. Exercises every domain
//! (users, web sessions, API keys, agents, sessions, session messages,
//! configs, projects, goals, cron jobs, channel leases, token usage).

use chrono::Utc;
use cleanclaw_core::{AgentId, ApiKeyId, ProjectId, SessionKey, UserId};
use cleanclaw_store::sqlite::SqliteStore;
use cleanclaw_store::store::{StorageConfig, StorageType, Store};
use std::sync::Arc;
use std::time::Duration;

async fn fresh() -> Arc<SqliteStore> {
    let st = SqliteStore::open(":memory:").await.expect("open");
    st.migrate().await.expect("migrate");
    Arc::new(st)
}

fn make_user(uid: &str, username: &str) -> cleanclaw_store::models::UserRecord {
    cleanclaw_store::models::UserRecord {
        id: uid.to_string(),
        username: username.to_string(),
        email: format!("{username}@example.com"),
        password_hash: "argon2id$...".into(),
        display_name: username.to_string(),
        role: "user".into(),
        status: "active".into(),
        apikey_id: String::new(),
        external_id: String::new(),
        avatar_url: String::new(),
        agent_quota: -1,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[tokio::test]
async fn users_crud_and_idempotent_upserts() {
    let st = fresh().await;
    let u = make_user("u_alice", "alice");
    st.create_user(&u).await.unwrap();
    let got = st.get_user("u_alice").await.unwrap();
    assert_eq!(got.username, "alice");
    assert_eq!(st.count_users().await.unwrap(), 1);

    // dup username → conflict
    let mut dup = make_user("u_bob", "alice");
    dup.email = "another@example.com".into();
    assert!(st.create_user(&dup).await.is_err());
}

#[tokio::test]
async fn web_sessions_round_trip() {
    let st = fresh().await;
    st.create_user(&make_user("u_alice", "alice"))
        .await
        .unwrap();
    let sess = cleanclaw_store::models::WebSessionRecord {
        sid: "s_abc".into(),
        user_id: "u_alice".into(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::days(7),
    };
    st.create_web_session(&sess).await.unwrap();
    let got = st.get_web_session("s_abc").await.unwrap();
    assert_eq!(got.user_id, "u_alice");
}

#[tokio::test]
async fn api_key_lifecycle() {
    let st = fresh().await;
    st.create_user(&make_user("u_alice", "alice"))
        .await
        .unwrap();
    let k = cleanclaw_store::models::ApiKeyRecord {
        id: "k1".into(),
        user_id: "u_alice".into(),
        name: "test".into(),
        key_hash: "h".into(),
        key_prefix: "fk_".into(),
        r#type: "user".into(),
        created_at: chrono::Utc::now(),
        prev_hash: None,
        prev_hash_set_at: None,
    };
    st.create_api_key(&k).await.unwrap();
    let keys = st.list_api_keys("u_alice").await.unwrap();
    assert_eq!(keys.len(), 1);
    st.rotate_api_key("k1", "newhash", "newprefix")
        .await
        .unwrap();
    let got = st.get_api_key("k1").await.unwrap();
    assert_eq!(got.key_hash, "newhash");
    assert_eq!(got.key_prefix, "newprefix");
    let by_hash = st.lookup_api_key_by_hash("newhash").await.unwrap();
    assert_eq!(by_hash.id, "k1");
    // apikey_agents M:N
    st.set_api_key_agents("k1", &["agt_1".into(), "agt_2".into()])
        .await
        .unwrap();
    let agents = st.list_api_key_agents("k1").await.unwrap();
    assert_eq!(agents.len(), 2);
    st.delete_api_key("k1").await.unwrap();
    assert!(st.get_api_key("k1").await.is_err());
}

#[tokio::test]
async fn agents_and_workspace_files() {
    let st = fresh().await;
    st.create_user(&make_user("u_alice", "alice"))
        .await
        .unwrap();
    let a = cleanclaw_store::models::AgentRecord {
        id: "agt_1".into(),
        user_id: "u_alice".into(),
        name: "alpha".into(),
        config: serde_json::json!({"model": "openai/gpt-4o-mini"}),
        is_public: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    st.save_agent(&a).await.unwrap();
    let got = st.get_agent("agt_1").await.unwrap();
    assert_eq!(got.name, "alpha");
    assert_eq!(got.config["model"], "openai/gpt-4o-mini");

    // workspace files: owner-only then owner-fallback
    st.save_workspace_file("agt_1", "", "SOUL.md", b"# SOUL")
        .await
        .unwrap();
    st.save_workspace_file("agt_1", "u_alice", "MEMORY.md", b"chatter memory")
        .await
        .unwrap();
    let files = st.list_workspace_files("agt_1").await.unwrap();
    assert!(files.contains(&"SOUL.md".to_string()));
    assert!(files.contains(&"MEMORY.md".to_string()));

    let (row_user, content) = st
        .get_workspace_file("agt_1", "u_alice", "MEMORY.md")
        .await
        .unwrap();
    assert_eq!(row_user, "u_alice");
    assert_eq!(content, b"chatter memory");

    let (row_user, content) = st
        .get_workspace_file("agt_1", "u_alice", "SOUL.md")
        .await
        .unwrap();
    assert_eq!(row_user, ""); // fell through to owner
    assert_eq!(content, b"# SOUL");
}

#[tokio::test]
async fn sessions_and_message_archive() {
    let st = fresh().await;
    st.create_user(&make_user("u_alice", "alice"))
        .await
        .unwrap();
    let mut s = cleanclaw_store::models::SessionRecord {
        user_id: "u_alice".into(),
        agent_id: "agt_1".into(),
        session_key: "sk_1".into(),
        channel: "web".into(),
        account_id: "u_alice".into(),
        chat_id: "c_1".into(),
        project_id: "".into(),
        title: "First chat".into(),
        messages: serde_json::json!([{"role":"user","content":"hi"}]),
        message_count: 1,
        updated_at: Utc::now(),
        chatter_user_id: "u_alice".into(),
    };
    st.save_session("u_alice", "agt_1", "sk_1", &s)
        .await
        .unwrap();
    s.title = "renamed".into();
    st.save_session("u_alice", "agt_1", "sk_1", &s)
        .await
        .unwrap();
    let got = st.get_session("u_alice", "agt_1", "sk_1").await.unwrap();
    assert_eq!(got.title, "renamed");

    // append-only messages get monotonic seq
    let m1 = cleanclaw_store::models::SessionMessageRecord {
        user_id: "u_alice".into(),
        agent_id: "agt_1".into(),
        session_key: "sk_1".into(),
        seq: 0,
        role: "user".into(),
        content: "hi".into(),
        content_parts: serde_json::json!([]),
        tool_calls: serde_json::json!([]),
        tool_call_id: "".into(),
        name: "".into(),
        metadata: serde_json::json!({}),
        thinking: "".into(),
        raw_assistant: serde_json::json!({}),
        origin: "".into(),
        created_at: Utc::now(),
        chatter_user_id: "u_alice".into(),
    };
    let m2 = cleanclaw_store::models::SessionMessageRecord {
        role: "assistant".into(),
        content: "hello".into(),
        ..m1.clone()
    };
    let s1 = st.append_session_message(&m1).await.unwrap();
    let s2 = st.append_session_message(&m2).await.unwrap();
    assert_eq!(s1, 0);
    assert_eq!(s2, 1);
    let msgs = st
        .list_session_messages("u_alice", "agt_1", "sk_1")
        .await
        .unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[1].content, "hello");
}

#[tokio::test]
async fn configs_scope_tagged() {
    let st = fresh().await;
    st.create_user(&make_user("u_alice", "alice"))
        .await
        .unwrap();
    let rec = cleanclaw_store::models::ConfigRecord {
        id: "cfg_1".into(),
        kind: "provider".into(),
        scope: "user".into(),
        user_id: "u_alice".into(),
        agent_id: "".into(),
        name: "openai".into(),
        enabled: true,
        credential_key: "".into(),
        data: serde_json::json!({"apiKey": "sk-test", "apiBase": "https://api.openai.com/v1"}),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    st.save_config(&rec).await.unwrap();
    let got = st
        .get_config("provider", "u_alice", "", "openai")
        .await
        .unwrap();
    assert_eq!(got.data["apiKey"], "sk-test");

    let list = st.list_configs("provider", "u_alice", "").await.unwrap();
    assert_eq!(list.len(), 1);
    st.delete_config("provider", "u_alice", "", "openai")
        .await
        .unwrap();
    assert!(st
        .get_config("provider", "u_alice", "", "openai")
        .await
        .is_err());
}

#[tokio::test]
async fn cron_jobs_lifecycle() {
    let st = fresh().await;
    st.create_user(&make_user("u_alice", "alice"))
        .await
        .unwrap();
    let j = cleanclaw_store::models::CronJobRecord {
        id: "cj_1".into(),
        user_id: "u_alice".into(),
        agent_id: "agt_1".into(),
        name: "reminder".into(),
        r#type: "cron".into(),
        schedule: "0 9 * * *".into(),
        message: "喝口水".into(),
        channel: "web".into(),
        chat_id: "c_1".into(),
        account_id: "u_alice".into(),
        timezone: "UTC".into(),
        enabled: true,
        last_run: None,
        next_run: Some(Utc::now() - chrono::Duration::seconds(1)),
        locked_by: None,
        locked_at: None,
        failure_count: 0,
        created_at: Utc::now(),
    };
    st.save_cron_job(&j).await.unwrap();
    let due = st
        .list_due_cron_jobs(Utc::now().timestamp(), 100)
        .await
        .unwrap();
    assert!(due.iter().any(|x| x.id == "cj_1"));
    st.delete_cron_job("cj_1").await.unwrap();
}

#[tokio::test]
async fn channel_lease_acquire_renew_release() {
    let st = fresh().await;
    let acq1 = st
        .try_acquire_channel_lease("telegram", "bot1", "holder-A", Duration::from_secs(60))
        .await
        .unwrap();
    let acq2 = st
        .try_acquire_channel_lease("telegram", "bot1", "holder-B", Duration::from_secs(60))
        .await
        .unwrap();
    assert!(acq1);
    assert!(!acq2, "second holder should lose to active lease");

    st.renew_channel_lease("telegram", "bot1", "holder-A", Duration::from_secs(60))
        .await
        .unwrap();

    st.release_channel_lease("telegram", "bot1", "holder-A")
        .await
        .unwrap();
    let acq3 = st
        .try_acquire_channel_lease("telegram", "bot1", "holder-B", Duration::from_secs(60))
        .await
        .unwrap();
    assert!(acq3);
}

#[tokio::test]
async fn token_usage_upsert_aggregates() {
    let st = fresh().await;
    st.create_user(&make_user("u_alice", "alice"))
        .await
        .unwrap();
    let day = chrono::NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let mut r = cleanclaw_store::models::TokenUsageRecord {
        day,
        user_id: "u_alice".into(),
        agent_id: "agt_1".into(),
        session_key: "sk_1".into(),
        provider: "anthropic".into(),
        model: "claude-sonnet-4-6".into(),
        input_tokens: 100,
        output_tokens: 50,
        cache_read_tokens: 0,
        cache_create_tokens: 0,
        request_count: 1,
    };
    st.upsert_token_usage(&r).await.unwrap();
    r.input_tokens = 200;
    r.output_tokens = 100;
    r.request_count = 1;
    st.upsert_token_usage(&r).await.unwrap();
    let rows = st.list_token_usage(day).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].input_tokens, 300);
    assert_eq!(rows[0].output_tokens, 150);
    assert_eq!(rows[0].request_count, 2);
}

#[tokio::test]
async fn factory_opens_sqlite_with_migration() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = StorageConfig {
        r#type: StorageType::Sqlite,
        dsn: String::new(),
        auto_migrate: true,
    };
    let st = cleanclaw_store::open(&cfg, dir.path()).await.unwrap();
    let _: Box<dyn Store> = st;
}

#[allow(dead_code)]
fn _ids_compile() {
    let _ = UserId::generate();
    let _ = AgentId::generate();
    let _ = SessionKey::generate();
    let _ = ApiKeyId::generate();
    let _ = ProjectId::generate();
}

#[tokio::test]
async fn lookup_channel_by_credential_finds_user_row() {
    use cleanclaw_store::models::ConfigRecord;
    let st = fresh().await;
    let now = Utc::now();
    // A user-scope channel row with a credential_key set.
    let rec = ConfigRecord {
        id: "cfg_ch_alice_tg".into(),
        kind: "channel".into(),
        scope: "user".into(),
        user_id: "u_alice".into(),
        agent_id: "tg_bot_alice".into(),
        name: "telegram".into(),
        enabled: true,
        credential_key: "bot_alice_tg".into(),
        data: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    };
    st.save_config(&rec).await.unwrap();

    let got = st
        .lookup_channel_by_credential("telegram", "bot_alice_tg")
        .await
        .unwrap();
    let got = got.expect("user-scoped row must be findable");
    assert_eq!(got.user_id, "u_alice");
    assert_eq!(got.agent_id, "tg_bot_alice");
}

#[tokio::test]
async fn lookup_channel_by_credential_finds_system_row_when_credential_empty() {
    use cleanclaw_store::models::ConfigRecord;
    let st = fresh().await;
    let now = Utc::now();
    // A system-scope channel row (user_id="", empty credential).
    let rec = ConfigRecord {
        id: "cfg_ch_sys_slack".into(),
        kind: "channel".into(),
        scope: "system".into(),
        user_id: String::new(),
        agent_id: "slack_main".into(),
        name: "slack".into(),
        enabled: true,
        credential_key: String::new(),
        data: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    };
    st.save_config(&rec).await.unwrap();

    // A blank credential_key in the lookup still finds the
    // system row — the SQL OR-matches both shapes.
    let got = st.lookup_channel_by_credential("slack", "").await.unwrap();
    let got = got.expect("system-scoped row must be findable");
    assert!(got.user_id.is_empty());
    assert_eq!(got.agent_id, "slack_main");
}

#[tokio::test]
async fn lookup_channel_by_credential_returns_none_for_unknown() {
    let st = fresh().await;
    let got = st
        .lookup_channel_by_credential("telegram", "no-such-bot")
        .await
        .unwrap();
    assert!(got.is_none());
}
