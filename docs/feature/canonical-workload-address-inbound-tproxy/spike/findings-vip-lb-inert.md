# Spike findings (increment-c) — VIP/LB live-vs-inert under a real deploy (GH #241)

**Probe:** `spike-scratch/increment-c/` (gitignored, throwaway). Unlike
increment-b (a standalone aya probe of the connect4 firing *scope*), this
increment drives the **REAL production binaries** — `overdrive serve`
(composition root, real `EbpfDataplane`: XDP + cgroup attach) + `overdrive
deploy` — and OBSERVES the live dataplane. **Zero `crates/` modification.**

**Why this probe ran:** increment-b proved `cgroup_connect4_service` *FIRES*
for Path-A workloads, but a `connect4` firing is only half the question. The
hook only *rewrites* on a `LOCAL_BACKEND_MAP` hit; whether the ADR-0053
VIP/LB path is a **LIVE consumer** (something dials a VIP and the LB delivers
it, such that flipping `Backend.addr` to `workload_addr` (B2) would break a
working delivery) or **INERT** (programmed-but-never-dialed; all real dials go
direct-addr→nft-TPROXY) is undeterminable from code alone — a prior explorer's
reasoning self-contradicted on whether the LB has a live consumer. Per project
discipline this is settled by a real `serve` + `deploy`, not research/review.

**Kernel (pinned to the verdict):**

```
uname -r: 7.0.0-22-generic
```

> Dev Lima kernel, NOT the pinned-6.18 appliance kernel (ADR-0068). The
> primitives observed (cgroup-v2 connect4 rewrite, XDP attach, nft-TPROXY,
> per-netns /30 routing) all predate 6.18; authoritatively re-confirmed by the
> Tier-3 matrix when the slice lands.

---

## Binary verdict: **INERT (B2 safe → GATE is the correct reconciliation)**

Under a real `overdrive serve` + `overdrive deploy`, the ADR-0053
VIP/`SERVICE_MAP`/`LOCAL_BACKEND_MAP` load-balancer path has **no live v1
consumer**: nothing in the production build hands a VIP to a workload, and no
production component dials a VIP. The `cgroup_connect4` rewrite mechanism is
*mechanically live* (a deliberate VIP dial IS rewritten — see Pop-A below) but
**dead on the production path** — every real workload dial is a direct-addr
dial that the nft egress TPROXY captures for mTLS. Flipping `Backend.addr` to
`workload_addr` (B2) reclassifies the backend LOCAL→REMOTE, moving it off the
(unhit) `LOCAL_BACKEND_MAP` and onto the (also-unhit) XDP `SERVICE_MAP` path —
**breaking no delivery that works today.** GATE (gate the hydrator's LB
programming off mesh/Path-A backends so the dead XDP writes stop) is the
correct, sufficient reconciliation; TEACH (teach the LB `workload_addr`) is
not required.

---

## What was driven (production, real)

`overdrive serve` (no config file → `ServerConfig::new` defaults: single-node
veth shape) booted the **full real dataplane**:

- **XDP** `xdp_service_map` + `xdp_reverse_nat` attached to `ovd-veth-cli` /
  `ovd-veth-bk` (driver mode).
- **cgroup** `cgroup_connect4` / `cgroup_sendmsg4` / `cgroup_recvmsg4` attached
  at `/sys/fs/cgroup/overdrive.slice` (`multi` flag, ancestor of the alloc
  scope) — confirmed via `bpftool cgroup show`.
- All maps created + pin-by-name (`SERVICE_MAP` HoM pinned at
  `/sys/fs/bpf/overdrive/SERVICE_MAP`).

`overdrive deploy --detach svc.toml` deployed a TCP Service (one
`8080/tcp` listener, `nc -l 0.0.0.0 8080` backend). It converged fully:

```
Service 'spike-svc' (kind: Service)
Replicas (desired/running): 1/1
Alloc                    State        Restarts   Since
alloc-spike-svc-0        Running      0          (c=4,w=local)
    reason: driver started
VIP:           10.96.0.2
Listeners:
  8080/tcp
Issued certificates:
  serial: e9ee30ea844bca288f9d6cbcd864699f
    spiffe_id: spiffe://overdrive.local/job/spike-svc/alloc/alloc-spike-svc-0
```

