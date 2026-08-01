# RCA — cross-restart LWW counter regression in the observation store

**Verdict: REAL — REPRODUCED end-to-end through real `overdrive serve` +
`overdrive deploy` + restart.**

Investigated 2026-08-01 against `945654dd` (branch
`marcus-sa/mtls-fault-injection-test-infra`). The claim was recorded in
ADR-0076 § 7d finding 1 as *"Verified from source; NOT reproduced at
runtime."* It is now reproduced at runtime, and the blast radius is
**wider than § 7d states**: three further durable row types are affected,
and one variant of the same mechanism **needs no restart at all**.

---

## 1. Summary

`tick_n` is `let mut tick_n: u64 = 0;` inside `spawn_convergence_loop`
(`crates/overdrive-control-plane/src/lib.rs:2434`), incremented once per
loop iteration at `:2469` — **outside** the `for eval in pending` loop
that closes at `:2467`, so it advances every cadence period whether or
not work was pending. At `DEFAULT_TICK_CADENCE` = 100 ms (`lib.rs:648`)
the counter is, in effect, **deciseconds of control-plane uptime**.
Nothing seeds it from persistent state.

`timestamp_for` derives the LWW counter from that tick alone
(`crates/overdrive-control-plane/src/action_shim/mod.rs:1753`):

```rust
const fn timestamp_for(tick: &TickContext, writer: NodeId) -> LogicalTimestamp {
    LogicalTimestamp { counter: tick.tick.saturating_add(1), writer }
}
```

Observation rows are durable across restarts, and the writer `NodeId` is
a compile-time literal. So after a restart the writer's counter starts at
1 while surviving rows carry the pre-restart high-water mark, and **every
tick-derived write for a pre-existing row is silently discarded until the
tick counter climbs back past it** — a window equal to the previous
process's uptime.

The three preconditions are each independently confirmed:

| Precondition | Status | Evidence |
|---|---|---|
| Rows survive the restart | **Yes** | `LocalObservationStore::open` uses redb `Database::create`, which opens with `.truncate(false)` (`redb-2.6.3/src/db.rs:1198-1208`); commits default to `InternalDurability::Immediate` (fsync). Existing passing test `crates/overdrive-store-local/tests/acceptance/local_observation_store.rs:113` (`restart_round_trip_alloc_status`) asserts reopen survival. |
| The counter resets | **Yes** | `lib.rs:2434`, a literal `0`. No seed from any store. |
| The tiebreak cannot rescue a tie | **Yes** | `lib.rs:1701` — `NodeId::new("local")`, a compile-time literal, identical every boot. `dominates`' `Equal` arm (`crates/overdrive-core/src/traits/observation_store.rs:268`) evaluates `"local" > "local"` → **`false`, deterministically**. The loss is total and reproducible, never intermittent. |

And the failure is **invisible**. `apply_alloc_status_lww` returns
`Ok(dominates)` (`crates/overdrive-store-local/src/observation_backend.rs:1010-1036`);
`write` commits regardless and returns `Ok(())` with no error and no log
(`:397-504`). No caller can distinguish a dropped write from a successful
one.

---

## 2. Reproduction

Per `.claude/rules/debugging.md` § 4/§ 10, each probe carried a written
hypothesis, prediction, and falsification path, and was scored against
the **prediction**.

### 2.1 Probe A — real `action_shim::dispatch` across a store reopen

> **Hypothesis:** after reopening a durable `LocalObservationStore`, a real
> `action_shim::dispatch` at `tick = 0` for an alloc whose surviving row
> carries `counter = 6000, writer = "local"` is silently rejected; the same
> dispatch for an alloc with no prior row lands.
> **Predicted:** surviving → unchanged (`Running`, `counter 6000`), `dispatch`
> returns `Ok(())`. Brand-new → lands at `counter 1`.
> **Falsification:** surviving row shows `counter 1`, or `dispatch` returns `Err`.

```
=== LIFETIME 1 (pre-restart control plane, ~10 min uptime) ===
    surviving  (as written, lifetime 1): state=Running counter=6000 writer=local reason=None

=== PROCESS EXIT -> REOPEN SAME PATH (lifetime 2, tick counter reset to 0) ===
    surviving  (after reopen, BEFORE dispatch): state=Running counter=6000 writer=local reason=None
    brand-new  (after reopen, BEFORE dispatch): <no row>

=== POPULATION A: SURVIVING alloc, first tick after restart (tick=0) ===
    dispatch(StartAllocation, tick=0) -> Ok(())
    surviving  (AFTER dispatch): state=Running counter=6000 writer=local reason=None

=== POPULATION B: BRAND-NEW alloc, same tick (tick=0), same store ===
    dispatch(StartAllocation, tick=0) -> Ok(())
    brand-new  (AFTER dispatch): state=Running counter=1 writer=local reason=Some(Started)
```

