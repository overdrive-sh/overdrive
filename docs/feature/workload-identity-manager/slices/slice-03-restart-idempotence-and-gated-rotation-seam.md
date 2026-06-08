# Slice 03 — Restart-idempotence + gated #40 rotation seam

**Job**: J-SEC-002 | **Feature**: workload-identity-manager (GH #35) | **Story**: US-WIM-01 (O4 facet + the pre-wired seam)
**Release**: Release 2 (durability + the rotation seam)

## Goal (one sentence)

Make the held-identity set recoverable across a control-plane restart — the
`SvidLifecycle` View persists issuance *inputs* (not a derived `expires_at`), held
state recomputes on boot with no redundant re-issue — and pre-wire (but gate) the
#40 rotation seam so the near-expiry branch is a clean no-op until #40 registers
`cert_rotation`.

## IN scope

- The `SvidLifecycle` View persists issuance **inputs** (the issued-at /
  validity-window facts known at issuance time + the `AllocationId` / `SpiffeId` /
  issuer serial) per `.claude/rules/development.md` § "Persist inputs, not derived
  state". `BTreeMap`-keyed; the runtime owns persistence (bulk-load + write-through).
  The exact input shape is a DESIGN call (feature-delta Open-Questions #4).
- On control-plane restart, held state is recomputed from the persisted inputs +
  the running-allocation set: a Running alloc with a valid persisted issuance is
  NOT re-issued (idempotence O4); a Running alloc with no/expired persisted
  issuance IS (re-)issued.
- **Pre-wired-but-gated #40 rotation seam**: the `reconcile` near-expiry branch is
  structurally present — it recomputes near-expiry every tick from the persisted
  inputs + the live TTL policy (never a persisted `expires_at`) and targets
  `Action::StartWorkflow(cert_rotation)` — but the actual EMIT is **gated/dormant**
  until #40 registers the `cert_rotation` kind. The gate is the load-bearing part
  (see Caveat).
- DST: restart-mid-run scenario asserting recompute-from-inputs with no redundant
  `IssueSvid`; a seed-deterministic twin run.

## OUT scope

- The `cert_rotation` **workflow** itself (near-expiry → mint-fresh → swap →
  retire) → GH #40 (depends on #39 workflow primitive). This slice provides only
  the dormant seam (branch + View input), never a throwaway synchronous
  sync-rotate path (single-cut violation #40 would delete).
- Flipping the gate ON → #40 registers the kind and enables the emit, with no #35
  View/branch rework.
- The read surface → Slice 02. Issue/hold/drop core → Slice 01.

## Learning hypothesis

- **Disproves if it fails**: "held state is recomputable from persisted issuance
  *inputs* after a control-plane restart with no redundant re-issue, and the #40
  rotation seam is a clean no-op (no `UnknownWorkflow`-per-tick) until #40
  registers the kind." If recompute re-issues redundantly (orphaning the prior
  held cred / churning the audit trail), or the gated seam raises
  `WorkflowEngineError::UnknownWorkflow` every tick, the O4 idempotence outcome
  and the #40 seam are broken.
- **Confirms if it succeeds**: the subsystem survives restart cleanly, persists
  the right shape (inputs, not derived state), and #40 can register the workflow
  and flip one gate with zero #35 rework.

## Caveat (load-bearing — DIVERGE D-WIM-8, grounded in code)

A committed `StartWorkflow` for an **unregistered** kind surfaces
`WorkflowEngineError::UnknownWorkflow` (`overdrive-control-plane/src/lib.rs:417-418`),
isolated per-action by the shim (`action_shim/mod.rs:429`) but **re-emitted each
tick the near-expiry condition holds**. So a naïve emit raises `UnknownWorkflow`
every tick until #40 lands. The seam MUST keep the emit **gated/dormant** (fires
only once #40 registers `cert_rotation`) — pre-wired branch + View input, but a
*clean* no-op, never an `UnknownWorkflow`-per-tick. The exact gating mechanism is
a DESIGN call.

## Acceptance criteria

- [ ] The `SvidLifecycle` View persists issuance INPUTS (issued-at / validity-window + identity keys), never a derived `expires_at` / `next_renewal_at` (review-rejection smell); `BTreeMap`-keyed; runtime-owned persistence.
- [ ] After a control-plane restart, held state recomputes from the persisted inputs + the running set: a Running alloc with a valid persisted issuance is NOT re-issued; one with no/expired issuance IS re-issued. (O4 idempotence.)
- [ ] A DST restart-mid-run scenario proves a workload running across a control-plane restart keeps its held identity (recomputed from persisted issuance INPUTS) and is not redundantly re-issued; the surviving leaf still verifies under `openssl verify` at the TEST tier (observable O4 proof). (The operator `alloc status` render across a restart is deferred to **#215** O05, blocked on #35 — NOT this slice's AC.)
- [ ] The near-expiry branch is present and recomputes near-expiry every tick from persisted inputs + live TTL policy; the `Action::StartWorkflow(cert_rotation)` emit is gated/dormant and produces NO `UnknownWorkflow` while `cert_rotation` is unregistered.
- [ ] DST: restart-mid-run scenario asserts recompute-from-inputs, no redundant `IssueSvid`, and the gated seam emits nothing; seed-deterministic twin run.

## Dependencies

- Slice 01 (the issue path + the held store + the View to persist into).
- The reconciler-runtime ViewStore (bulk-load + write-through) — exists
  (`.claude/rules/development.md` § "Reconciler I/O").
- The `WorkflowEngine` / action-shim surface for the (dormant) `StartWorkflow`
  emit — exists; #40 registers the `cert_rotation` kind later.

## Effort estimate

~1.5 days (≤6h is tight given the restart-mid-run DST scenario + the gating). The
View-persists-inputs + recompute-on-boot mirrors the `RetryMemory` worked example
in `development.md`; the new parts are the held-state recompute + the gated seam
+ the restart DST scenario.

## Pre-slice SPIKE

Not needed — the persist-inputs/recompute pattern is documented (`development.md`
§ "Reconciler I/O" worked example) and the `UnknownWorkflow` behaviour is already
verified in code (`lib.rs:417-418`, `action_shim/mod.rs:429`). The gating
mechanism is a DESIGN decision, resolved before crafting.

## Taste-test note

Additive durability + the dormant seam on the proven Slice-01 loop. The O4 facet
is a genuine value behavior (a workload keeps its identity across a control-plane
restart with no redundant re-issue — NOT infra-only), so the slice carries
user-visible value alongside the seam plumbing. Production-data O4 proof: the DST
restart-mid-run scenario (held state recomputed from persisted issuance INPUTS, no
redundant `IssueSvid`) + `openssl verify` on the surviving leaf at the TEST tier.
(The operator `alloc status` render across a restart is #215's, blocked on #35 —
#35 is a FOUNDATION feature; F2.) Disproves a real pre-commitment (persist-inputs
idempotence + clean gated seam). Distinct from every other slice (the DURABILITY
+ seam tier). The #40 seam rides ALONGSIDE the O4 value story — the slice is not
seam-only.
