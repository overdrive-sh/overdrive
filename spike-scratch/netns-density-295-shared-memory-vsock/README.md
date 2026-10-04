# Shared-memory / virtio-vsock probe

Canonical retained code/evidence for the bounded `netns-density-295` PROBE. This is standalone probe code, not production implementation or a test gate. **Final corrected profile: 64 real VM identities, each 96MiB RAM and 0.125 host logical-CPU quota, all 576 VMM threads included. One guest-vCPU topology is separate from that quota. A separate 128-VM/96MiB run without explicit CPU quota also passed. The original 16,384 DESIGN/E18 population remains NOT VALIDATED.**

Each increment has its own `Cargo.toml` with `[workspace]`, exact `src/main.rs`, exact native orchestration `run.py`, and retained evidence. Rust implements the guest AF_VSOCK, guest ordinary TCP wrappers and host Unix backend. Python packages/starts/stops/drives the real VMMs. No production workspace wiring, kernel patch/fork, C/eBPF or external source mirror was created.

## Browse / run index

| Attempt | Exact code | Actual result |
|---|---|---|
| [01](increment-01-native-20261004T0940Z/README.md) | [Rust](increment-01-native-20261004T0940Z/src/main.rs), [runner](increment-01-native-20261004T0940Z/run.py) | Native build passed; canonical path exceeded Unix-socket pathname length before any VMM. |
| [02](increment-02-native-20261004T0951Z/README.md) | [Rust](increment-02-native-20261004T0951Z/src/main.rs), [runner](increment-02-native-20261004T0951Z/run.py) | Two real VMs booted, but stripped guest fixture lacked BusyBox. Children stopped; two owned API lock files remained. |
| [03](increment-03-native-20261004T0949Z/README.md) | [Rust](increment-03-native-20261004T0949Z/src/main.rs), [runner](increment-03-native-20261004T0949Z/run.py) | Exact prior lock files removed; musl ioctl argument-type compile error, no new VM. |
| [04](increment-04-native-20261004T0951Z/README.md) | [Rust](increment-04-native-20261004T0951Z/src/main.rs), [runner](increment-04-native-20261004T0951Z/run.py) | Real two-VM TCP-wrapper echo, inbound ordinary HTTP, CID/port denial and exact cleanup passed. |
| [05](increment-05-native-20261004T0953Z/README.md) | [Rust](increment-05-native-20261004T0953Z/src/main.rs), [runner](increment-05-native-20261004T0953Z/run.py) | Same transport code passed with changed dependencies; a local prep failure had prevented the intended attestation edit. Preserved honestly. |
| [06](increment-06-native-20261004T0954Z/README.md) | [Rust](increment-06-native-20261004T0954Z/src/main.rs), [runner](increment-06-native-20261004T0954Z/run.py) | Two-VM mechanism passed, plus loaded guest executable/module hashes, actual CID ioctl, guest resources, 0700 UDS directory and host PSS attestation. |
| [07](increment-07-ram128-20261004T1011Z/README.md) | [Rust](increment-07-ram128-20261004T1011Z/src/main.rs), [runner](increment-07-ram128-20261004T1011Z/run.py) | Two real VMs/services passed at 128MiB, no explicit CPU quota. |
| [08](increment-08-ram96-20261004T1014Z/README.md) | [Rust](increment-08-ram96-20261004T1014Z/src/main.rs), [runner](increment-08-ram96-20261004T1014Z/run.py) | Two real VMs/services passed at 96MiB, smallest demonstrated working profile. |
| [09](increment-09-ram64-20261004T1017Z/README.md) | [Rust](increment-09-ram64-20261004T1017Z/src/main.rs), [runner](increment-09-ram64-20261004T1017Z/run.py) | 64MiB profile exited before init; VCPU InternalError, cause unproved, exact cleanup. |
| [10](increment-10-ram80-20261004T1019Z/README.md) | [Rust](increment-10-ram80-20261004T1019Z/src/main.rs), [runner](increment-10-ram80-20261004T1019Z/run.py) | 80MiB profile likewise did not reach init/service readiness; not an architectural minimum claim. |
| [11](increment-11-staged128-20261004T1014Z/README.md) | [Rust](increment-11-staged128-20261004T1014Z/src/main.rs), [runner](increment-11-staged128-20261004T1014Z/run.py) | 104 real VMMs spawned / 101 HTTP-verified before a readiness-prefix probe check failed; final console passes all predicates; inference preserved, cleanup matched. |
| [12](increment-12-staged128-complete-marker-20261004T1023Z/README.md) | [Rust](increment-12-staged128-complete-marker-20261004T1023Z/src/main.rs), [runner](increment-12-staged128-complete-marker-20261004T1023Z/run.py) | 128 actual retained VM identities at 96MiB; all services/full-pool recheck passed, no explicit CPU quota. |
| [13](increment-13-quota0125-two-vm-20261004T1029Z/README.md) | [Rust](increment-13-quota0125-two-vm-20261004T1029Z/src/main.rs), [runner](increment-13-quota0125-two-vm-20261004T1029Z/run.py), [CPU owner](increment-13-quota0125-two-vm-20261004T1029Z/quota.py) | CPU setup succeeded but optional cpuset-file readback assumption failed before any VMM; owned parent removed. |
| [14](increment-14-quota0125-staged64-20261004T1032Z/README.md) | [Rust](increment-14-quota0125-staged64-20261004T1032Z/src/main.rs), [runner](increment-14-quota0125-staged64-20261004T1032Z/run.py), [CPU owner](increment-14-quota0125-staged64-20261004T1032Z/quota.py) | 64 actual VMs/services passed at 96MiB and `cpu.max=12500/100000`; all 576 VMM threads, ancestor limits and throttling read back, full-pool recheck/cleanup passed. |