Prediction matched exactly. **The population diff is the diagnosis**
(§ 5): identical action, identical tick, identical store — the only
difference is the presence of a surviving row. The surviving alloc keeps
`reason=None` (the seed value); the brand-new alloc gets
`reason=Some(Started)` from the shim's write. `dispatch` returned `Ok(())`
in **both** cases.

Boundary sweep at `prior_counter = 6000`:

```
  tick=5999  -> stamp counter=6000  => post-restart write LOST (silently dropped)
  tick=6000  -> stamp counter=6001  => post-restart write WON  (landed)
```

`tick=5999` produces stamp 6000, ties, and loses to the deterministic
`false` tiebreak — confirming the `node_id` analysis above.

### 2.2 Probe B — full `overdrive serve` + `overdrive deploy` + restart

The decisive probe: the real binary, the real operator verbs, one fixed
`data_dir` reused across both boots.

One blocker was hit and cleared honestly: the first `serve` refused to
boot with `KEK unavailable at boot; control-plane refusing to start`. It
was resolved through the **production** systemd-creds path
(`CREDENTIALS_DIRECTORY`), not a test seam — no `SimKek`, no dev env-var
opt-in.

**Run 1** — boot 1 up ~52 s, then killed:

```
=== DEPLOY (boot 1) ===
Accepted.
Workload ID:   probe-job
Attempt  State        Exit   Started              Duration
1        Running      —      (c=522,w=local)      —

--- durable redb dump after killing boot 1 ---
  alloc=alloc-probe-job-0 state=Running counter=522 writer=local reason=Some(Started)

=== BOOT 2, SAME data_dir ===
=== DRIVE ACTION: overdrive job stop probe-job  (t+173ms after listen) ===
Stopped workload 'probe-job'.
stop exit=0
=== describe AFTER stop ===
1        Running      —      (c=522,w=local)      —
```

The operator's `job stop` returned **exit 0** and printed
`Stopped workload 'probe-job'.` — while the store still read `Running`.

Then, with **no further operator command issued**, polling alone:

```
   1        Running      —      (c=522,w=local)      —
   1        Terminated   —      (c=523,w=local)      —
```

This is the observation that rules out the competing explanation. Per
§ 11, an unchanged surface is a downstream symptom — the reconciler was
**not** failing to emit. It emitted on every tick; every write was
rejected until the counter crossed, and the first one to win did so at
`523 = prior + 1`, the *minimum* dominating stamp.

**Run 2** — boot 1 killed early, prior counter **4**. Predicted window
≈ 0.4 s:

```
  row on boot 1: 1 Running — (c=4,w=local) —
=== BOOT 2 (SAME data_dir; prior row counter = 4) ===
t+10    ms  1 Running — (c=4,w=local) —
t+322   ms  1 Running — (c=4,w=local) —
t+647   ms  1 Terminated — (c=5,w=local) —
```

**Run 3** — the production-path population diff, *one* control plane,
two allocs. `surv` deployed pre-restart (row at `c=269`); `fresh`
deployed after the restart. Both stopped at ≈ t+0:

```
t+2029  ms | SURVIVING: 1 Running    — (c=269,w=local) | BRAND-NEW: 1 Terminated — (c=20,w=local)
t+28723 ms | SURVIVING: 1 Running    — (c=269,w=local) | BRAND-NEW: 1 Terminated — (c=20,w=local)
t+30797 ms | SURVIVING: 1 Terminated — (c=270,w=local) | BRAND-NEW: 1 Terminated — (c=20,w=local)
```

Same binary, same boot, same tick stream, same operator verb. The
brand-new alloc terminated in under 2 s; the surviving alloc was stuck
`Running` for ~29 s, then flipped at `prior + 1`.

### 2.3 The drop is completely silent

The entire boot-2 log across the ~29 s window in which every stop write
was rejected:

```
=== FULL boot-2 log ===
2026-08-01T13:28:56.929784Z  INFO adopt-on-restart §5: swept 1 surviving per-workload nft-TPROXY rule(s) ... swept=1
2026-08-01T13:28:56.930240Z  INFO control plane listening endpoint=https://127.0.0.1:7097/
=== line count: 2 ===
=== any warn/error/lww/reject/drop mention? ===
  NONE — the drop is completely silent
```

