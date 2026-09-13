# DELIVER Review — VM recreation allocation identity, step 01-02

## Metadata

| Field | Value |
|---|---|
| Feature | `vm-recreation-allocation-id-reuse` |
| Step | `01-02` — Prove production-owner restart isolation |
| Commit | `e7fd61c74e227be7ea0e9e12eff18ce9b1b40b48` |
| Reviewer | Fresh isolated `nw-software-crafter-reviewer` |
| Review date | 2026-09-13 |
| Iterations | 1 |
| Final verdict | **APPROVED** |

## Scope and authority

This review covers commit `e7fd61c74e227be7ea0e9e12eff18ce9b1b40b48` and the
complete changed test file:

`crates/overdrive-sim/tests/e10_vm_early_exit_spike.rs`

The authority set is:

- accepted `docs/product/architecture/adr-0104-vm-recreation-fresh-allocation-identity.md`;
- accepted feature artifact
  `docs/feature/vm-recreation-allocation-id-reuse/feature-delta.md`;
- approved roadmap step `01-02` in
  `docs/feature/vm-recreation-allocation-id-reuse/deliver/roadmap.json`;
- `AGENTS.md`, `CLAUDE.md`, and the required Rust, testing, debugging, design,
  BPF, development, and verification rules.

The step contract is evidence-only: no production mechanism or public surface
is authorized. The intended activation is S-284-SIM-01, with the existing
no-restart control retained. The review preserves the pre-existing dirty
`AGENTS.md`, the untracked DES execution log, and the previously written
`review-01-01.md`; no production or test source was edited by this reviewer.

## Source and design checklist

| Contract | Result | Evidence |
|---|---|---|
| Registered production-owner composition | PASS | The test registers `WorkloadLifecycle`, `ServiceLifecycle`, and `VmReclamation` at `e10_vm_early_exit_spike.rs:359-362`, builds `AppState` with the real `VmDriver` and existing ports at `:346-381`, and starts the existing exit observer at `:382-387`. |
| No fabricated observation row or private lifecycle state | PASS | The workload intent is archived and written through the existing `IntentStore` at `:414-416`; all allocation rows are produced by action-shim dispatch and read back through `ObservationStore`. The only fault is the existing `SimObservationStore::inject_write_failure` at `:535-537`. No row, runtime View, `VmDriver` live map, or private claim is seeded. `RecordedVmm::create` records Sim host facts only after the production `Vmm::create` returns at `:129-140`. |
| Rejected fresh Running publication | PASS | `:528-596` drives the real fresh start, waits for the fresh beacon, injects one observation write failure, and asserts typed failure, no rejected row, released supervision, awaited host cleanup, and the two exact C3/TAP provision/teardown pairs. |
| Reservation durability across runtime restart | PASS | `:598-623` drops the live redb-backed `ReconcilerRuntime`, reopens the same ViewStore directory, re-registers all three owners, and verifies the unpublished allocation key was restored by WorkloadLifecycle bulk-load while the predecessor row remains unchanged. |
| Higher next fresh identity | PASS | `:625-644` advances the injected clock past the existing candidate backoff, re-drives the same WorkloadLifecycle owner, waits on the expected fresh beacon, and then asserts the old, rejected, and accepted VMM controls at `:646-658`; the accepted row is read at `:660-664`. |
| Current row and predecessor/history separation | PASS | The accepted fresh row is `Running` with `restart_count == 0` and no `last_terminated` at `:660-664`; the predecessor row and its full occurrence vector remain equal at `:665-673`, and the rejected reservation has no observation row at `:617-622`. |
| Delayed predecessor disposal and stale authorship | PASS | The registered reclamation owner is driven at `:678-692`. Assertions at `:696-724` prove old VMM/artifacts are removed, replacement artifacts and process survive, replacement observation is unchanged, and the replacement C3/TAP plan is not torn down. |
| Repeated reclamation is a no-op | PASS | A second registered reclamation evaluation at `:736-750` leaves both the artifact-discard and host-scope-kill histories byte-for-byte unchanged after the predecessor host surfaces are absent. |
| Final operator stop and cleanup complement | PASS | The existing stop-intent key is submitted at `:752-770`; the final registered reclamation observation follows at `:771-781`. `:782-797` asserts absence for all three execution artifact sets, an empty VM supervision set, exactly three provisions and teardowns, and pairwise-identical C3/TAP plans. |
| In-process boundary | PASS | The test uses the existing runtime/action-shim/VmDriver/Sim adapters and real temporary Unix beacon sockets. It does not spawn the built Overdrive binary, invoke a verification expectation, import an expectation runner, or emit expectation evidence. The file is gated by `integration-tests` at `:12`. |
| Exact API and scope | PASS | The commit changes one test file only; it adds no production mechanism, action, type, field, trait, parameter, dependency, retry, sleep, lock, compatibility path, or persistence surface. |
| Contract Shape | PASS | Both live test entry points retain the exact `/// CONTRACT_SHAPE: bounded-change.` declaration at `:832` and `:842`; their bounded universes remain present and unchanged. |

