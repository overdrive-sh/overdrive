# Probe increment 02

Two actual VMs booted. The stripped fixture lacked the requested BusyBox binary, so the guest stopped before vsock use. Exact children exited; two API lock files remained and were witnessed/removed in attempt 03. No successful transport verdict.

Exact sources: `Cargo.toml`, `Cargo.lock`, `src/main.rs`, `run.py`; the top-level launcher snapshot is `launch-executed.py`. Sources were retained unchanged after actual native execution. Evidence is under `evidence/`; compressed files have `.gz` suffix.

Native lease receipt:

```text
pid=792392
started_at=2026-10-04T09:47:11Z
action=run
scenario=spike-vsock-increment-02-native-20261004T0951Z
workspace=/Users/marcus/conductor/workspaces/helios/wellington-v2
commit=5ea2d0c506f5c1f0672c2a4b5b7c5d0d415d6bde
token=a350f6b34b844fc3ac213e33
```

## Mechanical outcomes

- `build`: {"at_s": 0.7955009546130896, "binary_bytes": 765352, "binary_sha256": "907ba98c94938b34dceae677e3af56a0a185f15a4ddfa6767b5716e716b5f6f0", "elapsed_s": 0.5715155443176627, "event": "build"}

All successful two-VM results prove the transport mechanism only. 16,384 VM identities/endpoints and the current Overdrive policy/observer/lifecycle contract remain NOT VALIDATED.