---

## 3. Recovery window

**Window ≈ `prior_counter` ticks ≈ the pre-restart control-plane uptime.**
Four independent measurements:

| prior counter | predicted first winning tick | predicted wall clock | measured flip |
|---|---|---|---|
| 4 | 4 | 0.4 s | between 0.32 s and 0.65 s |
| 269 | 269 | 26.9 s | between 28.7 s and 30.8 s |
| 522 | 522 | 52.2 s | ≈ 52 s (coarse 5 s poll) |
| 6000 (Probe A, synthetic) | 6000 | 600 s | exact — 5999 loses, 6000 wins |

Measured slightly exceeds `counter × 100 ms` because
`spawn_convergence_loop` sleeps the cadence *after* the tick's work, so
the true period is > 100 ms. The honest bound:

> **A surviving allocation is unwritable for at least as long as the
> previous control-plane process was up.** A control plane up for a day
> leaves surviving allocs frozen for at least a day after restart.

The scaling is the damning part — the longer the system has run
successfully, the longer the outage after a restart.

---

## 4. Blast radius

### 4.1 Every production tick-derived durable write site

Ten sites across four row types. All are redb-persisted and LWW-guarded,
so all inherit the regression.

| Row type | Production sites | Counter source |
|---|---|---|
| `AllocStatusRow` | `action_shim/mod.rs:526` (netns fail-closed), `:1076` (`FinalizeFailed`), `:1251` (`StartAllocation`), `:1470` (`RestartAllocation`), `:1580` (`StopAllocation`) | `tick.tick + 1` |
| `ServiceBackendRow` | `overdrive-core/src/reconcilers/backend_discovery_bridge.rs:392`, `overdrive-core/src/service_lifecycle.rs:860` | `tick.tick + 1` |
| `ServiceHydrationResultRow` | `action_shim/dataplane_update_service.rs:126`, `:155` | `tick.tick + 1` |
| `ReconcileConflictRow` | `reconciler_runtime.rs:1444` | `tick.tick + 1` |

ADR-0076 § 7d finding 3 named `ServiceBackendRow` and left reachability
unaudited; `ServiceHydrationResultRow` and `ReconcileConflictRow` were
not named at all.

Three of the five `AllocStatusRow` sites (`:1076`, `:1470`, `:1580`)
**already hold the prior row in scope** and take the *writer* from it —
`timestamp_for(tick, prior_row.node_id.clone())` — while still taking the
*counter* from the tick. The prior counter was available at each of these
call sites and was not consulted. That is what makes the remedy in § 7
cheap at these three.

### 4.2 Self-healing vs permanently stuck

Not uniform. Two distinct shapes:

**Self-healing (transient, bounded by the recovery window) —
`WorkloadLifecycle`.** Its guards read the *observation row*, not its
view, so a stale row makes the guard *not* fire and the action re-emits
each tick (`crates/overdrive-core/src/reconcilers/workload_lifecycle.rs:637-643`,
`:686-691`). Fresh placement re-mints the same `alloc_id` from the
observed row count (`:812-816`), so the retry is idempotent on the key.
This is what Probe B Run 1 captured: the write retried every tick and
landed the moment the counter crossed.

**Fire-once (unbounded) — `BackendDiscoveryBridge`.** Its dedup is
stamped at **emit** time, not write-confirmed time
(`backend_discovery_bridge.rs:374-379`, `:428`):

```rust
let new_fp = fingerprint(&listener.vip, &backends);
let prev_fp = view.last_written_fingerprint.get(service_id).copied();
if Some(new_fp) == prev_fp {
    // Dedup: no change since last successful write.
    continue;
}
```

The comment says *"since last successful write"*; the code has no notion
of a successful write. Worse, the view is fsynced **before** dispatch
(`reconciler_runtime.rs:1369-1373`, `:1493`), so the fingerprint outlives
the write it was meant to record. On the next tick the fingerprint
matches → `continue` → zero actions →
`view_has_backoff_pending` is hard-coded `false` for this reconciler
(`reconciler_runtime.rs:1573`) → `has_work` false → no self-re-enqueue
(`:1355-1356`, `:1521-1526`) → the broker drains empty.

This claim was adversarially attacked and **no recovery channel exists**:

- `workload_lifecycle.rs:184-195` dual-emits `EnqueueEvaluation` at the
  bridge — this re-*ticks* it but the dedup `continue` still emits
  nothing. Re-ticking is not re-writing.
