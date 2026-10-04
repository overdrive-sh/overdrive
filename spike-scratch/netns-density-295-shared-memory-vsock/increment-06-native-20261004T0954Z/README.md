# Probe increment 06

Attestation-enhanced two-VM transport passed. Both loaded guest executable hashes and actual CID ioctls matched; stock guest module hashes, process/FD resources, root-owned 0700 runtime directory, host PSS and exact cleanup are retained. The 16,384 population was not run.

Exact sources: `Cargo.toml`, `Cargo.lock`, `src/main.rs`, `run.py`; the top-level launcher snapshot is `launch-executed.py`. Sources were retained unchanged after actual native execution. Evidence is under `evidence/`; compressed files have `.gz` suffix.

Native lease receipt:

```text
pid=796012
started_at=2026-10-04T09:55:05Z
action=run
scenario=spike-vsock-increment-06-native-20261004T0954Z
workspace=/Users/marcus/conductor/workspaces/helios/wellington-v2
commit=5ea2d0c506f5c1f0672c2a4b5b7c5d0d415d6bde
token=b46672e94bde74ab6b50d834
```

## Mechanical outcomes

- `build`: {"at_s": 0.9202268626540899, "binary_bytes": 731184, "binary_sha256": "78a0b5b32f27637c449f210b1c928c020c06ba0e8267842153d0a84d293327c4", "elapsed_s": 0.6990827601402998, "event": "build"}
- `transport-summary`: {"actual_host_bridge_ports_created": 0, "actual_host_taps_created": 0, "actual_linux_microvms": 2, "all_passed": true, "at_s": 6.6553516034036875, "configured_memory_mib_per_vm": 256, "configured_vcpu_per_vm": 1, "event": "transport-summary", "scope": "Rust guest AF_VSOCK -> CH userspace virtio-vsock -> owned Unix backend; ordinary TCP applications use explicit loopback wrapper", "unique_guest_cids": [39501, 39502]}
- `cleanup`: {"administrative_configuration_matched": true, "at_s": 6.7934156293049455, "different_keys": [], "event": "cleanup", "foreign_modules_unloaded": false, "new_host_netdevices_requested": 0, "owned_processes_alive": [], "owned_uds_remaining": [], "sysctls_tuned": false}
- `final`: {"at_s": 6.793471604585648, "event": "final", "passed": true, "wall_s": 6.7934697307646275}

All successful two-VM results prove the transport mechanism only. 16,384 VM identities/endpoints and the current Overdrive policy/observer/lifecycle contract remain NOT VALIDATED.
