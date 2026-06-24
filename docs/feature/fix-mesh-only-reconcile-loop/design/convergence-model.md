# DESIGN — `ServiceMapHydrator` convergence-model realignment (fingerprint-domain fix)

**Wave:** DESIGN (DES-exempt — a convergence-model defect fix, not a roadmap step).
**Architect:** Morgan (nw-solution-architect), Propose mode.
**Date:** 2026-06-24.
**Governs:** `crates/overdrive-core/src/reconcilers/service_map_hydrator.rs`
(`reconcile`, `should_dispatch`), and the SSOT it implements —
`docs/feature/phase-2-xdp-service-map/design/architecture.md` § 8.
**Supersedes (for the convergence axis):** the 02-02 D3 ratification and
RCA § 5 "Option A" recommendation — both disqualified by the executed
evidence in RCA § 10 (the bug has three faces, Option A patches one).
**Inputs read:** `rca.md` (full, incl. § 10 ADDENDUM); `architecture.md`
§ 8; `service_map_hydrator.rs` (`reconcile` :311-455, `should_dispatch`
:544-575, `project_service_desired` :106-134); `dataplane_update_service.rs`;
`register_local_backend.rs` / `deregister_local_backend.rs`; `traits/dataplane.rs`
:180-222 (the `update_service` empty-purge contract); `fingerprint.rs`;
`overdrive-sim/src/invariants/service_map_hydrator.rs` :81-292; the tests
`mesh_backend_lb_gate.rs` and `service_map_hydrator_dispatch.rs`.

---

## 1. The defect in one sentence

The hydrator measures convergence as `actual.fingerprint == desired.fingerprint`,
but **`desired.fingerprint` covers the FULL backend set while every signal the
system actually produces about `actual` covers only the PROGRAMMED REMOTE
subset** — so the equality is structurally unreachable for any service that
isn't remote-only, and three distinct non-converging faces result (RCA § 10.3,
executed evidence § 10.2).

The fingerprint serves **three roles** today, all keyed on the wrong (full-set)
domain:

| Role | Site | Correct domain |
|---|---|---|
| **Dispatch decision** — "re-program the dataplane?" | `should_dispatch` arg `desired_fingerprint` (:325) | the programmable-remote projection |
| **Convergence comparison** — "is the dataplane confirmed at the state I drove it to?" | `Completed.fingerprint == desired_svc.fingerprint` (:445, :566) | the programmable-remote projection |
| **Action payload** — which backends to push | `DataplaneUpdateService.backends` (:407) | the programmable-remote projection |

All three must key on the **same** backend set the action carries and the shim
hashes back. The full-set fingerprint (`desired_svc.fingerprint`) is **not** that
set whenever gating drops any backend.

---

## 2. Chosen model — Option (i): converge over the programmable projection

**Decision: the hydrator's dispatch decision AND convergence comparison are
both keyed on a NEW derived value, `programmed_fingerprint = fingerprint(vip,
remote_survivors)` — the fingerprint over the EXACT backend set the emitted
`DataplaneUpdateService` carries and the action-shim writes back in its
`Completed` row.** This is task option (i) (compute the convergence fingerprint
over the post-gate programmable projection), chosen over (ii) (carry the
full-set fingerprint through the action) for the reasons in § 7.

The full-set `desired_svc.fingerprint` is **retained, unchanged, for its OTHER
role**: it is the *identity/churn* key the runtime's evaluation broker uses to
collapse a row-change burst into one pending evaluation, and the value
`project_service_desired` stamps onto `ServiceDesired`. It is no longer the
convergence target. Nothing about its computation, its persistence, or its wire
shape changes — see § 7 "What does NOT change."

### 2.1 Why this needs an empty-purge for the degenerate set

The programmable projection of an **all-mesh** service is `∅`. Under this model
`programmed_fingerprint = fingerprint(vip, [])` — a well-defined, deterministic
value. For `actual` to ever reach it, the hydrator must DRIVE the dataplane to
the empty state and OBSERVE confirmation. The mechanism that does both already
exists and is contract-blessed:

- `Dataplane::update_service(frontend, [])` is the documented **per-proto purge**
  (`traits/dataplane.rs:197-204`, ADR-0060 D4).
- `dataplane_update_service::dispatch` over an empty-backends action writes
  `Completed{ fingerprint: fingerprint(vip, []), .. }` (`dataplane_update_service.rs:111,138`
  — `fp = fingerprint(vip, backends)` is total over an empty slice).

So **a service whose programmable-remote projection is empty emits exactly one
empty-backends `DataplaneUpdateService` purge, which round-trips to
`Completed{fingerprint(vip,[])}` and settles.** This is the all-mesh decision
(§ 4) and — for free, because the same gate now emits a purge on the transition
— the Finding-2 teardown fix (§ 6).

---

## 3. The pinned contract for the crafter

> **The crafter is forbidden from inventing API surface beyond what is named
> here (repo CLAUDE.md § "Implement to the design").** Every changed signature,
> the exact fingerprint computation, and the precise emit/compare conditions are
> pinned below. Where a gap remains it is surfaced as a blocker in § 9, NOT left
> to crafter latitude.

### 3.1 New derived value — `programmed_fingerprint` (NOT persisted)

Inside `reconcile`, per service, AFTER the mesh/local/remote partition is
computed and BEFORE the dispatch decision, compute:

```rust
// The post-gate REMOTE survivors are the only backends the dataplane
// (XDP/REVERSE_NAT) is driven to program. Recompute every tick from
// inputs (`backends`, `workload_subnet`, `host_ipv4`) — NEVER persisted
// (.claude/rules/development.md § Persist inputs, not derived state).
let programmed_fingerprint: BackendSetFingerprint =
    crate::dataplane::fingerprint::fingerprint(&desired_svc.vip, &remote_backends);
```

where `remote_backends: Vec<Backend>` (owned, in deterministic `BTreeMap`
iteration order — preserve the existing partition order) is the post-gate
REMOTE survivor set: `desired_svc.backends`, minus mesh backends
(`is_mesh_backend`), minus LOCAL backends (`addr.ip() == host_ipv4`). This is
**exactly** the set the existing code already computes as `remote` at :392-396 —
the design change is only that its fingerprint becomes a first-class value used
by the dispatch/compare decisions, and that the empty case now emits a purge.

**Domain pin (load-bearing):** `programmed_fingerprint` is `fingerprint(vip,
remote_survivors)`. It is byte-identical to what `dataplane_update_service::dispatch`
computes (`fingerprint(vip, action.backends)`) **because the action carries
exactly `remote_survivors`**. This equality is the entire fix; the crafter MUST
NOT compute it over any other set (not the full set, not local+remote, not
local-only).

> **V6-VIP note.** The current V6 arm (:355-387) emits `DataplaneUpdateService`
> over `non_mesh` (it has no LOCAL partition — every non-mesh backend is
> "remote" for a V6 VIP). For a V6 VIP, `remote_backends == non_mesh`, so
> `programmed_fingerprint = fingerprint(vip, non_mesh)`. The same emit/compare
> rules apply; the V6 arm gains the same empty-purge behavior (an all-mesh V6
> service emits the empty purge and settles, instead of emitting nothing). Pin
> the V6 arm to the SAME `programmed_fingerprint` discipline — do not leave it
> on the old full-set comparison.

### 3.2 `should_dispatch` — re-key the decision onto `programmed_fingerprint`

`should_dispatch`'s SIGNATURE IS UNCHANGED. Only its *caller* changes the value
passed for the `desired_fingerprint` parameter — from `desired_svc.fingerprint`
to `programmed_fingerprint`:

```rust
// reconcile, at the call site (currently :323-328):
let need_dispatch = should_dispatch(
    actual_status,
    programmed_fingerprint,          // CHANGED: was desired_svc.fingerprint
    view.retries.get(service_id),
    tick.now_unix,
);
```

`should_dispatch`'s body (the `None|Pending` / `Completed` / `Failed` match arms,
the backoff gate, the level-triggered fingerprint-change reset) is **unchanged**.
It now compares `actual_status`'s fingerprint against the *programmable*
fingerprint, which is the value the `Completed` row actually carries — so the
`Completed`-arm equality (:566) becomes reachable for every shape.

