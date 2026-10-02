# Regenerated DELIVER Roadmap Remediation — `netns-density-295`

## Metadata

- Role: isolated `nw-solution-architect` roadmap remediation
- Date: 2026-10-02
- Input review: `deliver/review-roadmap-regenerated.md`
- Remediated artifact: `deliver/roadmap.json`
- Scope: documentary correction of top-level metadata and pending phases 05-10
- Frozen history: phases 01-04 preserved exactly; `execution-log.json` untouched
- Validation status: `pending`, awaiting independent re-review
- Source-code inspection: none

## Authoritative contract

The remediation applies the already accepted correctness-recovery contract in
`feature-delta.md`: D-295-R1 through D-295-R22, ADR-0127 through ADR-0143,
the final DISTILL follow-ups, and the section *DELIVER re-roadmap input*. Test
ownership, body homes, substrates, typed outcomes, and receipt boundaries come
from `distill/test-scenarios.md` and `distill/red-classification.md`. No
acceptance body, runtime mechanism, public API, architecture decision, or
normative test contract was created or changed.

## Finding dispositions

| Finding | Disposition | Roadmap correction | Authoritative evidence |
|---|---|---|---|
| RRR-01 | Remediated | Replaced the superseded named-TAP/no-fd-handoff goal and protocol with the accepted R1-R22 replacement contract. Added exact ADR-0127 through ADR-0143 paths to `design_refs`. | `feature-delta.md` *Correctness-Recovery Replacement DESIGN — ACCEPTED 2026-09-24* and *DELIVER re-roadmap input*; `test-scenarios.md` opening contract. |
| RRR-02 | Remediated | Set phase 05 `depends_on` to an empty list. Step 05-00 remains the dependency-free forward root; frozen phase 04 is historical and non-gating. | `feature-delta.md` *DELIVER re-roadmap input*; review RRR-02. |
| RRR-03 | Remediated | Recast 08-01 as a native evidence gate. Only the approved standing R18/R19 branch may proceed. Any non-reproduction or E14(e) outcome stops before GREEN and returns through a separate exact DESIGN record, required user decision, and independent review. DELIVER cannot withdraw fields, errors, effects, ordering, or quiescence. | `feature-delta.md` R18/R19 conditional branch and E14(e); repository native-falsification gate. |
| RRR-04 | Remediated | Expanded 08-04 to capture L, Q, restore and last-TAP-down time, `element_effects` hold time, K, and W at the pinned populations. Expanded 09-01 to set all three bounds, evaluate both five-second fit rules, restate double-loss exposure, apply the pinned R15 threshold branch, keep rustdoc as the sole value record, and commit the method/raw-sample research report outside `docs/feature/**`. | `feature-delta.md` *Runtime shared-network supervisor*, *Kill loop*, and final DISTILL follow-ups; `test-scenarios.md` M-ND295-E18 receipt row. |
| RRR-05 | Remediated | Assigned the three DISTILL-authored E23 cells to 06-02, 06-04, and 09-01 with their exact typed outcomes and explicit Lima-root `pids.max` commands. No scenario ID or body was invented. | `test-scenarios.md` E23 row; `red-classification.md` Phase G Run 7. |
| RRR-06 | Remediated | Added native x86_64 launch-filter evidence to 05-02; native `Vmm::create`, failed-spawn, and per-thread Cloud Hypervisor evidence to 05-03; metal S-ND295-72(e) to 06-02; the feature-gated xtask integration binary to 05-04; and actual CLI, control-plane, worker, host, and dataplane package/test targets to 10-04. | `test-scenarios.md` S-ND295-40/41/43/45/46/72 lane records and scenario-to-test matrix; prior `review-roadmap.md` RMR-06 target inventory. |
| RRR-07 | Remediated | Replaced unresolved marker questions with DISTILL's authored inventory. S-ND295-46 names its source-local and xtask-integration homes. S-ND295-13B names its authored `shared_guest_network_startup` body, file linkage, and exact activation command. | `test-scenarios.md` S-ND295-13B and S-ND295-46 homes/dispositions; `red-classification.md` S-ND295-46 inventory and S-ND295-13B classification. |
| RRR-08 | Remediated | Removed proposed-location and architecture-confirmation language from 08-04 and 10-05. Both steps use the DISTILL-owned non-EDD support harness at `crates/overdrive-control-plane/bin/netns_density_benchmark.rs`; it is not an operator-facing command. | `test-scenarios.md` *Receipts — not tests*, including M-ND295-E18 and T1 receipt homes. |
| RRR-09 | Remediated | Step 10-01 now names the exact in-process driving boundary: `serve::run_with_kek` plus the public `deploy` handler. It records `overdrive deploy <SPEC>` only as stakeholder journey language, forbids a product-binary/subprocess SUT, and uses the accepted `fd=[3]` launch topology. | `test-scenarios.md` binding rules and S-ND295-01/S-ND295-35 driving-port rows; repository Rust-test boundary rule. |

