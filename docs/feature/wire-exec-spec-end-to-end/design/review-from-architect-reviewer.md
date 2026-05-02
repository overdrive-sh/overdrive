# DESIGN review — wire-exec-spec-end-to-end

**Reviewer**: nw-solution-architect-reviewer
**Date**: 2026-04-30
**Verdict**: APPROVE

## Critical (block ship to DISTILL if any)

None identified.

## Major (must address before DISTILL, but not a hard block)

None identified.

## Minor (worth fixing if cheap)

- [m1] Reuse Analysis table line 72 (`ResourcesInput`) justification could be more explicit about the gate — add phrase: "This separation gates `utoipa::ToSchema` adds from rkyv archive surface; extends to [microvm]/[wasm] configs without rkyv-side churn."

- [m2] ADR-0031 §Compliance lines 570-577 (newtypes rationale) could reference the project memory `feedback_newtypes_strict` for consistency, since the design explicitly overrides the STRICT default. Currently correct but backref helps future reviewers understand the decision pattern.

## Nitpick (style/wording, optional)

- [n1] ADR-0031 title is long; consider "Job spec: `[resources]` + `[exec]` tables; `Spawn`/`Restart` actions carry `AllocationSpec`" (drop "tagged-driver `JobSpecInput`" from title, retain in abstract §1). This is *optional* — current title is accurate.

## What works well

- **Reuse Analysis is exhaustive and evidence-based.** Every CREATE NEW decision (ResourcesInput, ExecInput, DriverInput) cites concrete evidence traced to architectural constraints, not preference. The state-layer-hygiene separation (ResourcesInput, line 72) is justified by the rkyv vs. serde surface split. The DriverInput tagged-enum (line 74) defends the flatten mechanism as load-bearing for implicit-by-table-name, citing Alternatives B and C rejection (ADR-0031 §10). Deletions (build_phase1_restart_spec, build_identity, default_restart_resources) are justified with precision against the feedback rules.

- **ADR-0031 alternatives are thorough with specific rejection rationales.** Alternative B's rejection nails the operator mental model mismatch ("typing the discriminator twice"). Alternative E's rejection defends the reconciler-has-the-job-in-scope principle by showing it preserves ADR-0023's stateless-dispatcher contract. The C4 component diagram (§10) visualizes the data-flow end-to-end, making state-layer boundaries visible.

- **Single-cut migration is operationally hardened.** No two-shape period proposed. Section §9 and Compliance lines 588-590 close the loop: no `#[serde(alias)]`, all ~25 fixtures migrate together, rkyv break is acknowledged as intentional (greenfield, no production data). This honors the project's single-cut-greenfield discipline rigorously.

- **Reconciler purity is preserved structurally.** Spec construction (wave-decisions D5) is a deterministic projection (two `.clone()` calls on rkyv fields), not I/O. Compliance line 559-560 documents the exact code shape: `Action::Start/RestartAllocation { ..., spec: AllocationSpec { command: job.command.clone(), ... } }` — no `.await`, no libSQL access. The DST twin-invocation invariant (ADR-0013) will catch any drift.

- **Action shim contract is honored (ADR-0023).** Restart variant grows `spec: AllocationSpec`, mirroring StartAllocation. The "carry everything the shim needs" principle is met: spec is fully populated before the shim runs. Deletions of build_phase1_restart_spec, build_identity, and default_restart_resources are justified with precision — no rebuild-from-obs-row necessary when spec is on the action.

## Reuse Analysis assessment (RCA F-1 HARD GATE)

**PASSED.**

**Every CREATE NEW decision has evidence:**
- ResourcesInput (line 72): state-layer hygiene constraint vs. direct rkyv serde (concrete; ADR-0031 referenced).
- ExecInput (line 73): no existing artifact carries exec-specific fields (concrete; field list justifies shape).
- DriverInput (line 74): tagged-enum flatten mechanism is load-bearing for implicit-by-table-name; Alternatives B and C rejected explicitly (concrete; ADR-0031 §10 referenced).

**All types introduced are in the table:** ResourcesInput, ExecInput, DriverInput (3 new); deleted functions (build_phase1_restart_spec, build_identity, default_restart_resources, test). No orphaned types.

**NonEmptyString critique addressed:** Design does NOT create a newtype for `command`. wave-decisions D3 (line 18-22) explicitly: "No newtypes — the driver passes these to `tokio::process::Command::new`..." Compliance lines 570-577 defend this rigorously: validation is constructor-side (`Job::from_spec`), not type-side, per `development.md` Newtypes rule. Justification is structural — type signature `AsRef<OsStr>` already constrains them. Not a weak "it's cleaner" rationale; it's a principled choice about validation sites.

## ADR cross-reference assessment

Spot-checks on 4 cited ADRs:

✓ **ADR-0011 (Aggregates and JobSpecInput collision):**
Claim: "Job::from_spec is THE single validating constructor" (ADR-0031 line 539).
Verification: ADR-0011 lines 74-75 confirm; §Compliance lines 111-112 state explicitly.
✓ Accurate.

✓ **ADR-0023 (Action shim placement):**
Claim: "Restart arm reads `spec` straight off the action; shim's stateless dispatcher contract preserved" (ADR-0031 lines 552-555).
Verification: ADR-0023 §2 shows shim signature takes `&dyn Driver, &dyn ObservationStore` — no IntentStore. Restart match arm shows shim consuming typed actions. Spec on action = no rebuild shim-side.
✓ Accurate.

✓ **ADR-0013 (Reconciler primitive runtime):**
Claim: "reconcile remains pure; no `.await`, no I/O" (ADR-0031 Compliance lines 556-560).
Verification: ADR-0013 §2c forbids `Instant::now()`, `SystemTime::now()`, `.await` inside `reconcile`. Design's spec materialisation is two `.clone()` calls on rkyv fields — no I/O.
✓ Accurate.

✓ **ADR-0030 (ExecDriver + AllocationSpec { command, args }):**
Claim: "This ADR is the operator-surface counterpart; ADR-0030 ratified the internal shape" (ADR-0031 Context §1, Compliance lines 532-535).
Verification: ADR-0031 Context §1 shows AllocationSpec struct shape matches ADR-0030 internal driver contract.
✓ Accurate.

## Verdict rationale

**APPROVE.**

This design is comprehensive, architecturally sound, and disciplined in its reuse analysis. The decision to wire operator-facing job spec (`[resources]` + `[exec]` tables) through a tagged-enum `DriverInput` is well-justified: the implicit-by-table-name TOML shape is operator-familiar (Nomad mental model), and the extension story is additive (future `[microvm]`, `[wasm]` add enum variants without restructuring existing code). The choice to NOT create a `NonEmptyString` newtype is defensible and evidence-based — validation belongs at the constructor, not in the type system, per the project's validation discipline.

The Reuse Analysis table meets the hard gate (F-1): every CREATE NEW has specific architectural evidence, no types are missing, deletions are justified with precision. The reconciler purity contract (ADR-0013) is preserved — spec materialisation in `reconcile` is pure deterministic projection. The action shim contract (ADR-0023) is honored — Restart grows `spec: AllocationSpec`, matching StartAllocation.

The single-cut migration is operationally disciplined: no aliases, all ~25 fixtures move together, rkyv break is intentional (greenfield, acknowledged). State-layer hygiene is enforced: wire-shape types stay separate from intent-shape types.

The two minor suggestions are ergonomic, not structural. No revisions required before handoff to DISTILL.
