# Final DELIVER-Wave Mutation Gate

## Status

**NOT REQUIRED — explicit user instruction:** “no mutation testing is required”. This instruction replaces the final mutation gate, including its kill-rate and D17 mutation-witness requirements. No mutation run was executed and no kill rate is claimed. Production invariants and non-mutation verification remain required.

The earlier precondition investigation below is retained as history and does not block finalization under this instruction.

**Earlier precondition result — the mutation run did not start because its unmutated baseline failed.** No kill rate or mutation-gate pass is claimed.

| Field | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Source baseline | `dca2350aad3a9dc5994e1788605fb4903fa8c127` |
| Comparison ref | `origin/main` (`4711f82362998d9ac1c023b2fd2414563ab774b7`) |
| Approved threshold | 80% per package invocation |
| Mutation command | Not run; baseline precondition failed |
| D17 waiver evidence | Not run; required five witnesses remain unverified |

The commands ran from an isolated detached worktree at the reviewed source commit. This kept the primary workspace's unrelated dirty files and any concurrent edits out of the baseline. Raw command output is retained under `.context/service-allocation-readiness-and-ending-mutation/`.

## Gate preconditions and target derivation

All 17 on-disk step reviews are approved. `review-03-02.md` records the final **APPROVED — iteration 2** verdict; `review-05-07.md` records **APPROVED — iteration 2 (final)**. `review-04-02.md` records `Verdict history: Iteration 1: APPROVED`. The roadmap validation status is `approved`, and its `final_wave_gate` requires one cumulative gate, at least 80%, production-only file targets, mandatory Service and Workload reconcile bodies, and all five D17 witnesses.

`execution-log.json` is schema 3.0 and contains 89 phase events, but no `completed_steps[].files_modified.implementation` field. I derived the target set from all 17 approved roadmap step file lists, then reconciled those paths against the step implementation commit stats and review artifacts. The roadmap has 55 unique listed files, including 47 existing Rust source paths. The target inventory below also includes production paths present in the implementation commit stats but omitted from the roadmap lists.

### Production source target union

These are the intended `--file` targets for the cumulative gate. They are grouped by package; each invocation must use `--diff origin/main`, package scope, and `--features integration-tests`. The mandatory reconcile targets are explicitly retained.

| Package | Production source targets |
|---|---|
| `overdrive-control-plane` | `src/worker/exit_observer.rs`; `src/action_shim/mod.rs`; `src/action_shim/validate.rs`; `src/action_shim/enqueue_evaluation.rs`; `src/action_shim/reclamation.rs`; `src/handlers.rs`; `src/streaming.rs`; `src/reconciler_runtime.rs`; `src/lib.rs`; `src/vm_reclamation_boot.rs`; `src/gateway_identity_lifecycle.rs`; `src/probe_runner_boot.rs`; `src/api.rs`; `src/error.rs` |
| `overdrive-core` | `src/aggregate/mod.rs`; `src/aggregate/probe_descriptor.rs`; `src/aggregate/service_spec.rs`; `src/aggregate/workload_spec.rs`; `src/api/mod.rs`; `src/api/submit.rs`; `src/api/describe.rs`; `src/observation/probe_result_row.rs`; `src/traits/driver.rs`; `src/traits/guest_listener_reports.rs`; `src/transition_reason.rs`; `src/vm/beacon.rs` |
| `overdrive-reconcilers` | `src/service_lifecycle.rs`; `src/workload_lifecycle.rs` |
| `overdrive-worker` | `src/guest_listener_board.rs`; `src/mtls_intercept_worker.rs`; `src/probe_runner/mod.rs`; `src/probe_runner/supervisor.rs`; `src/vm_driver.rs` |
| `overdrive-cli` | `src/render.rs`; `src/commands/deploy.rs`; `src/http_client.rs` |
| `overdrive-init` | `src/main.rs`; `src/transport/kernel.rs`; `src/transport/mod.rs`; `src/workload.rs` |
| `overdrive-store-local` | `src/observation_backend.rs` |

`workload_lifecycle.rs` and `service_lifecycle.rs` remain targets for every applicable reconciler invocation. The expanded production targets come from the implementation commit stats: `overdrive-control-plane/src/error.rs`, `overdrive-worker/src/probe_runner/supervisor.rs`, and the three userspace `overdrive-init` transport/workload files. The D10 API relocation is resolved by carrying `overdrive-core/src/api/mod.rs`, `submit.rs`, and `describe.rs`; the former `src/api.rs` path no longer exists. `provision_failure_cleanup.rs` under `overdrive-sim/src/invariants/` is also absent and was removed upstream before this feature.

The file list is the cumulative source inventory, not a claim that each path has a diff against `origin/main`. The required `--diff origin/main` filter determines which mutations actually overlap the current base.

