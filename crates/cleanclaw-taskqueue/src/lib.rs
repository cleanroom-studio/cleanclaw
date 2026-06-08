//! Per-chat FIFO task queue with global concurrency limit.
//!
//!

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use cleanclaw_bus::InboundMessage;
use thiserror::Error;
use tokio::sync::{mpsc, Mutex, Semaphore};
use tokio::time::Instant;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskStatus {
    Pending,
    Running,
    Done,
    Failed,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Running => "running",
            TaskStatus::Done => "done",
            TaskStatus::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub agent_id: String,
    pub owner_user_id: String,
    pub chat_key: String,
    pub message: InboundMessage,
    pub account_id: String,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub done_at: Option<DateTime<Utc>>,
    pub result: String,
    pub error: Option<String>,
}

#[derive(Debug, Error)]
pub enum TaskError {
    #[error("queue shut down")]
    Shutdown,
    #[error("handler error: {0}")]
    Handler(String),
    #[error("timeout after {0:?}")]
    Timeout(Duration),
}

impl TaskError {
    pub fn handler(msg: impl Into<String>) -> Self {
        TaskError::Handler(msg.into())
    }
}

/// Type-erased task handler. Callers can pass plain `Fn(Task) -> impl
/// Future<...>` closures; the `handler()` helper boxes them up.
pub type BoxedTaskHandler =
    Arc<dyn Fn(Task) -> Pin<Box<dyn Future<Output = Result<String, TaskError>> + Send>> + Send + Sync>;

/// Helper to turn a plain async closure into a `BoxedTaskHandler`.
pub fn handler<F, Fut>(f: F) -> BoxedTaskHandler
where
    F: Fn(Task) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<String, TaskError>> + Send + 'static,
{
    Arc::new(move |t| {
        let fut = f(t);
        Box::pin(fut)
    })
}

const IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const CHAT_QUEUE_CAPACITY: usize = 100;
const TASK_RETENTION: usize = 200;

struct ChatQueue {
    tx: mpsc::Sender<Task>,
    last_used: Arc<Mutex<Instant>>,
}

struct QueueState {
    tasks: HashMap<String, Task>,
    chat_queues: HashMap<String, ChatQueue>,
}

pub struct Queue {
    inner: Arc<Inner>,
}

struct Inner {
    max_concurrent: usize,
    task_timeout: Duration,
    sem: Arc<Semaphore>,
    handler: BoxedTaskHandler,
    state: Mutex<QueueState>,
    seq: AtomicU64,
    shutdown: AtomicBool,
}

