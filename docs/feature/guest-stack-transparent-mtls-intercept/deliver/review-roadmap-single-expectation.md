# DELIVER single-expectation roadmap review

- **Review ID:** `roadmap_single_expectation_rev_20260830_iteration_1`
- **Reviewer:** `nw-solution-architect-reviewer` (fresh isolated reviewer)
- **Reviewed commit:** `8c31246060504ce47ab1c71617b58b6dd681033e`
- **Parent:** `f908e0cf7a07fdf7f90f22cfdbcd6b23edc237ab`
- **Artifact:** `docs/feature/guest-stack-transparent-mtls-intercept/deliver/roadmap.json`
- **Iteration:** 1
- **Final verdict:** **APPROVED**

## Executive summary

The regenerated roadmap is coherent, executable after approval metadata is
advanced, and faithful to the effective DESIGN/DISTILL boundary. It has exactly
one active EDD mapping and expectation directory: E07 drives one checked-in VM
Job caller to one checked-in Exec Service callee through the built
default-feature product and succeeds only on the byte-exact reply. E07 remains
a small public black-box journey; strict D7, netlink, nft, generation,
notification, counter, packet-capture, wire TLS/kTLS, private lifecycle,
failure, cleanup, reclamation, and idempotency claims remain Rust-owned.

The roadmap removes E08/E09 from the active expectation inventory and assigns
their former failure/recovery/stop obligations to Rust-only steps 02-05 and
02-06. Step 02-04 honestly remains partial after one failing RED event and no
GREEN or COMMIT event. The six completed steps have matching approved native
Markdown review artifacts and commit evidence. Remaining work is ordered
02-04, 02-05, 02-06, each through RED-GREEN-COMMIT-review before the next step,
with no per-step mutation testing and one final DELIVER mutation gate.

No blocking, high, medium, or low-severity roadmap finding remains.

## Review basis

- Effective DESIGN decisions in `design/wave-decisions.md` and the approved
  one-expectation handoff in `feature-delta.md`.
- DISTILL's fifteen stakeholder scenarios, Rust supporting contracts, and the
  sole EDD boundary in `distill/test-scenarios.md` and
  `distill/red-classification.md`.
- The checked-in E07 expectation, shared preparation contract, public runner
  contract, INDEX entry, and the complete
  `examples/guest-stack-transparent-mtls-intercept/` bundle.
- `execution-log.json`, completed-step commit ranges, and the full on-disk
  reviews for 01-01 through 02-03.
- The roadmap-only remediation diff from
  `f908e0cf7a07fdf7f90f22cfdbcd6b23edc237ab` to the reviewed commit.

## Required boundary audit

| Requirement | Result | Evidence |
|---|---|---|
| One active EDD expectation | PASS | The executable map has 68 unique rows: 67 Rust-owned and exactly one EDD row, `E07-vm-job-calls-exec-service`, owned by 02-04. The expectation filesystem and INDEX likewise contain only E07 for this feature. |
| One VM Job caller and one Exec Service callee | PASS | The checked-in bundle has one caller `[job]` + `[vm]` spec and one callee `[service]` + `[exec]` spec. The caller's successful exit is causally dependent on the exact byte-distinct callee reply. |
| Built-product black-box E07 | PASS | E07 is constrained to the default-feature `overdrive` binary's public `serve`, `deploy`, `workload describe`, and `job stop` surfaces plus bounded marker-owned fixture cleanup. It does not invoke tests or link/import product crates. |
| No internal D7 claims in E07 | PASS | The expectation contract explicitly excludes netlink framing, normalized nft programs, generation stability, notifications, counters, packet capture, TLS/kTLS, wire confidentiality, and private product cleanup. Those claims are mapped to Rust tests in 02-04. |
| E08/E09 retired | PASS | No active roadmap mapping, INDEX row, expectation gate, or `verification/expectations/E08*`/`E09*` directory remains. Failure/cleanup obligations are Rust-only in 02-05; same-id reclamation, failed reinstall, exact stop, and repeat-stop obligations are Rust-only in 02-06. |
| Honest partial 02-04 state | PASS | The execution log has one 02-04 `RED/EXECUTED` failure caused by the missing anonymous counter and no 02-04 GREEN or COMMIT event. The roadmap labels the worktree partial and uncommitted, forbids treating it as GREEN, and requires the executing crafter to audit it and log only phases it executes. |
| Completed boundary | PASS | 01-01 through 02-02 remain historical-approved; 02-03 is completed-approved at review iteration 9. Every listed review artifact exists, ends APPROVED, and matches the declared step commit evidence. |
| Strict sequence | PASS | 02-04 depends on approved 02-03, 02-05 depends on 02-04, and 02-06 depends on 02-05. The execution policy requires fresh isolated crafters/reviewers and approval before advancement. |
| Scope semantics | PASS | Step and global scope lists are explicitly guidance rather than restrictive allowlists; tightly related production, API, renderer, test, harness, configuration, tooling, manifest, and compiler fallout are allowed when documented. |
| Mutation placement | PASS | Every step forbids mutation testing. The only accepted mutation evidence is one qualified native-metal final gate after reviews 02-03 through 02-06 and E07 evidence are approved. |
| Approval transition | PASS | The reviewed commit correctly remains `validation.status = pending` and `remediated_awaiting_fresh_independent_review`. This on-disk APPROVED review supplies the required independent decision; changing validation metadata is a subsequent roadmap update, not a reviewer edit. |

