# Evolution — dial-by-name-responder (GH #243 · ADR-0072)

**Finalized:** 2026-06-28 · **Wave arc:** SPIKE → DESIGN → DISTILL → DELIVER
(no separate DISCUSS wave — the product-owner discuss output was the `feature-delta.md`;
SPIKE ran immediately after DISCUSS as Slice 00) · **Branch:**
`marcus-sa/node-local-name-responder` · **Architect:** ADR-0072

> **STATUS — honest, load-bearing.** Implementation complete; all 10 steps
> GREEN on **dev-Lima 7.0.0-22**; **MERGE-GATED on the pinned-6.18 appliance-kernel
> Tier-3 CI run (ADR-0068) — not yet observed.** Two ATs (S-DBN-WS-STABLE,
> S-DBN-CHURN) are landed `#[ignore]`'d to GH #249 (backend-instance-replacement
> verb absent). This document records the work as it stands at the close of
> finalize.

---

## Feature summary

Workloads can now dial each other by **name** — `<job>.svc.overdrive.local` —
instead of by per-instance backend address. A root-netns UDP listener (single
`0.0.0.0:53` wildcard socket, `IP_PKTINFO` source-pinned) answers DNS for every
in-netns client via the per-netns `resolv.conf` gateway, returning a **stable
per-logical-workload frontend address F** in `10.98.0.0/16` that persists across
alloc cycles. The re-keyed `MtlsResolve` translates F → the current live backend
at connect time. A new `type route hook output` nft companion (REV-5) intercepts
the agent's host-originated leg-B re-dial so the inter-workload hop is mTLS'd
end-to-end.

The feature solves the SQ1 stale-address problem: because clients receive a
stable F rather than a per-instance `workload_addr`, a backend restart does not
orphan in-flight resolvers. The FrontendAddrAllocator retains F across transient
stop/restart (withhold-not-release, Finding-2 / ADR-0072). F is released only on
logical-workload deletion — the same lifecycle the ServiceVipAllocator adopts
post-#251 (ADR-0049 §6 amendment, 2026-06-28).

The end-to-end shape:

```text
client workload (netns B, /30)           server workload (netns C, /30)
  getaddrinfo("server.svc.overdrive.local")
    └─ per-netns resolv.conf → gateway → DnsResponder (root-netns 0.0.0.0:53)
        └─ NameIndex.answer_for(name, A) → Records([F])   F ∈ 10.98.0.0/16

  connect(F:service_port)
    └─ egress nft-TPROXY Path-A rule (from #241) + OUTPUT hook companion (REV-5)
        └─ leg-B intercepted → re-dialed to workload_addr_C:service_port
            └─ leg-C IP_TRANSPARENT+IP_FREEBIND → mTLS handshake → splice
                └─ server workload (plaintext)

DNS NameIndex: by <job> ← running-AND-healthy ServiceBackendRow gate (WITHHOLD seam)
FrontendAddrAllocator: <job> → F (written at admission, retained across alloc cycles)
MtlsResolve by_frontend: (F, port, Proto) → ServiceId → first-by-Ord healthy backend
```

## Business context

