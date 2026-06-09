# Wave Decisions — workload-identity-manager (GH #35 · roadmap step 2.13)

---

# DIVERGE Decisions — workload-identity-manager

**Wave**: DIVERGE (wave 1.5 of the nWave flow — optional, between DISCOVER and DISCUSS)
· **Agent**: Flux (nw-diverger) · **Date**: 2026-06-08 · **work_type**: brownfield
(net-new subsystem over the shipped `Ca` port) · **research_depth**: comprehensive

## Key Decisions

- **[D-WIM-1] Mint a new job J-SEC-002, distinct from J-SEC-001.** #35 is NOT the consumer
  surface of J-SEC-001 (CA minting). The two jobs are independently satisfiable and
  independently failable — J-SEC-001's progress is *forgery-resistance / no external PKI*;
  J-SEC-002's progress is *liveness/availability/lifecycle-binding of identity for the
  running set*. Different trigger (allocation lifecycle transition vs absence-of-CA),
  different failure mode (unidentifiable-running-workload / leaked-stopped-key vs forgeable
  identity). The udp-sendmsg4 precedent (elevate-under-existing-job) does NOT apply — that
  was the *same* job at finer granularity; this is a genuinely distinct job. (See
  `diverge/job-analysis.md` §3 for the full three-reason justification + counter-argument.)
- **[D-WIM-2] Recommend Option 1** — standalone `SvidLifecycle` reconciler + typed
  `Action::IssueSvid`/`DropSvid` + action-shim executor → shared `Arc<IdentityMgr>`;
  consumers read via sync getters behind an `IdentityRead` port. Weighted taste **4.45,
  rank 1, no weight adjustment**. (See `diverge/taste-evaluation.md`.)
- **[D-WIM-3] The reconciler-purity rule bounds the option space as a CORRECTNESS
  constraint, not a taste judgment.** "Issue synchronously via `Ca::issue_svid`" = the
  reconciler *emits an `Action`*; the action-shim executor calls the CA (mirroring
  `ServiceMapHydrator`→`Action::DataplaneUpdateService`→executor). Any option putting CA
  I/O inside `reconcile()` is excluded structurally. (See `diverge/options-raw.md` header.)
- **[D-WIM-4] The recommendation CONFIRMS the issue-pinned shape** (standalone
  `SvidLifecycle`, `Arc<IdentityMgr>`, persist-inputs, `StartWorkflow` rotation seam
  deferred to #40). This is the honest outcome — the roads-not-taken (kernel map, View-as-
  store, observation-row rebuild, fold-into-WorkloadLifecycle, watch-channel) are the
  documented dissent, not a manufactured contrarian pick. Competitive research independently
  converges on the same shape (shared identity-keyed in-memory store; rotation as a
  decoupled trigger — research L1/L3/L4).
- **[D-WIM-5] Rotation stays deferred to #40.** The near-expiry branch emits NO synchronous
  re-issue. Recommended seam: a no-op `Action::StartWorkflow(cert_rotation)` (or absent
  branch) — never a throwaway sync-rotate path (single-cut violation #40 would delete).
- **[D-WIM-6] No new GitHub issues created.** Every deferral maps to an existing issue
  (#40 rotation, #26 sockops consumer, #36 multi-node) or an existing roadmap line (4.7
  ACME unification). One product-decision BLOCKER (the 4.7 commitment) is surfaced for the
  user, not actioned.
- **[D-WIM-7] BLOCKER B1 RESOLVED — Option 1 LOCKED (user, 2026-06-08).** The user
  confirmed the 4.7 unified-`IdentityMgr` (SVID + ACME) commitment is **firm**. The
  dissent condition for Option 2 (fold into `WorkloadLifecycle`) therefore did NOT fire:
  4.7's public-trust ACME gateway certs have no allocation behind them, so a dedicated
  `IdentityMgr` is the correct seam and folding identity into the workload supervisor would
  be a seam 4.7 must later unbuild. Option 1 is the locked recommendation into DISCUSS — no
  longer conditional. (Recorded also in `recommendation.md` § Decision and `review.yaml`
  blocker B1.)
- **[D-WIM-8] BLOCKER B2 RESOLVED — pre-wire the #40 rotation seam, option (a) (user,
  2026-06-08).** The `SvidLifecycle` near-expiry branch is **pre-wired** in #35: the View
  persists the issuance-time *input* (issued-at / validity window — an input, not a derived
  `expires_at`, per persist-inputs) so near-expiry is computable, and the branch targets
  `Action::StartWorkflow(cert_rotation)` — the exact seam the issue names. **DESIGN caveat
  (grounded in code, NOT optional):** a committed `StartWorkflow` for an *unregistered*
  kind surfaces `WorkflowEngineError::UnknownWorkflow`
  (`overdrive-control-plane/src/lib.rs:417-418`); the action-shim isolates it per-action
  (`action_shim/mod.rs:429`) but the reconciler re-emits each tick the condition holds, so
  a naïve emit would raise `UnknownWorkflow` every tick until #40 registers `cert_rotation`.
  DESIGN MUST keep the actual emission **gated/dormant** (fires only once #40 registers the
  kind) so the pre-wired seam is a *clean* no-op, not an `UnknownWorkflow`-per-tick. #40
  then registers the workflow and flips the gate — no #35 View/branch rework. Single-cut-
  clean; still NO throwaway sync-rotate path. This was the one non-blocking DESIGN
  preference; the user resolved it here rather than deferring to DESIGN.

