# Step 03-01 DISTILL remediation: VM hook chronology

## Reproduced failure and approved contract

The existing `vm_driver_delegates_existing_probe_lifecycle_hooks_to_shared_runner` acceptance test failed on Metal at its Running-phase assertion that Liveness was already live. The composed fixture constructs the real `VmDriver` with one shared `ProbeRunner`, then calls its existing `on_alloc_running`, `on_alloc_stable`, and `on_alloc_terminal` hooks. The failure was deterministic; no seed or concurrency schedule was involved.

D9/ADR-0261 assigns the gate to `ProbeRunner`: `start_alloc` starts Startup and Readiness but retains Liveness descriptors, and `VmDriver::on_alloc_stable` retires Startup then starts the retained Liveness tasks. The Stable hook follows a committed Stable row in the production action shim. The current implementation has exactly that path (`vm_driver.rs:2063-2075`, `probe_runner/mod.rs:314-356`, `:394-411`). The test's Running-phase Liveness expectation was the pre-D9 contract.

## Bounded test correction

The same test now asserts the exact role chronology:

| Hook | Supervisor | Startup | Readiness | Liveness |
| --- | --- | --- | --- | --- |
| `on_alloc_running` | one | live | live | retained, not live |
| `on_alloc_stable` | still one | stopped | live | live |
| `on_alloc_terminal` | zero | stopped | stopped | stopped |

The test still drives the existing `VmDriver` hooks and inspects the shared production `ProbeRunner`; it adds no seam or API. Its `/// CONTRACT_SHAPE: bounded-change.` declaration remains. No production behavior, step activation, DES event, or commit changed in this remediation.

## Verification

| Check | Result |
| --- | --- |
| Before: `cargo xtask metal run -- cargo nextest run -p overdrive-worker --test acceptance -E 'test(vm_driver_delegates_existing_probe_lifecycle_hooks_to_shared_runner)'` | Reproduced the old Running-phase Liveness assertion failure, exit 100. |
| After: named VM lifecycle hook test, restart re-registration control, and TCP probe observation control on Metal | 3/3 passed, exit 0. |
| Changed-file `rustfmt --check --edition 2024` and `git diff --check` | Passed. |

The after-fix command was:

```sh
cargo xtask metal run -- cargo nextest run -p overdrive-worker --test acceptance --no-fail-fast -E 'test(vm_driver_delegates_existing_probe_lifecycle_hooks_to_shared_runner) | test(vm_restart_reregistration_is_idempotent_and_does_not_duplicate_probe_tasks) | test(vm_tcp_probe_records_guest_connect_outcome_without_owning_running)'
```

Changed file: `crates/overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs` (oracle only). Independent DISTILL review is the handoff gate before the original step 03-01 crafter resumes GREEN and COMMIT.