The production path invoked by the test is the existing one: registered
reconciler hydration and pure decision, runtime View persistence before
dispatch, action-shim C3 provisioning, `VmDriver::start` and its
allocation-keyed VMM/cgroup/run-directory/clone state, awaited write-rejection
unwind, `VmReclamation` observation and exact-key disposal, and the existing
operator-stop path. The Sim host coupling is at driven-port boundaries; it
does not replace any production lifecycle decision or manufacture an
observation row.

## Test-fixture correction equivalence audit

The parent of this commit is the post-01-01 state in which the complete
S-284-SIM-01 body was already authored and still carried only its
`pending DELIVER step 01-02` ignore. The authoring commit was
`ca09f160` (`test(distill): specify fresh VM allocation identity`). The
commit under review changes exactly the following:

1. It snapshots `network.provisions` and `network.teardowns` into immutable
   local vectors before the initial assertions (`:512-515`). The original
   expressions took two locks on the same mutex within one assertion macro
   expression, allowing Rust 1.95's temporary guard lifetime to deadlock.
2. It applies the same snapshot-only correction before the rejected-start
   assertions (`:584-590`).
3. It removes only `#[ignore = "pending DELIVER step 01-02"]` from
   `vm_recreation_reserves_each_execution_and_old_cleanup_cannot_cross_ids`
   (`:837-840`).

The before/after oracle audit is exact:

| Assertion | Pre-correction oracle | Post-correction oracle | Result |
|---|---|---|---|
| Initial C3/TAP count | `network.provisions.lock().len() == 1` | `initial_network_provisions.len() == 1` | Equivalent |
| Initial teardown content/order | `network.teardowns` equals the first provision plan | `initial_network_teardowns` equals the first provision plan | Equivalent |
| Rejected C3/TAP count | `network.provisions.lock().len() == 2` | `rejected_network_provisions.len() == 2` | Equivalent |
| Rejected teardown content/order | `network.teardowns` equals provision plans 0 and 1 | `rejected_network_teardowns` equals provision plans 0 and 1 | Equivalent |
| Seed, IDs, timing, rerun text, assertions, and bounded universe | unchanged | unchanged | Preserved |

The correction changes only lock lifetime mechanics. It does not alter any
expected count, teardown value or ordering, seed (`257205`), printed rerun
command, production call, fault, assertion, or Contract Shape universe. It
therefore satisfies the explicitly authorized acceptance-fixture correction
exception and does not accommodate a production behavior change.

## DES, commit, and mechanical evidence

