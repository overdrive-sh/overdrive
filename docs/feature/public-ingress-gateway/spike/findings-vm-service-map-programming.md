# D4 — VM Service backend is programmed into the ordinary eBPF Service map

Verdict: **WORKS** (hypothesis holds). Decision D1 is not voided by this probe.

- Date: 2026-11-01. Tree synced: `549d78fa65ebe5a9259fd9adcf6c0d7b694cf7f8` (clean tree plus the untracked
  `spike-scratch/pig-d4-service-map/`). No file under `crates/` was touched.
- Substrate: native metal `ubuntu@151.115.99.251`, `uname -r` = `7.0.0-38-generic`, x86_64,
  `systemd-detect-virt` = `none`, `/dev/kvm` present (every dump below prints `uname=7.0.0-38-generic`).
- Runs: `cargo xtask metal run` run ids `fcf596a695d6f5ffe25ff856` (first full pass) and
  `7e4dd4b3fe848fae3b816c1f` (final pass, adds the +10s/+23s after-stop observations). Raw logs:
  `spike-scratch/pig-d4-service-map/out/run10.log`, `run12.log` (gitignored).

## Hypothesis / prediction / falsification

- Hypothesis: a VM Service deployed through a real `overdrive serve` + `overdrive deploy <SPEC>` has its backend
  (guest address from `100.95.0.0/16`) programmed into the ordinary Service map by `ServiceMapHydrator` ->
  `Action::DataplaneUpdateService` -> `EbpfDataplane::update_service`, with no gateway-specific demand/TEACH machinery.
- Prediction: once the Service is Running+healthy, SERVICE_MAP has an entry for VIP:port, the inner Maglev map holds
  backend id(s), BACKEND_MAP carries the `100.95.x.y` address and declared port, REVERSE_NAT_MAP has the matching
  entry, and LOCAL_BACKEND_MAP does not contain the VM backend.
- Falsification: Service Running but SERVICE_MAP/BACKEND_MAP carry no entry for it, or the backend lands only in
  LOCAL_BACKEND_MAP, or is excluded.
- Outcome: every part of the prediction held; falsification not observed.

## Source anchors re-verified on the current tree

- `crates/overdrive-core/src/guest_transport/mod.rs:594`: `GUEST_NODE_PREFIX = 100.95.0.0/16`.
- `crates/overdrive-control-plane/src/lib.rs:5427-5438`: `service_map_hydrator(host_ipv4)` builds
  `ServiceMapHydrator::canonical(host_ipv4, WORKLOAD_SUBNET_BASE)`. `WORKLOAD_SUBNET_BASE` is the `10.99.0.0/16`
  Path-A/mesh subnet.
- `crates/overdrive-reconcilers/src/service_map_hydrator.rs:336-414`: the gate drops only backends inside
  `workload_subnet` (`is_mesh_backend`, `:345-348`); the rest partition into LOCAL (`== host_ipv4`) and REMOTE;
  only `remote_survivors` go into `Action::DataplaneUpdateService` (`:414`). A `100.95.x.y` guest address is outside
  `10.99.0.0/16` and not equal to `host_ipv4`, so it is a REMOTE survivor. (Source shows this; the dump confirms it.)
- `crates/overdrive-dataplane/src/lib.rs:1771`: `update_service(frontend, backends)`; empty set purges that
  `(vip, port, proto)`. Wire layouts: `ServiceKey` (`vip_host:u32, port_host:u16, proto:u8, pad:u8`, host-order,
  `maps/mod.rs:52-60`) and `BackendEntryPod` (`ipv4_host:u32, port_host:u16, weight:u16, healthy:u8, pad[3]`,
  `maps/mod.rs:108-119`).

## What had to be done to get a VM Service to Running (honest disclosure)

The checked-in E08 example (`examples/service-kind-vm-workloads/run-example.sh run healthy`) does not reach Running
on this host as shipped. Four environment/fixture gaps were bridged in a throwaway copy of the runner
(`spike-scratch/pig-d4-service-map/run-d4.sh`); none touches the code path under test (serve -> deploy -> hydrator ->
dataplane):

1. Metal run needs `OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel` and `OVERDRIVE_METAL_ROOTFS=...` (passed
   in the process env; `.env` unchanged) and a prior `cargo xtask bpf-build` (BPF object was stale) and
   `cargo xtask guest-kit-prepare` (builds `ovd_fwd.ko` and the static `overdrive-guest-peer`).
