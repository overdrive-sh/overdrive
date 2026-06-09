# Slice 03 — Restart recovery (re-issue) + retry-memory View + gated #40 rotation seam

> **DESIGN RESOLVED THIS — implement ADR-0067 (rev 2), NOT the DISCUSS-era text
> below.** The DESIGN review REJECTED rev 1's "recompute held state on boot with no
> re-issue" as **impossible** (the leaf key is non-persistable — `CaKeyPem` has no
> `Serialize`, ADR-0063 D9 — and non-reconstructable — each `issue_and_audit` mints
> a fresh leaf, `ca_issuance.rs:34-40`). The corrected model (ADR-0067 rev 2, D1):
> **the held set IS the reconciler's `actual`; on restart `actual = ∅` → re-issue
> every still-Running alloc** (`running ∧ ¬held → IssueSvid`, bounded, audited). The
> View is **RETRY MEMORY** (`IssueRetry{attempts, last_failure_seen_at}` — D8), NOT
> issuance success facts (no `serial`/`issued_at`/`spiffe_id` — `serial` is a
> post-dispatch executor output and `next_view` persists BEFORE dispatch,
> `reconciler_runtime.rs:1222-1226` vs `:1324`). O4/K3 are reframed "no redundant
> re-issue" → "**bounded, audited restart re-issue; no stale/silent credential**"
> (see `design/upstream-changes.md`). Implement ADR-0067 rev 2 D1/D4/D8, not the
> "DESIGN call" / "recompute-without-reissue" wording anywhere below.

