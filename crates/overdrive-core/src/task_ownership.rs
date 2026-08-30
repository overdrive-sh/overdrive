//! Runtime task ownership with an atomic registration/teardown boundary.
//!
//! [`OwnedTaskSet`] is the dependency-neutral ownership primitive for a task
//! tree whose children may be registered by already-running parent tasks. The
//! registry and teardown mode share one lock, so registration cannot race past
//! owner teardown as an unaccounted child.

use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// A cancellation-safe, one-shot async completion fence.
///
/// The first caller starts `work` in an independently owned Tokio task. Every
/// caller, including a replacement after the first caller is cancelled, waits
/// on the same completion notification. The supervisor's drop guard opens the
/// fence even if `work` panics or its task is aborted, so waiters cannot be
/// stranded behind a process-local elected future.
#[derive(Clone)]
pub struct CompletionFence {
    inner: Arc<CompletionFenceInner>,
}

struct CompletionFenceInner {
    started: Mutex<bool>,
    complete: watch::Sender<bool>,
}

impl Default for CompletionFence {
    fn default() -> Self {
        let (complete, _receiver) = watch::channel(false);
        Self { inner: Arc::new(CompletionFenceInner { started: Mutex::new(false), complete }) }
    }
}

struct CompletionGuard(Arc<CompletionFenceInner>);

impl Drop for CompletionGuard {
    fn drop(&mut self) {
        // `send` discards the value when no receiver currently exists. A fast
        // supervisor can finish before its caller reaches `wait`, so retain the
        // terminal value unconditionally for every later subscriber.
        self.0.complete.send_replace(true);
    }
}

impl CompletionFence {
    /// Create an unstarted fence.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Start `work` once in an independently owned supervisor.
    ///
    /// This method is synchronous: once it returns, dropping the caller
    /// cannot prevent the work from running or the fence from opening.
    pub fn start_with<F, Fut>(&self, work: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let starts = {
            let mut started = self.inner.started.lock();
            if *started {
                false
            } else {
                *started = true;
                true
            }
        };
        if starts {
            let inner = Arc::clone(&self.inner);
            tokio::spawn(async move {
                let _completion = CompletionGuard(inner);
                work().await;
            });
        }
    }

    /// Wait until the one-shot work has completed.
    pub async fn wait(&self) {
        let mut complete = self.inner.complete.subscribe();
        while !*complete.borrow() {
            if complete.changed().await.is_err() {
                break;
            }
        }
    }

    /// Start `work` once and wait for its independently owned supervisor.
    pub async fn complete_with<F, Fut>(&self, work: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.start_with(work);
        self.wait().await;
    }
}

/// Owns a dynamically growing set of Tokio tasks until a lifecycle boundary.
///
/// The root owner registers every task that can itself register descendants.
/// [`spawn`](Self::spawn) holds the same short critical section that changes
/// the owner into shutdown mode, so "task was spawned" and "its join handle is
/// owned" are one atomic operation. Once shutdown wins that race, the spawn
/// closure is never invoked.
///
/// [`abort_and_join`](Self::abort_and_join) elects one shutdown leader. Every
/// concurrent caller observes the same completion fence and returns only after
/// all children owned before the fence have been aborted and joined.
#[derive(Clone)]
pub struct OwnedTaskSet {
    inner: Arc<OwnedTaskInner>,
}

struct OwnedTaskInner {
    state: Mutex<OwnedTaskState>,
    shutdown: CompletionFence,
}

#[derive(Default)]
struct OwnedTaskState {
    lifecycle: Lifecycle,
    tasks: Vec<JoinHandle<()>>,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum Lifecycle {
    #[default]
    Open,
    ShuttingDown,
    Shutdown,
}

impl Default for OwnedTaskSet {
    fn default() -> Self {
        Self {
            inner: Arc::new(OwnedTaskInner {
                state: Mutex::new(OwnedTaskState::default()),
                shutdown: CompletionFence::new(),
            }),
        }
    }
}

impl Drop for OwnedTaskInner {
    fn drop(&mut self) {
        for task in self.state.get_mut().tasks.drain(..) {
            task.abort();
        }
    }
}

impl OwnedTaskSet {
    /// Create an open task owner.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the owner has crossed its no-new-children shutdown fence.
    #[must_use]
    pub fn is_shutdown(&self) -> bool {
        self.inner.state.lock().lifecycle != Lifecycle::Open
    }

