# Research: Dial-by-Name Thin Path (Canonical Address) vs. Cilium Socket-LB — Codebase-Grounded DESIGN Input

**Date**: 2026-06-25 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (code-grounded) | **Sources**: 8 (6 local source files across 2 repos + 2 corroborating ADRs; 1 corroborating public doc)

## Pinned commits (every code citation is anchored to these)

- **Cilium** (`/Users/marcus/git/cilium/cilium`): `e99150f8` (2026-05-06) — provided by dispatch.
- **Overdrive** (`/Users/marcus/conductor/workspaces/helios/karachi-v1`): `04fa3d18` — HEAD of branch `marcus-sa/node-local-name-responder` ("feat(dial-by-name-responder): NameAnswer enum + hickory-proto wire codec"), from the repo git status at dispatch time.

> Bash was unavailable in this research context, so SHAs are taken from dispatch-provided values and the repo git-status snapshot rather than a live `git rev-parse`. The Cilium SHA matches the dispatch; the Overdrive SHA is the top commit of the working branch. **Knowledge-gap caveat**: these were not re-verified by a live `rev-parse` (see Knowledge Gaps).

---

## Executive Summary

The user ratified shifting dial-by-name from answering a volatile per-instance backend addr (ADR-0072 headless v1) to answering a **stable address** with the dataplane owning backend churn (the ClusterIP split), via the **thin path** (reuse the per-workload canonical address + the live nft-TPROXY + mTLS intercept) rather than the **full VIP path** (#61 XDP `SERVICE_MAP`). This research, grounded in `file:line` evidence from two local repos, finds the thin path is **conditionally sufficient and far thinner than #61 — but NOT zero-delta, and resting on a premise that the code does not satisfy as stated.**

The load-bearing finding (SQ1): **the existing per-workload canonical address is NOT stable across alloc cycles.** It is pure slot arithmetic — `workload_addr = WORKLOAD_SUBNET_BASE.network() + slot*4 + 2` (`veth_provisioner.rs:307,522-543`) — and the slot is assigned **per `AllocationId`** by a smallest-free scan (`veth_provisioner.rs:673-729`). A backend cycle produces a *new* `AllocationId`, which acquires the smallest-free slot — frequently a *different* slot, hence a *different* address. The dispatch's framing ("answer the existing canonical address; it never goes stale") therefore does not hold against the address as currently derived. Compounding this (SQ2): MtlsResolve (`ServiceBackendsResolve`) re-resolves per-connection against the current live-and-healthy set (the churn half genuinely works), but it keys its index on the **backend addr directly** — "Headless v1 (D-TME-10): the addr DNS returns IS the backend addr, so the index is keyed by the backend addr DIRECTLY" (`mtls_resolve_adapter.rs:89-91`) — so a query for a stable canonical addr would MISS the index and resolve `NonMesh` (silent cleartext).

Cilium is the canonical reference (SQ4): `cgroup/connect4` translates the stable ClusterIP to a currently-selected backend at connect() time (`bpf_sock.c:452-453`) and a reverse-NAT map (`bpf_sock.c:145-171,580-624`) preserves the ClusterIP illusion for getpeername/recvmsg — stable frontend constant, backend map churns underneath; functionally equivalent to Overdrive's MtlsResolve-at-intercept. On churn (SQ5), Cilium ACTIVELY force-terminates sockets to deleted/unhealthy backends via `bpf_sock_destroy` / netlink `SOCK_DESTROY` (`bpf_sock_term.c`, `sockets.go:34`), triggered by a backends-table watch (`termination.go:152-178`) — but it defaults TCP termination OFF for cost (`termination.go:212-219`), shipping "in-flight breaks, next connect re-LBs" as its own default.

**Verdict (SQ6)**: the thin path is sufficient with NO #61 reactivation IF the architect introduces a **stable per-`<job>` frontend address** (the Overdrive ClusterIP analogue, bound across alloc cycles) AND re-keys MtlsResolve to translate that stable addr → current live backend (the ClusterIP→backend translation headless-v1 deliberately deferred). The dns_responder delta is genuinely small (wire codec, `NameAnswer`, `MeshServiceName`, SOA-TTL all reused unchanged; IPv4 substrate unchanged — no AAAA/IPv6-VIP work); the real delta is the stable-frontend source (SQ1) and the resolve re-keying (SQ2). For v1, do NOT build a `sock_destroy`-equivalent — "in-flight breaks, next dial is live" matches Cilium's TCP default and Overdrive's single-node Phase-2 scope; name active drain a future refinement. **Two items are flagged for the architect, not decided here**: (1) the active-termination posture, and (2) — load-bearing — that shifting the answered addr from backend addr to stable frontend addr is an **ADR-0072 byte-consistency answer-contract revision** (which addr is byte-consistent, and what MtlsResolve must recognise, both change), to be re-ratified, not silently re-keyed.

---

## The Question

The user RATIFIED: dial-by-name should answer a **STABLE address** and let the dataplane own backend churn (the ClusterIP split), NOT the per-instance backend addr that ADR-0072 "headless v1" answers. Of the two ways to do it — (a) **thin path** = answer the per-workload **canonical address** + deliver via the already-live nft-TPROXY + mTLS intercept (ADR-0071 Path-A); (b) **full VIP path** (#61) = reactivate the deferred XDP `SERVICE_MAP` VIP-LB — the user chose **(a) the thin path**. This research establishes, with code evidence, whether the thin path is **sufficient** and what its **minimal delta** is, using Cilium socket-LB as the reference implementation of the same stable-address→live-backend pattern.

---

## Per-SQ Verdicts (up front)

| # | Sub-question | Verdict |
|---|--------------|---------|
| SQ1 | Is the per-workload canonical address STABLE across alloc cycles? | **NO.** `workload_addr = base + slot*4 + 2`; slot is per-`AllocationId` smallest-free; a cycled instance gets a NEW alloc id → possibly a different slot → a different address. Per-instance, NOT per-logical-workload. The thin path's "stable answer" premise does NOT hold against the existing canonical addr. |
| SQ2 | Does MtlsResolve resolve canonical addr → CURRENT live backend? | **PARTIALLY.** It DOES re-resolve per-connection against the current live-and-healthy set (churn half holds). BUT it keys on the BACKEND addr (headless v1), so a canonical-addr query MISSES → `NonMesh` → cleartext. Re-keying required. |
| SQ3 | What is the dns_responder answer delta, and how big? | **Small + concentrated in the UNBUILT name_index/answer source.** Wire codec, `NameAnswer` enum, `MeshServiceName`, SOA-TTL all reused UNCHANGED. `Records(Vec<SocketAddrV4>)` already holds 1 stable addr. IPv4 substrate unchanged (no #61 AAAA). Delta = name_index keying + the stable-addr source. |
| SQ4 | Cilium: stable-address → live-backend at connect time (reference) | **Cilium IS the canonical impl.** `cgroup/connect4` translates ClusterIP→selected backend at connect(); rev-NAT map preserves the ClusterIP illusion for getpeername/recvmsg. Functionally equivalent to MtlsResolve-at-intercept. Stable frontend constant; backend map churns. |
| SQ5 | Cilium: backend churn / stale-connection handling — does Overdrive need it? | **Cilium ACTIVELY terminates sockets to deleted/unhealthy backends (`bpf_sock_destroy`/netlink `SOCK_DESTROY`), BUT defaults TCP termination OFF.** Overdrive v1 does NOT need it: "in-flight breaks, next dial is live" is acceptable (matches Cilium's TCP default + single-node scope). Name it a future refinement. |
| SQ6 | VERDICT: is the thin path sufficient? minimal delta? borrow-from-Cilium? | **Conditionally YES** — sufficient with NO #61, IF a stable per-`<job>` frontend addr is introduced + MtlsResolve re-keyed to it. NOT zero-delta (the dispatch undercounted: the existing canonical addr is per-instance). Borrow Cilium's stable-frontend PATTERN; skip sock_destroy for v1. **FLAG: this is an ADR-0072 byte-consistency answer-contract revision — architect ratifies.** |

---

## Findings

### SQ1 — Is the per-workload canonical address STABLE across alloc cycles?

**VERDICT: NO. The canonical `workload_addr` is bound to the ALLOCATION (instance), not the logical workload. It is per-instance and CHANGES across an alloc cycle.** The thin path's premise — "DNS answers a stable address once, never goes stale" — therefore does NOT hold against the canonical address as currently derived. This is the load-bearing finding of the whole research and it inverts the thin-path assumption.

#### Finding 1.1 — The canonical address is pure slot arithmetic: `workload_addr = WORKLOAD_SUBNET_BASE.network() + slot*4 + 2`

**Evidence** (`crates/overdrive-control-plane/src/veth_provisioner.rs`):
- `WORKLOAD_SUBNET_BASE` = `10.99.0.0/16` (`veth_provisioner.rs:307`).
- The address is derived purely from the slot, never from the alloc id or workload id (`veth_provisioner.rs:522-543`):
  ```rust
  pub fn derive_workload_netns_plan(slot: NetSlot, responder_addr: Ipv4Addr) -> WorkloadNetnsPlan {
      // ...
      let network = WORKLOAD_SUBNET_BASE.network().saturating_add(u32::from(slot.0) * 4); // base + slot*4
      // ...
      let workload_addr = network.saturating_add(2); // second usable host of the /30
  ```
- The doc comment is explicit that the alloc id "derives nothing here — the alloc↔slot binding lives in the 02-04 slot↔alloc map" (`veth_provisioner.rs:499-500`), and the netns/veth/subnet are all "SLOT-keyed, NOT alloc-id-keyed (B3)" (`veth_provisioner.rs:269`).

**Source**: `crates/overdrive-control-plane/src/veth_provisioner.rs:269,307,499-543` (Overdrive @ `04fa3d18`)
**Confidence**: High (direct source).
**Analysis**: The address is a deterministic function of the slot alone. So the stability of the address reduces entirely to the stability of the **slot** for a given logical workload across instance cycles.

#### Finding 1.2 — The slot is assigned per `AllocationId` by a smallest-free scan; it is NOT keyed to a stable workload identity

**Evidence** (`crates/overdrive-control-plane/src/veth_provisioner.rs`):
- The allocator's held map is keyed by **`AllocationId`**, not by `JobId` / `MeshServiceName` / any logical-workload identity (`veth_provisioner.rs:673-678`):
  ```rust
  pub struct NetSlotAllocator {
      /// `AllocationId → NetSlot` binding for every currently-held allocation.
      held: Arc<Mutex<BTreeMap<AllocationId, NetSlot>>>,
  }
  ```
- `assign(alloc: AllocationId)` returns the **smallest-free** slot via a scan over currently-held values (`veth_provisioner.rs:709-729`):
  ```rust
  let taken: BTreeSet<NetSlot> = held.values().copied().collect();
  let slot = smallest_free_slot(&taken)?;
  held.insert(alloc, slot);
  ```
  Idempotent re-entry returns the existing slot only "if `alloc` is ALREADY held" (`veth_provisioner.rs:692-721`) — i.e. for the *same* `AllocationId`, never for a *new* instance of the same workload.
- `release(alloc)` frees the slot when the alloc tears down (`veth_provisioner.rs:739-744`); the released slot "becomes the smallest-free candidate again iff it is the lowest free value" (`veth_provisioner.rs:737-738`).

**Source**: `crates/overdrive-control-plane/src/veth_provisioner.rs:673-744` (Overdrive @ `04fa3d18`)
**Confidence**: High (direct source).
**Analysis**: An alloc cycle = old `AllocationId` released, new `AllocationId` started. The new instance calls `assign` with a fresh `AllocationId` and receives the smallest-free slot — which is the previously-released slot ONLY if it happens to be the lowest free value at that moment, and otherwise a different slot. With concurrent churn across multiple workloads, the released slot is frequently NOT the one the cycled instance re-acquires. There is no mechanism that re-binds a logical workload to its prior slot. Therefore `workload_addr` is per-instance and not stable across cycles.

#### Finding 1.3 — Restart adoption preserves the slot for a SURVIVING same-AllocationId process, not for a cycled instance

**Evidence** (`crates/overdrive-control-plane/src/veth_provisioner.rs`):
- On a `serve` restart the in-RAM allocator map is reconstructed EMPTY and rebuilt by re-assigning still-Running allocs (`veth_provisioner.rs:655-658,681-684`): "ephemeral runtime state, NEVER persisted, rebuilt on restart … no cross-restart slot persistence — criterion 6."
- The adopt-on-restart pass (`veth_provisioner.rs:1772-1797`, `adopt(alloc, slot)` at `:780-806`) recovers the slot↔alloc binding for survivors via cgroup→PID→`/proc/<pid>/ns/net` inode correlation, keyed by the surviving **`AllocationId`** (`ObservedAdoptNetns.owner: Option<AllocationId>`, `:1789-1797`). A slot held by a *different* alloc is a fatal conflict (`:780-806`).

**Source**: `crates/overdrive-control-plane/src/veth_provisioner.rs:655-684,780-806,1772-1797` (Overdrive @ `04fa3d18`)
**Confidence**: High (direct source).
**Analysis**: Adoption is about preserving the slot for the *same* surviving process across a *control-plane* restart — it explicitly correlates by the alloc that is *still running*. It does nothing to give a *newly-created* (cycled) instance of the same logical workload the old slot. So even with adoption, the address is stable only as long as the *instance* survives; a backend cycle breaks it.

#### SQ1 consequence for the thin path

The thin path was premised on "answer the canonical addr once; it never goes stale." Because the canonical addr is per-instance, **answering it does NOT by itself solve staleness** — when the backend cycles, the canonical addr the DNS answered points at the OLD (released) slot's /30, and the new instance lives at a different address. The thin path needs an additional ingredient to be churn-resilient: either (a) a **stable per-logical-workload address** that the allocator binds across cycles (a NEW allocator keying, see SQ6 delta), or (b) per-connection re-resolution that maps a stable name → current live instance (the MtlsResolve question, SQ2) — but SQ2 must confirm whether MtlsResolve keys on the canonical addr (which moves) or on a stable identity.

### SQ2 — Does MtlsResolve resolve the canonical addr → CURRENT live backend?

**VERDICT: PARTIALLY — and the part that matters for the thin path is the gap.** MtlsResolve (`ServiceBackendsResolve`) DOES re-resolve at connect time against the CURRENT live-and-healthy backend set (the "dataplane owns churn" half holds), so a cycled backend IS picked up on the next dial WITHOUT a DNS change. BUT it keys its index on the **backend addr directly** (`orig_dst == backend addr`, the headless-v1 contract), NOT on a stable canonical addr. So if the thin path changes DNS to answer the **canonical addr**, that canonical addr is NOT a key in the resolve index and `resolve` returns `NonMesh` (cleartext miss). **The thin path's resolve leg requires re-keying the index to the canonical addr — it does not work as-is.**

#### Finding 2.1 — `resolve` re-resolves per connection against the CURRENT running-and-healthy set (the churn-resilient half)

**Evidence** (`crates/overdrive-control-plane/src/mtls_resolve_adapter.rs`):
- The resolve path is a per-call point-lookup + pure classification over an in-RAM index, kept current by a background drain of the `service_backends` observation watch (`mtls_resolve_adapter.rs:521-540`):
  ```rust
  async fn resolve(&self, orig_dst: SocketAddrV4) -> Result<MtlsResolution> {
      if !self.watch_healthy.load(Ordering::SeqCst) { /* StoreUnreadable */ }
      Ok(self.index.read().classify(orig_dst))
  }
  ```
- `classify` returns `Mesh` only when a `running`-and-`healthy` backend is present at the addr, `MeshUnreachable` when claimed-but-unhealthy, `NonMesh` on a miss (`mtls_resolve_adapter.rs:292-300`). Healthiness is the readiness gate recomputed by `service_lifecycle` (`mtls_resolve_adapter.rs:131-134`).
- The index is maintained by **List-then-Watch**: List-at-probe seeds it, a single-owner drain folds every `service_backends` row, and a `Lagged` drop triggers a relist (`mtls_resolve_adapter.rs:31-120,408-448`). So the index always reflects the latest LWW `service_backends` snapshot.

**Source**: `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs:31-120,292-300,408-448,521-540` (Overdrive @ `04fa3d18`)
**Confidence**: High (direct source + the test suite at `:614-1084` exercises all three arms + relist recovery).
**Analysis**: This is genuinely the "dataplane owns churn at connect time" property. A new connection re-classifies against the current healthy set — a cycled backend that has (re)registered a `service_backends` row is picked up on the next dial. This half of the thin-path premise HOLDS, *for whatever address the index is keyed on*.

#### Finding 2.2 — The index is keyed on the BACKEND addr, NOT a canonical addr (the headless-v1 contract — the gap)

**Evidence** (`crates/overdrive-control-plane/src/mtls_resolve_adapter.rs`):
- Module rustdoc, verbatim (`mtls_resolve_adapter.rs:89-91`):
  > "Headless v1 (D-TME-10): the addr DNS returns IS the backend addr, so the index is keyed by the backend addr DIRECTLY — there is NO VIP→backend translation in the resolve path (that is #167/#61, out of scope)."
- The index type is `by_addr: BTreeMap<SocketAddrV4, BTreeMap<ServiceId, Backend>>` (`mtls_resolve_adapter.rs:207-214`), populated from `Backend.addr` of each `service_backends` row (`apply_row`, `:234-253`: `if let SocketAddr::V4(v4) = backend.addr { ... }`).
- `resolve(orig_dst)` does `self.index.read().classify(orig_dst)` — a direct point lookup of `orig_dst` in `by_addr` (`:292-293`). There is NO translation step from a frontend/canonical addr to a backend addr.

**Source**: `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs:89-91,207-253,292-300` (Overdrive @ `04fa3d18`)
**Confidence**: High (direct source).
**Analysis**: This is the structural mismatch. Today: DNS answers backend addr → app connects to backend addr → `orig_dst` = backend addr → index lookup hits. The thin path proposes: DNS answers canonical addr → app connects to canonical addr → `orig_dst` = canonical addr → **index lookup MISSES** (the index holds backend addrs, not the canonical addr) → `NonMesh` → silent cleartext. So the thin path needs the resolve index to be keyed (or to carry a translation) `canonical_addr → current backend`. This is the resolve-side delta the dispatch's framing under-counted: it is NOT "no resolve change."

#### Finding 2.3 — The canonical addr flows through the LIVE mesh path (nft-TPROXY + MtlsResolve), NOT the gated XDP-VIP-LB

**Evidence**:
- The `service_map_hydrator` GATES Path-A/mesh backends OUT of BOTH load-balancer paths (`crates/overdrive-core/src/reconcilers/service_map_hydrator.rs:265-274`): a backend whose addr is a member of `WORKLOAD_SUBNET_BASE` (`10.99.0.0/16`) gets "no `RegisterLocalBackend`, no `DataplaneUpdateService` because nft-TPROXY owns its delivery (D-GATE / D-GATE-PRED; reconciles ADR-0053 ↔ ADR-0071)."
- ADR-0071's framing: Path A is "per-workload netns + veth + nft-TPROXY + mTLS intercept both directions" (`docs/product/architecture/adr-0071-...md:21,34`), and the resolve mechanism is "the industry-canonical shape" modelled on Cilium (`adr-0071-...md:625-647`).

**Source**: `crates/overdrive-core/src/reconcilers/service_map_hydrator.rs:265-274`; `docs/product/architecture/adr-0071-...md:21,34,625-647` (Overdrive @ `04fa3d18`)
**Confidence**: High (direct source).
**Analysis**: Confirms SQ2's ADR-0053-gate tie: the canonical addr (a `10.99.0.0/16` member) is deliberately excluded from the XDP `SERVICE_MAP` / cgroup `LOCAL_BACKEND_MAP` LB paths and delivered exclusively via nft-TPROXY + MtlsResolve. The thin path therefore does NOT touch the gated #61 XDP-VIP-LB — consistent with the user's choice. The mesh delivery datapath is already the live one.

### SQ3 — dns_responder answer delta (what changes, how big)

**VERDICT: The wire codec / `NameAnswer` enum / `MeshServiceName` / SOA-negative-TTL machinery are REUSED UNCHANGED. The delta is concentrated in the (not-yet-built) ANSWER SOURCE — what the `name_index` (step 01-03) maps a name to. Today: name → N backend addrs (`Records(Vec<SocketAddrV4>)`). Thin path: name → 1 stable canonical addr. The IPv4 `SocketAddrV4` substrate holds, so there is NO AAAA/IPv6-VIP widening (unlike #61). BUT the byte-consistency contract changes WHICH addr is consistent — an ADR-0072 revision (see SQ6 flag).**

#### Finding 3.1 — `NameAnswer::Records` carries N backend addrs today; the wire codec renders one A-record per addr

**Evidence**:
- `pub enum NameAnswer { Records(Vec<SocketAddrV4>), NoData, NxDomain }` (`crates/overdrive-core/src/id.rs:954-961`). The v1 contract: "`Records(addrs)` — the name has ≥1 running-AND-healthy IPv4 backend; an `A` query is answered with one A record per addr" (`id.rs:946-947`).
- `encode` renders one A record per addr: `for addr in addrs { ... message.add_answer(Record::from_rdata(owner.clone(), A_RECORD_TTL_SECS, rdata)); }` (`crates/overdrive-control-plane/src/dns_responder/wire.rs:146-151`).
- The positive A-record TTL is `A_RECORD_TTL_SECS = 1`, justified verbatim: "Short — **a backend addr can change as allocations cycle**, so a dialer should not cache it long." (`wire.rs:47-49`).

**Source**: `crates/overdrive-core/src/id.rs:946-961`; `crates/overdrive-control-plane/src/dns_responder/wire.rs:47-49,146-151` (Overdrive @ `04fa3d18`)
**Confidence**: High (direct source).
**Analysis**: The short TTL exists PRECISELY because the current answer is a volatile per-instance backend addr (the same staleness the user wants to eliminate). The thin path's stable-address answer is what makes the short TTL unnecessary in principle (the prior research doc's recommendation). Note the data type `Vec<SocketAddrV4>` already comfortably holds "1 stable addr per name" — answering a single canonical addr is `Records(vec![canonical_addr])`, NO type change.

#### Finding 3.2 — Only the `wire` codec has landed; `name_index` (step 01-03) + `answer.rs` + `responder.rs` are NOT yet built

**Evidence**:
- The module map (`crates/overdrive-control-plane/src/dns_responder/mod.rs:9-31`): "Step 01-02 lands ONLY [`wire`] … The remaining components are later slices and are NOT declared here yet": `answer.rs` (`answer_for`), `name_index.rs` (the name-keyed `NameIndex`, "List-then-Watch sibling reader over the `service_backends` rows"), `responder.rs` (the bind + recv/sendmsg adapter).
- ADR-0072 specifies the answer source as a "name-keyed index reader over the SAME `service_backends` rows" (`docs/product/architecture/adr-0072-...md:96-106`).

**Source**: `crates/overdrive-control-plane/src/dns_responder/mod.rs:9-31`; `docs/product/architecture/adr-0072-...md:96-106` (Overdrive @ `04fa3d18`)
**Confidence**: High (direct source).
**Analysis**: The thin-path delta lands almost entirely in the UNBUILT `name_index` + `answer_for`. Because those are not yet written, the thin path is mostly a re-spec of the as-yet-unbuilt step, NOT a rewrite of shipped code — a favourable position. The dispatch's note is correct that the name_index work + the OQ-1 `SpiffeId`→`<job>` accessor it needs are the not-yet-built step 01-03.

#### Finding 3.3 — The concrete delta: index keying + answer source, NOT the substrate

**Evidence (today's answer source contract)** — ADR-0072 § "v1 DNS answer contract" (`docs/product/architecture/adr-0072-...md:51-71`):
> "The headless return-shape (D-TME-10): the answered `A` addr is a **running-and-healthy** `service_backends` addr — **byte-identical** to the addr `MtlsResolve` resolves … the shipped byte-consistency guarantee: `MtlsResolve` resolves a running-and-healthy [addr] → `Mesh`, never `MeshUnreachable`. No VIP, no #167, no [VIP widening]." (`adr-0072-...md:63-71`).
- IPv4 substrate is explicit: "v1 is IPv4-only"; "IPv6 / real AAAA records — widening the `SocketAddrV4`/`Ipv4Addr` substrate; out of v1 scope (AAAA is NODATA in v1)" (`adr-0072-...md:376,421-422`). The canonical `workload_addr` is itself an `Ipv4Addr` (SQ1, `veth_provisioner.rs:475,532`), so answering it keeps the substrate unchanged.

**The concrete delta table**:

| dns_responder surface | Today (headless v1) | Thin path | Change? |
|---|---|---|---|
| `MeshServiceName` grammar (`<job>.svc.overdrive.local`) | as shipped | as shipped | **UNCHANGED** |
| `wire::decode` / `wire::encode` codec | as shipped | as shipped (still renders `Records` as A-records) | **UNCHANGED** |
| `NameAnswer` enum type | `Records(Vec<SocketAddrV4>)` | `Records(vec![canonical_addr])` — 1 stable addr | **UNCHANGED type; changed value** |
| SOA negative-TTL machinery (DDN-8) | as shipped | as shipped | **UNCHANGED** |
| `A_RECORD_TTL_SECS = 1` | short, because backend addr is volatile | could relax (stable addr), but harmless to keep | **OPTIONAL** |
| `name_index` keying (step 01-03, unbuilt) | `<job>` → running-and-healthy backend addrs | `<job>` → that workload's STABLE canonical addr | **NEW SPEC (unbuilt)** |
| Answer source rows | `service_backends` (backend addrs) | a stable per-`<job>` canonical addr (source TBD — see SQ1/SQ6) | **NEW SPEC (unbuilt)** |
| OQ-1 `SpiffeId`→`<job>` accessor (step 01-03, unbuilt) | needed for `service_backends`→name keying | still needed (name→canonical keying) | **STILL NEEDED** |
| IPv4 `SocketAddrV4` substrate / AAAA=NODATA | as shipped | as shipped (canonical addr is IPv4) | **UNCHANGED — no #61 AAAA change** |

**Source**: `docs/product/architecture/adr-0072-...md:51-71,376,421-422`; `crates/overdrive-control-plane/src/veth_provisioner.rs:475,532` (Overdrive @ `04fa3d18`)
**Confidence**: High (direct source).
**Analysis**: The delta is small in code surface (concentrated in the unbuilt name_index/answer source) and zero in substrate. The genuine difficulty is NOT in dns_responder — it is the SQ1 problem (where does a STABLE per-`<job>` canonical addr come from, given the allocator binds slots per-`AllocationId`?) and the SQ2 problem (MtlsResolve must recognise the canonical addr as a key). The dns_responder change is the easy two-thirds; the hard third is upstream.

### SQ4 — Cilium socket-LB reference implementation of stable-address → live-backend

**VERDICT: Cilium IS the canonical implementation of the pattern. The client always targets the STABLE ClusterIP; the kernel-side `cgroup/connect4` hook translates ClusterIP→a CURRENTLY-selected backend AT connect() time and records a reverse-NAT entry so `getpeername`/`recvmsg` present the ClusterIP back. Functionally equivalent to Overdrive's MtlsResolve-at-intercept-time: both keep the client-visible address stable and choose the live backend at connection establishment. The frontend (ClusterIP) is constant; only the backend map entry changes on churn.**

#### Finding 4.1 — `__sock4_xlate_fwd` translates the stable frontend addr → a selected backend at connect()

**Evidence** (`/Users/marcus/git/cilium/cilium/bpf/bpf_sock.c`):
- The hook keys a service lookup on the connect()'s destination (the ClusterIP): `key.address = dst_ip` where `dst_ip = ctx->user_ip4`; `svc = lb4_lookup_service(&key, true)` (`bpf_sock.c:301-324`).
- Backend SELECTION happens here, against the current backend slots: `key.backend_slot = (sock_select_slot(ctx_full) % svc->count) + 1; backend_slot = __lb4_lookup_backend_slot(&key); backend_id = backend_slot->backend_id; backend = __lb4_lookup_backend(backend_id);` (`bpf_sock.c:417-429`). (Maglev / session-affinity are alternative selectors in the same function, `:392-415`.)
- The REWRITE: `ctx->user_ip4 = backend->address; ctx_set_port(ctx, backend->port);` (`bpf_sock.c:452-453`) — the socket's destination is rewritten from ClusterIP to the selected backend BEFORE the syscall proceeds.
- Entry point `cil_sock4_connect` (`__section("cgroup/connect4")`) calls `__sock4_xlate_fwd(ctx, ctx, false, true)` and returns `SYS_PROCEED` (`bpf_sock.c:458-475`).

**Source**: `/Users/marcus/git/cilium/cilium/bpf/bpf_sock.c:290-475` (Cilium @ `e99150f8`)
**Confidence**: High (direct source).
**Verification (public doc)**: Cilium "kube-proxy replacement" / "Socket LB" docs describe socket-level (cgroup `connect`/`sendmsg`/`recvmsg`) translation of ClusterIP to a backend at connection time, eliminating per-packet DNAT for E/W traffic (docs.cilium.io — see Full Citations). The source code is the primary; the doc corroborates the behaviour.
**Analysis**: Backend selection is per-connect against the *current* `svc->count` backend slots — a backend added/removed in the maps changes future selections without any change to the ClusterIP the client dials. This is the "DNS stable, dataplane owns churn" split realised in the socket-LB datapath.

#### Finding 4.2 — `sock4_update_revnat` + `__sock4_xlate_rev` preserve the illusion that the app talks to the ClusterIP

**Evidence** (`/Users/marcus/git/cilium/cilium/bpf/bpf_sock.c`):
- On the forward path, after selecting the backend, the hook records a reverse-NAT entry keyed by `(socket cookie, backend->address, backend->port)` whose VALUE is the original `(dst_ip, dst_port)` = the ClusterIP: `key.cookie = sock_local_cookie(ctx); key.address = backend->address; key.port = backend->port; val.address = dst_ip; val.port = dst_port; ... map_update_elem(&cilium_lb4_reverse_sk, &key, &val, 0)` (`bpf_sock.c:145-171`, called at `:446`).
- On `getpeername4` / `recvmsg4`, `__sock4_xlate_rev` looks up the rev map by `(cookie, backend addr, backend port)` and rewrites the address BACK to the stored ClusterIP: `ctx->user_ip4 = val->address; ctx_set_port(ctx, val->port);` (`bpf_sock.c:580-624`, entry points `cil_sock4_recvmsg` / `cil_sock4_getpeername` at `:640-653`).

**Source**: `/Users/marcus/git/cilium/cilium/bpf/bpf_sock.c:145-171,446,580-624,640-653` (Cilium @ `e99150f8`)
**Confidence**: High (direct source).
**Analysis**: The application connect()s to ClusterIP and, on `getpeername`/`recvmsg`, is shown the ClusterIP — never the backend. The stable address is the client-visible identity end-to-end; the backend is invisible. (Note: a stale rev entry — service gone or `rev_nat_index` changed — is deleted with `REASON_LB_REVNAT_STALE`, `bpf_sock.c:609-613`; a churn-cleanup detail relevant to SQ5.)

#### Finding 4.3 — On endpoint churn the agent updates the BACKEND maps; the frontend service ID is constant

**Evidence** (`/Users/marcus/git/cilium/cilium/pkg/maps/lbmap`, `pkg/loadbalancer`):
- The lbmap layer exposes backend add/delete that mutate the backend maps independently of the service frontend (`pkg/maps/lbmap`): `AddBackend` / `DeleteBackend` / `UpdateBackendWithState` write the `lb4_backends` / backend-slot maps; the service frontend (the ClusterIP→`lb4_service` entry, with its `rev_nat_index`) is a separate write.
- `__lb4_lookup_backend_slot` + `__lb4_lookup_backend` (consumed by `__sock4_xlate_fwd`, SQ4.1) read whatever the current backend maps hold — so a backend map update is the entire churn mechanism on the fast path.

**Source**: `/Users/marcus/git/cilium/cilium/pkg/maps/lbmap` + `pkg/loadbalancer` (Cilium @ `e99150f8`); fast-path consumers `bpf_sock.c:421-429`.
**Confidence**: Medium-High (the `bpf_sock.c` consumer is direct/High; the Go map-writer layer is corroborated by the canonical reconciler structure rather than line-pinned in this pass — see Knowledge Gaps for the un-line-pinned Go writers).
**Analysis**: This is the architectural lesson for Overdrive: the **stable frontend ID stays constant; only the backend map entry changes** on churn. Overdrive's equivalent is the `service_backends` observation row (the backend set the MtlsResolve index folds) — the "backend map," updated on churn — versus a stable name/canonical-addr the client dials (the "frontend").

#### Finding 4.4 — The lesson: Cilium translates in the socket-LB datapath; Overdrive translates in MtlsResolve at intercept time — functionally equivalent for the thin-path premise

**Evidence**: Cilium does ClusterIP→backend at the `cgroup/connect4` socket hook (SQ4.1). Overdrive does its `orig_dst`→backend classification at the nft-TPROXY mTLS intercept, per-connection, against the live `service_backends` set (SQ2.1). ADR-0071 explicitly names Cilium as the reference for this read mechanism: "Cilium is the canonical implementation of exactly this pattern" (`docs/product/architecture/adr-0071-...md:625-647`).

**Source**: `bpf_sock.c:290-475` (Cilium); `mtls_resolve_adapter.rs:31-120,521-540` + `adr-0071-...md:625-647` (Overdrive) — both @ their pinned SHAs.
**Confidence**: High.
**Analysis**: Both architectures: (1) client dials a stable address, (2) a kernel/intercept layer selects a live backend at connection-establishment time, (3) the backend set is the only thing that changes on churn. They ARE functionally equivalent for the thin-path premise — PROVIDED Overdrive's stable address is genuinely stable (SQ1) and MtlsResolve recognises it (SQ2). The Cilium parallel is exact only when Overdrive supplies BOTH halves: a stable frontend AND a translation that maps it to the current backend. Cilium has both (ClusterIP + rev-NAT map); today's Overdrive has the translation keyed on the *backend* addr, not a stable frontend (SQ2.2) — the missing half.

### SQ5 — Cilium backend churn / stale-connection handling — does Overdrive need an equivalent?

**VERDICT (mechanism): Cilium ACTIVELY force-terminates existing sockets to a backend when that backend is deleted or goes unhealthy, so clients re-LB to a live backend instead of hanging on a dead one. Trigger = a StateDB backends-table watch firing on `change.Deleted || !backend.IsAlive()`; kernel mechanism = `bpf_sock_destroy` (BPF socket-iterator) OR netlink `SOCK_DESTROY` (sock_diag) fallback, filtered by the rev-NAT map. Notably, TCP termination is OFF by default (cost), so even Cilium ships a "new connections re-LB; in-flight TCP just breaks on its own" posture as the default.**

**RECOMMENDATION (for the architect, NOT a decision): the thin path does NOT need a sock_destroy-equivalent for v1. "In-flight breaks, next dial is live" is an acceptable v1 posture given Overdrive's single-node Phase-2 scope and the fact that Cilium itself defaults TCP termination OFF. Capture active socket-termination as a named future refinement, not a v1 blocker.**

#### Finding 5.1 — The trigger: a backends-table watch fires termination on delete-or-unhealthy

**Evidence** (`/Users/marcus/git/cilium/cilium/pkg/loadbalancer/reconciler/termination.go`):
- Module purpose, verbatim: "monitors the backends table for unhealthy and deleted backends and terminates UDP & TCP sockets connected to these backends to signal to the application that the destination has become unreachable" (`termination.go:31-36`).
- The loop watches `p.Backends.Changes(...)` and, per change, fires termination iff `change.Deleted || !backend.IsAlive()` (`termination.go:152-178`):
  ```go
  if change.Deleted || !backend.IsAlive() {
      opSupported := terminateConnectionsToBackend(p, sd, backend.Address)
  ```
- Gated on socket-LB being enabled (`p.ExtConfig.EnableSocketLB || ...BPFSocketLBHostnsOnly`, `termination.go:106-108`) and on a kernel-support probe (`InetDiagDestroyEnabled`); if unsupported it degrades gracefully and does NOT fail (`termination.go:110-123`).

**Source**: `/Users/marcus/git/cilium/cilium/pkg/loadbalancer/reconciler/termination.go:31-36,106-123,152-178` (Cilium @ `e99150f8`)
**Confidence**: High (direct source).
**Analysis**: This is the active-drain leg Overdrive does NOT have. Backend deletion/unhealth → existing sockets to that backend are force-closed so the client gets an error and reconnects (re-LBing to a live backend) rather than hanging on a black-hole.

#### Finding 5.2 — The kernel mechanism: `bpf_sock_destroy` iterator OR netlink `SOCK_DESTROY`, filtered by the rev-NAT map

**Evidence**:
- BPF path (`/Users/marcus/git/cilium/cilium/bpf/bpf_sock_term.c`): `iter/tcp` + `iter/udp` BPF iterators call `bpf_sock_destroy(sk)` (the kernel kfunc, declared `__section(".ksyms")`, `bpf_sock_term.c:39,82,102`) for each socket whose cookie matches the rev-NAT map entry for the target `(address, port)` (`matches_v4`/`matches_v6`, `bpf_sock_term.c:44-66`). So ONLY sockets that were socket-LB-translated to the deleted backend are destroyed.
- Netlink fallback (`/Users/marcus/git/cilium/cilium/pkg/datapath/sockets/sockets.go`): `SOCK_DESTROY = 21` (`sockets.go:34`); `DestroySocket` "sends a socket destroy message via netlink" (`sockets.go:96-100`); the `netlinkSocketDestroyer` filters via sock_diag and destroys (`sockets.go:127-150`). The selection between the BPF destroyer (`bpfSocketDestroyer`, `sockets.go:227-244`) and the netlink destroyer is the support-probe outcome.
- The filter that scopes destruction to LB'd sockets is the rev-NAT map check: `checkSockInRevNat` → `p.LBMaps.ExistsSockRevNat(cookie, id.Destination, id.DestinationPort)` (`termination.go:237-241`).

**Source**: `bpf/bpf_sock_term.c:39,44-66,82,102,148-170`; `pkg/datapath/sockets/sockets.go:34,96-100,127-150,227-244`; `pkg/loadbalancer/reconciler/termination.go:237-241` (Cilium @ `e99150f8`)
**Confidence**: High (direct source).
**Verification (public/official)**: `sock_destroy` / `SOCK_DESTROY` is the kernel's sock_diag socket-termination primitive (kernel.org / man7 sock_diag(7)); Cilium documents the feature as "socket-level load-balancer socket termination" / `lb-sock-terminate` in the kube-proxy-replacement docs (docs.cilium.io — see Full Citations). Source is primary; docs corroborate.
**Analysis**: The mechanism is non-trivial: a BPF socket-iterator program OR a netlink sock_diag sweep, both kernel-version-gated, both scoped by a rev-NAT map Overdrive does not maintain (Overdrive has no socket-cookie→backend rev map; its translation is per-connection at the TPROXY intercept, not a persistent socket-LB rev entry).

#### Finding 5.3 — Even Cilium ships "in-flight TCP just breaks" as the DEFAULT (TCP termination is off)

**Evidence** (`/Users/marcus/git/cilium/cilium/pkg/loadbalancer/reconciler/termination.go`):
- TCP termination is hidden behind `LBSockTerminateAllProtos`, OFF by default, with the rationale verbatim: "Currently terminating TCP is false by default since iterating TCP sockets can become expensive compared to UDP due to the sheer number of sockets in the system. Once this is optimized … the hidden config flag can be removed." (`termination.go:212-219`):
  ```go
  if !p.Config.LBSockTerminateAllProtos {
      return
  }
  ```

**Source**: `/Users/marcus/git/cilium/cilium/pkg/loadbalancer/reconciler/termination.go:212-219` (Cilium @ `e99150f8`)
**Confidence**: High (direct source).
**Analysis**: This is the strongest argument for the v1 posture recommendation: the reference implementation itself defaults to NOT actively terminating TCP connections to deleted backends — i.e. its default behaviour for the most common protocol IS "the in-flight connection breaks on its own; the next connect() re-LBs to a live backend." Overdrive adopting "in-flight breaks, next dial is live" for v1 is therefore consistent with Cilium's own default, not a corner Overdrive is cutting.

#### Finding 5.4 — Does Overdrive need it? — assessment (architect decides)

- **What Overdrive already has** (SQ2.1): NEW connections to the resolve-recognised address re-classify against the current healthy set, so a cycled backend is picked up on the next dial. This is the new-connection half Cilium also relies on.
- **What Overdrive lacks**: active termination of IN-FLIGHT connections whose backend just cycled. Without it, an established connection to a now-dead backend will hang/RST per normal TCP timeouts; the application reconnects and re-resolves to a live backend.
- **Cost of adding it**: a socket-cookie→backend rev map (Overdrive has none — its TPROXY intercept does not persist one), a backends-table change watch (Overdrive HAS the `service_backends` watch already, SQ2.1, so the *trigger* is cheap), and a kernel `sock_destroy`/netlink sweep gated on kernel support (Overdrive pins kernel 6.18, ADR-0068, which comfortably supports `SOCK_DESTROY`/`bpf_sock_destroy`).
- **Single-node Phase-2 scope**: churn is local; reconnect latency is a sub-second LAN round-trip; there is no cross-region black-hole window to defend. The blast radius of "in-flight breaks" is small.

**Recommendation (flagged for architect, not decided)**: For v1, do NOT build a sock_destroy-equivalent. Adopt "in-flight breaks → client reconnects → re-resolves to a live backend." Record active socket-termination (graceful drain) as a named future refinement — its natural home is the same place Cilium puts it (a backends-change-watch reconciler), and Overdrive already owns the trigger surface (`service_backends` watch). If a workload class emerges where in-flight breakage is unacceptable (long-lived stateful streams), revisit with the Cilium mechanism as the template — but note even Cilium gates the TCP case OFF by default.

### SQ6 — VERDICT for the architect

#### Is the thin path SUFFICIENT to make dial-by-name's answer stable + churn-resilient, reusing shipped infra, with NO #61 XDP-VIP-LB reactivation?

**Conditionally YES — but ONLY if a stable per-logical-workload address is introduced. The thin path as the dispatch framed it ("answer the EXISTING canonical address") is INSUFFICIENT as-is, for two compounding reasons found in code:**

1. **The existing canonical `workload_addr` is NOT stable across alloc cycles** (SQ1). It is `base + slot*4 + 2`, the slot is assigned per-`AllocationId` by a smallest-free scan, and a cycled instance gets a *new* `AllocationId` → potentially a *different* slot → a *different* address. Answering it once does not survive churn.
2. **MtlsResolve keys its resolve index on the BACKEND addr, not on a canonical/frontend addr** (SQ2). Even if DNS answered a stable canonical addr, `resolve(canonical_addr)` would MISS the index (which holds backend addrs) and return `NonMesh` → silent cleartext.

**The thin path becomes sufficient with a bounded delta that does NOT touch #61's XDP-VIP-LB:** introduce a STABLE per-logical-workload address (a per-`<job>` frontend, the Overdrive analogue of Cilium's ClusterIP) and make MtlsResolve translate `that stable addr → current live backend`. This is exactly Cilium's split (SQ4): stable frontend constant, backend map churns underneath. It reuses the entire nft-TPROXY + intercept datapath and the `service_backends` watch; it does NOT need the XDP `SERVICE_MAP`/cgroup `LOCAL_BACKEND_MAP` LB paths (which the hydrator already gates mesh backends out of, SQ2.3). So the user's "no #61" choice holds.

**Crucial scoping correction for the architect**: the dispatch's "thin path" assumed the per-workload canonical address is already stable. It is not (SQ1). The genuinely-thin, genuinely-stable shape is NOT "reuse the existing per-alloc `workload_addr`" — it is "introduce a stable per-`<job>` frontend addr (small new allocator keying) + re-key MtlsResolve to it." That is still far thinner than #61 (no XDP, no AAAA/IPv6-VIP widening, no `SERVICE_MAP` reactivation), but it is NOT zero-delta on the resolve/allocator side. Calling it "answer the existing canonical address" undercounts the work and would ship a stale-on-cycle answer.

#### The minimal delta — exactly which surfaces change, which stay

| Surface | Change / Unchanged | Why |
|---|---|---|
| `MeshServiceName` grammar (`<job>.svc.overdrive.local`) | **Unchanged** | Names are already logical-workload-keyed; the stable frontend is keyed on `<job>`, which this already is. (`id.rs` MeshServiceName) |
| `dns_responder::wire` codec (`decode`/`encode`) | **Unchanged** | Still renders `NameAnswer::Records` as A-records; `SocketAddrV4` substrate unchanged. (`wire.rs:146-151`) |
| `NameAnswer` enum type | **Unchanged type, changed value** | `Records(vec![stable_addr])` — 1 stable addr; `Vec<SocketAddrV4>` already holds it. (`id.rs:954-961`) |
| SOA negative-TTL / `A_RECORD_TTL_SECS` | **Unchanged (TTL optionally relaxable)** | A stable answer makes the 1 s TTL no longer load-bearing, but keeping it is harmless. (`wire.rs:45-49`) |
| `dns_responder` `name_index` (step 01-03, UNBUILT) | **NEW SPEC** | Key `<job>` → that workload's STABLE frontend addr, not → backend addrs. |
| OQ-1 `SpiffeId`→`<job>` accessor (step 01-03, UNBUILT) | **Still needed** | Needed to key name↔workload either way. |
| **Stable per-`<job>` frontend address source** | **NEW (the load-bearing delta)** | A per-logical-workload address bound across alloc cycles — the missing stable frontend (SQ1). Candidate homes: (a) a `<job>`-keyed frontend allocator alongside `NetSlotAllocator` (the Overdrive ClusterIP); (b) extend `NetSlotAllocator` to bind slots per logical workload, not per `AllocationId` — but that conflates the netns slot (genuinely per-instance) with the frontend addr (must be per-workload), so (a) is cleaner. ARCHITECT TO DESIGN. |
| `MtlsResolve` / `ServiceBackendsResolve` index keying | **CHANGE** | Re-key (or add a translation) so `resolve(stable_frontend_addr) → current live backend`, the Cilium ClusterIP→backend translation Overdrive's headless-v1 deliberately deferred ("that is #167/#61", `mtls_resolve_adapter.rs:89-91`). This is the resolve-side delta the thin path needs. ARCHITECT TO DESIGN keying. |
| nft-TPROXY intercept datapath (ADR-0071 Path-A) | **Unchanged** | The entire interception/capture/handshake/kTLS/splice path is reused verbatim; only the `orig_dst` it captures changes (now a stable frontend, not a backend addr). |
| `service_map_hydrator` mesh-gate (`WORKLOAD_SUBNET_BASE`) | **Unchanged (review subnet membership)** | Mesh backends are gated out of LB paths by `10.99.0.0/16` membership (SQ2.3). If the stable frontend addr lives in a NEW subnet, confirm the gate's membership test still classifies it as mesh (a check, likely not a change). |
| #61 XDP `SERVICE_MAP` / cgroup `LOCAL_BACKEND_MAP` VIP-LB | **Unchanged (stays inert for mesh)** | Not reactivated. The thin path delivers via nft-TPROXY, not XDP. |
| IPv4 `SocketAddrV4` substrate / AAAA=NODATA | **Unchanged** | The stable frontend addr is IPv4 (like `workload_addr`). No #61 IPv6-VIP widening. |

#### Borrow-from-Cilium recommendation

- **Stable frontend → live backend translation (SQ4): YES, borrow the PATTERN, not the mechanism.** Cilium's ClusterIP→backend at the socket-LB datapath is the exact reference for "stable address, dataplane owns churn." Overdrive's equivalent is the stable per-`<job>` frontend + MtlsResolve translation. Adopt the *split* (stable frontend constant, backend churns underneath); the *mechanism* stays Overdrive's (MtlsResolve at TPROXY intercept), not eBPF socket-LB.
- **Active socket-termination on churn (SQ5): NO for v1 — name it a future refinement.** New connections already re-resolve to a live backend (SQ2.1). In-flight connections to a cycled backend break and the client reconnects → re-resolves. Even Cilium defaults TCP socket-termination OFF (SQ5.3); single-node Phase-2 scope makes the in-flight blast radius small. Record graceful-drain / `sock_destroy`-equivalent as a future refinement whose trigger surface (the `service_backends` watch) Overdrive already owns.

#### FLAG (do not decide) — ADR-0072 answer-contract revision required

**This is an explicit ADR-0072 revision the architect must ratify, not an implementation detail.** ADR-0072's shipped byte-consistency contract is: *the answered `A` addr is byte-identical to the addr `MtlsResolve` resolves* — and today that addr is the **running-and-healthy backend addr** (`adr-0072-...md:63-71`). The thin path shifts the answered addr from the **per-instance backend addr** to a **stable per-`<job>` frontend addr**. Byte-consistency must therefore be re-stated as: *the answered stable frontend addr is byte-identical to the addr MtlsResolve now recognises (and translates) — and MtlsResolve must translate that frontend addr to a current live backend, never miss it as `NonMesh`.* This changes WHICH addr is byte-consistent and what MtlsResolve must recognise. It also moots ADR-0072's "headless v1 / no VIP / no #167" framing — the stable frontend IS a (single-node, IPv4, no-XDP) VIP-shaped object, even though it is delivered via TPROXY, not XDP. **Surface this to the architect as the contract revision; do not silently re-key MtlsResolve without re-ratifying the answer contract.** (Same flag the prior research doc raised, now with the SQ1/SQ2 code evidence that makes it unavoidable.)

---

## Research Methodology

**Search Strategy**: PRIMARY — direct `Read`/`Grep` of source in two local repos (`/Users/marcus/conductor/workspaces/helios/karachi-v1` for Overdrive, `/Users/marcus/git/cilium/cilium` for Cilium), pinned to fixed SHAs. SECONDARY — one `WebFetch` of docs.cilium.io to corroborate the Cilium socket-LB + socket-termination behaviour already located in `bpf/`+`pkg/`.
**Source Selection**: Source code (authoritative for code-behaviour claims) + accepted ADRs (authoritative for design contracts) + one official open-source doc (docs.cilium.io, high reputation). Where source and doc could conflict, source wins (none did — they agree).
**Quality Standards**: each code-behaviour claim carries ≥1 `path:line` citation (authoritative-minimum for code behaviour, per the relaxed minimum-2 rule the dispatch set); each Cilium public-behaviour claim is corroborated by the docs.cilium.io fetch.

## Source Analysis

| Source | Repo / Domain | Reputation | Type | Access Date | Cross-verified |
|--------|---------------|------------|------|-------------|----------------|
| `crates/overdrive-control-plane/src/veth_provisioner.rs` | Overdrive @ `04fa3d18` | High | source code | 2026-06-25 | self (SQ1) + ADR-0071 |
| `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs` | Overdrive @ `04fa3d18` | High | source code | 2026-06-25 | self (SQ2) + ADR-0071/0072 |
| `crates/overdrive-control-plane/src/dns_responder/{mod,wire}.rs` + `crates/overdrive-core/src/id.rs` | Overdrive @ `04fa3d18` | High | source code | 2026-06-25 | self (SQ3) + ADR-0072 |
| `crates/overdrive-core/src/reconcilers/service_map_hydrator.rs` | Overdrive @ `04fa3d18` | High | source code | 2026-06-25 | self (SQ2.3) |
| ADR-0071 / ADR-0072 | Overdrive @ `04fa3d18` | High | accepted design contract | 2026-06-25 | source code |
| `bpf/bpf_sock.c` | Cilium @ `e99150f8` | High | source code | 2026-06-25 | docs.cilium.io (SQ4) |
| `bpf/bpf_sock_term.c` + `pkg/loadbalancer/reconciler/termination.go` + `pkg/datapath/sockets/sockets.go` | Cilium @ `e99150f8` | High | source code | 2026-06-25 | docs.cilium.io (SQ5) |
| Cilium "kube-proxy replacement" docs | docs.cilium.io | High | official OSS doc | 2026-06-25 | corroborates Cilium source |

Reputation: High: 8 (100%) | Avg: 1.0. All sources are source-of-truth code, accepted ADRs, or the official Cilium documentation — no medium/excluded-tier sources used.

## Knowledge Gaps

### Gap 1 — SHAs not re-verified by live `git rev-parse`
**Issue**: Bash was unavailable in this research context, so both SHAs are taken from dispatch-provided values (Cilium `e99150f8`) and the repo git-status snapshot (Overdrive `04fa3d18`, top of branch `marcus-sa/node-local-name-responder`). **Attempted**: `git rev-parse` via Bash (tool not enabled). **Recommendation**: the architect/orchestrator should confirm `git -C <repo> rev-parse --short HEAD` matches before pinning citations in a downstream artifact. The `file:line` citations are still valid against the read content regardless of the exact SHA label.

### Gap 2 — Cilium Go backend-map-writer call sites not line-pinned (SQ4.3)
**Issue**: SQ4.3 ("agent updates the backend maps on churn") is supported at the *fast-path consumer* (`bpf_sock.c:421-429`, High) but the Go writer layer (`pkg/maps/lbmap` `AddBackend`/`DeleteBackend`/`UpdateBackendWithState`) was identified by name/structure, not pinned to exact lines in this pass. **Attempted**: located the package; did not exhaustively line-cite the writers (the architecturally load-bearing point — "frontend constant, backend map churns" — is fully established by the rev-NAT illusion + the connect-time selection, both line-pinned). **Recommendation**: if the architect wants the exact Go write sites, grep `pkg/maps/lbmap/lbmap.go` for `AddBackend`/`DeleteBackend`; the conclusion does not depend on it.

### Gap 3 — Where exactly a stable per-`<job>` frontend addr should be sourced (design open, not a research gap)
**Issue**: SQ6 names two candidate homes (a new `<job>`-keyed frontend allocator vs. extending `NetSlotAllocator`'s keying) but does not pick one — that is an architect DESIGN decision, deliberately left open. **Recommendation**: the architect designs the frontend-addr allocator; this research establishes only that the *existing* `workload_addr` cannot be that source (SQ1).

## Conflicting Information

### Conflict 1 — The dispatch's "thin path = answer the EXISTING canonical address" vs. the code's per-instance slot binding
**Position A (dispatch premise)**: "answer the existing per-workload canonical address … DNS answers it once, never goes stale." — the ratified thin-path framing.
**Position B (source code)**: `workload_addr` is per-`AllocationId` (smallest-free slot scan, `veth_provisioner.rs:673-729`), so it changes across an alloc cycle — answering it once DOES go stale on churn.
**Assessment**: Position B wins — it is direct source code (authoritative for code behaviour). The dispatch's framing rests on an assumption the implementation does not satisfy. This does not kill the thin path; it re-sizes it (SQ6): a stable per-`<job>` frontend addr must be introduced. Surfaced as the load-bearing scoping correction so the architect does not design against the false premise.

### Conflict 2 — ADR-0072 "no VIP, no #167" framing vs. the thin path's stable-frontend object
**Position A (ADR-0072)**: "No VIP, no #167, no [VIP widening]" — headless v1 answers a backend addr directly (`adr-0072-...md:63-71`).
**Position B (this research's SQ6)**: the thin path's stable per-`<job>` frontend addr IS a (single-node, IPv4, no-XDP) VIP-shaped object, even though delivered via TPROXY not XDP.
**Assessment**: Not a true contradiction — a re-scoping. ADR-0072's "no VIP" meant "no XDP `SERVICE_MAP` / `fdc2::/16` / #167 IPAM." A stable IPv4 frontend delivered via the existing TPROXY datapath is VIP-*shaped* but does not reactivate #61. The honest framing for the architect: the thin path introduces a VIP-shaped frontend without the #61 machinery — and that is an ADR-0072 contract revision to ratify (the SQ6 FLAG), not a silent re-key.

## Full Citations

**Local source code (PRIMARY — authoritative for code behaviour):**

[1] Overdrive @ `04fa3d18`. `crates/overdrive-control-plane/src/veth_provisioner.rs` (lines 269, 307, 475, 499-543, 561-578, 655-806, 1772-1797). `NetSlotAllocator`, `WORKLOAD_SUBNET_BASE`, `derive_workload_netns_plan`, adopt-on-restart. Accessed 2026-06-25.

[2] Overdrive @ `04fa3d18`. `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs` (lines 31-160, 180-301, 408-540). `ServiceBackendsResolve` (MtlsResolve host adapter), `BackendIndex`, List-then-Watch, classify. Accessed 2026-06-25.

[3] Overdrive @ `04fa3d18`. `crates/overdrive-control-plane/src/dns_responder/mod.rs` (lines 1-31) + `dns_responder/wire.rs` (lines 1-216). Module map (only `wire` landed), wire codec, `A_RECORD_TTL_SECS`, SOA negative-TTL. Accessed 2026-06-25.

[4] Overdrive @ `04fa3d18`. `crates/overdrive-core/src/id.rs` (lines 938-961). `NameAnswer` enum + v1 answer contract. Accessed 2026-06-25.

[5] Overdrive @ `04fa3d18`. `crates/overdrive-core/src/reconcilers/service_map_hydrator.rs` (lines 256-311). Mesh-gate on `WORKLOAD_SUBNET_BASE` membership (D-GATE). Accessed 2026-06-25.

[6] Overdrive @ `04fa3d18`. `docs/product/architecture/adr-0071-transparent-mtls-enrollment-path-a-...md` (lines 21, 34, 122-237, 625-647) + `adr-0072-dial-by-name-responder-node-local-dns.md` (lines 14-15, 51-71, 96-140, 360, 376, 413-422). Path-A design contract + dial-by-name answer contract + byte-consistency. Accessed 2026-06-25.

[7] Cilium @ `e99150f8`. `bpf/bpf_sock.c` (lines 145-171, 290-475, 580-654) + `bpf/bpf_sock_term.c` (lines 39-173) + `pkg/loadbalancer/reconciler/termination.go` (lines 31-247) + `pkg/datapath/sockets/sockets.go` (lines 34-244). Socket-LB connect-time translation, reverse-NAT, socket termination on churn. Accessed 2026-06-25.

**Public docs (SECONDARY — corroboration of Cilium behaviour):**

[8] Cilium Authors. "Kubernetes Without kube-proxy". Cilium Documentation. https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/ . Accessed 2026-06-25. Corroborates: socket-LB translates the destination service IP to a backend on `connect`/`sendmsg`/`recvmsg` via cgroup BPF hooks (connect4/sendmsg4/recvmsg4/getpeername4); Cilium "forcefully terminates application sockets that are connected to deleted service backends, so that applications can be re-load-balanced to active backends"; requires `CONFIG_INET_DIAG` / `CONFIG_INET_UDP_DIAG` / `CONFIG_INET_DIAG_DESTROY` (the `SOCK_DESTROY` kernel surface). Reputation: High (official OSS project docs). No prompt-injection / attack patterns detected in fetched content (adversarial validation per operational-safety: clean).

## Research Metadata

Duration: ~1 session | Examined: 7 source files across 2 repos + 2 ADRs + 1 public doc | Cited: 8 | Cross-refs: every code claim ≥1 `path:line`; both Cilium public-behaviour claims corroborated by docs.cilium.io | Confidence: High (code-grounded; the one Medium-High sub-claim, SQ4.3 Go writers, does not affect any verdict) | Output: `docs/research/networking/dial-by-name-thin-path-canonical-address-vs-cilium-socketlb-research.md`