impl Queue {
    pub fn new(
        max_concurrent: usize,
        task_timeout: Duration,
        handler: BoxedTaskHandler,
    ) -> Arc<Self> {
        let mc = if max_concurrent <= 0 { 10 } else { max_concurrent };
        let tt = if task_timeout.is_zero() {
            Duration::from_secs(5 * 60)
        } else {
            task_timeout
        };
        let inner = Arc::new(Inner {
            max_concurrent: mc,
            task_timeout: tt,
            sem: Arc::new(Semaphore::new(mc)),
            handler,
            state: Mutex::new(QueueState {
                tasks: HashMap::new(),
                chat_queues: HashMap::new(),
            }),
            seq: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
        });
        let q = Arc::new(Self { inner: inner.clone() });
        // Kick off idle cleanup loop.
        let cleanup_inner = inner.clone();
        let shutdown = Arc::new(AtomicBool::new(false));
        // Reuse the inner shutdown signal; this local one is just for
        // breaking the spawned cleanup future if `stop` fires.
        let cleanup_handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(60));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                if cleanup_inner.shutdown.load(Ordering::Relaxed) {
                    break;
                }
                cleanup_idle(cleanup_inner.clone()).await;
            }
        });
        // Stash the cleanup handle for join() — we leak it because the
        // queue is rarely recreated; this matches the Go behavior of
        // firing-and-forgetting the cleanup goroutine.
        std::mem::forget(cleanup_handle);
        let _ = shutdown; // silence unused
        q
    }

    pub fn max_concurrent(&self) -> usize {
        self.inner.max_concurrent
    }

    pub fn task_timeout(&self) -> Duration {
        self.inner.task_timeout
    }

    /// Submit a new task. Returns the task id; the task is dispatched
    /// onto its chat's FIFO and runs when its turn comes up.
    pub async fn submit(
        &self,
        agent_id: &str,
        chat_key: &str,
        msg: InboundMessage,
        account_id: &str,
    ) -> String {
        let now = Utc::now();
        let seq = self.inner.seq.fetch_add(1, Ordering::Relaxed);
        let task_id = format!("task-{:x}-{:x}", now.timestamp_millis(), seq);
        let task = Task {
            id: task_id.clone(),
            agent_id: agent_id.to_string(),
            owner_user_id: msg.owner_user_id.clone(),
            chat_key: chat_key.to_string(),
            message: msg,
            account_id: account_id.to_string(),
            status: TaskStatus::Pending,
            created_at: now,
            started_at: None,
            done_at: None,
            result: String::new(),
            error: None,
        };

        // Lock state, possibly create a new chatQueue, push task.
        let tx = {
            let mut st = self.inner.state.lock().await;
            st.tasks.insert(task_id.clone(), task.clone());
            if !st.chat_queues.contains_key(chat_key) {
                let (tx, rx) = mpsc::channel::<Task>(CHAT_QUEUE_CAPACITY);
                let last_used = Arc::new(Mutex::new(Instant::now()));
                let cq = ChatQueue {
                    tx: tx.clone(),
                    last_used: last_used.clone(),
                };
                st.chat_queues.insert(chat_key.to_string(), cq);
                // Spawn the per-chat drainer.
                let drainer_inner = self.inner.clone();
                tokio::spawn(chat_drainer(
                    drainer_inner,
                    chat_key.to_string(),
                    rx,
                    last_used,
                ));
            }
            let cq = st.chat_queues.get(chat_key).expect("just inserted");
            cq.tx.clone()
        };
        let depth = CHAT_QUEUE_CAPACITY - tx.capacity() + 1;

        info!(
            task_id = %task_id,
            chat_key = %chat_key,
            agent_id = %agent_id,
            queue_depth = depth,
            "task submitted"
        );
        if depth > 100 {
            warn!(chat_key = %chat_key, depth, "queue depth high");
        }

        if tx.send(task).await.is_err() {
            warn!(task_id = %task_id, "chat queue receiver gone before send");
        }
        task_id
    }

    /// Snapshot of recent tasks, newest first, capped at `limit`.
    /// `limit == 0` means "all".
    pub async fn recent_tasks(&self, limit: usize) -> Vec<Task> {
        let mut all: Vec<Task> = {
            let st = self.inner.state.lock().await;
            st.tasks.values().cloned().collect()
        };
        all.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        if limit > 0 && all.len() > limit {
            all.truncate(limit);
        }
        let len = self.inner.state.lock().await.tasks.len();
        if len > TASK_RETENTION {
            let me = self.inner.clone();
            tokio::spawn(async move { prune_old_tasks(me).await });
        }
        all
    }

    /// Mark the queue as shutting down. Existing tasks finish; new
    /// submits are still accepted but their drainers will exit on
    /// the next iteration.
    pub fn stop(&self) {
        self.inner.shutdown.store(true, Ordering::Relaxed);
    }

    pub fn is_shutdown(&self) -> bool {
        self.inner.shutdown.load(Ordering::Relaxed)
    }
}

async fn chat_drainer(
    inner: Arc<Inner>,
    chat_key: String,
    mut rx: mpsc::Receiver<Task>,
    last_used: Arc<Mutex<Instant>>,
) {
    while let Some(task) = rx.recv().await {
        if inner.shutdown.load(Ordering::Relaxed) {
            break;
        }
        execute_task(inner.clone(), task).await;
        *last_used.lock().await = Instant::now();
    }
    debug!(chat_key = %chat_key, "chat drainer exiting");
}

