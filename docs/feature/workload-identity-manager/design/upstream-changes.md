# Upstream Changes — workload-identity-manager (DESIGN rev 2 → DISCUSS / product SSOT)

**Feature:** workload-identity-manager (GH #35 [2.13]) | **Job:** J-SEC-002
**Wave:** DESIGN (rev 2, 2026-06-08) | **Author:** Morgan (nw-solution-architect)
**Trigger:** the REJECTED-pending-revisions DESIGN review
(`docs/feature/workload-identity-manager/design/review-design.md`) — Critical
finding (restart-idempotence impossible as written).

This file records a DESIGN-wave back-propagation per the nw-design contract
(*"if architecture constraints require changes to user stories or acceptance
criteria, write them to `design/upstream-changes.md` for the product owner to
review"*). **The architect does NOT edit the product SSOT** — `jobs.yaml` is PO
territory. This record flags the edits the PO / orchestrator must make. **STATUS: ✅ APPLIED
2026-06-08 (orchestrator) — both product-SSOT sites below have been updated; this
record is retained for traceability.**

---

## The change — O4/K3 reframe: "no redundant re-issue" → "bounded, audited restart re-issue; no stale/silent credential"

### Why (the load-bearing fact the DESIGN wave surfaced)

The held `SvidMaterial` — including the node-held leaf private key — **cannot
survive a control-plane restart**:

- **Non-persistable:** `CaKeyPem` (the leaf private key wrapper) derives no
  `Serialize` (`crates/overdrive-core/src/traits/ca.rs:100`) — by ADR-0063 D9, the
  leaf key is in-process-only and must never reach disk (it is not an audit fact).
- **Non-reconstructable:** each call to `ca_issuance::issue_and_audit` mints a
  **FRESH** leaf with a distinct serial and a new validity window
  (`crates/overdrive-control-plane/src/ca_issuance.rs:34-40`) — there is no
  deterministic "re-derive the same leaf" path.

Therefore "recompute held state on boot **without re-issue**" — the rev-1 model —
is **impossible**. There is no source from which a post-restart `IdentityMgr` can
reconstruct a usable `SvidMaterial` containing the leaf key. The DESIGN review's
Critical finding is correct, and the DELIVER wave would have had no coherent
implementation path against the rev-1 K3/O4 target.

The honest model (DESIGN rev 2, ADR-0067 D1): **re-issue on boot** for every
still-Running allocation (`running ∧ ¬held → IssueSvid`, one per running alloc,
bounded, each audited via `issue_and_audit`). This is **RECOVERY** — distinct from
#40's scheduled near-expiry rotation. No secret at rest; every re-issue leaves an
`issued_certificates` audit row.

### What the PO must update (two product-SSOT sites) — ✅ APPLIED 2026-06-08

#### 1. `jobs.yaml` — J-SEC-002 outcome **O4**

> **ORIGINAL (verbatim, as it reads in the feature-delta DISCUSS mirror of the
> product SSOT — feature-delta.md § Opportunity-Outcomes, O4 row):**
>
> "**O4** | Minimize likelihood a control-plane restart leaves a running workload
> with no held SVID, **or re-issues redundantly** (idempotence; persist issuance
> INPUTS). | 13.0 | Under-served"

> **PROPOSED NEW WORDING:**
>
> "**O4** | Minimize likelihood a control-plane restart leaves a running workload
> with no held SVID, **or re-issues without an audit trail** (bounded, audited
> restart re-issue — no stale or silent credential). | 13.0 | Under-served"

The opportunity score (13.0) and the "Under-served" status are unchanged — only
the *target framing* shifts from an unachievable "no redundant re-issue" to an
achievable "bounded + audited, no silent credential."

#### 2. feature-delta DISCUSS **Outcome KPIs K3** row (`feature-delta.md`)

> **ORIGINAL (verbatim — feature-delta.md § Wave: DISCUSS / Outcome KPIs, K3
> row):**
>
> "| K3 | A control-plane restart | leaves a Running allocation with no held SVID,
> **or re-issues a redundant SVID for one already validly held** | 0
> missing-after-restart holds **and 0 redundant re-issues for already-validly-held
> allocations** | n/a (no held-state recompute exists today) | DST: restart the
> control plane mid-run; **held state recomputes from persisted issuance INPUTS
> with no redundant `IssueSvid`** (Slice 03) | Guardrail |"

> **PROPOSED NEW WORDING:**
>
> "| K3 | A control-plane restart | leaves a Running allocation with no held SVID,
> **or re-issues without leaving an `issued_certificates` audit row** | 0
> missing-after-restart holds **and every restart re-issue leaves an audit row**
> (bounded — one re-issue per still-Running alloc) | n/a (no held-state recovery
> exists today) | DST: restart the control plane mid-run; **every still-Running
> alloc is re-issued (`running ∧ ¬held → IssueSvid`), each writing an
> `issued_certificates` row; a surviving leaf verifies (`openssl verify`)** (Slice
> 03) | Guardrail |"

The guardrail *type* (Guardrail) and the K3 position in the metric hierarchy are
unchanged. The change replaces the unachievable "0 redundant re-issues" sub-target
with the achievable, stronger "every re-issue is audited" sub-target.

### Rationale for the reframe (not a weakening)

- The rev-1 target ("0 redundant re-issues" + "recompute without re-issue") was
  **un-meetable** — it assumed a capability the cryptography forbids. A KPI that
  cannot be met is not a guardrail; it is a contradiction DELIVER would have had to
  resolve by inventing surface (the failure mode CLAUDE.md § "Implement to the
  design" exists to prevent).
- The reframe preserves the *intent* — a restart must not leave a workload without
  identity (still the primary K3 clause, unchanged) and must not silently mint
  (now strengthened to "every re-issue is audited," which O5/K4 also reinforce).
- It is in fact a **stronger** no-silent-credential guarantee: the rev-1 framing
  was silent on auditing the (impossible) recompute; the rev-2 framing requires an
  `issued_certificates` row for every restart re-issue, which the operator surface
  (#215) renders as legible recovery rather than an anomaly.

### Downstream consistency (already handled in the DESIGN artifacts)

The DESIGN-owned artifacts have already been reworked to the new framing (the
architect owns these):

- **ADR-0067** (rev 2): Context driver 4, D1, D8, A3, Consequences, § Revision
  (rev 2), § Downstream boundary with #40/#215.
- **`docs/product/architecture/brief.md`** § workload-identity-manager: the
  Capability/DDD restart paragraph, the held-set-as-`actual` + enqueue subsections,
  the O4/K3 quality scenario, the #40/#215 Out-of-scope boundary rows.
- **`docs/product/architecture/c4-diagrams.md`** § Workload Identity Manager: the
  L1 CI-gate K3 label, the L2/L3 intros, the L3 component diagram (held-set
  projection + enqueue handoff).
- **feature-delta.md** § Wave: DESIGN: DDD-2/4/7/9/12, Component Decomposition,
  Decisions Table, Reuse Analysis, Open Questions, Handoff, § Changed Assumptions.
- **slice-03** (and slice-01/02): the restart model corrected to re-issue
  (recovery), the View to retry memory.

Only the **two product-SSOT rows above** (`jobs.yaml` O4 + the feature-delta
DISCUSS K3) **were applied by the orchestrator on 2026-06-08** (the architect left them
unedited per the back-prop contract; the PO/orchestrator applied the wording).

---

## Note — `jobs.yaml` is NOT edited by the architect

Per the project convention (`CLAUDE.md` — product SSOT / PO territory) and the
nw-design back-propagation contract, this file is the architect's *record of the
needed change*, not the change itself. The PO / orchestrator applies the two
wording updates above to `jobs.yaml` (J-SEC-002 O4) and the feature-delta DISCUSS
K3 row. No GitHub issue is created (no new scope — this is a wording reframe of an
existing outcome). No `gh` mutation is performed.
