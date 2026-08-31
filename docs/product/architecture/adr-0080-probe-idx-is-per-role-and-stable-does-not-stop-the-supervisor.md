# ADR-0080 — `ProbeIdx` is per-role and lives in the durable key; `Stable` does not stop the probe supervisor; the bridge takes sole ownership of `ServiceBackendRow`

## Status

Accepted. 2026-08-02.
Decision-makers: Morgan (nw-solution-architect, DESIGN wave). Mode: propose.
Tags: phase-1, probes, reconcilers, observation-store, service-discovery, application-arch.

**Amended 2026-08-31 (TRC-ARCH-003).** D2's key remains unchanged, but its
"V1 payload unchanged" statement is historical: ProbeResultRow V2 adds the
accepted Running logical attempt, and latest-row LWW compares attempt before
wall time. D3's liveness consumer gains exact-attempt hydration while retaining
per-role index 0.

Closes the live instance named in `.claude/rules/reconcilers.md` § "Codebase
precedent" → "The same shape, still live in tree"
(`ServiceLifecycleView::last_emitted_backend_fingerprint`) and executes the
end-state recorded in
[ADR-0079](adr-0079-backend-discovery-bridge-converges-on-the-rows-it-manages.md)
§ D9.

Amends [ADR-0054](adr-0054-probe-runner-subsystem.md) § 5 (the durable
composite key gains `role`) and completes
[ADR-0057](adr-0057-health-check-toml-spec.md) § 172 (the
`ProbeDescriptor.idx` field the implementation dropped). Supersedes ADR-0079
§ D2's carry-through containment (§ D5).

**Scope boundary.** This ADR fixes the defect that readiness and liveness probe
**0** is never consulted. It deliberately does **not** fix the separate,
pre-existing defect that probes **1..N** are never consulted — see § "A fourth,
pre-existing gap this ADR deliberately does NOT address" and § A8.

Depends on `.claude/rules/development.md` § "Ground the premise: a state only a
test seam can produce is not a feature", § "Type-driven design", § "Persist
inputs, not derived state", § "Reconciler I/O", § "rkyv schema evolution",
§ "Deletion discipline"; `.claude/rules/reconcilers.md` § "Symptoms during
review"; ADR-0035 / ADR-0036 (reconciler runtime), ADR-0048 (envelope
discipline), ADR-0050 (`WorkloadIntent` envelope relocation), ADR-0079.

**No GitHub issue is created and none is cited.** Per CLAUDE.md § "Deferrals
require GitHub issues", an unbacked forward pointer is forbidden — so every
open question below is closed by *deciding* it, not by promising a slice.

---

## Context

### The premise, grounded

`.claude/rules/development.md` § "Ground the premise" binds this ADR, and it
binds **inverted**: the reported symptom was machinery defending a state
production cannot reach. The check was run first. The premise **holds** — but
the reported *direction* was wrong, and correcting it changes the severity from
"a feature does nothing" to "a feature makes the service unreachable".

The production path is real and named. `overdrive deploy <spec>` parses
`[[health_check.*]]` into **`ServiceSpecV2`**
(`crates/overdrive-core/src/aggregate/service_spec.rs:138-157`), which projects
into the intent-side **`ServiceV1`**
(`crates/overdrive-core/src/aggregate/mod.rs:445-466`) inside
`WorkloadIntentV1::Service`. These are two distinct types that each carry three
`Vec<ProbeDescriptor>` fields; § D1 pins both. `overdrive serve` hydrates them
at the desired boundary via `project_probe_descriptors`
(`crates/overdrive-control-plane/src/reconciler_runtime.rs:2363`, reading
`WorkloadIntent::Service(svc)` — i.e. `ServiceV1`), threads them into
`AllocationSpec.probe_descriptors`
(`crates/overdrive-core/src/traits/driver.rs:157`), and the action shim hands
them to `ExecDriver::on_alloc_running` → `ProbeRunner::start_alloc`
(`crates/overdrive-worker/src/driver.rs:752-756`). No test override, no `Sim*`
adapter, no disabled composition gate.

### Finding 1 — verified, direction corrected

**Reported**: `Backend.healthy` may never be `false` in production.

**Verified**: `Backend.healthy` is never a function of *observation at all*. It
is a constant function of *intent*:

| Service declares | `has_readiness_probe` | `latest_readiness_probe` | `Backend.healthy` |
|---|---|---|---|
| no readiness probe | `false` | — (short-circuit) | **`true`**, always |
| ≥1 readiness probe, **≥1 startup probe** | `true` | `None` — Mechanism 1 | **`false`**, permanently |
| ≥1 readiness probe, **no startup probe** | `true` | `None` — Mechanism 2 | **`false`**, permanently |

The two lower rows reach the same result by *different* mechanisms, which is why
both fixes are required (§ "The two mechanisms are independent"). ADR-0058's
default inference fills `startup_probes` unless the operator explicitly opts out,
so the middle row is the production-typical shape.

`compute_backend_healthy`
(`crates/overdrive-core/src/service_lifecycle.rs:893-921`) short-circuits to
`true` when `!fact.has_readiness_probe` (`:898-901`); otherwise it requires
`matches!(fact.latest_readiness_probe, Some(ProbeStatus::Pass))` (`:919`).
`latest_readiness_probe` is structurally always `None` (below), so the second
row resolves to `false` and never leaves it.

The reported claim is therefore the *opposite* of the defect for the case that
matters. Declaring a readiness probe does not fail to gate traffic — it
**permanently withholds** it. `Backend.healthy == false` is consumed
fail-closed at three live seams:

- `mtls_resolve_adapter.rs:573-580` — `classify_by_addr` returns
  `MtlsResolution::MeshUnreachable` when no backend at the addr is healthy
  (`Some(_) => MeshUnreachable`, `:578`). Refuse, no cleartext.
- `mtls_resolve_adapter.rs:489-501` — `first_healthy_backend_for` filters on
  `backend.healthy` (`:497`), so the frontend re-key yields nothing.
- `dns_responder/name_index.rs:25-31` — the withhold seam: a `<workload>` with
  zero running-AND-healthy backends is WITHHELD → `answer_for → NxDomain`.

Net operator-visible behaviour: **a Service that declares
`[[health_check.readiness]]` never resolves by name and refuses every mesh
dial, forever.** A Service that declares none is unconditionally healthy. In
neither case does the readiness probe do anything.

`latest_readiness_probe` is structurally `None` because of **two independent
mechanisms**. Both were verified; neither subsumes the other.

#### Mechanism 1 — `probe_idx` is assigned across the concatenated vector, consumed per-role

`project_probe_descriptors`
(`crates/overdrive-core/src/reconcilers/workload_lifecycle.rs:1259-1267`)
concatenates in a fixed order — startup (`:1263`), readiness (`:1264`),
liveness (`:1265`). `ProbeRunner::start_alloc` then assigns `probe_idx` from the
**flat enumerate index** over that concatenation
(`crates/overdrive-worker/src/probe_runner/mod.rs:323`, `:337`):

```rust
for (idx, descriptor) in probe_descriptors.into_iter().enumerate() {
    let probe_idx = ProbeIdx::new(u32::try_from(idx).unwrap_or(u32::MAX));
```

So readiness probe 0 lands at flat index `S = svc.startup_probes.len()`, and
liveness probe 0 at `S + R`.

Every consumer filters on **per-role index 0**
(`crates/overdrive-control-plane/src/reconciler_runtime.rs`):

- startup — `:3023-3030`: `role == Startup && probe_idx == 0`
- readiness — `:3035-3042`: `role == Readiness && probe_idx == 0`
- liveness — `:3046-3053`: `role == Liveness && probe_idx == 0`

Startup matches **only by accident of ordering**. Readiness matches only when
`startup_probes` is empty — and ADR-0058's inference fills it by default, so the
dominant production shape has `S ≥ 1`. Liveness matches only when startup *and*
readiness are both empty.

The consumers are not wrong to expect per-role. **Per-role is the specified
contract**, in three independent places:

