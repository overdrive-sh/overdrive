# netns-density-295: guest-side vsock capture (V-2, V-3, V-4, V-5, D15)

Throwaway spike. Findings:
`docs/feature/netns-density-295/spike/guest-vsock-capture-findings.md`.

An unmodified program in a real booted guest with no NIC (patched CH,
`--vsock cid=N,backend=vhost-kernel`, stock 7.0.0-29 host and guest) uses
ordinary AF_INET sockets. Guest-kernel Aya carries its traffic over vsock and
host-kernel Aya forwards it. Neither owner copies payload.

- `bpf/` holds the Aya eBPF ELF loaded by both sides (`no_std`, aya-ebpf 0.1.1):
  - cgroup sock_addr hooks, sock_ops, sock_release
  - SK_SKB verdict plus strparser
  - TCX frame decode on `lo`
  - `fexit(skb_send_sock)` drain counter
- `vfwd/` is the userspace crate (aya 0.13.1):
  - `vfwd guest`: the overdrive-init stand-in
  - `vfwd host`: the guest-flow-owner stand-in
  - `vfwd agent`: the guest test agent on vsock 4000
  - litmus and stress tools: plain AF_INET programs with no vsock knowledge
- `guest/init.sh` is guest PID 1. The guest has only `lo` and `dummy0`
  (workload address).
- `harness.py` holds the native-metal build, guest image, VM, owners, servers
  and cleanup. `increment-*/run.py` are the probes, one immutable increment
  each.
- `launch.py <increment>` acquires the canonical lease through
  `cargo xtask metal run`, streams the log with the address redacted, and
  retrieves `evidence/`.
- `rsync-owned-filter.sh` keeps the metal sync away from root-owned evidence
  and `__pycache__`.

| Increment | Role |
|---|---|
| a–f | development and diagnosis; each one found a kernel behaviour that changed the mechanism (findings, "Discoveries") |
| g | first full pre-registered matrix (OOB mode) |
| h | final build: all 10,000-iteration stresses, guest + host strace, K1 negative controls, fail-closed |

The host programs load into the physical host kernel, scoped to owned maps, a
tuple-gated TCX on `lo`, and a cgroup that holds only the owner. Every run
restores modules, links, `lo` TCX, root cgroup BPF and spike cgroups (see
`cleanup` in each `results.json`).