2. Host state: the root cgroup's `subtree_control` lacked `io`, so `serve` failed
   `cgroup subtree_control write failed: No such file or directory`. I ran `echo +io > /sys/fs/cgroup/cgroup.subtree_control`
   on the host (runtime-only, not persistent; output in `out/host-cgroup-io-enable.txt`). Also `ulimit -n 1048576`
   (guest-network probe refused: `file-descriptor budget 376832 exceeds RLIMIT_NOFILE 1024`) and `umask 022`
   (the metal runner's default umask made `data/vm` `0750 root:root`, so the confined VMM uid/gid 4200 got
   `Permission denied` opening its rootfs clone).
3. Guest kernel is the stock Ubuntu `7.0.0-38` kernel (`CONFIG_DUMMY=m`, `CONFIG_VSOCKETS=m`). Production
   `overdrive-init` as `/sbin/init` refused: `guest transport setup failed: creating the transport dummy device failed:
   Not supported (os error 95)`. The guest-kit layout was reproduced in the prepared rootfs: `overdrive-guest-peer` as
   PID 1 (loads `dummy.ko`, diag modules, then execs `/lib/overdrive/overdrive-init`), plus `ovd_fwd.ko` at
   `/lib/overdrive/`. This is the same mechanism the guest-kit Tier-3 lanes use (`crates/overdrive-testing/bin/guest_peer.rs:1260-1330`).
4. The example guest workload (`guest_server.rs`) panics (exit 101) on `write_all(...).expect(..)` when a platform
   probe connects and drops, so the alloc crash-looped ~1.1 s after READY. I recompiled a copy with those writes
   made non-fatal (`guest_server_tolerant.rs`, diff = `.expect(..)` -> `.unwrap_or(..)` on four write sites and one
   read) and installed it at the same guest path. Unrelated to the hypothesis, but a real defect of the checked-in
   example against current probe behaviour.

## Evidence (final pass, run 7e4dd4b3fe848fae3b816c1f unless stated)

Deploy (real CLI through a PTY): `Service 'service-vm-e08' is stable (settled in 2410ms; witness: startup probe[0] (tcp 0.0.0.0:18081))`.

`workload describe` while Running (run10):

```
Service 'service-vm-e08' (kind: Service)
Replicas (desired/running): 1/1
alloc-service-vm-e08-0   Running      0          (c=10,w=local)
    terminal: Stable
Addresses:
  alloc-service-vm-e08-0: 100.95.0.2
VIP:           10.96.0.2
Listeners:
  18080/tcp
  18081/tcp
Probes:
  startup probe[0] tcp 0.0.0.0:18081 last=pass
  readiness probe[0] http GET http://0.0.0.0:18080/ready last=pass
```

`bpftool map show` (all maps enumerated, per debugging.md section 11; bpftool truncates names to 15 chars so
`LOCAL_BACKEND_MAP` prints as `LOCAL_BACKEND_M`):

```
508: hash_of_maps  name SERVICE_MAP     key 8B value 4B max_entries 4096
512: hash          name REVERSE_NAT_MAP key 8B value 8B max_entries 1048576
514: hash          name BACKEND_MAP     key 4B value 12B max_entries 65536
516: hash          name LOCAL_BACKEND_M key 8B value 8B max_entries 4096
521: array (inner Maglev)  key 4B value 4B max_entries 16381
522: array (inner Maglev)  key 4B value 4B max_entries 16381
```

Before deploy (positive control, same serve, no workload): `SERVICE_MAP`, `BACKEND_MAP`, `REVERSE_NAT_MAP`,
`LOCAL_BACKEND_M` each `Found 0 elements`; only the prototype inner array (all-zero) exists.

Immediately after Stable / while Running (run10, `dump_maps running`):

```
--- dump name SERVICE_MAP
key: 02 00 60 0a a0 46 06 00  inner_map_id: 521
key: 02 00 60 0a a1 46 06 00  inner_map_id: 522
--- dump name BACKEND_MAP
key: 01 00 00 00  value: 02 00 5f 64 a0 46 01 00  01 00 00 00
key: 02 00 00 00  value: 02 00 5f 64 a1 46 01 00  01 00 00 00
--- dump name REVERSE_NAT_MAP
key: 02 00 5f 64 a0 46 06 00  value: 02 00 60 0a a0 46 00 00
key: 02 00 5f 64 a1 46 06 00  value: 02 00 60 0a a1 46 00 00
--- dump name LOCAL_BACKEND_M
Found 0 elements
--- dump id 521 (inner)   key: 00..  value: 01 00 00 00   (slots 0x00-0xa1 shown all = backend id 1)
--- dump id 522 (inner)   key: 00..  value: 02 00 00 00   (slots shown all = backend id 2)
```

