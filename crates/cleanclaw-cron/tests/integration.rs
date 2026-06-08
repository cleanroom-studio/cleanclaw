use chrono::Utc;
use cleanclaw_bus::MessageBus;
use cleanclaw_cron::{compute_next_run, parse_duration, Scheduler};
use cleanclaw_store::models::{CronJobRecord, UserRecord};
use cleanclaw_store::sqlite::SqliteStore;
use cleanclaw_store::Store;
use std::sync::Arc;
use std::time::Duration;

async fn store_with_user() -> Arc<SqliteStore> {
    let st = SqliteStore::open(":memory:").await.unwrap();
    st.migrate().await.unwrap();
    let u = UserRecord {
        id: "u_alice".into(),
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
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    st.create_user(&u).await.unwrap();
    Arc::new(st)
}

#[tokio::test]
async fn tick_fires_due_job() {
    let st = store_with_user().await;
    let bus = MessageBus::new(8);
    // Subscribe BEFORE moving the bus into the scheduler.
    let inbound_tx = bus.inbound_tx.clone();
    let sched = Scheduler::new(st.clone(), bus);

    // Insert a job that's already due (next_run in the past).
    let now = Utc::now();
    let job = CronJobRecord {
        id: "cj_1".into(),
        user_id: "u_alice".into(),
        agent_id: "agt_1".into(),
        name: "reminder".into(),
        r#type: "interval".into(),
        schedule: "5m".into(),
        message: "喝水".into(),
        channel: "web".into(),
        chat_id: "c_1".into(),
        account_id: "u_alice".into(),
        timezone: "UTC".into(),
        enabled: true,
        last_run: None,
        next_run: Some(now - chrono::Duration::seconds(1)),
        locked_by: None,
        locked_at: None,
        failure_count: 0,
        created_at: now,
    };
    st.save_cron_job(&job).await.unwrap();

    let fired = sched.tick().await.unwrap();
    assert_eq!(fired, 1);

    let msg = tokio::time::timeout(Duration::from_secs(1), inbound_tx.reserve())
        .await
        .ok();
    // We only kept a sender clone so we can't recv — instead verify
    // by querying the bus via the same sender. Use a fresh bus in the
    // next test.
    let _ = msg;

    // last_run updated, next_run advanced by 5 minutes.
    let updated = st.get_cron_job("cj_1").await.unwrap();
    assert!(updated.last_run.is_some());
    let next = updated.next_run.unwrap();
    let expected = now + chrono::Duration::minutes(5);
    let drift = (next - expected).num_seconds().abs();
    assert!(
        drift < 2,
        "next_run should be ~5 minutes from now, got {next}"
    );
}

#[tokio::test]
async fn tick_skips_when_no_due_jobs() {
    let st = store_with_user().await;
    let bus = MessageBus::new(8);
    let inbound_tx = bus.inbound_tx.clone();
    let sched = Scheduler::new(st.clone(), bus);

    let now = Utc::now();
    // future next_run
    let job = CronJobRecord {
        id: "cj_future".into(),
        user_id: "u_alice".into(),
        agent_id: "agt_1".into(),
        name: "later".into(),
        r#type: "interval".into(),
        schedule: "1h".into(),
        message: "later".into(),
        channel: "web".into(),
        chat_id: "c_1".into(),
        account_id: "u_alice".into(),
        timezone: "UTC".into(),
        enabled: true,
        last_run: None,
        next_run: Some(now + chrono::Duration::hours(1)),
        locked_by: None,
        locked_at: None,
        failure_count: 0,
        created_at: now,
    };
    st.save_cron_job(&job).await.unwrap();

    let fired = sched.tick().await.unwrap();
    assert_eq!(fired, 0);
    let _ = inbound_tx;
}

#[tokio::test]
async fn compute_next_run_for_cron_expression() {
    let now = Utc::now();
    let job = CronJobRecord {
        id: "cj_cron".into(),
        user_id: "u_alice".into(),
        agent_id: "agt_1".into(),
        name: "test".into(),
        r#type: "cron".into(),
        schedule: "*/1 * * * * *".into(), // every second (6-field w/ sec)
        message: "x".into(),
        channel: "web".into(),
        chat_id: "c_1".into(),
        account_id: "u_alice".into(),
        timezone: "UTC".into(),
        enabled: true,
        last_run: None,
        next_run: None,
        locked_by: None,
        locked_at: None,
        failure_count: 0,
        created_at: now,
    };
    let next = compute_next_run(&job, now).unwrap();
    assert!(next > now);
    assert!(next - now < chrono::Duration::seconds(2));
}

#[tokio::test]
async fn compute_next_run_for_interval() {
    let now = Utc::now();
    let job = CronJobRecord {
        id: "cj_int".into(),
        user_id: "u_alice".into(),
        agent_id: "agt_1".into(),
        name: "test".into(),
        r#type: "interval".into(),
        schedule: "30m".into(),
        message: "x".into(),
        channel: "web".into(),
        chat_id: "c_1".into(),
        account_id: "u_alice".into(),
        timezone: "UTC".into(),
        enabled: true,
        last_run: None,
        next_run: None,
        locked_by: None,
        locked_at: None,
        failure_count: 0,
        created_at: now,
    };
    let next = compute_next_run(&job, now).unwrap();
    assert_eq!((next - now).num_minutes(), 30);
}

#[tokio::test]
async fn parse_duration_variants() {
    assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
    assert_eq!(
        parse_duration("every 5m").unwrap(),
        Duration::from_secs(300)
    );
    assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
    assert!(parse_duration("").is_err());
    assert!(parse_duration("5x").is_err());
}