Directory timestamps are nominal preparation labels, not authoritative execution start times. Exact actual UTC lease starts and monotonic event times are in the evidence. Each `launch-executed.py` is a byte-preserved copy of the top-level launcher executed for that attempt; it is an archival source snapshot, not a second entry point.

[Findings](../../docs/feature/netns-density-295/spike/shared-memory-vsock-findings.md), [CH source/ABI audit](source-audit.md), [native hardware budgets](hardware-budget.json), [final post-release PID/path/cgroup/lease attestation](post-release-attestation-04.json), [measured summary](measurement-summary.json), [#303/#306 interaction source metadata](interaction-sources.json).

## Actual native command

Final CPU-corrected epoch 14 was run from the repository root with the existing `.env` target/kernel/rootfs selection:

```sh
python3 spike-scratch/netns-density-295-shared-memory-vsock/launch.py increment-14-quota0125-staged64-20261004T1032Z
```

The launcher called:

```sh
cargo xtask metal run -- python3 spike-scratch/netns-density-295-shared-memory-vsock/increment-14-quota0125-staged64-20261004T1032Z/run.py
```

`launch.py` acquires the canonical exclusive native lease through xtask, with a one-second acquisition timeout, mandatory physical x86_64/KVM/source preflight, and no literal metal target in code/receipts. It refuses to overwrite receipts; `run.py` refuses to reuse an output directory. These recorded commands must **not** be rerun with existing IDs. Any new attempt gets a new increment with source copied before execution.

Runtime targets/builds/images live only under ignored top-level `target/` and `out/`. Linux Unix paths require a shorter task-owned `/run/v295-PID/` directory; only sockets/API locks are placed there, and exact cleanup removes it after the child VMMs exit. The 64MiB baseline guest image is copied per VM, never edited in place. Only existing stock modules are loaded inside newly created guests. No host module unload, global route/nft/BPF cleanup, system-wide sysctl tuning or neighbor flush occurs.

## Evidence preservation

- Each `evidence/` retains source/build/version/fingerprint receipts, actual guest consoles and host receiver bytes, monotonic events, process/FD/memory inventory, raw before/after configuration, and exact cleanup.
- Text artifacts over 16KiB use lossless `.gz` compression, with byte-for-byte verified decompression. Read them with `gzip -dc FILE.gz`. No timer/counter was rewritten to make a complement match.
- The launcher redacts the configured metal target address from shared text only. Original complete native archives remain under ignored `out/` with independent hashes.
- Whole-host captures contain unrelated SSH target headers. Exact raw PCAPs remain under ignored `out/raw-captures/` and in the private archive. Shared `all-interfaces.target-redacted.pcap.gz` files are **explicit redacted derivatives**, replacing only that target IPv4 with `198.18.255.254` and recomputing affected checksums. They are not raw wire evidence or a zero-leakage receipt; AF_PACKET cannot see this vsock channel in any case.
- [preserve_evidence.py](preserve_evidence.py) and [evidence-preservation.json](evidence-preservation.json) retain the exact compression/redaction method, original/derivative/compressed hashes and transformations.
- [audit.py](audit.py) is the read-only final native PID/path/lease audit. Its earlier shell-quoting failure is retained separately as `post-release-attestation-01-failed.json`; the successful second receipt contains all false existence checks.

The user approved “lets lower both so it fits within the machine”. RAM probes selected the smallest demonstrated service-capable stock profile (96MiB). CPU correction uses only newly owned cgroup-v2 parents/children; each VMM enters before exec, all VMM worker threads inherit the quota, and every exact cgroup is removed after owned processes exit. Root/foreign controllers, CPU affinities and cgroups are never modified. The shared host receiver is outside the per-VMM quota and measured separately.

The retained rootfs/kernel/runtime is Linux `7.0.0-29-generic`; this does not attest the pinned 6.18 appliance. CH reports v53.0. Fixture root VMMs/guest init do not prove the production confinement, TCX/TPROXY/kTLS/splice, DNS/Service registry, zero-unmarked/frame observer, activation or restart contract. No 16,384 / 100,000 VM capacity or promotion is inferred.

Combined #303/#306 module/kTLS + this transport/resource profile: **NOT RUN**. The findings include a bounded bidirectional compatibility/ownership/capture/key/resource assessment, without implementing the proposed module or accepting its design. The initial hour was extended only for explicit resource/count/CPU corrections and this requested assessment.

The user explicitly excluded testing the combined guest-mTLS path here. The #303/#306 assessment is documentary only; NOT RUN is the intended scope boundary, not a pending gate for this probe.
