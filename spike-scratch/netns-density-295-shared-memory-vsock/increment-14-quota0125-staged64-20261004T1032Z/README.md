# Probe increment 14

64 retained real VMM/device/backend identities at 96MiB passed all services and final full-pool HTTP recheck with per-VMM cpu.max=12500/100000 (0.125 logical-CPU time budget), separate one-guest-vCPU topology and ancestor parent quota 800000/100000. All 576 VMM threads were read back in their owned cgroups; actual throttling occurred. Exact process/path/64-child-cgroup/parent cleanup matched; host oom_kill 37→37.

Exact retained code: `Cargo.toml`, `Cargo.lock`, `src/main.rs`, `run.py`, `quota.py`. `launch-executed.py` is the byte-preserved top-level launcher snapshot, not a separate run entry point. Native raw/readback evidence is under `evidence/`; files over 16KiB use lossless `.gz` compression. Shared PCAPs are explicitly target-redacted derivatives; originals remain in ignored canonical out/.

Native lease:

```text
pid=807451
started_at=2026-10-04T10:33:33Z
action=run
scenario=spike-vsock-increment-14-quota0125-staged64-20261004T1032Z
workspace=/Users/marcus/conductor/workspaces/helios/wellington-v2
commit=5ea2d0c506f5c1f0672c2a4b5b7c5d0d415d6bde
token=9ac53d6bc51dc43546286f4d
```

## Actual mechanical outcomes

- `admission-budget`: {"aggregate_cpu_budget_logical_cpus": 8.0, "at_s": 0.010530389845371246, "boot_batch_size": 8, "configured_guest_memory_kib": 6291456, "disk_free_bytes": 698115637248, "disk_reserve_bytes": 8589934592, "event": "admission-budget", "guest_memory_mib": 96, "guest_vcpu_topology": 1, "host_cpu_quota_logical_cpus": 0.125, "host_memavailable_kib": 63438212, "host_reserved_kib": 20971520, "image_copy_budget_bytes": 4294967296, "memory_overcommit_relied_on": false, "overhead_allowance_kib": 4194304, "required_available_kib": 31457280, "revised_target_vm_count": 64, "vcpu_per_vm": 1}
- `stage`: {"actual_live_communicating_vm_count": 16, "at_s": 32.041231570765376, "configured_guest_memory_total_mib": 1536, "event": "stage", "guest_kernel": "7.0.0-29-generic", "host_backend_fds": 67, "host_backend_pss_kib": 864, "host_backend_rss_kib": 872, "host_backend_threads": 65, "host_bridge_ports_created": 0, "host_memavailable_kib": 61772800, "host_taps_created": 0, "ready_guest_identities": 16, "reserve_kib": 20971520, "successful_guest_echo_count": 16, "successful_host_http_count": 16, "total_vmm_fds": 1776, "total_vmm_pss_kib": 1591616, "total_vmm_rss_kib": 1662784, "total_vmm_threads": 144, "unique_cids": 16, "unique_vm_pids": 16}
- `stage`: {"actual_live_communicating_vm_count": 32, "at_s": 62.73134702723473, "configured_guest_memory_total_mib": 3072, "event": "stage", "guest_kernel": "7.0.0-29-generic", "host_backend_fds": 67, "host_backend_pss_kib": 864, "host_backend_rss_kib": 872, "host_backend_threads": 65, "host_bridge_ports_created": 0, "host_memavailable_kib": 60190192, "host_taps_created": 0, "ready_guest_identities": 32, "reserve_kib": 20971520, "successful_guest_echo_count": 32, "successful_host_http_count": 32, "total_vmm_fds": 3552, "total_vmm_pss_kib": 3178496, "total_vmm_rss_kib": 3325568, "total_vmm_threads": 288, "unique_cids": 32, "unique_vm_pids": 32}
- `stage`: {"actual_live_communicating_vm_count": 64, "at_s": 124.26840055733919, "configured_guest_memory_total_mib": 6144, "event": "stage", "guest_kernel": "7.0.0-29-generic", "host_backend_fds": 67, "host_backend_pss_kib": 864, "host_backend_rss_kib": 872, "host_backend_threads": 65, "host_bridge_ports_created": 0, "host_memavailable_kib": 56967448, "host_taps_created": 0, "ready_guest_identities": 64, "reserve_kib": 20971520, "successful_guest_echo_count": 64, "successful_host_http_count": 64, "total_vmm_fds": 7104, "total_vmm_pss_kib": 6352280, "total_vmm_rss_kib": 6651160, "total_vmm_threads": 576, "unique_cids": 64, "unique_vm_pids": 64}
- `final-full-pool-recheck`: {"actual_live_vm_count": 64, "at_s": 124.56343129090965, "elapsed_s": 0.29388953745365143, "event": "final-full-pool-recheck", "successful_live_http_identities": 64, "unique_cids": 64}
- `transport-summary`: {"actual_host_bridge_ports_created": 0, "actual_host_taps_created": 0, "actual_linux_microvms": 64, "actual_ready_communicating_identities": 64, "aggregate_parent_cpu_max": "800000 100000", "all_passed": true, "at_s": 124.56382927019149, "configured_memory_mib_per_vm": 96, "configured_vcpu_per_vm": 1, "event": "transport-summary", "full_16384_target_validated": false, "host_cpu_quota_logical_cpus": 0.125, "scope": "bounded revised target under explicit resource/count approval; no production network-policy or observer acceptance"}
- `cleanup`: {"administrative_configuration_matched": true, "at_s": 126.63838605582714, "different_keys": [], "event": "cleanup", "foreign_modules_unloaded": false, "host_memavailable_kib": 63464932, "owned_images_remaining": [], "owned_processes_alive": [], "owned_uds_directory_exists": false, "sysctls_tuned": false}
- `host-oom-counter`: {"after": 37, "at_s": 126.64307100698352, "before": 37, "event": "host-oom-counter", "no_new_host_oom_kills": true}
- `final`: {"actual_ready_communicating_count": 64, "at_s": 126.64311269018799, "event": "final", "passed": true, "wall_s": 126.64311101846397}

User approval: “lets lower both so it fits within the machine”. The original 16,384 DESIGN/E18 contract remains pending; no production security/observer or combined #303/#306 proof is inferred.