## Job Summary

- **Validated job:** J-SEC-002 — *"Keep every running workload holding a live, readable
  identity the dataplane can present — and nothing held for a workload that has stopped."*
  At physical/strategic level (irreducible function = bind-credential-lifetime-to-workload-
  lifetime + make-it-readable). `relates_to: J-SEC-001`.
- **ODI outcomes:** 6 (O1 running⇒held; O2 stopped⇒dropped; O3 consumer read latency;
  O4 restart idempotence; O5 no silent issuance; O6 mechanism economy). O1/O2/O3 are the
  high-opportunity under-served core.

## Options Evaluated

- **9 generated** (SCAMPER S/C/A/M/P/E/R + Crazy 8s X1/X2), **curated to 6** (M folded as
  a read-surface dimension; X1 folded into Option 2's family; X2 logged un-promoted),
  **6 survived the DVF filter** (none < 6; 4/5/6 cleared the bar but cluster low).
- **Recommended:** Option 1 — Shared `Arc<IdentityMgr>` + `SvidLifecycle` reconciler +
  `IssueSvid`/`DropSvid` actions — 4.45.
- **Dissent:** Option 2 — fold into `WorkloadLifecycle` (4.20) — wins iff the 4.7 unified-
  `IdentityMgr`-across-SVID+ACME commitment is dropped. Secondary: Option 3 (watch-channel
  push, 3.45) is the natural *next step* (getter→watch behind the `IdentityRead` port),
  not a competing foundation.

## SSOT Updates

- `jobs.yaml`: **created J-SEC-002** (full statement + dimensions + O1–O6 +
  `relates_to: J-SEC-001`) + changelog entry referencing this feature-id and the new-job
  verdict justification.

## Handoff

- **To DISCUSS (nw-product-owner):** `recommendation.md` (top-3 + dissent + decision
  statement + the design-sensitive surfaces DESIGN must pin + the 4.7 product BLOCKER) +
  the four `diverge/*.md` internal artifacts + `diverge/review.yaml`.
- **Peer review:** see § Review Record below + `diverge/review.yaml`.

## Review Record

- **nw-diverger-reviewer (Prism) — independent, 2026-06-08: APPROVE (zero nits).** All 5
  dimensions PASS (JTBD rigor | research quality | option diversity | taste correctness |
  recommendation coherence). Prism re-verified the matrix arithmetic by hand (Option 1 =
  4.450, Option 2 = 4.200) and pressure-tested the four high-stakes calls: J-SEC-002 mint
  EARNED, reconciler-purity PASS, single-cut PASS, confirmation-by-evidence (not anchoring)
  PASS. Run from the top-level orchestrator context (the DIVERGE subagent could not invoke
  the reviewer itself — Task tool unavailable in a subagent). Full record in
  `diverge/review.yaml` § `independent_review`.
