# Bugfix RCA — mixed local+remote backends loop without backoff under the cross-route validator rule

**Feature ID**: `fix-mixed-backend-dispatch-spin`
**Surfaced via**: code review on `crates/overdrive-control-plane/src/action_shim/validate.rs:159-167`.
**RCA validated by**: @nw-troubleshooter (5 Whys, evidence-cited).
**Solution researched by**: @nw-researcher — `docs/research/reconcilers/dispatch-boundary-validation-and-attempt-budget-backoff.md` (8 trusted sources, High confidence).
**Design-artifact reconciliation by**: @nw-solution-architect — ADR-0053 revision 2026-06-03 + D11 evolution-doc scope clarification (already landed; uncommitted in working tree at RCA time).

---

## Defect (one line)

A service with BOTH local and remote backends causes `ServiceMapHydrator::reconcile` to emit a `DataplaneUpdateService` (XDP route) + `RegisterLocalBackend` (cgroup route) for the same VIP in one tick; `validate_reconcile_output` rejects this legitimate ADR-0053 §4 dual-path as a "cross-route conflict", dispatch is skipped, no `ServiceHydrationStatus` row is ever written, and `should_dispatch`'s unconditional `None | Pending => true` arm re-emits the conflicting pair every tick with no backoff — an unbounded tight loop that never converges.

---

## Two independent root causes

### Root cause A (primary, design-contradiction)

The validator's cross-route arms (`validate.rs:160-167` XDP-after-cgroup, `181-188` cgroup-after-XDP) treat ANY XDP-write + cgroup-write on the same VIP as a conflict. This **contradicts the already-accepted ADR-0053** §2/§4/§5, which specifies the mixed-backend dual-path as correct:

- §2 (lines ~295-300): `update_service(vip)` and `register_local_backend(vip, port)` for the same VIP are *"not mutually exclusive at the adapter."*
- §4 (lines ~390-394): a mixed service *"emits both action kinds for the same `service_id`. The two dataplane paths are independent and the trait contract permits this concurrent dual-path."*
- §5 (lines ~408-413): no precedence race — `cgroup_connect4` rewrites local connects at `connect(2)` *before* the kernel routes the SYN to XDP ingress. Two disjoint kernel maps, two hooks, disjoint backend sets.

The validator **misattributes** its rule to *"Phase 16 D11"* (`validate.rs:16`). The real D11 finding (`docs/evolution/2026-05-23-backend-discovery-bridge-service-reachability.md` § "Reconcile-output invariant…", lines ~332-354) is about two SAME-CLASS `WriteServiceBackendRow` writes with conflicting backend sets — a genuine same-slot overwrite. D11 says nothing about cross-route composition. The cross-route rule is an over-generalization of D11.

**External precedent confirms** (`disjoint key space ⇒ no conflict`): Kubernetes Server-Side Apply keys conflicts on individual owned fields; Cilium runs socket-LB (`cgroup connect4`) and XDP/tc LB as complementary transparent surfaces for one ClusterIP. Correct conflict granularity is `(route, key-tuple)`, never the shared parent VIP.

### Root cause B (independent contributor, localized code bug)

`should_dispatch` (`service_map_hydrator.rs:434-452`) applies the retry-memory backoff window ONLY on the `Failed` arm. When dispatch is suppressed (cause A, or any other suppression) the `service_hydration_results` row is never written, so `actual_status` stays `None`, the `None | Some(Pending) => true` arm fires unconditionally every tick, and the loop is *tight* — `attempts` climbs forever, throttling nothing. Retry inputs (`attempts`, `last_failure_seen_at`, `last_attempted_fingerprint`) are already persisted in `RetryMemory` but never consulted in the `None`/`Pending` path.

**External precedent** (client-go workqueue `ItemExponentialFailureRateLimiter`): backoff is keyed on requeue/attempt count alone, independent of observed object status; level-triggered controllers reset backoff on spec (fingerprint) change.

---

## Solution (researched + user-decided)

