# DESIGN amendment review — wire-exec-spec-end-to-end (ADR-0031 Amendment 1)

**Reviewer**: nw-solution-architect-reviewer
**Date**: 2026-04-30
**Verdict**: APPROVE

## Critical (block ship to DISTILL re-run if any)

None. No critical issues detected.

## Major (must address before DISTILL re-run, but not a hard block)

None.

## Minor (worth fixing if cheap)

None.

## Nitpick (style/wording, optional)

None detected in the review scope. The amendment is cleanly structured and consistently marked.

## What works well

1. **Consistency principle is sound.** The wire-shape `JobSpecInput` already carries a tagged-enum `DriverInput`; mirroring this on the intent-shape `Job` with `WorkloadDriver` is consistency-with-existing-design, not premature abstraction. The amendment correctly frames this as responsive to the user's explicit finding, not speculative.

2. **ADR-0030 §6 is correctly understood.** The amendment's claim that "per-driver-class spec types are the Phase 2+ shape ADR-0030 already committed to" is **exactly accurate**. ADR-0030 §6 states: "The right shape when those drivers land is per-driver-type spec types — likely a `Spec` enum with `Spec::Exec(ExecSpec)`, `Spec::MicroVm(MicroVmSpec)`, `Spec::Wasm(WasmSpec)`" and explicitly documents the shared `AllocationSpec` as "a Phase 1 simplification." The amendment correctly defers the Phase 2+ split to future ADR work and preserves the flat `AllocationSpec` shape in full fidelity with ADR-0030's predicted shape.

3. **Naming is unambiguous.** `WorkloadDriver` (not bare `Driver`) disambiguates cleanly from the `Driver` trait in `traits/driver.rs`. No pre-existing `WorkloadDriver` or bare `Exec` struct exists in the codebase — collision scan confirms zero conflicts. The inner name `Exec` (not `ExecSpec`) under the qualified path `WorkloadDriver::Exec(Exec)` is the cleanest naming option.

4. **Reconciler purity is preserved.** The new populating expression `let WorkloadDriver::Exec(Exec { command, args }) = &job.driver;` is sync, pure, and contains no `.await`, no `Instant::now()`, no I/O — fully compliant with ADR-0013's reconciler contract. This is a deterministic projection of a hydrated aggregate, not a side effect.

5. **Validating-constructor discipline is unbroken.** `Job::from_spec` remains THE single path into the intent aggregate per ADR-0011. The DriverInput-to-WorkloadDriver projection happens inside that constructor, preserving single-source-of-truth validation. The `exec.command` rule fires on both CLI and server lanes by construction.

6. **Action shim contract is unchanged.** The shim's signature (per ADR-0023) takes `&dyn Driver` and `&dyn ObservationStore`, and consumes flat `AllocationSpec` from the action. The amendment explicitly preserves this — only the populating expression inside the reconciler changes, not the shim's surface. The shim remains a stateless dispatcher.

7. **State-layer hygiene is maintained.** `JobSpecInput` (wire-shape, serde, deny_unknown_fields) and `Job` (intent-shape, validated, rkyv-archived) remain distinct types. The projection from `DriverInput` to `WorkloadDriver` happens at the boundary (inside `Job::from_spec`). No cross-layer type aliasing or merging.

8. **OpenAPI is unaffected.** The amendment correctly notes that `WorkloadDriver` and `Exec` do NOT derive `utoipa::ToSchema` — they are intent-shape, not wire-shape. The OpenAPI schema continues to be derived from `JobSpecInput` and its wire-level inputs only. The wire-shape `utoipa` derives are unchanged.

9. **CLAUDE.md consistency rule is addressed.** The amendment explicitly tackles the "don't design for hypothetical future requirements" rule with Rationale point 4: the Job reshape is "responsive to a present requirement (wire-shape consistency the user explicitly named)," not speculative. The justification is that the abstraction already exists in the wire shape (`DriverInput`); the amendment is alignment, not invention. This reasoning is sound.

10. **Supersession markers are unambiguous.** The Status block and the Amendments section clearly mark which sections are SUPERSEDED and by which amendment. The original §§3-6, §10 are retained as historical record with explicit SUPERSEDED labels inline. Amendment 1 is self-contained as the new SSOT.

11. **Migration scope is explicit.** The amendment enumerates downstream artifacts (DISTILL test-scenarios, production scaffolds, existing acceptance tests, roadmap ACs) that must update. No artifact is left unaccounted for. The "no two-shape acceptance period" discipline is maintained — single-cut migration in the DELIVER PR.

12. **C4 component diagram is regenerated.** The amendment includes a full C4 diagram showing the new types in place, not a "see Amendment" footnote on the original diagram. The diagram is self-contained and fully describes the new shape.

## ADR-0030 §6 cross-reference verification

