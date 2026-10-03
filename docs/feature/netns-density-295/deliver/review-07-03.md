# DELIVER Review — Step 07-03

## Metadata

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Step | `07-03` — placement read-port, held occupancy, restart gating, and reclaim emission |
| Reviewer | `nw-software-crafter-reviewer` |
| Model | GPT-6 Luna, maximum reasoning |
| Iteration | 1 |
| Commits reviewed | `f8a66fa15f8aeec27dd230371e7dae567b8a22c6`, `8b76a73ffde9caa6ee547d6a7e811953fa49b93f` |
| Verdict | **APPROVED** |

The review applied `nw-sc-review-dimensions`, `nw-tdd-review-enforcement`, and `nw-tdd-methodology`.

## Scope and contract sources

Reviewed the two step commits against approved roadmap step 07-03 (`deliver/roadmap.json:1152-1204`), accepted D-295-R8 / ADR-0134 (`feature-delta.md:2883-3180`), accepted D-295-R11 (`feature-delta.md:4907-5129`), and the DISTILL outcomes S-ND295-05B, 05C, 05D, 55, and 57 (`distill/test-scenarios.md:753-1085`). The review covered the read-port production composition, placement and restart behavior, reclaim ownership on every reconciliation return path, retry inputs and wakeups, public and persisted shape, the exact authored test stimuli and oracles, timing budgets, DES evidence, and commit/worktree mechanics.

`crates/overdrive-core/src/scheduler.rs` was omitted from the roadmap's `files_to_modify` list, but changing its already-threaded occupancy input was necessary to meet the explicit criterion that `schedule` use held occupancy. The file list is implementation guidance; this small change is directly required by the criterion and adds no API.

## Contract Shape Compliance

The implementation matches the accepted interface and persisted-state contract. It adds no public method, type, trait member, enum variant, parameter, wire value, owner, lifecycle state, or persisted field. `schedule` retains its existing signature, including the already-threaded `GuestAttachmentOccupancy`; the change consumes `held` instead of counting workload-local Running rows (`scheduler.rs:85-116`). `ReclaimAllocationNetwork` and the two `WorkloadLifecycleView` retry fields were already introduced by 07-02; 07-03 only emits the existing action and updates those existing inputs (`feature-delta.md:4950-5000`). The added helpers are private.

Actual-workload hydration collects that workload's allocation IDs and calls `GuestAttachmentView::observe` once (`workload_lifecycle.rs:672-704`). In production, `AppState` builds `GuestPoolAttachmentView` from the same `Arc<GuestAddressPool>` used by dispatch (`control-plane/src/lib.rs:390-394, 741-747`); runtime hydration lends that view through `HydrationContext` (`reconciler_runtime.rs:1694-1712`). The pool reads node-wide held/retiring occupancy and requested leases under one lock acquisition (`guest_network.rs:697-708`). This preserves the accepted distinction between node-wide occupancy and workload-scoped lease lookup.

The scheduler refuses placement when node-wide `held >= MAX_GUEST_NETWORK_ATTACHMENTS`, including Retiring leases; CPU and memory checks remain in place. The restart branch checks held occupancy before reserving a successor allocation ID. Below the cap, a due restart retains the existing successor-first path. At the cap, it emits no restart; the outer reclaim pass emits reclaim only when the predecessor is leased. A due predecessor without a lease waits, and a not-yet-due predecessor remains owned by its pending restart at every occupancy (`workload_lifecycle.rs:1191-1209, 397-477`). These cases match the R8 restart table and R11 cleanup ownership table.

`append_reclaim_actions` runs after `reconcile_inner` returns and therefore covers its Stop, deleted/GC, terminal-fence, Running, Draining, operator-stop, natural-exit, restart, finalize, and placement outcomes. It selects only Failed/Terminated rows with a held lease and no competing Start, Restart, Stop, or FinalizeFailed owner; BTreeMap traversal preserves allocation-ID order (`workload_lifecycle.rs:225-226, 383-477`). Retry state is the accepted persisted input pair: emission count and last-emitted instant. Emissions are counted, timestamps are recorded at emission, entries are pruned when the lease is absent, and there is no attempt ceiling (`:480-529`). The cadence remains the accepted constant one second (`feature-delta.md:4993-5000`). `next_evaluation_at` computes the earliest restart or reclaim deadline without persisting a derived deadline (`workload_lifecycle.rs:312-379`). The runtime persists the updated View before dispatch and self-re-enqueues after an action even when dispatch returns a recoverable error, allowing persisted retry memory to drive the next evaluation (`reconciler_runtime.rs:1537-1552, 1600-1641, 1660-1677`).

## Acceptance integrity, boundary, and Contract Shape

The accepted test bodies and assertions were authored upstream by DISTILL. In the step diff, their test changes are activation-only removals of pending `#[ignore]` attributes; no test body, expected value, assertion, seed, cap, iteration count, or acceptance scenario was weakened, replaced, or re-authored. The 07-03 step activates 10 pending bodies; 11 scoped bodies are active across 05B, 05C, 05D, the three non-validator 55 bodies, and 57 when the already-active S05C not-yet-due body is included. The S-ND295-55 validator body remains outside this step as specified; `action_shim/validate.rs` is unchanged.