**Job**: J-SEC-002 | **Feature**: workload-identity-manager (GH #35) | **Story**: US-WIM-01 (O4 facet + the pre-wired seam)
**Release**: Release 2 (durability + the rotation seam)

## Goal (one sentence)

Make the held-identity set recoverable across a control-plane restart — on boot
the held set is empty (the leaf key cannot persist), so the `SvidLifecycle`
reconciler (held set = `actual`) **re-issues a fresh SVID for every still-Running
allocation** (`running ∧ ¬held → IssueSvid`, bounded, each audited via
`issue_and_audit`), with a **retry-memory View** so a *failed* re-issue backs off —
and pre-wire (but gate) the #40 rotation seam so the near-expiry branch (keyed off
the held cert's real `not_after` from `actual`) is a clean no-op until #40
registers `cert_rotation`.

## IN scope

- The `SvidLifecycle` View is **RETRY MEMORY** — `IssueRetry{ attempts,
  last_failure_seen_at }` (the `development.md` § "Reconciler I/O" `RetryMemory`
  shape) — so a *failed* `IssueSvid` (CA error / audit-write failure) backs off
  instead of re-firing every tick. `BTreeMap`-keyed; the runtime owns persistence
  (bulk-load + write-through). **NO `serial`/`issued_at`/`spiffe_id` success
  facts** (ADR-0067 D8): `serial` is a post-dispatch executor output the pure
  reconciler cannot know; "is this alloc held?" is answered by `actual`; the
  success fact lives in `issued_certificates`.
- On control-plane restart, the in-memory `IdentityMgr` is empty (held set never
  persisted — the leaf key cannot reach disk), so `actual = ∅`: the reconciler
  (held set = `actual`) emits `IssueSvid` for **every still-Running allocation**
  (`running ∧ ¬held`) — bounded (one per running alloc), each audited via
  `issue_and_audit`. This is RECOVERY (ADR-0067 D1), distinct from the gated #40
  near-expiry rotation. There is NO "recompute without re-issue" — that is
  impossible.
- **Pre-wired-but-gated #40 rotation seam**: the `reconcile` near-expiry branch is
  structurally present — it reads the held cert's real `not_after` off `actual`
  (`held_snapshot`; NEVER a persisted `expires_at`) and targets
  `Action::StartWorkflow(cert_rotation)` — but the actual EMIT is **gated/dormant**
  until #40 registers the `cert_rotation` kind. Rotation (`held ∧ near-expiry`) is a
  *distinct branch* from restart re-issue (`¬held`); keeping them separate ensures
  restart recovery never routes through the (forbidden) synchronous-rotation path.
  The gate is the load-bearing part (see Caveat).
- DST: restart-mid-run scenario asserting `actual = ∅` → re-issue every
  still-Running alloc (each leaving a fresh `issued_certificates` row), no
  stale/silent credential; a *failed* re-issue backs off via the retry-memory View;
  a seed-deterministic twin run.

## OUT scope

- The `cert_rotation` **workflow** itself (near-expiry → mint-fresh → swap →
  retire) → GH #40 (depends on #39 workflow primitive). This slice provides only
  the dormant seam (branch + View input), never a throwaway synchronous
  sync-rotate path (single-cut violation #40 would delete).
- Flipping the gate ON → #40 registers the kind and enables the emit, with no #35
  View/branch rework.
- The read surface → Slice 02. Issue/hold/drop core → Slice 01.

## Learning hypothesis

- **Disproves if it fails**: "a workload running across a control-plane restart is
  re-issued a fresh, audited SVID (bounded — one per still-Running alloc), the
  retry-memory View backs off a *failed* re-issue, and the #40 rotation seam is a
  clean no-op (no `UnknownWorkflow`-per-tick) until #40 registers the kind." If the
  restart leaves a Running alloc without a held SVID, or re-issues without an audit
  row, or hammers `IssueSvid` every tick on a CA failure, or the gated seam raises
  `WorkflowEngineError::UnknownWorkflow` every tick, the O4/K3 recovery outcome and
  the #40 seam are broken.
- **Confirms if it succeeds**: the subsystem recovers cleanly across restart
  (every still-Running alloc re-held with a fresh audited SVID, no stale/silent
  credential), the View carries the right shape (retry memory, not success facts),
  and #40 can register the workflow and flip one gate with zero #35 rework.

## Caveat (load-bearing — DIVERGE D-WIM-8, grounded in code)

A committed `StartWorkflow` for an **unregistered** kind surfaces
`WorkflowEngineError::UnknownWorkflow` (`overdrive-control-plane/src/lib.rs:417-418`),
isolated per-action by the shim (`action_shim/mod.rs:429`) but **re-emitted each
tick the near-expiry condition holds**. So a naïve emit raises `UnknownWorkflow`
every tick until #40 lands. The seam MUST keep the emit **gated/dormant** (fires
only once #40 registers `cert_rotation`) — pre-wired branch (keyed off
`actual.not_after`), but a *clean* no-op, never an `UnknownWorkflow`-per-tick. The
gating mechanism is RESOLVED in ADR-0067 D8: `const ROTATION_ENABLED: bool = false`
(or simply the absent emit). Implement that, not a "DESIGN call."

## Acceptance criteria

- [ ] The `SvidLifecycle` View is RETRY MEMORY (`IssueRetry{attempts, last_failure_seen_at}`), never issuance success facts (no `serial`/`issued_at`/`spiffe_id`) and never a derived `expires_at`/`next_renewal_at` (review-rejection smell); 6 derive bounds (`+Eq`) + manual `Default` (`UnixInstant: !Default`); `BTreeMap`-keyed; runtime-owned persistence.
- [ ] After a control-plane restart, the held set is empty (`actual = ∅`) and the reconciler re-issues a fresh, audited SVID for every still-Running allocation (`running ∧ ¬held → IssueSvid`); each re-issue writes an `issued_certificates` row; no Running alloc is left without a held SVID and no re-issue is silent. (O4/K3 — bounded, audited restart recovery.)
- [ ] A *failed* `IssueSvid` (CA error / audit-write failure) records `IssueRetry` and backs off (`now_unix >= last_failure_seen_at + backoff_for_attempt(attempts)`) — it does NOT re-fire every tick.
- [ ] A DST restart-mid-run scenario proves a workload running across a control-plane restart is re-held with a fresh, audited SVID; the surviving leaf verifies under `openssl verify` at the TEST tier (observable O4/K3 proof). (The operator `alloc status` render across a restart is deferred to **#215** O05, blocked on #35 — NOT this slice's AC.)
- [ ] The near-expiry branch is present and reads the held cert's real `not_after` off `actual` (NOT a View field); the `Action::StartWorkflow(cert_rotation)` emit is gated/dormant (`ROTATION_ENABLED = false`) and produces NO `UnknownWorkflow` while `cert_rotation` is unregistered.
- [ ] DST: restart-mid-run scenario asserts re-issue-of-every-running-alloc (each audited), retry-backoff on a failed issue, and the gated seam emits nothing; seed-deterministic twin run.

## Dependencies

- Slice 01 (the issue path + the held store + the View to persist into).
- The reconciler-runtime ViewStore (bulk-load + write-through) — exists
  (`.claude/rules/development.md` § "Reconciler I/O").
- The `WorkflowEngine` / action-shim surface for the (dormant) `StartWorkflow`
  emit — exists; #40 registers the `cert_rotation` kind later.

## Effort estimate

~1.5 days (≤6h is tight given the restart-mid-run DST scenario + the gating). The
retry-memory View mirrors the `RetryMemory` worked example in `development.md`; the
restart-recovery branch is the `running ∧ ¬held → IssueSvid` arm Slice 01 already
builds (re-running because `actual = ∅` on boot); the new parts are the
retry-backoff path + the gated seam + the restart DST scenario.

## Pre-slice SPIKE

Not needed — the retry-memory pattern is documented (`development.md` § "Reconciler
I/O" worked example), the held-set-as-`actual` is the Slice-01 convergence re-run
on an empty held set, and the `UnknownWorkflow` behaviour is already verified in
code (`lib.rs:417-418`, `action_shim/mod.rs:429`). The gating mechanism is resolved
in ADR-0067 D8 (`ROTATION_ENABLED = false`), not a "DESIGN call."

## Taste-test note

Additive recovery + the dormant seam on the proven Slice-01 loop. The O4/K3 facet
is a genuine value behavior (a workload running across a control-plane restart is
re-held with a fresh, audited SVID — no stale/silent credential, NOT infra-only),
so the slice carries user-visible value alongside the seam plumbing. Production-data
O4/K3 proof: the DST restart-mid-run scenario (`actual = ∅` → re-issue every
still-Running alloc, each audited; a *failed* re-issue backs off) + `openssl verify`
on the surviving leaf at the TEST tier. (The operator `alloc status` render across
a restart is #215's, blocked on #35 — #35 is a FOUNDATION feature; F2.) Disproves a
real pre-commitment (bounded audited restart re-issue + retry-memory backoff + clean
gated seam). Distinct from every other slice (the RECOVERY + seam tier). The #40
seam rides ALONGSIDE the O4/K3 value story — the slice is not seam-only.