- ADR-0057:132-134 — "`probe_idx` is the 0-indexed position within the per-role
  array".
- `crates/overdrive-core/src/observation/probe_result_row.rs:159` — "0-indexed
  position within the **role array**".
- `docs/feature/service-health-check-probes/discuss/shared-artifacts-registry.md:11-15`
  — the per-role contract.

The `probe_result_row.rs:159` docstring is currently false, and the comment at
`probe_runner/mod.rs:332-336` **mis-cites its own stated source**: it claims
"`ProbeIdx` is 0-indexed across the descriptor vector per the
`discuss/shared-artifacts-registry.md` contract", while that registry specifies
per-role.

#### Root cause — the dropped `ProbeDescriptor.idx` field

ADR-0057:172 specified `pub idx: ProbeIdx, // 0-indexed; parser-assigned`. The
implemented struct has **no `idx` field**
(`crates/overdrive-core/src/aggregate/probe_descriptor.rs:147-161` — `role`,
`mechanic`, `timeout_seconds`, `interval_seconds`, `max_attempts`,
`failure_threshold`, `success_threshold`, `inferred`; nothing else).

That omission is the root cause of Mechanism 1. Deprived of the parser-assigned
per-role index, `start_alloc` had to invent one, and the only index available at
that call site is the flat vector position. Every downstream consumer kept the
ADR-0057 semantics; the producer silently switched index spaces.

#### Why nothing caught it — the key cannot represent the contract

The durable composite key is `(alloc_id, probe_idx)` — **`role` is not in it**:

- ADR-0054:336 — "Key shape `(alloc_id, probe_idx)` is a composite primary key".
- `crates/overdrive-store-local/src/observation_backend.rs:128-136` — "Key
  layout: `alloc_id_bytes || 0x00 || probe_idx LE u32`".
- `crates/overdrive-core/src/traits/observation_store.rs:2174-2181` — the trait
  contract restates LWW "at the same `(alloc_id, probe_idx)`".

Under **true** per-role indexing, `startup[0]`, `readiness[0]` and
`liveness[0]` all encode to the identical key and clobber each other under LWW.
The flat concatenated index is therefore **load-bearing for key uniqueness** —
the (almost certainly accidental) mitigation keeping the three roles from
destroying one another's rows.

This is the trap a crafter would otherwise hit: "filter on `role` alone" and
"make `ProbeIdx` per-role" each look individually correct and each is
individually catastrophic. Per-role indices **without** `role` in the key
convert a silent read-miss into silent durable data loss.

No test caught it because **no test exercises the hydrate filter**. The
readiness acceptance suite constructs `ServiceAllocFact` by hand with
`latest_readiness_probe: Some(...)` and `startup_probes_empty: false`
(`crates/overdrive-control-plane/tests/acceptance/service_lifecycle_readiness.rs:96-99`),
seeding the counter at `ProbeIdx::new(0)` (`:243`, `:287`). That is a state the
production hydrate cannot produce — § "Ground the premise" running in the other
direction.

#### Mechanism 2 — a `Stable` terminal tears down the whole supervisor

Independent of the index defect, and **it fires for every Service, not only
empty-startup ones**. Both Stable branches emit
`Action::FinalizeFailed { terminal: Some(TerminalCondition::Stable { .. }) }`:

- branch (a'), empty-startup opt-out — `service_lifecycle.rs:557-575`
  (emission at `:568`);
- branch (a), startup-probe-Pass — `:580-600` (emission at `:593`).

Both reach the same unconditional terminal hook
(`crates/overdrive-control-plane/src/action_shim/mod.rs:1203`), guarded by a
comment stating the intent (`:1198-1202`):

> Probe-supervisor cleanup is correct for BOTH a Stable and a genuine terminal
> (a Stable alloc has indeed passed startup, so its supervisor hook is
> benign-or-correct) — NOT gated on `is_stable`.

It is not benign. `ExecDriver::on_alloc_terminal`
(`crates/overdrive-worker/src/driver.rs:765-769`) calls
`ProbeRunner::stop_alloc`, which removes the supervisor and cancels **every**
task under it — readiness and liveness included (`probe_runner/mod.rs:367-375`).
The two genuinely destructive teardowns immediately below are already correctly
gated on `!is_stable` (`action_shim/mod.rs:1210-1223`); the probe hook was left
outside that gate.

This contradicts ADR-0055, titled "…`Stable` as **non-terminal** condition"
(:1) and stating it four times, including the variant docstring (:267-276):

> Unlike other variants, `Stable` is NON-TERMINAL: the reconciler continues to
> process readiness, liveness, and restart for the alloc after emission.

and the role contract at `probe_result_row.rs:77-78`:

> startup: bounded by `startup_deadline`; readiness/liveness: **continuous
> post-Stable**.

ADR-0055's readiness branch (:228-238) and liveness branch (:240-249) are gated
"only when `stable_announced`" — reachable *only after* Stable. Cancelling
supervision at Stable makes both structurally unreachable for their entire
intended lifetime.

The executed population diff confirms the timing: with `startup_probes: vec![]`
the alloc settles Stable at `settled_in_ms: 114`, `all probe rows for alloc:
Ok([])`, `active_alloc_count = Some(0)` — the supervisor is gone 114 ms in, well
inside the 2 s default readiness interval (ADR-0057:93).

#### The two mechanisms are independent — neither is a symptom of the other

- Fix the index alone, and **every** Service still loses readiness/liveness
  supervision at Stable (Mechanism 2 fires on both branches).
- Fix the teardown alone, and the dominant non-empty-startup case still discards
  every readiness row, because they land at flat index ≥ 1.

Both are required. The *root-cause* ruling: the dropped `ProbeDescriptor.idx`
field is the root cause **of Mechanism 1**; Mechanism 2 is a separate contract
violation (a non-terminal condition routed through a terminal hook) with its own
root cause, not downstream of it.

#### A third instance, same class

The liveness filter (`reconciler_runtime.rs:3046-3053`) is broken identically
and more severely — it requires both startup and readiness to be empty. So
liveness-driven restart (`liveness_restart_action`, `service_lifecycle.rs:741`,
emission at `:798`) is also dead on the production path for any Service with a
startup probe. Fixed here as the same defect, not as scope creep.

#### A fourth, pre-existing gap this ADR deliberately does NOT address

`spec_facts_for_service` (`reconciler_runtime.rs:1958-1969`) projects
`max_attempts`, `startup_deadline`, `mechanic_summary` and `inferred` from
`svc.startup_probes[0]` **only** (`:1963`), under an explicit comment that
Phase 1 consults only probe 0. Combined with the index-0 filters, a Service
declaring two startup probes today has probe 1 spawned, ticking, and writing
durable rows that **no decision ever consults**.

Multi-probe-per-role is therefore a second, independent defect. It **predates**
the one this ADR diagnoses and **survives** every fix in it: after D1–D5 a
second probe of any role is still spawned, still writes rows, and is still
consulted by nothing.

**This ADR does not fix it, and does not promise to.** Recording the boundary
explicitly, because the two are easy to conflate:

- ADR-0080's defect is that readiness and liveness probe **0** — the probe that
  exists in every Service — is never consulted. That is what makes a Service
  unreachable, and it is what D1–D5 close.
- The multi-probe gap is that probes **1..N** are never consulted. Closing it
  requires ratifying readiness and liveness combinators that ADR-0055 § 5 leaves
  specified for startup only, and respecifying `ProbeWitness`,
  `update_startup_attempts` and `spec_facts_for_service`'s single-probe
  projection. That is a separate design with its own evidence requirement.

An earlier draft of this ADR tried to close both — first by ratifying
combinators (rejected, § A3) and then by rejecting multi-probe at the parser
(rejected, § A8). Both attempts expanded the blast radius well beyond the
diagnosed defect while leaving the diagnosed defect no better closed. The
boundary above is the ruling.

One thing D1 + D2 **do** change for the multi-probe case, as a side effect
rather than a goal: probes 1..N currently share a durable key space with the
other roles, so their rows can be clobbered. Under the three-part key they are
stored correctly and distinctly. Storage becomes correct; consultation stays as
it is today.

### Finding 2 — verified

`ServiceLifecycleView::last_emitted_backend_fingerprint`
(`crates/overdrive-core/src/service_lifecycle.rs:387`) is read at `:861` and
stamped on the **emit** path at `:865`, before the write is attempted:

```rust
let prev_fp = next_view.last_emitted_backend_fingerprint.get(&dataplane.service_id).copied();
if prev_fp == Some(current_fp) { return None; }                                  // :862-864
next_view.last_emitted_backend_fingerprint.insert(dataplane.service_id, current_fp); // :865
```

This is the anti-pattern ADR-0079 removed from `BackendDiscoveryBridge`, and
`.claude/rules/reconcilers.md` names this site as the live instance. Its own
docstring concedes the consequence (`:384-385`): "a dropped readiness write is
still permanently forgotten." Compounding it, the runtime fsyncs the `View`
**before** dispatching actions (`development.md` § "Reconciler I/O", STEP 7 →
STEP 8), so the marker outlives the effect across crashes.

ADR-0079 § D4 excluded it structurally; § D9 recorded why (`ServiceLifecycle`
authors only `healthy` on a **shared** row, so converging it on the whole row
would make it fight the bridge) and named sole bridge ownership as the leading
candidate.

Also verified: `readiness_backend_row_action` is invoked **unconditionally** at
`service_lifecycle.rs:675` — *not* gated on `stable_announced`, diverging from
ADR-0055:228. Its only guards are `service_dataplane.is_some()` (`:837`), a
non-empty alloc set (`:838-840`), ≥1 `Running` alloc (`:844-846`, `:856-858`),
and the fingerprint dedup. So it fires in production and is what writes the
permanent `healthy: false`.

The bridge carries the observed `healthy` through rather than authoring it
(`backend_discovery_bridge.rs:377-379`), defaulting to `true` via `is_none_or`
for an alloc with no observed entry. Steady state: bridge writes `true` first,
`ServiceLifecycle` overwrites with `false`, bridge carries `false` forward.
`false` sticks.

---

## Decision

### D1 — Restore `ProbeDescriptor.idx: ProbeIdx`, parser-assigned per role

Add the field ADR-0057:172 specified and the implementation dropped:

```rust
// crates/overdrive-core/src/aggregate/probe_descriptor.rs
pub struct ProbeDescriptor {
    /// 0-indexed position within THIS descriptor's role array
    /// (`[[health_check.<role>]]`). Parser-assigned at
    /// `ServiceSpecV2` construction and carried verbatim into
    /// `ServiceV1`; never re-derived downstream.
    pub idx: ProbeIdx,
    pub role: ProbeRole,
    // ... existing fields unchanged
}
```

`ProbeDescriptor` is a **single shared struct** used by both type families named
in Context. Assignment sites:

- `parse_startup_probes` (`crates/overdrive-core/src/aggregate/workload_spec.rs:1070`),
  `parse_readiness_probes` (`:1229`), `parse_liveness_probes` (`:1327`) — each
  assigns `ProbeIdx::new(i)` for its own 0-based array position `i`.
- the ADR-0058 inference site (`workload_spec.rs:1111`) — assigns
  `ProbeIdx::new(0)`.
- `ServiceV1::from_submit` (`crates/overdrive-core/src/aggregate/mod.rs:592-594`)
  — the **second ingress**, used by the API/wire path, which today validates
  probe mechanics only. It **MUST** re-assign `idx` from each vector's position
  rather than trusting a caller-supplied value, so a wire client cannot inject a
  duplicate `(role, idx)` pair and collide two allocs' rows under the new
  durable key (§ D2). This is the only behavioural addition to `from_submit`.
- the `ServiceSpecV2 → ServiceV1` projection carries `idx` **verbatim**; it does
  not recompute it.

`ProbeRunner::start_alloc` **MUST** consume `descriptor.idx` and **MUST NOT**
derive an index from `enumerate()`:

```rust
// crates/overdrive-worker/src/probe_runner/mod.rs — replaces :323 + :337
for descriptor in probe_descriptors {
    let probe_idx = descriptor.idx;
    // ... unchanged
}
```

For this Stage-1 index correction, `start_alloc`'s signature was unchanged.
TRC-ARCH-003 later adds the accepted logical-attempt argument; the index rule
and `project_probe_descriptors` concatenating body remain unchanged. The flat
vector is still a transport carrying no index semantics.

The false comment at `probe_runner/mod.rs:332-336` is **deleted** in this
commit, per `feedback_behavior_change_must_mark_stale_adjacent_docs`. The
docstring at `probe_result_row.rs:159` becomes true without edit.

**rkyv handling — follow the in-tree precedent, do NOT bump an envelope.**
`ProbeDescriptor` is reachable from two persisted archives: `WorkloadIntentV1`
(via `ServiceV1`) and `ServiceSpecV2` (via `ServiceSpecEnvelope::V2`). Adding a
field shifts positional offsets in both. The governing local precedent is
`crates/overdrive-core/src/aggregate/mod.rs:413-427`, recorded for **this exact
change shape** (adding probe vecs to `ServiceV1`):

> under the Phase-1 greenfield single-cut migration policy … the new field set
> is admitted **in-place** rather than minting `WorkloadIntentV2`. The
> historical golden-bytes fixtures under `tests/schema_evolution/workload_intent.rs`
> are **regenerated in this same commit** to pin the new V1 layout — the
> structural defense (every persisted layout has a pinned golden-bytes fixture)
> is preserved.

This ADR rules the same way, and extends it to the second archive. Concretely:

- **No** `WorkloadIntentV2`, **no** `ServiceSpecV3`, **no** `ProbeDescriptorV1/V2`
  fork. There is no `JobEnvelope` — `JobV1` is a sibling `WorkloadIntent`
  variant carrying no `ProbeDescriptor` (`aggregate/mod.rs:198-207`, ADR-0050)
  and is untouched.
- **Regenerate exactly two fixture sets**, in the same commit: the
  `crates/overdrive-core/tests/schema_evolution/workload_intent.rs` fixtures
  (per the precedent above), and `FIXTURE_V2` at
  `crates/overdrive-core/tests/schema_evolution/service_spec.rs:82`.
- **`FIXTURE_V1` at `service_spec.rs:77` is NOT regenerated, and its two
  freeze docstrings (`:72-76`, `:95-97`) stand unamended.** `ServiceSpecV1`
  (`service_spec.rs:110-116`) carries `id / replicas / exec / resources /
  listeners` and **no `ProbeDescriptor`**, so adding `ProbeDescriptor.idx`
  provably cannot shift its archived layout. `FIXTURE_V1` backs
  `service_spec_v1_decodes_through_current_envelope` (`:88-95`) — the one
  genuine "old persisted bytes still readable" assertion in the file. Touching
  it would forfeit the only real cross-version evolution signal to fix a layout
  that did not move. If regenerating `FIXTURE_V1` appears necessary, the change
  has grown beyond this ADR — stop and surface it.
- Regenerating the other two is the one sanctioned exception to `development.md`
  § "rkyv schema evolution"'s "existing fixtures are NEVER touched" rule. That
  rule protects the evolution signal *across released* layouts; the greenfield
  single-cut policy says there are none to protect, and the precedent above is
  the local ratification. The crafter regenerates rather than hand-edits and
  states in the commit body that the regeneration is this ADR's § D1.

### D2 — `role` joins the durable composite key

The key becomes `(alloc_id, role, probe_idx)`. **Amends ADR-0054 § 5.**

Key layout, extending `observation_backend.rs:128-136`:

```
alloc_id_bytes || 0x00 || role_byte || probe_idx LE u32
```

`role_byte` comes from a new method on the existing enum, per `development.md`
§ "Label enums own their string representation":

```rust
// crates/overdrive-core/src/observation/probe_result_row.rs
impl ProbeRole {
    /// Stable durable-key discriminant. Values are PERSISTED —
    /// never renumber; append only.
    pub const fn as_key_byte(self) -> u8 {
        match self { Self::Startup => 0, Self::Readiness => 1, Self::Liveness => 2 }
    }
}
```

The byte precedes `probe_idx` so a role's rows stay contiguous under the alloc
prefix, preserving ordered per-role iteration.

**Exact edit sites** (all must change together):

1. **`encode_probe_result_key`** — `crates/overdrive-store-local/src/observation_backend.rs:177`,
   invoked from `apply_probe_result_lww` at `:1183`. Gains the `role_byte`. Its
   signature takes `role` alongside the existing `(alloc_id, probe_idx)`.
   Its sibling `encode_probe_result_prefix` (`:190-196`) is **unchanged**: the
   scan ranges `[alloc||0x00, alloc||0x01)` (`:762-765`, applied `:771`), so
   inserting `role_byte` *after* the NUL is transparent to the prefix scan, and
   `list_probe_results_for_alloc` keeps its signature
   (`async fn list_probe_results_for_alloc(&self, alloc_id: &AllocationId) -> Result<Vec<ProbeResultRow>, ObservationStoreError>`,
   `:752-755`) and keeps returning all roles' rows in one scan.
2. **`SimObservationStore`** — `crates/overdrive-sim/src/adapters/observation_store.rs`.
   This adapter does **not** use a byte key; it holds
   `by_probe_results: Mutex<BTreeMap<(AllocationId, ProbeIdx), ProbeResultRow>>`
   (`:128`), composed as a tuple at `:451` and scanned by filter at `:471-474`.
   The requirement is therefore a **tuple widening** to
   `(AllocationId, ProbeRole, ProbeIdx)` in the same field/compose/scan
   positions — *not* byte identity. `ProbeRole` must derive `Ord` consistent
   with `as_key_byte` so the two adapters agree on iteration order, per
   `development.md` § "Trait definitions specify behavior, not just signature".
3. **Trait contract docstrings** —
   `crates/overdrive-core/src/traits/observation_store.rs`. Four sites assert
   the two-part key and are each falsified by this change: `:2168`
   ("`row.probe_idx` matches the spec's 0-indexed probe position"), `:2176-2177`
   ("the composite primary key `(alloc_id, probe_idx)`"), `:2178-2181` (the LWW
   clause), and `:2189` ("Concurrent writes for the same
   `(alloc_id, probe_idx)`"). All four are updated to the three-part key.

**D1 without D2 is forbidden.** Per-role indices without `role` in the key make
`startup[0]`, `readiness[0]` and `liveness[0]` collide on one key and clobber
each other under LWW — converting today's silent read-miss into silent durable
data loss. They land in one commit.

**Migration**: greenfield single-cut — the on-disk redb file is deleted.
`ProbeResultRowV1`'s **payload is unchanged** (`role` is already a payload field,
`probe_result_row.rs:162`), so no envelope bump and no probe-result fixture
change. Only key encoding moves.

#### D2a — TRC-ARCH-003 attempt-aware latest-row amendment

The D2 migration statement above describes the Stage-1 key correction. The
later TRC-ARCH-003 amendment keeps that exact key and evolves only the value:
append `ProbeResultRowEnvelope::V2` with
`alloc_attempt: Option<LogicalTimestamp>`, migrate V1 as `None`, and move both
public payload aliases to V2. V1 bytes/discriminant stay pinned; V2 receives its own
fixture per ADR-0048.

For the same `(alloc_id, role, probe_idx)` key, both adapters use this total
disposition before considering diagnostic wall time:

1. dominating `Some(new_attempt)` beats `Some(old_attempt)` regardless of
   `last_observed_at_unix_ms`;
2. equal `Some(attempt)` uses the existing strict wall-clock comparison;
3. older `Some(attempt)` loses;
4. `Some` beats legacy `None`, `None` loses to `Some`; and
5. `None` versus `None` uses the existing strict wall-clock comparison.

The ProbeRunner is the sole production writer and always supplies the accepted
Running row's `updated_at`. The key, scan order, list signature, and latest-only
cardinality do not change. This is not an attempt-keyed history.

### D3 — Consumers keep consulting per-role index 0; the `probe_idx == 0` predicate is now correct

**Stage-1 consumer shape (amended by TRC-ARCH-003).** `ServiceAllocFact` keeps its three scalar
fields — `latest_startup_probe` (`service_lifecycle.rs:114`),
`latest_readiness_probe` (`:150`), `latest_liveness_probe` (`:181`) — and the
three filters (`reconciler_runtime.rs:3023-3030`, `:3035-3042`, `:3046-3053`)
keep the `probe_idx == ProbeIdx::new(0)` predicate **verbatim**.

After D1 and D2 that predicate *means what every consumer already assumed*:
"this role's first declared probe". Readiness probe 0 now matches; liveness
probe 0 now matches. The defect closes without touching a single consumer.

TRC-ARCH-003 later adds `ServiceAllocFact.status_updated_at` and filters only
the liveness scalar to a V2 row whose `alloc_attempt` exactly equals that
Running logical identity. Startup/readiness behavior and the index-0 predicate
remain unchanged.

Consequently **unchanged**, and explicitly out of scope: `update_startup_attempts`
(`service_lifecycle.rs:939-943`, called `:539-543`), `startup_probe_failed_action`
(reads `:977`, derives `:979-982`), branch (a)'s Stable predicate (`:580-581`),
branch (c)'s `no_pass` (`:629`), both `ProbeWitness` constructions (`:562-567`,
`:587-592`), `spec_facts_for_service` (`reconciler_runtime.rs:1958-1969`),
`readiness_facts_for_service` (`:1978-1983`), `liveness_facts_for_service`
(`:1992-2000`), `compute_backend_healthy`'s `ProbeIdx::new(0)` counter key
(`service_lifecycle.rs:903`), and both `View` counter maps (`:297`, `:303`).

