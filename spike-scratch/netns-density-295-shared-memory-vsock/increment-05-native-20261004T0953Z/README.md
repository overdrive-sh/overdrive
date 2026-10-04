# Probe increment 05

Same transport source as 04 with changed/pinned dependencies passed. A local prep-script substring error prevented the intended attestation source edit before dispatch; the actual native source hash confirms the unchanged transport code. This version is retained as actually executed.

Exact sources: `Cargo.toml`, `Cargo.lock`, `src/main.rs`, `run.py`; the top-level launcher snapshot is `launch-executed.py`. Sources were retained unchanged after actual native execution. Evidence is under `evidence/`; compressed files have `.gz` suffix.

Native lease receipt:

```text
pid=795027
started_at=2026-10-04T09:53:36Z
action=run
scenario=spike-vsock-increment-05-native-20261004T0953Z
workspace=/Users/marcus/conductor/workspaces/helios/wellington-v2
commit=5ea2d0c506f5c1f0672c2a4b5b7c5d0d415d6bde
token=830f691ff877cd9707dbf302
```

## Mechanical outcomes

- `build`: {"at_s": 2.5569288972765207, "binary_bytes": 713344, "binary_sha256": "914bf13314cd6b44ba139bd0bfc23b66b7732939c51a642d1b4adb7e63f3a6b7", "elapsed_s": 2.337174555286765, "event": "build"}
- `transport-summary`: {"actual_host_bridge_ports_created": 0, "actual_host_taps_created": 0, "actual_linux_microvms": 2, "all_passed": true, "at_s": 8.236499872989953, "configured_memory_mib_per_vm": 256, "configured_vcpu_per_vm": 1, "event": "transport-summary", "scope": "Rust guest AF_VSOCK -> CH userspace virtio-vsock -> owned Unix backend; ordinary TCP applications use explicit loopback wrapper", "unique_guest_cids": [39501, 39502]}
- `cleanup`: {"administrative_configuration_matched": true, "at_s": 8.359826867468655, "different_keys": [], "event": "cleanup", "foreign_modules_unloaded": false, "new_host_netdevices_requested": 0, "owned_processes_alive": [], "owned_uds_remaining": [], "sysctls_tuned": false}
- `final`: {"at_s": 8.3598776133731, "event": "final", "passed": true, "wall_s": 8.359876051545143}

All successful two-VM results prove the transport mechanism only. 16,384 VM identities/endpoints and the current Overdrive policy/observer/lifecycle contract remain NOT VALIDATED.
