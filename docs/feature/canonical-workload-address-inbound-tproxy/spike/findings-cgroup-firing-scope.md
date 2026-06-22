# Spike findings (increment-b) — cgroup_connect4_service firing scope under Path-A (GH #241)

**Probe:** `spike-scratch/increment-b/` (gitignored, throwaway; `probe-bpf/`
aya-ebpf kernel-side connect4 + `probe-host/` aya userspace harness; standalone
workspaces, no `overdrive-*` dependency edge, zero `crates/` modification).

**Why this probe ran:** the DESIGN wave for #241 found that flipping
`service_backends.Backend.addr` from `host_ipv4:port` to `workload_addr:port`
(Decision B2 — required so the egress `MtlsResolve` index classifies a dial to
the canonical `workload_addr` as Mesh) collides with `ServiceMapHydrator`
(ADR-0053 same-host LB), which partitions backends LOCAL-vs-REMOTE by
`addr == host_ipv4` and drives the `cgroup_connect4_service`
(cgroup_sock_addr / `BPF_CGROUP_INET4_CONNECT`) same-host delivery path.
ADR-0071 D-TME-3 retired the *mTLS* cgroup sibling (`cgroup_connect4_mtls`)
but NOT ADR-0053's *LB* sibling. Whether that LB path is inert (Path-A dials go
DNS→addr→TPROXY, never VIP→cgroup) or still live for Path-A per-workload-netns
allocs was **undeterminable from ADRs/code**. Per project discipline a
`cgroup_sock_addr` firing-scope question has no `BPF_PROG_TEST_RUN` backstop and
must be settled by a real Tier-3 `connect()`, not research/review.

**Kernel (pinned to the verdict):**

```
uname -r: 7.0.0-22-generic
```

> Dev Lima kernel, NOT the pinned-6.18 appliance kernel (ADR-0068). cgroup-v2
> `BPF_CGROUP_INET4_CONNECT` ancestor→descendant effective-program semantics
> are stable well before 6.18; the verdict is expected to hold on 6.18,
> authoritatively re-confirmed by the Tier-3 matrix when the slice lands.

---

## Binary verdict: **FIRES**

The ADR-0053 `cgroup_connect4_service` (`BPF_CGROUP_INET4_CONNECT`) program
**FIRES** on a `connect(2)` issued by a process inside a Path-A per-workload
netns + per-workload cgroup scope, under production's exact attach topology.
The LB cgroup path is **live for Path-A workloads — NOT inert.** This
**falsifies** the "inert, just retire it under a single-cut" hypothesis.

---

## Production attach topology mirrored (quoted)

- **Attach cgroup:** `/sys/fs/cgroup/overdrive.slice` —
  `DEFAULT_CGROUP_ATTACH_PATH` (`crates/overdrive-dataplane/src/lib.rs:118`),
  resolved as the default in `crates/overdrive-control-plane/src/lib.rs:1518-1520`.
- **Attach call / flags:**
  `cgroup_prog.attach(&cgroup_file, CgroupAttachMode::Single)`
  (`crates/overdrive-dataplane/src/lib.rs:709-715`). `CgroupAttachMode::Single`
  → kernel attach flags = 0 (verified in aya-0.13.1 `src/programs/links.rs:51`).
- **Workload cgroup placement:** `overdrive.slice/workloads.slice/<alloc>.scope`
  (`crates/overdrive-worker/src/cgroup_manager.rs:68-73`) — a **descendant** of
  the attach cgroup.
- **Workload process placement:** `setns(CLONE_NEWNET)` into a per-workload
  netns (`crates/overdrive-worker/src/driver.rs:389-392`) + PID enrolled into
  the scope's `cgroup.procs` (`driver.rs:495`).

The attach cgroup (`overdrive.slice`) is an **ancestor** of the workload scope
(`overdrive.slice/workloads.slice/<alloc>.scope`), so under cgroup-v2
effective-program semantics the connect4 hook applies to the workload's
`connect()`.

---

## Evidence (real pasted output, final run)

```
FIRE_COUNT total (all CPUs):              3
FIRE_BY_PORT[15101]  PATH-A (netns+desc):  1   <- fresh netns + descendant scope
FIRE_BY_PORT[15102]  POSITIVE-CONTROL:     1   <- detection works
FIRE_BY_PORT[15103]  PATH-A-NO-NETNS:      1   <- not netns-dependent
POSITIVE CONTROL: FIRED (detection works)
PATH-A VERDICT (fresh netns):   FIRES
PATH-A VERDICT (host netns):    FIRES
```

