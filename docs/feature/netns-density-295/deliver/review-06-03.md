# DELIVER Review — Step 06-03

## Metadata

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Step | `06-03` — One admission pool per server with lease states and refusal projection |
| Reviewer | `nw-software-crafter-reviewer` |
| Iteration | 1 |
| Commits reviewed | `b696e86107c58d0c27c60ce258997d45c0e48317`, `2b22d8e07ea7401b04ba12a64a398caa2eece38a` |
| Verdict history | Iteration 1: **APPROVED** |

The reviewer applied `nw-sc-review-dimensions`, `nw-tdd-review-enforcement`, and `nw-tdd-methodology`.

## Scope and contract sources

Reviewed the step against `deliver/roadmap.json` step 06-03, feature-delta D-295-R6/R7 and the accepted node-wide guest-attachment admission component, ADR-0132, ADR-0133, and DISTILL scenarios S-ND295-04, 05A, 05E, and 06. The review focused on the per-server pool, lease state and retirement semantics, refusal behavior and events, accepted API shape, nine required test activations, and the step's workspace gates.

## Contract Shape Compliance

The dispatch seams retain their existing signatures. Production dispatch and the test-gated seam obtain the provisioner and pool from `AppState`; dispatch reads the pool from `state.guest_pool`. The private `GuestNetworkProvisionerAndPool` alias carries the pair through internal functions and does not add a public or cross-crate interface.

The production composition creates one `Arc<GuestAddressPool>` in `run_server` and injects it into `AppState`. `GuestPoolAttachmentView` wraps a clone of that same `Arc`, and `build_hydration_context` reads the view from `state`. The process-global action pool and its free-function accessors are removed. The pool constructor and type visibility follow the already accepted doc-hidden `AppState` constructor contract.

The pool's behavior matches the accepted R6/R7 shape:

- `assign` returns a byte-equal plan for an existing Admitted lease, returns `LeaseRetiring` for an existing Retiring lease, then checks held occupancy against the core cap before selecting an address. A refusal leaves the pool unchanged.
- `retire` changes only Admitted to Retiring, reports that transition once, and keeps the lease counted. `release` removes either lease state and frees the address only after the provisioner's teardown succeeds.
- `observe` reads occupancy and requested allocation leases under one lock acquisition. The retiring count is maintained with the lease map, so the cap property can exercise the full 16,384-entry state without reducing its population or property cases.
- `AdmissionCapReached` and `LeaseRetiring` are returned through the existing `ShimError::GuestNetwork` shape. Admission refusals emit `guest_network.admission_refused` with allocation, held, retiring, and cap fields. Refused starts do not write an allocation row or emit a lifecycle occurrence; a refused restart still runs the single predecessor cleanup attempt.
- `MAX_GUEST_NETWORK_ATTACHMENTS` and `GuestAttachmentOccupancy` are core-owned. The production pool and scheduler import the same constant; no duplicated production cap literal was found.

The source diff adds no public method, type, enum variant, trait member, or parameter outside the accepted R6/R7 contract. The pool view and dispatch pair are crate-private. `AppState`'s pool constructor wiring is the step's already-approved composition boundary.

## Retirement and refusal path review

The changed cleanup paths retire before their first guest-network teardown and release only after teardown returns successfully. The dispatch helpers record `guest_network.lease_retired` only on the Admitted-to-Retiring transition and `guest_network.lease_released` after release. The inspected call sites cover failed provision and identity setup, rejected driver start, rejected initial Running writes, failed mTLS install and activation, restart successor cleanup, restart predecessor cleanup, genuine-terminal finalization, and stop. The Running-write start and restart bodies assert the driver stop, retirement, teardown, and release ordering.

`Action::ReclaimAllocationNetwork` remains the separately sequenced 07-02 scaffold; this review did not treat its not-yet-implemented path as a live 06-03 cleanup path.

## Test integrity, coverage, and budget

All nine assigned bodies are active: S-ND295-04 (two pool properties), S-ND295-05A (the cap property and killed-mode restart integration), S-ND295-05E (start refusal and refused restart projection), and S-ND295-06 (provision refusal plus fresh-start and restart Running-write rejection). Each transitioned body has its `CONTRACT_SHAPE` declaration and the project outcome anchor. The active pool properties are stateful bounded-change properties; no source-local pure-function property was transitioned by this step.

