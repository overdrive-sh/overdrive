# RCA-251 — dial-by-name keeps resolving a stopped workload's frontend `F` (no NXDOMAIN after `job stop`)

Issue: overdrive-sh/overdrive#251
Method: Toyota 5 Whys (multi-causal), evidence at every level (file:line or pasted Lima output).
Verdict: **Mechanism 3** (NOT mechanism 1 or 2 as framed in the issue). Pinned with a live Lima population-diff probe.

> **⬆ AS-LANDED (2026-06-28):** the shipped fix (`88679ed8`, `Closes #251`) is the
> `service_vip_release_emission` **gate-swap** (release-on-deletion / retain-on-stop) —
> **NOT** the "Option A" prescribed below. The *diagnosis* (mechanism 3) stands and is
> what the landed fix addresses; the *prescription* (§ "The pinned fix-site", §
> "Proposed minimal fix") is **SUPERSEDED** — see § **As-Landed Addendum** below.

---

## TL;DR (decision-ready)

The zero-backend `service_backends` retraction row **is never written on the operator-stop path** — but **not** because the bridge fails to re-tick (it does) and **not** because the alloc fails to leave the Running set (it does leave it). It is because, on the same terminal stop transition, `WorkloadLifecycle` emits `Action::ReleaseServiceVip`, whose executor **evicts the Service's VIP from the allocator memo**. The bridge's `hydrate_bridge_desired_listeners` reads that memo (`allocator.get(&digest)`); once it is gone the hydrate returns **empty `desired.listeners`**, so the bridge's `reconcile` loop body never runs and **emits no retraction**. The stale non-empty `service_backends` row survives forever, the `name_index` never sees a drop, and `frontend_for("server")` keeps returning `Some(F)`.

It is a **race between two terminal effects of one stop**: (a) the VIP-memo release and (b) the bridge's zero-backend retraction — where (a) destroys the input (b) needs. The population diff proves the race: a **server-only** fixture writes the retraction (bridge wins the race); a **server+client** fixture does not (release wins the race).

**Pinned fix-site (SUPERSEDED — see § As-Landed Addendum):** the *prescribed* site below was
`reconciler_runtime.rs::hydrate_bridge_desired_listeners`; the **shipped** fix-site is
`crates/overdrive-core/src/reconcilers/workload_lifecycle.rs::service_vip_release_emission`
(L901–933). `hydrate_bridge_desired_listeners` was **not touched** — retaining the VIP keeps
its memo present, so the bridge's existing projection works unchanged.

---

## ⬆ As-Landed Addendum (2026-06-28) — the shipped fix is the `service_vip_release_emission` gate-swap, NOT Option A

**The fix that shipped (`88679ed8`, `Closes #251`) is a THIRD option, not the "Option A"
recommended in § "Proposed minimal fix" below.** Those prescription sections are **SUPERSEDED**
by this addendum. The **diagnosis is unchanged and correct** — mechanism 3 (the
`ReleaseServiceVip` executor evicts the VIP memo that `hydrate_bridge_desired_listeners`
depends on, racing the zero-backend retraction), pinned by the live population-diff probe in
§ "Evidence". Only the *prescription* changed.

**As-landed fix-site:** `crates/overdrive-core/src/reconcilers/workload_lifecycle.rs::service_vip_release_emission`
(L901–933) — NOT `reconciler_runtime.rs::hydrate_bridge_desired_listeners`, which was **not
touched**. The release-emission gate changed from **release-on-terminal**
(`actual.allocations…any(|r| r.terminal.is_some())`) to **release-on-deletion**
(`if desired.job.is_some() { return None; }`). A stopped-but-still-declared Service now
**RETAINS** its VIP, so the memo the bridge reads is **never evicted on stop** — the bridge's
existing dependency is satisfied unchanged, the zero-backend retraction fires, the `name_index`
folds it, and resolution collapses to NXDOMAIN. **No bridge edit** (the root of the race is
*removed*, not worked around).