## Preservation and normative-diff audit

- The semantic SHA-256 of phases 01-04 is
  `8fbf6c9e76c006aaf363d1cfd7884dae2835811c3819cd67f115bac7b1da86d1`
  both at `HEAD` and in the remediated worktree.
- `execution-log.json` has no diff and no DES phase event was created.
- `roadmap.validation.status` remains `pending`.
- The forward plan remains 24 steps and 284 estimated hours, with no missing
  step or phase dependency target.
- Removed normative-looking text was limited to the superseded pre-recovery
  named-TAP/no-fd-handoff contract and unresolved documentary questions. The
  replacement text restores the stronger accepted fd-handoff, launch-filter,
  lifecycle, evidence, fail-closed, and native-falsification requirements. No
  `zero`, `never`, `must`, `exactly`, `fail-closed`, `forbidden`, or `required`
  outcome was relaxed.

## Mechanical verification

- `jq` parse: pass.
- All `design_refs`: present.
- Forward count and estimate: 24 steps, 284 hours.
- Missing step dependencies: zero.
- Missing phase dependencies: zero.
- `PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity docs/feature/netns-density-295/deliver/ --roadmap-only`: pass — `Roadmap format OK` with no errors.
- `git diff --check`: pass.

## Handoff

The corrected roadmap is documentary-complete for these nine findings but is
not self-approved. An independent roadmap reviewer must re-review phases 05-10
and top-level metadata before DELIVER may execute 05-00.

## Iteration 2 — RRR-06 command-routing disposition

- Input: independent iteration 2 re-review in
  `deliver/review-roadmap-regenerated.md`; eight findings closed, RRR-06 open.
- Disposition: **remediated, awaiting independent re-review**.
- Step 05-02's forbidden bare `cargo nextest run` command was removed. The
  identical selection remains executable through the existing canonical
  `cargo xtask metal run -- cargo nextest run -p overdrive-host --lib --features integration-tests -E 'test(/launch_seccomp_kernel/)' --no-fail-fast`.
- The associated implementation note now attributes S-ND295-41 and
  S-ND295-43's accepted native x86_64 evidence to that routed command; it no
  longer describes direct execution on an x86_64 CI kernel lane. The selected
  module, native substrate, and acceptance obligations are preserved.
- Only step 05-02's verification list and associated implementation note were
  corrected. Phases 01-04, the execution log, and the independent review
  artifact are preserved. No normative contract was weakened, and
  `roadmap.validation.status` remains `pending`.
- Verification: `PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity docs/feature/netns-density-295/deliver/ --roadmap-only`
  passed with `Roadmap format OK` and no errors; `git diff --check` passed.
  A JSON/hash check confirmed the bare command is absent, the exact metal
  command remains, phases 01-04 match their pre-edit state and `HEAD`, and
  the execution-log and review-artifact bytes match their pre-edit hashes.
