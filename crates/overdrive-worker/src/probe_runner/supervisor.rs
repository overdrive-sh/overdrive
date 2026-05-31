//! Per-alloc supervisor — owns a [`tokio::task::JoinSet`] of per-
//! probe tasks plus the [`tokio_util::sync::CancellationToken`] that
//! cooperatively shuts them down.
//!
//! Per ADR-0054 §2: each allocation gets ONE supervisor; that
//! supervisor spawns ONE task per declared/inferred probe. Tasks
//! observe the supervisor's cancellation token on every `select!`
//! round and exit cooperatively — no [`tokio::task::JoinHandle::abort`]
//! per `.claude/rules/testing.md` § cooperative-shutdown discipline.
//!
//! The supervisor is `Send + Sync` so the `ProbeRunner` can hold it
//! inside a `parking_lot::Mutex<BTreeMap<AllocationId, _>>` and
//! mutate from any async context.

use tokio_util::sync::CancellationToken;

/// Cooperative-shutdown handle for a single per-probe task.
///
/// Phase-1 shape carries only the child-token used to signal
/// cancellation. Phase-2 may extend with a [`tokio::task::JoinHandle`]
/// when the supervisor needs to `await` per-probe completion
/// (currently the parent supervisor's cancellation drains every
/// child cooperatively).
#[derive(Debug)]
pub struct ProbeTaskHandle {
    /// Child token derived from the supervisor's root token.
    /// Dropping the supervisor cancels the root, which propagates to
    /// every child token simultaneously.
    cancel: CancellationToken,
}

impl ProbeTaskHandle {
    /// The child token this task observes. Cloned into the
    /// per-probe `select!` arm — the body checks
    /// `cancel.is_cancelled()` on every loop iteration.
    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

/// Per-alloc supervisor. Owns the root [`CancellationToken`] and a
/// [`JoinSet`] tracking every per-probe task spawned beneath it.
///
/// Cancellation propagates atomically: cancelling the supervisor
/// (via [`Self::cancel`] or via `drop`) cancels every derived child
/// token in the same instant. Task bodies that observe the child
/// token in a `select!` arm exit on the next async yield.
pub struct AllocSupervisor {
    /// Root cancellation token. Owned by the supervisor; every
    /// per-probe task receives a `child_token()` cloned from this.
    /// Phase 2 may extend this struct with a `tokio::task::JoinSet`
    /// when the supervisor needs to `await` per-probe completion;
    /// today cancellation alone is sufficient (probes self-terminate
    /// on cancellation observation).
    root: CancellationToken,
    /// Set to `true` after the first `start_alloc` spawns probe
    /// tasks. Subsequent calls return the existing token without
    /// re-spawning — structural guard against duplicate task sets
    /// writing to the same `(alloc_id, probe_idx)` store keys at
    /// double cadence.
    started: bool,
}

impl AllocSupervisor {
    /// Construct a fresh supervisor with a new root cancellation
    /// token. The supervisor owns no tasks until
    /// [`Self::spawn_probe_task`] is called.
    #[must_use]
    pub fn new() -> Self {
        Self { root: CancellationToken::new(), started: false }
    }

    /// The root cancellation token. Per-probe tasks observe a
    /// `child_token()` cloned from this; cancelling the root cancels
    /// every child in the same instant.
    #[must_use]
    pub fn token(&self) -> CancellationToken {
        self.root.clone()
    }

    /// Register a per-probe task handle so cancellation-derived
    /// child tokens can be issued from a single root. Returns the
    /// handle owning the child token the task observes.
    pub fn spawn_probe_task(&self) -> ProbeTaskHandle {
        ProbeTaskHandle { cancel: self.root.child_token() }
    }

    /// Whether probe tasks have already been spawned under this
    /// supervisor. Used by `start_alloc` to guard against duplicate
    /// task spawning on re-entry.
    pub const fn is_started(&self) -> bool {
        self.started
    }

    /// Mark this supervisor as having spawned its probe tasks.
    pub const fn mark_started(&mut self) {
        self.started = true;
    }

    /// Cancel every per-probe task spawned under this supervisor.
    /// Cooperative — task bodies observe the cancellation on their
    /// next `select!` round.
    pub fn cancel(&self) {
        self.root.cancel();
    }
}

impl Default for AllocSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AllocSupervisor {
    fn drop(&mut self) {
        // Belt-and-braces: every public stop path already calls
        // `cancel()` before the supervisor is removed from the
        // owning map, but a panic on the spawn path could leave a
        // partially-constructed supervisor uncancelled. Cancelling
        // here is idempotent.
        self.root.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_propagates_to_child_tokens() {
        let supervisor = AllocSupervisor::new();
        let handle = supervisor.spawn_probe_task();
        let child = handle.cancellation_token();
        assert!(!child.is_cancelled(), "child token is not cancelled before parent cancel");
        supervisor.cancel();
        assert!(child.is_cancelled(), "child token is cancelled after parent cancel");
    }

    #[test]
    fn drop_propagates_to_child_tokens() {
        let child = {
            let supervisor = AllocSupervisor::new();
            let handle = supervisor.spawn_probe_task();
            handle.cancellation_token()
        };
        // Supervisor dropped at end of inner scope — drop impl
        // cancels the root, which propagates to the surviving
        // child token clone.
        assert!(child.is_cancelled(), "child token is cancelled when supervisor is dropped");
    }
}