**Why "Option A" was REJECTED:** Option A keeps **release-on-terminal** (this doc's own § "Risk":
*"`ReleaseServiceVip` still runs and returns the VIP to the pool"*) and makes the bridge robust
to the eviction. That contradicts the **withhold-not-release** contract — the VIP is an
*identity* retained across a transient stop and released only on logical-workload deletion,
**symmetric with the dial-by-name frontend `F`** (ADR-0072). Retaining the VIP (rather than
working around its release) was ratified as the **ADR-0049 §6 amendment (2026-06-28)**, backed
by prior-art research (`docs/research/orchestration/service-vip-dns-lifecycle-stop-vs-delete-k8s-nomad.md`):
both Kubernetes ClusterIP and Consul mesh VIP retain the stable virtual IP across
stop/scale-to-zero and release only on delete — Overdrive's release-on-terminal was the
outlier. (This RCA's § "Why here, not elsewhere" reasoning — *"removing/deferring the release
would regress the VIP-reuse contract (ADR-0049)"* — is what pointed at Option A; the ADR-0049
amendment supersedes that premise.)

**Deletion-path note (ADR-0049 D3):** the v1 hydrator (`read_job`) zeroes
`service_spec_digest` alongside `desired.job` on intent withdrawal, so the new
release-on-deletion branch is **inert on the v1 convergence path** until a deletion verb
supplies the digest at hydrate time — tracked in **#211**. Today the VIP (like `F`) is retained
for the process lifetime. The inert path is pinned by
`workload_lifecycle.rs::withdrawn_service_without_digest_emits_no_release` and the inline note
at `vip_allocator_lifecycle.rs:733-748`.

---

## Scope