Two of those retentions carry a **pre-existing** inaccuracy that this ADR
neither introduces nor repairs, recorded so a reviewer does not read the
retention as an endorsement: `ProbeWitness { probe_idx: 0, .. }` and
`spec_facts_for_service`'s `startup_probes[0]` projection are correct only for
a single-probe role. They are exactly as wrong after this ADR as before it —
see § "A fourth, pre-existing gap this ADR deliberately does NOT address".

This is a deliberate narrowing. An earlier draft replaced the three scalars with
`BTreeMap<ProbeIdx, ProbeStatus>` and ratified per-role combinators. Adversarial
review showed that change breaks four unspecified consumers, contradicts the
per-alloc threshold projections (`.first()` at `reconciler_runtime.rs:1980-1981`
and `:1996`), and leaves `ProbeWitness.probe_idx` and `spec_facts_for_service`
undefined — while closing nothing the narrow fix does not close. See § A3.

### D4 — `Stable` stops only startup-role probes; it does not stop the supervisor

`TerminalCondition::Stable` is non-terminal (ADR-0055:1, :23-28, :255-258,
:267-276). Routing it through `on_alloc_terminal` is a category error.

**Supervisor state.** `AllocSupervisor`
(`crates/overdrive-worker/src/probe_runner/supervisor.rs:49-63`) currently holds
exactly `{ root: CancellationToken, started: bool }` and retains no per-task
state — `spawn_probe_task` (`:85-87`) returns a handle whose child token
`start_alloc` immediately consumes and drops (`probe_runner/mod.rs:324-325`).
Per-role cancellation therefore requires new state. It is added as a **per-role
intermediate token layer**, not a task collection:

