# DELIVER Review — Step 07-04

## Metadata

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Step | `07-04` — cleanup-pending predicate, server projection, renderer, OpenAPI |
| Reviewer | `nw-software-crafter-reviewer` |
| Model | GPT-6 Luna, maximum reasoning |
| Iteration | 1 |
| Commits reviewed | `2ae44fec5a8ba2c139cab13ee8b7a27a3ada1ac1`, `adab0f3ca7874eb539e71c527c39154898f96cae` |
| Verdict | **APPROVED** |

The review applied `nw-sc-review-dimensions`, `nw-tdd-review-enforcement`, and `nw-tdd-methodology`.

## Scope and contract sources

Reviewed the commits against approved roadmap step 07-04 (`deliver/roadmap.json:1194-1231`), accepted D-295-R20 (`feature-delta.md:5992-6120`), and DISTILL scenarios S-ND295-58, 59, and 60 (`distill/test-scenarios.md:1063-1128`). The review covered the exact lease/row predicate, live server projection, replica count, public JSON/OpenAPI shape, live CLI rendering path, scenario boundary and assertions, and step evidence.

The roadmap's file list names earlier aliases (`guest_network.rs`, `openapi.rs`, and `render/alloc_status.rs`). The commits instead touch the actual production homes `guest_attachment_view.rs`, `handlers.rs`, and `render.rs`. Those are directly required by the acceptance criteria; this is a necessary path correction, not a contract or scope expansion. No unrelated files were added. OpenAPI generation and checking were reported successful; the generated document had no diff because the accepted API field was already present in the scaffold and schema.

## Contract Shape Compliance

The public core method matches the accepted signature exactly: `GuestAttachmentLease::cleanup_pending(self, row_state: AllocState) -> bool` (`crates/overdrive-core/src/traits/guest_attachment_view.rs:35-45`; FD `:6005-6014`). Its decision is exact: every `Retiring` lease is pending; an `Admitted` lease is pending only for a terminal row. `AllocState::is_terminal` is exactly `Failed | Terminated` (`observation_store.rs:219-232`). The no-lease case is represented by absence in the observed lease map and projects to false, as specified.

The production `alloc_status` handler first filters rows to the requested workload, derives their allocation IDs, and calls `state.guest_pool.observe(&alloc_ids)` once (`handlers.rs:1187-1195`). The pool returns the requested leases under one lock acquisition (`guest_network.rs:697-708`); `AppState.guest_pool` is the node-wide pool used for dispatch (`control-plane/src/lib.rs:388-393, 741-747`). The handler derives each wire flag from that live lease and row and excludes pending rows from `replicas_running` (`handlers.rs:1229-1248`). It writes no pending flag, timestamp, cache, or ledger to durable state. The API field remains the accepted additive `network_cleanup_pending: bool` with `#[serde(default)]` (`api.rs:427-430`); there is no change to persisted row or rkyv shape.

The renderer uses private helpers only. Both the Service row and Job attempt state cells become `CleanupPending` when flagged; the presence-guarded detail line preserves the lifecycle state (`render.rs:531-544, 698-703, 1075-1084, 1149-1157`). The Job verdict still derives from the row state. No public method, type, trait member, enum variant, parameter, wire value, lifecycle state, or persisted field is invented.

The tests and production path share the actual operator renderer. The CLI `workload describe` command calls `commands::workload::describe` and prints `render::workload_describe` (`main.rs:170-174`). That command obtains the full response through `ApiClient::alloc_status_for_workload`, which issues `GET /v1/allocs?job=...` and carries the returned snapshot into `WorkloadDescribeOutput` (`commands/workload.rs:184-207`; `http_client.rs:333-343`). The test-only `describe_snapshot` helper is not the product command path. The production router registers `GET /v1/allocs` to `handlers::alloc_status` (`control-plane/src/lib.rs:7943-7951`); the same handler is listed in the OpenAPI path set (`control-plane/src/api.rs:518`).

## Acceptance integrity, boundary, and Contract Shape

S-ND295-58 exhaustively exercises `{Admitted, Retiring} × every AllocState` through the production predicate; its declaration is `pure-function` (`netns_density_cleanup_pending.rs:53-78`). S-ND295-59 drives the production server composition and reads through HTTPS `GET /v1/allocs` (`network_cleanup_pending_status.rs:1-10, 385-394`). It covers a healthy row, a failed stop that stays `Running` but is not counted as a running replica, successful stop retry, a crashed `Failed` row with an admitted lease, and a reclaiming `Failed` row with a retiring lease through pinned lease events. It checks release after reclaim and allocation identity on captured lease events (`:482-600` and the remainder of the scenario). These are in-process production-composition tests, not a binary runner or expectation harness.