Every activated body has its accepted Contract Shape declaration. The source-local pure-function properties use the exact `/// CONTRACT_SHAPE: pure-function.` line, including the restart and reclaim properties (`workload_lifecycle.rs:3184-3187, 3241-3245, 3282-3285, 3604-3607, 3686-3689, 3751-3753`). The pool observation property is correctly declared `bounded-change` because it asserts the read-only complement over pool state (`guest_network.rs:11704-11710`). The seeded integration bodies are `bounded-change` (`netns_density_node_admission.rs:1298-1303`; `netns_density_reclaim.rs:1188-1208`).

S-ND295-05D remains in `overdrive-sim/tests/integration` and drives the registered production reconciler/runtime composition through `SimNode::compose`, `SimDriver`, and `SimSharedGuestNetworkOwner` (`netns_density_node_admission.rs:840-935, 1318-1321`). The fixture's ledger delegates owner effects to the sim owner; the body uses no host network socket or kernel state. Its two default seeds each fill a fresh simulated node at the fixed cap. This respects the accepted socket-free Sim composition and the user's requirement that the body stay in the Sim integration lane.

S-ND295-57's at-cap body measured 177.361 seconds across its three default seeds, exceeding the default profile's 120-second timeout, so it correctly remains in the Sim integration binary with its existing targeted `10 × 60s` override. S-ND295-05D's exact override remains scoped to `package(overdrive-sim) & binary(integration) & test(node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads)` with the existing `25 × 60s` budget (`.config/nextest.toml:207-232`). Its two-seed default run completed in 650.222 seconds, below the 1,500-second budget. The nextest diff changes timing/evidence comments only; it does not broaden a timeout selector, lower the cap, remove seeds, or weaken the oracle. The DISTILL documentation updates report the semantic RED and measured GREEN evidence without changing normative outcomes (`red-classification.md:879, 1046-1053`; `test-scenarios.md:1050-1068`).

For the test-budget check, the minimum behavior count is 19: the 15 distinct `ReturnPath` arms in S-ND295-55 (`workload_lifecycle.rs:3431-3474`), plus one behavior family for each of the other four scenario IDs. That gives a minimum `2 × behaviors` budget of 38; 11 scoped test bodies are active. This conservative count does not split the additional oracle clauses within 05B, 05C, 05D, or 57. The bodies exercise the externally composed runtime and pure reconciler outcomes, rather than testing private helpers as the only evidence.

## RED, verification, and DES evidence

The RED classification records semantic failures for held-cap placement, due restart and raced-refusal behavior, the 05D contended-slot / placement-view oracle after its 06-03 fixture precondition was satisfied, every-path reclaim, retry cadence, and S-ND295-57 release / at-cap ordering (`red-classification.md:300-304, 879, 962, 965-966`). In particular, the latest S-ND295-05D RED evidence is the second admission refusal and an admission refusal inside an at-cap placement window after the 16,384 fill; the earlier lease-release precondition failure is separately recorded as a preceding-step gap. The RED gate's PASS means its semantic-failure gate passed, not that the feature tests were green.

`execution-log.json:986-1004` records exactly RED → GREEN → COMMIT for 07-03, each `EXECUTED` with `PASS`. The required results were reported as passing:

| Gate | Result | Command / evidence |
|---|---|---|
| Workload lifecycle selector | PASS | `cargo xtask lima run -- cargo nextest run -p overdrive-reconcilers --lib -E 'test(workload_lifecycle)' --no-fail-fast` |
| Placement-cap selector | PASS | `cargo xtask lima run -- cargo nextest run -p overdrive-core --test acceptance -E 'test(netns_density_placement_cap)' --no-fail-fast` |
| Reclaim Sim integration selector | PASS | `cargo xtask lima run -- cargo nextest run -p overdrive-sim --test integration --features integration-tests -E 'test(netns_density_reclaim)' --no-fail-fast` |
| Seeded 05D selector | PASS | `cargo xtask lima run -- env OVERDRIVE_ND295_ADMISSION_SEED=186055177052160001 cargo nextest run -p overdrive-sim --test integration --features integration-tests -E 'test(node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads)' --no-fail-fast` |
| 05D default-seed run | PASS | Both default seeds completed in 650.222 s; recorded in `red-classification.md` and `.config/nextest.toml` |
| Workspace check | PASS | `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` |
| Workspace clippy | PASS | `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` |
| Formatting and diff check | PASS, reported | `rustfmt` and `git diff --check` |

These are the step/orchestrator-reported command results; this reviewer did not rerun the suite. Mutation testing was not run during the individual roadmap step, as required.

## Commit and worktree evidence

Both commits retain Marcus Schack Abildskov as Git author, include exactly `Co-Authored-By: Codex <codex@openai.com>`, and carry `Step-Id: 07-03`. The reviewed commit bodies have no Claude, Anthropic, or generated-by attribution. Before this review artifact was written, the only dirty user-owned path was `AGENTS.md`; it was preserved. No production, test, design, roadmap, or execution-log file was changed by the reviewer.

## Findings and remediation dispositions

No proven in-scope defect, API or persisted-shape divergence, acceptance weakening, Contract Shape omission, production-composition gap, retry/wakeup mismatch, or evidence-gate failure was found. No remediation was required or dispatched.

## Iteration 1 verdict

**APPROVED.** Step 07-03 implements the exact R8 and R11 part-2 contract with the existing read-port, scheduler input, action, and retry fields. The production read-port observes the same per-server pool used for admission; the restart and reclaim ownership rules match the accepted tables; and reclaim emission is appended across every reconciliation return path with the accepted persisted-input cadence. The DISTILL-authored outcomes remain intact, S-ND295-05D stays in the socket-free Sim integration composition, the measured runtime fits its exact budget, and the required reported gates pass.
