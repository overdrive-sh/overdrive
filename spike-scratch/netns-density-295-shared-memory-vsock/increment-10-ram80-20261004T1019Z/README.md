# Probe increment 10

80MiB profile did not reach guest init/service readiness; a VMM reported VcpuRun InternalError. No architectural minimum or host OOM claim follows. Cleanup matched.

Exact retained code: `Cargo.toml`, `Cargo.lock`, `src/main.rs`, `run.py`. `launch-executed.py` is the byte-preserved top-level launcher snapshot, not a separate run entry point. Native raw/readback evidence is under `evidence/`; files over 16KiB use lossless `.gz` compression. Shared PCAPs are explicitly target-redacted derivatives; originals remain in ignored canonical out/.

Native lease:

```text
pid=800262
started_at=2026-10-04T10:18:04Z
action=run
scenario=spike-vsock-increment-10-ram80-20261004T1019Z
workspace=/Users/marcus/conductor/workspaces/helios/wellington-v2
commit=5ea2d0c506f5c1f0672c2a4b5b7c5d0d415d6bde
token=97060a8bd0420a277e266bf1
```

## Actual mechanical outcomes

- `cleanup`: {"administrative_configuration_matched": true, "at_s": 1.2668592659756541, "different_keys": [], "event": "cleanup", "foreign_modules_unloaded": false, "new_host_netdevices_requested": 0, "owned_processes_alive": [], "owned_uds_remaining": [], "sysctls_tuned": false}
- `final`: {"at_s": 1.2668941281735897, "event": "final", "passed": false, "wall_s": 1.2668929267674685}

User approval: “lets lower both so it fits within the machine”. The original 16,384 DESIGN/E18 contract remains pending; no production security/observer or combined #303/#306 proof is inferred.