S-ND295-60 calls the one live `workload_describe` renderer. Its test covers Service and Job output across lifecycle states, asserts the `CleanupPending` cell and lifecycle detail line, checks that the only output delta is the required row status/detail, preserves Job verdicts, and retains byte-identical output for non-pending rows (`render_workload_describe.rs:1331-1423` and its following golden case). This is the pure renderer boundary specified by DISTILL. E20 intentionally has no black-box verification expectation; the accepted evidence lane is covered by S-ND295-59 and S-ND295-60.

The four bodies newly activated in this step carry the required outcome anchor and Contract Shape declaration: S-ND295-58 and the pending S-ND295-60 renderer body are `pure-function`; both S-ND295-59 bodies are `bounded-change`. The non-pending S-ND295-60 golden baseline remains active with its existing `pure-function` declaration. The commit diff only removes pending `#[ignore]` attributes; it does not edit fixtures, assertions, expected values, names, or setup. No assertion was weakened, deleted, replaced, or skipped. The tests cross the specified production boundaries and do not implement the behavior in fixtures.

For the budget check, the three accepted scenarios express three behavior groups, giving a `2 × 3 = 6` maximum; five in-scope test bodies are active. S-ND295-59's two in-process integration bodies are included conservatively in that count. No separate unit-property test was required beyond the exhaustive S-ND295-58 acceptance property.

## RED, verification, and DES evidence

`execution-log.json:1007-1025` records RED → GREEN → COMMIT in order, each `EXECUTED` with `PASS`. The DISTILL classification identifies the RED cases as missing functionality at the predicate scaffold, the live API projection, and the live renderer (`red-classification.md:440-443, 967-968`). The scaffold failure for S-ND295-58 reaches the exact unimplemented D-295-R20 predicate. No test body was changed to obtain GREEN.

The following step results were reported by the orchestrator; this reviewer did not rerun the suites:

| Gate | Result | Command / evidence |
|---|---|---|
| Core predicate acceptance | PASS | `cargo xtask lima run -- cargo nextest run -p overdrive-core --test acceptance -E 'test(netns_density_cleanup_pending)' --no-fail-fast` |
| Control-plane projection integration | PASS | `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test integration --features integration-tests -E 'test(network_cleanup_pending_status)' --no-fail-fast` |
| CLI renderer acceptance | PASS | `cargo xtask lima run -- cargo nextest run -p overdrive-cli --test acceptance -E 'test(render_workload_describe)' --no-fail-fast` |
| OpenAPI generation and check | PASS | `cargo xtask lima run -- cargo run -p overdrive-control-plane --bin openapi -- generate`; `cargo xtask lima run -- cargo run -p overdrive-control-plane --bin openapi -- check`; generated artifact unchanged |
| Workspace check | PASS | `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` |
| Workspace clippy | PASS | `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` |
| Formatting and diff check | PASS, reported | `rustfmt` and `git diff --check` |

Mutation testing was not run in this individual step, as required for the DELIVER wave.

## Commit and worktree evidence

Both commits preserve Marcus Schack Abildskov as author, carry exactly `Co-Authored-By: Codex <codex@openai.com>`, and include `Step-Id: 07-04`; neither contains Claude, Anthropic, or generated-by attribution. The production/test commit is limited to the six directly relevant implementation and acceptance files plus the DES log; the metadata commit updates only the DES log. The pre-existing dirty `AGENTS.md` was preserved. The reviewer changed only this review artifact and did not alter source, tests, design, roadmap, or execution events.

## Findings and remediation dispositions

No proven in-scope defect, accepted API or persisted-shape divergence, missing live-path wiring, incorrect lease predicate, replica-count error, acceptance weakening, Contract Shape omission, testing-theater issue, or evidence-gate failure was found. No remediation was required or dispatched.

## Iteration 1 verdict

**APPROVED.** Step 07-04 implements the accepted D-295-R20 predicate and derives operator status from the live lease and row state at read time. The filtered handler uses one pool observation, excludes cleanup-pending rows from the running replica count, and the actual `workload describe` path renders the status while retaining lifecycle detail and Job verdict behavior. The DISTILL-authored scenarios remain intact and exercise the required production boundaries; the recorded gates pass.
