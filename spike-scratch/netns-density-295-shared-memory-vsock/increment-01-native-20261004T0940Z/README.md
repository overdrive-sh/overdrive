# Probe increment 01

Native build passed; no VM was started because the UDS path-length guard rejected the canonical long path. Administrative complement matched.

Exact sources: `Cargo.toml`, `Cargo.lock`, `src/main.rs`, `run.py`; the top-level launcher snapshot is `launch-executed.py`. Sources were retained unchanged after actual native execution. Evidence is under `evidence/`; compressed files have `.gz` suffix.

Native lease receipt:

```text
pid=791515
started_at=2026-10-04T09:45:54Z
action=run
scenario=spike-vsock-increment-01-native-20261004T0940Z
workspace=/Users/marcus/conductor/workspaces/helios/wellington-v2
commit=5ea2d0c506f5c1f0672c2a4b5b7c5d0d415d6bde
token=94a56588edb2c9fcddbee63c
```

## Mechanical outcomes

- `build`: {"at_s": 2.0443956507369876, "binary_bytes": 765352, "binary_sha256": "907ba98c94938b34dceae677e3af56a0a185f15a4ddfa6767b5716e716b5f6f0", "elapsed_s": 1.825001971796155, "event": "build"}
- `cleanup`: {"administrative_configuration_matched": true, "at_s": 2.095596695318818, "different_keys": [], "event": "cleanup", "foreign_modules_unloaded": false, "new_host_netdevices_requested": 0, "owned_processes_alive": [], "owned_uds_remaining": [], "sysctls_tuned": false}
- `final`: {"at_s": 2.0956523288041353, "event": "final", "passed": false, "wall_s": 2.0956504670903087}

All successful two-VM results prove the transport mechanism only. 16,384 VM identities/endpoints and the current Overdrive policy/observer/lifecycle contract remain NOT VALIDATED.
