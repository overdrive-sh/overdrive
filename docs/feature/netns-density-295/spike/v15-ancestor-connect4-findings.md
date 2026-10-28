# V-15 / A-19 findings: ancestor `connect4` inheritance

## Verdict

**WORKS.** The native kernel in the disposable Cloud Hypervisor guest applied
the Aya `vip_connect4` program attached at
`/sys/fs/cgroup/overdrive.slice` to sockets created after the process moved to
`/sys/fs/cgroup/overdrive.slice/control-plane.slice`.

The successful raw capture is [increment-f evidence](../../../spike-scratch/netns-density-295-ancestor-connect4/increment-f/evidence/).
The prior increments are retained as probe history: increment-a records the
initial vfwd compile failure, increments b-e record corrected fixture attempts,
and increment-f is the canonical successful capture after the launch retrieval
path and guest-owner handling were corrected.

## Substrate and command

The launcher recorded the canonical Metal run
`OVERDRIVE_METAL_RUN_ID=8493f2bedb4b40c8c0618762` with terminal exit `0`.
The native preflight was x86_64, `systemd-detect-virt=none`,
`/dev/kvm` usable, and the host/guest kernel readback was
`7.0.0-38-generic`. Cloud Hypervisor was the vendored
`overdrive/vhost-kernel-vsock` checkout at
`f252bb20b0d141d59e4c15a9ac09c5c2dd7c350d`.
The BPF and vfwd builds both returned `0`; exact artifact and source hashes
are in `artifact-pins.json` and `source-manifest.json`.

The executed guest command was:

```text
/vfwd ancestor-vip --obj /vfwd-bpf.o
```

The command's raw JSON output was retained in
`increment-f/evidence/results.json`. Its decisive fields were:

```text
ancestor=/sys/fs/cgroup/overdrive.slice
owner=/sys/fs/cgroup/overdrive.slice/control-plane.slice
owner_cgroup=0::/overdrive.slice/control-plane.slice
tcp_backend=127.0.0.1:37083
tcp_getpeername=127.0.0.1:37083
udp_backend=127.0.0.1:55521
udp_getpeername=127.0.0.1:55521
program=vip_connect4(overdrive.slice), verified_insns=38
```

The complete litmus line captured by `guest-commands.jsonl` was:

```json
{"ancestor":"/sys/fs/cgroup/overdrive.slice","cgroup_listing":"ID       AttachType      AttachFlags     Name           \n8        cgroup_inet4_connect multi           vip_connect4                   \n","controls":{"direct_peer_is_backend":true,"outside_subtree_refused":true,"tcp_vip_peer_is_backend":true,"udp_vip_peer_is_backend":true,"unmapped_vip_refused":true},"direct_getpeername":"127.0.0.1:37083","kernel":"7.0.0-38-generic","ok":true,"outside":"/sys/fs/cgroup/ancestor-connect4-outside-124","outside_cgroup":"0::/ancestor-connect4-outside-124","outside_mapped_vip_error":"Network is unreachable (os error 101)","owner":"/sys/fs/cgroup/overdrive.slice/control-plane.slice","owner_cgroup":"0::/overdrive.slice/control-plane.slice","program":{"id":8,"name":"vip_connect4(overdrive.slice)","tag":"d6ec622dab236cf1","verified_insns":38},"tcp_backend":"127.0.0.1:37083","tcp_getpeername":"127.0.0.1:37083","udp_backend":"127.0.0.1:55521","udp_getpeername":"127.0.0.1:55521","unmapped_vip_error":"Network is unreachable (os error 101)"}
```

## Controls and cleanup

- Positive TCP: the child-cgroup socket connected to the mapped VIP and
  `getpeername()` returned the mapped backend `127.0.0.1:37083`; the echo
  completed.
- Positive UDP: the child-cgroup connected UDP socket's `getpeername()`
  returned `127.0.0.1:55521`; the echo completed.
- Non-VIP control: a direct child-cgroup connect returned the backend tuple
  without a rewrite.
- Unmapped VIP control: `10.251.0.11:7200` returned
  `Network is unreachable (os error 101)`.
- Outside-subtree control: after moving the same process to
  `/sys/fs/cgroup/ancestor-connect4-outside-124`, the mapped VIP returned
  `Network is unreachable (os error 101)`.
- `bpftool cgroup show /sys/fs/cgroup/overdrive.slice` reported the live
  `cgroup_inet4_connect` attachment named `vip_connect4`.
- Host inventory before/after recorded unchanged root cgroup BPF, links, routes,
  netns, TCX state, module set, and zero remaining spike cgroups. The physical
  host was never used as the probe's cgroup substrate; all cgroup/BPF effects
  were inside the disposable guest.

This discharges A-19/V-15 for the D24 placement premise. The old V-11 probe's
owner-only attachment remains retained as historical evidence and is not used
as proof of ancestor inheritance.