Decode (little-endian bytes of host-order fields):

- SERVICE_MAP key `02 00 60 0a | a0 46 | 06 | 00` = vip `0x0a600002` = **10.96.0.2**, port `0x46a0` = **18080**, proto 6 (TCP);
  second key port `0x46a1` = **18081**. These are the Service's declared listeners and VIP.
- BACKEND_MAP value `02 00 5f 64 | a0 46 | 01 00 | 01 | 00 00 00` = ipv4 `0x645f0002` = **100.95.0.2** (matches describe
  `Addresses`), port **18080**, weight 1, healthy 1. Key 2 is the same address with port **18081**.
- REVERSE_NAT_MAP key = backend `100.95.0.2:18080/tcp` -> value = VIP `10.96.0.2:18080`; same for 18081.
- Inner maps 521/522 hold backend ids 1 and 2 in every slot printed (the dump was cut at 6000 bytes by the script's
  `head -c`; the full slot count was not captured, only that the visible slots all carry the backend id).
- LOCAL_BACKEND_M: 0 elements in every dump: the VM backend is not in the host-local path.

## Edge cases observed

- **healthy byte lags readiness.** In the final pass the dump taken the instant the deploy returned Stable shows
  BACKEND_MAP value `... 01 00 | 00 | 00 00 00` (healthy = 0); about 1 s later, with readiness `pass`, it reads
  `... 01 00 | 01 | 00 00 00`. The entry exists from startup-Stable; the healthy bit follows readiness (run12 lines 146-147 vs the
  `running` dump).
- **Entries persisted after stop.** After `overdrive job stop` (alloc `Terminated`, reason `stopped`, replicas 1/0) the
  SERVICE_MAP, BACKEND_MAP and REVERSE_NAT_MAP entries were still present, healthy = 1, at +5 s, +10 s and +23 s
  (run12 `=== BRIEF after-stop` blocks). I did not investigate why (not retracted by the hydrator, stop path, or
  something else). This probe's claim is only about programming, but a consumer must not read map presence as
  liveness.
- Two listeners produce two outer entries and two backends (one per `(vip, port)`), each with its own inner map.
- A restart of the alloc produces a new guest address (100.95.0.3, .4, ...) in the crash-loop run (run9
  `Addresses`); the lease changes per alloc. Not exercised further.
- Teardown clean: `E08 teardown deltas: vm=0 probe=0 network=0 cgroup=0 run-directory=0 mount=0 loop=0 preparation=0`,
  no `cloud-hypervisor`/`serve` processes left, no `ovd-*` links, no mounts (checked over ssh afterwards). An earlier,
  failed pass (run7) leaked two `ovd-veth-*` links which the runner's later passes did not report; none remain.
- Host changes left behind: root cgroup `subtree_control` now includes `io` (runtime-only). Staged `target/` build output
  and `target/kmod`, `target/x86_64-unknown-linux-musl` under `/home/ubuntu/overdrive` (synced tree).

## Design implication for D1

On current main, the VM backend reaches `SERVICE_MAP`/`BACKEND_MAP`/`REVERSE_NAT_MAP` through the ordinary hydrator
path with no gateway involvement. The prerequisite for retiring the branch demand-gated TEACH decision (branch ADR-0133; retired, see ADR-0242) and the TEACH half of the packet-entry decision (now ADR-0224) for VM
backends holds: ordinary Service-map programming covers a VM backend, keyed on the Service VIP:port, with the
`100.95.x.y` address and declared port. What the probe does not show: that a gateway upstream path actually delivers
traffic via these entries (no packet was sent through the VIP here), and any behaviour on guest-address change, stop
retraction, or multi-replica sets. D2's bounded wait / typed-unavailable outcome at the gateway's consuming port should
key on the Service's healthy bit/readiness, not on map-entry presence (see stop edge case).

## Gate recommendation

PASS: D1 stands (retire gateway demand/TEACH for VM backends); carry the stale-after-stop and healthy-lag observations
into the DESIGN as inputs to D2, and have the architect decide whether the stale-after-stop behaviour needs its own
check before the gateway relies on the map.
