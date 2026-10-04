# Probe increment 08

Two real Linux VMs passed the same checks at 96MiB each. This is the smallest demonstrated service-capable profile; 256MiB is not an architectural floor.

Exact retained code: `Cargo.toml`, `Cargo.lock`, `src/main.rs`, `run.py`. `launch-executed.py` is the byte-preserved top-level launcher snapshot, not a separate run entry point. Native raw/readback evidence is under `evidence/`; files over 16KiB use lossless `.gz` compression. Shared PCAPs are explicitly target-redacted derivatives; originals remain in ignored canonical out/.

Native lease:

```text
pid=798492
started_at=2026-10-04T10:14:53Z
action=run
scenario=spike-vsock-increment-08-ram96-20261004T1014Z
workspace=/Users/marcus/conductor/workspaces/helios/wellington-v2
commit=5ea2d0c506f5c1f0672c2a4b5b7c5d0d415d6bde
token=c977bf2348b95bdd926793b0
```

## Actual mechanical outcomes

- `transport-summary`: {"actual_host_bridge_ports_created": 0, "actual_host_taps_created": 0, "actual_linux_microvms": 2, "all_passed": true, "at_s": 6.432433708570898, "configured_memory_mib_per_vm": 96, "configured_vcpu_per_vm": 1, "event": "transport-summary", "scope": "Rust guest AF_VSOCK -> CH userspace virtio-vsock -> owned Unix backend; ordinary TCP applications use explicit loopback wrapper", "unique_guest_cids": [39501, 39502]}
- `cleanup`: {"administrative_configuration_matched": true, "at_s": 6.569096405990422, "different_keys": [], "event": "cleanup", "foreign_modules_unloaded": false, "new_host_netdevices_requested": 0, "owned_processes_alive": [], "owned_uds_remaining": [], "sysctls_tuned": false}
- `final`: {"at_s": 6.569130918011069, "event": "final", "passed": true, "wall_s": 6.569129555486143}

User approval: “lets lower both so it fits within the machine”. The original 16,384 DESIGN/E18 contract remains pending; no production security/observer or combined #303/#306 proof is inferred.