### Fix A1 — narrow the validator to `(route, key-tuple)` granularity
- **Verified 2026-06-03 against HEAD `99733646`:** the `(vip, port, proto)` widening **already landed** via committed steps 02-01 (`12611316`, SERVICE_MAP) and 02-02 (`0876de79`, LOCAL_BACKEND_MAP). The validator already has `xdp_keys: BTreeSet<(Ipv4Addr,u16,Proto)>` and `cgroup_keys: BTreeSet<(Ipv4Addr,u16,Proto)>`. So the "widen the trackers" task is a no-op — do NOT redo it. (The session's original RCA assumed pre-widening trackers; that read was stale.)
- The remaining A1 work: **remove the two cross-route match arms** (XDP-after-cgroup `~182-194`, cgroup-after-XDP — later in file) **and the now-dead `xdp_vips` / `cgroup_vips` cross-route trackers** (their ONLY consumers are the cross-route arms; after removal they are dead code → delete per deletion discipline).
- Fix the **stale committed comment** at `validate.rs:153-155` (`"the cgroup path carries no proto yet, step 02-02"`) — 02-02 landed; `cgroup_keys` carries Proto. Bring comment into line with code while applying the architect doc prose.
- Apply the architect's verbatim module-doc replacement (drops "Conflict class 2" as a conflict; records cross-route as the §4 dual-path; corrects the D11 citation). Verbatim prose in the architect handoff — see `## Architect doc prose` below.
- Flip the two cross-route rejection tests to assert **acceptance** (rename `validate_rejects_*_for_same_vip` → `validate_accepts_xdp_and_cgroup_for_same_vip` per deletion/test discipline). Keep same-route same-slot rejection tests; extend them to exercise the proto dimension (tcp/53 + udp/53 = distinct slots, no conflict).

### Fix B — attempt-budget on `should_dispatch` `None`/`Pending`
- Lift the `Failed`-arm gate (`now >= last_failure_seen_at + backoff_for_attempt(attempts)`) into a shared helper.
- The `None | Pending` arm stops returning `true` unconditionally: apply the backoff window when `retry` shows prior attempts at the SAME `last_attempted_fingerprint`; dispatch immediately (and the next emission resets the window) when `desired_fingerprint != last_attempted_fingerprint`. Uses only inputs `RetryMemory` already persists — no `next_attempt_at` (honors "Persist inputs, not derived state").

### Fix C — queryable observation row for genuine conflicts *(User decision 2026-06-03: escalate from log-only)*
- Add a new observation row capturing a genuine same-slot reconcile-output conflict (the surviving violation class after A1), so operators can query it rather than grep tracing logs.
- **Crosses the rkyv schema-evolution boundary**: new row needs a `VersionedEnvelope` enum + `Latest`/payload aliases + discriminant-offset const + a golden-bytes fixture under `crates/overdrive-core/tests/schema_evolution/` per `.claude/rules/development.md` § "rkyv schema evolution" and `.claude/rules/testing.md` § "Archive schema-evolution roundtrip". Template: `ServiceHydrationResultRowEnvelope` (`observation_store.rs:783-861`).
- `run_convergence_tick` writes the row on `Err(violation)` (keep the existing structured `reconciler.output.invariant_violation` tracing event too — surface-then-continue; do NOT import controller-runtime `TerminalError` stop-semantics, per `.claude/rules/reconcilers.md` self-heal posture).

---

## Posture (retained, not changed)

Converge-and-retry-never-stop. Once A1 lands the spurious per-tick violation storm vanishes. For a *genuine* same-slot self-conflict: loud structured signal + queryable observation row (Fix C) + skip-dispatch-but-persist-View + retry next tick. No hard stop — the appliance OS has no operator shell (`.claude/rules/reconcilers.md`: "the system must self-heal").

---

## Files affected

| File | Change | Fix |
|---|---|---|
| `crates/overdrive-control-plane/src/action_shim/validate.rs` | Remove cross-route arms + `cgroup_vips` tracker; widen trackers to `(vip, port, proto)` / `(vip, vip_port, proto)`; apply architect module-doc prose; flip/extend tests | A1 |
| `crates/overdrive-core/src/reconcilers/service_map_hydrator.rs` | Attempt-budget gate on `should_dispatch` `None`/`Pending` arm; shared backoff helper | B |
| `crates/overdrive-core/src/traits/observation_store.rs` | New conflict observation row + `VersionedEnvelope` + discriminant-offset const | C |
| `crates/overdrive-core/tests/schema_evolution/*` | Golden-bytes fixture for the new row | C |
| `crates/overdrive-control-plane/src/reconciler_runtime.rs` | Write conflict row on validator `Err`; (optional) wire hydrator `RetryMemory` into `view_has_backoff_pending` (GH #160) | B/C |
| `docs/product/architecture/adr-0053-...md`, `docs/evolution/2026-05-23-...md` | **Already landed by architect** (commit pending) | A |

---

## Regression tests (the primary deliverable)

1. **Mixed-backend accepted (A1):** a reconcile output with `DataplaneUpdateService(vip)` + `RegisterLocalBackend(vip, port)` for one VIP passes `validate_reconcile_output` (was rejected). RED against current code.
2. **No-spin convergence (A1+B):** a hydrator tick for a mixed-backend service dispatches both actions; the next tick with unchanged fingerprint and no status row does NOT re-dispatch immediately (backoff window engaged). RED against current code (spins).
3. **Genuine same-slot conflict still rejected + observed (A1+C):** two XDP writes to `(vip, port, proto)` (or two cgroup writes to one slot) are rejected AND produce the queryable observation row.
4. **Proto dimension (A1):** tcp/53 + udp/53 on one VIP are distinct slots — accepted.

---

## Out of scope (do NOT expand without user approval)

- Progressive (exponential) backoff — `backoff_for_attempt` is constant 1s today (pre-existing TODO #137). Fix B throttles the spin to ~1 Hz; true exponential backoff is #137's concern.
- Full wiring of hydrator `RetryMemory` into the runtime re-enqueue predicate is GH #160 (deferral, tracked) — only touch if Fix B requires it; otherwise leave to #160.
- IPv6 VIP conflict class — GH #155.
