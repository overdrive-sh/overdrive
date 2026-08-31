# ADR-0073: Backend instance replacement — `overdrive workload restart` + a minimal desired-run generation precursor

## Status

Accepted (2026-06-29). **Revised 2026-06-29 (post-DESIGN-review iteration 1)** —
resolved the iteration-1 review's one Critical + three findings: the generation
bump is now atomic + monotonic via a NEW `TxnOp::IncrementU64` store primitive
(Critical; the prior read-then-`Put`-retry-on-`Conflict` relied on a conflict
the store cannot produce); `RestartOutcome` is singularly pinned with its
classification source + race semantics (Finding 2 — no residual open question);
the running-origin state machine is pinned as a transition table (Finding 4);
the dead `Conflict` variant is noted (forward-looking RaftStore surface).
**Revised 2026-06-30 (post-DESIGN-review iteration 2)** — resolved the
iteration-2 Critical: the cardinality contract was over-claimed as
*non-idempotent / each call → a fresh instance* while the state machine
(stamp `observed = desired` on placement) *coalesces* multiple pre-placement
bumps into one placement. The contract is now stated coherently as
**level-triggered coalescing** everywhere (Option B): generation advances
monotonically per call (audited); the reconciler converges to **one** fresh
instance for the latest generation; *sequential* restarts each cycle the
workload, *concurrent / pre-placement* restarts coalesce. The mechanism is
unchanged (stamp `observed = desired`); only the prose, the R5 note, the
`RestartOutcome` discussion, and the concurrency test assertion were corrected
to match. See § "Idempotency posture: level-triggered coalescing".
**Revised 2026-06-30 (post-DESIGN-review iteration 3)** — resolved the
iteration-3 Critical: the iteration-2 reconciler gate vetoed on
`any(is_operator_stopped)` across ALL alloc history, so a SUPERSEDED prior-
generation `Terminated{Operator}` row (the `payments-0` `mint_alloc_id`
deliberately retains to achieve `A1 ≠ A2`) re-armed the veto after the fresh
instance was placed — wedging the fresh instance's later crash forever (the
`is_restartable` crash-restart branch sits after the veto and was never reached).
The veto is now **scoped to the workload's CURRENT instance** (the latest-placed
alloc — the numerically-highest `mint_alloc_id` attempt index, via a pure
`current_alloc` helper): it fires only when the *current* instance is itself
operator-stopped, so a superseded prior-generation row can never veto. A new
R1-crash row is added to the R1–R5 table (post-restart fresh-alloc failure →
`RestartAllocation`, NOT veto), the stale-row-does-not-veto invariant is made
explicit, and a REGRESSION acceptance case (deploy → stop → restart → fresh
instance Running → fresh instance crashes → asserts crash-restart of the fresh
instance, both stopped-origin and running-origin) is added to the verification
plan as a mandatory mutation target. NO rkyv `AllocStatusRow` schema change was
needed (the current-instance signal reuses the existing alloc-id suffix
monotonicity — no per-row `generation` field, no ADR-0048 envelope bump). See
§ 5 → "Why the veto must be scoped to the current instance". Closes the
`[D1]` hard gate of the `backend-instance-replacement` DISCUSS wave (GH #249).
Forward-compatible with — and a deliberate down-scope of — the revision-lineage
model deferred to #180 (per ADR-0050 OQ-1, "Option β").

## Context

### The gap (`[D1]`)

Overdrive has no production verb to **replace a declared workload's backend
instance** — end the current instance (`A1`, `workload_addr`) and bring up a
fresh one (`A2`, new `workload_addr`) while the `workloads/<id>` intent stays
declared. The three existing paths each do something else: `overdrive job
stop` writes the sticky `workloads/<id>/stop` sentinel (suspend, no
counterpart `start`/`restart`); crash-restart (`RestartAllocation`) reuses the
alloc-id/slot (no new identity); deletion (#211) withdraws intent (the
opposite operation).

DISCUSS decided the *mechanism class* (an explicit lifecycle verb; `overdrive
deploy` stays pure-declare — the Kubernetes `apply` vs `rollout restart`
split) and handed DESIGN the **verb name + semantics + HTTP surface +
response/error shape + sentinel/generation mechanics + the reconciler edit +
this ADR** as a hard gate.

### The line-520 blocker (grounded; verified against live code)

`crates/overdrive-core/src/reconcilers/workload_lifecycle.rs:520`:

```rust
if allocs_vec.iter().any(|r| is_operator_stopped(r)) {
    return (Vec::new(), view.clone());
}
```

This veto iterates **all** alloc rows and reads the **observed**
`AllocStatusRow` (`state == Terminated` AND `terminal`/`reason` carrying
`Stopped { by: Operator }`). It is distinct from the **Stop branch**
(`workload_lifecycle.rs:377`, `desired.desired_to_stop` from the
`workloads/<id>/stop` sentinel). The asymmetry (verified): a `SystemGc`-stopped
row is **overridable** (filtered out of `active_allocs_vec`, line 477), so a
resubmit lands a fresh placement; an `Operator`-stopped row is **overriding**
(short-circuits the Run branch) — the `fix-exec-driver-exit-watcher` Bug-3
invariant, so a same-spec re-deploy does NOT resurrect an operator-stopped
workload.

Consequence (the reason the reconciler edit is mandatory): clearing the
`workloads/<id>/stop` sentinel is **necessary but NOT sufficient**. After a
clear, the Stop branch stops firing, but the observed `payments-0`
Terminated/Operator row persists in obs and line 520 keeps vetoing the fresh
placement. The replace path needs a signal that tells the Run branch "this
observed Operator-stop is superseded — place a fresh instance."

The fix is **two-fold** (§ 5): (a) the generation gate makes the veto
overridable *while a restart is pending*; and (b) — the post-iteration-2
correction — the veto is **scoped to the workload's CURRENT instance**, never the
`any(...)`-over-all-history form shown above. The `any(...)` form is the *bug*:
once the fresh instance is placed and `restart_pending` flips false, the retained
superseded `payments-0 / Operator` row would re-arm the veto and wedge the fresh
instance's later crash. The pinned gate keys off `current_alloc(&allocs_vec)`
(the latest-placed instance) only — see § 5 → "Why the veto must be scoped to the
current instance."

### No-sentinel direct-operator-stop path is absent (verified)

The only production writer of `StoppedBy::Operator` is the reconciler's Stop
branch → `Action::StopAllocation { terminal: Operator }` → action-shim →
`Driver::stop` sets `intentional_stop` → `exit_observer::classify` stamps
`Terminated { by: Operator }`. The code comment at `workload_lifecycle.rs:502-505`
anticipates a "direct CLI/API operator action" that stops without a sentinel,
but **it is not built** — no verb emits it. Therefore an Operator-stopped row
in obs implies the sentinel was once present, and the generation gate below
can treat that observed row as overridable when the generation advances.
Assumption documented; DELIVER must not introduce a no-sentinel Operator-stop
path without revisiting this gate.

## Decision

Ship a new top-level **`overdrive workload restart <id>`** verb (a new
`workload` subcommand namespace, aligned with #220's planned `workload
describe <id>`). It bumps a minimal **desired-run `generation: u64`** intent
record; the `WorkloadLifecycle` reconciler places a fresh instance when
`observed_generation < generation`. The generation comparison — not a standing
read of a past `Stopped{Operator}` observation — drives placement. This is the
**generation-precursor** mechanism: it pulls forward ONLY the minimal
`generation`/`observed_generation` seam from #180's revision-lineage model
(see § Forward-compat), and aligns its vocabulary to #180's
`workloads/<id>/current` pointer.

### Semantics (single verb, rollout-restart breadth)

`overdrive workload restart <id>`, for a **declared** `workloads/<id>`:

- **running** → stop-then-start a fresh instance (end-then-bring-up; a brief
  no-live-instance gap is acceptable — confirmed the right v1 cut by #253);
- **operator-stopped** → just start a fresh instance (the sentinel is cleared,
  the generation bump overrides the observed Operator-stop veto);
- **non-existent** (`no workloads/<id>` row) → honest **404** (same posture as
  `stop_workload`), not a silent no-op.

`overdrive deploy` stays **pure-declare**: a same-spec deploy `put_if_absent`s
`workloads/<id>`, a no-op when present, and **does NOT bump the generation** —
so it cannot resurrect an operator-stopped workload (Bug 3 preserved).

## The six pinned signatures

### 1. CLI surface — new `workload` namespace

`crates/overdrive-cli/src/cli.rs` — add a `Command::Workload(WorkloadCommand)`
arm and the enum:

```rust
/// Workload lifecycle — restart (and, per #220, describe).
#[command(subcommand)]
Workload(WorkloadCommand),

#[derive(Debug, Subcommand)]
pub enum WorkloadCommand {
    /// Replace a declared workload's backend instance with a fresh one.
    /// Rollout-restart breadth: running → stop-then-start; stopped →
    /// start. Intent stays declared. 404 if the workload was never deployed.
    Restart { id: String },
}
```

Command handler in `crates/overdrive-cli/src/commands/` (new module
`workload.rs`, sibling to `deploy.rs`; the `Restart` handler does NOT live on
`deploy`):

```rust
/// Arguments to [`restart`]. Mirrors `StopArgs`.
#[derive(Debug, Clone)]
pub struct RestartArgs {
    pub id: String,
    pub config_path: PathBuf,
}

/// Typed output of `overdrive workload restart`.
#[derive(Debug, Clone)]
pub struct RestartOutput {
    pub workload_id: String,
    pub outcome: RestartOutcome,   // re-exported from overdrive_control_plane::api
    pub endpoint: Url,
}

pub async fn restart(args: RestartArgs) -> Result<RestartOutput, CliError>;
```

`RestartOutcome` (the API enum, item 2) carries the rollout-restart breadth as
the operator-observable distinction:

```rust
#[serde(rename_all = "snake_case")]
pub enum RestartOutcome {
    /// The workload was running (or never operator-stopped); its instance
    /// was ended and a fresh one will be placed (generation bumped + any
    /// stop sentinel cleared).
    Restarted,
    /// The workload was operator-stopped; a fresh instance will be placed
    /// (generation bumped + sentinel cleared).
    Resumed,
}
```

**The two-variant decision is PINNED, not deferred (resolves review Finding 2).**
Both variants ship. The classification source and its race semantics are pinned
precisely — there is NO residual open question about whether the handler labels
the outcome:

- **The label is classified from the single check-exists read in step 2 of the
  handler sequence** (§ item 6), BEFORE the bump transaction — NOT from a
  separate observation read, and NOT from the increment's result. In the same
  point-in-time read that fetches `workloads/<id>` for the 404 gate, the handler
  also reads `workloads/<id>/stop`:
  - `/stop` **present** at that read ⇒ `Resumed` (the workload was
    operator-stopped — the sentinel was on file).
  - `/stop` **absent** at that read ⇒ `Restarted` (the workload was not
    operator-stopped; rollout-restart of a running/declared instance).
- **Race semantics.** The label is a *cosmetic, best-effort* report of the
  origin the handler observed; the *placement* decision is the reconciler's
  generation gate, never the label. A `/stop` written by a converging stop
  *between* the check-exists read and the bump txn does not change the label
  (it was read once, before the mutation) and does not change correctness: the
  bump txn deletes `/stop` atomically regardless, and the reconciler places a
  fresh instance on the generation advance either way. The label therefore
  cannot wedge or mislabel placement — at worst it reports `Restarted` for a
  workload that an in-flight stop was about to suspend, which is the honest
  view from the read the handler actually took.
- **Coalescing-loser semantics.** `RestartOutcome` stays `{ Restarted, Resumed }`
  — both variants are correct under coalescing, because a concurrent restart
  that *coalesces* (its bump advanced `desired` but its placement merged with a
  peer's) still got the workload cycled to a fresh instance for the latest
  generation. A concurrent loser therefore returns its outcome **truthfully**:
  the workload was (or is being) cycled. Neither variant promises a *distinct*
  instance per call — only that the workload was cycled toward the latest
  desired generation, which is exactly what the level-triggered contract
  guarantees.

(The `/stop` read for labelling is folded into step 2's existence read — one
read transaction, two key lookups — so it adds no extra round-trip and no extra
TOCTOU surface.)

**Idempotency posture: level-triggered coalescing (decided — see the dedicated
subsection below).** `restart` advances `desired.generation` **monotonically**
(the `IncrementU64` bump; no lost bump), and the reconciler **converges to
exactly one fresh instance for the latest `desired.generation`**. *Sequential*
restarts (each issued after the prior placement, so `observed` has caught up)
each yield a fresh instance — the normal operator loop. *Concurrent / rapid
pre-placement* restarts **coalesce** into a single fresh instance for the
latest generation. There is no `AlreadyRestarted` no-op variant; the 404 path
is the only refusal. (This is a different posture from `stop`'s
`Stopped`/`AlreadyStopped` idempotency — stop is a sticky-sentinel suspend,
restart is a level-triggered generation-advance.)

#### Idempotency posture: level-triggered coalescing (why B, not per-generation consumption)

Overdrive is a **level-triggered reconciler** — it converges *actual* to a
desired *level*; it does not replay a *log* of commands. A restart sets the
desired level "a fresh instance for the latest generation." There is **no
coherent level** that means "N distinct instances in sequence" — that is an
edge-triggered command queue, which the alternative (per-generation
consumption — `observed = observed.saturating_add(1)`, re-enter while
`observed < desired`) would graft onto the reconciler: a durable count of
un-consumed restarts to replay one-by-one. That is the exact
reconciler-vs-workflow anti-pattern ADR-0064's two-primitive doctrine rejects
(command-replay is workflow / journal territory, not reconciler territory). A
"replace the instance" operation is definitionally level-shaped, so coalescing
is the **architecturally correct** contract, not a concession.

The contract this pins, precisely:

- `overdrive workload restart` advances `desired.generation` **monotonically**
  (the `IncrementU64` bump; no lost bump — the iteration-1 atomicity fix
  stands). The generation always advances by N for N restarts; this is
  auditable.
- The reconciler **converges to exactly one fresh instance for the latest
  `desired.generation`**.
- **Sequential** restarts each cycle the workload. `restart` #1 bumps
  `desired = 1`; the reconciler places `payments-1` and stamps `observed = 1`.
  `restart` #2 (issued after that placement) bumps `desired = 2`; the
  reconciler sees `observed = 1 < desired = 2` → places again (`payments-2`).
  This is the normal operator loop ("it came up wedged, restart again") and it
  is **preserved**.
- **Concurrent / rapid pre-placement** restarts **coalesce** into a single
  fresh instance for the latest generation — by design. Two restarts that both
  land before the reconciler places advance `desired` to 2; the reconciler
  places ONCE and stamps `observed = 2`. The generation still advanced by 2
  (audited); only the *placement* coalesces. A double-fired restart must not
  thrash the workload through back-to-back instances.
- This aligns with the Kubernetes research's level-triggered `rollout restart`
  (Finding 7) — consistent with the `generation` model already adopted, where a
  pod-template re-stamp re-rolls toward one desired level rather than queuing a
  replay.

### 2. HTTP route + handler + types

**Route (decided — `/v1/jobs/:id/restart`, mirroring `stop`):**

```rust
.route("/v1/jobs/:id/restart", post(handlers::restart_workload))
```

**Route-naming rationale.** The existing operator routes are all `/v1/jobs/...`
(`/v1/jobs`, `/v1/jobs/:id`, `/v1/jobs/:id/stop`) even though `IntentKey` uses
`workloads/<id>` and the CLI verb is now `workload`. Restart **mirrors `stop`**
under `/v1/jobs/:id/restart` for two reasons: (a) consistency with the live
route family — a lone `/v1/workloads/...` route would split the HTTP surface
mid-feature with no migration of the others; (b) the `jobs/` HTTP prefix is an
independent wire-naming concern from the `workloads/` IntentKey prefix and the
`workload` CLI namespace — relocating the whole `/v1/jobs` family to
`/v1/workloads` is a separate, larger surface change (a deferral candidate, not
this feature's scope). The CLI verb being `workload` while the HTTP path is
`/v1/jobs/:id/restart` is the same already-shipped split as `overdrive job
stop` → `POST /v1/jobs/:id/stop`.

**Handler (mirrors `stop_workload`):**

```rust
pub async fn restart_workload(
    State(state): State<AppState>,
    Path(job_id_str): Path<String>,
) -> Result<axum::Json<RestartWorkloadResponse>, ControlPlaneError>;
```

**Response type** (`crates/overdrive-control-plane/src/api.rs`):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RestartWorkloadResponse {
    pub workload_id: String,
    pub outcome: RestartOutcome,
}
```

**HTTP status mapping:** 200 on success (`Restarted`/`Resumed`); **404**
(`ControlPlaneError::NotFound { resource: "workloads/<id>" }`) when the
`workloads/<id>` aggregate is absent — byte-identical 404 shape to
`stop_workload`; 400 on a malformed id (`parse_workload_id_path`); 500 on store
error.

### 3. CLI http-client method

`crates/overdrive-cli/src/http_client.rs`:

```rust
/// `POST /v1/jobs/{id}/restart` — replace a declared workload's instance.
/// Empty request body. Returns `RestartWorkloadResponse` on 200; 404 maps
/// to `CliError::HttpStatus` with `body.error == "not_found"`.
pub async fn restart_workload(&self, id: &str) -> Result<RestartWorkloadResponse, CliError> {
    self.post_typed(&format!("v1/jobs/{id}/restart"), &serde_json::json!({})).await
}
```

### 4. Generation intent surface

A **NEW standalone sibling intent key**, NOT a field on the rkyv aggregate body:

```rust
// crates/overdrive-core/src/aggregate/mod.rs (IntentKey impl block)
/// Desired-run generation for a workload — `workloads/<id>/generation`.
/// A monotonic `u64` bumped by `overdrive workload restart`; the
/// `WorkloadLifecycle` reconciler places a fresh instance when its
/// View's `observed_generation < generation`. Absent key ⇒ generation 0.
/// Aligns with #180's `workloads/<id>/current` pointer vocabulary; folds
/// into that pointer row when #180's revision lineage lands (a single-cut
/// migration already anticipated in ADR-0050 § Consequences).
pub fn for_workload_generation(id: &WorkloadId) -> Self {
    Self(format!("workloads/{id}/generation").into_bytes())
}
```

**Value encoding:** the `u64` generation as **8-byte big-endian**
(`generation.to_be_bytes()`), read back via `u64::from_be_bytes` with a
length/`.first()`-guarded decode per `development.md` § "Safe byte-slice
access" (absent or short ⇒ generation 0). Big-endian fixed-width (not CBOR, not
rkyv) keeps the read path branch-free, the value trivially hex-debuggable, and
**avoids any ADR-0048 versioned-envelope bump or golden-bytes fixture** — the
key carries no rkyv aggregate. (Sibling-key precedent: `workloads/<id>/stop` is
an empty-byte sentinel; `workloads/<id>/kind` is a single ASCII discriminator
byte — neither is envelope-wrapped.)

**Bump operation (atomic + monotonic — a NEW `TxnOp` read-modify variant).**

The original draft had the handler `get` the current generation, compute `g+1`,
and `TxnOp::Put` it — retrying on `TxnOutcome::Conflict`. **That was a
correctness blocker (review Critical):** the live store surface cannot produce
that conflict. `TxnOp` has only blind `Put`/`Delete`
(`crates/overdrive-core/src/traits/intent_store.rs:62-66`); `LocalIntentStore::txn`
returns `TxnOutcome::Committed` unconditionally
(`crates/overdrive-store-local/src/redb_backend.rs:302-358`, both exit points).
Two concurrent restarts both read `0`, both blind-`Put 1`, both `Committed` —
one bump is silently lost. Worse, a STALE read can `Put` a value *below* the
current generation, driving `generation` backwards and permanently wedging
future restarts (`observed < desired` would never hold again). This violates
`development.md` § "Check-and-act must be atomic" — the check (read current gen)
and the act (write g+1) were two ops with a TOCTOU window the store cannot
guard.

**Decision — extend `TxnOp` with a read-modify variant that performs the
increment INSIDE the store's write transaction:**

```rust
// crates/overdrive-core/src/traits/intent_store.rs
#[derive(Debug, Clone)]
pub enum TxnOp {
    Put { key: Bytes, value: Bytes },
    Delete { key: Bytes },
    /// Read the big-endian u64 at `key` (absent ⇒ 0), write `current + 1`
    /// (saturating). The read and the write happen inside the SAME store
    /// write transaction as every other op in the `txn` batch — see the
    /// trait contract below. This is the atomic-monotonic-increment
    /// primitive; it cannot be expressed by `Put` (blind write) or
    /// `put_if_absent` (insert-if-absent, no increment).
    IncrementU64 { key: Bytes },
}
```

The handler issues **ONE** `txn` carrying both the increment and the
sentinel-delete — they commit atomically:

```rust
let gen_key  = IntentKey::for_workload_generation(&workload_id);
let stop_key = IntentKey::for_workload_stop(&workload_id);
state.store.txn(vec![
    TxnOp::IncrementU64 { key: gen_key.as_bytes().into() },
    TxnOp::Delete       { key: stop_key.as_bytes().into() },
]).await?;   // → TxnOutcome::Committed (no Conflict path exercised)
```

**No `get`-then-`Put` at the call site. No Conflict retry.** redb serializes
write transactions (single exclusive writer; concurrent `txn` calls queue), so
the read-modify-write executes atomically against every other writer. Two
concurrent restarts each enter the write txn in some serial order; each reads
the value the prior committed and writes `+1`. Result: `generation` advances by
exactly the number of restarts, monotonically, with no lost bump and no
backwards wedge — the increment is structurally incapable of going below the
current value.

**Why this redb implementation is atomic + monotonic** (mirrors the shipped
`put_if_absent` precedent at `redb_backend.rs:231-269`, which does a `get` +
`insert` inside one `begin_write`/`commit`): the `IncrementU64` arm in the `txn`
spawn-blocking body does `table.get(key)` → decode BE u64 (absent/short ⇒ 0) →
`current.saturating_add(1)` → `table.insert(key, next.to_be_bytes())`, all
inside the single `begin_write` already opened for the batch, committed once.
A second `txn` cannot interleave because redb holds the exclusive write lock for
the whole `begin_write`/`commit` span.

**Trait behavior contract (REQUIRED on the `TxnOp` variant + the `txn` rustdoc,
per `development.md` § "Trait definitions specify behavior, not just
signature").** DELIVER MUST land the following contract on the trait, not leave
it implicit — every adapter reads the trait, so the contract lives there:

- **Preconditions.** `key` may name an absent row (treated as the u64 `0`) or a
  row holding exactly 8 big-endian bytes. A row at `key` whose length is not 8
  is decoded as `0` per `development.md` § "Safe byte-slice access"
  (length-guarded decode) — never a panic.
- **Postconditions.** After a `txn` containing `IncrementU64 { key }` returns
  `Ok(TxnOutcome::Committed)`, a subsequent `get(key)` returns the 8-byte BE
  encoding of `prev + 1` (saturating at `u64::MAX`), where `prev` is the value
  visible at the instant the batch's write transaction began. The increment and
  every sibling op in the same `txn` batch are committed atomically — there is
  no observable state in which the increment landed but a sibling `Delete` did
  not (or vice versa).
- **Edge cases.** Absent key ⇒ post-state is `1`. Row of `< 8` or `> 8` bytes ⇒
  decoded as `0`, post-state `1` (the read path is corruption-tolerant; the
  write path always emits canonical 8-byte BE). `u64::MAX` ⇒ saturates, stays
  `u64::MAX` (the verb's monotonic-advance contract degrades to "no further
  advance" at the ceiling, never wraps to a lower value that would wedge the
  reconciler).
- **Observable invariant (the property the test asserts).** Across any number
  of concurrent `txn`s each carrying one `IncrementU64 { key }`, the final
  value equals the count of those `txn`s that committed (modulo the `u64::MAX`
  saturation ceiling). The sequence of values a serial reader would observe is
  strictly non-decreasing. No committed increment is lost; the value never goes
  backwards.

**Adapter implementations to specify.** There is exactly ONE production
`IntentStore` implementation in the tree — `LocalIntentStore` (redb-backed,
`crates/overdrive-store-local/src/redb_backend.rs:174`) — and the DST/simulation
path uses that SAME implementation (tempdir-backed redb; per CLAUDE.md's store-
composition table, "Simulation uses the single-mode path since Raft itself is
tested by dedicated consensus tests"; the `SimIntentStore` name in
`overdrive-sim` is a doc alias for the tempdir `LocalIntentStore`, not a second
adapter). DELIVER therefore implements `IncrementU64` once, in
`LocalIntentStore::txn`'s match arm, following the `put_if_absent` get-then-
insert-inside-one-write-txn shape. The other two `impl IntentStore` blocks in
the tree are test doubles
(`FaultInjectingIntentStore` / `CountingIntentStore`); they delegate to or wrap
`LocalIntentStore` and gain the variant for free (exhaustive-match compile
forces the arm — a RED scaffold if not implemented).

**Enforcement test (REQUIRED — the equivalence discipline applied to the one
adapter).** Because there is a single production adapter, the cross-adapter
equivalence harness `development.md` mandates *where two impls genuinely differ*
collapses here to a **concurrency acceptance test against `LocalIntentStore`**,
landed alongside the existing `tests/acceptance/put_if_absent.rs` (Strategy C —
real redb, `tempfile::TempDir`):

- `tests/acceptance/txn_increment_u64.rs` (or a section in an
  `increment_equivalence.rs`) drives **N concurrent `txn`s** (e.g. via
  `tokio::join!` / a `JoinSet` of `N` tasks) each issuing
  `txn(vec![IncrementU64 { gen_key }, Delete { stop_key }])` against the same
  `LocalIntentStore`, then asserts the final `get(gen_key)` decodes to exactly
  `N` (monotonic, no lost bump). A single-restart case asserts `0 → 1`; an
  absent-key case asserts the absent ⇒ `1` edge; a `> 8`-byte corrupt-row case
  asserts the corruption-tolerant `0 ⇒ 1` decode. This is gated
  `integration-tests` (real redb I/O) per `.claude/rules/testing.md`.
- A mutation-testing target: the `IncrementU64` arm is decision logic on the
  hot mutation surface — the test above MUST kill a mutation that swaps `+ 1`
  for `+ 0`, or drops the saturating add.

### 5. The reconciler edit

**State** (`WorkloadLifecycleState`, `workload_lifecycle.rs:1038`) — add:

```rust
/// Desired-run generation, hydrated from `workloads/<id>/generation`
/// (absent ⇒ 0). Bumped by `overdrive workload restart`.
pub generation: u64,
```

**View** (`WorkloadLifecycleView`, `workload_lifecycle.rs:1179`) — add:

```rust
/// The generation this reconciler has already placed a fresh instance
/// for. Persisted input per § "Persist inputs, not derived state": the
/// reconciler places when `observed_generation < desired.generation`
/// and stamps `observed_generation = desired.generation` once it does.
#[serde(default)]
pub observed_generation: u64,
```

Additive `#[serde(default)]` field on the CBOR-serialized `View` per
`development.md` § "Reconciler I/O → Schema evolution" — no envelope bump
(CBOR, not rkyv).

**Decision logic — before/after of the line-520 region (Run branch):**

*Before* (`workload_lifecycle.rs:520`):

```rust
if allocs_vec.iter().any(|r| is_operator_stopped(r)) {
    return (Vec::new(), view.clone());   // overriding veto — always fires
}
```

*After* — gate the veto on the generation comparison **AND scope it to the
current instance** (the post-iteration-2 blocking-bug fix; see § "Why the veto
must be scoped to the current instance" below):

```rust
// A replace bumps `desired.generation`. While the reconciler has not yet
// placed a fresh instance for this generation (observed < desired), the
// current instance's Operator-stop is OVERRIDABLE — the operator's restart
// intent (the generation advance) supersedes the prior stop, exactly as a
// SystemGc row is overridable. When generations are EQUAL (a same-spec
// deploy did NOT bump), the veto stands — Bug 3 preserved: a re-deploy
// cannot resurrect an operator-stopped workload.
//
// CRITICAL: the veto keys off the workload's CURRENT instance only, NOT
// `any(is_operator_stopped)` across all history. `mint_alloc_id`
// deliberately KEEPS the superseded `payments-0 / Terminated{Operator}`
// row (that is how A1 ≠ A2 is achieved), but an operator-stop from a
// SUPERSEDED generation is history, not current intent — it must not veto
// the current instance's lifecycle (incl. crash-restart of the fresh
// alloc). Keying off `any(...)` would let a stale superseded row re-arm
// the veto after the fresh instance is placed and later crashes, wedging
// the fresh instance forever (the bug this revision fixes).
let restart_pending = view.observed_generation < desired.generation;
let current = current_alloc(&allocs_vec); // highest mint_alloc_id index
let veto = !restart_pending && current.is_some_and(is_operator_stopped);
if veto {
    return (Vec::new(), view.clone());
}
```

where `current_alloc` is a **pure helper co-located with `mint_alloc_id`**
(`workload_lifecycle.rs:863`) that returns the workload's latest-placed instance
— the row whose `mint_alloc_id` attempt index is numerically greatest:

```rust
/// The workload's CURRENT instance — the row with the numerically-highest
/// `mint_alloc_id` attempt suffix (`alloc-<workload>-<N>`). This is the
/// most-recently-placed instance; every superseded prior generation has a
/// strictly lower suffix. Returns `None` for an empty alloc set.
///
/// Determination is the NUMERIC max of the parsed `<N>` suffix — NOT the
/// `BTreeMap`/`.values()` iteration order, which is LEXICAL on the raw
/// `AllocationId` string (`alloc-payments-10` sorts BEFORE `alloc-payments-2`),
/// so "last in iteration" is WRONG once the attempt index reaches double
/// digits. Co-located with `mint_alloc_id` so the parse and the mint stay
/// in lockstep (the suffix grammar is `mint_alloc_id`-internal). A row whose
/// suffix fails to parse sorts below any parseable suffix (a defensive floor
/// — never the current instance).
fn current_alloc<'a>(allocs: &[&'a AllocStatusRow]) -> Option<&'a AllocStatusRow>;
```

The determination is robust by construction: `mint_alloc_id` mints
`attempt = allocs_vec.len()` and the feature relies on alloc rows being **never
deleted** (the superseded `payments-0` row is intentionally retained), so the
attempt indices are a strictly-increasing `0, 1, 2, …` series and the numeric
max is unambiguously the latest placement. This needs **no new per-row field**
(no `generation` on `AllocStatusRow` ⇒ no ADR-0048 envelope bump — see the
"current/latest" determination note below). An equivalent positive form is the
"no higher-index successor" predicate — `is_operator_stopped(r) && no alloc has
a higher attempt index than r` — semantically identical; the `current_alloc`
max form is pinned because it factors the "which is current" question into one
named helper the rest of the gate (and the running-origin R2 stop selection)
can reuse.

When `restart_pending` and the fresh placement is emitted (the `first_fit_place`
arm, `workload_lifecycle.rs:725-794`), the reconciler stamps
`next_view.observed_generation = desired.generation` so the next tick sees
`observed == desired` and the veto re-arms for the *new* instance. The fresh
AllocationId/`workload_addr` come for free: `mint_alloc_id` indexes by
`allocs_vec.len()` (the SystemGc-resubmit precedent), so with `payments-0`
present the placement mints `payments-1` (`A1 ≠ A2`, new `/30`).

**Stop-then-start for the running-origin case:** when the workload is running
at restart time, the bump+clear handler also needs the current Running instance
ended so the fresh placement is a genuine *replacement*. The Run branch's
existing `running_alloc.is_some()` early-return (line 485) is gated the same
way: when `restart_pending` and a Running instance exists for a generation the
reconciler has not yet placed, the reconciler emits a `StopAllocation` for the
current Running instance (terminal `Stopped { by: Operator }`) on this tick and
places the fresh instance once the prior is Terminated on a subsequent tick —
the end-then-bring-up shape.

**Running-origin state-transition table (PINNED here; resolves review Finding
4).** The sequencing is load-bearing — repeated reconcile ticks while the old
alloc is still Running MUST NOT emit duplicate `StopAllocation`s, and
`observed_generation` MUST NOT be stamped until the fresh placement actually
happens. The state machine below is the contract; DELIVER pins only the
*concrete tick wiring* (which existing branch each row maps to), not the
machine. `restart_pending = view.observed_generation < desired.generation`.

The veto in every row below is the **current-instance-scoped** form
(`!restart_pending && current_alloc(&allocs_vec).is_some_and(is_operator_stopped)`)
— NOT `any(is_operator_stopped)`. A superseded prior-generation Operator-stop row
(e.g. the retained `payments-0` after `payments-1` is placed) is NEVER the
current instance, so it can never re-arm the veto. This is the load-bearing
invariant the post-iteration-2 fix establishes (see § "Why the veto must be
scoped to the current instance").

| # | `restart_pending` | Actual-state of the workload's allocs | Action emitted | `observed_generation` (next View) |
|---|---|---|---|---|
| R1 | `false` | any (Running / Terminated-Operator / none) | the unchanged pre-#249 behavior (Run/Stop/restartable handling), with the veto **scoped to the current instance**: it fires only when `current_alloc(&allocs_vec)` is itself operator-stopped (a *current* stop), NEVER on a superseded prior-generation Operator-stop row. A current Operator-stopped instance vetoes (Bug 3); a Running/Failed/Completed current instance flows to the existing Run-branch handling. | unchanged (`= view.observed_generation`) |
| R1-crash | `false` | the current instance is Terminated/Failed with a CRASH reason (NOT `Stopped{Operator}`), AND one or more SUPERSEDED `Terminated{Operator}` rows from prior generations are also present | `RestartAllocation` for the **current (crashed) instance** via the existing `is_restartable`/backoff branch — the stale superseded Operator-stop rows do NOT veto, because `current_alloc(...)` is the crashed instance, not the superseded stop | unchanged (crash-restart reuses the current instance's slot per the existing branch) |
| R2 | `true` | a Running alloc exists for the current (un-replaced) generation | **one** `StopAllocation { terminal: Stopped { by: Operator } }` for that Running (current) alloc | **unchanged** — NOT yet stamped (the fresh instance has not been placed) |
| R3 | `true` | the prior alloc is now Terminated (Operator), AND no Running alloc remains | `StartAllocation` (`first_fit_place`; `mint_alloc_id` mints the next index ⇒ `A1 ≠ A2`, new `/30`) | **stamped** `= desired.generation` (placement is happening this tick) |
| R4 | `true` | operator-stopped origin: a Terminated/Operator row, no Running alloc, no intervening stop needed | `StartAllocation` (the fresh placement — the `Resumed` path) | **stamped** `= desired.generation` |
| R5 | `true` | a `StopAllocation` was already emitted (R2) and the alloc is still draining (not yet Terminated) | **none** (no duplicate stop — the prior `StopAllocation` is in flight) | **unchanged** |

**R1-crash is the row the post-iteration-2 fix adds.** It is the
`restart_pending == false` case where the *current* instance is a genuine crash
(a `payments-1` that reached Running after a restart, then `Failed`/crash-
`Terminated`) while a *superseded* `payments-0 / Terminated{Operator}` row
lingers. Under the old `any(is_operator_stopped)` veto this row hit the veto and
returned `(Vec::new(), …)` BEFORE the `is_restartable` branch — wedging the fresh
instance forever. Under the scoped veto, `current_alloc(...)` is the crashed
`payments-1` (a crash reason, not Operator), so the veto does not fire, the Run
branch falls through to the existing crash-restart/backoff branch, and the fresh
instance's crash converges normally. This is the regression the verification
plan's new acceptance case pins forever.

**Idempotency of the stop (R2 → R5).** The "still Running on a later tick"
re-entry (R5) emits no action because the Run branch's existing
`is_alloc_mutating_action` / in-flight-action collapse (and the broker's
`(reconciler, target)` keying) already debounce a second `StopAllocation`; the
table makes the no-duplicate-stop requirement explicit so DISTILL can write a
focused state-machine test (deploy → run → restart → assert exactly one
`StopAllocation` across the draining ticks). **The stamp happens once, on the
placement tick (R3/R4), never on the stop tick (R2/R5)** — this is the
load-bearing ordering: stamping on R2 would let `observed == desired` re-arm the
veto before the fresh instance exists, stranding the workload Terminated.

**Coalescing vs. sequential under this table (the load-bearing distinction).**
The stamp is `observed_generation = desired.generation` (NOT
`observed + 1`), so the machine coalesces by construction:

- **Concurrent / pre-placement restarts coalesce.** If a second restart arrives
  *before* the reconciler places — while `restart_pending` is true and the
  placement has not yet happened — it advances `desired.generation` again
  (say to 2) but does NOT add a second pending placement. When the reconciler
  reaches R3/R4 it places ONCE and stamps `observed = desired = 2`; the next
  tick sees `observed == desired`, so `restart_pending` is **false** and the
  veto re-arms. There is no second placement: the two bumps coalesced into one
  fresh instance for the latest generation. (Earlier drafts of this note
  claimed a coalesced second bump "re-enters at R2/R3 and places again" — that
  was wrong; once `observed == desired` after the placement, `restart_pending`
  is false and the machine does not re-place.)
- **Sequential restarts each cycle.** If the second restart arrives *after* the
  first placement has stamped `observed = 1`, it bumps `desired = 2`, so
  `restart_pending = (1 < 2)` is true again — the machine re-enters at R2/R3 and
  places `payments-2`. This is the preserved normal operator loop.

(DELIVER pins the precise existing-branch mapping as it exercises the oracle;
the machine above is the contract.)

#### Why the veto must be scoped to the current instance (post-iteration-2 blocking-bug fix)

The iteration-3 review surfaced a **blocking correctness bug** in the
iteration-2 gate. The proposed veto keyed off *any* operator-stopped row across
all alloc history:

```rust
// BUG (iteration-2): keys off ALL history.
if !restart_pending && allocs_vec.iter().any(|r| is_operator_stopped(r)) {
    return (Vec::new(), view.clone());
}
```

`mint_alloc_id` deliberately KEEPS the superseded `payments-0 /
Terminated{Operator}` row — that retention is exactly how `A1 ≠ A2` is achieved
(`attempt = allocs_vec.len()` mints `payments-1` only because `payments-0`
survives). The generation gate made the override **transient** (in force only
while `restart_pending`), but the veto must be **permanent** for superseded rows.
The failure sequence (manifests for BOTH stopped-origin and running-origin):

1. Restart places `payments-1`, stamps `observed_generation = desired.generation`
   → `restart_pending` flips to **false**.
2. While `payments-1` is Running, the `running_alloc` early-return (line 485)
   wins — fine.
3. `payments-1` **later crashes** (terminal is a crash reason — `Failed` /
   `Terminated` with a non-`Stopped{Operator}` reason). Now line 485 finds no
   Running alloc and falls through to the veto: `!restart_pending` is `true` AND
   the STALE `payments-0 / Operator` row still satisfies
   `any(is_operator_stopped)` → the veto fires, `return (Vec::new(), …)`. The
   `is_restartable` crash-restart branch (which sits AFTER the veto) is never
   reached.
4. **The fresh instance is wedged** — its crash never converges, because a
   *superseded historical* operator-stop row re-arms the veto.

**Root cause.** An operator-stop from a *superseded generation* is no longer the
workload's current operative intent; it is history. The veto must honor the
operator's **current** stop intent only. The scoped form
(`current_alloc(...).is_some_and(is_operator_stopped)`) fires the veto **only
when the workload's current (latest-placed) instance is itself operator-stopped**
— a genuine current suspension — and ignores every superseded prior-generation
row. This is the iteration-3 review's recommended "make the veto
generation-aware / scoped to non-superseded rows," realized via the existing
alloc-id suffix monotonicity (the latest-placed instance has the numerically
highest `mint_alloc_id` attempt index) rather than a new per-row field — the
**lightest** of the review's three acceptable shapes:

- It does NOT record consumed Operator-stop alloc ids in the View (option 1) —
  no new View field, no extra persisted-input bookkeeping.
- It does NOT add a per-alloc placement-generation marker (option 2) — and
  crucially **adds no `generation` field to the rkyv-persisted `AllocStatusRow`**,
  so there is **no ADR-0048 versioned-envelope bump and no golden-bytes fixture**
  (consistent with the thin-seam principle — the only generation state remains
  the `workloads/<id>/generation` sibling key + the View's `observed_generation`).
- It reuses the existing minting invariant (`attempt = allocs_vec.len()`, rows
  never deleted) that the feature already depends on for `A1 ≠ A2`.

**The three required walks (all confirmed under the scoped veto):**

- **Bug-3 preserved.** Operator stops `payments`, then a same-spec `deploy` (no
  generation bump → `restart_pending = false`). The current instance
  (`current_alloc(...)`) IS the operator-stopped `payments-0` (no later
  placement happened), so `current.is_some_and(is_operator_stopped)` is true →
  the veto fires → the workload stays stopped. A re-deploy does NOT resurrect an
  operator-stopped workload. ✓
- **Fresh-alloc crash after restart converges (the bug being fixed).** deploy →
  stop → restart → `payments-1` Running → `payments-1` CRASHES (`Failed`, not
  Operator). `restart_pending = false` (stamped on placement), but
  `current_alloc(...)` is `payments-1` (a crash reason, NOT Operator) → the veto
  does NOT fire → the Run branch falls through to `is_restartable` →
  `RestartAllocation` for `payments-1`. The stale `payments-0 / Operator` row is
  ignored (it is not the current instance). The fresh instance's crash
  converges. ✓ (R1-crash in the table above.)
- **Running-origin restart.** deploy (Running `payments-0`) → restart. R2:
  `restart_pending = true` overrides, the reconciler emits `StopAllocation` for
  the current Running `payments-0` (now `payments-0` becomes the current
  operator-stopped instance, but `restart_pending = true` overrides the veto). R3:
  the prior instance is Terminated, the reconciler places fresh `payments-1` (now
  the current instance) and stamps `observed = desired`. A later `payments-1`
  crash → `current_alloc(...)` is `payments-1` (crash) → converges via R1-crash. ✓

**Two further cases the scoped veto handles consistently:**

- **Restart-after-restart (sequential).** Each placement makes its
  `payments-<N>` the current instance; a later operator-stop or a further restart
  always evaluates against the *latest* instance, so the operative state is the
  one the operator last acted on — never a superseded `payments-<N-k>`.
- **A genuine re-stop after a restart.** The operator stops the fresh
  `payments-1` (the Stop branch, line 377, fires on the `workloads/<id>/stop`
  sentinel — the Run branch is not reached). Were the Run branch reached,
  `current_alloc(...)` would be the operator-stopped `payments-1` and the scoped
  veto would correctly suspend the workload — consistent with the sentinel-driven
  Stop branch.

### 6. The handler's full sequence + atomicity argument

`restart_workload`, mirroring `stop_workload`'s shape:

1. **Parse** `id` via `parse_workload_id_path` (400 on malformed).
2. **Check-exists → 404, and classify the outcome label:** one point-in-time
   read transaction looking up two keys — `state.store.get(IntentKey::for_workload(&id))`
   (absent ⇒ `ControlPlaneError::NotFound { resource: "workloads/<id>" }`) and
   `state.store.get(IntentKey::for_workload_stop(&id))`. The `/stop` lookup
   classifies the `RestartOutcome` label ONLY (present ⇒ `Resumed`, absent ⇒
   `Restarted`, per § item 1); it does NOT gate placement — that is the
   reconciler's generation gate. The handler does **not** read the current
   generation value at all (the increment is store-side; see step 3).
3. **Atomic bump + clear:** one `IntentStore::txn` doing
   `TxnOp::IncrementU64 { key: workloads/<id>/generation }` **and**
   `TxnOp::Delete { key: workloads/<id>/stop }`. The increment is read-modify-
   write *inside* the store's write transaction (§ item 4) — the handler never
   reads-then-writes the generation, so there is no call-site TOCTOU and no
   `Conflict` retry. Returns `TxnOutcome::Committed`.
4. **Enqueue evaluation** for the `job-lifecycle` reconciler
   (`enqueue_workload_lifecycle_eval`), exactly as `stop_workload` does.
5. **Respond** 200 `{ workload_id, outcome }`.

**Atomicity argument** (`development.md` § "Check-and-act must be atomic"): the
generation increment and the sentinel clear are a **single `txn` commit** with
the increment performed read-modify-write *inside* that write transaction — the
check (read current gen) and the act (write g+1) are the SAME atomic op, not two
ops with a window between them. redb serializes write transactions, so two
concurrent restarts are forced into a serial order and each reads the value the
prior committed: the bump is monotonic and no bump is lost. There is no window
in which the sentinel is cleared but the generation not yet bumped (which a
converging stop could otherwise re-observe to flip-flop the reconciler). The
check-exists read (step 2) is a separate read, but it gates a 404 + a cosmetic
label only — it does not gate a mutation that races, and a workload deleted
between step 2 and step 3 simply leaves a generation key on a now-absent
aggregate (the reconciler's Run branch reads `desired.job.is_none()` and GCs;
the stray generation key is harmless and reaped by #211 deletion semantics).
The bump is monotonic (`saturating_add` inside the serialized write txn), so a
concurrent second restart does not race-and-lose: it serializes after the first,
reads the already-incremented value, and bumps again — **two restarts advance
`generation` by exactly 2, never a lost bump and never a backwards step.** The
*placement* cardinality is then the reconciler's level-triggered concern, not
the handler's: two restarts that *coalesce* (both bumps land before the
reconciler places) converge to **one** fresh instance for the latest generation
(the reconciler places once and stamps `observed = desired = 2`); two
*sequential* restarts (the second issued after the first placement stamps
`observed = 1`) each cycle the workload, because `restart_pending = (1 < 2)`
re-holds. Either way the generation advanced by exactly 2 (audited); the
monotonic bump is what makes both cases correct.

**On the dead `TxnOutcome::Conflict` variant.** With `IncrementU64` the handler
never observes `Conflict` — `LocalIntentStore::txn` returns `Committed`
unconditionally and the atomic increment removes any need for optimistic-
concurrency retry. The `Conflict` variant is therefore **currently unproduced by
the only `IntentStore` implementation**; it is retained as forward-looking
surface for the future `RaftStore` HA path (ADR-0020 lineage), where a
log-append conflict under multi-writer consensus could genuinely arise. This ADR
relies on NO `Conflict` behavior; any earlier ADR text implying the store
produces it on the restart path is superseded by § item 4.

## Alternatives considered

### Alt-A — Lean narrow-veto edit (clear sentinel + relax line 520 to read the sentinel, no generation) — REJECTED

Edit line 520 to skip the veto when the sentinel is absent (i.e. make the
observation-veto track the intent sentinel rather than the observed row). This
is the smallest possible change and closes #249's happy path. Rejected because
it leaves **no forward-compat seam**: #64 (rolling deploy), #253 (zero-downtime),
and #254 (multi-replica) all need a value-compared `generation` to gate
*which* instances to replace and *when* a placement is "for this revision."
Without it, every one of those features re-opens the same reconciler decision
and rips out the narrow edit. The generation precursor costs one `u64` key +
two struct fields now and is reused verbatim by all three — the explicit #180
alignment the locked decision mandates.

### Alt-B — Re-stamp the observed row to `SystemGc` (make Operator-stop overridable by relabelling) — REJECTED

On restart, rewrite the observed `payments-0` row's terminal from `Operator` to
`SystemGc` so the existing `active_allocs_vec` filter (line 477) makes it
overridable, reusing the SystemGc-resubmit path untouched. Rejected on two
counts: (a) it **mutates an observation row to lie about its cause** — the
operator *did* stop it; relabelling corrupts the audit/`alloc status` honesty
contract Ana relies on (the persona's core promise). (b) Observation rows are
owner-writer-only, eventually-consistent, and gossiped (state-layer hygiene);
the restart handler is not their writer and reaching across to rewrite them
violates the intent/observation boundary. The generation gate achieves the same
"override the veto" outcome by adding *intent* (a generation advance), never by
falsifying *observation*.

### Alt-C — Pull #180's full revision lineage forward (Option α) — REJECTED

Land `workloads/<id>/current` pointer + `workloads/<id>/revisions/<RevisionId>`
rows + retention policy + status reporting now, so restart is "advance the
current pointer to a new revision." Rejected: this is precisely the scope
ADR-0050 OQ-1 deferred to #180 ("Option β") for sound reasons (≈30-40h vs
≈12-18h; final-shape revision storage is a rolling-update concern, not an
instance-replace concern). #249 needs ONE fresh instance of the SAME spec, not
a new revision of a CHANGED spec — revision rows, `RevisionId`, and retention
buy nothing for same-spec replacement and would over-build infrastructure no
#249 path exercises (the vertical-slice anti-pattern). The seam is kept minimal:
`generation`/`observed_generation` only.

## Forward-compatibility (#180 / #64 / #253 / #254)

- **#180 (revision lineage):** when it lands, `generation` moves from the
  standalone `workloads/<id>/generation` sibling into the `workloads/<id>/current`
  pointer row (where ADR-0050 § Consequences already says it belongs:
  "synthetic `revision_id = spec_digest`, `generation = 1`"). The reconciler's
  `observed_generation < generation` gate is unchanged — only the hydrate source
  moves. A single-cut migration ADR-0050 already anticipates.
- **#64 (rolling deploy reconciler):** reuses `generation`/`observed_generation`
  verbatim as the per-revision placement gate; the restart verb becomes the
  degenerate single-step rollout.
- **#253 (zero-downtime):** the end-then-bring-up cut here is the v1; #253
  flips the reconciler to bring `A2` up *before* ending `A1`, gated on the same
  generation comparison.
- **#254 (multi-replica replace-all vs replace-one):** the generation gate
  scales to "replace every instance whose placement generation < desired" with
  no new intent surface.

The seam is **THIN by construction**: no revision rows, no `RevisionId`, no
retention, no status reporting are pulled forward.

## Bug-3 preservation argument

The `fix-exec-driver-exit-watcher` Bug-3 invariant — *a same-spec re-deploy must
NOT resurrect an operator-stopped workload* — is preserved because **only
`restart` bumps the generation**. `overdrive deploy` `put_if_absent`s the
aggregate (no-op when present) and never touches `workloads/<id>/generation`,
so after a deploy `observed_generation == desired.generation` (`restart_pending`
is false). In the Bug-3 scenario no later placement happened, so the workload's
**current instance** (`current_alloc(...)`) IS the operator-stopped `payments-0`
row — `current.is_some_and(is_operator_stopped)` is true → the scoped veto fires
→ the workload stays stopped. (The scoped veto does NOT weaken Bug-3: scoping
narrows *which* row arms the veto, and the operator-stopped current instance is
exactly the row that should.) SystemGc rows remain overridable via the unchanged
`active_allocs_vec` filter (the override there is driven by intent withdrawal +
resubmit, orthogonal to generation). The generation gate is purely *additive*
to the existing asymmetry: it adds a third, intent-driven way to make an
Operator-stop overridable (an explicit restart), without weakening the
deploy-can't-resurrect guarantee.

## TOCTOU / atomicity argument

See items 4 and 6. The bump + clear is a single `IntentStore::txn` commit
carrying the NEW `TxnOp::IncrementU64` (read-modify-write *inside* the store's
write transaction) plus `TxnOp::Delete`. The increment is atomic and monotonic
because redb serializes write transactions — concurrent restarts queue and each
reads the value the prior committed. There is **no `get`-then-`put` window on the
mutating path** (the original draft's read-then-`Put` TOCTOU is gone) and **no
reliance on an unproduced `TxnOutcome::Conflict`** (the prior draft's retry-on-
Conflict was a path the store cannot exercise). The only standalone read
(check-exists + cosmetic `/stop` label, item 6 step 2) gates a 404 + a best-
effort outcome label and races harmlessly — it does not gate a mutation.
`development.md` § "Check-and-act must be atomic" is satisfied: the check and the
act are one op, not two.

## Reuse Analysis amendment — the `IncrementU64` store primitive (CREATE-NEW on the port, justified)

The atomic-bump fix adds one genuinely-new surface to the `IntentStore` port:
the `TxnOp::IncrementU64` variant. This is an **honest CREATE-NEW**, not an
EXTEND-of-an-equivalent: the existing surface genuinely cannot express atomic
monotonic increment — `Put` is a blind write (TOCTOU under concurrency),
`put_if_absent` is insert-if-absent (no increment), and the standalone `get` +
`Put` shape is exactly the race the review rejected. The variant is the minimal
addition that makes the bump atomic + monotonic.

It is **forward-compatible seam work, not throwaway:** #180's revision-lineage
model needs the same atomic monotonic advance to move
`generation`/`observed_generation` from the standalone sibling key into the
`workloads/<id>/current` pointer row (ADR-0050 § Consequences). The
`IncrementU64` primitive is reused verbatim there — pulling it forward now costs
one `TxnOp` arm + one redb match arm and is consumed by #180/#64/#253/#254. The
feature-delta, wave-decisions, and brief Reuse-Analysis tables are updated to
record this row.

## Consequences

### Positive

- Closes #249's `[D1]` with a forward-compatible seam reused by #64/#253/#254.
- No ADR-0048 envelope bump, no golden-bytes fixture (generation is a sibling
  key, not an rkyv aggregate field).
- Observation honesty preserved (Alt-B rejected) — no row relabelling.
- The `workload` CLI namespace lands aligned with #220.
- The new `TxnOp::IncrementU64` primitive makes the generation bump provably
  atomic + monotonic against concurrent restarts (review Critical resolved), and
  is the SAME primitive #180's generation model will reuse.

### Negative

- A new HTTP route family member under `/v1/jobs/:id/restart` while the CLI verb
  is `workload` — the same already-shipped CLI/HTTP-prefix split as `job stop`;
  a future `/v1/jobs → /v1/workloads` route relocation is out of scope (deferral
  candidate, not created here).
- A stray `workloads/<id>/generation` key can outlive a deleted aggregate until
  #211 deletion reaps it — harmless (reconciler GCs on `job.is_none()`), noted.
- `restart` is **level-triggered / coalescing**, not edge-triggered: the
  generation advances monotonically per call (audited), but the reconciler
  converges to *one* fresh instance for the latest generation. Sequential
  restarts each cycle the workload; concurrent / pre-placement restarts coalesce
  into one cycle. This is the correct rollout-restart posture (it cannot thrash
  a workload through back-to-back instances on a double-fired restart) but
  differs from `stop`'s sticky-sentinel idempotency; documented in the
  `RestartOutcome` rationale and the § "Idempotency posture: level-triggered
  coalescing" subsection.