```rust
// crates/overdrive-worker/src/probe_runner/supervisor.rs
pub struct AllocSupervisor {
    root: CancellationToken,
    /// Per-role intermediate tokens, each derived from `root` via
    /// `child_token()` and created on first use. A probe task's token
    /// is a child of ITS ROLE's token, so cancelling one role cancels
    /// only that role's tasks, while cancelling `root` still cancels
    /// every role in the same instant.
    per_role: BTreeMap<ProbeRole, CancellationToken>,
    started: bool,
}

impl AllocSupervisor {
    /// Now takes the spawning task's role and derives its token from
    /// that role's intermediate token.
    pub fn spawn_probe_task(&mut self, role: ProbeRole) -> ProbeTaskHandle;

    /// Cancel every task of `role`, leaving other roles and the
    /// supervisor itself running. Idempotent; a role with no tasks
    /// is a no-op.
    pub fn cancel_role(&self, role: ProbeRole);

    /// Whether `role` has a live (created, un-cancelled) token.
    /// The observation surface D7 item 4 asserts on.
    pub fn is_role_live(&self, role: ProbeRole) -> bool;
}
```

`BTreeMap`, not `HashMap`, per `development.md` § "Ordered-collection choice".
`spawn_probe_task` becomes `&mut self`; `start_alloc` already holds the
supervisor mutably (`probe_runner/mod.rs:311-312`, `get_mut`), so no locking
change is needed and the per-descriptor loop (`:323-352`) simply reborrows.
`cancel()` and `Drop` keep cancelling `root`, which still cancels every role
transitively — the atomicity claim at `supervisor.rs:45-48` stays true. The
existing `is_started()` guard (`probe_runner/mod.rs:319-321`) means a cancelled
role token can never parent a newly-spawned task.

**`is_role_live` reports token liveness, not task liveness.** That is a valid
proxy *only* because `supervised_probe_loop` has exactly one exit —
`child_token.cancelled() => return` (`probe_runner/mod.rs:593`) — so a live
token implies a live task. This coupling is load-bearing for D7 item 4; if a
future change gives the loop a second exit, the observable must be revisited.

**Three docstrings become false under this change and are corrected in the same
commit** (`feedback_behavior_change_must_mark_stale_adjacent_docs`):

- `supervisor.rs:42-43` — the struct docstring's claim to own "a [`JoinSet`]
  tracking every per-probe task". Already false today; the accurate note at
  `:52-55` says no `JoinSet` exists.
- `supervisor.rs:1-2` — the **module** docstring makes the identical false
  `JoinSet` claim.
- `supervisor.rs:26-29` — `ProbeTaskHandle`'s "Child token derived from the
  supervisor's root token" becomes inaccurate: the token becomes a child of its
  *role* token, i.e. a grandchild of `root`.

**`ProbeRunner` surface**:

```rust
// crates/overdrive-worker/src/probe_runner/mod.rs
/// Cancel every probe task of `role` under `alloc_id`, leaving the
/// supervisor and all other roles running. Cooperative shutdown only,
/// same discipline as `stop_alloc`. Idempotent: an unknown alloc, or a
/// role with no live tasks, is a no-op.
pub fn stop_role(&self, alloc_id: &AllocationId, role: ProbeRole);

/// Whether `role` is still supervised for `alloc_id`. Inspection
/// surface for tests and operator diagnostics, beside
/// `active_alloc_count` (`:380-382`).
pub fn is_role_live(&self, alloc_id: &AllocationId, role: ProbeRole) -> bool;
```

`stop_alloc` (`:367-375`) and `active_alloc_count` (`:380-382`) are
**unchanged**. `active_alloc_count` still reports 1 after a `stop_role` — the
supervisor is alive; only one role's tasks are cancelled.

**`Driver` trait** gains a third hook, symmetric with the existing pair
(`crates/overdrive-core/src/traits/driver.rs:498`, `:512`):

```rust
/// Lifecycle hook fired by the action shim when an allocation is
/// announced `Stable` (ADR-0055 — a NON-terminal condition).
///
/// Startup probing is bounded by the startup window and is complete at
/// Stable; readiness and liveness are continuous post-Stable per
/// `ProbeRole`'s contract. Implementations stop ONLY the startup-role
/// tasks and leave the supervisor alive.
///
/// Default no-op — symmetric with `on_alloc_running` / `on_alloc_terminal`.
fn on_alloc_stable(&self, _alloc_id: &AllocationId) {}
```

`ExecDriver` implements it beside `:765-769`:

```rust
fn on_alloc_stable(&self, alloc_id: &AllocationId) {
    if let Some(ref runner) = self.probe_runner {
        runner.stop_role(alloc_id, ProbeRole::Startup);
    }
}
```

**Action shim** replaces the unconditional call at `action_shim/mod.rs:1203`
with a branch on the `is_stable` flag it already computes:

```rust
if is_stable {
    driver.on_alloc_stable(&row.alloc_id);
} else {
    driver.on_alloc_terminal(&row.alloc_id);
}
```

This sits directly above the existing `if !is_stable { ... }` block
(`:1210-1223`), making all three teardowns consistently gated. The misleading
comment at `:1198-1202` is deleted and replaced with one stating the
non-terminal contract. The `StopAllocation` arm's `on_alloc_terminal` call
(`:1748`) is **unchanged** — a stop is a genuine terminal.

Per-role teardown rather than "leave everything running" is required because
`supervised_probe_loop` is **unbounded** — it ticks until cancelled
(`probe_runner/mod.rs:590-616`); `max_attempts` is enforced by the reconciler's
`StartupProbeFailed` branch, not the loop. Removing the teardown outright would
leave startup probes ticking forever post-Stable, contradicting
`probe_result_row.rs:77-78` and polluting the observation surface with
post-Stable startup rows that LWW into `latest`.

### D5 — `BackendDiscoveryBridge` takes sole ownership of `ServiceBackendRow`

Executes ADR-0079 § D9's leading candidate. `ServiceLifecycle` stops writing
`service_backends` entirely, deleting `last_emitted_backend_fingerprint` **by
construction** rather than by discipline.

**Moves to the bridge**: `compute_backend_healthy`
(`service_lifecycle.rs:893-921`) verbatim, and
`readiness_consecutive_successes: BTreeMap<(AllocationId, ProbeIdx), u32>`
(`:303`), relocated onto `BackendDiscoveryBridgeView`.

**Deleted outright**, per `development.md` § "Deletion discipline" (production
code and its tests go together — no gate, no salvage). All in
`crates/overdrive-core/src/service_lifecycle.rs` unless noted:

- `ServiceLifecycleView::last_emitted_backend_fingerprint` (`:387`) and its
  read/stamp sites (`:861-865`).
- `readiness_backend_row_action` (`:832-888`) and its call site (`:675`).
- `ServiceLifecycleState::service_dataplane` (`:247`) and `prior_backend_row_at`
  (`:253`), plus their hydration in
  `crates/overdrive-control-plane/src/reconciler_runtime.rs:2929-2952` —
  including `backend_port` (`:2935`) and the `backend_port` parameter of
  `hydrate_service_alloc_facts` (`:2986`, passed at `:2960`), which exist only
  to build `backend_addr`.
- `ServiceAllocFact::backend_spiffe` (`:166`) and `backend_addr` (`:168`), and
  their construction at `reconciler_runtime.rs:3077-3081` — the address
  duplication ADR-0079 § D8 had to keep byte-identical across two writers. Sole
  ownership **eliminates** that standing constraint; the bridge's expression
  (`backend_discovery_bridge.rs:396-399`) becomes the only one.

**`ServiceLifecycle` retains** (line numbers re-verified against HEAD):
`Action::FinalizeFailed` emissions at `:568`, `:593`, `:631`, `:779`, `:983`;
`Action::RestartAllocation` at `:798`; the startup gate; the liveness branch;
and the `View`'s `startup_attempts_per_alloc`, `liveness_consecutive_failures`
(`:297`), `stable_announced` (`:309`), `terminal_announced`, `observed`
(`:338`), `startup_last_fail_seen_at` (`:316`). It does **not** own
`ServiceSubmitEvent` — that is an API/streaming type, not in this file.

**Bridge state additions**:

