# Probe increment 11

First staged 128 target reached 104 actual VMM spawns and 101 echo/HTTP-verified identities before a readiness-prefix check failed. Final v101 console passes every predicate; partial terminal-line observation is the recorded inference. Exact owned cleanup matched, host oom_kill 37→37. No 128 success is claimed from this epoch.

Exact retained code: `Cargo.toml`, `Cargo.lock`, `src/main.rs`, `run.py`. `launch-executed.py` is the byte-preserved top-level launcher snapshot, not a separate run entry point. Native raw/readback evidence is under `evidence/`; files over 16KiB use lossless `.gz` compression. Shared PCAPs are explicitly target-redacted derivatives; originals remain in ignored canonical out/.

Native lease:

```text
pid=801139
started_at=2026-10-04T10:19:37Z
action=run
scenario=spike-vsock-increment-11-staged128-20261004T1014Z
workspace=/Users/marcus/conductor/workspaces/helios/wellington-v2
commit=5ea2d0c506f5c1f0672c2a4b5b7c5d0d415d6bde
token=51acbe2b1bacfadd5fbe2116
```

## Actual mechanical outcomes

- `admission-budget`: {"at_s": 0.010248415172100067, "boot_batch_size": 8, "configured_guest_memory_kib": 12582912, "disk_free_bytes": 698148986880, "disk_reserve_bytes": 8589934592, "event": "admission-budget", "guest_memory_mib": 96, "host_memavailable_kib": 63341508, "host_reserved_kib": 20971520, "image_copy_budget_bytes": 8589934592, "memory_overcommit_relied_on": false, "overhead_allowance_kib": 4194304, "required_available_kib": 37748736, "revised_target_vm_count": 128, "vcpu_per_vm": 1}
- `stage`: {"actual_live_communicating_vm_count": 16, "at_s": 13.843426872044802, "configured_guest_memory_total_mib": 1536, "event": "stage", "guest_kernel": "7.0.0-29-generic", "host_backend_fds": 131, "host_backend_pss_kib": 1120, "host_backend_rss_kib": 1128, "host_backend_threads": 129, "host_bridge_ports_created": 0, "host_memavailable_kib": 61882836, "host_taps_created": 0, "ready_guest_identities": 16, "reserve_kib": 20971520, "successful_guest_echo_count": 16, "successful_host_http_count": 16, "total_vmm_fds": 1776, "total_vmm_pss_kib": 1590648, "total_vmm_rss_kib": 1661816, "total_vmm_threads": 144, "unique_cids": 16, "unique_vm_pids": 16}
- `stage`: {"actual_live_communicating_vm_count": 32, "at_s": 26.896291534416378, "configured_guest_memory_total_mib": 3072, "event": "stage", "guest_kernel": "7.0.0-29-generic", "host_backend_fds": 131, "host_backend_pss_kib": 1120, "host_backend_rss_kib": 1128, "host_backend_threads": 129, "host_bridge_ports_created": 0, "host_memavailable_kib": 60295364, "host_taps_created": 0, "ready_guest_identities": 32, "reserve_kib": 20971520, "successful_guest_echo_count": 32, "successful_host_http_count": 32, "total_vmm_fds": 3552, "total_vmm_pss_kib": 3168288, "total_vmm_rss_kib": 3315360, "total_vmm_threads": 288, "unique_cids": 32, "unique_vm_pids": 32}
- `stage`: {"actual_live_communicating_vm_count": 64, "at_s": 53.417670361697674, "configured_guest_memory_total_mib": 6144, "event": "stage", "guest_kernel": "7.0.0-29-generic", "host_backend_fds": 131, "host_backend_pss_kib": 1120, "host_backend_rss_kib": 1128, "host_backend_threads": 129, "host_bridge_ports_created": 0, "host_memavailable_kib": 57086056, "host_taps_created": 0, "ready_guest_identities": 64, "reserve_kib": 20971520, "successful_guest_echo_count": 64, "successful_host_http_count": 64, "total_vmm_fds": 7104, "total_vmm_pss_kib": 6327500, "total_vmm_rss_kib": 6626380, "total_vmm_threads": 576, "unique_cids": 64, "unique_vm_pids": 64}
- `cleanup`: {"administrative_configuration_matched": true, "at_s": 89.90420244541019, "different_keys": [], "event": "cleanup", "foreign_modules_unloaded": false, "host_memavailable_kib": 63493720, "owned_images_remaining": [], "owned_processes_alive": [], "owned_uds_directory_exists": false, "sysctls_tuned": false}
- `host-oom-counter`: {"after": 37, "at_s": 89.90757032483816, "before": 37, "event": "host-oom-counter", "no_new_host_oom_kills": true}
- `final`: {"actual_ready_communicating_count": 101, "at_s": 89.90759720467031, "event": "final", "passed": false, "wall_s": 89.90759588219225}

User approval: “lets lower both so it fits within the machine”. The original 16,384 DESIGN/E18 contract remains pending; no production security/observer or combined #303/#306 proof is inferred.
