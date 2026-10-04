# Probe increment 13

Owned CPU-quota parent/controller setup worked, but a probe readback incorrectly assumed cpuset.cpus.effective existed. It failed before any VMM, then removed the exact owned parent. This is not a native service failure at 0.125 and does not justify 0.25 fallback.

Exact retained code: `Cargo.toml`, `Cargo.lock`, `src/main.rs`, `run.py`, `quota.py`. `launch-executed.py` is the byte-preserved top-level launcher snapshot, not a separate run entry point. Native raw/readback evidence is under `evidence/`; files over 16KiB use lossless `.gz` compression. Shared PCAPs are explicitly target-redacted derivatives; originals remain in ignored canonical out/.

Native lease:

```text
pid=806632
started_at=2026-10-04T10:30:58Z
action=run
scenario=spike-vsock-increment-13-quota0125-two-vm-20261004T1029Z
workspace=/Users/marcus/conductor/workspaces/helios/wellington-v2
commit=5ea2d0c506f5c1f0672c2a4b5b7c5d0d415d6bde
token=f3d5053f8291d6d23bdf1f3b
```

## Actual mechanical outcomes

- `cleanup`: {"administrative_configuration_matched": true, "at_s": 0.26614306401461363, "different_keys": [], "event": "cleanup", "foreign_modules_unloaded": false, "new_host_netdevices_requested": 0, "owned_processes_alive": [], "owned_uds_remaining": [], "sysctls_tuned": false}
- `final`: {"at_s": 0.2661756332963705, "event": "final", "passed": false, "wall_s": 0.2661745911464095}

User approval: “lets lower both so it fits within the machine”. The original 16,384 DESIGN/E18 contract remains pending; no production security/observer or combined #303/#306 proof is inferred.