> **Local-axis consequence (read § 8).** Because the `RegisterLocalBackend` emit
> lives INSIDE the `if need_dispatch` block, re-keying `need_dispatch` onto the
> remote-only `programmed_fingerprint` would mean a local-backend churn whose
> remote projection is unchanged does NOT re-fire the local emit once the service
> has settled. This is the **local-churn re-drive gap**. It is **fixed in this
> feature via mechanism L-a** (§ 8.3): a DECOUPLED local-emission convergence
> signal, gated on a per-service `local_fingerprint` diff INDEPENDENT of the
> remote-keyed `need_dispatch`. The re-key of `need_dispatch` therefore governs
> ONLY the remote/XDP emit + the empty-remote purge; the local path has its own
> level-triggered signal (§ 8.3) that survives the re-key. Do NOT read the
> `need_dispatch` re-key as gating the local path — under L-a it does not.

### 3.3 The emit condition — emit `DataplaneUpdateService` ALWAYS on the remote/XDP path, including empty

Replace the **guarded** remote emit (current :401 `if !remote_is_empty`) with an
**unconditional** emit on the remote/XDP path when `need_dispatch` is true:

```rust
// V4 path, replacing the `if !remote_is_empty { push DUS }` block at :401-414.
// Emit the DataplaneUpdateService UNCONDITIONALLY on dispatch — an empty
// `remote_backends` is the documented per-proto PURGE (traits/dataplane.rs
// :197-204). This drives the dataplane to the empty state AND produces the
// Completed{fingerprint(vip,[])} row that lets an all-mesh / local-only
// service settle, AND tears down stranded REVERSE_NAT/SERVICE_MAP entries
// on a non-mesh->all-mesh transition (Finding 2).
actions.push(Action::DataplaneUpdateService {
    service_id: *service_id,
    vip: desired_svc.vip,
    port: desired_svc.port,
    proto: desired_svc.proto,
    backends: remote_backends.clone(),   // MAY be empty (purge)
    correlation: CorrelationKey::derive(&target_str, &spec_hash, "update-service"),
});
```

The `RegisterLocalBackend` emission for LOCAL backends (`push_register_local_backend_actions`,
:416-427) is **unchanged** — it still emits one action per surviving LOCAL
backend.

**`spec_hash` pin.** `spec_hash` (the `CorrelationKey` content input, currently
`ContentHash::of(desired_svc.fingerprint.to_le_bytes())` at :332) — keep it keyed
on the **full-set** `desired_svc.fingerprint`. Rationale: the correlation key
identifies "this service's desired-state revision"; the full-set fingerprint is
the stable identity of that revision (a mesh-only-backend churn that changes
nothing programmable SHOULD still produce a distinct correlation so a redundant
purge is correlated to the new revision, not silently coalesced with the old).
Do NOT re-key `spec_hash` onto `programmed_fingerprint`. (This is the one place
the full-set fingerprint is deliberately retained as the churn signal — see § 5.)

### 3.4 The retry-bump guard — re-key onto `programmed_fingerprint`, drop the empty-set guard

Replace the line-437 guard block:

```rust
// REMOVE the `if !(local_is_empty && remote_is_empty)` guard entirely.
// A dispatch ALWAYS emitted a DataplaneUpdateService now (incl. the purge),
// so a dispatch always has a programmable fingerprint to record. Record the
// PROGRAMMED fingerprint (matches what the Completed row will carry), NOT the
// full-set fingerprint.
let entry = next_view.retries.entry(*service_id).or_default();
entry.attempts = entry.attempts.saturating_add(1);
entry.last_failure_seen_at = tick.now_unix;
entry.last_attempted_fingerprint = Some(programmed_fingerprint);  // CHANGED domain
```

`last_attempted_fingerprint` now records the **programmable** fingerprint —
consistent with `should_dispatch`'s comparison and the `Completed` row. The
"don't record a phantom for an all-mesh service" reasoning (the 02-02 guard) is
**obsolete under this model**: an all-mesh service is no longer a no-op — it
genuinely dispatches a purge, so recording its (empty-set) programmed
fingerprint is honest, not a lie. The View's `retries` entry for an all-mesh
service now carries `last_attempted_fingerprint = Some(fingerprint(vip, []))`,
which is exactly what the next `Completed{fingerprint(vip,[])}` row matches to
settle.

### 3.5 The convergence-reset arm — re-key onto `programmed_fingerprint`

The `else if Completed && fingerprint == desired` arm (:443-448) re-keys:

```rust
} else if let Some(ServiceHydrationStatus::Completed { fingerprint, .. }) = actual_status
    && *fingerprint == programmed_fingerprint   // CHANGED: was desired_svc.fingerprint
{
    next_view.retries.remove(service_id);
}
```

### 3.6 Summary of the changed surface (all in `service_map_hydrator.rs`)

| Site | Change | Domain after |
|---|---|---|
| `reconcile` (per service) | compute `programmed_fingerprint = fingerprint(vip, remote_backends)` | NEW derived local, not persisted |
| `should_dispatch` call (:325) | pass `programmed_fingerprint` not `desired_svc.fingerprint` | programmable |
| remote emit (:401) | unconditional emit (empty = purge); drop `if !remote_is_empty` | — |
| V6 arm emit (:367) | unconditional emit over `non_mesh` (empty = purge); drop `if !non_mesh.is_empty()` | programmable |
| retry-bump (:437) | drop empty-set guard; record `programmed_fingerprint` | programmable |
| convergence-reset (:445) | compare `*fingerprint == programmed_fingerprint` | programmable |
| `spec_hash` (:332) | **UNCHANGED** — stays keyed on full-set `desired_svc.fingerprint` | full-set (churn identity) |
| **local-emission seam (NEW — L-a, § 8.3)** | compute `local_fingerprint = fingerprint(vip, local_survivors)` per service; re-emit `RegisterLocalBackend` for the current local set when `local_fingerprint != view.last_applied_local_fingerprint.get(sid)`, DECOUPLED from `need_dispatch`; record the applied local fingerprint | local (programmable cgroup) |
| **`ServiceMapHydratorView` (NEW field — L-a, § 8.3)** | add `last_applied_local_fingerprint: BTreeMap<ServiceId, BackendSetFingerprint>` (`#[serde(default)]`, additive CBOR, no envelope, no migration) | — |

**`should_dispatch` keeps its exact 4-arg signature. `RetryMemory` keeps its
3-field shape. `Action::DataplaneUpdateService` / `RegisterLocalBackend` enum
shapes are unchanged. No new public type, method, enum variant, or parameter.**

**One additive persisted surface IS added (L-a, B5 = build-now):** the
`ServiceMapHydratorView` gains a SECOND per-service map,
`last_applied_local_fingerprint: BTreeMap<ServiceId, BackendSetFingerprint>`,
annotated `#[serde(default)]`. This is additive CBOR schema evolution per
`.claude/rules/development.md` § "Reconciler I/O → Schema evolution": a
V1-written View (no field) deserialises with an empty map — tolerant, NO
versioned envelope, NO migration. `programmed_fingerprint` and `local_fingerprint`
remain function-local, recomputed every tick from inputs, NEVER persisted
("persist inputs, not derived state"). The `last_applied_local_fingerprint` map
persists the INPUT (the applied local-set fingerprint), not a derived
"needs-redrive" boolean — the re-drive decision is recomputed every tick from
that input + the freshly-computed `local_fingerprint`. This is the ONLY new
persisted surface in the whole fix.

---

## 4. The all-mesh (empty programmed set) decision — STATED EXPLICITLY

**Decision: settle via the empty-backends `DataplaneUpdateService` purge.**

An all-mesh service emits exactly ONE `DataplaneUpdateService { backends: [] }`
on its first dispatch tick. The shim calls `update_service(frontend, [])` (the
per-proto purge — removes any stranded `REVERSE_NAT` keys, no-op if none) and
writes `Completed{ fingerprint: fingerprint(vip, []), applied_at }`. The next
tick reads that row; `should_dispatch`'s `Completed`-arm finds
`fingerprint(vip,[]) == programmed_fingerprint` → `false` → no further dispatch.
The convergence-reset arm clears `retries`. **The service is settled and
observable** (a `Completed` hydration row exists with the empty-set fingerprint).