async fn execute_task(inner: Arc<Inner>, mut task: Task) {
    let permit = match inner.sem.clone().acquire_owned().await {
        Ok(p) => p,
        Err(_) => return,
    };

    let now = Utc::now();
    {
        let mut st = inner.state.lock().await;
        if let Some(t) = st.tasks.get_mut(&task.id) {
            t.status = TaskStatus::Running;
            t.started_at = Some(now);
        }
        task.status = TaskStatus::Running;
        task.started_at = Some(now);
    }
    let concurrent = inner.max_concurrent - inner.sem.available_permits();
    info!(
        task_id = %task.id,
        agent_id = %task.agent_id,
        chat_key = %task.chat_key,
        concurrent_count = concurrent,
        "task started"
    );

    let handler = inner.handler.clone();
    let timeout = inner.task_timeout;
    let task_for_handler = task.clone();
    let res = tokio::time::timeout(timeout, async move {
        (handler)(task_for_handler).await
    })
    .await;

    let done_at = Utc::now();
    let duration_ms = task
        .started_at
        .map(|s| (done_at - s).num_milliseconds())
        .unwrap_or(0);

    let (status, err_text) = match res {
        Ok(Ok(out)) => {
            task.result = out;
            (TaskStatus::Done, None)
        }
        Ok(Err(e)) => {
            task.error = Some(e.to_string());
            (TaskStatus::Failed, Some(e.to_string()))
        }
        Err(_) => {
            let msg = format!("timeout after {:?}", timeout);
            task.error = Some(msg.clone());
            (TaskStatus::Failed, Some(msg))
        }
    };
    task.status = status;
    task.done_at = Some(done_at);
    {
        let mut st = inner.state.lock().await;
        if let Some(t) = st.tasks.get_mut(&task.id) {
            *t = task.clone();
        }
    }

    match status {
        TaskStatus::Done => {
            info!(
                task_id = %task.id,
                agent_id = %task.agent_id,
                chat_key = %task.chat_key,
                duration_ms,
                "task completed"
            );
        }
        _ => {
            error!(
                task_id = %task.id,
                agent_id = %task.agent_id,
                chat_key = %task.chat_key,
                duration_ms,
                error = err_text.as_deref().unwrap_or(""),
                "task failed"
            );
        }
    }
    drop(permit);
}

async fn cleanup_idle(inner: Arc<Inner>) {
    let now = Instant::now();
    let mut to_remove: Vec<String> = Vec::new();
    {
        let st = inner.state.lock().await;
        for (key, cq) in st.chat_queues.iter() {
            let lu = *cq.last_used.lock().await;
            if now.duration_since(lu) > IDLE_TIMEOUT && cq.tx.capacity() == CHAT_QUEUE_CAPACITY {
                to_remove.push(key.clone());
            }
        }
    }
    if to_remove.is_empty() {
        return;
    }
    let mut st = inner.state.lock().await;
    for key in to_remove {
        st.chat_queues.remove(&key);
        debug!(chat_key = %key, "idle chat queue removed");
    }
}