- `handlers.rs:76` submits `WorkloadLifecycle` only.
- There is **no boot-time seed and no periodic full re-enqueue**. Positive
  control per § 3: the same greps *do* find the known sites
  (`action_shim/enqueue_evaluation.rs:58`, `exit_observer.rs:234/254/295/319`,
  `reconciler_runtime.rs:1524`) — the search could have found a boot seed;
  there is none.
- The fingerprint has no time-varying input (`bdb.rs:351-372`; `healthy`
  is a hardcoded `true`), pinned by `fingerprint_deterministic_across_runs`
  (`bdb.rs:694`).
- Self-healing is **structurally impossible**: `BackendDiscoveryBridgeState`
  (`bdb.rs:169-175`) has only `desired` and `actual` fields, and
  `hydrate_actual` (`reconciler_runtime.rs:2688-2696`) reads
  `alloc_status_rows()` — never `service_backends_rows`. The reconciler
  has no field through which it could ever observe the stored row.

**Precision on "permanent":** the *dropped row* is never retried, and no
tick count, re-enqueue, or reboot recovers it. The stale row stands for
as long as the backend set does not change again. It is unbounded in
time for the current state — not "no write to this key ever succeeds."
The operational impact (`ServiceBackendRow` advertising dead backends
into the DNS name index and `MtlsResolve`) is unaffected by that
scoping.

### 4.3 The root cause of § 4.2: the bridge does not converge

`BackendDiscoveryBridge` is nominally a `Reconciler` impl
(`backend_discovery_bridge.rs:309`) but is **structurally apply-once**.
Compare its `State` against its sibling's:

```rust
pub struct BackendDiscoveryBridgeState {   // bdb.rs:170
    pub desired: ServiceListenerSet,       // declared listeners
    pub actual:  RunningAllocSet,          // Running allocs — NOT what it manages
}

pub struct ServiceMapHydratorState {       // service_map_hydrator.rs:143
    pub desired: BTreeMap<ServiceId, ServiceDesired>,
    pub actual:  BTreeMap<ServiceId, ServiceHydrationStatus>,  // the managed resource
}
```

The bridge's `actual` is the **running alloc set** — a *second input* it
cross-products with `desired.listeners` to compute the row it should
write. It is not the actual state of the resource the bridge manages,
which is the `service_backends` rows. The bridge never reads those:
`hydrate_actual` (`reconciler_runtime.rs:2688-2696`) calls
`alloc_status_rows()`, and the only `service_backends_rows` call in the
runtime (`:1698`) belongs to `ServiceMapHydrator`.

So `last_written_fingerprint` is not retry memory layered on top of a
real diff — which is what the hydrator's `RetryMemory`
(`service_map_hydrator.rs:150-157`) is, sitting *beside* a genuine
`actual`. **The fingerprint *is* the diff.** That is the
`.claude/rules/reconcilers.md` adopt-and-skip symptom in different
clothing:

```rust
if resource.exists() { return Ok(()) }      // reconcilers.md § Symptoms
if Some(new_fp) == prev_fp { continue }     // bdb.rs:376
```

Presence of a matching hash *of what the bridge emitted* is taken as
proof the desired state is satisfied — Bar 1 of `reconcilers.md`
("converge, don't apply-once") is not met.

The View docstring (`:203-211`, `:237-241`) makes the inversion explicit,
and cites the rule it breaks:

> *"Carries the per-service fingerprint of the last row the bridge
> **successfully wrote** — the canonical *input* per
> `.claude/rules/development.md` § 'Persist inputs, not derived state'."*

Both halves are wrong. There is **no success signal** to record — `:428`
stamps the fingerprint on the *emit* path and `obs.write` returns
`Ok(())` on a dropped write, so "successfully wrote" is unverifiable by
construction. And the fingerprint is **derived state**, not an input: it
is a cached hash of `(vip, backends)` — a value the bridge computed. The
genuine input is the observed `ServiceBackendRow`, which the bridge
declines to read.

This is the root cause of § 4.2. The fire-once behaviour is not a
missing retry; it is the absence of convergence.

---

## 5. Operator-visible symptom

**The CLI reports success for an operation that did not take effect, and
then contradicts itself.**

Measured directly in Probe B Run 1: `overdrive job stop probe-job`
printed `Stopped workload 'probe-job'.` and exited **0**, while
`overdrive workload describe` continued to render `Running (c=522)` for
~52 s.

Three compounding surfaces:

1. **`overdrive job stop` / `deploy` report success.** `obs.write` returns
   `Ok(())` on a dropped write, so `?` never fires.
