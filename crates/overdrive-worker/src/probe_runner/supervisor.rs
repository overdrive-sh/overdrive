//! Per-alloc supervisor — owns the root
//! [`tokio_util::sync::CancellationToken`] that cooperatively shuts
//! down every per-probe task, plus one intermediate token per
//! [`ProbeRole`] so a single role can be stopped independently.
//!
//! Per ADR-0054 §2: each allocation gets ONE supervisor; that
//! supervisor spawns ONE task per declared/inferred probe. Tasks
//! observe their cancellation token on every `select!` round and exit
//! cooperatively — no [`tokio::task::JoinHandle::abort`] per
//! `.claude/rules/testing.md` § cooperative-shutdown discipline. The
//! supervisor holds no [`tokio::task::JoinSet`]; cancellation alone is
//! sufficient because every probe task self-terminates on observing
//! its token.
//!
//! Per ADR-0080 § D4 the token graph is two levels deep —
//! `root → per-role → per-task` — so `Stable` (a NON-terminal
//! condition per ADR-0055) can retire only the startup role while
//! readiness and liveness keep ticking. Cancelling `root` still
//! cancels every role transitively, so the whole-supervisor teardown
//! stays atomic.
//!
//! The supervisor is `Send + Sync` so the `ProbeRunner` can hold it
//! inside a `parking_lot::Mutex<BTreeMap<AllocationId, _>>` and
//! mutate from any async context.

use std::collections::BTreeMap;

use overdrive_core::observation::ProbeRole;
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
    /// Child token derived from the task's ROLE token, which is
    /// itself a child of the supervisor's root token (ADR-0080 § D4).
    /// Cancelling the role cancels only that role's tasks; dropping
    /// or cancelling the supervisor cancels the root, which propagates
    /// through every role token to every task simultaneously.
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

/// Per-alloc supervisor. Owns the root [`CancellationToken`] plus one
/// intermediate token per [`ProbeRole`]; holds no [`tokio::task::JoinSet`]
/// (cancellation alone drains every probe task cooperatively).
///
/// Whole-supervisor cancellation propagates atomically: cancelling the
/// supervisor (via [`Self::cancel`] or via `drop`) cancels the root,
/// which propagates through every role token to every task in the same
/// instant. Task bodies that observe their token in a `select!` arm
/// exit on the next async yield.
pub struct AllocSupervisor {
    /// Root cancellation token. Owned by the supervisor; every role
    /// token is a `child_token()` of this, and every per-probe task's
    /// token is in turn a child of its role's token.
    root: CancellationToken,
    /// Per-role intermediate tokens, each derived from `root` via
    /// `child_token()` and created on first use (ADR-0080 § D4). A
    /// probe task's token is a child of ITS ROLE's token, so
    /// cancelling one role cancels only that role's tasks, while
    /// cancelling `root` still cancels every role in the same instant.
    ///
    /// `BTreeMap`, not `HashMap`, per `.claude/rules/development.md`
    /// § "Ordered-collection choice" — the map is iterated by
    /// [`Self::is_role_live`]'s sibling diagnostics and by DST
    /// observation.
    per_role: BTreeMap<ProbeRole, CancellationToken>,
    /// Set to `true` after the first `start_alloc` spawns probe
    /// tasks. Subsequent calls return the existing token without
    /// re-spawning — structural guard against duplicate task sets
    /// writing to the same `(alloc_id, role, probe_idx)` store keys at
    /// double cadence. Load-bearing for per-role cancellation: a
    /// cancelled role token can never parent a newly-spawned task,
    /// because no second spawn round happens.
    started: bool,
}

impl AllocSupervisor {
    /// Construct a fresh supervisor with a new root cancellation
    /// token. The supervisor owns no tasks until
    /// [`Self::spawn_probe_task`] is called.
    #[must_use]
    pub fn new() -> Self {
        Self { root: CancellationToken::new(), per_role: BTreeMap::new(), started: false }
    }

    /// The root cancellation token. Per-probe tasks observe a
    /// `child_token()` cloned from this; cancelling the root cancels
    /// every child in the same instant.
    #[must_use]
    pub fn token(&self) -> CancellationToken {
        self.root.clone()
    }

    /// Register a per-probe task handle for a task of `role`. Returns
    /// the handle owning the token the task observes — a child of
    /// `role`'s intermediate token, created on first use, which is
    /// itself a child of the supervisor's root.
    ///
    /// Per ADR-0080 § D4 the role level is what makes [`Self::cancel_role`]
    /// possible without a task collection.
    pub fn spawn_probe_task(&mut self, role: ProbeRole) -> ProbeTaskHandle {
        // The fresh child is built BEFORE the `entry` borrow so
        // `self.root` and `self.per_role` are never borrowed at once.
        // A `CancellationToken` that loses the `or_insert` race is
        // simply dropped — an un-awaited token has no side effect.
        let fresh = self.root.child_token();
        let role_token = self.per_role.entry(role).or_insert(fresh).clone();
        ProbeTaskHandle { cancel: role_token.child_token() }
    }

    /// Cancel every task of `role`, leaving other roles and the
    /// supervisor itself running.
    ///
    /// Cooperative shutdown only — task bodies observe the
    /// cancellation on their next `select!` round. Idempotent; a role
    /// with no tasks (hence no token) is a no-op.
    pub fn cancel_role(&self, role: ProbeRole) {
        if let Some(token) = self.per_role.get(&role) {
            token.cancel();
        }
    }

