# Probe increment 03

Exact two prior owned API lock files were removed. New source failed to compile at musl ioctl request casts before any new VM; complement matched.

Exact sources: `Cargo.toml`, `Cargo.lock`, `src/main.rs`, `run.py`; the top-level launcher snapshot is `launch-executed.py`. Sources were retained unchanged after actual native execution. Evidence is under `evidence/`; compressed files have `.gz` suffix.

Native lease receipt:

```text
pid=793331
started_at=2026-10-04T09:49:37Z
action=run
scenario=spike-vsock-increment-03-native-20261004T0949Z
workspace=/Users/marcus/conductor/workspaces/helios/wellington-v2
commit=5ea2d0c506f5c1f0672c2a4b5b7c5d0d415d6bde
token=cff4259e1b45afa0a403965f
```

## Mechanical outcomes

- `cleanup`: {"administrative_configuration_matched": true, "at_s": 0.5143544273450971, "different_keys": [], "event": "cleanup", "foreign_modules_unloaded": false, "new_host_netdevices_requested": 0, "owned_processes_alive": [], "owned_uds_remaining": [], "sysctls_tuned": false}
- `final`: {"at_s": 0.5143882185220718, "event": "final", "passed": false, "wall_s": 0.5143870264291763}

All successful two-VM results prove the transport mechanism only. 16,384 VM identities/endpoints and the current Overdrive policy/observer/lifecycle contract remain NOT VALIDATED.