The commit diff removes the pending `#[ignore]` markers. It does not alter the bodies' assertions, expected values, or refusal stimuli. The `mtls_install_fail_closed` fixture additions start the shared owner, supply the predecessor's held guest assignment, and shut the owner down; those are fixture preconditions and cleanup, not changed acceptance outcomes. The production owner and dispatch path remain under test. No expectation runner launches the Overdrive binary, and no Rust test acts as an expectation runner.

Counting the nine observable outcomes in the assigned scenarios gives a budget of 18 tests under the review role's `2 × behaviors` rule. Nine assigned bodies are active, within budget; integration bodies remain separate wiring evidence. The pool properties compare generated operations against a model and verify the lease map, occupancy, plan replay, refusal, and address reuse. The refusal and Running-write bodies assert observable dispatch, row, lifecycle, owner, driver, and event outcomes.

| Gate | Result | Evidence |
|---|---|---|
| G1 — activation discipline | PASS | Only the nine S-ND295-04/05A/05E/06 bodies assigned to 06-03 were activated. |
| G2 — valid RED | PASS, recorded | The 06-03 DES log retains RED PASS before GREEN; the later GREEN FAIL is followed by GREEN PASS entries. |
| G3 — assertion failure | PASS | The transitioned bodies assert typed refusal, pool state, event payloads, or lease/cleanup ordering. |
| G4 — no internal domain mocks | PASS | Pool properties use the pool driving surface; dispatch tests use guest-network ports at the seam. |
| G5 — business language | PASS | Activated bodies name their accepted S-ND295 scenario and outcome anchor. |
| G6 — all green | PASS, reported | Targeted integration, acceptance, and library selections passed. |
| G7 — green before commit | PASS, recorded | The log records GREEN PASS before COMMIT PASS. |
| G8 — test budget | PASS | 9 behaviors; budget 18; 9 assigned bodies. |
| G9 — no test weakening | PASS | Diff inspection found activation and fixture-support changes, with assertions and expected outcomes preserved. |
| External validity | PASS | Tests exercise the accepted dispatch seam and server composition; the integration tests run in-process. |

**Test integrity:** no weakened or deleted assertions, no newly skipped tests, no testing theater, and no test harness/product-binary boundary violation were found. Mutation testing was not run during this roadmap step.

**RPP scan:** L1 and L2 reviewed within the changed surface; no actionable step-scoped smell was found. The edits thread the pool through existing private dispatch flow and add the required lease transitions without introducing another owner or general mechanism.

## DES and commit evidence

The `execution-log.json` sequence for 06-03 is RED PASS, GREEN FAIL, GREEN PASS, GREEN PASS, COMMIT PASS. It preserves the initial failure and later passing GREEN records; no COMMIT precedes a passing GREEN. The reviewed feature commit retains Marcus as author, includes exactly `Co-Authored-By: Codex <codex@openai.com>`, carries `Step-Id: 06-03`, and contains no Claude, Anthropic, or generated-by attribution. The separate log commit also carries the required trailer and step identifier.

## Verification

The following results were reported for the step and were not rerun during this review:

| Command | Reported result |
|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test integration --features integration-tests -E 'test(guest_attachment_pool_per_server) or test(mtls_install_fail_closed)' --no-fail-fast` | 13 passed |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test acceptance -E 'test(netns_density_guest_network)' --no-fail-fast` | 2 passed |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --lib -E 'test(guest_network)' --no-fail-fast` | 27 passed in 23.595 s; the full-cap property stayed within the default lane |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | Passed |
| `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | Passed |
| Changed-file formatting and `git diff --check` | Passed |

## Findings and remediation dispositions

No proven blocker or required remediation was found. No failure hypothesis is promoted to a finding; no new regression was needed because the review identified no reachable defect.

## Iteration 1 verdict

**APPROVED.** The implementation satisfies the 06-03 per-server admission-pool and R6/R7 lease/refusal contracts, preserves the accepted API and seam shapes, activates all nine assigned tests without changing their assertions, and has reported clean required workspace check and clippy gates. The review found no proven in-scope defect.