    /// Whether `role` has a live (created, un-cancelled) token.
    ///
    /// Reports TOKEN liveness, not task liveness. That is a valid
    /// proxy only because `supervised_probe_loop` has exactly one exit
    /// — its `child_token.cancelled()` arm — so a live token implies a
    /// live task. This coupling is load-bearing for the ADR-0080 § D7
    /// item-4 regression guard; if the loop ever gains a second exit,
    /// this observable must be revisited.
    #[must_use]
    pub fn is_role_live(&self, role: ProbeRole) -> bool {
        self.per_role.get(&role).is_some_and(|token| !token.is_cancelled())
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

    /// Cancel every per-probe task spawned under this supervisor,
    /// across every role. Cooperative — task bodies observe the
    /// cancellation on their next `select!` round.
    ///
    /// Cancelling the root propagates through every role token
    /// transitively, so whole-supervisor teardown remains atomic
    /// under the ADR-0080 § D4 two-level token graph.
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
        let mut supervisor = AllocSupervisor::new();
        let handle = supervisor.spawn_probe_task(ProbeRole::Startup);
        let child = handle.cancellation_token();
        assert!(!child.is_cancelled(), "child token is not cancelled before parent cancel");
        supervisor.cancel();
        assert!(child.is_cancelled(), "child token is cancelled after parent cancel");
    }

    #[test]
    fn drop_propagates_to_child_tokens() {
        let child = {
            let mut supervisor = AllocSupervisor::new();
            let handle = supervisor.spawn_probe_task(ProbeRole::Startup);
            handle.cancellation_token()
        };
        // Supervisor dropped at end of inner scope — drop impl
        // cancels the root, which propagates through the role token
        // to the surviving grandchild token clone.
        assert!(child.is_cancelled(), "child token is cancelled when supervisor is dropped");
    }

    /// ADR-0080 § D4 — `cancel_role` retires exactly one role's tasks
    /// and leaves every other role's tasks running. This is the unit
    /// under `Stable`: startup probing is complete at Stable while
    /// readiness and liveness are continuous post-Stable per
    /// `ProbeRole`'s own contract.
    #[test]
    fn cancel_role_retires_only_that_roles_tasks() {
        let mut supervisor = AllocSupervisor::new();
        let startup = supervisor.spawn_probe_task(ProbeRole::Startup).cancellation_token();
        let readiness = supervisor.spawn_probe_task(ProbeRole::Readiness).cancellation_token();
        let liveness = supervisor.spawn_probe_task(ProbeRole::Liveness).cancellation_token();

        supervisor.cancel_role(ProbeRole::Startup);

        assert!(startup.is_cancelled(), "the cancelled role's task token is cancelled");
        assert!(!readiness.is_cancelled(), "readiness survives a startup-role cancellation");
        assert!(!liveness.is_cancelled(), "liveness survives a startup-role cancellation");
        assert!(!supervisor.is_role_live(ProbeRole::Startup));
        assert!(supervisor.is_role_live(ProbeRole::Readiness));
        assert!(supervisor.is_role_live(ProbeRole::Liveness));
    }

    /// Two tasks of the SAME role share one intermediate token, so a
    /// single `cancel_role` retires both. Guards the multi-probe-per-role
    /// shape (probes 1..N of a role are spawned today even though no
    /// decision consults them — ADR-0080 § "A fourth, pre-existing gap").
    #[test]
    fn cancel_role_retires_every_task_of_that_role() {
        let mut supervisor = AllocSupervisor::new();
        let first = supervisor.spawn_probe_task(ProbeRole::Readiness).cancellation_token();
        let second = supervisor.spawn_probe_task(ProbeRole::Readiness).cancellation_token();

        supervisor.cancel_role(ProbeRole::Readiness);

        assert!(first.is_cancelled(), "first readiness task cancelled");
        assert!(second.is_cancelled(), "second readiness task cancelled");
    }

    /// Cancelling the root still cancels every role transitively —
    /// the atomicity claim the struct docstring makes.
    #[test]
    fn root_cancel_propagates_through_every_role_token() {
        let mut supervisor = AllocSupervisor::new();
        let startup = supervisor.spawn_probe_task(ProbeRole::Startup).cancellation_token();
        let readiness = supervisor.spawn_probe_task(ProbeRole::Readiness).cancellation_token();

        supervisor.cancel();

        assert!(startup.is_cancelled());
        assert!(readiness.is_cancelled());
        assert!(!supervisor.is_role_live(ProbeRole::Startup));
        assert!(!supervisor.is_role_live(ProbeRole::Readiness));
    }

    /// A role that never spawned a task has no token; cancelling it
    /// and querying its liveness are both no-ops (the idempotence the
    /// `cancel_role` contract promises).
    #[test]
    fn cancel_role_for_undeclared_role_is_a_noop() {
        let mut supervisor = AllocSupervisor::new();
        let readiness = supervisor.spawn_probe_task(ProbeRole::Readiness).cancellation_token();

        supervisor.cancel_role(ProbeRole::Liveness);
        supervisor.cancel_role(ProbeRole::Liveness);

        assert!(!supervisor.is_role_live(ProbeRole::Liveness), "undeclared role is never live");
        assert!(!readiness.is_cancelled(), "an undeclared role's cancel touches nothing else");
    }
}