The producer ran (rule 11B): `nc -l 0.0.0.0 8080` bound, in a **per-workload
netns** (Path-A) inside the alloc cgroup scope:

```
nc /proc/.../cgroup: 0::/overdrive.slice/workloads.slice/alloc-spike-svc-0.scope
workload netns: net:[4026532367]   host netns: net:[4026531833]   (DISTINCT => Path-A)
# workload netns is a /30 with the host as default gateway:
ovd-wl-0000@if20136 UP   10.99.0.2/30
default via 10.99.0.1 dev ovd-wl-0000
```

---

## Evidence — the maps the deploy programmed (real `bpftool map dump`)

`Backend.addr` is advertised **today** as `host_ipv4:port` (bridge
`backend_discovery_bridge.rs:349`: `SocketAddr::new(IpAddr::V4(self.host_ipv4),
listener.port)`), `host_ipv4 = 10.96.0.1` (the `ovd-veth-cli` addr). The
hydrator's ADR-0053 §4 partition (`b.addr.ip() == host_ipv4` → LOCAL) therefore
classified the single backend **LOCAL**, programming ONLY `LOCAL_BACKEND_MAP`:

```
[LOCAL_BACKEND_MAP] (cgroup_connect4's ONLY map):
  key:   02 00 60 0a  90 1f  06 00   => vip_host 10.96.0.2, port 8080, proto TCP
  value: 01 00 60 0a  90 1f  00 00   => backend 10.96.0.1, port 8080
  Found 1 element

[SERVICE_MAP]     (XDP remote-LB outer HoM):  Found 0 elements
[REVERSE_NAT_MAP] (XDP reverse path):         Found 0 elements
[BACKEND_MAP]     (XDP inner backend table):  Found 0 elements
```

(`REVERSE_LOCAL_MAP` carries the symmetric reverse entry `10.96.0.1:8080 →
10.96.0.2` for the same-host UDP reply path; `DROP_COUNTER` all-zero — XDP
dropped nothing.)

**The entire XDP VIP-LB path was programmed with ZERO entries** under a real,
fully-converged single-node deploy. The XDP `SERVICE_MAP` stood up,
attached, pinned — and received nothing.

> Inspection-tool note (debugging §11A): the kernel truncates map names to 15
> chars, so `bpftool map dump name LOCAL_BACKEND_MAP` errors `can't parse name`.
> All dumps above are **by map ID** after enumerating `bpftool map show` — the
> empty-XDP / populated-LOCAL split is real, not a wrong-surface read.

---

## The decisive probe — is the cgroup rewrite a LIVE consumer? (population diff)

A dialer was spawned **inside the alloc cgroup scope + the workload netns**
(joined `cgroup.procs`, `nsenter -n` into the workload netns), then used
`getsockname` on the connected socket to read the kernel's ACTUAL peer — the
falsifiable rewrite signal (a rewrite changes the peer from the requested VIP
to the backend). The cgroup attach was confirmed live for the scope:

```
/sys/fs/cgroup/overdrive.slice
  59611  cgroup_inet4_connect  multi  cgroup_connect4
```

**Population A — dial the VIP `10.96.0.2:8080` (what a VIP-LB consumer would do):**
```
  joined cgroup: 0::/overdrive.slice/workloads.slice/alloc-spike-svc-0.scope
  CONNECT-OK requested=10.96.0.2:8080 kernel_peer=10.96.0.1:8080
  REWRITE-DETECTED: cgroup hook changed dest 10.96.0.2 -> 10.96.0.1
```

**Population B — control, dial a non-VIP `10.99.0.1:8080` (not in LOCAL_BACKEND_MAP):**
```
  CONNECT-OK requested=10.99.0.1:8080 kernel_peer=10.99.0.1:8080
  NO-REWRITE: kernel peer == requested (10.99.0.1)
```

