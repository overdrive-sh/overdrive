# Recommendation — workload-identity-manager (GH #35 · roadmap step 2.13)

**Wave**: DIVERGE → handoff to DISCUSS (nw-product-owner) · **Agent**: Flux (nw-diverger)
· **Date**: 2026-06-08

This is the navigable handoff artifact. Full reasoning lives in
`diverge/{job-analysis, competitive-research, options-raw, taste-evaluation}.md`. The
peer-review record is `diverge/review.yaml`.

---

## TL;DR

- **Job verdict:** Mint **J-SEC-002** (a *new, distinct* job — "every running workload
  holds a live, readable identity the dataplane can present"), `relates_to: J-SEC-001`.
  *Not* a sub-surface of J-SEC-001 — different progress, different failure mode, different
  trigger. Justified in `job-analysis.md` §3.
- **Recommended direction:** **Option 1** — a standalone `SvidLifecycle` reconciler +
  typed `Action::IssueSvid`/`DropSvid` + action-shim executor → shared `Arc<IdentityMgr>`;
  consumers read via sync getters behind an `IdentityRead` port. **Weighted taste score
  4.45, rank 1, no weight adjustment.**
- **This CONFIRMS the issue-pinned shape** (the dispatch's honest expected outcome). The
  roads-not-taken are the documented dissent — not a manufactured contrarian pick.
- **Dissent (2nd, 4.20):** Option 2 (fold into `WorkloadLifecycle`) wins *iff* the
  whitepaper's unified-`IdentityMgr`-across-SVID+ACME commitment (step 4.7) is dropped.
- **No new GitHub issues created** (project rule). One product-decision BLOCKER surfaced
  for the orchestrator (the 4.7 commitment — see § Blockers).

---

## Top 3 options

### 1 (RECOMMENDED) — Shared `Arc<IdentityMgr>` + `SvidLifecycle` reconciler + actions — 4.45

A standalone `SvidLifecycle` reconciler observes alloc Running↔Stopped and emits
`Action::IssueSvid` (on Running) / `Action::DropSvid` (on Stop). An action-shim executor
(`action_shim/issue_svid.rs`, mirroring `dataplane_update_service.rs`) calls the **shipped**
`ca_issuance::issue_and_audit` (which already binds issuance + the `issued_certificates`
audit row, ADR-0063 D6) and writes the result into a shared
`Arc<IdentityMgr>` holding `parking_lot::RwLock<BTreeMap<AllocationId, SvidMaterial>>` +
the current `TrustBundle`. Consumers (sockops #26 / gateway / telemetry) read via sync
getters behind an `IdentityRead` port trait (so the read surface is testable/mockable and
upgradeable). The reconciler View persists **issuance inputs** (not a derived `expires_at`)
for restart-idempotence (O4). The near-expiry branch emits a deferred
`Action::StartWorkflow(cert_rotation)` that is a **no-op until #40**.

- **Pro:** Exact mirror of the shipped `ServiceMapHydrator`→`Action`→executor pattern;
  reconciler stays pure; matches whitepaper §7 `Arc<IdentityMgr>`; cleanest #40 + #26 +
  4.7 seam; top DVF and Progressive-Disclosure scores.
- **Con / trade-off:** Slightly more new surface than Option 2 (a struct + reconciler + 2
  actions + 2 executors).
- **Key risk (must hold):** Identity warrants its **own** convergence target separate from
  `WorkloadLifecycle`. Supported by the 4.7 unified-store commitment + the SPIRE/istio
  precedent of a *dedicated* identity manager.
- **Hire criteria:** "Build identity the way the platform's other reconcilers are built —
  independently testable, clean seams for rotation and ACME."

### 2 (DISSENT) — No new reconciler: fold into `WorkloadLifecycle` / executor — 4.20

`WorkloadLifecycle` (already sees Running↔Stopped) emits the issue/drop actions — or, at
the thinnest end (X1), the alloc start/stop executor issues/drops inline with no new
Action. Held store still a shared `Arc<IdentityMgr>`.

- **Pro:** Fewest new mechanisms (top Subtraction); smallest cut.
- **Con / trade-off:** Couples identity into the workload supervisor; the "executor
  side-effect, no Action" end weakens DST observability of issue/drop (issuance becomes an
  un-actioned side effect, against the ADR-0023 typed-Action spirit).
- **Key risk (must hold):** Identity never needs a path independent of the allocation
  lifecycle — **contradicted by 4.7** (ACME gateway certs have no allocation behind them).
- **Hire criteria:** Hard surface/time pressure AND a decision to drop/defer 4.7's unified
  store.

### 3 (NEXT-STEP, not a competitor) — istio-SDS-style `watch`-channel push read surface — 3.45

Option 1's store, but the read surface is a `tokio::sync::watch`/`broadcast` channel:
consumers subscribe and are notified on change (the istio SDS push precedent, research L2).

- **Pro:** Top Speed; strongest #40 rotation-seam alignment (push rotated SVID down the
  same channel, no-restart swap).
- **Con / trade-off:** Adds a channel subsystem before any consumer exists to use it
  (#26/gateway/telemetry are unbuilt); channel-across-`.await` hazards.
- **Key risk:** That push is needed *now* — speculative with no consumer built.
- **Hire criteria:** Once a real consumer demands change-notification, or to pre-build the
  #40 seam. **This is the natural evolution of Option 1** (getter→watch is a non-breaking
  change behind the `IdentityRead` port), not a competing foundation.

*(Options 4 kernel-`IDENTITY_MAP` (2.60), 5 View-as-store (2.80), 6 observation-row-rebuild
(2.85) ranked below — full matrix in `taste-evaluation.md`. Option 4 ships #26-owned
kernel surface (single-cut violation); Option 5 can't hold the non-persistable leaf key in
the View; Option 6 gates held-set correctness on gossip convergence.)*

---

## Decision statement for DISCUSS

> **Proceed with Option 1** — a standalone `SvidLifecycle` reconciler emitting typed
> `Action::IssueSvid` / `Action::DropSvid`; an action-shim executor that mints via the
> shipped `ca_issuance::issue_and_audit` and writes a shared `Arc<IdentityMgr>`
> (`RwLock<BTreeMap<AllocationId, SvidMaterial>>` + current `TrustBundle`); consumers
> reading via sync getters behind an `IdentityRead` port trait; the View persisting
> **issuance inputs** (not a derived `expires_at`) for restart-idempotence; and the
> near-expiry rotation seam left as a deferred `Action::StartWorkflow(cert_rotation)` that
> is a **no-op until #40**. The key-risk assumption — that identity warrants its own
> convergence target separate from `WorkloadLifecycle` — is **CONFIRMED**: the user locked
> the 4.7 unified-`IdentityMgr`-across-SVID+ACME commitment as firm (2026-06-08, blocker B1
> resolved), and 4.7's public-trust ACME gateway certs have no allocation behind them, so a
> dedicated identity manager is the correct seam. **Option 1 is LOCKED, no longer
> conditional; Option 2 does not re-open.**

"Both options are viable" is explicitly **not** the decision: Option 1 is the locked
recommendation. Option 2 was the named fallback under one named condition (4.7 dropped) —
that condition did **not** fire.

---

## What DESIGN must still pin (design-sensitive surfaces — do NOT let a crafter invent)

These are gaps the issue under-specifies; they belong to DESIGN (architect), not to this
wave, and not to a crafter's initiative (CLAUDE.md § "Implement to the design"):

1. **The new `Action` variants' exact shape** — `Action::IssueSvid { alloc_id,
   spiffe_id, node_id, correlation }` and `Action::DropSvid { alloc_id, correlation }`
   (names/fields illustrative). The `Action` enum is `#[non_exhaustive]`-friendly; the
   exact field set is a DESIGN decision.