Total fires = 3 = exactly one per case, no spurious fires. `connect()` returned
`ECONNREFUSED` (no listener) — irrelevant, since `connect4` fires at the syscall
before the SYN; `/proc/self/cgroup` confirmed each child was in its intended
descendant scope.

**Compare-populations / positive control (guards against inspection-tool
false-negatives):** the positive control (a process in a known-covered leaf
scope) FIRED with the same detection method, so the FIRES verdict for the
Path-A case is trustworthy — a DOESN'T-FIRE would have been credible only if the
control fired, and here everything fired as predicted.

---

## Edge cases / surprises

1. **`CgroupAttachMode::Single` (flags = 0) is sufficient for descendant
   firing** — no `AllowMultiple`/`AllowOverride` needed. The attach at the
   ancestor `overdrive.slice` covers every descendant workload scope.
2. **Netns is orthogonal** — fresh-netns and host-netns descendants fired
   identically. The hook is cgroup-determined, not netns-determined.
3. **cgroup-v2 "no internal processes" rule** caused an `EBUSY` on the first
   positive-control attempt (enrolling directly into `overdrive.slice`, which
   has child cgroups). Caught mid-spike; control moved to its own leaf scope and
   re-ran → fired. The dev VM's pre-existing
   `overdrive.slice/control-plane.slice/` confirms the real production layout
   puts control-plane and workloads in distinct descendant leaves.
4. **`bpftool cgroup tree` displays the attach flag as `multi`** — a display
   artifact of aya's `bpf_link_create` link-API, NOT the kernel flag (which is
   0 / Single).
5. **Cross-check (not surprising):** standard cgroup-v2 ancestor→descendant
   effective-program semantics, matching the Cilium precedent (Cilium attaches
   `cgroup/connect4` at the cgroup-v2 root; it fires for every descendant
   pod/container).

---

## Design implication: GATE / TEACH — NOT retire

Because the hook FIRES for Path-A workloads, #241 **cannot** retire ADR-0053's
LB path under a Path-A single-cut. Once `Backend.addr` flips to
`workload_addr:port` (B2), the live `cgroup_connect4_service` participates on
every Path-A `connect()` against the maps `ServiceMapHydrator` programs — the
collision is real. The DESIGN reconciliation must be one of:

- **GATE** — gate `ServiceMapHydrator` (and/or the LB map programming) off
  mesh / Path-A workloads, so the firing hook finds a **miss** and nft-TPROXY
  owns delivery for mesh workloads (the LB path continues to serve any
  non-mesh workloads).
- **TEACH** — teach `ServiceMapHydrator` that `workload_addr` is host-local for
  the LB partition, so LB and mTLS coexist coherently.

The spike does NOT pick between GATE and TEACH — that is an architecture
decision about which path owns same-host delivery for mesh workloads. The spike
only **falsifies the "inert, just delete it" hypothesis** and proves the
collision is real.

---

## Gate recommendation: **GATE/TEACH (do NOT retire)**

`cgroup_connect4_service` empirically FIRES for Path-A per-workload-netns +
per-workload-cgroup connects on kernel 7.0 under production's exact attach
topology, so DESIGN must gate `ServiceMapHydrator` off Path-A/mesh workloads
(or teach it the `workload_addr` keying) rather than retiring ADR-0053's LB
path.

---

## Housekeeping

- **Isolation verified:** all probe code in gitignored
  `spike-scratch/increment-b/` (`probe-bpf/` aya-ebpf kernel-side, `probe-host/`
  aya userspace harness), zero `crates/` modifications, working tree clean. Not
  committed (spike-scratch is gitignored). increment-a's `findings.md` /
  `wave-decisions.md` left untouched.
- **eBPF was aya-rs Rust, not C** (per spike discipline) — the kernel-side probe
  is an `aya-ebpf` connect4 program modelled on `crates/overdrive-bpf`.
- **Teardown clean:** 0 leftover `cgroup_inet4_connect` programs, 0 leftover
  netns, probe scopes `rmdir`'d. Pre-existing dev-VM
  `overdrive.slice/control-plane.slice/` left as found.
- **No GitHub issues created; no deferrals surfaced.**
