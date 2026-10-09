# V-20 — intake-child identity by clone-flagged socket storage (M-1, A-24, D7)

**Verdict: all sub-claims HOLD** on Lima (`7.0.0-34-generic`, aarch64) and metal (`7.0.0-29-generic`, x86_64, `systemd-detect-virt` none): 15 verdicts each, 0 falsified. Not run on the 6.18 pin. Two wording / DELIVER findings (F-1, F-2), neither falsifies M-1.

Probe and logs: `spike-scratch/netns-density-295-v20/` (`ebpf/` aya-rs `sock_ops` `v20_sockops` + `sk_lookup` `v20_steer`; `probe/` with raw `bpf(2)` for storage maps; `run.sh`, `build.sh`, `build-x86.sh`; `out/run-lima-aarch64-full.log`, `out/run-metal-x86_64-full.log`; earlier `out/run-aarch64-*` are failed iterations from probe bugs).

## Model

Netns `v20-host` (root stand-in), `v20-peer` client over veth `10.77.20.0/24`. Guest prefix `100.96.0.0/24` with a tagged `local` route on `lo`, production fwmark policy routing, `tproxy ip to 127.0.0.1:19999` for client source `10.77.20.3`. `sock_ops` on cgroup `v20-owner` holding the whole probe (intake listeners, leg-C listener, foreign listeners, clients — the ADR-0151 shape). Intake listeners bind `workload_addr:20480` (ADR-0178 block), steered by `sk_lookup` keyed `(workload_addr, 8080)`. Clients from `.2` reach the intake, from `.3` reach leg-C, both to `100.96.0.x:8080` — every child of both kinds has local `100.96.0.x:8080` (20,000/20,000). Tag: `u32` intake id in `BPF_MAP_TYPE_SK_STORAGE` created with `BPF_F_NO_PREALLOC | BPF_F_CLONE` (0x201); negative control: same listeners in a map without `BPF_F_CLONE`. Checks: userspace `BPF_MAP_LOOKUP_ELEM` by child fd, and `bpf_sk_storage_get` at `PASSIVE_ESTABLISHED` joined via `SO_COOKIE`; `STATE_CB` re-read.

## Verdicts (identical on both kernels)

| # | Sub-claim | Actual | Verdict |
|---|---|---|---|
| S1 | Every intake child carries its listener's tag at `PASSIVE_ESTABLISHED` | 10,000/10,000 both checks; 1,250 per tag over ids 101–108 | HOLDS |
| S2 | Leg-C children, same local addr:port, same cgroup, same time, carry no tag | 10,000/10,000 untagged | HOLDS |
| S3 | Children that write 5 B and FIN before `accept()` carry the tag | 10,000/10,000; leg-C 2,000/2,000 untagged | HOLDS |
| S4 | Concurrent accepts (4 threads × 9 listeners × 16 clients; 16 accept threads on one listener, 10,000 connects) | 0 mis-tagged, 0 missing | HOLDS |
| S5 | Listener closed with 1,000 children alive: children keep the tag | 1000/1000 before and after; 100/100 still exchange data; `STATE_CB` after peer FIN 1000 tagged | HOLDS |
| S6 | Listener re-created on the same addr:port | old children keep 600 (500/500); new get 601 (500/500) | HOLDS |
| S6b | Tag follows the socket, not addr:port | untagged re-creation → untagged children; entry deleted → no inheritance; re-tag 601→603 applies only to later children | HOLDS |
| S7 | Foreign listener in the cgroup never yields tagged children | loopback 2000/2000 untagged; wildcard host service 1000/1000 untagged | HOLDS |
| S7a | Active sockets in the cgroup never carry the tag | `active_established=20000 tagged=0` | HOLDS |
| S8 | Syncookie path (`tcp_syncookies=2`) | `SyncookiesRecv +2000`, 2000/2000 tagged | HOLDS |
| S9 | TCP Fast Open | `TCPFastOpenPassive +1000`, 1001/1001 tagged | HOLDS |
| S10 | Negative control: entry in a map without `BPF_F_CLONE` | `noclone_tagged=0` of 10,000 | HOLDS (the clone flag causes inheritance) |

```
uname -r: 7.0.0-34-generic    uname -m: aarch64
intake children: children=10000 userspace_lookup ok=10000 bad=0 | sock_ops ok=10000 missing=0 bad=0 | noclone_map_tagged=0
leg-C children:  children=10000 ...  (untagged)
sock_ops counters: passive=20000 passive_tagged=10000 passive_untagged=10000 active=20000 active_tagged=0 noclone_tagged=0 no_sk=0
== SUMMARY: 15 verdicts, 0 falsified
metal: 7.0.0-29-generic, x86_64, systemd-detect-virt none, SUMMARY: 15 verdicts, 0 falsified
```

## Findings

- **F-1 (wording).** `BPF_F_CLONE` is a flag of the **map** (set at creation), not of an entry. Every entry in a clone-flagged map is copied to accepted children; an entry in a map without the flag is not. Design text "a socket-local storage entry created with the clone flag" should read "an entry in a clone-flagged socket-storage map". The owner needs a dedicated clone-flagged map for intake identity; leg-C and every other listener are never stored in it. The tag is a copy taken at child creation: survives the listener's close (S5), unaffected by later re-tagging (S6b), not keyed by addr/port (S6).
- **F-2 (DELIVER tooling).** `SK_STORAGE` needs BTF key/value types; aya 0.13.1 refuses map type 24 unless `EbpfLoader::allow_unsupported_maps()` and creates legacy maps without BTF. The probe creates the maps with `BPF_MAP_CREATE` + a minimal `BPF_BTF_LOAD` blob and pins them; the object declares them legacy with `pinning = ByName` (the HASH_OF_MAPS shape). `aya-ebpf` 0.1.1 has no `SkStorage`; the program calls `bpf_sk_storage_get` via `aya_ebpf::helpers::gen` with `ctx.ops.__bindgen_anon_2.sk`. Userspace tagging is `BPF_MAP_UPDATE_ELEM` keyed by fd; `BPF_MAP_DELETE_ELEM` fails `EINVAL` if attr bytes after `key` are non-zero.
- The ADR-0151 "owner-only cgroup needs a process split" fallback is not triggered.

## Not covered

6.18; IPv6; `SO_REUSEPORT` groups; MPTCP; children still in the accept queue at listener close (kernel resets them). nft rules are a model of PORT-295-C's.

**Gate recommendation:** PROMOTE A-24 / V-20; apply F-1 wording; carry F-2 into DELIVER step 4.