GH #243 ("Node-local DNS responder for dial-by-name"). Closes the dial-by-name
user story arc from the transparent-mtls-enrollment (#236 / ADR-0071) DISCUSS:
workloads should be able to reach mesh peers by logical name without knowing any
peer's ephemeral address. Operator impact: `overdrive deploy a.toml` + `overdrive
deploy b.toml` → A and B dial each other by name over mTLS within ~15 s, with no
SDK, no sidecar, and no DNS server to operate.

---

## Key decisions

### SPIKE (Slice 00) — 2026-06-24

**D1 — wildcard `0.0.0.0:53` with `IP_PKTINFO` source-pinning.**
One root-netns wildcard socket receives queries from all per-netns gateways and
replies with `ipi_spec_dst` = the queried gateway. Without source-pinning,
`getaddrinfo` (glibc) rejects the reply silently; `dig @gw` alone masks this.
The acceptance signal is therefore `getent`/`getaddrinfo`, never `dig @gw` alone.

**D2 — PROMOTE to DESIGN (not a walking-skeleton promotion).**
The mechanism was proven, but the production API surface (type/signatures,
`run_server` composition) was a DESIGN gate. Walking skeleton deferred to Slice 01
per `CLAUDE.md` § "Implement to the design".

### DESIGN → DISTILL → DELIVER — 2026-06-24 to 2026-06-28

**D3 — stable per-logical-workload frontend addr F (1a-A) over a hash
(1a-C) and per-instance backend addr.**
The `FrontendAddrAllocator` assigns a stable F ∈ `10.98.0.0/16` at `<job>`
declaration (admission in the `POST /v1/jobs` Service arm) and retains it
across alloc cycles. Deterministic hash (1a-C) was rejected — birthday-bound
collision risk. Per-instance `workload_addr` was rejected — the SQ1 stale-address
failure this feature exists to prevent.

**D4 — withhold-not-release (Finding-2).**
On zero running-and-healthy backends, the DNS answer is WITHHELD (NXDOMAIN with
1 s SOA negative-TTL — DDN-8). F is NOT released. Release fires only on
logical-workload deletion, mirroring the post-#251 ServiceVipAllocator lifecycle.
Releasing F on a transient stop would reintroduce the SQ1 stale-cached-F failure
on every restart.

**D5 — by_frontend three-way classify (1b-A, REV-2).**
`BackendIndex` was extended additively with a `by_frontend` map keyed on
`(SocketAddrV4, Proto)` = `(F, listener.port, listener.protocol)`. A frontend-
subnet miss (`orig_dst.ip() ∈ 10.98.0.0/16` but key absent) classifies
`MeshUnreachable` — fail-closed, no cleartext. This structural defense (Finding-3)
holds regardless of inter-drain timing between the re-keyed MtlsResolve and
the DNS NameIndex (the two drains are independent; the security posture does
not depend on ordering).

**D6 — REV-3: deploy-time assigner as the single WRITER (01-05).**
REV-2 named all the readers (`name_index`, `by_frontend`) and the single-owner
injection (02-01) but no WRITER. The just-landed 01-03 made the DNS read path
the de-facto writer (`allocator.assign` on every query — a non-deterministic
second writer). REV-3 corrected this by adding 01-05 (the deploy-time assigner:
`assign(<job>)` at admission in the `POST /v1/jobs` Service arm + empty-on-boot
converge-on-boot rebuild). `frontend_for` in `name_index.rs` was re-scoped to a
pure read-only lookup of the allocator's existing binding.

**D7 — REV-5: `type route hook output` nft companion (mid-DELIVER,
spike-proven during 02-02).**
When F ≠ the backend `workload_addr`, the agent's host-originated leg-B re-dial
traverses the kernel OUTPUT hook — the PREROUTING-only nft-TPROXY rule (Path A,
#241) never sees it. A new `type route hook output` nft chain + leg-S exemption
diverts leg-B. `type route` (not `type filter`) is load-bearing: it forces a
kernel route re-evaluation after `meta mark set`, firing the existing fwmark
`ip rule`→`local` route. Leg-C requires `IP_FREEBIND`. Cross-checked against
Cilium's from-host `mangle OUTPUT` path.

**D8 — re-size at 02-02: two ATs deferred to #249.**
`S-DBN-WS-STABLE` (alloc-cycle/stable-F Tier-3 proof) and `S-DBN-CHURN`
(in-flight churn fails fast) require a backend-instance-replacement /
restart-after-stop production verb that does not exist: `POST /v1/jobs/:id/stop`
writes a STICKY operator-stop intent; same-spec resubmit takes the `Unchanged`
path; `#211` is logical-workload deletion, not replacement. Both ATs landed
`#[ignore]`'d to GH #249. The core loop (`S-DBN-WS`, `S-DBN-SINGLE-SRC`) is
GREEN.

**D9 — egress TEST MODEL corrected (RCA, mid-02-02).**
The test client (and the ping-pong app) speaks **plaintext** over a bare
`TcpStream`. The egress capture lands on the agent's plaintext leg-F — not a
TLS leg. The keystone's "client presents TLS" shape is the INBOUND
(PREROUTING→leg-C) premise and is structurally wrong for the egress path.
mTLS proof is observed on the inter-agent leg-B↔leg-C wire (TLS 1.3 `0x17`
records), not via the client's handshake. See
`docs/analysis/root-cause-analysis-dial-by-name-agent-originated-mtls-stall.md`.

**D10 — #251 / ADR-0049 §6 amendment (2026-06-28, side effect).**
During 03-01 investigation, the `WorkloadLifecycle::service_vip_release_emission`
gate was found to release the ServiceVip on terminal (including stop) — the exact
SQ1 failure mode the dial-by-name frontend respects. Fix (`88679ed8`) swapped the
gate to release-on-deletion (`desired.job.is_none()`), making the ServiceVip and
the frontend F symmetric under one withhold-not-release lifecycle.

---

## Steps completed

| Step | Name | Status |
|------|------|--------|
| 01-01 | MeshServiceName newtype | GREEN — committed 2026-06-24 |
| 01-02 | NameAnswer enum + hickory-proto wire codec | GREEN — committed 2026-06-24 |
| 01-04 | FrontendAddrAllocator (1a-A) | GREEN — committed 2026-06-25 |
| 01-03 | NameIndex + pure answer_for (PURE READER after REV-3) | GREEN — committed 2026-06-25 |
| 01-05 | FrontendAddrAllocator WRITER (REV-3, deploy-time assigner) | GREEN — committed 2026-06-25 |
| 02-00 | MtlsResolve re-key — by_frontend + three-way classify (1b-A) | GREEN — committed 2026-06-25 / 2026-06-26 |
| 02-01 | DnsResponder host adapter + FrontendAddrAllocator + re-keyed MtlsResolve wired into run_server | GREEN — committed 2026-06-26 |
| 02-02 | Walking skeleton — getent resolves stable F, F translates to live backend, inter-agent hop is mTLS'd (REV-5 output-hook datapath + workload_addr forward-carry fix) | GREEN (S-DBN-WS + S-DBN-SINGLE-SRC); S-DBN-WS-STABLE + S-DBN-CHURN `#[ignore]`'d to #249 — committed 2026-06-27 |
| 03-01 | NXDOMAIN end-to-end — withhold before-running, after-stop, unknown name | NXDOMAIN-01 + NXDOMAIN-03 GREEN; NXDOMAIN-02 withhold-after-stop GREEN (enabled by #251 fix); NXDOMAIN-02 recovery `#[ignore]`'d to #249 — committed 2026-06-28 |
| 03-02 | Bidirectional ping-pong demo + E05 EDD expectation | GREEN (in-process Tier-3; ping_pong.py checked in; E05 honest-pending on #227/#75) — committed 2026-06-28 |

**43/43 S-DBN-\* scenarios mapped.** 41 GREEN/committed + 2 DEFERRED to #249
(S-DBN-WS-STABLE, S-DBN-CHURN) + 1 deferred (NXDOMAIN-02 recovery half to #249).

---

## Lessons learned

### 1. Name the single WRITER up front in the design

REV-2 named every READER (`name_index`, `by_frontend`) and the single-owner
injection (02-01) but no WRITER. The crafter made the DNS read path the de-facto
writer (`allocator.assign` on every query, `name_index.rs:262`). This created a
second writer that was order-dependent and non-deterministic. REV-3 corrected it
by adding 01-05 and re-scoping 01-03. The lesson: for any shared mutable resource
a design introduces, name the WRITER (what code calls the mutating method, when,
under what condition) as precisely as the readers. "Who writes" is as load-bearing
as "who reads."

### 2. The egress DIALER speaks PLAINTEXT — not TLS

The inbound keystone (`#241`) established a pattern where the test client presents
TLS (playing the originating agent's leg-B). That pattern is structurally wrong
for the egress path: the egress capture lands on the plaintext leg-F, not a TLS
leg. A test client presenting TLS toward the egress address stalls (no `ServerHello`
returns) with no rejection and no helpful error — it just hangs. The investigation
cost a multi-layer debugging session before a population-diff probe (plaintext dial
round-trips; rustls dial hangs) isolated it. Codified in `CLAUDE.md` § "East-west
mTLS tests — the egress DIALER speaks PLAINTEXT".

### 3. Walking-skeleton surprises surface NEW production gaps — not test-harness bugs

The 02-02 walking skeleton revealed that the mesh→mesh egress datapath was
unbuilt: the agent's host-originated leg-B re-dial was not intercepted by the
PREROUTING-only nft-TPROXY rule. This was correctly treated as a production gap
(SPIKE → REV-5 output-hook), not papered over with test-side workarounds. The
two deferred ATs (S-DBN-WS-STABLE, S-DBN-CHURN) are also genuine production
gaps (no backend-instance-replacement verb), not test design choices. Accept the
gap honestly, surface it to the user, land `#[ignore]`'d with a real issue number.

### 4. Security posture must not depend on inter-drain ordering

An early REV-2 shape relied on a temporal ordering barrier (by_frontend populated
BEFORE name_index) to guarantee the security invariant (no cleartext on a
frontend-subnet dial). This was fragile and irreducibly hard to test. REV-4
(ADR-0072) eliminated the ordering dependency: the fail-closed-on-frontend-subnet-miss
arm (`orig_dst.ip() ∈ 10.98.0.0/16` → `MeshUnreachable`) holds unconditionally,
regardless of inter-drain timing. The byte-identity property (answered F ==
recognised F) is enforced structurally via the ONE shared `FrontendAddrAllocator`
instance, not via a scheduling guarantee.

### 5. Verify precedents against the actual live code before citing them

REV-3 cited the `ServiceVipAllocator` precedent to justify "VIP released only on
conflict-rollback, never on stop." That was false at the time of the citation: the
VIP's release-on-terminal gate (`WorkloadLifecycle::service_vip_release_emission`)
fired on stop, which was the #251 bug. The false precedent cited in REV-3 was
coincidentally remedied when #251 was fixed (88679ed8, ADR-0049 §6 amendment)
during 03-01 development. Lesson: `gh issue view <N> --comments` + a `grep` of
the live code before asserting "the existing system does X."

---

## Issues encountered

| Issue | Impact | Resolution |
|-------|--------|------------|
| REV-3 missing-WRITER gap (name_index made de-facto writer) | Blocked 01-03; needed re-scope + new step 01-05 | Added 01-05 (deploy-time assigner), re-scoped 01-03 to pure reader |
| OUTPUT hook RST during 02-02 walking skeleton | Mesh→mesh egress not intercepted — connection stalled, not rejected | Spike increment-c proved `type route hook output` + IP_FREEBIND; landed as REV-5 (mtls_intercept.rs) |
| Egress test model error (client presenting TLS stalls) | Multi-layer debug; appeared as a datapath bug | Population-diff probe (plaintext vs TLS dial) isolated as test-harness error; CLAUDE.md updated |
| #249 — no backend-instance-replacement verb | S-DBN-WS-STABLE + S-DBN-CHURN + NXDOMAIN-02 recovery cannot be driven | ATs landed `#[ignore]`'d citing #249; Tier-1 contracts remain fully tested |
| #251 — ServiceVip released on stop (pre-fix) | NXDOMAIN-02 WITHHOLD-after-stop always resolved stale F | Fixed 88679ed8 as a standalone `/nw-bugfix`; ADR-0049 §6 amended |
| workload_addr forward-carry missing (#241-territory) | `FinalizeFailed{Stable}` dropped workload_addr, breaking bridge advertisement of F | Fixed in action_shim/mod.rs during 02-02 |
| Stale VIP memo eviction race (RCA-251, mechanism 3) | Dial-by-name name kept resolving stale F after stop | Resolved by #251 fix (the bridge's VIP memo is retained until deletion, not stop) |

---

## Migrated artifacts

| Source | Destination |
|--------|-------------|
| `distill/test-scenarios.md` | `docs/scenarios/dial-by-name-responder/test-scenarios.md` |
| `feature-delta.md` | `docs/architecture/dial-by-name-responder/feature-delta.md` |
| `spike/wave-decisions.md` | (key decisions extracted above; file discarded) |

---

## Open items

| Item | Tracker |
|------|---------|
| Backend-instance-replacement / restart-after-stop verb (unlocks S-DBN-WS-STABLE, S-DBN-CHURN, NXDOMAIN-02 recovery) | GH #249 |
| Logical-workload deletion verb (unlocks FrontendAddrAllocator::release production call site) | GH #211 |
| Merge-gate: Tier-3 matrix re-confirm on pinned-6.18 appliance kernel (ADR-0068) | DEVOPS obligation |
| E05 EDD expectation capture (black-box, real workload-identity CA, no mtls_identity_override seam) | Pending GH #227 / #75 (full-system EDD harness) |
| Residual host_ipv4 fallback masking a missing mesh workload_addr | GH #248 |
