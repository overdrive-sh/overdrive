# V-LEGB-MARK — leg-B marks and source-binds same-node VM dials; no route covers the guest prefix (Linux 7.0)

Probe increment: `spike-scratch/netns-density-295-v33-legb-mark/` (sources, `run.sh`, `launch.py`/`s8.sh`, raw transcripts under `evidence/`). Kernel citations are `vendors/linux` at tag `v7.0`. Research: `docs/research/networking/guest-prefix-enetunreach-alternatives-research.md`. Recorded by the orchestrator from the spike agent's report (the agent's Write tool was refused).

## Verdict

**WORKS.** For a dial whose destination is inside `GUEST_NODE_PREFIX`, leg-B sets `SO_MARK = MTLS_TPROXY_MARK (0x1)` and `bind(127.0.0.1:0)` before `connect()`. With no default route and no route covering the prefix (only the gateway `/32` local route), the dial reaches the TPROXY leg-C listener, which sees `peer=127.0.0.1:<eph>`, `local=100.95.0.7:8080`. The `fwmark 0x1 -> table 100` rule, the output tuple-set divert, TPROXY and the constant reject/drop rules were not touched.

| # | Probe | Verdict | Result |
|---|---|---|---|
| S1 | marked+bound, no default, no prefix route | WORKS (Lima aarch64 7.0.0-38, metal x86_64 7.0.0-29) | round trip; `peer=127.0.0.1:38052 local=100.95.0.7:8080`; counters 0 |
| S2 | same, default route via `up0` | WORKS (both) | 0 IPv4 packets on `up0` (positive control: S5 saw its 2 SYNs there) |
| S3 | marked, tuple not in set | WORKS (both) | TCP `ECONNREFUSED`, UDP `EPERM`; decoys `:8080`/`:53` silent; counters 2/2 |
| S4 | unmarked / mark-only / bind-only | WORKS, two predictions corrected | both halves load-bearing; mark-only `ENETUNREACH`; bind-only `ENETUNREACH`/`EINVAL`; unmarked+default route+in-set tuple still reaches leg-C (today's behaviour) |
| S5 | non-prefix dial | WORKS (both) | `setup=[]`; SYN on `up0`; no default route -> `ENETUNREACH` as before |
| S6 | `SO_BINDTODEVICE=lo` | WORKS only with `bind(127.0.0.1)`; DOESN'T-WORK without | without bind the source is the node address `192.0.2.10` and the handshake times out |
| S7 | production `ip rule` order | WORKS | production client installs `32765: fwmark 0x1 lookup 100` ahead of `32766 main` |
| S8 | real serve+deploy, real microVM | NOT RUN as specified; equivalent WORKS | real CH microVM behind a leg-C stand-in over vsock answered |

## Open questions

1. `GUEST_NODE_PREFIX` is **not** per-node: one const `100.95.0.0/16`, gateway `100.95.0.1` (`crates/overdrive-core/src/guest_transport/mod.rs:382,386`), used to build each node's `GuestAddressPool` (`crates/overdrive-control-plane/src/lib.rs:7769`). No ADR read (0120, 0125, 0145, 0152, 0173, 0185) says how a client node's leg-B addresses a remote node's VM. The discriminator selects exactly the same-node dials only if remote VM backends are never dialled by a prefix address. DESIGN must pin this.
2. The table-100 route accepts `src 127.0.0.1` (`ip route replace local 0.0.0.0/0 dev lo table 100 src 127.0.0.1`, exit 0). A marked unbound dial then connects from 127.0.0.1 (`peer=127.0.0.1:40528`), making the bind redundant. Cost: `add_local_route` has no `src` parameter (new netlink surface) and table 100 is shared by every `fwmark 1` lookup. Recommend the bind.
3. The pre-marked connect and the diverted packet take the same route: `ip route get 100.95.0.7 mark 1 from 127.0.0.1` and `... from 192.0.2.10` both `local ... dev lo table 100`; the SYNs `127.0.0.1.34318 > 100.95.0.7.8080` and `192.0.2.10.33896 > 100.95.0.7.8080` both cross `lo` and are accepted by leg-C with the same `local`. By source, the nft route chain re-routes only if saddr/daddr/mark/tos changed (`net/netfilter/nft_chain_route.c:37-48`), so an already-marked packet gets no second lookup. The reroute call was not traced.

## Environment

| | Lima | Metal |
|---|---|---|
| Kernel | `7.0.0-38-generic` aarch64 | `7.0.0-29-generic` x86_64, `systemd-detect-virt`=none, AMD EPYC 8024P |
| Run | `cargo xtask lima run --`, root, netns `v33ns` | canonical lease via `launch.py` (no rsync), root, `/tmp/v33-work`, netns `v33ns` |
| iproute2/nft | 6.19.0 / v1.1.6 | not recorded |
| sysctls | route_localnet=0, rp_filter=2 | route_localnet=0 (pinned), rp_filter=0 (inherited) |
| VMM | n/a | Cloud Hypervisor v53.0.0 fork binary sha256 `eadddf1bbcdc...`; kernel sha256 `b51367c7dab2...` |

**Sysctl correction.** A new netns inherits init_net (`devconf_inherit_init_net=0`); the metal root netns has `all.route_localnet=1`, which removed K10's `EINVAL` (bound-only unmarked dial with default route succeeded; `SO_BINDTODEVICE=lo` alone worked). Production never sets `route_localnet` (grep of crates/xtask/infra), so the appliance value is 0. That run is kept as `evidence/run-20261013T144820Z-superseded-route_localnet1-inherited/` and not used. Verdicts use the pinned 0, printed atop each transcript. Two earlier harness runs (tcpdump PID bug; capture stopped before the ring buffer was read) are not used.

## What mirrors production and what does not

- nft: table `ip overdrive-mtls`; `prerouting` filter and `output` route chains at priority mangle; set `inbound_destinations` (`ipv4_addr . inet_service`); leg-S exemption `mark 0x2 accept` at head; TPROXY/divert rules on set members (`crates/overdrive-netlink/src/nft.rs:731-811,4100-4125`). `crates/overdrive-dataplane/src/mtls/nft.rs` does not exist; the install is `overdrive-worker/src/mtls_intercept.rs`.
- Not today's tree: it still has `managed_guest_ips` drops. ADR-0172's end state was used (constant prefix reject tcp-reset/plain in `output` after the divert, drop in `prerouting`, gateway excluded per GW-LOCAL), loaded with the `nft` CLI.
- FIB rule and table 100: Lima used the production client calls `Client::add_fib_rule_fwmark(0x1,100)` and `add_local_route(100,"lo")` via a probe linking `overdrive-netlink` by path; metal used `ip rule add fwmark 0x1 lookup 100` (identical resulting order).
- Gateway `ip route add local 100.95.0.1/32 dev lo proto 247`; no other route covers the prefix.
- leg-C is a Rust `IP_TRANSPARENT` listener, not production leg-C. `v33 dial --leg-b` implements only the proposed contract; `dial_leg` (`crates/overdrive-dataplane/src/mtls/mod.rs:647`) is unchanged. Probe sets `IP_BIND_ADDRESS_NO_PORT`.

## Kernel facts (`vendors/linux` @ v7.0)

- Connect-time lookup uses the socket's mark/source/oif: `include/net/route.h:319-335`; two-step source-choosing lookup `:337-356`.
- `RTN_LOCAL` sets `saddr` only `if (!fl4->saddr)` (prefsrc, else destination): `net/ipv4/route.c:2848-2868`.
- A non-local source without `FLOWI_FLAG_ANYSRC` fails: `__ip_dev_find` -> `goto out`, `rth = ERR_PTR(-ENETUNREACH)` (`route.c:2729,2768`). This explains S4(c): ADR-0188 predicted a connect from the VM's own address; the connect itself fails.
- Loopback source on a non-loopback device is `EINVAL` unless `route_localnet`: `route.c:2578-2582`.
- On-link fallback after a failed lookup with `oif`: `route.c:2818-2843`; `inet_select_addr(dev_out,0,RT_SCOPE_LINK)` for `lo` yields a node address (S6x). Bound-device nexthop filter: `net/ipv4/fib_trie.c:1413`.
- Rule added without priority: `pref = second rule's pref - 1` (`net/core/fib_rules.c:83-98`, used at `:745`) = 32765.
- `ip_route_me_harder` uses `skb->mark` and `sk->sk_bound_dev_if`: `net/ipv4/netfilter.c:21-53`.

## Raw results (unedited extracts)

**S1** — hypothesis: lookup matches fwmark->table 100; bound source skips dest-as-source. Prediction: round trip, `peer=127.0.0.1`, `local=100.95.0.7:8080`, counters 0. Falsification: `ENETUNREACH`/`EINVAL`, other pair, counter >0.
```
32765:	from all fwmark 0x1 lookup 100          (ip rule show; 0 local, 32766 main, 32767 default)
local default dev lo table 100 proto static scope host
local 100.95.0.1 dev lo table local proto 247 scope host      (no route covers 100.95.0.0/16)
ip route get 100.95.0.7 mark 1 from 127.0.0.1 -> local 100.95.0.7 from 127.0.0.1 dev lo table 100 mark 1 uid 0 / cache <local>
ip route get 100.95.0.7                       -> RTNETLINK answers: Network is unreachable
v33 dial --ip 100.95.0.7 --port 8080 --mark 1 --bind 127.0.0.1
RESULT ok client-local=127.0.0.1:38052 client-peer=100.95.0.7:8080 reply="RESP:REQ-100.95.0.7:8080" elapsed_ms=0.2 setup=[mark=1 bind=127.0.0.1]
v33 dial --ip 100.95.0.7 --port 8080 --leg-b   -> RESULT ok client-local=127.0.0.1:38064 ...
ACCEPT peer=127.0.0.1:38052 local=100.95.0.7:8080 req="REQ-100.95.0.7:8080"
ACCEPT peer=127.0.0.1:38064 local=100.95.0.7:8080 req="REQ-100.95.0.7:8080"
counters: drop 0, reject-with-tcp-reset 0, reject 0
```
`lo` capture: `IP 127.0.0.1.42298 > 100.95.0.7.8080 [S]`, `IP 100.95.0.7.8080 > 127.0.0.1.42298 [S.]`, data, FIN. Metal same (`ACCEPT peer=127.0.0.1:58076`). As predicted.

**S2** — default route present.
```
default via 192.0.2.1 dev up0
ip route get 100.95.0.7 mark 1 from 127.0.0.1 -> local ... dev lo table 100
ip route get 100.95.0.7                       -> 100.95.0.7 via 192.0.2.1 dev up0 src 192.0.2.10
RESULT ok client-local=127.0.0.1:35370 client-peer=100.95.0.7:8080 ...   (x2)
capture up0: count=0
```
Same on metal. As predicted.

**S3** — decoys `0.0.0.0:8080/tcp`, `0.0.0.0:53/udp` present.
```
--ip 100.95.0.7  --port 9999 marked+bound  -> ECONNREFUSED
--ip 100.95.0.99 --port 8080 marked+bound  -> ECONNREFUSED
--ip 100.95.0.7  --port 8080 --udp         -> send-fail EPERM
--ip 100.95.0.99 --port 53   --udp         -> send-fail EPERM
--ip 100.95.0.1  --port 53   --udp         -> no-reply ETIMEDOUT; decoy: WILD-UDP-RECV from=127.0.0.1:41453 n=17
counters: drop 0 / tcp reject 2 packets 120 bytes / reject 2 packets 93 bytes
```
Only delivery to a decoy is the gateway (host-local by GW-LOCAL; leg-B never dials it; excluded from the reject). As predicted. Same on metal.

**S4** — controls (metal agrees):
```
(a) unmarked, no default:   100.95.0.7:8080 and 100.95.0.99:8080 -> ENETUNREACH
(d1) bind only, no default: ENETUNREACH
(c) mark only, unbound:     ip route get 100.95.0.7 mark 1 -> local ... dev lo table 100 src 100.95.0.7 ; dial -> ENETUNREACH
(c') table-100 route "src 127.0.0.1" (ip route replace ... exit 0): ip route get -> src 127.0.0.1 ; dial --mark 1 (no bind)
     -> RESULT ok client-local=127.0.0.1:40528 client-peer=100.95.0.7:8080 ; ACCEPT peer=127.0.0.1
with default route:
(b1) unmarked, tuple not in set: 100.95.0.99:8080 -> ECONNREFUSED ; udp :53 -> send-fail EPERM local=192.0.2.10
(b2) unmarked, tuple in set:     RESULT ok client-local=192.0.2.10:47920 ... ; ACCEPT peer=192.0.2.10:47920 local=100.95.0.7:8080
(d2) bind only (127.0.0.1):      EINVAL (both tuples)
```
Predictions corrected: (c) the connect fails `ENETUNREACH` rather than connecting from the VM address (`route.c:2768,2729`); (b2) an unmarked in-set dial with a default route is still diverted to leg-C from the uplink address (today's behaviour). Both halves are required; `src` on the table-100 route replaces the bind.

**S5**
```
ip route get 198.51.100.7 -> via 192.0.2.1 dev up0 src 192.0.2.10
v33 dial --leg-b --ip 198.51.100.7 --port 8080 -> ETIMEDOUT setup=[]   (SYN on up0, src 192.0.2.10)
v33 dial --leg-b --ip 192.0.2.50  --port 8080 -> ETIMEDOUT setup=[]   (SYN on up0)   up0 count=2
no default route: 198.51.100.7 -> ENETUNREACH
```
`ip route get 198.51.100.7 mark 1` is `local ... dev lo table 100`, so the mark must stay conditional. As predicted.

**S6**
```
no default:  --dev lo --bind 127.0.0.1 -> ok peer=127.0.0.1 ; --dev lo (no bind) -> ETIMEDOUT 1510 ms
default:     --dev lo --bind 127.0.0.1 -> ok ; --dev lo -> ETIMEDOUT ; up0 count=0
not in set:  --dev lo --bind -> ECONNREFUSED
S6x: ip route get 100.95.0.7 oif lo -> 100.95.0.7 dev lo src 192.0.2.10 ; lo: SYN 192.0.2.10.x > 100.95.0.7.8080, SYN-ACK back, no ACK
```
Why the SYN-ACK is not accepted was not investigated.

**S7** — production client (`crates/overdrive-netlink/src/client.rs:815-860,905-930`; calls as in `crates/overdrive-worker/src/mtls_intercept.rs:995-1010`):
```
0: local ; 32765: fwmark 0x1 lookup 100 ; 32766: main ; 32767: default
ip route get 100.95.0.7 mark 1 from 127.0.0.1 -> local ... dev lo table 100
ip route get 100.95.0.7 mark 0               -> via 192.0.2.1 dev up0
ip route get 198.51.100.7 mark 1             -> local ... dev lo table 100 src 198.51.100.7
```
A second probe run did not stack a rule (its route add got `EEXIST`, which production swallows). Metal identical order.

**S8-equivalent** (metal, lease via `launch.py`, nothing under `/home/ubuntu/overdrive` touched): real Cloud Hypervisor microVM (`--vsock cid=4331,backend=vhost-kernel`, guest `7.0.0-29-generic`, V-28 initramfs with `v33 vm-guest` on vsock 5000); leg-C stand-in `v33 vm-legc` (IP_TRANSPARENT listener splicing to the guest over AF_VSOCK); netns with no default route and no prefix route.
```
ip route show -> 192.0.2.0/24 dev up0 proto kernel scope link src 192.0.2.10   (no default)
32765: from all fwmark 0x1 lookup 100
(1) --leg-b --ip 100.95.0.7 --port 8080 -> RESULT ok client-local=127.0.0.1:47320 client-peer=100.95.0.7:8080
    reply="FROM-MICROVM uname_r=7.0.0-29-generic req=REQ-100.95.0.7:8080" setup=[mark=1 bind=127.0.0.1]
(2) unmarked to the VM tuple -> ENETUNREACH
(3) --leg-b --ip 100.95.0.7 --port 9999 -> ECONNREFUSED
LEGC ACCEPT peer=127.0.0.1:47320 local=100.95.0.7:8080 -> vsock cid=4331 port=5000 connected
guest serial: GUEST_SERVED FROM-MICROVM uname_r=7.0.0-29-generic req=REQ-100.95.0.7:8080
```
Not run as specified: the tree has no production leg-B mark, and the vsock owner, guest transport and prefix-route boot step are DESIGN scaffolds (26 `panic!("{SCAFFOLD}")` in `guest_transport/mod.rs`; `overdrive-host/src/{vmm,cid_claim,beacon_io}.rs`). Differences: no serve/deploy, control plane, CA/mTLS (plaintext), real leg-C, owner or `workload_addr` allocation; guest app is the probe binary on a V-28 initramfs, not an Overdrive rootfs; `nft`/`ip` CLI; hand-added set element; mark and bind in a probe binary.

**Q3**
```
ip route get 100.95.0.7 mark 1 from 127.0.0.1  -> local ... dev lo table 100 mark 1
ip route get 100.95.0.7 mark 1 from 192.0.2.10 -> local ... dev lo table 100 mark 1
default route present: pre-marked -> ACCEPT peer=127.0.0.1:34318 ; unmarked (divert) -> ACCEPT peer=192.0.2.10:33896 ; both local=100.95.0.7:8080
```

## Design implications

- One conditional `SO_MARK` plus `bind(127.0.0.1)` removes the connect-time `ENETUNREACH` with no prefix route, default-route dependency, netdevice, BPF program or node state. PREFIX-ROUTE-LO, its protocol tag, the conflict refusal and INB-1-READ become unnecessary; INB-1 holds at the route table.
- Fail-closed shape is measured: unmarked `ENETUNREACH` or the existing reject/divert; mark-only `ENETUNREACH`; bind-only `ENETUNREACH`/`EINVAL`; marked non-registered tuple rejected. Only the gateway reached a host socket (by design).
- The contract must state both halves and the condition: peer inside the guest prefix -> mark `MTLS_TPROXY_MARK` and bind `127.0.0.1`; otherwise neither (S7: `198.51.100.7 mark 1` goes to `lo`).
- leg-C's same-node peer address becomes `127.0.0.1` on every host, where today it is the uplink address with a default route (S4 b2, Q3).
- DESIGN must pin how a remote VM backend is addressed (open question 1). `HostMtlsEnforcement` needs the guest prefix as a required constructor input.
- A2 (bind to `lo`) only works together with `bind(127.0.0.1)`, relies on a legacy kernel branch, and gives no advantage over A. `src 127.0.0.1` on the table-100 route is a valid alternative to the bind but adds netlink surface and changes a shared route.
- S1/S4 supply tests with no default route; S2 with one, replacing R5-43's reliance on a default route in test hosts.
- The `EINVAL` control relies on `route_localnet=0` (production never sets it).

## Not run, and limits

S8 as specified; each probe once per host (no repetition count, concurrency or IPv6); no ftrace of `ip_route_me_harder`; `IP_BIND_ADDRESS_NO_PORT` on/off not compared; S6x cause not investigated; UDP only as controls; production nft writer and real leg-C not used; metal iproute2/nftables versions not recorded.

## Cleanup

Namespace `v33ns` removed by each phase's trap; `ip netns list` empty on Lima and metal; no probe processes left; metal `/tmp/v33-work` removed under the lease; Lima `~/.cargo-target-v33` and `~/.cargo-home-v33` removed. Nothing under `crates/`, `specs/`, `tests/` or other docs was edited; nothing committed.

## Gate recommendation

**PROMOTE** alternative A (conditional `SO_MARK = MTLS_TPROXY_MARK` + `bind(127.0.0.1)` in leg-B for guest-prefix peers; no prefix route) to replace ADR-0188, pending user approval and the DESIGN pin on remote-VM addressing.
