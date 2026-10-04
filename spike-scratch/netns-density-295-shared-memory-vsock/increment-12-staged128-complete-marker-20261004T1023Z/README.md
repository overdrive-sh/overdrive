# Probe increment 12

128 retained actual VMM/device/backend identities at 96MiB passed both traffic directions, CID/binary checks, four transport negatives each, live stages and a final full-pool HTTP recheck. No explicit CPU quota was imposed. Cleanup matched, host oom_kill 37→37.

Exact retained code: `Cargo.toml`, `Cargo.lock`, `src/main.rs`, `run.py`. `launch-executed.py` is the byte-preserved top-level launcher snapshot, not a separate run entry point. Native raw/readback evidence is under `evidence/`; files over 16KiB use lossless `.gz` compression. Shared PCAPs are explicitly target-redacted derivatives; originals remain in ignored canonical out/.

Native lease:

```text
pid=803710
started_at=2026-10-04T10:23:35Z
action=run
scenario=spike-vsock-increment-12-staged128-complete-marker-20261004T1023Z
workspace=/Users/marcus/conductor/workspaces/helios/wellington-v2
commit=5ea2d0c506f5c1f0672c2a4b5b7c5d0d415d6bde
token=5c2755e9a8234cee1745790b
```

## Actual mechanical outcomes

- `admission-budget`: {"at_s": 0.011163081973791122, "boot_batch_size": 8, "configured_guest_memory_kib": 12582912, "disk_free_bytes": 698135662592, "disk_reserve_bytes": 8589934592, "event": "admission-budget", "guest_memory_mib": 96, "host_memavailable_kib": 63456848, "host_reserved_kib": 20971520, "image_copy_budget_bytes": 8589934592, "memory_overcommit_relied_on": false, "overhead_allowance_kib": 4194304, "required_available_kib": 37748736, "revised_target_vm_count": 128, "vcpu_per_vm": 1}
- `stage`: {"actual_live_communicating_vm_count": 16, "at_s": 13.631788886152208, "configured_guest_memory_total_mib": 1536, "event": "stage", "guest_kernel": "7.0.0-29-generic", "host_backend_fds": 131, "host_backend_pss_kib": 1164, "host_backend_rss_kib": 1172, "host_backend_threads": 129, "host_bridge_ports_created": 0, "host_memavailable_kib": 61483596, "host_taps_created": 0, "ready_guest_identities": 16, "reserve_kib": 20971520, "successful_guest_echo_count": 16, "successful_host_http_count": 16, "total_vmm_fds": 1776, "total_vmm_pss_kib": 1591632, "total_vmm_rss_kib": 1662800, "total_vmm_threads": 144, "unique_cids": 16, "unique_vm_pids": 16}
- `stage`: {"actual_live_communicating_vm_count": 32, "at_s": 26.609626850113273, "configured_guest_memory_total_mib": 3072, "event": "stage", "guest_kernel": "7.0.0-29-generic", "host_backend_fds": 131, "host_backend_pss_kib": 1164, "host_backend_rss_kib": 1172, "host_backend_threads": 129, "host_bridge_ports_created": 0, "host_memavailable_kib": 59955060, "host_taps_created": 0, "ready_guest_identities": 32, "reserve_kib": 20971520, "successful_guest_echo_count": 32, "successful_host_http_count": 32, "total_vmm_fds": 3552, "total_vmm_pss_kib": 3178516, "total_vmm_rss_kib": 3325588, "total_vmm_threads": 288, "unique_cids": 32, "unique_vm_pids": 32}
- `stage`: {"actual_live_communicating_vm_count": 64, "at_s": 52.65909955278039, "configured_guest_memory_total_mib": 6144, "event": "stage", "guest_kernel": "7.0.0-29-generic", "host_backend_fds": 131, "host_backend_pss_kib": 1164, "host_backend_rss_kib": 1172, "host_backend_threads": 129, "host_bridge_ports_created": 0, "host_memavailable_kib": 57107772, "host_taps_created": 0, "ready_guest_identities": 64, "reserve_kib": 20971520, "successful_guest_echo_count": 64, "successful_host_http_count": 64, "total_vmm_fds": 7104, "total_vmm_pss_kib": 6352264, "total_vmm_rss_kib": 6651144, "total_vmm_threads": 576, "unique_cids": 64, "unique_vm_pids": 64}
- `stage`: {"actual_live_communicating_vm_count": 128, "at_s": 105.5170980039984, "configured_guest_memory_total_mib": 12288, "event": "stage", "guest_kernel": "7.0.0-29-generic", "host_backend_fds": 131, "host_backend_pss_kib": 1164, "host_backend_rss_kib": 1172, "host_backend_threads": 129, "host_bridge_ports_created": 0, "host_memavailable_kib": 50681776, "host_taps_created": 0, "ready_guest_identities": 128, "reserve_kib": 20971520, "successful_guest_echo_count": 128, "successful_host_http_count": 128, "total_vmm_fds": 14208, "total_vmm_pss_kib": 12696872, "total_vmm_rss_kib": 13299368, "total_vmm_threads": 1152, "unique_cids": 128, "unique_vm_pids": 128}
- `final-full-pool-recheck`: {"actual_live_vm_count": 128, "at_s": 106.16395253315568, "elapsed_s": 0.6439965777099133, "event": "final-full-pool-recheck", "successful_live_http_identities": 128, "unique_cids": 128}
- `transport-summary`: {"actual_host_bridge_ports_created": 0, "actual_host_taps_created": 0, "actual_linux_microvms": 128, "actual_ready_communicating_identities": 128, "all_passed": true, "at_s": 106.16446264088154, "configured_memory_mib_per_vm": 96, "configured_vcpu_per_vm": 1, "event": "transport-summary", "full_16384_target_validated": false, "scope": "bounded revised target under explicit resource/count approval; no production network-policy or observer acceptance"}
- `cleanup`: {"administrative_configuration_matched": true, "at_s": 111.03672235459089, "different_keys": [], "event": "cleanup", "foreign_modules_unloaded": false, "host_memavailable_kib": 63471820, "owned_images_remaining": [], "owned_processes_alive": [], "owned_uds_directory_exists": false, "sysctls_tuned": false}
- `host-oom-counter`: {"after": 37, "at_s": 111.03994755912572, "before": 37, "event": "host-oom-counter", "no_new_host_oom_kills": true}
- `final`: {"actual_ready_communicating_count": 128, "at_s": 111.03997407946736, "event": "final", "passed": true, "wall_s": 111.03997292742133}

User approval: “lets lower both so it fits within the machine”. The original 16,384 DESIGN/E18 contract remains pending; no production security/observer or combined #303/#306 proof is inferred.