```rust
// crates/overdrive-core/src/reconcilers/backend_discovery_bridge.rs
/// Per-Running-alloc readiness observation — the LWW-latest
/// `ProbeResultRow` at `(role = Readiness, probe_idx = 0)`. OBSERVED
/// INPUT; `healthy` is recomputed every tick, never persisted.
pub readiness_probe: BTreeMap<AllocationId, ProbeStatus>,
/// Intent-derived, uniform across the Service's allocs:
/// `(has_readiness_probe, success_threshold)`. Re-derived from the
/// live spec every tick per § "Persist inputs, not derived state".
pub readiness_facts: (bool, u32),
```

`readiness_facts` mirrors `readiness_facts_for_service`'s existing return shape
(`reconciler_runtime.rs:1978-1983`) exactly, so the bridge reuses that function
rather than introducing a second projection.

`compute_backend_healthy` moves with an **explicitly respecified** parameter
list, because the bridge supplies no `ServiceAllocFact` (D5 deletes two of its
fields) and no `ServiceLifecycleView`:

```rust
// relocated to crates/overdrive-core/src/reconcilers/backend_discovery_bridge.rs
fn compute_backend_healthy(
    alloc_id: &AllocationId,
    latest_readiness_probe: Option<&ProbeStatus>,
    readiness_facts: (bool, u32),          // (has_readiness_probe, success_threshold)
    next_view: &mut BackendDiscoveryBridgeView,
) -> bool
```

The body is otherwise verbatim from `service_lifecycle.rs:893-921`: the
`!has_readiness_probe → true` short-circuit (`:898-901`), the
`(alloc_id, ProbeIdx::new(0))` counter key (`:903`), the increment/reset arms
(`:904-917`), and the final predicate (`:919-920`).

**Hydration seam.** The bridge's hydrate arm
(`reconciler_runtime.rs:2764-2822`) today binds only `workload_id` (`:2765`),
`rows` (`:2766`), `running` (`:2778`), `listeners` (`:2792`),
`service_backends` (`:2793`) and `s` (`:2809`). It performs **no spec read at
all**, and `:2792`'s `hydrate_bridge_desired_listeners` returns *listeners*, not
a spec — the `ServiceV1` is parsed inside that helper (declared `:2118-2127`,
intent read `:2129-2137`, `from_store_bytes` `:2138-2143`, `Service` match
`:2144-2151`) and never escapes.

The seam is therefore pinned as a **return-type widening of the existing
helper**, NOT a second intent read:

```rust
// crates/overdrive-control-plane/src/reconciler_runtime.rs — replaces :2118-2127
async fn hydrate_bridge_desired_listeners(
    state: &AppState,
    workload_id: &WorkloadId,
) -> Result<(BTreeMap<ServiceId, BridgeListener>, ServiceV1), ConvergenceError>
```

(the listener half keeps its current element type verbatim; only the tuple's
second member is new). A **second, independent intent read inside the bridge arm
is forbidden** — it would create exactly the two-readers drift hazard the
one-source/two-readers discipline exists to prevent
(`workload_lifecycle.rs:1275-1281`), and the spec is already decoded one stack
frame away.

The arm then gains, after the `alloc_status_rows` read so the Running set is
known:

1. `readiness_facts_for_service(&spec)` (`:1978`) against the widened return's
   spec;
2. for each Running alloc, `list_probe_results_for_alloc(&alloc_id)` followed by
   the same LWW-latest projection the `ServiceLifecycle` arm uses
   (`reconciler_runtime.rs:3035-3042`). That projection is **extracted into a
   shared free function** in `reconciler_runtime.rs` and called from both arms,
   so the two readers cannot drift:

```rust
fn latest_probe_status(
    rows: &[ProbeResultRow],
    role: ProbeRole,
    probe_idx: ProbeIdx,
) -> Option<ProbeStatus>
```

Extracting it and repointing the three existing filters
(`:3023-3030`, `:3035-3042`, `:3046-3053`) at it is a **Stage 1** refactor, so
Stage 2 has one call site to reuse rather than a fourth copy to write.

`Backend.healthy` stops being carried through
(`backend_discovery_bridge.rs:377-379`) and becomes authored; the
`is_none_or(|b| b.healthy)` default-`true` disappears, and with it ADR-0079
§ D2's carry-through containment, which this decision supersedes.

**Cost, priced.** `list_probe_results_for_alloc` is per-alloc keyed
(`observation_backend.rs:752-793`), so the bridge pays **one redb prefix scan
per Running alloc per tick**, where today it pays zero. This is the identical
scan `hydrate_service_alloc_facts` already performs on the `ServiceLifecycle`
path (`reconciler_runtime.rs:3015-3019`) — so at the workload level it is a
**relocation, not an addition**, and becomes a net reduction once D5 deletes the
`ServiceLifecycle` readiness hydration. Row cardinality is bounded by spec, not
time (ADR-0054:503-505: N allocs × M probes, with M fixed by the spec).
Phase 1 is single-node with bounded alloc counts, and each scan is an ordered
range read over a contiguous prefix. Accepted.

**Rejected alternative** (restated from ADR-0079 § D9, still rejected): drop
`healthy` from the row and join at read time. The BPF program cannot join — the
flag must be *in* the map (`service_map_handle.rs:141`, `:336`).

### D6 — Sequencing: two stages, two commits, Stage 1 first

**Stage 1 = D1 + D2 + D3 + D4**, one commit. D1 and D2 are inseparable (D2's
forbidden-split rule). D3 is a no-op edit that D1+D2 give meaning to, plus the
`latest_probe_status` extraction D5 later reuses. D4 is independent but shares
every fixture and is required for Stage 1 to be observable at all.

**Stage 2 = D5**, a separate later commit.

Stage 1 is a **precondition** for Stage 2 being meaningful. Relocating `healthy`
authorship while the readiness path structurally cannot produce a value would
move dead machinery between reconcilers and make the relocation unverifiable —
the destination would compute the same constant. They **cannot** land together.

The ordering is also safest for the operator-visible failure: Stage 1 alone
converts `healthy` from a constant into a real function of observation, fixing
"a Service with a readiness probe is permanently unreachable" immediately.
Stage 2 is then a pure ownership refactor with no behaviour change, verifiable
by the tests Stage 1 introduces.

**Stage 1 file inventory** (sizing aid; not exhaustive of test files):

| File | Change |
|---|---|
| `overdrive-core/src/aggregate/probe_descriptor.rs` | add `idx` field |
| `overdrive-core/src/aggregate/workload_spec.rs` | assign `idx` in 3 parsers (`:1070`, `:1229`, `:1327`) + inference site (`:1111`) |
| `overdrive-core/src/aggregate/mod.rs` | re-assign `idx` in `ServiceV1::from_submit` (`:592-594`); carry `idx` through the `ServiceSpecV2 → ServiceV1` projection |
| `overdrive-core/src/observation/probe_result_row.rs` | add `ProbeRole::as_key_byte`; derive/confirm `Ord` consistent with it |
| `overdrive-core/src/traits/observation_store.rs` | 3-part-key contract docstrings at `:2168`, `:2176-2177`, `:2178-2181`, `:2189` |
| `overdrive-core/src/traits/driver.rs` | add `on_alloc_stable` (default no-op) |
| `overdrive-store-local/src/observation_backend.rs` | `encode_probe_result_key` (`:177`) gains `role_byte`; call site `:1183`; docstring `:128-136` |
| `overdrive-sim/src/adapters/observation_store.rs` | widen tuple key to `(AllocationId, ProbeRole, ProbeIdx)` at `:128`, `:451`, `:471-474` |
| `overdrive-worker/src/probe_runner/supervisor.rs` | `per_role` map; `spawn_probe_task(role)`; `cancel_role`; `is_role_live`; correct docstrings `:1-2`, `:26-29`, `:42-43` |
| `overdrive-worker/src/probe_runner/mod.rs` | consume `descriptor.idx` (`:323`, `:337`); delete comment `:332-336`; add `stop_role` / `is_role_live` |
| `overdrive-worker/src/driver.rs` | implement `on_alloc_stable` |
| `overdrive-control-plane/src/action_shim/mod.rs` | branch `:1203` on `is_stable`; rewrite comment `:1198-1202` |
| `overdrive-control-plane/src/reconciler_runtime.rs` | extract `latest_probe_status`; repoint filters `:3023-3030`, `:3035-3042`, `:3046-3053` |
| `overdrive-core/tests/schema_evolution/workload_intent.rs` | regenerate fixtures (§ D1) |
| `overdrive-core/tests/schema_evolution/service_spec.rs` | regenerate **`FIXTURE_V2` only** (`:82`); `FIXTURE_V1` (`:77`) and its freeze docstrings untouched |
| `overdrive-store-local/tests/integration/probe_result_roundtrip.rs` | `probe_results_per_alloc_per_probe_idx_independent_keys` (`:104-145`) extended to cover the role dimension |
| `overdrive-control-plane/src/reconciler_runtime.rs` (tests) | `hydrate_service_alloc_facts_probe_filter_requires_both_role_and_idx` (`:3868-3913`) — its `Startup/idx 1` mutant bait (`:3880-3889`) still constructs; re-verify it kills under the new key |