The control (Pop-B) proves the observation method is sound — a non-mapped dial
is NOT rewritten, so Pop-A's rewrite is genuine, not an artifact. So the
cgroup_connect4 LB rewrite **mechanically works**: dial the VIP and the hook
rewrites it to the backend. **But that VIP dial was issued artificially by the
probe.** The live-consumer question is whether *production* issues such a dial.

---

## Is there a live VIP-dial path in v1? — **NO**

Confirmed by code inspection cross-checked against the running system:

1. **No production code hands a VIP to a workload.** No env var / config /
   file injects `10.96.0.2` into the workload. The workload netns (a /30) only
   knows `10.99.0.2` + `default via 10.99.0.1`; the VIP `10.96.0.2` is never
   advertised to it.
2. **No name→VIP resolution path in v1.** The DNS responder that would map a
   service name to its VIP is **deferred (#243)**; VIP-dial-by-name is
   **#167/#61, also deferred**. No `resolv.conf` injection maps a name to a VIP
   in the production worker path.
3. **The egress mTLS path resolves the DIALED addr, not a VIP.** The worker's
   outbound accept loop recovers each captured connection's `orig_dst` via
   `getsockname` (`mtls_intercept_worker.rs`) and resolves it against
   `ServiceBackendsResolve`, which keys `by_addr` on `service_backends.
   Backend.addr` (`mtls_resolve_adapter.rs:214`). A workload dials a peer's
   **concrete addr** (today `host_ipv4`; B2 → `workload_addr`); the nft egress
   TPROXY (`iifname "ovd-hv-0000" tcp tproxy to 127.0.0.1:<agent>`, observed
   live) captures it. The VIP never enters this path.

So in v1 every real workload→workload dial is **direct-addr → nft egress
TPROXY → mTLS resolve(orig_dst)**. The VIP/cgroup-LB rewrite is reachable only
by a dial nothing in production issues.

---

## The B2 impact determination + evidence chain

B2 flips `Backend.addr` from `host_ipv4:port` to `workload_addr:port`. Effect
on the hydrator's ADR-0053 §4 LOCAL/REMOTE partition
(`service_map_hydrator.rs:340` `partition(|b| b.addr.ip() == host_ipv4)`):

- **Today (observed):** `addr = 10.96.0.1 (host_ipv4)` ⇒ LOCAL ⇒
  `RegisterLocalBackend` ⇒ `LOCAL_BACKEND_MAP` (1 entry, observed). XDP maps
  empty.
- **After B2:** `addr = workload_addr (10.99.0.x ≠ host_ipv4)` ⇒ REMOTE ⇒
  `DataplaneUpdateService` ⇒ XDP `SERVICE_MAP`/`REVERSE_NAT_MAP`/`BACKEND_MAP`.
  `LOCAL_BACKEND_MAP` no longer carries the entry.

**Does that break a working delivery?** No. The thing that would break is a
VIP dial that hits `LOCAL_BACKEND_MAP` (cgroup-rewrite) and stops hitting it
after B2 — but **nothing dials the VIP** (no live consumer, established above).
The XDP path B2 *moves the entry onto* is equally unhit (no VIP dial reaches
XDP either). Both the source map and the destination map are dead on the
production path; moving an entry between two dead paths breaks nothing live.
Meanwhile the *intended* B2 consumer — the egress mTLS `resolve(orig_dst)` —
gets exactly the `workload_addr` key it needs (D-TME-10 one-source/two-readers).

Evidence chain: (a) deploy converged, single backend classified LOCAL,
`LOCAL_BACKEND_MAP` got the only entry, **XDP maps stayed empty** [real
`bpftool` dump]; (b) the cgroup rewrite is mechanically live but only fires on
a VIP dial [Pop-A/Pop-B getsockname diff]; (c) **no v1 production path dials a
VIP** [no VIP injection / DNS #243 / VIP-dial #167-#61 deferred; egress resolves
`orig_dst`, not VIP]. ⇒ the LB/VIP path is **INERT on the production path**;
B2 is **safe**.

---

## Design implication

- **GATE is correct and sufficient.** DESIGN's GATE reconciliation —
  gate `ServiceMapHydrator`'s LB-map programming off mesh/Path-A workloads so
  the cgroup hook finds a `LOCAL_BACKEND_MAP` miss (→ no rewrite → the dial
  falls through to nft-TPROXY, which owns mesh delivery) — does NOT break any
  live delivery, because there is none on the LB path today. The hydrator
  should also be gated from the **dead XDP writes** for mesh backends (B2 would
  otherwise start programming `SERVICE_MAP`/`REVERSE_NAT_MAP` entries that no
  dial ever consults — dead writes, not a correctness break, but pure waste and
  a future-reader trap).
- **TEACH is NOT required.** Teaching the LB partition that `workload_addr` is
  host-local (so LB + mTLS coexist) buys nothing in v1 — there is no VIP-dial
  consumer to keep serving. TEACH only becomes relevant if/when a live VIP-dial
  path ships (DNS responder #243 + VIP-dial #167/#61); that is a separate,
  later, independently-drivable slice and should gate its own spike then.
- **increment-b reconciled:** increment-b's "cgroup_connect4 FIRES → GATE/TEACH,
  do NOT retire" stands. increment-c sharpens it: FIRES-and-rewrites, yes, but
  with **no live VIP-dial consumer**, so the weaker GATE (not TEACH, not retire)
  is the right cut.

---

## One-line gate recommendation

**GATE** — B2 is safe; gate `ServiceMapHydrator` so mesh/Path-A backends are
neither registered into `LOCAL_BACKEND_MAP` nor programmed into the XDP
`SERVICE_MAP` (the cgroup hook then misses → nft-TPROXY owns mesh delivery),
because the VIP/LB path has **no live v1 consumer** under a real `serve` +
`deploy`; TEACH is unnecessary until a VIP-dial path ships (#243 / #167 / #61).

---

## Cross-check against the convergence-dataplane-gap RCA

`docs/analysis/root-cause-analysis-convergence-dataplane-gap.md` documents the
single-node *local* backend delivery via `LOCAL_BACKEND_MAP`/cgroup (the path
the spike observed populated) and that the XDP `REVERSE_NAT_MAP`/`SERVICE_MAP`
are the *remote*-backend path, **empty by design on single-node localhost** —
exactly what this spike observed (debugging §11A: do not read the empty XDP
maps as "the dataplane is dead"; the surface this config writes is
`LOCAL_BACKEND_MAP`, and it was non-empty). The RCA is consistent with the
INERT verdict: today the live delivery surface is the cgroup/LOCAL path, and
B2 shifts a backend off it without any VIP-dial consumer to break.

---

## Housekeeping

- **Isolation verified:** all probe code in gitignored
  `spike-scratch/increment-c/` (`svc.toml`, `run*.sh`, `dial*.sh`). **Zero
  `crates/` modification**, working tree clean. Not committed (spike-scratch is
  gitignored). increment-a / increment-b findings untouched.
- **Real production binaries** (`overdrive serve` + `overdrive deploy`), not a
  `#[test]` harness; real `EbpfDataplane` (XDP + cgroup), real CA boot (KEK via
  the sanctioned `SystemdCredsKeyring` dev-opt-in fallback —
  `OVERDRIVE_CA_KEK` + `OVERDRIVE_CA_KEK_DEV_OPT_IN`, the production adapter's
  own gated dev path, NOT a test-double), real convergence loop.
- **Teardown clean:** 0 leftover XDP attachments, 0 overdrive veths
  (`ovd-veth-*` / `ovd-hv-*`), 0 alloc scopes, 0 named overdrive netns, nft
  `overdrive-mtls` table deleted, bpffs `SERVICE_MAP` pin removed, serve + nc
  processes killed. Verified post-teardown.
- **No GitHub issues created; no new deferrals surfaced** (the VIP-dial-path
  deferrals #243 / #167 / #61 are pre-existing and cited, not invented).