- **In scope:** the operator-stop → `Terminated{by:Operator}` → bridge-retraction → `name_index` WITHHOLD path for a **dial-by-name Service** whose intent is retained (`job stop` keeps the Service declared, #249).
- **Out of scope (confirmed unrelated):** `name_index::apply_row` empty-row eviction (correct); the `healthy:true` hardcode (orthogonal — a stopped alloc leaves the Running set entirely); the `frontend_for` pure-reader contract (correct); `FrontendAddrAllocator` F-retention (correct, and must NOT change — withhold-not-release).

---

## Evidence — the decisive Lima population diff (§5 / §11 of debugging.md)

A temporary probe (`rca251_probe_dump_service_backends_after_stop`, appended to the nxdomain Tier-3 file, run as root under Lima, **reverted after**) dumped the `service_backends` + `alloc_status` rows over a 10 s window after a production `POST /v1/jobs/server/stop`, and ran `getent` before/after.

**Run A — server only deployed:**
```
[RCA251 PRE-STOP] alloc alloc-server-0 state=Running counter=2
[RCA251 T+500ms] service_backends sid=ServiceId(7016738190153817369) vip=10.96.0.2 backends.len=0 counter=4
[RCA251 T+500ms] alloc alloc-server-0 state=Terminated counter=3
... (backends.len=0 persists)
[RCA251 VERDICT] any zero-backend service_backends row in 10s post-stop window? true
```
→ a zero-backend retraction row **DID** land. (Bridge won the race.)

**Run B — server AND client deployed (the realistic scenario the failing test uses):**
```
[RCA251 PRE-STOP getent] resolved_frontend = Some(10.98.0.1)
[RCA251 PRE-STOP] service_backends sid=ServiceId(7016738190153817369) vip=10.96.0.2 backends.len=1 counter=3   <- server
[RCA251 PRE-STOP] service_backends sid=ServiceId(798176193463013194) vip=10.96.0.3 backends.len=1 counter=4   <- client
[RCA251 PRE-STOP] alloc alloc-server-0 state=Running counter=3
[RCA251 T+500ms] service_backends sid=ServiceId(7016738190153817369) vip=10.96.0.2 backends.len=1 counter=3   <- server row STAYS non-empty
[RCA251 T+500ms] alloc alloc-server-0 state=Terminated counter=9
... (server service_backends stays backends.len=1 counter=3 for the full 10s)
[RCA251 POST-STOP getent] v4_addrs=[10.98.0.1] exit=Some(0) is_nxdomain=false
[RCA251 VERDICT] any zero-backend service_backends row in 10s post-stop window? false
```
→ NO zero-backend retraction. The server's row is **frozen at counter=3** (the original Running write) — the bridge **never rewrote it**. getent keeps returning the stale `F = 10.98.0.1`. **This reproduces #251 exactly.**

The single differentiator between A and B is **a second Service workload being deployed**, which is what makes the bridge's post-Terminated retraction tick lose the race to the VIP-memo release (timing, see WHY-3/WHY-4). The diff isolates the mechanism to *the bridge's desired-listener hydrate going empty after the VIP release*, not to the alloc state, the re-enqueue, or the name_index.

---

## Five Whys (single dominant branch; the issue's two candidate branches are falsified)

**PROBLEM:** After `POST /v1/jobs/server/stop` converges the alloc to `Terminated`, `getent server.svc.overdrive.local` keeps returning the stable `F` (`[10.98.0.1] code 0`) indefinitely; never `NXDOMAIN`.

**WHY 1 — The responder keeps answering `F` for the stopped `<job>`.**
`name_index::frontend_for` returns `Some(F)` because `is_resolvable("server")` is still true — the `<job>` key is still in `by_name`.
[Evidence: `name_index.rs:333-349` `frontend_for` returns the allocator binding iff `is_resolvable`; `:247-249` presence == resolvable. Probe Run B: `getent ... -> [10.98.0.1] (code 0)` post-stop.]

**WHY 2 — `is_resolvable("server")` is still true because the `name_index` never folded a zero-backend `service_backends` row for the server's `service_id`.**
The drain only drops a `<job>` when `apply_row` is fed a row whose healthy set is empty for that service; no such row ever arrives.
[Evidence: `name_index.rs:400-403` drain folds `ServiceBackend` rows via `apply_row`; `:181-230` `apply_row` evicts a `<job>` only when the incoming row contributes no healthy backend for the service. Probe Run B: the server's `service_backends` row stays `backends.len=1 counter=3` the whole window — no zero-backend row is ever produced.]

**WHY 3 — No zero-backend `service_backends` row is written because the `BackendDiscoveryBridge`, when it re-ticks after `Terminated`, hydrates an EMPTY `desired.listeners` and its reconcile loop body never executes.**
`reconcile` iterates `for (service_id, listener) in &desired.desired.listeners { ... emit WriteServiceBackendRow ... }`. With zero desired listeners the loop never runs → zero actions → no retraction. (The actual-side Running set IS correctly empty — that is not the gap.)
[Evidence: `backend_discovery_bridge.rs:344-429` — the per-listener loop is the ONLY emitter of `WriteServiceBackendRow`; an empty `desired.listeners` emits nothing. `reconciler_runtime.rs:2649-2656` the actual-side filter `state == Running` correctly drops the Terminated alloc, so `actual.running` IS empty — confirming the actual side is not the problem.]

**WHY 4 — `desired.listeners` hydrates empty because `hydrate_bridge_desired_listeners` consults the VIP allocator memo (`allocator.get(&digest)`), and that memo entry has already been RELEASED on the same terminal stop transition.**
The hydrate computes the Service's `service_id`s from the allocator-issued VIP. If `get(&digest)` returns `None`, the hydrate logs `bridge.allocator_memo_absent` and returns an EMPTY map.
[Evidence: `reconciler_runtime.rs:2120-2135` — `let Some(assigned_vip) = assigned_vip_opt else { ...debug "bridge.allocator_memo_absent"...; return Ok(BTreeMap::new()); }`. The intent itself is retained (`IntentKey::for_workload`, L2090-2099, present), so the issue's "hydrate_desired keeps the listeners" claim is *half* right — it keeps the INTENT but not the VIP, and the VIP is the load-bearing input.]

**WHY 5 (ROOT CAUSE) — On a Service alloc reaching a terminal claim, `WorkloadLifecycle` emits `Action::ReleaseServiceVip`, whose executor evicts the digest from the allocator memo; the bridge's desired-listener projection has an undocumented data-dependency on that memo, so the release destroys the very input the bridge's retraction needs — and the release lands BEFORE the bridge's post-Terminated retraction tick.**
The two terminal effects of one stop race, and on a multi-Service host the release wins.
[Evidence:
- Emit: `workload_lifecycle.rs:150-156` calls `service_vip_release_emission`; `:891-909` returns `Action::ReleaseServiceVip` once `actual.allocations.values().any(|row| row.terminal.is_some())` (true the moment the action-shim's `StopAllocation` writes `terminal: Some(Stopped{Operator})` — `action_shim/mod.rs:1576-1595`).
- Execute: `action_shim/release_service_vip.rs:55-77` → `PersistentServiceVipAllocator::release(digest)` which "removes the entry from the in-memory memo AND from the IntentStore `allocator_entries` table" (`:4-8`).
- Post-release `get` is `None`: `allocators/service_vip.rs:142-143` `get` reads `by_digest`; `:154` `release` does `by_digest.remove(digest)`; persistent wrapper `allocators/persistent_service_vip.rs:251` `get` / `:262` `release`.
- Race confirmed by the population diff: server-only → bridge retraction wins (Run A); server+client → release wins (Run B).]

→ **ROOT CAUSE:** the bridge's desired-listener hydrate depends on the *transient* VIP memo, which is *released on terminal stop*; the release races ahead of the bridge's zero-backend retraction and nulls its input. The WITHHOLD-on-stop seam is therefore **never driven** on the operator-stop path whenever the release wins the race.

---

## Backwards-chain validation

If the root cause holds: on stop, `ReleaseServiceVip` removes the memo → next bridge tick for `job/server` hydrates empty listeners → no `WriteServiceBackendRow{backends:[]}` → `name_index` never folds a zero-backend row → `is_resolvable("server")` stays true → `frontend_for` returns `Some(F)` → `getent` resolves `F` forever. ✔ Matches the observed symptom and the Run B trace exactly. The server's `service_backends` row frozen at `counter=3` is the direct fingerprint of "the bridge never re-wrote it." ✔

Why Run A differs (no contradiction): with only one Service, the bridge's post-Terminated tick happens to drain before the `ReleaseServiceVip` action executes, so the memo is still present, the listener set hydrates, and the retraction lands. The race is order-of-broker-drain sensitive; a second workload perturbs the drain ordering enough that release wins. The fix must make the bridge robust to *either* order, not rely on winning the race.

---

## Falsified hypotheses (the issue's two candidates + the static "ruled out")

- **Mechanism 1 as framed ("the alloc does not leave the bridge's Running set on operator stop") — FALSE.** The actual-side hydrate filters `state == Running` (`reconciler_runtime.rs:2651-2654`); the Terminated alloc *does* drop. Run B shows `alloc-server-0 state=Terminated` while the stale row persists — the Running set is empty yet no retraction is written. The gap is on the **desired** side, not the actual side.
- **Mechanism 2 ("the bridge is not re-enqueued on the operator-stop write") — FALSE.** The bridge IS re-enqueued, on TWO paths: the `WorkloadLifecycle` Stop branch dual-emits `EnqueueEvaluation(backend-discovery-bridge, job/<id>)` (`workload_lifecycle.rs:181-195`, predicate includes `StopAllocation` at `:315-323`; acceptance test `stop_allocation_branch_dual_emits_bridge_enqueue`), and the exit observer re-enqueues post-Terminated (`exit_observer.rs:253-256`). The bridge runs — it just hydrates empty.
- **Issue's static "hydrate_bridge_desired_listeners is ruled out" — PARTIALLY WRONG.** It correctly notes the hydrate does not consult the *stop sentinel*. It MISSED that the hydrate's listener projection requires the *VIP memo*, which the terminal `ReleaseServiceVip` evicts. The intent is retained; the VIP is not. That is the actual defect site.

---

## The pinned fix-site & why it (not the adjacent candidates) is correct

> **SUPERSEDED (2026-06-28) — see § As-Landed Addendum.** This section reasoned toward Option A
> (keep release-on-terminal, make the bridge robust to the eviction). The shipped fix instead
> *removed the release on stop* (`service_vip_release_emission` → release-on-deletion), so the
> bridge was not touched. The bullet below — *"removing/deferring the release would regress the
> VIP-reuse contract"* — was the premise the ADR-0049 §6 amendment reversed.

**Site:** `crates/overdrive-control-plane/src/reconciler_runtime.rs::hydrate_bridge_desired_listeners` (≈L2080–2154), at the VIP-memo dependency (L2120–2135).

**Why here, not elsewhere:**
- **Not the exit observer / WorkloadLifecycle enqueue (mechanism 2):** the bridge already re-ticks; adding more enqueues changes nothing while the hydrate returns empty.
- **Not `backend_discovery_bridge::reconcile`:** the reconcile logic is correct given non-empty `desired.listeners`; the unit test `reconcile_terminated_alloc_drops_backend` already proves it drops backends. The defect is that it never *receives* the listeners.
- **Not `name_index` / `frontend_for`:** correct; they faithfully reflect the (missing) retraction row.
- **Not `ReleaseServiceVip` emission timing:** removing/deferring the release would regress the VIP-reuse contract (ADR-0049) and the `released_for_terminal` idempotency; the release is correct — the bug is the bridge's *dependence* on the memo for a stopped-but-declared Service.

The fix must let the bridge project the Service's listener set (VIP + per-listener `service_id`s) for a stopped-but-declared Service **without** depending on the still-present VIP memo — so the zero-backend retraction is emitted regardless of whether `ReleaseServiceVip` has already run.

---

## Proposed minimal fix (smallest change that drives the WITHHOLD seam, without releasing `F`)

> **SUPERSEDED (2026-06-28) — NOT the shipped fix; see § As-Landed Addendum.** Neither Option A
> nor Option B was taken. The shipped fix is a third option: retain the VIP until deletion
> (`service_vip_release_emission` gate-swap), so the memo is never evicted on stop and the
> bridge needs no change. Option A was rejected because it keeps release-on-terminal, which
> contradicts the withhold-not-release amendment (ADR-0049 §6, 2026-06-28).

The bridge needs the VIP to derive the listeners' `service_id`s. Two candidate shapes — recommend **Option A**:

**Option A (recommended — no new public API):** in `hydrate_bridge_desired_listeners`, when the allocator memo lookup returns `None` for a Service intent that is **still declared**, fall back to the VIP carried on the **last `service_backends` row(s)** the bridge already wrote for this workload, instead of returning an empty map. The `ServiceBackendRow` carries `vip` and `service_id` (`reconciler_runtime.rs` reads them; the probe shows `vip=10.96.0.2 sid=...`), and `ObservationStore::all_service_backends_rows` / the keyed read are already available to the runtime. Re-derive the `ProjectedListener` set from those rows so the loop body runs and emits the zero-backend retraction (Running set is already empty). This is a pure hydrate-path change in one function; it adds no public type/method/variant/trait/parameter. **It needs no API surface beyond what the runtime already calls.** The existing `ServiceId::derive` path stays the source of `service_id`s when the memo IS present; the fallback only fires on the released-memo-but-declared-intent case.

**Option B (defer the release until the bridge has retracted):** make `service_vip_release_emission` gate on "the bridge has already written the zero-backend `service_backends` row for this workload" (an ordering dependency between the two terminal effects). This is heavier — it couples two reconcilers' terminal ordering and risks the inverse race — and is NOT recommended for a minimal fix.

**API-surface flag (CLAUDE.md "implement to the design — never invent API surface"):** Option A uses only the existing `ObservationStore` read surface and the existing `ProjectedListener` / `ServiceId::derive` shapes — **no new public API**. If, during implementation, the crafter finds the VIP cannot be recovered from the existing `service_backends` read surface without a new accessor, that is a design-gap to **STOP and surface**, not to improvise a new public method. The fix as scoped does not require it.

**Withhold-not-release invariant (must hold):** this fix touches only the `service_backends` retraction path. It does NOT call `FrontendAddrAllocator::release` and does NOT touch `<job> → F`. `F` is retained across the zero-healthy window (release is logical-workload-DELETION only, #211). The bridge releasing the *VIP memo* (10.96.x, the service-map VIP) is unrelated to the *frontend* `F` (10.98.x) the allocator retains. ✔

---

## Files affected

**Production (the fix) — AS-LANDED (`88679ed8`):**
- `crates/overdrive-core/src/reconcilers/workload_lifecycle.rs` — `service_vip_release_emission`
  (L901–933): release-emission gate swapped from release-on-terminal to release-on-deletion
  (`desired.job.is_none()`), plus the `released_for_terminal` → `released_for_deletion` View-field
  rename (`#[serde(alias = "released_for_terminal")]`). `reconciler_runtime.rs::hydrate_bridge_desired_listeners`
  was **NOT** modified (the originally-prescribed Option-A site).

**Regression test to un-ignore (primary Tier-3 oracle):**
- `crates/overdrive-control-plane/tests/integration/dns_responder_nxdomain.rs::after_backend_stops_the_job_is_withheld_nxdomain_never_a_stale_addr` — remove the `#[ignore]` (body intact). Note its docstring/`#[ignore]` reason text also references the (now-corrected) mechanism framing — update the prose to cite mechanism 3 when un-ignoring.

---

## Regression-test plan

1. **Primary oracle (un-ignore):** `after_backend_stops_the_job_is_withheld_nxdomain_never_a_stale_addr` (Tier-3, Lima, root). It deploys **both** server and client (the population that triggers the race), so it exercises the failing path directly. Run sequentially (`--test-threads=1`) per the file's `:53`-singleton note.

2. **Cheaper unit-level companion (fails RED on the pinned seam):** add a `reconciler_runtime` unit/acceptance test for `hydrate_bridge_desired_listeners` that, with the Service intent present but the **VIP memo released** AND a prior non-empty `service_backends` row in obs, asserts the returned `desired.listeners` is **non-empty** (so the bridge will emit the retraction). On the un-fixed code this returns empty (RED); on the fixed code it returns the projected listener (GREEN). This pins the exact production seam without the full Tier-3 boot and is the inner-loop guard. (Mirror the existing `hydrate_actual_*` test shape in the same module, ≈L3513+.)

---

## Risk assessment

- **Busy-loop risk:** LOW. The fallback only fires when (intent present ∧ memo absent ∧ a prior `service_backends` row exists). It produces exactly one zero-backend `WriteServiceBackendRow` (the Running set is empty); the bridge's `last_written_fingerprint` dedup then suppresses further writes, and the GC sweep drops the fingerprint once the row is retracted. No re-enqueue churn — the action vector drains to empty on the next tick.
- **Interaction with #249 (operator-stop stickiness):** NONE adverse. The Service intent is retained by design; the fix relies on that retention (it is what keeps the bridge enqueued and the intent readable). The fix does not clear or consult the stop sentinel.
- **Interaction with the crash/exit path (which already works):** SAFE. On crash, the memo may still be present (no operator-stop terminal release timing) OR the same fallback applies — either way the bridge emits the retraction. The fix is additive to the memo-present path (unchanged when `get(&digest)` is `Some`).
- **Interaction with VIP reuse (ADR-0049):** SAFE. `ReleaseServiceVip` still runs and returns the VIP to the pool; the fix only changes how the bridge *projects listeners during the same terminal window*, not the release itself.
- **DST / mutation surface touched:** `hydrate_bridge_desired_listeners` is a runtime hydrate fn (not a pure reconcile body), so it is outside the core mutation gate, but the companion unit test gives it a kill-able assertion. No `Action` enum or reconcile-body change → no new DST replay-equivalence surface. The bridge's `reconcile` is unchanged (its mutation coverage stands).
- **Withhold-not-release contract:** preserved — no `FrontendAddrAllocator::release` call added; `F` retained (Tier-1 gated at 01-04, untouched).

---

## Appendix — evidence index (file:line)

- Symptom & stale-row freeze: Lima probe Run A/B (pasted above).
- `frontend_for` / resolvability: `dns_responder/name_index.rs:247-249`, `:333-349`.
- Drain folds rows: `name_index.rs:400-403`; `apply_row` eviction: `:181-230`.
- Bridge emits only inside the per-listener loop: `reconcilers/backend_discovery_bridge.rs:344-429`.
- Actual-side Running filter (drops Terminated): `reconciler_runtime.rs:2649-2656`.
- Desired-listener hydrate VIP dependence + empty-on-`None`: `reconciler_runtime.rs:2090-2099`, `:2120-2135`, `:2136-2153`.
- `ReleaseServiceVip` emission on terminal: `reconcilers/workload_lifecycle.rs:150-156`, `:891-909`.
- `StopAllocation` writes `terminal: Some(Stopped{Operator})`: `action_shim/mod.rs:1576-1595`.
- `ReleaseServiceVip` executor evicts memo: `action_shim/release_service_vip.rs:4-8`, `:55-77`.
- `get`/`release` over `by_digest`: `overdrive-dataplane/src/allocators/service_vip.rs:142-143`, `:154`; `persistent_service_vip.rs:251`, `:262`.
- Bridge re-enqueue (mechanism 2, falsified): `workload_lifecycle.rs:181-195`, `:315-323`; `worker/exit_observer.rs:253-256`; acceptance `workload_lifecycle_enqueues_bridge_on_alloc_transitions.rs:281-336`.
- Production boot wires exit observer WITH runtime: `lib.rs:2279-2286`.
