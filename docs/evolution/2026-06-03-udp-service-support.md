# Evolution — udp-service-support

**Finalized:** 2026-06-03 · **Wave lifecycle:** DISCUSS → DIVERGE → DESIGN
→ DISTILL → DELIVER · **Source brief:** GitHub issue **#163** —
*"REVERSE_NAT_MAP lockstep populates only TCP entries; UDP responses
silently bypass source rewrite."* · **SSOT (preserved):**
`docs/feature/udp-service-support/feature-delta.md` (lean v3.14 — the
feature workspace is retained, not archived).

> **This record covers the whole feature in two increments.** Phase 01
> (5 slices, 2026-06-02) threaded `ServiceFrontend` + per-proto
> REVERSE_NAT and fixed #163. **Phase 02** (P2-Q4, 2026-06-03 — the
> increment this finalize emphasizes) threaded L4 proto into **both**
> service-LB map OUTER keys, IPVS-style: `SERVICE_MAP` and
> `LOCAL_BACKEND_MAP` are now keyed `(vip, port, proto)`. The two
> phases share a feature workspace and interleave in the git history.
> Sections below marked **[Phase 02]** are the proto-in-key increment;
> all others are the Phase 01 #163 fix.

## Summary

Thread the per-service L4 protocol end-to-end so the production
`EbpfDataplane` installs `REVERSE_NAT_MAP` entries that match a service's
declared protocol — and add a lockstep gate so the Sim≡Ebpf reverse-NAT
divergence that shipped #163 cannot recur silently.

Before this feature the shipped `Dataplane::update_service(vip: Ipv4Addr,
backends)` (Q-Sig option C) carried **no protocol at all**. The kernel
reverse-NAT key is `BackendKey { ip, port, proto }`, so the production
adapter hard-installed only TCP entries; every UDP backend response hit
`xdp_reverse_nat_lookup` with `proto=17`, found no entry, and returned
`XDP_PASS` without rewriting the source. The operator saw `deploy`
succeed and `alloc status` show Running while the client silently timed
out on a response sourced from the backend IP instead of the VIP.

The fix threads `(ServiceVip, port, Proto)` through a new
**`ServiceFrontend`** newtype on the existing call shape, makes the Ebpf
Step-4b fan-out honor `frontend.proto`, narrows the Sim's over-broad
`[Tcp, Udp]` hardcode to the declared proto, and pins the cross-adapter
key-set equality with a three-tier gate.

## Business context

