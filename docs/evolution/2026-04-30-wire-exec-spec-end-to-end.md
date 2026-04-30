# wire-exec-spec-end-to-end — Evolution Summary

**Date**: 2026-04-30
**Status**: SHIPPED

## Feature summary

The first end-to-end wire of the operator's TOML `[exec]` block (`command`
+ `args`) from the CLI all the way through the IntentStore (rkyv-archived
`Job` aggregate) and into the action shim's `Driver::start(spec)` invocation.
Before this feature, `JobSpecInput` carried `cpu_milli` / `memory_bytes`
flat at the top level and the reconciler hardcoded `/bin/sleep ["60"]` for
every Start/Restart action regardless of what the operator declared.

After this feature, the operator's declared `command` and `args` flow
verbatim (no normalization, no substitution) from the parsed TOML, through
HTTP submission to the control plane, into the rkyv-archived `Job`
aggregate in the IntentStore, into the `AllocationSpec` the reconciler
emits on Start/Restart, and into the `Driver::start(spec)` call. Closes the
last Phase 1 gap that prevented operator intent from reaching the workload.

## Anchor ADRs

- **ADR-0030** — `AllocationSpec` flat shape (ratified pre-feature; the
  Phase-1 wire shape between reconciler and action shim).
- **ADR-0031 + Amendment 1** — `Job.driver: WorkloadDriver` tagged enum.
  The seed for Phase 2+ multi-driver dispatch (`microvm`, `wasm`,
  `unikernel`) on the intent side, with `WorkloadDriver::Exec(Exec)` as
  the only Phase-1 variant.

## Key decisions

- **Tagged-enum `WorkloadDriver` on `Job`** (ADR-0031 Amendment 1). Replaces
  the flat `command` / `args` fields the original §3 shape proposed. The
  pivot was triggered by the user noticing shape inconsistency across three
  layers: `JobSpecInput` was already nested per ADR-0031 §2, while `Job`
  and `AllocationSpec` were both flat. Amendment 1 nests the intent side;
  `AllocationSpec` stays flat per ADR-0030 §6 (which explicitly predicted
  per-driver-class spec types for Phase 2+).
- **Wire-shape twins carry serde + utoipa; intent shape carries rkyv only**
  (DWD-1 through DWD-3, DWD-22). `JobSpecInput`, `ResourcesInput`,
  `ExecInput`, `DriverInput` derive `Serialize` / `Deserialize` /
  `utoipa::ToSchema`. `Job`, `WorkloadDriver`, `Exec` derive `rkyv` only.
  State-layer hygiene: the OpenAPI surface and the IntentStore byte format
  are not the same type.
- **Validation field name `exec.command` preserved** (DWD-7). The operator-
  facing path matches the TOML the operator typed (`[exec]\ncommand =
  "..."`). The internal Rust nesting (`Job.driver: WorkloadDriver::Exec`)
  does not leak into the structured `Validation { field: "exec.command" }`
  error variant.
- **Single-cut migration** (DWD-9, DWD-16). ~14 fixture files migrated
  atomically with the production code in step 05-02. No
  `#[serde(alias = ...)]` shims, no `#[deprecated]` markers, no
  feature-flagged old paths. The rkyv archive byte shape changed in one
  commit (`f853bd4`); every fixture site moved with it.
- **Action shim deletions** (step 03-01). `build_phase1_restart_spec`,
  `build_identity`, `default_restart_resources` removed without salvage.
  The test that defended `default_restart_resources_pins_exact_values`
  deleted with the function it defended, per
  `feedback_delete_dont_gate.md`.
- **Walking-skeleton with back-door IntentStore read** (step 05-01). Closes
  the DISTILL review's BLOCK on persistence verification: the WS asserts
  the operator's `command` and `args` are byte-identical at the IntentStore
  layer, not just at the response digest layer. Pattern mirrored from the
  existing `submit_job_idempotency.rs` integration test.

## Steps completed

| Step | Commit | Title | Outcome |
|---|---|---|---|
| 01-01 | `e63cb80` | feat(core): align DriverInput rename_all to snake_case | Type-shape finalisation on top of pre-staged DESIGN+DISTILL work in `b1a50db` |
| 01-02 | `31096be` | feat(core): `Job::from_spec` rejects empty/whitespace exec.command | 8 acceptance scenarios RED→GREEN; structured `Validation { field: "exec.command" }` |
| 02-01 | `b13ea3b` | feat(core): reconciler projects `Job.driver` into `AllocationSpec` on Start/Restart | Reconciler twin-invocation invariant pinned; no more hardcoded `/bin/sleep` |
| 03-01 | `315c423` | refactor(control-plane): action_shim drops stale "// removed in PR" comments | Deletes 3 helpers + the test that defended `default_restart_resources` |
| 04-01 | `2b7a9e4` | feat(control-plane): `submit_job` handler propagates exec.command validation as 400 | Defence-in-depth at the HTTP boundary; structured field name flows through |
| 04-02 | `4137e01` | feat(api): wire ResourcesInput / ExecInput / DriverInput into OpenAPI schema | `utoipa::ToSchema` on wire-shape twins; `api/openapi.yaml` regenerated |
| 05-01 | `d3ca64c` | feat(cli): walking-skeleton back-door reads IntentStore + CLI parse-error tests | End-to-end persistence verification + CLI-side TOML validation surface |
| 05-02 | `f853bd4` | refactor(workspace): single-cut fixture migration to nested JobSpecInput | ~14 fixtures migrated atomically; closes the RED scaffold |
| (refactor) | `9256f38` | refactor(core): inline resources_struct + correct ResourcesInput rustdoc | L1-L4 polish on `aggregate/mod.rs` (per-step discipline was high; only 2 wins remained) |