**Why this over "recognize ∅ as a no-op with no action" (the rejected
alternative):** the no-action path is precisely the current bug — it produces no
observation row, so `actual` is never populated and the service is *invisible*
to any convergence reader (the latent P3→P1 hazard, RCA § 4). The purge path
costs **one** action + one obs-write on the first dispatch and then settles to
**zero** I/O forever (the `Completed`-arm returns `false`, `persist_view`'s
Eq-diff skip elides the no-op View write). It is strictly more observable for a
one-time bounded cost, and it is the ONLY option that also closes Finding 2.

**The all-mesh service is NOT special-cased.** There is no `if all_mesh` branch.
The empty programmable projection flows through the SAME unconditional-emit path
as any other backend set; `∅` is just the degenerate value. This is the
"make the degenerate case not a special case" discipline — the model has ONE
path, and the all-mesh service is that path with an empty payload.

---

## 5. Is the full-set "advertised" fingerprint still needed anywhere?

**Yes — in exactly two retained roles, neither of which is convergence:**

1. **Evaluation-broker churn key (runtime, unchanged).** The runtime keys
   evaluations on `(ReconcilerName, ServiceId)` and re-enqueues on any
   `service_backends` row change. `desired_svc.fingerprint` (full-set) is what
   `project_service_desired` computes and stamps; a mesh-only-backend churn
   (e.g. a Path-A backend's health flips) changes the full-set fingerprint and
   re-triggers a reconcile even though it changes nothing *programmable*. That
   re-trigger is correct and desirable: the reconcile recomputes
   `programmed_fingerprint` (unchanged → `Completed`-arm → no dispatch) and
   settles again cheaply. **Losing the full-set fingerprint here would not break
   convergence, but retaining it costs nothing and preserves the existing churn
   signal — so it stays.**

2. **`spec_hash` / correlation identity (§ 3.3, unchanged).** The full-set
   fingerprint is the stable identity of a desired-state revision for
   correlation-key derivation. Retained.

**Conclusion:** the full-set fingerprint is demoted from "the convergence target"
to "the churn/identity signal," a role it already implicitly plays. No code that
computes or persists it changes. The convergence machinery moves to
`programmed_fingerprint`. This cleanly answers the task's "decide whether the
advertised full-set fingerprint is still needed" — **needed, but only as the
churn/identity key, never as the settle comparison.**

---

## 6. Finding 2 (non-mesh → all-mesh teardown) — STATED EXPLICITLY

**Decision: FOLDED IN. The chosen model fixes Finding 2 as a direct
consequence, with no extra mechanism.**

The current line-390 gate (`if !remote_is_empty`) suppresses the
`DataplaneUpdateService` whenever the remote set is empty — which is exactly the
non-mesh→all-mesh transition, stranding the prior `REVERSE_NAT`/`SERVICE_MAP`
entries. The chosen model **removes that guard** (§ 3.3) and emits
`DataplaneUpdateService { backends: [] }` unconditionally on dispatch. On the
transition, the empty-backends action IS the documented purge
(`traits/dataplane.rs:197-204`) — the stranded entries are removed. There is no
separate transition-detection input and no new action: the level-triggered
recompute naturally emits the purge the moment `remote_backends` becomes empty
(the new programmable fingerprint `fingerprint(vip,[])` differs from the
last-attempted non-empty one → `should_dispatch` fires → purge emitted → settles).

**This is not "widening the fix to invent teardown behavior."** The teardown
behavior is the dataplane contract's OWN documented edge case
(`backends.is_empty()` ⇒ purge); the bug was that the gate prevented the
contract-blessed purge from ever being emitted. Removing the gate lets the
contract operate as written. No design surface is invented — the purge action
already exists and is already contract-specified.

**Caveat carried forward (not a blocker):** RCA § 4 notes it is unestablished
whether a real service can transition non-mesh→all-mesh in Phase 1 without
changing `ServiceId`. The fix is correct *if* the transition occurs and is a
strict no-op if it never does (an always-all-mesh service emits the same purge
on first dispatch regardless). So folding it in carries zero risk even if the
transition is unreachable today; it removes the latent hazard unconditionally.

---

## 7. Option (i) vs (ii) — the trade-off, and what does NOT change

**Option (ii) — carry the full-set fingerprint THROUGH the action** (the shim
echoes a fingerprint computed over the full set, or the action carries
`desired_svc.fingerprint` and the shim writes it back verbatim): **REJECTED.**

- **It would make the `Completed` row lie.** The row's `fingerprint` is meant to
  certify "the dataplane is confirmed at THIS state." If the action programmed
  only the remote subset but the row reports the full-set fingerprint, the row
  asserts the dataplane holds backends it never programmed (the mesh + local
  ones). That is persisting a *derived* claim that does not match the *observed*
  effect — a direct violation of `.claude/rules/development.md` § "Persist
  inputs, not derived state" and § "Trait definitions specify behavior" (the
  `Completed.fingerprint` must reflect what `update_service` was actually given).
- **It requires a wire/shim change** — the action would need to carry a separate
  "advertised fingerprint" field OR the shim would need the full backend set to
  recompute it, widening the `DataplaneUpdateService` action surface (a new
  persisted/wire field) for no convergence benefit.
- **It does not fix local-only or all-mesh** — those emit NO `DataplaneUpdateService`
  under the current gate, so there is no action to carry any fingerprint through.

Option (i) keeps the `Completed` row honest (`fingerprint(vip, actually-programmed
backends)`), needs no wire change, and — paired with the unconditional emit —
makes all four shapes settle. It is the only option consistent with the
"observed effect, not derived claim" discipline.

### What does NOT change (regression-surface containment)

- `fingerprint()` (the function) — unchanged.
- `desired_svc.fingerprint` computation in `project_service_desired` — unchanged
  (still full-set; now the churn/identity key).
- `RetryMemory` field shape — **unchanged** (3 fields).
- `ServiceMapHydratorView` — **ADDITIVELY EXTENDED** by ONE field (L-a, B5 =
  build-now): a new `last_applied_local_fingerprint: BTreeMap<ServiceId,
  BackendSetFingerprint>` annotated `#[serde(default)]`. This is the one
  schema change in the fix — **additive CBOR, NO versioned envelope, NO
  migration** (a V1 View with no field deserialises to an empty map). The
  `retries` field is untouched. (A prior draft claimed "View field shape —
  unchanged / no schema evolution"; that held under the rejected L-d scope-out
  and is corrected here — § 3.6 / § 8.3 / § 12 govern.)
- `should_dispatch` signature — **unchanged** (4 args).
- `Action::DataplaneUpdateService` / `RegisterLocalBackend` enum shape —
  **unchanged**.
- `dataplane_update_service::dispatch` / `register_local_backend::dispatch` —
  **unchanged** (the shim already computes `fingerprint(vip, action.backends)`;
  the fix makes the hydrator AGREE with it, not the other way around).
- `Dataplane::update_service` empty-purge contract — **unchanged** (already
  specifies `backends.is_empty()` ⇒ purge).
- The remote-only happy path — **unchanged behavior** (remote-only's
  `programmed_fingerprint == desired_svc.fingerprint` because no backend is
  gated; it converges at tick 1 exactly as before).

---

## 8. The local-only / cgroup path — STATED EXPLICITLY (CORRECTED → BUILT, 2026-06-24)