| Check | Result | Evidence |
|---|---|---|
| Roadmap readiness | PASS | `roadmap.json` has `validation.status = approved`, with the independent architect review approving the exact 21/21 scenario mapping. |
| DES phase order for 01-02 | PASS | `deliver/execution-log.json:27-52` records `RED EXECUTED FAIL`, initial `GREEN EXECUTED FAIL`, resumed `GREEN EXECUTED PASS`, then `COMMIT EXECUTED PASS`, in timestamp order. |
| Commit scope | PASS | `git show --stat` reports one changed file: `crates/overdrive-sim/tests/e10_vm_early_exit_spike.rs`. |
| Attribution | PASS | Commit author and committer remain Marcus Schack Abildskov; the message has exactly one `Co-Authored-By: Codex <codex@openai.com>` trailer and `Step-Id: 01-02`, with no Claude/Anthropic attribution. |
| Diff hygiene | PASS | `git diff --check e7fd61c^ e7fd61c` is clean. |
| Mutation testing | PASS — not run | Individual roadmap-step mutation testing is expressly prohibited; the final DELIVER-wave gate remains pending until all steps are reviewed. |

The DES `GREEN` retry is consistent with the recorded fixture-only lock
deadlock: the first implementation attempt did not pass, the bounded
snapshot correction resumed GREEN, and no production source was changed in
this step.

## Verification evidence

The review independently reran the relevant evidence through the required
Lima runner:

| Command | Result |
|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test e10_vm_early_exit_spike -E 'test(vm_recreation_reserves_each_execution_and_old_cleanup_cannot_cross_ids) or test(no_intervening_restart_reclamation_preserves_authored_ending_control)' --no-capture --no-fail-fast` | PASS — 2/2 |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --features integration-tests -E 'test(reconciler_runtime_view_store) or test(vm_reclamation_claim_lifecycle)' --no-fail-fast` | PASS — 13/13 |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | PASS |
| `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | PASS |
| `cargo xtask lima run -- cargo fmt --all -- --check` | PASS |

No verification expectation was created or touched. No black-box Overdrive
binary was started by the Rust test, and no mutation run was performed.

## Review iteration 1

### Adversarial assessment

The test asserts observable state and driven-port effects rather than private
production implementation reachability. It enters through the registered
reconciler runtime and action shim, uses a real `LocalIntentStore` and
redb-backed `ViewStore`, and obtains every allocation row from normal
`ObservationStore::write_alloc_lifecycle` calls. `RecordedVmm` and the two
coupling adapters observe and model the existing Sim driven-port substrate;
they do not return the expected rows or bypass the owner path. Each altered
ordering claim is witnessed by the fixed seed and the production owner
sequence, while real Unix socket effects remain explicitly a Sim-layer
composition rather than a qualified-metal claim.

The original nested-lock expressions were the only test-fixture defect found.
The accepted snapshot correction preserves their semantic oracle exactly. No
test expectation was weakened, removed, skipped, or replaced; the one ignore
removed is the roadmap-authorized activation marker. No implementation bias,
testing theater, port-boundary violation, missing step criterion, API
divergence, or scope expansion is proven reachable in this step.

### Findings

| ID | Severity | Reachability / evidence | Disposition |
|---|---|---|---|
| None | — | No proven, reachable, in-scope defect remains. The only observed failure was the pre-authored same-mutex temporary-guard deadlock, and the committed snapshots remove that lock-lifetime hazard while preserving the complete oracle. | No remediation required. |

### Iteration verdict

**APPROVED.** The activated S-284-SIM-01 test satisfies the approved
production-owner, rejected-publication, redb close/reopen, fresh-ID,
predecessor-isolation, reclamation idempotence, final cleanup, and C3/TAP
complement obligations. The test correction is mechanically and semantically
equivalent to the authored fixture, DES evidence is complete, verification is
green, the commit is focused and correctly attributed, and no mutation run was
performed.

## Final verdict

**APPROVED.** Step `01-02` may advance to the next roadmap step. The complete
review artifact is this file:

`docs/feature/vm-recreation-allocation-id-reuse/deliver/review-01-02.md`