2. **The streaming lane broadcasts a lifecycle event that never happened.**
   All six shim write sites pair an unconditional `emit_event` immediately
   after `obs.write(...)?` (`action_shim/mod.rs:456`/`:457`, `:543`/`:544`,
   `:1103`/`:1136`, `:1282`/`:1332`, `:1496`/`:1530`, `:1600`/`:1620`).
   `overdrive deploy` shows the operator a transition that is not in the
   store; a subsequent `overdrive workload describe` reads the stale row
   and disagrees with what `deploy` just printed.
3. **Nothing is logged.** Two INFO lines across the whole window (§ 2.3).
   No warning, no metric, no counter. The exit observer's retry loop is
   equally blind: `run_with_retry` (`exit_observer.rs:413-447`) retries
   only on `Err`, and an LWW drop is `Ok`.

An operator diagnosing this sees a workload that ignores stop commands,
with a clean log and a zero exit code, self-resolving after a delay
proportional to the previous uptime. There is no signal pointing at LWW.

**Adjacent effect, deliberately not conflated:** in Run 1 the workload
process (`/bin/sleep 3600`) kept running after `job stop`, because the
driver's in-memory handle map is also lost on restart and `driver.stop`
returned `NotFound`, absorbed best-effort. That is a *separate* restart
effect. This probe measured the **observation write** being dropped and
did not isolate the process-survival effect; it is flagged here as an
open question (§ 9), not claimed as part of this defect.

---

## 6. Immune paths — confirmed