This is the subtlest decision and the task flags it directly. The cgroup
(`RegisterLocalBackend`) path writes NO hydration observation row
(`register_local_backend.rs:17-20` — "the cgroup hook is not an HTTP call
surface and produces no observation row").

> **CORRECTION (2026-06-24), then BUILD DECISION.** A prior revision of this
> section claimed "reconcile re-emits the `RegisterLocalBackend` for the current
> local set" on a local-backend change **as a free consequence of the re-key**.
> That claim was false: the `RegisterLocalBackend` emission
> (`push_register_local_backend_actions`, service_map_hydrator.rs:416-427) is
> INSIDE the `if need_dispatch` block (:330-442), and § 3.2 re-keys `need_dispatch`
> onto `programmed_fingerprint = fingerprint(vip, remote_survivors)`. If the local
> re-emit stayed gated by `need_dispatch`, then for a local-only (or
> mixed-where-only-local-churned) service `remote_survivors` never changes when
> local backends churn, so `programmed_fingerprint` is invariant,
> `should_dispatch` returns `false` once settled, the `if need_dispatch` block is
> skipped, and the local re-emit **would not happen** — a local-backend
> add/remove/health-flip with an unchanged remote projection would be **silently
> dropped**, the same defect class as the bug this design fixes, on the
> local/cgroup axis.
>
> **DECISION (2026-06-24, user-ratified — B5 = BUILD NOW).** Rather than scope the
> local-churn re-drive out, this feature **CLOSES it via mechanism L-a** (§ 8.3):
> a DECOUPLED local-emission convergence signal that re-emits `RegisterLocalBackend`
> whenever the desired LOCAL backend set's fingerprint differs from the
> last-applied one, INDEPENDENT of the remote-keyed `need_dispatch`. The gap is
> latent/unreachable on the production path today (§ 8.1: the LOCAL partition is
> empty by construction), but L-a is built now so the model is correct
> regardless — there is no asserted-away gap and no "pinned-but-unbuilt"
> deferral. The reachability analysis (§ 8.1) is retained as the rationale for why
> the surface stays minimal (one additive View field, no observation surface);
> the observation-surface question (external observability of the cgroup path)
> remains DEFERRED, tracked as **GH #246** (§ 8.2, B2).

### 8.1 Reachability — the LOCAL partition is EMPTY on the production path (evidence)

**Finding: under the current ADR-0071 mesh model, on the production `overdrive
serve` path, every Service backend is a MESH backend (`addr.ip() ∈ 10.99.0.0/16`)
and the hydrator's LOCAL partition is EMPTY by construction. `RegisterLocalBackend`
is never emitted in production. Local-backend churn is latent/unreachable today.**

Three-layer evidence chain (each link verified against live code):

1. **The sole producer of the rows the hydrator consumes is `BackendDiscoveryBridge`,
   and it constructs every backend address as `workload_addr.unwrap_or(host_ipv4)`**
   (`backend_discovery_bridge.rs:355-367`). A LOCAL backend (`addr.ip() ==
   host_ipv4`, the hydrator's LOCAL-partition key at :394) is produced **iff**
   the alloc's `workload_addr` is `None`.
2. **`workload_addr` is `Some(/30 addr ∈ 10.99.0.0/16)` for every Path-A
   (mTLS-composed) alloc and `None` only for a host-netns alloc**
   (`action_shim/mod.rs:811-839`; feature-delta canonical-workload-address D-B2 /
   D-BLOCKER2). The C3 provision seam sets `spec.workload_addr =
   Some(plan.workload_addr)` only when `mtls_worker.is_some()`; on the non-mTLS
   path it returns early (`:811-813`) and `workload_addr` stays `None`.
3. **`mtls_worker.is_some()` iff the production real `EbpfDataplane` is composed**
   (`lib.rs:1824-1826`): `compose_mtls = config.dataplane_override.is_none()`.
   The production boot uses the real dataplane (`dataplane_override == None`) ⇒
   `compose_mtls == true` ⇒ every Service alloc is Path-A mesh ⇒ `workload_addr
   = Some(mesh /30)` ⇒ **every backend is MESH ⇒ gated out at the hydrator
   (D-GATE-PRED) ⇒ `remote_survivors == []` AND `local == []` for every Service.**
   `dataplane_override == Some(SimDataplane)` (the only `None`-`workload_addr`
   path) is a **test/DST-only boot**, never `overdrive serve`.

**Consequence for this design:** on the production mesh path the LOCAL arm is
dead — `local` is always empty, so `push_register_local_backend_actions` emits
nothing and there is no local state to keep converged. The local-churn re-drive
gap **cannot fire in production today.** It is reachable ONLY through:

- (a) the **pure `reconcile` surface driven directly** — where the existing
  default-lane test `mesh_backend_lb_gate.rs::host_address_backend_still_
  registers_as_local_backend` (a single backend pinned at `host_ipv4`, ∉ mesh
  subnet) exercises the LOCAL arm and asserts `register_count == 1`; and
- (b) a **hypothetical future configuration** — a Service workload advertised at
  `host_ipv4` (non-mesh, host-netns) on a *real* dataplane, which the current
  composition (`compose_mtls = dataplane_override.is_none()`) does not produce.

Neither (a) nor (b) is a live production traffic path. The gap is real on the
pure surface, latent in production. **This reachability finding does NOT decide
whether to FIX the gap — the user's B5 decision (build L-a now) does. It decides
how MINIMAL the fix can be:** because the LOCAL partition is empty in production,
L-a (§ 8.3) needs only one additive View field + a pure decoupled signal — no
observation surface, no shim change, no `hydrate_actual` projection. The EXTERNAL
cgroup observation surface (L-c) is the part that stays DEFERRED (B2, GH #246),
precisely because it would be machinery certifying an empty production partition.

### 8.2 Decision — BUILD local-churn re-drive (L-a); DEFER the cgroup observation surface (B2, GH #246)

**Decision: this design ships (i) remote-axis convergence over the programmable
projection (§ 2), (ii) a one-time local install on first dispatch, AND (iii) the
local-backend-churn RE-DRIVE via mechanism L-a (§ 8.3). The model is correct
regardless of the LOCAL partition's reachability — there is no scoped-out
correctness gap.**

This is task option **(L-a)**, chosen over the prior L-d scope-out (user
decision B5 = BUILD NOW). L-a's surface cost is exactly **one additive View
field** (`last_applied_local_fingerprint`, additive CBOR `#[serde(default)]`, no
envelope, no migration) plus a pure local-emission seam — the minimal correct
surface. The reachability analysis (§ 8.1) does NOT make L-a unnecessary; it
makes the surface MINIMAL: because the LOCAL partition is empty on the production
path, L-a needs no observation surface, no shim change, no `hydrate_actual`
projection — only the View field + the decoupled signal.

**L-b (active deregister-on-removal) is NOT built here** — it is a SUPERSET of
L-a requiring a `DeregisterLocalBackend` over a *removed* local backend, and the
current `deregister_local_backend` shim's exact trigger shape is out of scope.
L-a re-drives the *current* local set on change (re-`RegisterLocalBackend`); if a
future slice needs to actively tear down a removed local entry, that is the L-b
extension, surfaced as its own pinned decision then.

**L-c (give the cgroup path an external observation surface) — DEFERRED, tracked
as GH #246 (B2).** L-c is the EXTERNAL OBSERVABILITY of the local-path applied
state (an observation-store row/status a convergence-reasoning reader could query
to distinguish "local backend installed" from "remote programmed"). It is a
materially larger blast radius — a new observation-store row/status, a new shim
write on the cgroup path, a new `hydrate_actual` projection, touching ADR-0053's
cgroup-path contract and the observation-store schema — to certify a partition
that is empty on every production boot (§ 8.1). Per CLAUDE.md § "Build vertical
slices… never isolated mechanisms," an observation surface no production deploy
exercises is out of scope for this fix. **It is NOT built here; it is tracked as
[GH #246](https://github.com/overdrive-sh/overdrive/issues/246).** L-a (the
re-drive) and L-c (the external observation surface) are independent: L-a makes
the local path *converge correctly*; L-c would make its applied state
*externally queryable*. This fix builds L-a; L-c stays at #246.

**What the model guarantees for a local-only service (production-unreachable
today, but correct regardless under L-a):**

1. `remote_survivors == []` ⇒ `programmed_fingerprint = fingerprint(vip, [])`.
2. First dispatch tick (`should_dispatch` true, `actual = None`) emits ONE
   `RegisterLocalBackend` per local backend (idempotent cgroup map insert) AND
   ONE `DataplaneUpdateService { backends: [] }` (the empty-remote purge → the
   observable `Completed{fingerprint(vip,[])}` row). The L-a seam records
   `last_applied_local_fingerprint[sid] = fingerprint(vip, local_survivors)`.
3. Next tick reads `Completed{fingerprint(vip,[])}` ⇒ `should_dispatch` false ⇒
   the remote axis is settled; the L-a seam finds `local_fingerprint ==
   last_applied_local_fingerprint[sid]` ⇒ emits nothing for the local path ⇒
   settled.
4. **A subsequent local-backend churn** (add/remove/health-flip changing the
   local set) makes `local_fingerprint != last_applied_local_fingerprint[sid]`
   ⇒ the L-a seam re-emits `RegisterLocalBackend` for the new current local set,
   INDEPENDENT of `need_dispatch` (whose remote-keyed `programmed_fingerprint`
   is unchanged) ⇒ the cgroup map is re-driven ⇒ records the new
   `last_applied_local_fingerprint` ⇒ settles again.

The cgroup map insert is **idempotent converge-on-apply** (re-inserting an
existing entry is a no-op; once installed it persists until deregistered) — the
walking-skeleton precedent (`register_local_backend.rs:17-20`: "convergence is
observable via the read-back from the production handle … NOT via an obs row").
**The one-time install AND the post-install churn re-drive are both honored
under L-a.**

### 8.3 The built local-churn re-drive mechanism (L-a) — IN SCOPE, pinned precisely

This is the mechanism the crafter **builds in this fix** (B5 = build-now). It is
pinned to **zero crafter latitude**: the exact View field + serde shape, the
exact local-set the local fingerprint covers, where the diff is computed, and how
the emission is gated INDEPENDENTLY of `need_dispatch` are all fixed below. The
crafter MUST NOT invent any other shape and MUST NOT partially build it (do NOT
ship the View field without the decoupled signal, or vice versa).

**Mechanism (L-a): drive the local emission on its OWN convergence signal,
decoupled from the remote-keyed `need_dispatch`.**

1. **New persisted View surface (additive — the ONLY new persisted surface in
   the whole fix).** Add a second per-service map to `ServiceMapHydratorView`
   (live shape at `service_map_hydrator.rs:187-192`):

   ```rust
   #[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
   pub struct ServiceMapHydratorView {
       /// Per-service retry inputs (UNCHANGED).
       #[serde(default)]
       pub retries: BTreeMap<ServiceId, RetryMemory>,
       /// Fingerprint of the LOCAL backend set most recently driven via
       /// `RegisterLocalBackend`. Persists the INPUT (the applied local-set
       /// fingerprint), not a derived "needs re-drive" boolean — the re-drive
       /// decision is recomputed every tick from this input + the freshly-
       /// computed `local_fingerprint`.
       #[serde(default)]
       pub last_applied_local_fingerprint: BTreeMap<ServiceId, BackendSetFingerprint>,
   }
   ```

   This is **additive CBOR schema evolution** per `.claude/rules/development.md`
   § "Reconciler I/O → Schema evolution": a new `#[serde(default)]` map field. A
   V1-written View (no field) deserialises with an empty map — tolerant, **NO
   migration, NO versioned envelope** (CBOR additive-field tolerance, not the
   rkyv versioned-envelope path — the View is CBOR-encoded in the runtime-owned
   `ViewStore`). The `retries` field is byte-for-byte unchanged. **This is the
   one and only View-shape change in the fix.**

2. **Decoupled local convergence (pure, recomputed each tick).** The local-set
   partition is already computed in `reconcile` — `local: Vec<&Backend>` at
   `service_map_hydrator.rs:392-396` (the V4 partition of `non_mesh` into LOCAL
   `addr == host_ipv4` and REMOTE arms). **Hoist the local-emission decision
   OUT of the `if need_dispatch` block** so it runs every tick per service,
   independent of the remote-keyed dispatch gate. Per service, with `local_survivors`
   = the post-gate LOCAL partition (mesh-filtered, `addr.ip() == host_ipv4`):

   ```rust
   // Computed per service, OUTSIDE / independent of `if need_dispatch`.
   // (For a V6 VIP there is no LOCAL partition — local_survivors is empty,
   //  local_fingerprint = fingerprint(vip, []), and the seam is a no-op.)
   let local_fingerprint: BackendSetFingerprint =
       crate::dataplane::fingerprint::fingerprint(&desired_svc.vip, &local_survivors);

   if next_view.last_applied_local_fingerprint.get(service_id) != Some(&local_fingerprint) {
       // The LOCAL set changed (first install OR post-install churn) — re-emit
       // RegisterLocalBackend for the CURRENT local set, gated on the LOCAL
       // fingerprint diff, NOT on need_dispatch / programmed_fingerprint.
       push_register_local_backend_actions(&mut actions, &local_survivors, &LocalBackendEmit { .. });
       next_view.last_applied_local_fingerprint.insert(*service_id, local_fingerprint);
   }
   ```

   - **Domain pin:** `local_fingerprint = fingerprint(vip, local_survivors)` —
     the fingerprint over the EXACT LOCAL set `push_register_local_backend_actions`
     is driven with. It is **derived, never persisted**; only its last-applied
     VALUE is persisted (step 1). Recomputed every tick from inputs (`backends`,
     `workload_subnet`, `host_ipv4`).
   - **Gating pin:** the emission is gated SOLELY on `local_fingerprint !=
     last_applied_local_fingerprint.get(sid)` — NOT on `need_dispatch`, NOT on
     `programmed_fingerprint`, NOT on the `Completed` row. A local churn whose
     remote projection is unchanged still fires the local re-emit. This is the
     decoupling that closes the local-churn gap.
   - **Settle pin:** on equality (the steady state once installed), the seam
     emits nothing — `local_survivors == last-applied` ⇒ zero local actions ⇒
     settled. Combined with the `persist_view` Eq-diff skip, a settled
     local-only service does zero I/O per tick.
   - **GC pin:** extend the existing `next_view.retries.retain(...)` GC
     (`service_map_hydrator.rs:452`) to ALSO drop
     `last_applied_local_fingerprint` entries for services no longer in
     `desired` (mirror the `retain` on the new map). The crafter MUST keep the
     two maps GC'd in lockstep.
   - **Purity pin:** time only via `tick.now_unix` (the local seam does not read
     time at all); no `Instant::now()`; the function stays pure / DST-replayable;
     `cargo xtask dst-lint` stays green.

3. **L-b sub-choice (active deregister-on-removal) is a SUPERSET of L-a and is
   NOT built here**, because it requires a `DeregisterLocalBackend` action over a
   *removed* local backend and the current `deregister_local_backend` shim's
   exact trigger shape is out of scope for this fix. L-a re-drives the *current*
   set on change (re-`RegisterLocalBackend`); the idempotent cgroup insert plus
   the `last_applied_local_fingerprint` update is the minimal correct surface. If
   a future slice needs to actively tear down a removed local entry (not just
   stop re-adding it), that is the L-b extension — surface it as its own pinned
   decision then.

**Why NOT add an EXTERNAL observation surface for the cgroup path (L-c) — see
§ 8.2 (B2)**: deferred and tracked as
[GH #246](https://github.com/overdrive-sh/overdrive/issues/246) (larger blast
radius — observation-store schema + shim write + `hydrate_actual` projection — to
externally certify a partition that is empty on every production boot). L-a (this
mechanism) makes the local path *converge*; L-c (#246) would make its applied
state *externally queryable* — independent concerns.

### 8.4 Risk on the shipping (L-a) decision

Two risk surfaces, both bounded:

1. **The empty-remote purge** on a local-only service is a
   `update_service(frontend, [])` call on first dispatch — well-formed (`frontend`
   requires a valid V4 VIP, which a local-only service's V4 VIP satisfies), a
   kernel no-op when there are no REVERSE_NAT keys, one obs-row write then settled
   (the same bounded one-time cost the all-mesh path pays).
2. **The additive View field** (`last_applied_local_fingerprint`) is the only new
   persisted surface. Risk is contained by the additive-CBOR `#[serde(default)]`
   discipline (a V1 View deserialises to an empty map — no migration, no envelope)
   and by GC'ing it in lockstep with `retries`. The local re-emit is idempotent
   (re-`RegisterLocalBackend` of an existing entry is a cgroup no-op), so a
   spurious re-drive (e.g. after a View-field reset) costs at most one redundant
   idempotent insert, not a correctness fault.

The local-churn gap that the rejected L-d scope-out would have left open is
**closed** by L-a: a post-install local-set change is re-driven on its own
fingerprint diff, independent of the remote-keyed gate. The gap is
latent/unreachable on the production path today (§ 8.1: LOCAL partition empty by
construction), but the model is now correct regardless.

---

## 9. Blockers / under-specified gaps surfaced (NOT issues — per CLAUDE.md)

These were surfaced to the user as blockers/decisions; the user has RATIFIED
each (2026-06-24). The dispositions below are SETTLED — they record the decisions
and their rationale, not open questions.

- **(B1) — ACCEPTED: the empty-remote purge for local-only and all-mesh services
  is the settle mechanism.** The model deliberately emits a (often kernel-no-op)
  empty `DataplaneUpdateService` on every all-mesh / local-only service's first
  dispatch to produce the observable `Completed` row, then settles to zero I/O.
  The alternative (no row, invisible service) is the current bug. **The user has
  accepted the one-time obs-row-write cost** as the price of observability +
  Finding-2 closure — it is bounded, one-time, and the only path that settles all
  four shapes.

- **(B2) — DEFERRED, tracked as [GH #246](https://github.com/overdrive-sh/overdrive/issues/246):
  the cgroup/local path has no EXTERNAL observation surface.** This is distinct
  from B5/L-a (B5 makes the local path *converge*; B2 would make its applied state
  *externally queryable*). Under L-a (§ 8.3) the local-only service converges
  correctly (one-time install + churn re-drive), but the cgroup map-insert's state
  is certified only by idempotent re-apply + production-handle read-back (the
  walking-skeleton precedent), NOT by an observation-store row a remote
  convergence-reasoning reader could query. Building that external surface is a
  materially larger blast radius (observation-store row/status + shim write on the
  cgroup path + `hydrate_actual` projection) to certify a partition that is empty
  on every production boot (§ 8.1) — per CLAUDE.md § "Build vertical slices… never
  isolated mechanisms," an observation surface no production deploy exercises is
  out of scope for this fix. **It is DEFERRED and tracked as GH #246** — cited at
  § 8.2, § 8.3, § 11.5, and § 12.

- **(B5) — ACCEPTED, BUILD NOW: local-backend-churn re-drive is BUILT via L-a.**
  Without a fix, re-keying `need_dispatch` onto `programmed_fingerprint` (§ 3.2)
  would silently drop a local-backend add/remove/health-flip whenever the remote
  projection is unchanged. **The user has decided to CLOSE this gap in this fix**
  (not scope it out): mechanism L-a (§ 8.3) drives the `RegisterLocalBackend`
  re-emit on a DECOUPLED `local_fingerprint` convergence signal, independent of
  the remote-keyed `need_dispatch`, so a changed LOCAL backend set is re-applied
  regardless of whether the remote projection moved. The surface cost is exactly
  ONE additive View field (`last_applied_local_fingerprint: BTreeMap<ServiceId,
  BackendSetFingerprint>`, additive CBOR `#[serde(default)]`, no envelope, no
  migration) plus the pure local-emission seam. The gap is latent/unreachable on
  the production path today (§ 8.1: the LOCAL partition is empty by construction —
  evidence: bridge `workload_addr.unwrap_or(host_ipv4)` × C3 seam
  `mtls_worker`-gating × `compose_mtls = dataplane_override.is_none()`), but L-a is
  built now so the model is **correct regardless** — there is no asserted-away gap
  and no pinned-but-unbuilt deferral. **This flips the "no View change" claims in
  § 3.6 / § 7 / § 12** — they held under the rejected L-d disposition; under L-a the
  additive-CBOR schema-evolution note governs (the field is the only new persisted
  surface). The L-a mechanism is built in roadmap step 01-02 and pinned by a
  two-tick local-churn test (§ 11.4).

- **(B3) — FORWARD NOTE (non-blocking): Finding 2's live reachability is
  unestablished.** RCA § 4 could not establish whether a non-mesh→all-mesh
  transition is reachable in Phase 1 without a `ServiceId` change. The fix closes
  the hazard unconditionally and is a strict no-op if the transition never
  occurs, so it is folded in at zero risk. No separate decision needed; noting
  for completeness.

- **(B4) — PRE-EXISTING DOC DRIFT, DEFERRED TO A POST-CRAFTER DOC-SYNC (out of
  scope for this design + roadmap):** `architecture.md § 8`'s `ServiceDesired`
  snippet (:631-635) shows `{ vip, backends, fingerprint }` but the live code
  (`service_map_hydrator.rs:127-133`) carries `{ vip, port, proto, backends,
  fingerprint }`. This drift predates this fix (step 02-02 added `port`/`proto`).
  Per the user's B4 decision (AFTER CRAFTER), the `ServiceDesired` § 8 field-list
  sync is a **separate doc-sync done after the crafter lands** — it is NOT folded
  into this design, NOT into the § 8 amendment, and NOT a step in this fix's
  roadmap. Flagged here so it is not silently propagated.

---

## 10. The corrected DST invariant spec (the structural regression guard)

The current `HydratorEventuallyConverges` invariant
(`overdrive-sim/src/invariants/service_map_hydrator.rs:81-167`) is a **faithless
simulation**: at :126-134 it writes back `Completed{ fingerprint:
desired.fingerprint }` (the FULL-set fingerprint) for every emitted
`DataplaneUpdateService`, masking the subset mismatch entirely (RCA § 10.4). It
must be corrected to model the real shim, and exercise all four shapes.

### 10.1 Required correction — model the write-back fingerprint as the real shim does

The harness's write-back (:112-135) MUST compute the `Completed` fingerprint
**over the emitted action's `backends`**, exactly as
`dataplane_update_service::dispatch` does (`fp = fingerprint(vip, backends)` at
`dataplane_update_service.rs:111`):

```rust
// CORRECTED write-back (replacing :112-135):
for action in &actions {
    if let Action::DataplaneUpdateService { service_id, vip, backends, .. } = action {
        // Model the REAL shim: fingerprint over the ACTION's backends
        // (the programmed subset), NOT the desired full-set echo.
        let applied_fp = fingerprint(vip, backends);   // vip is the action's ServiceVip
        state.actual.insert(
            *service_id,
            ServiceHydrationStatus::Completed {
                fingerprint: applied_fp,
                applied_at: /* tick-derived UnixInstant, as today */,
            },
        );
    }
    // NOTE: RegisterLocalBackend emits NO row — the harness must NOT
    // synthesise one for it (model the real cgroup path's silence).
}
```

The `all_converged` predicate (:284-292) is then evaluated against the
PROGRAMMED fingerprint. Since the corrected hydrator records
`last_attempted_fingerprint = programmed_fingerprint` and compares against the
programmed-subset `Completed` row, `all_converged` must be redefined to check
`actual.Completed.fingerprint == fingerprint(desired.vip, remote_survivors)` —
i.e. the harness recomputes the same programmable projection the hydrator does,
OR (cleaner) asserts the operational property directly: **"within the budget,
every service reaches a tick where `actions.is_empty()` AND every desired service
has a `Completed` row."** The "no actions emitted AND a Completed row exists for
every service" steady-state is the honest, shape-agnostic convergence assertion
— it does not require the harness to re-derive `programmed_fingerprint`, only to
observe that the loop quiesced with a confirmed row per service.

**Preferred shape (avoids the harness re-deriving the gate):** drive the REAL
action-shim (`dataplane_update_service::dispatch` + a `SimDataplane` +
`SimObservationStore`) from the invariant, exactly as the RCA § 10 probe did,
and feed `actual` from the rows the shim actually wrote. This is the
highest-fidelity option — the harness then CANNOT diverge from production because
it IS production's shim. If the in-process wiring cost is acceptable (the probe
proved it is), prefer this over hand-modeling the write-back.

### 10.2 Required coverage — all four shapes

`HydratorEventuallyConverges` (and a companion no-spin assertion) MUST exercise,
each as its own fixture/sub-case:

| Shape | Backends (example) | Expected settled state |
|---|---|---|
| **remote-only** | `[10.96.0.50]` | `Completed{fp(vip,[10.96.0.50])}`; converged ≤ 2 ticks; 0 actions thereafter |
| **all-mesh** | `[10.99.0.6, 10.99.0.10]` | `Completed{fp(vip,[])}` (empty purge); converged ≤ 2 ticks; 0 actions thereafter |
| **mixed mesh+remote** | `[10.99.0.6, 10.96.0.50]` | `Completed{fp(vip,[10.96.0.50])}`; converged ≤ 2 ticks; **NO tight spin** (the face the faithless harness masked) |
| **local-only** | `[host_ipv4]` | `Completed{fp(vip,[])}` (empty purge) + `RegisterLocalBackend` on first tick only; converged ≤ 2 ticks; 0 `DataplaneUpdateService` thereafter |

The **mixed** shape is the load-bearing addition — it is the face the faithless
echo made structurally undetectable (RCA § 10.4). A regression of the
fingerprint-domain mismatch re-fails the mixed sub-case within
`CONVERGENCE_TICK_BUDGET`.

`HydratorIdempotentSteadyState` is similarly extended: after convergence, EACH
of the four shapes must emit **zero** actions per tick (the all-mesh and
local-only shapes must not re-emit the purge once `Completed{fp(vip,[])}` is
observed).

### 10.3 Why this is the structural guard

This closes Root Cause B (RCA § 8.1): the one gate that encodes "must converge"
is now (a) faithful to the real write-back fingerprint and (b) pointed at the
exact shapes that could not converge under the old model. A future re-introduction
of the full-set/subset domain mismatch fails the mixed sub-case at PR time.

---

## 11. Test reconciliation (which assertions change) — the risk note

### 11.1 `crates/overdrive-core/tests/mesh_backend_lb_gate.rs` — assertions INVERT for the all-mesh / mesh-only cases

These tests currently **encode the bug as the contract** (a mesh service emits
nothing, `programmed_fp == None`, all-mesh `view.retries` stays empty). Under the
fix they MUST change:

- **`mesh_subnet_backend_programs_neither_local_nor_remote_lb_path`** (:131) —
  the assertion "mesh backend emits NO `DataplaneUpdateService`" (:145-148) and
  "`programmed_fp == None`" (:149-152) **INVERT**. Post-fix, an all-mesh service
  emits exactly ONE `DataplaneUpdateService { backends: [] }` (the purge) and
  records `last_attempted_fingerprint = Some(fingerprint(vip, []))`. **The test's
  intent must be re-stated:** "a mesh backend programs NO LOCAL path and is
  EXCLUDED from the remote backend payload, but the service DOES emit the
  empty-remote purge that settles it." Concretely: `register_count == 0`
  (unchanged — no local emit for a mesh backend), `dataplane_count == 1`
  (CHANGED from 0 — the purge), and the emitted `DataplaneUpdateService.backends`
  is EMPTY (the mesh backend did not leak in). `programmed_fp` becomes
  `Some(fingerprint(vip, []))` (CHANGED from `None`).
- **`all_mesh_service_emits_nothing_and_keeps_retries_empty_across_ticks`** (:346)
  — this test's entire premise (emit nothing, retries empty) is the bug.
  **It must be REWRITTEN** to the new contract: tick 1 emits one empty-purge
  `DataplaneUpdateService`, records `retries[s].last_attempted_fingerprint =
  Some(fp(vip,[]))`; given a `Completed{fp(vip,[])}` row fed into `actual`, tick 2
  emits ZERO actions and clears `retries` (settled). The test name should change
  to reflect "settles via empty purge," e.g.
  `all_mesh_service_settles_via_empty_remote_purge`. Per
  `.claude/rules/development.md` § "Deletion discipline" the salvage-by-rewrite
  must be honest: this is a genuinely NEW contract (the old behavior is deleted),
  so the rewritten test gets a new name and new assertions describing the new
  requirement — NOT a quiet edit that pretends it "always" tested settling.
- **`v6_vip_all_mesh_service_emits_nothing_and_keeps_retries_empty`** (:494) —
  same inversion for the V6 arm: post-fix it emits the empty purge and settles.
  Rewrite to the new contract (the V6 all-mesh service emits one empty-purge
  `DataplaneUpdateService` and settles on the `Completed{fp(vip,[])}` row).
- **`host_address_backend_still_registers_as_local_backend`** (:160) — a
  local-only service. The local assertion (`register_count == 1`) is UNCHANGED,
  but `dataplane_count == 0` (:173-176) **CHANGES to 1** (the empty-remote purge),
  and the emitted `DataplaneUpdateService.backends` is EMPTY. `programmed_fp`
  becomes `Some(fingerprint(vip, []))` (was `Some` already, but now over the
  EMPTY set, not the full set). Update these assertions.
- **`non_mesh_non_host_backend_still_drives_dataplane_service_update`** (:190) —
  a remote-only service. **UNCHANGED.** `register_count == 0`, `dataplane_count
  == 1`, `programmed_fp.is_some()`. The remote-only happy path is preserved
  exactly — this is the regression anchor that proves the fix did not break the
  one shape that already converged.
- **`mixed_service_excludes_mesh_keeps_remote_backend_and_registers_local`** (:254)
  — **UNCHANGED for its existing assertions** (one `DataplaneUpdateService`
  carrying exactly the remote backend, one `RegisterLocalBackend` for the host
  backend, mesh leaks nowhere). The fix does not change a mixed service's emitted
  REMOTE payload (still exactly the remote survivor). It only changes the
  *fingerprint domain* the View/dispatch keys on — which this test does not
  assert. Safe.
- **`v6_vip_service_excludes_mesh_backend_from_dataplane_update`** (:425) —
  **UNCHANGED** (a V6 service with a surviving remote backend emits exactly that
  backend; mesh excluded). The fix preserves the payload.
- **`three_way_split_routes_each_address_class_to_exactly_one_disposition`** (PBT,
  :527) — **CHANGES.** The mesh arm currently asserts `dataplane_count == 0` and
  `programmed_fp == None` (:543-546). Post-fix, a mesh backend in an ALL-MESH
  single-backend service emits the empty purge: `dataplane_count == 1`,
  `programmed_fp == Some(fp(vip,[]))`, and the emitted backends are empty. **The
  PBT's mesh-arm branch must be rewritten** to assert the empty-purge contract.
  CAUTION: this PBT uses single-backend services, so its "mesh" case is an
  all-mesh service — every mesh case now emits the purge. The local and remote
  arms' assertions are unchanged EXCEPT the local arm now also emits the empty
  purge (`dataplane_count` for the local arm changes 0→1). Re-derive the three
  arms' expected `(register_count, dataplane_count, programmed_fp)` under the
  new model:
  - mesh (all-mesh): `(0, 1, Some(fp(vip,[])))`, emitted backends empty.
  - local: `(1, 1, Some(fp(vip,[])))` — local register + empty remote purge.
  - remote: `(0, 1, Some(fp(vip,[remote])))` — unchanged.

> **Pin for the crafter:** the PBT's invariant becomes "every single-backend
> service emits EXACTLY ONE `DataplaneUpdateService` (the remote/XDP path —
> populated for a remote backend, EMPTY purge for mesh/local), plus a
> `RegisterLocalBackend` iff the backend is local." The "mesh zeroes both paths"
> invariant is DELETED — it was the bug.

### 11.2 `crates/overdrive-control-plane/tests/integration/service_map_hydrator_dispatch.rs` — UNCHANGED

All three tests (`dispatch_writes_completed_row_on_dataplane_ok`,
`dispatch_writes_failed_row_on_dataplane_err`, `dispatch_rejects_ipv6_vip_with_failed_row`)
exercise the SHIM's row-writing over a NON-empty backend action and pin
`fingerprint(vip, backends)` over the action's backends — which is exactly the
behavior the fix RELIES ON and does not change. **No assertion changes.**
(Optional ADD, not required: a `dispatch_writes_completed_row_with_empty_backends_purge`
test pinning that `update_service(frontend, [])` → `Completed{fingerprint(vip,[])}`
— a thin guard on the empty-purge round-trip the fix depends on. Recommend
adding it as a one-line strengthening, but it is not a reconciliation of an
existing assertion.)

### 11.3 `should_dispatch` unit tests (`service_map_hydrator.rs` :751-832) — UNCHANGED

These test `should_dispatch` as a pure function over `(actual_status,
fingerprint, retry, now)`. The signature is unchanged; the tests pass an
arbitrary `fingerprint` value and assert the backoff/level-trigger logic. The
fix changes only WHICH fingerprint the *caller* passes, not `should_dispatch`'s
behavior for a given fingerprint. **No assertion changes.**

### 11.4 Local-churn axis — a two-tick local-churn test IS REQUIRED (L-a built, B5 = build-now)

The user asks directly: does `mesh_backend_lb_gate.rs`'s local arm change, or
does a new local-churn test get added? Under the **L-a shipping disposition (B5 =
build-now)**, BOTH:

- **`host_address_backend_still_registers_as_local_backend`** (the existing
  single-tick local arm) — changes ONLY as already enumerated in § 11.1
  (`register_count == 1` UNCHANGED; `dataplane_count` 0→1 for the empty-remote
  purge; `programmed_fp` now over the EMPTY set). It is a **single-tick
  first-dispatch** test (`actual = empty`, `view = default`), so the L-a seam
  does not regress it: on the first tick the `local_fingerprint !=
  last_applied_local_fingerprint.get(sid)` predicate is true (the View's new map
  is empty ⇒ `None != Some(fp)`), so the one `RegisterLocalBackend` is emitted
  exactly as before AND `next_view.last_applied_local_fingerprint` records the
  applied local fingerprint. **The local install is honored; the test stays GREEN
  with the § 11.1 purge-axis edits, plus an optional assertion that
  `next_view.last_applied_local_fingerprint` now carries the service's entry.**

- **A NEW default-lane two-tick local-churn test IS REQUIRED** — it pins the L-a
  re-drive guarantee that this fix newly provides. Pinned shape: a two-tick
  local-only (or mixed) service where —
  - **Tick 1:** local set `{B1}` (a `host_ipv4` backend, ∉ mesh subnet),
    `actual = None`, `view = default`. Asserts: one `RegisterLocalBackend` for
    `{B1}`, and `next_view.last_applied_local_fingerprint[sid] == fingerprint(vip,
    [B1])`. Feed the resulting `Completed{fingerprint(vip,[])}` row into `actual`
    for tick 2 (the remote axis settles).
  - **Tick 2 (steady-state probe):** same local set `{B1}`, `actual =
    Completed{fp(vip,[])}`, `view` = tick-1's `next_view`. Asserts: ZERO
    `RegisterLocalBackend` (the L-a seam finds `local_fingerprint ==
    last_applied` ⇒ no re-emit) AND ZERO `DataplaneUpdateService` (remote settled)
    — i.e. a settled local-only service does zero I/O.
  - **Tick 3 (the load-bearing churn):** local set changes `{B1}` → `{B2}`
    (remote projection UNCHANGED → `programmed_fingerprint` unchanged →
    `should_dispatch` STILL false). Asserts: tick 3 MUST emit a fresh
    `RegisterLocalBackend` for `{B2}`, driven by the decoupled `local_fingerprint
    != last_applied_local_fingerprint` signal NOT by `need_dispatch`, and records
    `next_view.last_applied_local_fingerprint[sid] == fingerprint(vip, [B2])`.

  This test FAILS against any implementation that gates the local emit on
  `need_dispatch` (the bug L-a fixes — tick 3 would emit nothing). Name it for the
  new contract, e.g.
  `local_backend_churn_redrives_register_local_backend_independent_of_remote_gate`.
  It lands with step **01-02** (the L-a build step). It is a pure-`reconcile`
  default-lane test in `mesh_backend_lb_gate.rs` (or the adjacent
  `service_map_hydrator.rs` `#[cfg(test)]` block) — no sim adapters, no feature
  flag.

> **Note on B2 (#246):** this test asserts the local path *converges* (the
> re-emit fires on churn) by observing the EMITTED ACTIONS from `reconcile`, NOT
> by querying an external observation row. The external observation surface for
> the cgroup path is the B2 deferral (GH #246) — out of scope here. The L-a test
> is honest because it observes `reconcile`'s own output, the surface L-a
> actually changes.

### 11.5 Risk summary

| Surface | Risk | Mitigation |
|---|---|---|
| remote-only happy path | **must stay converging** | unchanged by construction (`programmed_fp == desired_fp` when nothing gated); `non_mesh_non_host_backend...` test is the anchor |
| all-mesh / local-only | newly emit an empty purge (1 obs-write, then settle) | bounded one-time cost; B1 ACCEPTED |
| local-backend CHURN re-drive | **BUILT (L-a, B5=build-now)** — re-driven on a decoupled `local_fingerprint` diff, independent of `need_dispatch` | latent/unreachable on production path today (§ 8.1: LOCAL partition empty by construction) but correct regardless; pinned § 8.3; two-tick churn test § 11.4 is the guard |
| `mesh_backend_lb_gate.rs` | several assertions INVERT (they encoded the bug) | § 11.1 enumerates each; honest rewrite per Deletion discipline |
| local-arm test (single-tick) | NOT regressed by the L-a seam | § 11.4 — first-dispatch `actual=None`, empty View map ⇒ `local_fingerprint != None` ⇒ local install emitted; purge-axis edits + optional `last_applied_local_fingerprint` assertion |
| DST invariant | faithless harness masked the bug | § 10 correction is the structural guard against regression |
| View schema | **ONE additive field** (`last_applied_local_fingerprint`, additive CBOR `#[serde(default)]`, no envelope, no migration) | the only new persisted surface; GC'd in lockstep with `retries`; § 8.3 pins shape |
| cgroup EXTERNAL observation surface | **DEFERRED (B2, GH #246)** — not built here | isolated-mechanism rule: empty production partition; tracked at [#246](https://github.com/overdrive-sh/overdrive/issues/246) |
| Finding 2 | folded in at zero risk | empty purge IS the documented teardown |

---

## 12. Summary

| Question | Answer |
|---|---|
| Fingerprint domain | **Programmable-remote projection** (`fingerprint(vip, remote_survivors)`), used for dispatch + convergence; full-set fingerprint demoted to churn/identity key. Option (i). |
| All-mesh settled state | **Empty-backends `DataplaneUpdateService` purge → `Completed{fp(vip,[])}` → settled.** Not special-cased; the degenerate value of the one emit path. |
| Local-only convergence | Settles over its EMPTY remote projection (same purge) + a ONE-TIME local install on first dispatch, AND **local-backend CHURN re-drive is BUILT (L-a, B5=build-now)** — a decoupled `local_fingerprint` convergence signal re-emits `RegisterLocalBackend` on any LOCAL-set change, independent of the remote-keyed `need_dispatch` (§ 8.3). Latent/unreachable on the production path today (LOCAL partition empty by construction, § 8.1) but correct regardless. cgroup map-insert stays idempotent-re-apply (walking-skeleton precedent). The EXTERNAL cgroup observation surface is **DEFERRED (B2, GH #246)** — not built here. |
| Finding 2 | **Folded in** — removing the `if !remote_is_empty` guard lets the contract-blessed purge fire on the transition. Zero risk. |
| View shape | **ONE additive field** — `ServiceMapHydratorView.last_applied_local_fingerprint: BTreeMap<ServiceId, BackendSetFingerprint>` (`#[serde(default)]`, additive CBOR, NO versioned envelope, NO migration; a V1 View deserialises to an empty map). The `retries` field is unchanged; `RetryMemory` (3 fields) is unchanged. This is the L-a mechanism (§ 8.3, B5=build-now). |
| New persisted surface | **Exactly ONE: the additive `last_applied_local_fingerprint` View map (L-a).** `programmed_fingerprint` and `local_fingerprint` are recomputed every tick from inputs and NEVER persisted; only the last-APPLIED local fingerprint value is persisted (the input to the re-drive decision, not a derived boolean). |
| DST invariant fix | Model write-back as `fingerprint(vip, action.backends)` (or drive the real shim) + exercise all four shapes; mixed is the load-bearing addition. |
