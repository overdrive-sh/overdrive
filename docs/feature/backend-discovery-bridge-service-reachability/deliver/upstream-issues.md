# Upstream issues discovered during DELIVER

**Wave**: DELIVER | **Date opened**: 2026-05-20

Issues surfaced during DELIVER implementation that reveal gaps or
contradictions in prior wave artifacts. Each entry names the
originating document and the deviation/rationale.

---

## UI-01 — `Backend` field shape in architecture.md § 4.2 is a pre-typed draft

**Surfaced in**: step 01-02 (commit `04ba6ca1`).

**Origin**: `docs/feature/backend-discovery-bridge-service-reachability/design/architecture.md` § 4.2 (lines ~290-300 of the architecture document) shows the reconcile body building:

```rust
let backends: Vec<Backend> = actual.actual.running.iter()
    .map(|_alloc_id| Backend {
        ipv4: self.host_ipv4,
        port: listener.port.get(),
        weight: 1,
        healthy: true,
        _pad: 0,
    })
    .collect();
```

**Production reality**: the canonical `Backend` type at `crates/overdrive-core/src/traits/dataplane.rs:56` is:

```rust
pub struct Backend {
    pub alloc: SpiffeId,
    pub addr: SocketAddr,
    pub weight: u16,
    pub healthy: bool,
}
```

The architecture document was authored against a pre-typed-`Backend`
draft (likely from an earlier dataplane iteration before the typed
`SpiffeId` + `SocketAddr` migration). The `ipv4 / port / _pad`
shape does not match production.

**Deviation taken in 01-02**: used the production `Backend` shape.
`alloc: SpiffeId` is derived via `mint_alloc_identity(workload_id,
alloc_id)` mirroring the existing `mint_identity` pattern at
`crates/overdrive-core/src/reconciler.rs:1843` (sibling
`ServiceMapHydrator` reconciler). `addr: SocketAddr` is built from
`(host_ipv4, listener.port)`. `weight: 1` and `healthy: true` are
hardcoded for Phase 2.2 (health-check probing deferred to GH #170
per architecture.md § 9; weight tuning out of scope for Phase 2.2).

**Rationale**: matching the production type is the only sound
choice — the architecture doc shape would not compile against the
current `crates/overdrive-core/src/traits/dataplane.rs`. The
sibling `ServiceMapHydrator` already uses the production shape, so
consistency holds.

**Action**: none required for DELIVER — the deviation is correct
and downstream steps (01-03, 01-04, 01-05) inherit the production
shape. Architecture.md § 4.2 should be amended post-feature to
reflect the production Backend type; this is documentation hygiene,
not a behavior change.

**Status**: ACCEPTED.

---

## UI-02 — `fingerprint` pure fn already exists in `overdrive-core::dataplane::fingerprint`

**Surfaced in**: step 01-02 (commit `04ba6ca1`).

**Origin**: `docs/feature/backend-discovery-bridge-service-reachability/design/architecture.md` § 4.1 and roadmap step 01-02's
implementation_scope listed
`crates/overdrive-control-plane/src/reconcilers/backend_discovery_bridge/fingerprint.rs`
as a new file housing the deterministic
`fingerprint(&ServiceVip, &[Backend]) -> BackendSetFingerprint`
pure fn.

**Production reality**: the function already lives at
`crates/overdrive-core/src/dataplane/fingerprint.rs` and is used
by the sibling `ServiceMapHydrator` reconciler. Re-implementing it
in a new module would duplicate a shared algorithm.

**Deviation taken in 01-02**: the new
`crates/overdrive-control-plane/src/reconcilers/backend_discovery_bridge/fingerprint.rs`
module is a thin re-export of
`overdrive_core::dataplane::fingerprint`. Honors the
architecture-mandated module placement (the path exists) without
algorithm duplication.

**Status**: ACCEPTED.

---

## UI-03 — `Instant::now()` in 01-02 test module trips dst-lint

**Surfaced in**: step 01-03 (during quality-gate run).

**Origin**: `crates/overdrive-core/src/reconciler/backend_discovery_bridge.rs:446,449` — the `tick(counter)` test helper landed by 01-02 (commit `04ba6ca1`) calls `Instant::now()` twice. `core` crates are scanned by `cargo xtask dst-lint`; both calls are flagged as banned-API violations per `.claude/rules/development.md` § "Reconciler I/O".

**Production reality**: `dst-lint` flags both calls (2 violations) on the parent commit `04ba6ca1` before 01-03 began. The lint was passed-through at 01-02 commit time (whether by an oversight or because the gate was not re-run after the commit landed).