### D7 — Regression guards

The defect survived because every readiness test built `ServiceAllocFact` by
hand. The structural guard is a test that goes **through the hydrate boundary**.

Stage 1:

1. **Hydrate-boundary integration test** (`overdrive-control-plane`,
   `tests/integration/`): write `ProbeResultRow`s through a real
   `ObservationStore` for a Service with **one startup and one readiness probe**;
   call `hydrate_service_alloc_facts`; assert `latest_readiness_probe.is_some()`.
   Fails today. Direct guard for Mechanism 1. A liveness sibling covers the third
   instance.
2. **Durable-key separation test**: write `(alloc, Startup, 0)`,
   `(alloc, Readiness, 0)`, `(alloc, Liveness, 0)`; assert
   `list_probe_results_for_alloc` returns **three** rows. Fails under any
   implementation omitting `role` from the key — the guard for the catastrophic
   partial fix (A2).
3. **Adapter equivalence**: the same sequence against `SimObservationStore` and
   `LocalObservationStore`, asserting identical observable results, per
   `development.md` § "Trait definitions specify behavior".
4. **Stable-does-not-stop-supervision test**: with a fixture Service declaring
   **one startup AND one readiness probe** (both roles must be declared, since
   D4 creates per-role tokens on first use and an undeclared role has none),
   drive the alloc to `Stable` through the action shim with a wired
   `ProbeRunner`; assert `active_alloc_count() == 1`,
   `is_role_live(alloc, Startup) == false`, and
   `is_role_live(alloc, Readiness) == true`. Run once per Stable branch —
   empty-startup (`service_lifecycle.rs:568`) and startup-Pass (`:593`) — since
   both reach the hook. For the empty-startup branch the fixture declares
   readiness only, and the assertion is `is_role_live(alloc, Readiness) == true`
   alone.
5. **Terminal-still-stops test**: a genuine terminal and a `StopAllocation`
   (`action_shim/mod.rs:1748`) still drive `active_alloc_count()` to 0. Guards
   against D4 over-reaching.
6. **Proptest — index round-trip**: for arbitrary declared-probe combinations
   (including >1 per role), every descriptor's `idx` equals its position in its
   own role array and every `(role, idx)` pair is unique across the flat vector.
   The existing generators at
   `overdrive-core/tests/acceptance/intent_persists_probe_descriptors.rs:101-103`
   (`0..=3`) and `:358-360` (`1..=3`), and
   `overdrive-core/src/api/describe.rs:216-218` (`0..3`), keep their ranges —
   multi-probe remains representable (§ "A fourth, pre-existing gap"), and
   `:358-360` in particular now exercises the `from_submit` re-assignment D1
   adds.
7. **Multi-probe storage-distinctness test**: a Service with two startup probes
   writes rows at `(alloc, Startup, 0)` and `(alloc, Startup, 1)` that are both
   retrievable. Pins the D1+D2 side effect named in § "A fourth, pre-existing
   gap" — storage is correct even though consultation still reads index 0 only.
   `overdrive-core/tests/acceptance/workload_lifecycle_projects_service_probes_into_alloc_spec.rs`
   (two startup descriptors at `:146-149`, asserting `== 2` at `:191-194`)
   remains valid and is extended to assert the per-role `idx` values.
8. **Operator-surface updates** — every `probe_idx`-bearing render path, since
   D1 changes its meaning: `overdrive-cli/tests/acceptance/probes_section_render.rs`;
   `overdrive-cli/tests/acceptance/render_pure_fns.rs` (`:256`, `:265`, `:337`,
   `:342`, `:353`); the API projection (`api.rs:803`); and the
   `ProbeWitness.probe_idx` / `ServiceFailureReason::{StartupTimeout,
   StartupProbeFailed, LivenessProbeFailed}` renders
   (`overdrive-cli/src/render.rs:1024`, `:1226-1230`, `:1238-1239`). The rendered
   value is now truthfully the per-role index. `render_pure_fns.rs:265`
   (`"probe[2]"`) is the one assertion whose expected value may change; verify
   against its fixture rather than assuming.

Stage 2:

9. **Sole-writer invariant**: `ServiceLifecycle::reconcile` emits **no**
   `Action::WriteServiceBackendRow` for any input — the structural guard that the
   dual writer is gone.
10. **Dropped-write retry**: drop a `ServiceBackendRow` write at the store;
    assert the bridge re-emits next tick. The ADR-0079 § D4 exclusion finally
    closing — the behaviour `last_emitted_backend_fingerprint` made impossible.
11. **End-to-end readiness gate**: a failing readiness probe drives
    `Backend.healthy` to `false` through the real store, and recovery drives it
    back to `true`. Closes the gap recorded as "integration tests never exercise
    `healthy = false` against the store".
12. `bridge_carries_observed_healthy_through_on_rewrite`
    (`backend_discovery_bridge.rs:685`) is **deleted**, not rewritten — it
    defends a carry-through D5 removes. Per § "Deletion discipline", production
    code and its test go together.

Mutation gate ≥ 80 % per `.claude/rules/testing.md` applies to
`compute_backend_healthy` at its new home and to the key encoder — both are
reconciler-logic / durable-boundary surfaces and mandatory targets.

### D8 — `View` schema evolution

Both `View`s are CBOR-encoded in the runtime-owned `ViewStore`, so
`development.md` § "Reconciler I/O" → "Schema evolution" governs (not the rkyv
envelope rules).

- `ServiceLifecycleView` **loses** `last_emitted_backend_fingerprint` and
  `readiness_consecutive_successes` (both Stage 2). Serde's
  ignore-unknown-fields tolerance means a persisted view carrying them
  deserialises cleanly and the stale data is dropped. No envelope bump.
- `BackendDiscoveryBridgeView` **gains** `readiness_consecutive_successes`,
  annotated `#[serde(default)]`. It is currently field-less (ADR-0079 § D3,
  `backend_discovery_bridge.rs:256`); this is additive and stays
  backward-readable.
- The counter is **not migrated** across the two `View`s. It is a
  consecutive-success streak whose worst-case loss is one extra readiness tick
  before a backend is marked healthy — bounded, self-healing, strictly
  fail-closed. A cross-reconciler `View` migration to save one tick would be
  more machinery than the fact is worth.

---

## Consequences

### Positive

- A Service declaring `[[health_check.readiness]]` becomes reachable — today it
  is permanently `MeshUnreachable` and NXDOMAIN. The operator-visible headline.
- Readiness genuinely gates traffic; liveness genuinely restarts. Both are
  currently dead on the production path for any Service with a startup probe.
- `probe_idx` becomes truthful at every operator surface.
- The durable key can represent the specified contract; the
  three-roles-collide-on-one-key hazard becomes structurally impossible.
- Multi-probe rows stop sharing a key space across roles — probes 1..N are
  stored correctly and distinctly for the first time, even though consultation
  still reads index 0 only (§ "A fourth, pre-existing gap").
