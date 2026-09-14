# DISTILL execution classification — microVM-only greenfield cut

## Classification

The corrected design authorizes no feature-specific deletion/absence test and
therefore no new pending RED scaffold.

| Test delta | Count | Classification |
|---|---:|---|
| New test functions | 1 | `BEHAVIOR_LOCK` — valid VM control plus bounded generated unsupported sibling key; GREEN now |
| Modified existing test functions | 4 | `BEHAVIOR_LOCK` — VM positive roundtrip, generic missing-driver, and production no-write evidence; GREEN now |
| New pending/ignored tests | 0 | Not applicable |
| Rejected new `remove_*.rs` files deleted | 8 | Removed; no compile/source-shape or legacy-spelling fixture remains |

No import, fixture, setup, or observable-boundary failure occurred in the
authored executable delta.

## Commands and results

- `cargo fmt --all -- --check` — passed.
- `git diff --check` — passed.
- Lima `cargo check -p overdrive-core -p overdrive-control-plane --all-targets
  --features integration-tests` — passed.
- Lima strict clippy over the same targets/features (`-D warnings`) — passed.
- Combined missing-supported-driver test plus the `api_type_shapes` module —
  15 passed in nextest run `442e808f-af3c-41bc-abfc-56bab5ced4cb`, including
  the final AT-B01 valid-VM control plus unsupported-sibling-key property.
- Production invalid-submit/no-intent-write test — 1 passed, nextest run
  `13668ccd-47e0-472a-9656-8c1152353eea`.
- Four surviving VM HTTP/TCP probe lifecycle tests — 4 passed, nextest run
  `640cd03a-cac6-4d01-b141-3d7b4b56d84b`.
- Pure P-105 `vm_recreation_allocation_identity` suite — 11 passed, nextest
  run `6299388b-142b-4a9e-afa6-6734f7b6e3ad`.
- Existing trusted-runner composition baseline — 1 passed, nextest run
  `42d4b1a9-d1b8-415c-8c49-a5bc2fe44fcb`; its former two-driver fixture must
  narrow to VM during DELIVER.

## Pre-existing flaky fixture — non-blocking and out of scope

The existing composed P-105 test
`successor_outcome_precedes_blocked_predecessor_cleanup_for_every_driver` was
observed failing in two local samples, but mandatory review ran 50 iterations:
**39 passed / 11 failed**. The printed seed `284105106` controls model data,
not the unbiased Tokio `select!` choosing between already-ready notification
branches. The oracle is schedule-racy and does not establish reproducible
seeded production behavior.

Classification: `PRE_EXISTING_FLAKY_FIXTURE / OUT_OF_SCOPE_FOR_293`.
It is not a #293 RED, not a completeness blocker, and does not authorize
production or test changes in this pure-deletion feature. The test is left
untouched. C7b is N/A for #293.

## Final gate posture

- Authored executable delta: GREEN behavior locks only.
- Completeness: **15/15 — COMPLETE**.
- Native-metal VM tests and planned E14 evidence were not executed in DISTILL.
- E06/E08 remain historical receipts only.
- Mutation testing was not run.