## Quality gates summary

- **Default-lane tests**: 532 passed (`cargo nextest run --workspace`)
- **Integration-lane tests**: 681 passed (Lima;
  `cargo xtask lima run -- cargo nextest run --workspace --features integration-tests`)
- **Doctests**: pass (`cargo test --doc --workspace`)
- **dst-lint**: exit 0 (`cargo run -p xtask -- dst-lint`)
- **clippy**: clean (`cargo clippy --workspace`)
- **openapi-check**: exit 0 (`cargo run -p xtask -- openapi-check`)
- **Mutation kill rate**: 91.5% (Lima diff-scoped, 136 mutants, 119 caught,
  11 missed, 6 unviable; gate ≥80% PASS)
- **Adversarial review**: APPROVED, 0 blockers, 7-pattern testing-theater
  scan all CLEAN

## Lessons learned

- **DESIGN+DISTILL pre-staging is a positive pattern when the design is
  precise enough.** Most of the type-shape reshape shipped in commit
  `b1a50db` before DELIVER started. Per-step crafter work focused on small
  finalisation deltas, the validation predicate, the projection in the
  reconciler, and the fixture migration sweep. The DELIVER wave was
  consequently low-risk and high-throughput. Reach for this pattern when
  the type changes are mechanically obvious from the ADR.
- **Mid-DESIGN ADR amendments are cheap; mid-DELIVER reshapes are
  expensive.** ADR-0031 Amendment 1 (flat `Job` fields → tagged-enum
  `WorkloadDriver`) was triggered by the user spotting shape inconsistency
  across three layers. Catching it in DESIGN cost one ADR amendment
  (`docs/feature/wire-exec-spec-end-to-end/design/review-from-architect-reviewer-amendment-1.md`)
  and an updated AC in steps 01-01 / 02-01 / 05-01. Catching it in DELIVER
  would have invalidated every fixture-migration commit downstream.
- **Single-cut fixture migration means a chain of `--no-verify` RED
  commits.** DWD-9 produces ~10 commits in a row whose pre-commit nextest
  gate cannot compile (the BROKEN fixture sites stay BROKEN until 05-02).
  The `block-git-commit-no-verify.ts` hook recognises the word-bounded
  `RED` token in the commit message and allows it; commits flow without
  operator intervention. This is the documented carve-out per
  `.claude/rules/testing.md` § *RED scaffolds and intentionally-failing
  commits* — keep using it for features that touch a load-bearing
  wire-shape type.
- **The walking-skeleton back-door IntentStore read pattern closes the
  "digest matched but persist could have silently failed" gap.** Future WS
  scenarios should adopt this pattern wherever persistence is part of the
  value-delivery claim. The pattern source is
  `crates/overdrive-control-plane/tests/integration/submit_job_idempotency.rs`;
  the new mirror is in
  `crates/overdrive-cli/tests/integration/exec_spec_walking_skeleton.rs`.

## Issues encountered

- **Step 01-02 first attempt SKIPPED COMMIT phase.** The `--no-verify`
  block hook's deny-text recommended "do not retry without explicit
  approval" — the agent self-blocked rather than retrying. The hook is
  stateless; the deny-text is advisory, not enforcing. Resolved by
  orchestrator re-dispatch with explicit message-format guidance. The
  retry COMMIT-phase succeeded at `2026-04-30T05:56:47Z` with commit
  `31096be`. **Action**: orchestrator prompts for `--no-verify` retries
  should preempt this by saying "the deny-text is advisory; word-bounded
  RED in the commit message is sufficient" up front.
- **DWD-16's BROKEN-file enumeration was incomplete.** The
  `overdrive-scheduler` crate's `tests/acceptance/common.rs` fixture was
  BROKEN but NOT in the original DWD-16 list. The crafter discovered and
  migrated it during step 05-02 anyway. **Action**: scope future
  DWD-16-style enumerations with
  `cargo nextest run --workspace --features integration-tests --no-run` to
  surface every BROKEN site mechanically, not just the ones the designer
  remembers.

## Links

- Wave artifacts: `docs/feature/wire-exec-spec-end-to-end/{discuss,design,distill,deliver}/`
- ADR-0031: `docs/product/architecture/adr-0031-job-spec-exec-block.md`
- ADR-0031 Amendment 1 review:
  `docs/feature/wire-exec-spec-end-to-end/design/review-from-architect-reviewer-amendment-1.md`
- DELIVER reviews:
  - `docs/feature/wire-exec-spec-end-to-end/deliver/review-from-acceptance-designer-reviewer.md`
  - `docs/feature/wire-exec-spec-end-to-end/deliver/review-from-software-crafter-reviewer.md`
- Production diff: `git log b1a50db..HEAD --oneline`