| Path | Mechanism | Evidence |
|---|---|---|
| Exit observer | **prior-derived**, no tick component at all | `worker/exit_observer.rs:541-544` — `counter: prior.updated_at.counter.saturating_add(1)`, where `prior` is the LWW-winner loaded at `:534` |
| `superseding_timestamp` (the GH #250 fix) | **hybrid** `max(tick+1, prior+1)` | `action_shim/mod.rs:1771-1777`. The `max` is what makes it immune: at `tick = 0` the base is 1 and the `max` selects `prior + 1`. Its own docstring (`:1763-1766`) states this. Reached from `:444` (both mTLS fail-closed arms). |
| `NodeHealthRow` | **wall-clock**, not tick-derived | `overdrive-worker/src/node_health.rs:55` — `clock.unix_now().as_secs()`. Counter ≈ 1.77 × 10⁹ and monotone across restarts. Contradicts a plain reading of ADR-0076 § 7d finding 3, which grouped it with the tick-derived rows; it does **not** share this defect (its own two-heartbeats-per-second collision is a different, benign issue). |
| `IssuedCertificateRow` | no `LogicalTimestamp`; append-only by serial | `apply_issued_certificate` returns `Ok(false)` on any existing serial |
| `WorkflowTerminal` / `Signal` | no `LogicalTimestamp`; in-memory only | `observation_backend.rs:457` |

Note the irony worth recording: **`superseding_timestamp`, added as the
GH #250 fix, is already the correct shape.** The remedy in § 7 is
largely "apply the existing helper's discipline to the other nine
sites."

---

## 7. Two findings beyond the brief's scope

### 7.1 The same collision fires **in-process, with no restart**

`lib.rs:2443-2456` drains the broker and iterates:

```rust
let pending = { let mut broker = state.runtime.broker(); broker.drain_pending() };
for eval in pending {
    if let Err(e) = run_convergence_tick(&state, &eval.reconciler, &eval.target, now, tick_n, deadline).await
```

`tick_n` increments only *after* the loop (`:2469`). So **every
evaluation drained in one iteration shares one `tick_n`**.
`backend-discovery-bridge` and `service-lifecycle` are both keyed
`workload/<id>` and both enqueued by the same `workload_lifecycle.rs`
dual-emit; if drained together they can emit `WriteServiceBackendRow` for
the same `service_id` with a **byte-identical** `(counter, writer)`, and
the second is silently dropped. `validate_reconcile_output`
(`reconciler_runtime.rs:1400`) inspects one reconciler's action vector
and cannot see a cross-reconciler tie.

This does not need a restart. It is the same defect class GH #250 fixed
for the *intra*-arm case, surviving at the *inter-reconciler* level.
Structurally confirmed; the concrete co-fire is an open question (§ 9).

### 7.2 A second, independent `tick_n` starting at zero

`lib.rs:2526` declares a second `let mut tick_n: u64 = 0;` in
`spawn_workflow_emit_drain`, feeding `TickContext { tick: tick_n, .. }`
at `:2545` and bumped at `:2548`. Anything forwarded through
`dispatch_with_workflow_intent` is stamped off that unrelated sequence.

**Currently latent** — `WorkflowRegistry::new()` is empty in production
per `.claude/rules/workflows.md`, so no first-party workflow emits today.
It goes live the moment one is registered. The brief was right to insist
these two loops be told apart: they are different paths, and the second
one is a trap rather than a present defect.

---

## 8. Recommended remedy

### 8.1 Assessment of the ADR-0076 § 7d candidate

The candidate — **monotone-against-prior, `max(tick+1, prior+1)`, at
every write site**, enabled by `build_alloc_status_row`'s now-required
`updated_at` parameter (`action_shim/mod.rs:272-284`) — is **the right
fix**, and this investigation strengthens the case for it:

- It is not a new mechanism. `superseding_timestamp` (`:1771-1777`)
  already implements exactly this and is already in production.
- Three of the five `AllocStatusRow` sites (`:1076`, `:1470`, `:1580`)
  already hold `prior_row` in scope. At those three the change is
  swapping `timestamp_for(tick, prior_row.node_id.clone())` for
  `superseding_timestamp(tick, &prior_row)` — a one-line edit each.
- The required-`updated_at` parameter means the compiler enumerates every
  site; none can be silently missed.
- ADR-0076 already foreclosed the alternatives by reasoning: Alt-K (a
  shim `AtomicU64`) is restart-unsafe without seeding from the store's
  high-water mark — which is *this* defect; Alt-L (a synthesized tick) is
  a lie about which tick the write belongs to; Alt-M (changing
  `dominates`) would break gossip idempotency.

**Cost.** Two of the ten sites do not currently read a prior row —
`StartAllocation` (`:1251`) and the netns fail-closed helper (`:526`) —
so each needs a `find_prior_alloc_row` lookup it does not do today: one
extra redb read on the alloc-start path. The remaining reconciler-side
sites (`ServiceBackendRow` ×2, `ServiceHydrationResultRow` ×2,
`ReconcileConflictRow`) are **harder**, because a `Reconciler::reconcile`
is a pure sync function with no store handle by construction
(ADR-0035/0036) — the prior row must arrive through `actual` state. For
`BackendDiscoveryBridge` that means adding a `service_backends` field to
`BackendDiscoveryBridgeState` and hydrating it (`reconciler_runtime.rs`
`hydrate_actual`, `:2688-2696`), which is a real design change, not a
one-line edit. **That, not the shim work, is where the cost sits.**

### 8.2 What the candidate would NOT fix

Four things. Each needs a separate decision:

1. **The silence.** A monotone counter stops *this* cause of dropped
   writes; it does not make *any future* dropped write observable.
   `apply_alloc_status_lww` returning `Ok(false)` up through a `write`
   that returns `Ok(())` with no log remains a blind spot. A structured
   `tracing::warn!` on the reject path — or surfacing the `bool` — is a
   separate, cheap, high-value change and arguably should land *first*,
   since it converts every future instance of this class from silent to
   diagnosable.
2. **`BackendDiscoveryBridge` not converging (§ 4.3).** Entirely
   independent of the counter, and the more important of the two. Because
   the bridge diffs against a fingerprint of what it *emitted* rather than
   against the rows it manages, *any* write failure — LWW, I/O, a future
   cause — is permanently forgotten. A monotone counter removes today's
   cause of the dropped write and leaves the structural blindness intact.
   See § 8.4 for the fix.
3. **The same-drain in-process collision (§ 7.1).** `max(tick+1, prior+1)`
   *does* fix it for the second writer if that writer sees the first
   writer's row — but within one drain the first write may not yet be
   visible to the second reconciler's already-hydrated `actual` state. The
   ADR should state explicitly whether the remedy covers this, and the
   answer likely needs a per-drain sub-counter or a distinct tick per
   evaluation.
4. **The second latent `tick_n` (§ 7.2).** Fixing the write sites does not
   remove a second unseeded counter that will feed the same
   `timestamp_for` the moment a production workflow is registered.

### 8.3 Alternative worth weighing in the ADR

**Seed `tick_n` from the store's high-water mark at boot.** One read of
`max(counter)` across observation tables during `run_server`, used to
initialize both `tick_n` declarations. Trade-offs:

- **For:** a handful of lines at one site; fixes all ten write sites, both
  loops, and § 7.1's cross-reconciler case at once, with no per-site
  edits and no reconciler `State` redesign.
- **Against:** it is the "persist derived state" shape
  `.claude/rules/development.md` warns about — the counter becomes a
  cached function of durable rows; it does not make any *individual*
  write monotone (so a same-tick tie still needs `superseding_timestamp`);
  and it is a Lamport clock bolted onto a per-node counter, which needs
  re-examination when Phase 2 introduces real multi-node
  `NodeId`s and gossip.

### 8.4 Make `BackendDiscoveryBridge` converge (separable, high value)

Hydrate the rows the bridge manages into its `actual` and diff
desired-vs-actual, per `.claude/rules/reconcilers.md` Bar 1:

- Add a `service_backends: BTreeMap<ServiceId, ServiceBackendRow>` (or
  the projection the diff needs) to `BackendDiscoveryBridgeState`
  (`bdb.rs:170`).
- Populate it in `hydrate_actual` (`reconciler_runtime.rs:2688-2696`).
  **The plumbing already exists** — `service_backends_rows(&service_id)`
  is called one arm over at `:1698` for `ServiceMapHydrator`, so this is
  the established cost shape in this runtime, not a new one.
- Delete `last_written_fingerprint` and its GC sweep
  (`bdb.rs:236-250`, `:428`, `:433`) — with a real `actual` the dedup is
  just `desired != actual`.

What this buys, none of which the counter fix delivers:

- **A dropped write self-heals on the next tick.** `actual` still shows
  the stale row, so the diff re-fires and re-emits. The permanence in
  § 4.2 disappears; `ServiceBackendRow` degrades to the same *transient*
  recovery-window shape as `WorkloadLifecycle`, bounded by § 3.
- **It is failure-cause-agnostic.** Works for LWW, I/O errors, and
  whatever comes next — the bridge stops needing to *know* why a write
  failed.
- **It removes the `development.md` violation** in § 4.3, rather than
  documenting around it.
- **It converts a fire-once applier into a real reconciler**, which is
  what the file already claims to be.

Trade-offs, honestly: one `service_backends_rows` read per service per
tick (precedented at `:1698`); the bridge's target is `workload/<id>`
while the rows are keyed by `ServiceId`, so `hydrate_actual` must derive
the service ids — the reconcile body already computes them per listener,
so the mapping exists but moves upstream; and the View's schema-evolution
tests (`crates/overdrive-core/tests/backend_discovery_bridge_types.rs`)
change shape when the field is removed.

### 8.5 Recommended sequence

1. **Make the drop observable (§ 8.2 fix 1)** — first and independently.
   Cheap, a pure addition, and it converts this whole class from silent
   to diagnosable. Everything else is easier to verify once a dropped
   write is visible.
2. **Make `BackendDiscoveryBridge` converge (§ 8.4)** — separable from
   the counter work, removes the only *unbounded* consequence found, and
   is a bug fix against `reconcilers.md` Bar 1 in its own right.
3. **The § 7d monotone-against-prior remedy** as the primary counter fix,
   with the boot-seed (§ 8.3) recorded in the ADR as
   explicitly-considered. Note that (2) materially *reduces* (3)'s cost:
   once the bridge hydrates `service_backends`, the prior row is already
   in `actual`, so the `max(tick+1, prior+1)` stamp at `bdb.rs:392` needs
   no further State redesign.
4. **The same-drain collision (§ 7.1) and the second `tick_n` (§ 7.2)** —
   decide explicitly in the ADR rather than leaving them implied.

Steps 1 and 2 are worth landing regardless of what the ADR decides about
the counter.

---

## 9. Open questions

1. **Production trigger frequency for the `BackendDiscoveryBridge`
   fire-once case.** `veth_provisioner::adopt_on_restart_recovery`
   (`lib.rs:2132`) re-adopts surviving allocs by id, so a *pure* restart
   with zero alloc churn yields an unchanged fingerprint → dedup → no
   write → no drop. The bug needs the running set to change after restart
   while `tick_n` is still below the pre-restart counter. Whether
   restarted workloads get fresh `AllocationId`s was not traced.
2. **Whether `service-lifecycle` and `backend-discovery-bridge` actually
   co-emit for the same `service_id` in one drain** (§ 7.1). The
   structural hazard is confirmed; the concrete co-fire needs a runtime
   trace.
3. **Whether a dropped `ReconcileConflictRow` matters.** Tick-derived and
   LWW-guarded, so the regression applies, but the write is already
   declared best-effort (`reconciler_runtime.rs:1449-1458`) and the
   tracing channel is unaffected.
4. **The driver-handle-loss effect (§ 5).** On restart the driver's
   in-memory handle map is lost, so `driver.stop` returns `NotFound` and
   the workload process survives. Observed but not isolated; it may be a
   separate defect and deserves its own investigation.
5. **`SvidLifecycle` / `ServiceLifecycle` reconcile bodies** were not
   read. Both stay enqueued via `view_has_backoff_pending`
   (`reconciler_runtime.rs:1623`, `:1643`), so a suppressed tick does not
   drain the broker — but whether either carries an emit-time view stamp
   of the `BackendDiscoveryBridge` shape is unverified.

---

## 10. Falsifications tested and rejected

Every falsification path the brief named was tested; none held.

| Falsification | Result |
|---|---|
| Data dir wiped / table truncated on boot | **Rejected.** redb `Database::create` opens with `.truncate(false)`; no `remove_file`, `drain`, `clear`, or `remove_table` anywhere in `observation_backend.rs`; no `Drop` impl. Positive control per § 3: the same grep family finds 30+ sites elsewhere in `crates/`. |
| Something outside Rust wipes the data dir | **Rejected.** Production default is `$XDG_DATA_HOME/overdrive`, else `$HOME/.local/share/overdrive` (`crates/overdrive-cli/src/main.rs:248-256`), passed unmodified into `ServerConfig::data_dir`. No systemd units, no Dockerfiles, no tmpfs, no CI wipe. Positive control: the same `find` returns `lefthook.yml`, `infra/lima/overdrive-dev.yaml`, `.github/workflows/ci.yml`. |
| `tick_n` seeded from persistent state on an unread path | **Rejected.** Both declarations (`lib.rs:2434`, `:2526`) are literal `0`. |
| Every surviving-alloc write is prior-derived in practice | **Rejected.** Five production `AllocStatusRow` sites are tick-derived; Probe B drove one (`StopAllocation`, `:1580`) end-to-end and watched it lose. |
| The writer tiebreak rescues it | **Rejected.** `node_id` is the literal `"local"` (`lib.rs:1701`); `"local" > "local"` is `false`. Confirmed empirically — `tick=5999`/stamp 6000 loses against prior 6000. |
| The reproduction simply does not reproduce | **Rejected.** Reproduced at two levels, four measurement points. |

---

## 11. Blockers requiring user approval

**A GitHub issue should be opened for this defect, and no issue was
created.** Per `CLAUDE.md` § "Deferrals require GitHub issues — AND user
approval BEFORE creation", agents do not run `gh issue create`
unilaterally. Surfacing it here for approval. ADR-0076 § 7d records the
finding with **no forward pointer**, which is precisely the hand-wavy
shape that rule exists to prevent; that gap is now more acute, because
the finding has gone from "verified from source, not reproduced" to
"reproduced end-to-end, blast radius wider than recorded."

Recommended scope for the issue, should it be approved:

- The cross-restart regression across all **ten** production tick-derived
  durable write sites (§ 4.1).
- The **silent** LWW reject (§ 8.2 fix 1) — separable and worth landing
  first.
- `BackendDiscoveryBridge` not converging (§ 4.3) — independent of the
  counter, and a `reconcilers.md` Bar 1 violation in its own right. This
  may warrant a **separate** issue: it is a reconciler-discipline bug that
  this investigation surfaced, not an LWW bug.
- The in-process same-drain collision (§ 7.1) — **needs no restart**.
- The second latent `tick_n` (§ 7.2).

ADR-0076 § 7d finding 3's grouping of `NodeHealthRow` with the
tick-derived rows should also be corrected — it is wall-clock-derived and
immune (§ 6). Per `CLAUDE.md`, ADR edits go through the architect agent,
not inline.

---

## 12. Method notes

- All test execution went through `cargo xtask lima run --` per
  `.claude/rules/testing.md`. Kernel: Lima VM, Ubuntu 24.04.
- Probe code was throwaway and removed. `git status --porcelain` after the
  investigation shows only the pre-existing ` M .serena/project.yml`. No
  commits, no issues created, no ADR or feature artifact touched.
- Probe B's `serve` boot was unblocked through the production
  systemd-creds KEK path, **not** a test seam — a `SimKek` or dev env-var
  would have made the reproduction unfaithful to production composition.
- One methodological correction worth recording: an early census grep used
  `rg -rn`, where `-r` is ripgrep's **replace** flag; it silently
  substituted `n` for every match and produced misleading output. It was
  re-run correctly before any conclusion was drawn. This is § 3 in
  miniature — the tool gap looked like data.