async fn prune_old_tasks(inner: Arc<Inner>) {
    let mut st = inner.state.lock().await;
    if st.tasks.len() <= TASK_RETENTION {
        return;
    }
    let mut completed: Vec<(String, DateTime<Utc>)> = st
        .tasks
        .iter()
        .filter(|(_, t)| matches!(t.status, TaskStatus::Done | TaskStatus::Failed))
        .map(|(id, t)| (id.clone(), t.created_at))
        .collect();
    completed.sort_by_key(|(_, ts)| *ts);
    let to_remove = st.tasks.len() - TASK_RETENTION;
    for (id, _) in completed.iter().take(to_remove) {
        st.tasks.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn inbound(text: &str) -> InboundMessage {
        let mut m = InboundMessage::default();
        m.channel = "test".into();
        m.chat_id = "c1".into();
        m.text = text.into();
        m.user_id = "u1".into();
        m.owner_user_id = "u1".into();
        m
    }

    #[tokio::test]
    async fn submits_and_runs_to_completion() {
        let handler = handler(|t: Task| async move {
            Ok::<String, TaskError>(format!("echo:{}", t.message.text))
        });
        let q = Queue::new(2, Duration::from_secs(2), handler);
        let id = q.submit("a1", "test:c1", inbound("hello"), "acc").await;
        let mut done = false;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let st = q.inner.state.lock().await;
            if let Some(t) = st.tasks.get(&id) {
                if matches!(t.status, TaskStatus::Done) {
                    assert_eq!(t.result, "echo:hello");
                    done = true;
                    break;
                }
            }
        }
        assert!(done, "task did not reach Done state in time");
        q.stop();
    }

    #[tokio::test]
    async fn per_chat_serialization() {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));
        let i = in_flight.clone();
        let m = max_seen.clone();
        let handler = handler(move |_t: Task| {
            let i = i.clone();
            let m = m.clone();
            async move {
                let now = i.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
                i.fetch_sub(1, Ordering::SeqCst);
                Ok::<String, TaskError>("ok".into())
            }
        });
        let q = Queue::new(4, Duration::from_secs(2), handler);
        let mut ids = Vec::new();
        for n in 0..5 {
            let id = q
                .submit("a1", "test:c1", inbound(&format!("m{}", n)), "acc")
                .await;
            ids.push(id);
        }
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let st = q.inner.state.lock().await;
            if ids.iter().all(|id| {
                matches!(st.tasks.get(id).map(|t| t.status), Some(TaskStatus::Done))
            }) {
                break;
            }
        }
        assert_eq!(
            max_seen.load(Ordering::SeqCst),
            1,
            "per-chat serialization violated"
        );
        q.stop();
    }

    #[tokio::test]
    async fn global_concurrency_caps() {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));
        let i = in_flight.clone();
        let m = max_seen.clone();
        let handler = handler(move |_t: Task| {
            let i = i.clone();
            let m = m.clone();
            async move {
                let now = i.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(40)).await;
                i.fetch_sub(1, Ordering::SeqCst);
                Ok::<String, TaskError>("ok".into())
            }
        });
        let q = Queue::new(2, Duration::from_secs(2), handler);
        for n in 0..4 {
            q.submit(
                "a1",
                &format!("test:c{}", n),
                inbound(&format!("m{}", n)),
                "acc",
            )
            .await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(max_seen.load(Ordering::SeqCst), 2, "global cap violated");
        q.stop();
    }

    #[tokio::test]
    async fn timeout_marks_task_failed() {
        let handler = handler(|_t: Task| async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok::<String, TaskError>("ok".into())
        });
        let q = Queue::new(1, Duration::from_millis(50), handler);
        let id = q.submit("a1", "test:c1", inbound("slow"), "acc").await;
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let st = q.inner.state.lock().await;
            if let Some(t) = st.tasks.get(&id) {
                if matches!(t.status, TaskStatus::Failed) {
                    assert!(t.error.as_deref().unwrap_or("").contains("timeout"));
                    q.stop();
                    return;
                }
            }
        }
        panic!("task did not reach Failed (timeout) state in time");
    }

    #[tokio::test]
    async fn recent_tasks_sorted_newest_first() {
        let handler = handler(|_t: Task| async { Ok::<String, TaskError>("ok".into()) });
        let q = Queue::new(2, Duration::from_secs(2), handler);
        for n in 0..3 {
            q.submit("a1", &format!("test:c{}", n), inbound("m"), "acc")
                .await;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
        let recent = q.recent_tasks(10).await;
        assert!(recent.len() >= 3);
        for w in recent.windows(2) {
            assert!(w[0].created_at >= w[1].created_at, "not sorted newest first");
        }
        q.stop();
    }

    #[test]
    fn task_status_as_str_round_trip() {
        use crate::TaskStatus;
        assert_eq!(TaskStatus::Pending.as_str(), "pending");
        assert_eq!(TaskStatus::Running.as_str(), "running");
        assert_eq!(TaskStatus::Done.as_str(), "done");
        assert_eq!(TaskStatus::Failed.as_str(), "failed");
    }

    #[test]
    fn task_error_display_includes_kind() {
        use crate::TaskError;
        let e = TaskError::Handler("boom".into());
        assert!(e.to_string().contains("boom"));
        let e = TaskError::Timeout(Duration::from_secs(2));
        assert!(e.to_string().contains("timeout"));
    }

    #[test]
    fn task_error_is_send_sync() {
        fn assert_send<T: Send + Sync>() {}
        assert_send::<TaskError>();
    }
}
