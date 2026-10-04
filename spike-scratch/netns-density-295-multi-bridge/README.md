# Multi-bridge probe code

Canonical repository source for the user-requested multi-bridge PROBE. This is throwaway evidence code, not production code or a test gate. The project `.claude/rules/spike.md` places probe source in `spike-scratch/`; the user's location correction supersedes the earlier dispatch's `/tmp` choice.

- `probe.py`: actual TAP owner, stock root/leaf bridge setup, kernel readbacks, primitive packet checks, real-VM orchestration, neighbor-pressure check and exact cleanup.
- `guest.rs`: the actual credential-free Rust guest fixture used in epoch 03. It configures the existing `/16` and gateway, then exercises peer/gateway/service TCP and gateway DNS.
- `launch.py`: canonical `cargo xtask metal run` lease/preflight launcher, target-redacted receipts and archive retrieval.
- Each uniquely named `increment-*` directory: exact executed source plus that attempt's raw readbacks/logs/captures, with a provenance README. Original snapshots remain in `versions/epoch-*`. `versions/launch-before-location-correction.py` preserves the latest pre-relocation runner; early wrapper bytes were not separately attested for every epoch, and exact executed commands remain in native logs.

The native epochs executed the original sources at remote `/tmp/spike_netns_density_295_multi_bridge/` before the user's correction. Those original scratch files and native evidence remain retained. Current repository source has only source/output-path/launcher corrections: it executes the synced repository file and writes runtime output under ignored `out/`. No further native experiment ran after the correction, and these location edits are not represented as a new measured source version.

Large raw readback/capture files use lossless `.gz` compression for the review/commit tree. Exact uncompressed bytes remain in ignored `out/uncompressed-evidence/` and original `.context` archives. `receipts/evidence-preservation.json` records raw/compressed hashes and a verified byte-for-byte round trip. Small logs and event streams remain directly readable. Inspect compressed text with `gzip -dc FILE.gz`; inspect the pcap with `gzip -dc FILE.pcap.gz | tcpdump -n -r -`. No observation or volatile field was rewritten.

## Exact commands

From the repository root, with existing `.env` metal target/kernel/rootfs configuration:

```sh
# Attachment and primitive-packet profile (actual epoch 02 behavior).
SPIKE_MULTI_BRIDGE_ATTEMPT=05 SPIKE_MULTI_BRIDGE_MODE=primitive python3 spike-scratch/netns-density-295-multi-bridge/launch.py

# Two-real-VM profile (actual epoch 03 behavior).
SPIKE_MULTI_BRIDGE_ATTEMPT=06 SPIKE_MULTI_BRIDGE_MODE=vm python3 spike-scratch/netns-density-295-multi-bridge/launch.py

# Untuned neighbor-pressure profile (actual epoch 04 behavior; failed at source 342).
SPIKE_MULTI_BRIDGE_ATTEMPT=07 SPIKE_MULTI_BRIDGE_MODE=pressure python3 spike-scratch/netns-density-295-multi-bridge/launch.py
```

Equivalent canonical launch for a selected profile:

```sh
cargo xtask metal run -- python3 spike-scratch/netns-density-295-multi-bridge/probe.py primitive
```

The example attempt IDs are fresh IDs; existing receipts are never overwritten. These commands are reproducibility instructions, not an instruction to rerun now. The orchestrator directed no more native mutations after epoch 04. Pressure uses global kernel neighbor accounting and can induce normal global cache GC; preservation of foreign dynamic neighbor contents was not proved. Do not tune/flush/reconstruct that state from an unknown baseline.

## Recorded epochs

| Epoch | Actual outcome |
|---|---|
| [01](increment-01-native-20261004T085711Z/README.md) | 4097 actual TAPs/queue FDs; then probe-only `select` FD-range and duplicate logging-keyword errors. Preserved. |
| [02](increment-02-native-20261004T090250Z/README.md) | 4097 actual TAPs/queue FDs; cross-leaf ARP/unicast, gateway and off-subnet UDP passed; exact cleanup completed. |
| [03](increment-03-native-20261004T090824Z/README.md) | 1024 actual TAPs and two real CH guests; peer/gateway/off-subnet TCP and gateway DNS passed. Fixture plaintext and root VMMs; no production policy acceptance. |
| [04](increment-04-native-20261004T091503Z/README.md) | 1100 actual TAPs; gateway replies stopped at the 342nd distinct ARP source; 1023 visible probe neighbors and native `table_fulls 0→9`. |

The launcher acquires the canonical exclusive lease with a one-second acquisition timeout. The probe creates one isolated node network domain, no per-VM namespaces, and deletes only recorded probe links/stops exact child processes. No global route/nft/BPF purge, module unload, broad kill, system-wide sysctl change or production edits are performed. `vm` requires stock Cloud Hypervisor, selected guest artifacts, Rust's installed `x86_64-unknown-linux-musl` target, `debugfs`, and tcpdump. Root guest fixtures do not establish the production confinement, TCX, TPROXY, TLS/kTLS/splice, zero-frame or lifecycle contracts.

Findings: `docs/feature/netns-density-295/spike/multi-bridge-findings.md`. Native logs/archives and final attestation: `.context/spike-multi-bridge-*`.
