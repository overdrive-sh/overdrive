# Qualified-metal evidence — step 01-02

## Metadata

| Field | Value |
|---|---|
| Feature | `vm-recreation-allocation-id-reuse` |
| Step | `01-02` |
| Substrate | native non-virtualized x86_64 KVM metal host |
| Source commit | `72cb4fea03ac778d671a9904585e73bf162cc183` plus the step-01-02 fixture correction |
| Kernel | `/srv/vm/overdrive-testing/kernel` |
| Rootfs | `/srv/vm/overdrive-testing/rootfs.ext4` |

This is the current qualified-metal receipt for the corrective driver-neutral
delivery. Commit `b295d973de2167124f8cf6be0bea64660cbac078` retained the
receipt; the archived VM-only evidence under `superseded-vm-only/` is not used
to establish this result.

## Command

```text
OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4 cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests --test integration -E 'test(reclaiming_an_svid_holding_allocation_submits_the_fourth_evaluation) or test(reclaim_then_fresh_start_retains_predecessor_and_resets_history) or test(restarted_vm_boots_from_a_clean_unmodified_rootfs_copy) or test(a_restarted_microvm_workload_is_re_enrolled_in_the_mesh_before_it_runs_again) or test(failed_re_enrolment_after_platform_reclamation_stays_closed) or test(predecessor_cleanup_cannot_bind_or_remove_replacement_vm_artifacts)' --no-capture --no-fail-fast
```

Exit status: `0`

## Captured stdout

```text
Nextest run ID 391025fc-2e06-430e-93e5-52f4ddf0dde5 with nextest profile: default
Starting 6 tests across 1 binary (166 tests skipped)
PASS integration::guest_stack_mtls_egress::a_restarted_microvm_workload_is_re_enrolled_in_the_mesh_before_it_runs_again (32.645s)
PASS integration::guest_stack_mtls_egress::failed_re_enrolment_after_platform_reclamation_stays_closed (5.099s)
PASS integration::vm_reclamation_tier3::reclaim_then_fresh_start_retains_predecessor_and_resets_history (5.161s)
PASS integration::vm_reclamation_tier3::reclaiming_an_svid_holding_allocation_submits_the_fourth_evaluation (5.543s)
PASS integration::vm_stop_restart_and_vmm_death::predecessor_cleanup_cannot_bind_or_remove_replacement_vm_artifacts (17.363s)
PASS integration::vm_stop_restart_and_vmm_death::restarted_vm_boots_from_a_clean_unmodified_rootfs_copy (5.173s)
Summary [70.989s] 6 tests run: 6 passed, 166 skipped
```

The command's native preflight completed before the six-test run. The
predecessor-artifact test emitted the following exact strace capture record:

```text
GH #284 raw strace capture: path=/var/tmp/overdrive-test-evidence/vm-allocation-ownership/1789326561586357993-1787813/strace.raw command="cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests --test integration --run-ignored ignored-only -E 'test(predecessor_cleanup_cannot_bind_or_remove_replacement_vm_artifacts)' --no-capture" substrate=native-non-virtualized-x86_64-kvm source=overdrive-cli::integration::vm_stop_restart_and_vmm_death pid=1787813
retained GH #284 raw strace evidence at /var/tmp/overdrive-test-evidence/vm-allocation-ownership/1789326561586357993-1787813/strace.raw
```

The raw capture is mode-0700 evidence retained by the qualified-metal
fixture. It records distinct predecessor/successor beacon, run-directory,
socket and cleanup paths; the test also asserts final predecessor PID and
artifact absence after exact-old cleanup.
