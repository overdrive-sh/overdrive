# Probe increment 04

Real two-VM transport passed: guest ordinary TCP wrapper echo, host-to-guest ordinary HTTP, missing host/guest port and sibling/unknown CID denies, exact process/path cleanup and administrative complement.

Exact sources: `Cargo.toml`, `Cargo.lock`, `src/main.rs`, `run.py`; the top-level launcher snapshot is `launch-executed.py`. Sources were retained unchanged after actual native execution. Evidence is under `evidence/`; compressed files have `.gz` suffix.

Native lease receipt:

```text
pid=794129
started_at=2026-10-04T09:51:01Z
action=run
scenario=spike-vsock-increment-04-native-20261004T0951Z
workspace=/Users/marcus/conductor/workspaces/helios/wellington-v2
commit=5ea2d0c506f5c1f0672c2a4b5b7c5d0d415d6bde
token=279203eaba8a5b030d35d693
```

## Mechanical outcomes

- `build`: {"at_s": 0.7631938261911273, "binary_bytes": 713328, "binary_sha256": "2756d6116a3ccf94f85b39ca67fece549b239907154b2f435ee66a7fa78bfb20", "elapsed_s": 0.5433347076177597, "event": "build"}
- `transport-summary`: {"actual_host_bridge_ports_created": 0, "actual_host_taps_created": 0, "actual_linux_microvms": 2, "all_passed": true, "at_s": 6.434150285087526, "configured_memory_mib_per_vm": 256, "configured_vcpu_per_vm": 1, "event": "transport-summary", "scope": "Rust guest AF_VSOCK -> CH userspace virtio-vsock -> owned Unix backend; ordinary TCP applications use explicit loopback wrapper", "unique_guest_cids": [39501, 39502]}
- `cleanup`: {"administrative_configuration_matched": true, "at_s": 6.590346021577716, "different_keys": [], "event": "cleanup", "foreign_modules_unloaded": false, "new_host_netdevices_requested": 0, "owned_processes_alive": [], "owned_uds_remaining": [], "sysctls_tuned": false}
- `final`: {"at_s": 6.590378470718861, "event": "final", "passed": true, "wall_s": 6.590377368964255}

All successful two-VM results prove the transport mechanism only. 16,384 VM identities/endpoints and the current Overdrive policy/observer/lifecycle contract remain NOT VALIDATED.
