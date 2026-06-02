# Evolution — udp-service-support

**Finalized:** 2026-06-03 · **Wave lifecycle:** DISCUSS → DIVERGE → DESIGN
→ DISTILL → DELIVER (5 slices, all GREEN) · **Source brief:** GitHub
issue **#163** — *"REVERSE_NAT_MAP lockstep populates only TCP entries;
UDP responses silently bypass source rewrite."*

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
- **D8 — deferred** US-05 forward-key granularity (per-`(VIP,port)` vs
  VIP-only) to a future DESIGN; the shipped validator (VIP-only) and
  phase-2 §5 Drift-3 (`(VIP,port)`) disagreement is flagged in ADR-0060,
  not resolved here.

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

## What this unblocks

- **SCTP / further L4 protocols** — `ServiceFrontend` carries `Proto`;
  adding a protocol is a content change to the enum + the per-proto
  fan-out, not a structural change to the call surface.
- **Multi-listener forward-key granularity (D8)** — the per-`(VIP,port)`
  vs VIP-only SERVICE_MAP key question is teed up in ADR-0060 for a future
  DESIGN once a real multi-listener-on-one-VIP workload needs it.
- **The lockstep gate as a template** — any future cross-adapter dataplane
  invariant follows the Sim(Tier 1) ∪ Ebpf(Tier 2+3)-meeting-at-a-shared-key
  shape proven here.
