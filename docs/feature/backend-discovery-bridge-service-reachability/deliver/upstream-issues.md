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

**Status**: ACCEPTED (pre-existing from 01-02; surface to user before any remediation PR).