    /// Atomically spawn and register a task under this owner.
    ///
    /// Returns `true` when the closure was invoked and the handle registered.
    /// Returns `false` once shutdown has started; in that case the closure is
    /// not invoked, so no unowned late child can escape the completion fence.
    pub fn spawn(&self, spawn: impl FnOnce() -> JoinHandle<()>) -> bool {
        let mut state = self.inner.state.lock();
        if state.lifecycle != Lifecycle::Open {
            return false;
        }
        state.tasks.push(spawn());
        true
    }

    /// Seal the owner, abort all tracked tasks, and join them to completion.
    pub async fn abort_and_join(&self) {
        let leader_tasks = {
            let mut state = self.inner.state.lock();
            match state.lifecycle {
                Lifecycle::Open => {
                    state.lifecycle = Lifecycle::ShuttingDown;
                    Some(std::mem::take(&mut state.tasks))
                }
                Lifecycle::ShuttingDown | Lifecycle::Shutdown => None,
            }
        };
        if let Some(tasks) = leader_tasks {
            let inner = Arc::clone(&self.inner);
            self.inner.shutdown.start_with(move || async move {
                for task in &tasks {
                    task.abort();
                }
                for task in tasks {
                    let _ = task.await;
                }
                inner.state.lock().lifecycle = Lifecycle::Shutdown;
            });
        }
        self.inner.shutdown.wait().await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::{CompletionFence, OwnedTaskSet};

    struct DropWitness(Arc<AtomicBool>);

    impl Drop for DropWitness {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// CONTRACT_SHAPE: bounded-change (completion before the first waiter is durably observable).
    #[tokio::test]
    async fn fast_completion_before_subscription_does_not_lose_the_fence_signal() {
        let fence = CompletionFence::new();
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        fence.start_with(move || async move {
            finished_tx.send(()).expect("test receives work completion");
        });
        finished_rx.await.expect("supervisor completes before wait subscribes");

        tokio::time::timeout(Duration::from_secs(1), fence.wait())
            .await
            .expect("late waiter observes retained completion");
    }

    /// CONTRACT_SHAPE: bounded-change (one owned task is aborted, joined, and dropped).
    #[tokio::test]
    async fn abort_and_join_is_a_task_completion_fence() {
        let dropped = Arc::new(AtomicBool::new(false));
        let witness = DropWitness(Arc::clone(&dropped));
        let tasks = OwnedTaskSet::new();
        assert!(tasks.spawn(|| {
            tokio::spawn(async move {
                let _witness = witness;
                std::future::pending::<()>().await;
            })
        }));

        tokio::time::timeout(Duration::from_secs(1), tasks.abort_and_join())
            .await
            .expect("owned task shutdown must complete");

        assert!(dropped.load(Ordering::SeqCst));
    }

    /// CONTRACT_SHAPE: bounded-change (a child registration racing shutdown either belongs to the fence or never spawns).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_atomically_rejects_a_late_child_before_spawn() {
        let tasks = OwnedTaskSet::new();
        let tasks_for_parent = tasks.clone();
        let runtime = tokio::runtime::Handle::current();
        let child_spawned = Arc::new(AtomicBool::new(false));
        let child_spawned_for_parent = Arc::clone(&child_spawned);
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();

        assert!(tasks.spawn(|| {
            tokio::task::spawn_blocking(move || {
                started_tx.send(()).expect("parent reports it has started");
                release_rx.recv().expect("test releases the ending parent");
                let _ = tasks_for_parent.spawn(|| {
                    child_spawned_for_parent.store(true, Ordering::SeqCst);
                    runtime.spawn(std::future::pending::<()>())
                });
            })
        }));
        started_rx.recv_timeout(Duration::from_secs(1)).expect("parent starts");

        let shutdown = tokio::spawn({
            let tasks = tasks.clone();
            async move { tasks.abort_and_join().await }
        });
        loop {
            if tasks.inner.state.lock().lifecycle == super::Lifecycle::ShuttingDown {
                break;
            }
            tokio::task::yield_now().await;
        }
        release_tx.send(()).expect("release ending parent");

        tokio::time::timeout(Duration::from_secs(1), shutdown)
            .await
            .expect("shutdown must include the late child")
            .expect("shutdown task must join");
        assert!(!child_spawned.load(Ordering::SeqCst));
        assert!(tasks.inner.state.lock().tasks.is_empty());
    }

    /// CONTRACT_SHAPE: bounded-change (all concurrent shutdown callers share one completion fence).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_abort_and_join_callers_wait_for_the_same_blocking_child() {
        let tasks = OwnedTaskSet::new();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        assert!(tasks.spawn(|| {
            tokio::task::spawn_blocking(move || {
                started_tx.send(()).expect("blocking child reports start");
                release_rx.recv().expect("test releases blocking child");
            })
        }));
        started_rx.recv_timeout(Duration::from_secs(1)).expect("blocking child starts");

        let first = tokio::spawn({
            let tasks = tasks.clone();
            async move { tasks.abort_and_join().await }
        });
        tokio::task::yield_now().await;
        let mut second = tokio::spawn({
            let tasks = tasks.clone();
            async move { tasks.abort_and_join().await }
        });

        let second_returned_before_child =
            tokio::time::timeout(Duration::from_millis(25), &mut second).await.is_ok();
        release_tx.send(()).expect("release blocking child");
        first.await.expect("first shutdown joins");
        if !second_returned_before_child {
            second.await.expect("second shutdown joins");
        }

        assert!(
            !second_returned_before_child,
            "a concurrent shutdown caller must not return before the first caller's child joins"
        );
    }