**Result: ACCURATE.** ADR-0030 §6 explicitly ratifies per-driver-type spec types (`Spec::Exec(ExecSpec) | Spec::MicroVm(MicroVmSpec) | Spec::Wasm(WasmSpec)`) as the Phase 2+ shape and describes the shared `AllocationSpec` as a Phase 1 simplification. The amendment's reading is not ambiguous — it is the exact, unqualified meaning of the section. This is the single strongest gate, and it passes cleanly.

## Premature-abstraction tension

**Result: ADDRESSED.** The amendment tackles CLAUDE.md's "don't design for hypothetical future requirements" rule head-on (Rationale point 4). The case made is:

- **On `AllocationSpec` (flat, staying flat):** This would be premature. ADR-0030 already predicted the Phase 2+ shape; foreclosing it now with a `WorkloadDriver` discriminator would contradict ADR-0030 §6. Correct deferral.

- **On `Job` (flat → tagged-enum):** This is NOT premature because:
  1. The wire-shape `JobSpecInput.driver: DriverInput` already carries a tagged enum (existing, present).
  2. The user explicitly named this shape inconsistency as a design problem (responsive, not speculative).
  3. Alignment between wire and intent layers is a consistency principle the project already follows.

The amendment correctly distinguishes the two cases and does not over-apply the hypothetical-requirements rule. The reasoning is sound: the abstraction exists in the wire shape; the amendment aligns the intent shape to match.

## Naming collision spot-checks

**Grep verification results:**

- `grep -n "pub trait Driver" crates/overdrive-core/src/traits/driver.rs`: ✓ Confirmed, line 142. Named `Driver`.
- `grep -rn "WorkloadDriver" crates/`: ✓ Zero pre-existing matches. No collision.
- `grep -rn "pub struct Exec[^I]" crates/`: ✓ Zero matches (only `ExecInput` and `ExecDriver` exist; no bare `Exec`). No collision.
- `grep -rn "pub enum Exec" crates/`: ✓ Zero matches. No collision.

**Naming is collision-free. `WorkloadDriver` is unambiguously distinct from the `Driver` trait. The `Exec` struct naming is appropriate under the qualified `WorkloadDriver::Exec(...)` path.**

## Supersession integrity

**Result: CLEAN.**

- Status block (line 10-19) points at Amendment 1 and explicitly marks §§3-6, §10 as SUPERSEDED.
- Inline section headers carry explicit SUPERSEDED markers (e.g., line 78: "**BEFORE** (original §3, now SUPERSEDED)"; line 215: "// BEFORE (original §5, Amendment 1 supersedes)").
- Amendment 1 is self-contained (lines 21-401) and fully readable without referencing the original sections — the new SSOT property is satisfied.
- C4 diagram (lines 254-291) is regenerated in Amendment 1 form, not a footnote. Green = new; yellow = changed. Type shapes are explicitly shown.

## State-layer hygiene

**Result: PRESERVED.**

- `JobSpecInput` (wire: lines 493-523 of adr-0031) and `Job` (intent: lines 93-98 of Amendment 1) are distinct types.
- `DriverInput` (wire enum, line 505) projects into `WorkloadDriver` (intent enum, line 109) inside `Job::from_spec` (lines 174-188).
- The projection boundary is explicit: `match input.driver { DriverInput::Exec(exec_input) => WorkloadDriver::Exec(Exec { ... }) }`.
- Reverse projection `From<&Job> for JobSpecInput` is mentioned as existing (line 368) and projects back `WorkloadDriver → DriverInput`.
- No type aliasing or boundary blurring detected.

## Migration-impact completeness

**Result: COMPLETE.** The amendment enumerates:

1. **DISTILL artifacts** (test-scenarios.md, ACs) — lines 354-361
2. **Production scaffolds** (aggregate/mod.rs, reconciler.rs, action_shim.rs) — lines 363-376
3. **Existing scaffold files** (acceptance tests, integration tests) — lines 378-385
4. **Roadmap step ACs** — lines 387-392

All four categories are accounted for. The orchestrator's DISTILL dispatch note (line 385) correctly defers roadmap update to the next orchestrator dispatch.

Spot-check: reconciler.rs line 369 is mentioned as the exact file needing the destructure change. Action_shim.rs line 374 is mentioned as unchanged (only original §6 deletions apply). These are the two most critical sites, and both are explicitly covered.

## Verdict rationale

The amendment is **architecturally coherent, internally consistent, and correctly grounded in the existing ADRs it claims to build on.** The user's observation (wire-intent shape inconsistency) is real and material. The architect's choice (B2: nest Job, keep AllocationSpec flat) honors ADR-0030's explicit Phase 2+ commitment while making the intent layer consistent with the wire layer. The naming is clear, the supersession is unambiguous, the migration scope is complete, and the purity contracts are unbroken. The amendment successfully addresses the "premature abstraction" tension by correctly distinguishing responsive alignment (wire-intent matching) from speculative design (foreclosing ADR-0030's predicted Phase 2+ shape). **Ship to DISTILL re-run.**