**Deviation taken in 01-03**: no change. Documented here so a future step (or a focused remediation PR) can fix it deliberately. The fix is straightforward — use a deterministic `Instant` anchor (e.g. captured once via `OnceLock` at module init) — but the closure passed to `OnceLock::get_or_init(Instant::now)` is still detected by the AST scanner; the proper fix requires either a dst-lint scanner exemption for `#[cfg(test)]` modules, or replacing the `tick` builder with a `(now, deadline)` constructor that accepts the `Instant` as a parameter.

**Resolution (focused remediation, commit `516eee0d`)**: extended the dst-lint scanner's existing `cfg_test_depth` tracking from the `std::fs`-in-`async-fn` clause to the banned-API scanner (`Instant::now` / `SystemTime::now` / wall-clock / RNG paths). The two `Instant::now()` calls at lines 446 and 449 are now exempt because they live inside `#[cfg(test)] mod tests {}`. `HashMap` / `HashSet` (`BannedKind::OrderedCollection`) remains flagged in test code — iteration-order invariants apply to DST-trajectory assertions even in fixtures. `cargo xtask dst-lint` on the workspace reports zero violations.

**Status**: RESOLVED.

---

## UI-04 — Service-arm convergence gap: `read_job` discards Service driver/resources

**Surfaced in**: step 02-04 (walking-skeleton investigation; RCA at `rca-service-arm-convergence.md`).

**Origin**: `crates/overdrive-control-plane/src/reconciler_runtime.rs:1267-1275` — `read_job` returned `(None, Some(digest))` for `WorkloadIntent::Service`, discarding `ServiceV1.{id, replicas, resources, driver}`. The accompanying docstring at `reconciler_runtime.rs:1241-1244` treated this as the contract: *"For Service and Schedule kinds, `read_job` returns `Ok(None)` — the reconciler's `desired.job` field is `None` for those variants, which is the correct 'no Job allocation target' shape for Phase 1's Service-arm (allocations are not yet spawned for Services)."*

**Production reality**: this comment was a candid admission that the Service-arm *allocation-emission* gap was intentional in the hydrate layer, pending implementation in the reconciler — which never landed. The downstream consequence: every Service submit routed to the reconciler's `None`-arm GC branch (`crates/overdrive-core/src/reconciler.rs:1441-1464`), which only stops Running allocs and emits nothing for a never-started Service. `Action::StartAllocation` (the sole emission site, `reconciler.rs:1739`) was structurally unreachable for Service kind. Symptom: `state.obs.alloc_status_rows()` returned zero rows 10 s after a Service submit through the real HTTPS driving port; the downstream `BackendDiscoveryBridge` saw an empty `actual.running` set, no `ServiceBackendRow` was ever written, the `ServiceMapHydrator` never dispatched `DataplaneUpdateService`, and `S-BDB-01` (the walking-skeleton TCP round-trip through the VIP) was structurally impossible.

**The defect contradicted this feature's own design**: `docs/feature/backend-discovery-bridge-service-reachability/design/architecture.md:154-156` explicitly names step 3 as *"`WorkloadLifecycle.reconcile` emits `StartAllocation` (existing behaviour)"* — operating against the same hydrate-comment-as-contract that masked the gap. The existing `tests/acceptance/service_workload_convergence_no_panic.rs` regression test pinned panic absence (its bar was "convergence tick must not panic," added against an earlier `unreachable!()`) but never asserted that `StartAllocation` was emitted; the post-panic Service-arm behaviour went structurally unobserved.

**Resolution (focused remediation, commit `66935193`)**: `read_job`'s `WorkloadIntent::Service(svc)` arm now constructs a kind-agnostic `Job { id, replicas, resources, driver }` value from `ServiceV1`'s field-for-field-equivalent envelope and returns `(Some(job), Some(digest))`. `JobV1` and `ServiceV1` are structurally identical over `(id, replicas, resources, driver)` — the reconciler's `Some(job) => …` arm reads only these four fields, so the projection is lossless from its perspective. `ServiceV1.listeners` is consumed elsewhere via `ServiceV1`-typed reads, not through this projection. The `WorkloadKind::Service` discriminator continues to flow separately via `desired.workload_kind` (sourced from `read_workload_kind`) and is threaded onto `Action::StartAllocation` (`reconciler.rs:1750`) and `Action::RestartAllocation` (`reconciler.rs:1682`) so the action shim and observation rows correctly record `kind: Service` for Service-derived allocs. The fix also lands a new acceptance test (`crates/overdrive-control-plane/tests/acceptance/service_workload_emits_start_allocation.rs`) that asserts on emission shape — closes the coverage gap that let the defect slip past the sibling no-panic test. The acceptance test was written first, confirmed FAIL on parent commit `27e340b4` for the documented reason, then PASS after the H1 fix landed. Full crate suite (312 tests) and workspace suite (1410 tests) both green; `cargo xtask dst-lint` zero violations.

No reconciler-signature change was required (Option H2 from the RCA was rejected): the hydrate-layer projection preserves the kind-agnostic reconciler invariant.

**Status**: RESOLVED.