UDP-bearing services — DNS, QUIC edge, game servers, syslog — are
first-class workloads under vision.md principle 4 ("all workload types
are first class"). #163 made them quietly broken: the worst class of
dataplane defect, because every control-plane signal is green and the
failure only surfaces as a real client timeout. The job served is the
existing **J-OPS-004** operator-trust contract ("trust the wire signal
for a Service-kind workload") and **J-PLAT-004** correctness contract
("the dataplane lockstep/ESR invariant is mechanically checked"), now
extended to the protocol dimension. No new job was minted — fragmenting
operator-trust by protocol would owe a J-OPS-006 for SCTP next.

The single force that drove the design: **the anxiety of silent
asymmetry**. The whole intent is to convert that silence into a loud,
mechanical PR-time gate failure (US-03).

## Key decisions

### DIVERGE — the option study (closes review finding H3)

A scoped DIVERGE scored 6 ways to thread proto on a locked
*developer-tool* taste matrix. The user held a standing preference for
the typed aggregate (Option 2). The matrix did **not** rubber-stamp it:

| Rank | Option | Score | Verdict |
|---|---|---|---|
| 1 | **Option 6 — `ServiceFrontend` newtype** on the first arg, `backends` separate | **4.17** | **CHOSEN** |
| 2 | Option 1 — positional `proto` scalar | 4.13 | co-leader (Δ 0.04, scoring noise) |
| 3 | Option 2 — typed whole-call aggregate (user's preference) | 3.57 | documented dissent |

`ServiceFrontend` won because it is the **forward-path twin** of the
existing `BackendKey { ip, port, proto }` (prior art: Katran `VipKey`),
making the lockstep set-equality a one-liner, at the same 8-site blast
radius as the positional option but with type-enforced "no service is
described without its proto." The aggregate's dissent is legitimate but
conditional — it wins only if multi-listener becomes a *trait-surface*
concern or the team commits to `update_service`-as-typed-SSOT; neither
was established. Independently peer-reviewed (Prism, APPROVED; one
non-decision-affecting arithmetic erratum corrected at landing).

### DESIGN — locked decisions (SSOT: **ADR-0060**)

- **D1a/D1b** — `ServiceFrontend { vip: ServiceVip, port: NonZeroU16,
  proto: Proto }`. The VIP is **IPv4-guaranteed by construction**:
  `ServiceFrontend::new(...) -> Result<Self, ParseError>` validates IPv4
  at the action-shim (the existing operator-visible rejection site),
  adapters narrow `IpAddr → Ipv4Addr` infallibly via `vip_v4()`. IPv6 is
  *not* demoted to a late opaque `DataplaneError`; the Failed-row path is
  unchanged. `port: NonZeroU16` makes port=0 unrepresentable.
- **D2/D3** — `#[derive(Debug, Clone, Copy, PartialEq, Eq)]` only (not
  wire, not persisted → no serde/rkyv/utoipa). New file
  `crates/overdrive-core/src/dataplane/service_frontend.rs`, sibling of
  `backend_key.rs`.
- **D4 — per-proto purge.** `update_service(frontend_udp, [])` purges
  only `frontend.proto`'s REVERSE_NAT keys for the VIP; a co-resident
  TCP frontend on the same VIP (separate per-listener call) survives;
  cross-service shared-backend keys preserved by the existing `live_keys`
  difference check. (Corrected the DISCUSS "removes both protos" text.)
- **D5** — minted **ADR-0060**, superseding the phase-2 §5 Q-Sig
  *paper* locked-A (`update_service(service_id, ServiceVip, backends)`)
  that was **never landed** on the trait. The honest from-state was
  option C, and every option in the study transitioned from C.
- **D6 — true blast radius is 8 sites, not 5.** C3 ("Proto NEVER
  defaulted to Tcp") is satisfiable only if `Action::DataplaneUpdateService`
  **and** `ServiceDesired` + the observation→desired projection also
  carry proto. Proto provenance is a **listener-bearing fact**
  (`ListenerRow` / the `BackendDiscoveryBridge` per-listener projection)
  — never the proto-less `service_backends` row; an unresolvable listener
  proto is a structured Failed error, never a silent `Tcp` default.
- **D8 — deferred at Phase 01**, then **subsumed by P2-Q4** (below): the
  US-05 forward-key granularity question (per-`(VIP,port)` vs VIP-only)
  is resolved by Phase 02 — the forward key is `(VIP, port, proto)`.

### [Phase 02] P2-Q4 — proto in both service-LB map keys, IPVS-style (user-locked 2026-06-03)

The wider architectural correction beyond #163. Add L4 protocol to
**both** eBPF service-LB OUTER keys:

- `SERVICE_MAP` outer key `(ServiceVip, port)` → **`(ServiceVip, port, Proto)`**
  (the wire-boundary XDP forward path).
- `LOCAL_BACKEND_MAP` key `(VIP, vip_port)` → **`(VIP, vip_port, proto)`**
  (the same-host cgroup `connect4` path).

So a TCP listener and a UDP listener on the same `(VIP, port)` occupy
two distinct map slots — the canonical DNS `tcp/53 + udp/53`
co-location — instead of one overwriting the other.

**Decision: proto-in-key over proto-less.** User rationale (verbatim):
*"we don't want to fix incorrect architecture — do `(vip, port, proto)`
as IPVS."* Evidence-weighted by
`docs/research/dataplane/service-map-l4-proto-keying-research.md` (Nova,
High confidence, 13 trusted-domain sources):

1. **Linux IPVS keys virtual services on `{protocol, addr, port}`
   natively** (UAPI `ip_vs_service_user`); kube-proxy iptables mode is
   per-protocol. Proto-in-key is the default in the two oldest, most
   deployed k8s dataplanes.
2. **Cilium carried a proto-less `lb4_key` as a known defect for ~5.5
   years** — issue #9207 (Sept 2019) → fix PR #37164 (Jan 2025).
   Proto-less was a half-decade bug, not a valid model.
3. **Kubernetes treats TCP+UDP-on-same-port as first-class** — CoreDNS
   `tcp/53 + udp/53`, the `MixedProtocolLBService` gate, HTTP/3 QUIC. A
   `(vip, port)` key cannot represent the DNS service correctly.
4. **Widening a HASH_OF_MAPS outer key is structurally free** — the
   proto byte consumes a reserved `_pad` byte with no byte-width change.

**Design sub-choices:**

- **Struct layout — absorb a pad byte, keep 8 bytes.** Both `ServiceKey`
  and `LocalServiceKey` become `{ vip: u32, port: u16, proto: u8,
  _pad: u8 }`, stay 8-byte `#[repr(C)]` PODs with `_pad` deterministically
  zeroed for stable BPF hashing (mirrors Cilium's
  `__u8 proto; __u8 scope; __u8 pad[2]`).
- **cgroup proto-source = `bpf_sock_addr.protocol`.**
  `cgroup_connect4_service` reads the IANA L4 byte directly
  (IPPROTO_TCP=6 / IPPROTO_UDP=17 — zero translation, no
  SOCK_*→IPPROTO_* table, single byte so no byte-swap).
  `bpf_sock_addr.type` is the documented fallback only if a matrix
  kernel leaves `protocol` unset. **No Tier-2 backstop** exists for
  `cgroup_sock_addr` (`BPF_PROG_TEST_RUN` ENOTSUPP ≤6.8) — proto-source
  correctness is verified at Tier 3 (real connect).
- **Action proto field.** `Action::DataplaneUpdateService` (forward
  path) and `Action::RegisterLocalBackend` / `DeregisterLocalBackend`
  (same-host path) all gain a `Proto` dimension, sourced from a
  **listener-bearing fact** — NEVER a silent `Proto::Tcp` default (C3).
- **Validator conflict granularity** — same-route keys widen to
  `(vip, port, proto)` (the `tcp/53 + udp/53` co-location case no longer
  false-fires `ConflictingServiceWrites`); genuine identical-tuple
  duplicate-slot collisions are still caught; the **cross-route**
  cgroup-vs-XDP conflict check stays VIP-only (the single coupling
  point between the two map clusters; the hydrator's classifier picks
  exactly one route per backend and the validator defends that).
- **Single-cut greenfield migration** — key structs change, maps are
  recreated from intent on next boot by the `ServiceMapHydrator`. NO
  dual-key shim, NO proto-less fallback, NO deprecation path; a grep
  finds no parallel proto-less key struct or `proto: 0`/IPPROTO_ANY
  branch.
- **Reuse disposition** — every touched component is EXTEND of an
  existing map/struct/program/handle; zero CREATE NEW. `Proto` is REUSE
  (ADR-0060's `backend_key::Proto`). No topology change → no new C4
  diagram.

**SSOT (amended in place — already permanent, not migrated):**
ADR-0040 (rev 2026-06-03, SERVICE_MAP outer key), ADR-0053 (rev
2026-06-03, LOCAL_BACKEND_MAP key + cgroup_connect4 proto-source
contract + RegisterLocalBackend proto field + sendmsg4 scope note).
ADR-0060 already carried `ServiceFrontend { vip, port, proto }` at the
boundary — no change needed.

### Lockstep pinning (DV4 / H1, resolved at DIVERGE)

A pure in-process Tier-1 retarget of `ReverseNatLockstep` against the
**real** `EbpfDataplane` is **infeasible** — the real adapter needs a
kernel + bpffs. The "byte-identical key set across BOTH adapters" claim
is pinned by **Sim (Tier 1) ∪ Ebpf (Tier 2 + Tier 3)** meeting at the
shared `BackendKey` set:

- **Tier 1** — `ReverseNatLockstep` asserts the SimDataplane installs
  exactly the declared-`frontend.proto` `BTreeSet<BackendKey>` (per-PR
  critical path).
- **Tier 2** — a `BPF_PROG_TEST_RUN` triptych asserts
  `xdp_reverse_nat_lookup` rewrites a `proto=17` response source to the VIP.
- **Tier 3** — drives the real `EbpfDataplane.update_service(frontend_udp)`
  and asserts `bpftool map dump REVERSE_NAT_MAP` contains `(ip,port,udp)`
  plus a wire capture sourced from the VIP (integration lane, Lima).

## Steps completed

All five slices reached COMMIT/EXECUTED/PASS (execution-log.json,
2026-06-02). RED_UNIT was `SKIPPED — NOT_APPLICABLE` on 01-02…01-05: each
is a Tier-2/Tier-3 acceptance-driven slice whose observability surface is
asserted directly by the AT, with no separate PBT/unit layer needed to
reach GREEN (the Tier-1 lockstep universe guard landed in 01-01).

| Slice | Story | Outcome |
|---|---|---|
| **01-01** | US-01 | `ServiceFrontend` newtype trait migration — single-cut across all 8 sites; Sim `[Tcp,Udp]` hardcode narrowed to `frontend.proto`; **zero** production proto-behavior change (TCP e2e stays green). |
| **01-02** | US-02 | Production `EbpfDataplane` Step-4b installs REVERSE_NAT entries matching `frontend.proto`; Sim≡Ebpf diff → zero. |
| **01-03** | US-03 | Reverse-NAT lockstep gate — Tier-1 Sim set-equality + Tier-3 Ebpf acceptance + Tier-2 UDP triptych; a dropped fan-out fails loudly per tier. |
| **01-04** | US-04 | Single-UDP-listener forward+reverse e2e (Tier 3, walking skeleton) — real round-trip, reply sourced from the VIP. |
| **01-05** | US-05 | `ServiceMapHydrator` per-listener fan-out; multi-listener TCP+UDP service works on both protocols through the full CLI→control-plane→reconciler→dataplane chain. |

### [Phase 02] Steps completed (proto-in-key)

Both Phase 02 steps reached COMMIT/EXECUTED/PASS (2026-06-03) with full
5-phase traces (PREPARE / RED_ACCEPTANCE / RED_UNIT / GREEN / COMMIT).
Decomposed as DECOMP B (two compile-atomic steps — no broken
intermediate).

| Slice | Outcome |
|---|---|
| **02-01** | SERVICE_MAP outer key `(vip,port)`→`(vip,port,proto)` + validator XDP write-key widen. Kernel/userspace `ServiceKey` (8-byte, proto at offset 6, `_pad` zeroed), `xdp_service_map` key build, endianness-lockstep proptest, conflict-validator false-fire fix (same-vip/different-port and `tcp/53 + udp/53` now `Ok`; identical-tuple still `Err`). Compile-atomic boundary: cgroup cluster untouched. |
| **02-02** | LOCAL_BACKEND_MAP key `(vip,port)`→`(vip,port,proto)` via `cgroup_connect4` `bpf_sock_addr.protocol` + `RegisterLocalBackend`/`DeregisterLocalBackend` gain `proto` + hydrator per-listener proto fan-out + validator cgroup-route & cross-route widen + Tier-3 real-cgroup connect (TCP and UDP `connect(VIP:port)` to the same `(vip,port)` each reach the proto-correct backend). |

DES integrity over the whole deliver tree:
`des-verify-integrity docs/feature/udp-service-support/deliver` → "All 7
steps have complete DES traces", **exit 0**.

### Quality gates

- **Mutation:** `cargo xtask mutants --diff origin/main --features
  integration-tests` (via Lima) — **100.0% kill rate** (3 caught / 0
  missed / 1 unviable). One genuine equivalent mutant (`|| → &&` in
  `gather_service_listener_facts`, a fast-path skip the downstream rkyv
  bytecheck rejects identically) was excluded via `.cargo/mutants.toml`
  `exclude_re` with full justification. One missed mutant
  (`push_register_local_backend_actions`) was killed by two added
  default-lane unit tests rather than left to Tier-3-only coverage.
- **Mutated surface:** `overdrive-core` (hydrator, `service_frontend`,
  trait), `overdrive-control-plane` (`reconciler_runtime`, action-shim),
  `overdrive-cli` (Service deploy arm). `overdrive-sim`/`overdrive-dataplane`
  paths are protected by Tier-1 DST + Tier-3, excluded from the nextest
  mutants lane per `.cargo/mutants.toml`.

### [Phase 02] Quality gates

- **Peer review:** APPROVED (nw-solution-architect-reviewer "Atlas",
  opus, 2026-06-02 roadmap review). One high finding — Tier-3 black-box
  criteria asserted white-box internal hydrator call counts — was
  resolved in-line by rewriting the criteria to an observable proxy
  (`bpftool map dump REVERSE_NAT_MAP` shows one key per listener, each
  with its own proto byte) plus dual VIP-sourced wire captures.
- **Mutation:** 94.7% on the Phase-02 diff; the cross-package coverage
  gap (below) closed to 100% on the affected file via an extracted pure
  helper (`75cd13fe`).

### Verification catalogue (EDD)

Two operator-surface expectations graduated from DISTILL and live in
`verification/` (already a permanent location):

- **O03** — `overdrive deploy <udp-spec>` accepted; intent carries
  `Proto::Udp`; `alloc status` renders each Service listener as
  `<port>/<protocol>`. Captured black-box; **satisfied**.
- **E02** — UDP service reverse-path is VIP-sourced (the #163 wire
  proof). Reverse-NAT map dump + cluster preflight captured; the
  end-to-end reverse path depends on single-node veth wiring (ADR-0061,
  below).

> **FINALIZE note (2026-06-03).** The executed-evidence catalogue lives
> at the repo-root `verification/` (its permanent home) — O03/E02 above.
> There is **no feature-dir catalogue**
> (`docs/feature/udp-service-support/verification/` is absent), so there
> is nothing to archive into `docs/evolution/udp-service-support/verification/`.

## Issues encountered

- **The dependency surfaced during E02 verification: single-node veth
  wiring.** Driving the real reverse-path e2e on a single node required a
  veth dataplane the serve-boot path did not yet provision. This was
  factored into the sibling feature **single-node-dataplane-wiring**
  (**ADR-0061**), which unblocked the E02 serve boot. The two features
  were delivered in parallel and interleave in the git history.
- **`dns-resolver.toml` fixture port collision.** The fixture originally
  bound UDP **5353**, which `systemd-resolved` already owns in the Lima
  VM — `bind(): Address already in use`, so the backend never started and
  surfaced two layers downstream as an empty REVERSE_NAT map. Fixed by
  moving the fixture off 5353 (commit `5a9cebd2`) and relocating it to
  the shared `examples/dns-resolver.toml`.
- **Empty-collection misdiagnosis (the RCA that produced a rule).** An
  empty REVERSE_NAT map dump was initially read as "the wiring is broken."
  It was two independent downstream symptoms compounding: (a) the
  remote-path maps (`REVERSE_NAT_MAP`/`SERVICE_MAP`) are **empty by
  design** on single-node localhost — the path under test writes
  `LOCAL_BACKEND_MAP` via `cgroup_connect4`; and (b) the `socat` backend
  had crashed on the port collision above. A TCP-vs-UDP / free-vs-occupied
  population diff isolated both at once and **falsified the
  "UDP-specific bug" hypothesis**. The lesson was distilled into
  `.claude/rules/debugging.md` **§11** ("an empty collection is a
  downstream symptom — confirm the surface, then trace the producer";
  commit `c69de27a`).
- **CLI listener-protocol rendering.** `alloc status` did not surface the
  per-listener protocol on the live path; added `<port>/<protocol>`
  rendering (commits `7e79007f`, `e9cec107`).
- **[Phase 02] Pre-existing `host_ipv4`-collapses-to-localhost-on-override
  bug.** During Phase 02 DELIVER a latent control-plane bug surfaced: on
  real-dataplane override boots, `host_ipv4` collapsed to localhost
  instead of being resolved from the configured client interface —
  breaking the backend-discovery → hydrator → `RegisterLocalBackend`
  path. Root-caused in
  `docs/analysis/root-cause-analysis-bridge-hydrator-register-local-backend.md`;
  fixed in `299a7c1b`.
- **[Phase 02] Cross-package mutation-coverage gap on the override
  fallback.** The `host_ipv4` override fallback was only reachable from
  another package's tests, leaving a mutation blind spot. Closed by
  extracting the decision into a pure helper with an in-package unit
  test (`75cd13fe`), restoring kill-rate signal on the file.

## Lessons learned

- **Audit the from-state against source before scoring options.** The
  phase-2 "locked-A" decision was a *paper* decision never landed on the
  trait. Treating it as the from-state would have mis-scored every option
  and mis-stated the blast radius. DV1 forced verification against
  `dataplane.rs:101` — the honest from-state was option C.
- **C3 forces the blast radius wider than the abstraction suggests.** The
  "5 sites / hydrator unchanged" DISCUSS estimate was low: a no-default
  invariant ("proto never defaults to Tcp") only holds if proto is
  carried from a listener-bearing fact through the Action *and* the
  desired projection — 8 sites. The trait-surface change is the visible
  tip; the provenance plumbing is the load-bearing half.
- **Infeasible test retargets are a DESIGN fact, not an in-slice SPIKE.**
  H1 (can't run the real Ebpf adapter in a pure DST process) was resolved
  at DIVERGE as a two-pronged tier split, so slice 03 didn't carry an
  open research question into delivery.
- **A green control-plane count says nothing about the lowest layer that
  can fail.** `alloc status: Running` while the backend had crashed on a
  bind collision — the §11 rule now codifies probing the producer
  (`ss -ulnp`, the workload's stderr), not the aggregate above it.

## Permanent artifacts

| Artifact | Location |
|---|---|
| Architecture SSOT | `docs/product/architecture/adr-0060-service-frontend-update-service-signature.md` |
| Dependency ADR (veth wiring) | `docs/product/architecture/adr-0061-single-node-veth-dataplane-wiring.md` |
| Acceptance scenarios + slice briefs | `docs/scenarios/udp-service-support/` |
| UX journey (flow + emotional arc + Gherkin) | `docs/ux/udp-service-support/` |
| Product-level submit journey | `docs/product/journeys/submit-a-udp-service.yaml` |
| Executed-evidence catalogue (O03, E02) | `verification/expectations/{O03-*,E02-*}/` |
| Implementation | `crates/overdrive-core/src/dataplane/service_frontend.rs`, `crates/overdrive-{sim,dataplane,control-plane,cli}` (see git history 2026-06-02 → 2026-06-03) |

## Open deferral

- **[#200](https://github.com/overdrive-sh/overdrive/issues/200) —
  unconnected-UDP (`sendto(VIP, ...)` without `connect()`).** NOT
  delivered; **connected-UDP IS delivered.** Unconnected UDP needs a
  separate `sendmsg4` (`BPF_CGROUP_UDP4_SENDMSG`) hook, not implemented
  today (DNS resolvers `sendto` per query without connecting). See
  ADR-0053 amendment § "Out of scope".

## What this unblocks

- **SCTP / further L4 protocols** — `ServiceFrontend` and both map keys
  carry `Proto`; adding a protocol is a content change to the enum + the
  per-proto fan-out, not a structural change to the call/key surface.
- **Multi-listener forward-key granularity (D8) — RESOLVED by Phase 02.**
  The SERVICE_MAP forward key is `(VIP, port, proto)`; each listener
  occupies its own slot. The Phase-01 deferral is closed.
- **The lockstep gate as a template** — any future cross-adapter dataplane
  invariant follows the Sim(Tier 1) ∪ Ebpf(Tier 2+3)-meeting-at-a-shared-key
  shape proven here.

## Links (whole feature)

- ADR-0040 (rev 2026-06-03) — `docs/product/architecture/adr-0040-service-map-three-map-split-and-hash-of-maps.md`
- ADR-0053 (rev 2026-06-03) — `docs/product/architecture/adr-0053-same-host-backend-delivery-via-cgroup-sock-addr.md`
- ADR-0060 — `docs/product/architecture/adr-0060-service-frontend-update-service-signature.md`
- ADR-0061 (sibling) — `docs/product/architecture/adr-0061-single-node-veth-dataplane-wiring.md`
- Research — `docs/research/dataplane/service-map-l4-proto-keying-research.md`
- RCA — `docs/analysis/root-cause-analysis-bridge-hydrator-register-local-backend.md`
- Feature SSOT (preserved) — `docs/feature/udp-service-support/feature-delta.md`

Phase 02 commits: `e8083ce9` (resolve P2-Q4 — proto in service-LB map
keys), `12611316` (widen SERVICE_MAP outer key — 02-01), `299a7c1b` (fix
host_ipv4 override), `0876de79` (widen LOCAL_BACKEND_MAP key — 02-02),
`75cd13fe` (in-package unit test killing the mutation gap), `1e913da2`
(record Phase 02 DES roadmap + execution-log).