    /// CONTRACT_SHAPE: bounded-change (cancelling the elected caller cannot orphan the shared shutdown fence).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_shutdown_leader_does_not_strand_replacement_waiter() {
        let tasks = OwnedTaskSet::new();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        assert!(tasks.spawn(|| {
            tokio::task::spawn_blocking(move || {
                started_tx.send(()).expect("blocking child reports start");
                release_rx.recv().expect("test releases blocking child");
            })
        }));
        started_rx.recv_timeout(Duration::from_secs(1)).expect("blocking child starts");

        let leader = tokio::spawn({
            let tasks = tasks.clone();
            async move { tasks.abort_and_join().await }
        });
        while tasks.inner.state.lock().lifecycle != super::Lifecycle::ShuttingDown {
            tokio::task::yield_now().await;
        }
        leader.abort();
        let mut replacement = tokio::spawn({
            let tasks = tasks.clone();
            async move { tasks.abort_and_join().await }
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(25), &mut replacement).await.is_err(),
            "replacement must still wait for the owned child"
        );
        release_tx.send(()).expect("release blocking child");
        tokio::time::timeout(Duration::from_secs(1), replacement)
            .await
            .expect("replacement observes independently-owned completion")
            .expect("replacement task joins");
    }

    /// CONTRACT_SHAPE: bounded-change (registration after shutdown never invokes the spawn closure).
    #[tokio::test]
    async fn registration_after_shutdown_does_not_spawn_a_child() {
        let tasks = OwnedTaskSet::new();
        tasks.abort_and_join().await;
        let invoked = Arc::new(AtomicBool::new(false));
        let invoked_for_spawn = Arc::clone(&invoked);
        assert!(!tasks.spawn(|| {
            invoked_for_spawn.store(true, Ordering::SeqCst);
            tokio::spawn(std::future::pending::<()>())
        }));
        assert!(!invoked.load(Ordering::SeqCst));
    }
}
