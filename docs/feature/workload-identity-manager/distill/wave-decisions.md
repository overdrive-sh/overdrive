# Wave Decisions — workload-identity-manager DISTILL

**Wave**: DISTILL  
**Date**: 2026-06-08  
**Feature**: workload-identity-manager (GH #35)

## DWD-WIM-01 — Rust Scaffold Tests, No `.feature` Files

This project's ATDD policy and testing rules override generic Gherkin execution.
DISTILL creates Rust `#[test]` scaffolds and a prose
`distill/test-scenarios.md` companion. No Cucumber, pytest-bdd, or `.feature`
files are introduced.

## DWD-WIM-02 — ADR-0067 Rev 2 Supersedes Rev-1 Restart Wording

Acceptance scenarios use the rev-2 model: the held set is in-process only; after
restart it starts empty, so every still-Running allocation is re-issued during
recovery and every re-issue is audited. DIVERGE wording about recomputing held
state from persisted issuance inputs is historical and not executable.

## DWD-WIM-03 — Walking Skeleton Is Test-Tier, Not Operator-Tier

#35 is a foundation feature. Its walking skeleton proves issue, hold, audit,
chain verification, and drop at the test tier through real stores and
`openssl verify`. The operator-facing `alloc status` rendering of issued
certificates belongs to #215 and is not an acceptance criterion for #35.

## DWD-WIM-04 — Tier Selection

- Layer 1: pure `SvidLifecycle` reconciliation and View-shape contracts.
- Layer 1/2: action-shim and `IdentityRead` contracts with sim/fake adapters.
- Layer 2: `SimIdentityRead` equivalence and running-set DST invariant.
- Layer 3: real CA / real stores / `openssl verify`, gated by
  `integration-tests`.

## DWD-WIM-05 — Error-Path Coverage

Explicit negative scenarios cover audit-write refusal (S-WIM-07), read-after-drop
(S-WIM-05), View success-fact leakage (S-WIM-08), rotation emit gating (S-WIM-09),
broken hold/drop invariant teeth (S-WIM-11), and restart recovery (S-WIM-12). The
negative/guardrail ratio is 6 of 13 scenarios (46%); of these, 4 carry the
`@error` tag (S-WIM-05/07/09/12) and 2 are guardrail `@property` scenarios
(S-WIM-08/11). The remaining risk is covered by the walking skeleton and DST
invariant rather than additional example-only sad paths.

## DWD-WIM-06 — Outcome Registration

DISTILL introduces new typed contract surfaces and registers them in
`docs/product/outcomes/registry.yaml`: `SvidLifecycle`, `IdentityRead`, and the
running-set identity invariant.