2. **The `IdentityRead` port-trait surface** — `svid_for(&AllocationId) ->
   Option<SvidMaterial>` + `current_bundle() -> TrustBundle` is the recommended shape;
   DESIGN pins the exact signatures + the `SimIdentityRead` test double.
3. **`IdentityMgr` concurrency primitive** — `parking_lot::RwLock<BTreeMap<…>>` is
   recommended (per `BTreeMap`-not-`HashMap` if iterated/observed; the map *is* iterated
   for the `assert_eventually!` invariant). DESIGN confirms.
4. **The View's persisted issuance-input shape** — what inputs (spiffe_id, issuer serial,
   issued-at, alloc_id) the `SvidLifecycle` View persists for O4 idempotence; must be
   *inputs*, never a derived `expires_at` (development.md § "Persist inputs").
5. **Where the trust-bundle currency comes from** (fork C) — DIVERGE leaves this open:
   pull `Ca::trust_bundle()` on demand vs. reconciler-hydrated into `IdentityMgr`. Research
   L6 favors holding the bundle in the same store as the leaves; the *currency mechanism*
   is a DESIGN call.

---

## SSOT change made by this wave

- `docs/product/jobs.yaml` — **added J-SEC-002** (full statement + functional/emotional/
  social dimensions + the six ODI outcomes O1–O6 + `relates_to: J-SEC-001`) and a
  changelog entry recording the new-job verdict and its justification.

---

## Blockers for the orchestrator (no GH issues created — per project rule)

1. **[PRODUCT DECISION — RESOLVED 2026-06-08: Option 1 LOCKED] The 4.7 unified-`IdentityMgr`
   commitment.** Option 1 (recommended) wins over Option 2 *because* the whitepaper commits
   `IdentityMgr` to also hold public-trust ACME certs for the gateway (step 4.7,
   whitepaper §11). **The user confirmed 4.7 is firm** — so Option 1 stands as the locked
   recommendation and the 1-vs-2 decision does NOT re-open in DISCUSS. (Had 4.7 been
   dropped/deferred, Option 2 — fold into `WorkloadLifecycle` — would have become
   competitive.) No GH issue created; resolved by direct user decision.

2. **[RESOLVED 2026-06-08 — pre-wire the seam, option (a)] The #40 rotation seam concretion.**
   The user chose **(a) pre-wire the seam** over (b) leave-absent-until-#40. #35's
   `SvidLifecycle` near-expiry branch is structurally present and targets
   `Action::StartWorkflow(cert_rotation)`, and the View carries the issuance-time *input*
   (issued-at / validity window — not a derived `expires_at`) so near-expiry is computable.
   **DESIGN caveat (load-bearing, grounded in code):** a committed `StartWorkflow` for an
   *unregistered* kind surfaces `WorkflowEngineError::UnknownWorkflow` (`lib.rs:417-418`),
   isolated per-action by the shim (`action_shim/mod.rs:429`) but re-emitted each tick the
   condition holds. So DESIGN MUST keep the actual emission **gated/dormant** until #40
   registers `cert_rotation` — the seam is pre-wired (branch + View input shape) but the
   emit is a *clean* no-op, never an `UnknownWorkflow`-per-tick. #40 registers the workflow
   and flips the gate, with no #35 rework. Still single-cut-clean; no throwaway sync-rotate
   path.

No deferral in this feature requires a *new* GH issue: rotation → existing #40; sockops
consumer → existing #26; multi-node → existing #36; ACME unification → existing roadmap
4.7. No invented issue numbers, no hand-wavy forward pointers.