### Excluded paths and evidence

- The two roadmap BPF targets, plus the implementation's `overdrive-bpf/src/programs/listen.rs`, remain excluded by `.cargo/mutants.toml`'s `crates/overdrive-bpf/**` glob and the accepted `aya-rs`/unsafe exclusion. The final gate explicitly limits step 05-06 to userspace init code.
- `overdrive-sim/src/adapters/{driver,observation_store}.rs` and `src/invariants/terminal_contention.rs` remain excluded by the checked-in `overdrive-sim/src/{adapters,invariants}/**` rule. The package is the `adapter-sim` / DST harness; these paths are exercised by `cargo dst`, not the per-mutant nextest lane.
- Test-local modules under `overdrive-control-plane/src` are not production mutation targets. Their parent modules declare them under `#[cfg(test)]` or `#[cfg(all(test, feature = "integration-tests"))]`: examples include `guest_activation_acceptance.rs`, `replacement_acceptance.rs`, `guest_listener_gate_composed.rs`, `probe_count_fixture.rs`, and the `quint_*` modules. The same applies to `overdrive-worker`'s `run_numbering_tests.rs` and `quint_flows_beacon.rs`.
- `overdrive-core/src/testing/observation_store.rs` is shared test logic and is excluded by the checked-in `crates/*/src/testing/**` rule. `overdrive-testing/src/guest_lane.rs`, `overdrive-init/src/transport/tests.rs`, test directories, and generated `api/openapi.yaml` are test-support or generated inputs, not runtime mutation targets.

## Baseline evidence

The first canonical baseline invocation stopped during compilation because the isolated remote tree did not yet contain `target/bpf/overdrive_bpf.o`. I then built the prerequisite with the documented command `cargo xtask metal run -- env OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build`; it completed successfully (Metal run `c15ce2eed6dc979271275eca`). The build created BPF artifacts remotely and made no source changes.

After that prerequisite, the baseline command was rerun through the canonical Metal runner:

```text
cargo xtask metal run -- cargo nextest run \
  -p overdrive-control-plane -p overdrive-core -p overdrive-reconcilers \
  -p overdrive-worker -p overdrive-cli -p overdrive-init \
  -p overdrive-store-local --features integration-tests
```

Metal run `3c183e5b40201e243ab024a0` exited 100. Nextest discovered 3,078 tests across 43 binaries (19 skipped); 59 passed and one failed before nextest cancelled the remaining tests. The failing test is:

```text
overdrive-cli::acceptance::render_pure_fns::failed_block_reason_text_is_variant_specific
```

At `crates/overdrive-cli/tests/acceptance/render_pure_fns.rs:373-386`, the `Timeout { after_seconds: 90 }` case requires the text `workload did not converge within 90s`. The current renderer returned `workload did not become Stable within 90s; the platform may still be restarting it`. The complete failing output is retained in `baseline-nextest-retry.log`.

Because this baseline is not green, I did not invoke `cargo xtask mutants`. No mutants were evaluated, so there is no valid caught/missed/unviable/timeout count or kill rate. I did not edit tests, production code, mutation exclusions, or DES events. The checked-in `.cargo/mutants.toml` has the same SHA-256 in the primary checkout and isolated worktree: `097dcf12fbc2af593a95fadac4bfedd862f65b8ab68838194a9d76d31fc7aeb3`.

## D17 configuration constraint

The roadmap requires the mutation gate to kill the five S-48(b) witnesses: signal before broker insertion, dropped signal, waiter-only wake instead of a stored permit, removal of the submitted `select!` arm, and removal of the cadence arm. The current `.cargo/mutants.toml` excludes every mutation description containing `select!` at line 219. The `cargo xtask mutants` argument builder in `xtask/src/mutants.rs:295-357` does not pass a config override or mutation-description inclusion filter. Therefore the two required `select!`-arm witnesses would be filtered by the unchanged project config. No config was changed and no D17 mutant was reported as killed.

Before resuming, the baseline mismatch needs an owner-approved resolution, and the D17 gate invocation needs a narrowly scoped way to include its two required `select!` witnesses while preserving the other documented async-scheduling exclusions. The gate must then run cumulatively across the target union and demonstrate all five D17 witnesses; this report records neither as complete.

## Retained evidence

- `.context/service-allocation-readiness-and-ending-mutation/baseline-nextest-retry.log` — failed post-prerequisite baseline.
- `.context/service-allocation-readiness-and-ending-mutation/baseline-nextest.log` — initial attempt, blocked by the missing BPF object.
- `.context/service-allocation-readiness-and-ending-mutation/bpf-build.log` — successful documented BPF prerequisite build.
- `.context/service-allocation-readiness-and-ending-mutation/mutants.toml.inventory` — unchanged mutation configuration used for the attempt.