- `ServiceBackendRow` gets a single writer, deleting the last emit-time
  fingerprint in tree.
- ADR-0079 § D8's "the two expressions MUST stay byte-identical" standing
  constraint is **eliminated**, not merely documented.

### Negative

- **The multi-probe gap survives.** A Service declaring >1 probe of any role
  still has probes 1..N spawned, ticking and writing rows that no decision
  consults. This ADR narrows to the probe-0 defect deliberately (§ "A fourth,
  pre-existing gap"); the gap is neither closed nor promised.
- Greenfield single-cut discards existing `observation_probe_results` data and
  requires regenerating the `workload_intent` fixtures and `service_spec`'s
  `FIXTURE_V2` (§ D1). The rows are transient observations that re-derive within
  one probe interval.
- `ServiceV1::from_submit` gains an `idx` re-assignment — a behavioural change
  on the API/wire ingress, required so a wire client cannot inject a duplicate
  `(role, idx)` and collide rows under the new key.
- `AllocSupervisor` gains per-role state and `spawn_probe_task` becomes
  `&mut self` — a small widening of a hot-path struct.
- The bridge pays one probe-row prefix scan per Running alloc per tick. Priced
  in D5; a net reduction at the workload level.
- Stage 1 is a single sizeable commit (inventory in D6) because D1/D2 cannot be
  split without a data-loss window.

### Neutral

- `Stable`'s wire representation is unchanged; ADR-0055's structural
  `stable_announced` encoding is untouched.
- `project_probe_descriptors` keeps its concatenating body — the flat vector
  becomes a transport with no index semantics.
- No `ServiceAllocFact` field types change (§ D3), so the service-lifecycle
  reconciler's own consumers compile unmodified. Test files still change: the
  durable-key widening touches
  `overdrive-store-local/tests/integration/probe_result_roundtrip.rs:104-145`
  and the mutant-bait fixture at `reconciler_runtime.rs:3868-3913`, both listed
  in the D6 inventory.

---

## Alternatives considered

**A1 — Filter on `role` alone; keep the flat concatenated `ProbeIdx`.**
Delete the `probe_idx == 0` predicate and change nothing else. Rejected on three
counts, none of which depends on multi-probe being fixed: it leaves `ProbeIdx`
permanently contradicting ADR-0057:132-134, its own docstring
(`probe_result_row.rs:159`) and the shared-artifacts registry; it leaves the
operator-facing index showing an internal concatenation offset rather than the
operator's own array position; and — decisively — for a Service that *does*
declare multiple probes of a role it silently picks whichever row happens to be
LWW-latest across the whole role, which is a different wrong answer rather than
a fix. D1+D2 make the index mean what every consumer already assumes.

**A2 — Make `ProbeIdx` per-role without adding `role` to the key.** The
intuitive one-line fix. **Rejected as actively dangerous**: `startup[0]`,
`readiness[0]` and `liveness[0]` would encode to one key and clobber each other
under LWW, converting a silent read-miss into silent durable data loss.
Recorded explicitly because it is the shape a crafter is most likely to reach
for; D7.2 is its guard.

**A3 — Replace the three `latest_*_probe` scalars with maps and ratify per-role
combinators (AND-of-all readiness, OR-of-any liveness).** This ADR's own earlier
draft. Rejected on adversarial review: it breaks four unspecified consumers
(`update_startup_attempts`, `startup_probe_failed_action`, branches (a) and (c)),
contradicts the per-alloc `.first()` threshold projections
(`reconciler_runtime.rs:1980-1981`, `:1996`), leaves `ProbeWitness.probe_idx` and
`spec_facts_for_service`'s single-probe projection undefined, and — decisively —
solves a *different, pre-existing* problem from the one this ADR diagnoses,
while closing nothing the narrow fix does not close. That problem is bounded and
left open in § "A fourth, pre-existing gap"; see also § A8.

**A4 — Do not tear down probes at `Stable` at all.** Simpler than D4's per-role
surface. Rejected: `supervised_probe_loop` is unbounded
(`probe_runner/mod.rs:590-616`), so startup probes would tick forever
post-Stable, contradicting `probe_result_row.rs:77-78` and polluting the
observation surface with post-Stable startup rows that LWW into `latest`.

**A5 — Mint `WorkloadIntentV2` / `ServiceSpecV3` for the `ProbeDescriptor.idx`
addition.** The default reading of `development.md` § "rkyv schema evolution".
Rejected: it contradicts the precedent recorded for this exact change shape at
`aggregate/mod.rs:413-427`, and is not executable without forking
`ProbeDescriptorV1/V2` (a single shared struct reachable from both archives)
— machinery the greenfield single-cut policy exists to avoid.

**A6 — Land D1–D5 in one commit.** Rejected per D6: it makes the Stage 2
relocation unverifiable, because the destination would compute the same constant
the source does today.

**A8 — Reject >1 probe per role at the parser** (make the unspecified
multi-probe semantics unrepresentable). Carried as § D3b in this ADR's second
draft. Attractive under `development.md` § "Type-driven design", and it would
convert a silent lie into a loud error. **Rejected on blast radius and on
scope**: it invalidates live specification in three other ADRs (ADR-0055 § 5's
AND-of-all and its witness rule at :330-350 and :219; ADR-0058:166-172, whose
Alternative-C rejection *depends* on the AND-of-all gate; ADR-0054:342-344,
whose `BTreeMap` choice is justified by "walks every startup probe to find the
witness") plus `feature-delta.md:103`'s record of P2-Q7; it breaks in-tree tests
that legitimately construct multi-probe specs
(`workload_lifecycle_projects_service_probes_into_alloc_spec.rs:146-149`,
asserting `== 2` at `:191-194`; `probe_result_roundtrip.rs:104-145`) and the
mutant-bait fixture at `reconciler_runtime.rs:3880-3889`; and the parser is not
the only ingress, so the invariant would hold at one of two doors unless
`from_submit` (`aggregate/mod.rs:592-594`) also gated. Decisively: it addresses
a defect this ADR does not diagnose, while closing nothing the narrow fix does
not close. The boundary is recorded instead in § "A fourth, pre-existing gap".

**A7 — Keep dual writers; give `ServiceLifecycle` a write receipt** so it can
retry a dropped readiness write without a fingerprint. Rejected: it preserves
the two-writer arbitration ADR-0079 § D9 identified as the underlying problem,
and needs a receipt-returning `ObservationStore` surface ADR-0079 § D3
deliberately declined to add.

---

## Cross-references

- ADR-0054 § 5 — **amended** by D2 (durable key gains `role`). Its `BTreeMap`
  rationale at :342-344 ("walks every startup probe to find the witness") stays
  accurate as a statement of intent and stays unimplemented, unchanged by this
  ADR.
- ADR-0055 — its non-terminal `Stable` (:1, :255-258, :267-276) is the authority
  for D4. Its § 5 AND-of-all startup rule (:330-350) and its per-probe branch
  descriptions (:219, :228-238, :237-249) describe behaviour that is **still
  unimplemented** — as they did before this ADR. Not corrected here because this
  ADR does not change multi-probe behaviour in either direction; correcting them
  belongs with whatever closes § "A fourth, pre-existing gap".
- ADR-0057 — § 172 **completed** by D1. Its two-startup-probe example (:56, :63)
  is **left intact**: multi-probe stays parseable, so the example stays valid as
  syntax even though probe 1 remains unconsulted.
- ADR-0058 — unchanged. Its startup opt-out is confirmed startup-scoped; it is
  silent on readiness/liveness, and D4 rules that silence in favour of
  ADR-0057:163-164 ("no probes **of this role**").
- ADR-0079 — § D2 (carry-through) **superseded** by D5; § D4 (fingerprint
  exclusion) and § D9 (ownership) **closed** by D5; § D8's byte-identical-address
  constraint **eliminated** by D5.
- `.claude/rules/reconcilers.md` § "Codebase precedent" — the live
  `last_emitted_backend_fingerprint` instance is removed; that bullet is updated
  when Stage 2 lands.
