# Step 02-06 Iteration 8 Crafter Evidence

This append-only evidence records the legal DES commands and their observable
results for review remediation D33–D37. It is implementation evidence, not the
review verdict.

## RED

Command:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-worker -p overdrive-control-plane --features integration-tests -E 'test(same_owner_reinstall_waits_for_prior_teardown_before_readiness) or test(unclassified_start_error_removes_intercept_and_network_owner) or test(unclassified_restart_error_retains_typed_cleanup_failure_composition)' --no-fail-fast
```

Result: **FAIL for the intended behavior**, nextest run
`3ce553ad-dad6-4876-9f13-83b7153d896b`, 0/3 passed.

- Same-owner reinstall returned readiness while the prior exact teardown was
  still held at its injected fence.
- An unclassified fresh start error left the preinstalled intercept live.
- An unclassified same-id restart error returned only the primary driver error;
  the injected typed C3 rollback failure was absent.

## GREEN

### Focused remediation contracts

Command:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-worker -p overdrive-control-plane -p overdrive-cli --all-features -E 'test(same_owner_reinstall_waits_for_prior_teardown_before_readiness) or test(same_owner_reinstall_failure_keeps_readiness_closed_until_retry) or test(unclassified_start_error_removes_intercept_and_network_owner) or test(unclassified_restart_error_retains_typed_cleanup_failure_composition) or test(same_job_finalization_is_terminal_and_count_preserving) or test(production_driver_lifecycle_hooks_drive_wired_probe_runner_supervisor) or test(restart_observation_failure_awaits_cleanup_before_reporting)' --no-fail-fast
```

Result: **PASS**, nextest run
`cd8f6f18-a699-4a3b-bb39-af673c4c05d5`, 7/7 passed. The selection proves:

- the old same-owner rule/listener/task teardown is awaited before replacement
  readiness, and a typed teardown failure retains the exact handle for retry;
- unclassified Start and same-id Restart errors remove the new intercept, try
  C3 teardown, and retain the primary plus typed rollback failure when cleanup
  cannot converge;
- the terminal hook's actual process-owned supervisor boundary and the atomic
  durable lifecycle projection converge across pre-effect, temporary-file,
  post-publication, and fresh-production-composition cuts; and
- an injected early S-GTI-06a observation error reports only after both fixture
  cleanup futures complete.

After correcting validation's peer-stop/server-shutdown ordering, the bounded
cleanup contract was rerun on the final source with:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests --test integration -E 'test(restart_observation_failure_awaits_cleanup_before_reporting)' --no-fail-fast
```

Result: **PASS**, nextest run
`e923ddec-17e6-4bec-b65c-b175008b9ef4`, 1/1 passed within its own one-second
bound.

### Broad affected suites

The unfiltered affected-package command was:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-worker -p overdrive-control-plane --features integration-tests --no-fail-fast
```

Result: nextest run `1d75c8a9-2fa8-4786-9a93-72fdd7fd36f7`, 1021/1023 passed.
The two failures were independently classified: the unchanged checked-in
OpenAPI `workload_addr`/`workload_id` drift, and one transient Tier-3 splice
handshake. Neither source belongs to this remediation. The exact splice rerun
was:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-worker --features integration-tests -E 'test(outbound_enforce_substrate_bidirectional_splice_zero_copy)' --no-fail-fast
```

Result: **PASS**, nextest run
`46a50011-cbd6-4435-a96c-825338060850`, 1/1 passed. The final-source broad
command excluding only those two separately classified gates was:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-worker -p overdrive-control-plane --features integration-tests -E 'not (test(openapi_check_subprocess_exits_0_against_checked_in_yaml) or test(outbound_enforce_substrate_bidirectional_splice_zero_copy))' --no-fail-fast
```

Result: **PASS**, nextest run
`9bddc25f-7b2e-4de7-9a4c-bff80ba67958`, 1021/1021 passed.

### Qualified native evidence

The exact S-GTI-06a command below ran four consecutive times on the final
source:

```text
OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4 cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests --test integration -E 'test(a_restarted_microvm_workload_is_re_enrolled_in_the_mesh_before_it_runs_again)' --no-fail-fast
```

All four runs passed 1/1, including the no-pre-readiness-frame assertion and
post-oracle peer-stop followed by authoritative server shutdown:

| Repeat | Nextest run ID | Result |
|---:|---|---|
| 1 | `fde2f687-fa80-4bec-8ea7-4cd7609ed32d` | PASS, 1/1, 34.223s |
| 2 | `bac87e80-a911-47db-a738-b63c09349fbc` | PASS, 1/1, 34.166s |
| 3 | `19a26bb7-a74f-4b89-9f18-c6dbfa36c543` | PASS, 1/1, 34.034s |
| 4 | `85a3c93e-6a15-4cb7-8119-8355637a5a68` | PASS, 1/1, 34.093s |

One earlier complete native validation, run
`ff7045c8-99f5-4ece-8f1b-cc0336486706`, correctly failed 0/1 because peer
stop and server shutdown were driven concurrently; server shutdown won the
race and made the peer stop request unreachable. The final source restores
the required dependency order while storing both results so peer failure
cannot bypass server teardown. The four final-source passes above replace that
intermediate result.

The canonical mapped S-GTI-06a/06b/12a/12b command was:

```text
OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4 cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests --test integration -E 'test(a_restarted_microvm_workload_is_re_enrolled_in_the_mesh_before_it_runs_again) or test(failed_re_enrolment_after_platform_reclamation_stays_closed) or test(a_stopped_microvm_workloads_egress_mesh_guard_is_torn_down_never_left_behind) or test(job_stop_without_a_guest_egress_guard_is_idempotent)' --no-fail-fast
```

Result: **PASS**, nextest run
`15328f82-1796-4ba1-bb27-ce060e1a162c`, 4/4 passed in 84.969s.

### Static gates

Both final-source clippy commands passed with warnings denied:

```text
cargo xtask lima run -- cargo clippy -p overdrive-core -p overdrive-worker -p overdrive-control-plane -p overdrive-reconcilers -p overdrive-cli --all-targets -- -D warnings
cargo xtask lima run -- cargo clippy -p overdrive-core -p overdrive-worker -p overdrive-control-plane -p overdrive-reconcilers -p overdrive-cli --all-targets --all-features -- -D warnings
```

The following final-source checks also passed:

```text
cargo fmt --all -- --check
cargo xtask dst-lint
git diff --check
```

Mutation testing was not run; repository policy reserves it for the single
final DELIVER-wave gate.

## Commit

`des-commit` created initial scoped identity
`cf3a1718a4bce55d3060a2bde3799c73cdbe48df` with subject
`fix(mtls): make terminal and reinstall effects crash-safe`, parent
`1b5cef000540330abbc3b7ed924e4193fc544420`, the required `Step-Id: 02-06`
trailer, and exactly 19 owned files (including the reviewer-authored Iteration
7 review, this evidence, and the append-only DES log). The only following
amend is this commit disposition plus the append-only COMMIT event; unrelated
untracked files remain preserved.
