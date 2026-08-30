//! Runtime task ownership with an atomic registration/teardown boundary.
//!
//! [`OwnedTaskSet`] is the dependency-neutral ownership primitive for a task
//! tree whose children may be registered by already-running parent tasks. The
//! registry and teardown mode share one lock, so registration cannot race past
//! owner teardown as an unaccounted child.

use std::sync::Arc;

use parking_lot::Mutex;
use tokio::task::JoinHandle;

/// Owns a dynamically growing set of Tokio tasks until a lifecycle boundary.
///
/// The root owner registers every task that can itself register descendants.
/// [`abort_and_join`](Self::abort_and_join) first seals registration into
/// abort-on-arrival mode, aborts and joins the current roots, and then drains
/// descendants registered while those roots were ending. Once the registered
/// roots have joined, no conforming producer remains, so an empty final drain
/// is the deterministic completion fence.
///
/// [`detach`](Self::detach) is the explicit cooperative-stop alternative. It
/// drops current and future join handles without aborting their tasks; callers
/// must separately signal those tasks to finish.
#[derive(Clone, Default)]
pub struct OwnedTaskSet {
    inner: Arc<Mutex<OwnedTaskState>>,
}

#[derive(Default)]
struct OwnedTaskState {
    teardown: Teardown,
    tasks: Vec<JoinHandle<()>>,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum Teardown {
    #[default]
    Open,
    Detach,
    AbortAndJoin,
}

impl OwnedTaskSet {
    /// Create an open task owner.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a task under this owner.
    ///
    /// Registration after [`detach`](Self::detach) immediately detaches the
    /// task. Registration during [`abort_and_join`](Self::abort_and_join)
    /// immediately aborts the task and retains its handle for the final join
    /// drain.
    pub fn track(&self, task: JoinHandle<()>) {
        let mut state = self.inner.lock();
        match state.teardown {
            Teardown::Open => state.tasks.push(task),
            Teardown::Detach => drop(task),
            Teardown::AbortAndJoin => {
                task.abort();
                state.tasks.push(task);
            }
        }
    }

    /// Seal the owner for cooperative stop and detach every tracked task.
    ///
    /// This method does not cancel tasks. The caller must first signal its
    /// domain-specific cooperative stop condition. Calls after the first are
    /// idempotent.
    pub fn detach(&self) {
        let tasks = {
            let mut state = self.inner.lock();
            if state.teardown == Teardown::AbortAndJoin {
                return;
            }
            state.teardown = Teardown::Detach;
            std::mem::take(&mut state.tasks)
        };
        drop(tasks);
    }

    /// Seal the owner, abort all tracked tasks, and join the complete task tree.
    ///
    /// Parent tasks may register a final child while responding to shutdown.
    /// Such children are aborted on registration and joined by the subsequent
    /// drain. Callers must register every task-producing parent in this set.
    pub async fn abort_and_join(&self) {
        let mut tasks = {
            let mut state = self.inner.lock();
            state.teardown = Teardown::AbortAndJoin;
            std::mem::take(&mut state.tasks)
        };

        loop {
            for task in &tasks {
                task.abort();
            }
            for task in tasks {
                let _ = task.await;
            }

            tasks = {
                let mut state = self.inner.lock();
                std::mem::take(&mut state.tasks)
            };
            if tasks.is_empty() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::OwnedTaskSet;

    struct DropWitness(Arc<AtomicBool>);

    impl Drop for DropWitness {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// CONTRACT_SHAPE: bounded-change (one owned task is aborted, joined, and dropped).
    #[tokio::test]
    async fn abort_and_join_is_a_task_completion_fence() {
        let dropped = Arc::new(AtomicBool::new(false));
        let witness = DropWitness(Arc::clone(&dropped));
        let tasks = OwnedTaskSet::new();
        tasks.track(tokio::spawn(async move {
            let _witness = witness;
            std::future::pending::<()>().await;
        }));

        tokio::time::timeout(Duration::from_secs(1), tasks.abort_and_join())
            .await
            .expect("owned task shutdown must complete");

        assert!(dropped.load(Ordering::SeqCst));
    }

    /// CONTRACT_SHAPE: bounded-change (a child registered during parent shutdown is aborted and joined).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn abort_and_join_drains_a_child_registered_while_a_parent_ends() {
        let tasks = OwnedTaskSet::new();
        let tasks_for_parent = tasks.clone();
        let runtime = tokio::runtime::Handle::current();
        let child_dropped = Arc::new(AtomicBool::new(false));
        let child_dropped_for_parent = Arc::clone(&child_dropped);
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();

        tasks.track(tokio::task::spawn_blocking(move || {
            started_tx.send(()).expect("parent reports it has started");
            release_rx.recv().expect("test releases the ending parent");
            let witness = DropWitness(child_dropped_for_parent);
            tasks_for_parent.track(runtime.spawn(async move {
                let _witness = witness;
                std::future::pending::<()>().await;
            }));
        }));
        started_rx.recv_timeout(Duration::from_secs(1)).expect("parent starts");

        let shutdown = tokio::spawn({
            let tasks = tasks.clone();
            async move { tasks.abort_and_join().await }
        });
        loop {
            if tasks.inner.lock().teardown == super::Teardown::AbortAndJoin {
                break;
            }
            tokio::task::yield_now().await;
        }
        release_tx.send(()).expect("release ending parent");

        tokio::time::timeout(Duration::from_secs(1), shutdown)
            .await
            .expect("shutdown must include the late child")
            .expect("shutdown task must join");
        assert!(child_dropped.load(Ordering::SeqCst));
        assert!(tasks.inner.lock().tasks.is_empty());
    }

    /// CONTRACT_SHAPE: bounded-change (cooperative detach never cancels the child task).
    #[tokio::test]
    async fn detach_drops_ownership_without_aborting_the_task() {
        let tasks = OwnedTaskSet::new();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        tasks.track(tokio::spawn(async move {
            let _ = release_rx.await;
            let _ = finished_tx.send(());
        }));

        tasks.detach();
        release_tx.send(()).expect("detached task still receives its signal");
        tokio::time::timeout(Duration::from_secs(1), finished_rx)
            .await
            .expect("detached task must remain runnable")
            .expect("detached task must report completion");
    }
}