## Mandatory roadmap checks

### 1. External validity — PASS

E07 exercises the real built default-feature operator binary and public
Service/Job observations on qualified native metal. It does not substitute a
test binary, in-process crate call, or synthetic workload for the product path.
The production-facing outcome is exactly the one user journey retained by the
effective DESIGN/DISTILL handoff.

### 2. Acceptance-criterion implementation coupling — PASS

Each remaining step has five behaviorally anchored criteria. Step 02-04 owns
the strict D7 Rust proof and the separate one-reply E07 outcome; 02-05 owns
fresh/pre-READY failure and cleanup; 02-06 owns same-id reclamation, failed
reinstall, exact stop, and repeat-stop. Criteria do not collapse internal proof
obligations into E07 or split one stakeholder outcome across competing owners.

### 3. Step decomposition ratio — PASS

Three implementation steps remain across sixteen unique production, tooling,
and manifest entries in their declared scopes (`3 / 16 = 0.1875`). Their
outcomes and dependencies are distinct, and the sequence avoids redundant
micro-steps while preserving independent review gates.

### 4. Implementation code in roadmap — PASS

The roadmap states architectural constraints, exact externally meaningful
invariants, ownership, and prohibited substitutions. It does not embed code,
pseudocode, or a tutorial implementation. Detailed mechanics remain in the
approved DESIGN/DISTILL artifacts.

### 5. Concision and precision — PASS

The roadmap contains 2,293 words across JSON string values, below the 3,000
word ceiling. Each remaining description is at most 25 words, each step has
five criteria, the longest criterion is 29 words, and each implementation note
is below 100 words. The text is specific without duplicating the source design.

### 6. Unit/acceptance boundary — PASS

E07 owns only the public end-to-end reply. Rust source/component/integration and
qualified native tests own private lifecycle state, decoder closure, normalized
kernel-program identity, notification/loss detection, exact counters, packet
capture, wire security, failure classification, cleanup complements,
reclamation, and stop idempotency. The executable mappings provide one owner,
test file, function identity, status, and Contract Shape for every atomic row.

## Contract Shape and effect-isolation audit

The roadmap preserves the authoritative shape assignments: pure Rust
properties require the exact `/// CONTRACT_SHAPE: pure-function.` declaration;
bounded-change tests require explicit changed-region and preserved-complement
oracles; S-GTI-03 remains unbounded-preservation over the complete observed
peer-wire universe. E07 is a bounded public outcome and is not allowed to
reimplement any private Rust observer.

Effect ownership is also coherent. Product creation and cleanup remain product
behavior, while E07 preparation removes only marker-owned fixture paths and
uses public stop commands for the two workload identities. Wrong-hook mutation,
native failure fixtures, cleanup restoration, sibling preservation, and
same-allocation reclamation remain isolated in the owning Rust steps.

## Mechanical evidence

| Check | Result | Evidence |
|---|---|---|
| DES schema | PASS | `python3 -m des.cli.roadmap validate .../roadmap.json` reports `VALID: 2 phases, 9 steps`. |
| Roadmap integrity | PASS | `python3 -m des.cli.verify_deliver_integrity --roadmap-only ...` reports no validator errors. |
| Commit scope | PASS | The reviewed commit changes only `deliver/roadmap.json`: 55 insertions and 17 deletions. |
| Diff whitespace | PASS | `git diff --check <parent>..<reviewed-commit>` is clean. |
| Expectation inventory | PASS | One executable expectation mapping and one matching E07 directory; no E08/E09 directories. |
| Executable mapping counts | PASS | 15 stakeholder plus 53 supporting mappings equals 68; 67 Rust and one EDD. No duplicate mapping identity was found. |
| Completion counts | PASS | Six completed/approved steps and three pending steps match the phase entries and evidence. |
| E07 shell syntax | PASS | `bash -n` passes for the expectation runner and checked-in preparation/session/example scripts. |
| Native/mutation execution | NOT RUN | Correctly outside a roadmap review and forbidden as a per-step substitute. |

## Findings and disposition

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |

**APPROVED.** The roadmap accurately represents the approved single-E07
boundary, the completed 01-01 through 02-03 evidence, and the honest partial
02-04 state. After the roadmap's validation metadata is updated to record this
approval, DELIVER may resume at 02-04, including E07 capture and independent
review. It must then proceed through 02-05 and 02-06 in the declared
RED-GREEN-COMMIT-review sequence before the single final mutation gate can close
the wave.
