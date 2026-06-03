<!-- markdownlint-disable MD024 -->
# Feature Delta — udp-service-support

**Source brief:** GitHub issue #163 — "REVERSE_NAT_MAP lockstep populates
only TCP entries; UDP responses silently bypass source rewrite."

**Upstream (CLOSED, do not re-scope):** GH #164 shipped the intent-side
`Listener { port, protocol: Proto }` shape, the `Proto` enum (tcp/udp),
`[[listener]]` TOML parsing + validation, the `ListenerRow` observation
shape, and `ServiceSpec.listeners: Vec<Listener>`.

**This feature's scope (#163's core, unstarted):** thread the per-service
L4 protocol through a **`ServiceFrontend` newtype** (carrying
`(ServiceVip, port, Proto)`) on the existing `Dataplane::update_service`
call, make the production `EbpfDataplane` install REVERSE_NAT_MAP entries
matching the declared proto, and add a lockstep gate so the Sim≡Ebpf
REVERSE_NAT divergence cannot recur silently.

**Predecessor design:** `docs/feature/phase-2-xdp-service-map/design/architecture.md`
§ 5 (Q-Sig locked decision A — three explicit args). **Locked-A was a
paper decision NEVER landed on the trait.** The SHIPPED trait is Q-Sig
**option C** = `update_service(vip: Ipv4Addr, backends)` (raw `Ipv4Addr`,
no `service_id`, no `ServiceVip`), verified at
`crates/overdrive-core/src/traits/dataplane.rs:101`. **This feature
threads proto FROM shipped-C into a `ServiceFrontend` newtype; the
frontend re-absorbs `ServiceVip` (locked-A's newtype intent) but leaves
`service_id`/`correlation` on the `Action::DataplaneUpdateService`
envelope (`validate.rs:288`) by design; see D1 below.** The ADR
amendment (§5 Q-Sig C → `ServiceFrontend`, superseding the paper
locked-A) is the architect's job in DESIGN — this DISCUSS wave
forward-points to it, does not author it.

---

## Wave: DISCUSS / [REF] Artifact index

All artifacts live under `docs/feature/udp-service-support/`.

| Artifact | Purpose |
|---|---|
| `feature-delta.md` (this file) | Single narrative — REF / WHY / HOW: job grounding, scope assessment, story map, user stories (embedded AC + KPIs + elevator pitches), DoR validation, wave-decisions, self-review |
| `discuss/journey-udp-service-visual.md` | ASCII flow + emotional arc + TUI mockups + shared-artifact spine |
| `discuss/journey-udp-service.yaml` | Structured journey schema with embedded Gherkin per step (specification-only) |
| `slices/slice-01..05-*.md` | Machine-readable elephant-carpaccio slice briefs (one per release slice) |

SSOT updates landed by this wave:

| File | Change |
|---|---|
| `docs/product/journeys/submit-a-udp-service.yaml` | NEW — product-level UDP-Service submit journey (companion to `submit-a-service.yaml`; separate because this is a dataplane-wire-path concern, not a lifecycle-signal concern) |
| `docs/product/jobs.yaml` | NO new job. Stories trace to existing **J-OPS-004** + **J-PLAT-004** (rationale in WHY below). |

---

## Wave: DISCUSS / [REF] Story → job traceability

| Story | job_id | Why |
|---|---|---|
| US-01 (`ServiceFrontend` newtype migration) | J-PLAT-004 | The load-bearing abstraction that makes Sim≡Ebpf lockstep expressible. Correctness-of-reconciler-and-dataplane job. |
| US-02 (production Step 4b proto fan-out) | J-OPS-004 + J-PLAT-004 | Operator-facing: the UDP service actually works. Plus the dataplane-correctness job. |
| US-03 (reverse-NAT lockstep gate) | J-PLAT-004 | "Run a reconciler/dataplane against the simulated cluster and know it converges/matches." The ESR/lockstep invariant home. |
| US-04 (single-UDP-listener forward+reverse e2e — **walking skeleton**) | J-OPS-004 | The operator-trust outcome: submit a UDP service, the wire works both ways. |
| US-05 (ServiceMapHydrator per-listener fan-out; multi-listener TCP+UDP e2e) | J-OPS-004 | Operator submits a real multi-protocol service; both protocols' paths work end-to-end through the real CLI→control-plane→reconciler→dataplane chain. |

**Job-minting decision: NO new job.** UDP-as-first-class is not a new
motivation — it is the same J-OPS-004 operator-trust contract ("trust
the wire signal for a Service-kind workload") and the same J-PLAT-004
correctness contract ("the dataplane lockstep/ESR invariant is
mechanically checked"), now extended to the UDP protocol dimension.
vision.md principle 4 ("all workload types are first class") makes UDP
an *extension* of existing jobs, not a sibling. Minting a J-OPS-005
"submit a UDP service" would fragment the operator-trust job by protocol
— a smell (we would then owe a J-OPS-006 for SCTP, etc.). Per the
instruction's recommendation, traced to BOTH existing jobs.

---

## Wave: DISCUSS / [WHY] Job grounding (lightweight JTBD — job-story format)

Per Wizard Decision 4 (lightweight JTBD, no full four-forces; motivation
distilled from whitepaper/vision, not interviews).

**Job story (J-OPS-004 extension):**

> When I, an Overdrive platform engineer, submit a Service-kind workload
> that declares a `protocol = "udp"` listener, I want the platform to
> load-balance UDP traffic such that the backend's response is
> source-rewritten back to the VIP, so I can run UDP-bearing services
> (DNS, QUIC edge, game servers, syslog) and trust that the connection
> works both ways — never leaking the backend IP and silently breaking
> the client.

**Job story (J-PLAT-004 extension):**

> When I change the dataplane's service-update path, I want the
> SimDataplane and the production EbpfDataplane to be provably equivalent
> on their REVERSE_NAT key sets for every protocol, so a forward/reverse
> asymmetry cannot reach production undetected (the #163 failure mode).

**The single force that matters (lightweight):** the **anxiety of
silent asymmetry**. UDP reverse-path bugs are the worst class of
dataplane defect because `deploy` succeeds, `alloc status` shows
Running, and the failure only surfaces when a real client times out.
The whole design intent is to convert that silence into a loud,
mechanical PR-time gate failure (US-03).

---

## Wave: DISCUSS / [WHY] Scope assessment (Elephant Carpaccio early gate)

Run BEFORE journey-visualization investment per the phase-2 gate.

| Oversized signal | This feature |
|---|---|
| >10 user stories | NO — 5 stories |
| >3 bounded contexts | NO — 3 (`overdrive-core` trait + `overdrive-dataplane`/`overdrive-bpf`, `overdrive-sim`, `overdrive-control-plane` hydrator). The trait change touches core but the behavior lives in ≤3 contexts. |
| walking skeleton >5 integration points | NO — the skeleton (US-04) is a single UDP listener forward+reverse e2e, ~3 integration points (CLI submit, dataplane attach, wire capture) |
| effort >2 weeks | NO — ~5–6 days total |
| multiple independent shippable outcomes | NO — one outcome (UDP wire path works both ways), sliced thin |

**`## Scope Assessment: PASS — 5 stories, 3 contexts, estimated 5–6 days.**
Right-sized for a single DESIGN wave. No split proposed.

---

## Wave: DISCUSS / [REF] Story map

**Persona:** Ana, platform engineer. **Goal:** submit a UDP service and
trust the forward+reverse wire path.

### Backbone (operator activities, chronological)

| Declare | Submit & commit | Load-balance | Verify both ways | Guard forever |
|---|---|---|---|---|
| Write `protocol="udp"` listener (shipped #164) | `overdrive deploy` → intent → hydrator | Dataplane installs forward + reverse entries | Real UDP round-trip; response sourced from VIP | Lockstep gate Sim≡Ebpf |

### Ribs (tasks under each activity)

- **Declare:** udp listener parse/validate (shipped). Multi-listener TCP+UDP (US-05).
- **Submit & commit:** thread proto into the `ServiceFrontend` newtype (US-01); hydrator emits per-listener `update_service` (US-05).
- **Load-balance:** production Step 4b REVERSE_NAT proto fan-out (US-02).
- **Verify both ways:** single-listener e2e (US-04, **walking skeleton**); multi-listener e2e (US-05).
- **Guard forever:** retarget ReverseNatLockstep to both adapters + Tier 2 BPF_PROG_TEST_RUN UDP triptych (US-03).

### Walking skeleton (thinnest end-to-end slice)

**US-04 — single UDP listener forward + reverse e2e.** One task from each
activity: declare a udp listener → submit → dataplane installs the udp
REVERSE_NAT entry → a real UDP datagram round-trips with the response
sourced from the VIP. This is the minimum that connects ALL activities
and is the riskiest assumption ("does the reverse path actually rewrite
proto=17?"). It depends on US-01 (`ServiceFrontend` newtype) + US-02
(production fan-out) as foundation — see Priority Rationale for the
load-bearing sequencing nuance.

### Release slices (sliced by outcome, not technical layer)

- **Slice 01 / US-01 — `ServiceFrontend` newtype trait migration** (the load-bearing abstraction). Outcome: `update_service(frontend, backends)` carries `(ServiceVip, port, Proto)` in the frontend, `backends` separate, `service_id`/`correlation` on the Action; Sim+Ebpf both consume it. No PRODUCTION proto behavior change yet (Ebpf still installs what it installed); the Sim over-broad `[Tcp,Udp]` fan-out is narrowed to `frontend.proto`.
- **Slice 02 / US-02 — Production Step 4b proto fan-out.** Outcome: EbpfDataplane installs REVERSE_NAT entries matching `frontend.proto`; Sim≡Ebpf diff goes to zero.
- **Slice 03 / US-03 — Lockstep gate (Tier 1 Sim set-equality AND Tier 3 Ebpf acceptance AND Tier 2 UDP triptych).** Outcome: the divergence cannot recur silently.
- **Slice 04 / US-04 — Single-UDP-listener forward+reverse e2e (Tier 3).** Outcome: operator-verifiable UDP round-trip with VIP source.
- **Slice 05 / US-05 — ServiceMapHydrator per-listener fan-out (multi-listener TCP+UDP e2e, Tier 3).** Outcome: a real two-listener service works on both protocols through the full chain.

### Priority Rationale

Priority order is driven by outcome impact **and** the "ship the
abstraction first" taste test (per `nw-user-story-mapping` + the
instruction's explicit watch).

1. **US-01 (P0) first — the `ServiceFrontend` newtype abstraction is load-bearing.**
   Every other slice composes onto the new `update_service(frontend, backends)`
   signature. Sequencing it first avoids the anti-pattern of building
   US-02's proto fan-out against the shipped option-C signature
   (`update_service(vip, backends)`) and then re-migrating. This is
   deliberate "ship the abstraction first" — but US-01 is a *pure refactor
   with zero PRODUCTION proto behavior change* (the Sim's over-broad
   `[Tcp,Udp]` fan-out IS corrected to `frontend.proto` — see US-01 H2
   note), so it is demonstrable (the existing TCP e2e still passes)
   without leaving a half-built abstraction.
2. **US-02 (P0) — the actual bug fix.** Production fan-out matching
   `frontend.proto`. Highest J-OPS-004 movement: this is the line
   that was broken in #163.
3. **US-03 (P0) — the gate.** Without it, US-02 can silently regress;
   highest J-PLAT-004 movement. Riskiest-assumption guard.
4. **US-04 (P1, walking skeleton) — operator-verifiable proof.** Depends
   on US-01+US-02 landing; this is the e2e that an operator can run.
   It is the *walking skeleton by journey coverage* (touches every
   activity) even though it sequences after the abstraction, because the
   abstraction+fix are prerequisites for an honest e2e.
5. **US-05 (P2) — multi-listener fan-out.** The richest operator
   outcome (TCP+UDP on one service) but composes onto everything above;
   lowest urgency because single-listener UDP (US-04) already delivers
   the core value.

> Note on "walking skeleton sequenced after abstraction": the skeleton
> is the thinnest slice that exercises the *whole journey*, which is
> US-04. US-01/02/03 are the foundation US-04 stands on. The map draws
> the skeleton at US-04; the build order lands its prerequisites first.
> This is the standard brownfield shape (Wizard Decision 2:
> Depends/brownfield).

---

## Wave: DISCUSS / [REF] System Constraints (cross-cutting)

| ID | Constraint |
|---|---|
| C1 | Solution-neutral on kernel mechanics — DISCUSS does not pick checksum helpers, map layout, or attach mode (DESIGN owns these; phase-2 architecture.md already locked most). |
| C2 | The `ServiceFrontend` newtype is the SINGLE source of `(vip, port, proto)`. `service_id` and `correlation` remain `Action::DataplaneUpdateService`-envelope fields OUTSIDE the frontend BY DESIGN (action-routing/correlation concern, not a dataplane-key concern — the SERVICE_MAP key is `(VIP, port)` and the REVERSE_NAT key is `BackendKey{ip,port,proto}`; neither is keyed by `service_id`). **Pass condition (grep-checkable):** no call site reconstructs `(vip, port, proto)` from separate positional args after US-01 lands — `service_id` travelling separately on the Action is explicitly allowed and is NOT a C2 violation. |
| C3 | Proto is NEVER defaulted to Tcp anywhere on the intent→hydrator→`ServiceFrontend`→dataplane path. Every `Proto::Tcp` literal on that path must derive from `frontend.proto` or be a test fixture. |
| C4 | `BackendKey` newtype (`(ip,port,proto)`) remains the REVERSE_NAT key type — no raw-tuple keys (newtype-STRICT per `.claude/rules/development.md`). |
| C5 | Production code is not shaped by simulation — the lockstep gate reshapes the *test/invariant*, never adds sim-only arms to production `update_service`. |
| C6 | Single-cut migration — the shipped `update_service(vip: Ipv4Addr, backends)` signature (option C) is replaced with `update_service(frontend: ServiceFrontend, backends)`, not deprecated-alongside. Old call sites migrate in the same PR as US-01. |
| C7 | DST determinism — any new collection iterated by the lockstep invariant uses `BTreeMap`/`BTreeSet` (the existing `reverse_nat_keys_for` already collects into `BTreeSet`). |
| C8 | Tier mapping is mandatory: US-03 lands a Tier 2 `BPF_PROG_TEST_RUN` UDP triptych; US-04/US-05 land Tier 3 real-veth e2e behind `integration-tests`. |

---

## Wave: DISCUSS / [REF] User stories

> Combined-file heading discipline: stories use `##` titles; subsections
> use `###`. `markdownlint-disable MD024` set at top of file (repeated
> subsection headings).

## US-01: Thread the per-service protocol through a `ServiceFrontend` newtype

### Elevator Pitch
- **Before:** the SHIPPED `Dataplane::update_service(vip: Ipv4Addr, backends)`
  (option C, `dataplane.rs:101`) carries no protocol; the dataplane
  cannot know whether a service is TCP or UDP, so REVERSE_NAT entries
  are hard-coded to one proto.
- **After:** `overdrive deploy dns-resolver.toml` (with a udp
  listener) commits an intent whose protocol flows into a typed
  `ServiceFrontend`; the existing TCP e2e (`overdrive deploy web.toml`)
  still prints `Service 'web' is stable` unchanged — the migration is
  production-behavior-preserving. Observable proof: the existing
  `service_map_forward` Tier 3 test stays green, and the new frontend
  carries `proto: Udp` for the udp listener.
- **Decision enabled:** Ana (and the dataplane code) can rely on a
  single typed surface for "what protocol is this service?" instead of
  guessing — the precondition for the #163 fix.

### Problem
Ana is a platform engineer who runs both TCP and UDP services. She finds
it impossible to get correct UDP load-balancing because the dataplane's
shipped `update_service(vip: Ipv4Addr, backends)` surface has no protocol
at all — the kernel-side reverse-NAT key needs `(ip, port, proto)` but
the trait only supplies `(vip, backends)`. The protocol is stranded in
the intent layer and never reaches the dataplane.

### Who
- Platform engineer (Ana) | submitting Service-kind workloads with mixed L4 protocols | wants one typed surface, not positional-arg sprawl.
- Internal: dataplane + reconciler code | consuming `update_service` | needs `(vip,port,proto)` from one typed home without reconstructing it from scattered args.

### Solution
Replace the shipped `update_service(vip: Ipv4Addr, backends)` (option C)
with `update_service(frontend: ServiceFrontend, backends)`, where
`ServiceFrontend` is a newtype carrying `(ServiceVip, port, Proto)` and
`backends: Vec<Backend>` stays a SEPARATE positional argument (phase-2
architecture.md §5 Q-Sig **C → `ServiceFrontend`**; locked-A was a paper
decision never landed). **The frontend re-absorbs `ServiceVip`**
(locked-A's typed-VIP intent — the validated VIP that shipped-option-C
dropped to a raw `Ipv4Addr`) but **`service_id` and `correlation` STAY on
the `Action::DataplaneUpdateService` envelope** (`validate.rs:288`) by
design: they are action-routing/correlation concerns, not dataplane-key
concerns (the SERVICE_MAP key is `(VIP, port)`, the REVERSE_NAT key is
`BackendKey{ip,port,proto}`; neither is keyed by `service_id`). Both
SimDataplane and EbpfDataplane consume the frontend. **No PRODUCTION proto
behavior change in this slice** — Ebpf still installs exactly what it
installed before; the Sim's over-broad `[Tcp,Udp]` fan-out is CORRECTED
to `frontend.proto` (see Domain Example 3 / AC). This is the pure
abstraction that US-02 then exploits.

### Domain Examples
#### 1: Happy Path — TCP service unchanged in production
Ana submits `web.toml` (tcp/8080, backend 10.244.0.10:8080). After the
migration, the frontend carries `proto: Tcp`; the production forward+reverse
TCP path behaves exactly as before; `service_map_forward` Tier 3 test green.

#### 2: Carries the new dimension — UDP frontend
Ana submits `dns-resolver.toml` (udp/5353). The `ServiceFrontend` reaching
`update_service` carries `proto: Udp`. (Production reverse-NAT behavior
still pre-fix: US-02 makes the Ebpf reverse-NAT honor it.)

#### 3: Sim fan-out corrected (H2) — TCP-only Sim key set narrows
Before US-01, the Sim's `reverse_nat_keys_for` hardcodes `[Tcp, Udp]`
(`sim/dataplane.rs:277`), so a TCP-only `web.toml` service's Sim
REVERSE_NAT key set is `{(ip,8080,tcp), (ip,8080,udp)}` — an over-broad
fan-out. After US-01 narrows it to `frontend.proto`, the Sim key set for
the same TCP service becomes exactly `{(ip,8080,tcp)}`. This is the
intended correction (Sim was installing a phantom udp key); any existing
test/invariant asserting the two-proto Sim fan-out is updated in the same
single-cut PR.

### UAT Scenarios (BDD)
#### Scenario: TCP service round-trips unchanged in production after the migration
Given Ana has web.toml with a tcp listener on 8080 and one backend
When Ana runs `overdrive deploy web.toml`
Then the service reaches stable and the production forward+reverse TCP path works exactly as before the migration

#### Scenario: The frontend carries the declared protocol
Given Ana has dns-resolver.toml with a udp listener on 5353
When the service is hydrated and `update_service` is invoked
Then the `ServiceFrontend` reaching the dataplane carries proto Udp, not Tcp

#### Scenario: The protocol triple lives only in the frontend
Given the `update_service` migration has landed
When a reviewer greps the call path for the protocol
Then `(vip, port, proto)` is read from the single `ServiceFrontend` at every call site, never reassembled from separate arguments — while `service_id` and `correlation` travelling separately on the `Action::DataplaneUpdateService` envelope is allowed and is NOT a violation

### Acceptance Criteria
- [ ] `Dataplane::update_service` takes `(frontend: ServiceFrontend, backends: Vec<Backend>)` where `ServiceFrontend` carries `(ServiceVip, port, Proto)`; the shipped `(vip: Ipv4Addr, backends)` signature is gone (single-cut). `backends` stays a separate positional arg.
- [ ] The frontend re-absorbs `ServiceVip`; `service_id`/`correlation` remain on the `Action::DataplaneUpdateService` envelope (NOT folded into the frontend).
- [ ] **C2 pass condition:** no call site reconstructs `(vip, port, proto)` from separate positional args (grep-verified); `service_id` on the Action is explicitly permitted and is not a violation.
- [ ] Both SimDataplane and EbpfDataplane consume the frontend; the existing TCP Tier 3 tests stay green.
- [ ] The udp-listener frontend carries `proto: Udp` end-to-end from intent.
- [ ] Zero PRODUCTION proto behavior change in the Ebpf reverse-NAT path in this slice (verified: REVERSE_NAT entries identical to pre-migration for the TCP case).
- [ ] The Sim `reverse_nat_keys_for` fan-out is corrected from the over-broad `[Tcp, Udp]` hardcode to `frontend.proto`; any existing test/invariant asserting the two-proto Sim fan-out is updated in the same PR.

### Outcome KPIs
- **Who:** dataplane + reconciler code paths consuming `update_service`.
- **Does what:** read `(vip,port,proto)` from one typed `ServiceFrontend` surface.
- **By how much:** 100% of `update_service` call sites read the typed frontend; 0 positional-triple reconstructions (grep-verified).
- **Measured by:** code review grep + the existing TCP Tier 3 suite staying green.
- **Baseline:** 0% — protocol is absent from the shipped trait (option C) entirely today.

### Technical Notes
- Threads proto FROM shipped option C (`update_service(vip: Ipv4Addr, backends)`, `dataplane.rs:101`) → `ServiceFrontend`; locked-A (`update_service(service_id, vip: ServiceVip, backends)`, architecture.md §5:155) was a paper decision NEVER implemented. The frontend re-absorbs ServiceVip (locked-A's newtype intent) but leaves `service_id` on the Action envelope.
- The exact newtype field names/derives and whether `port` is `NonZeroU16` are DESIGN's call (P1-Q2); DISCUSS locks the *family* (thread-proto-as-typed-field, frontend re-absorbs ServiceVip, service_id on Action). The §5 Q-Sig amendment (C → ServiceFrontend) is the architect's job in DESIGN (forward-point only).
- Single-cut migration (C6); all call sites in the same PR. True blast radius = **8 sites**: trait, `ServiceFrontend` (new), SimDataplane, EbpfDataplane, action-shim dispatch, ReverseNatLockstep invariant, **`Action::DataplaneUpdateService` (+ proto)**, and **`ServiceDesired` + the observation→desired projection (+ proto)**. The DISCUSS "5 sites / hydrator unchanged" estimate was low: C3 (no `Tcp` default) requires the Action and the desired projection to carry proto from a **listener-bearing fact** (`ListenerRow` / `BackendDiscoveryBridge` per-listener projection — NOT `service_backends`, which carries neither port nor proto; ATLAS-1 b). If no listener proto can be resolved, that is an error (Failed/structured), never a silent `Proto::Tcp` default. The hydrator's *multi-listener fan-out* is still a separate US-05 concern.

  > **Changed Assumptions (DISTILL, 2026-06-02):** per
  > `design/upstream-changes.md` Correction 2 (ADR-0060 D6 + ATLAS-1 b),
  > the original "Blast radius = 5 sites … hydrator UNCHANGED" was low.
  > Sites 7–8 (the `Action` + the `ServiceDesired`/obs→desired
  > projection) are added because C3 is satisfiable only if proto is
  > carried end-to-end from a listener-bearing fact. The DISTILL C3-guard
  > scenarios S-01-C/D/E pin this provenance and make a silent `Tcp`
  > default a failing test.

- **Acceptance Criteria addition (DISTILL):** the protocol dimension is added to `Action::DataplaneUpdateService` and `ServiceDesired`; the desired projection reads it from a listener-bearing fact (`ListenerRow` and/or the `BackendDiscoveryBridge` per-listener projection), never from the proto-less `service_backends` row, and if no listener proto can be resolved that is an error (Failed/structured), never a silent `Proto::Tcp` default (C3).
- `Backend` already carries `addr` (ip:port); the frontend adds the service-level `(ServiceVip, port, Proto)`.

---

## US-02: Production EbpfDataplane installs REVERSE_NAT entries matching the declared proto

### Elevator Pitch
- **Before:** `overdrive deploy dns-resolver.toml` succeeds, the
  service runs, but the UDP backend's response hits
  `xdp_reverse_nat_lookup` with proto=17, finds NO entry, and returns
  `XDP_PASS` without rewriting the source — the client gets a response
  from the backend IP and the connection breaks (the #163 bug).
- **After:** `overdrive deploy dns-resolver.toml` installs a
  REVERSE_NAT_MAP entry `(backend_ip, 5353, udp) → vip`; the backend's
  UDP response is source-rewritten to the VIP. Observable proof: a
  `bpftool map dump` of REVERSE_NAT_MAP shows the udp-keyed entry; the
  Tier 3 wire capture (US-04) shows the VIP source.
- **Decision enabled:** Ana can deploy a UDP service and trust the
  reverse path works — she can stop avoiding UDP workloads on Overdrive.

### Problem
Ana is a platform engineer who needs DNS/QUIC/game-server (UDP) services
to work. She finds it impossible because the production
`EbpfDataplane::update_service` Step 4b inserts REVERSE_NAT entries with
`proto = Tcp` ONLY — so every UDP response silently bypasses source
rewrite, and her clients time out with no diagnostic anywhere in the
platform.

### Who
- Platform engineer (Ana) | deploying UDP-bearing services | needs the reverse path to rewrite proto=17 traffic.
- Internal: `xdp_reverse_nat_lookup` kernel program | doing the lookup | needs the udp-keyed entry to exist.

### Solution
In `EbpfDataplane::update_service` Step 4b, install REVERSE_NAT_MAP
entries per-backend per-`frontend.proto` (mirroring SimDataplane's
`reverse_nat_keys_for` shape, now itself narrowed to `frontend.proto` in
US-01), so the production REVERSE_NAT key set for a UDP service includes
the `(ip,port,udp)` entries. The diff between Sim and Ebpf goes to zero.

### Domain Examples
#### 1: Happy Path — UDP entry installed
Ana submits `dns-resolver.toml` (udp/5353, backend 10.244.0.20:5353).
REVERSE_NAT_MAP gains `(10.244.0.20, 5353, udp) → 10.96.0.10`. The
backend's reply is source-rewritten to 10.96.0.10.

#### 2: Edge — TCP unaffected
Ana submits `web.toml` (tcp/8080). REVERSE_NAT_MAP gains the tcp-keyed
entry exactly as before; no udp entry is spuriously added.

#### 3: Error/Boundary — empty backend set removes only THIS proto's entries

> **Changed Assumptions (DISTILL, 2026-06-02):** per
> `design/upstream-changes.md` Correction 1 (ADR-0060 D4 — per-proto
> purge), the original heading "removes **both protos'** entries"
> contradicted D4 and the example body itself. Empty-backends purge is
> **per-proto**: only `frontend.proto`'s keys are removed; a co-resident
> other-proto frontend on the same VIP survives.

Ana scales `dns-resolver` (udp/5353) to 0 backends. The update removes the
`(10.244.0.20, 5353, udp)` entry only; a co-resident tcp frontend on the
same VIP (installed by a separate `update_service` call) keeps its
`(…, tcp)` entries. Cross-service shared-backend keys are preserved by the
`live_keys` difference check — no stale udp entry lingers, no live tcp
entry is collaterally purged.

### UAT Scenarios (BDD)
#### Scenario: UDP service gains a reverse-NAT entry keyed by udp
Given Ana submits a udp Service with a backend at 10.244.0.20:5353
When the EbpfDataplane processes the update
Then REVERSE_NAT_MAP contains the key (10.244.0.20, 5353, udp) mapping to the VIP

#### Scenario: Sim and Ebpf REVERSE_NAT key sets match for UDP
Given the same `ServiceFrontend` with a udp listener
When both adapters process it
Then their REVERSE_NAT key sets are byte-identical

#### Scenario: Removing all backends purges the udp entry
Given a udp Service with one backend whose reverse-NAT entry is installed
When the service scales to zero backends
Then the udp-keyed REVERSE_NAT entry is removed and no stale entry remains

### Acceptance Criteria
- [ ] `EbpfDataplane` Step 4b installs REVERSE_NAT_MAP entries for `frontend.proto` (per-backend per-proto fan-out).
- [ ] For a udp service, `bpftool map dump` of REVERSE_NAT_MAP shows the `(ip,port,udp)` entry.
- [ ] The Sim-vs-Ebpf REVERSE_NAT key-set diff for a udp service is empty.
- [ ] Empty-backend updates purge the udp entry (no stale lingering entry).

### Outcome KPIs
- **Who:** operators submitting UDP Service-kind workloads.
- **Does what:** get a working reverse path (response sourced from VIP).
- **By how much:** 100% of UDP services have their `(ip,port,udp)` REVERSE_NAT entry installed (was 0%).
- **Measured by:** Tier 2/Tier 3 assertion on REVERSE_NAT_MAP contents + wire capture source address.
- **Baseline:** 0% — production installs only tcp entries today.

### Technical Notes
- Mirrors `reverse_nat_keys_for` (crates/overdrive-sim/src/adapters/dataplane.rs:266) — but driven by `frontend.proto`, not a hard-coded `[Tcp, Udp]` (the hardcode is itself narrowed to `frontend.proto` in US-01).
- Depends on US-01 (the `ServiceFrontend` newtype supplies the proto).
- Cross-service purge logic must mirror the Sim adapter's `difference`/`live_keys` check (architecture.md / existing Sim shape).

---

## US-03: Reverse-NAT lockstep gate exercises BOTH adapters so the divergence cannot recur

### Elevator Pitch
- **Before:** the `ReverseNatLockstep` invariant runs only against
  SimDataplane, so the DST suite never compares Sim vs production — the
  exact gap that let #163 ship.
- **After:** the lockstep is pinned by a two-pronged gate meeting at the
  shared `BackendKey` set — **Tier 1** Sim set-equality (the
  `ReverseNatLockstep` invariant asserts the Sim installs exactly the
  declared-`frontend.proto` key set), **Tier 3 acceptance** drives the
  real `EbpfDataplane.update_service(frontend_udp)` and asserts `bpftool
  map dump REVERSE_NAT_MAP` shows `(ip,port,udp)` + a wire capture with
  VIP source, **and Tier 2** a `BPF_PROG_TEST_RUN` triptych asserts
  `xdp_reverse_nat_lookup` rewrites a proto=17 response. Observable proof:
  `cargo dst` (Tier 1) + `cargo xtask bpf-unit` (Tier 2) + the Tier 3
  acceptance all fail loudly if a UDP fan-out is dropped from either adapter.
- **Decision enabled:** any future engineer changing the dataplane gets
  a PR-time signal instead of an at-3am incident — they can trust the
  lockstep holds.

### Problem
Ana (and every future dataplane author) is a platform engineer who needs
to trust that the simulated and real dataplanes behave identically. She
finds it impossible today because the lockstep invariant only watches
the sim adapter — a production-only divergence (like #163) is invisible
to the entire test stack until a client times out in production.

### Who
- Platform engineer / future dataplane author (Ana, and her teammates) | changing `update_service` | needs a gate that compares both adapters.
- Skeptic (CI) | running per-PR | must reject a dropped UDP fan-out.

### Solution
A pure Tier-1 retarget of `ReverseNatLockstep` against the REAL
`EbpfDataplane` is **infeasible** — the real adapter loads BPF programs
and needs a kernel + bpffs, while DST is pure-Rust in-process (review
H1, resolved by DIVERGE). The honest pinning is two-pronged, meeting at
the shared `BackendKey` set:
- **Tier 1 (Sim set-equality, per-PR critical path):** narrow
  `reverse_nat_keys_for`'s `[Tcp, Udp]` hardcode to `frontend.proto`
  (US-01) and assert the SimDataplane installs exactly the declared-proto
  `BTreeSet<BackendKey>`.
- **Tier 3 acceptance (real Ebpf, integration lane):** drive the real
  `EbpfDataplane.update_service(frontend_udp)` and assert `bpftool map
  dump REVERSE_NAT_MAP` contains `(ip,port,udp)` plus a wire capture with
  the VIP source — the production-adapter half of the equality.
- **Tier 2 (`BPF_PROG_TEST_RUN` triptych):** drive `xdp_reverse_nat_lookup`
  with a proto=17 UDP response packet and assert the source rewrite to
  the VIP fires.

The "byte-identical set across BOTH adapters" claim is pinned by Sim
(Tier 1) ∪ Ebpf (Tier 2 + Tier 3) meeting at the shared `BackendKey`
set — NOT by running both inside one DST process. The `ServiceFrontend`
twin shape (the forward twin of `BackendKey`) makes the Tier-1 expected
set a one-liner.

### Domain Examples
#### 1: Happy Path — gate passes when both adapters fan out udp
Both Sim and Ebpf install `(10.244.0.20, 5353, udp) → vip`. The lockstep
invariant sees identical key sets; the Tier 2 triptych sees the source
rewritten. Green.

#### 2: Regression caught — drop the production udp fan-out
A hypothetical edit reverts US-02 (Ebpf installs tcp only). The lockstep
gate fails: `Sim has (10.244.0.20,5353,udp), Ebpf missing it`. PR blocked.

#### 3: Kernel-level proof — proto=17 packet rewritten
The Tier 2 triptych feeds `xdp_reverse_nat_lookup` a synthetic UDP
response packet (proto=17) whose source is the backend; CHECK asserts
the output packet's source is the VIP.

### UAT Scenarios (BDD)
#### Scenario: Lockstep gate enforces key-set equality across adapters
Given the same udp `ServiceFrontend` driven through SimDataplane (Tier 1) and the real EbpfDataplane (Tier 3 acceptance)
When the lockstep gate runs in CI
Then the Tier-1 Sim set-equality and the Tier-3 Ebpf `bpftool` dump meet at the same `BackendKey` set and pass

#### Scenario: Dropping the production UDP fan-out fails the gate
Given a change that makes EbpfDataplane install only tcp reverse-NAT entries
When the lockstep gate runs
Then the gate fails before merge, naming the missing udp key

#### Scenario: Kernel rewrites a UDP response source to the VIP
Given a Tier 2 BPF_PROG_TEST_RUN triptych with a proto=17 response packet sourced from the backend
When `xdp_reverse_nat_lookup` runs against the populated REVERSE_NAT_MAP
Then the output packet's source 5-tuple is rewritten to the VIP

### Acceptance Criteria
- [ ] **Tier 1 (per-PR critical path):** the `ReverseNatLockstep` invariant asserts the SimDataplane installs exactly the declared-`frontend.proto` `BTreeSet<BackendKey>` for a udp service (Sim set-equality), and FAILS if the Sim fan-out drops the udp key.
- [ ] **Tier 3 acceptance (integration lane):** driving the real `EbpfDataplane.update_service(frontend_udp)` shows `bpftool map dump REVERSE_NAT_MAP` containing `(ip,port,udp)` and a wire capture sourced from the VIP — the production-adapter half of the equality.
- [ ] **Tier 2:** a `BPF_PROG_TEST_RUN` triptych asserts `xdp_reverse_nat_lookup` rewrites a proto=17 response source to the VIP.
- [ ] A dropped UDP fan-out in the Sim adapter fails the Tier-1 gate at PR time; a dropped fan-out in the Ebpf adapter fails the Tier-3 acceptance.
- [ ] The Tier-1 Sim set-equality is on the per-PR critical path (not nightly-only); the Tier-3 Ebpf acceptance runs in the integration lane.

### Outcome KPIs
- **Who:** dataplane authors (Ana + teammates).
- **Does what:** receive a PR-time failure on any Sim/Ebpf REVERSE_NAT divergence.
- **By how much:** 100% of REVERSE_NAT proto-fan-out divergences caught pre-merge (was 0% — #163 shipped undetected).
- **Measured by:** the Tier-1 Sim set-equality gate on the per-PR CI critical path + the Tier-3 Ebpf acceptance in the integration lane, plus a deliberately-broken-fan-out test proving each fails.
- **Baseline:** 0% — invariant runs against Sim only today.

### Technical Notes
- `reverse_nat_lockstep.rs` (crates/overdrive-sim/src/invariants/) is the Tier-1 site — it asserts the Sim set-equality over `frontend.proto` (the `[Tcp,Udp]` hardcode at `reverse_nat_lockstep.rs:158-165` is narrowed to `frontend.proto` in US-01). The real-Ebpf half is a SEPARATE Tier-3 acceptance test (a pure in-process retarget against the real adapter is infeasible — needs a kernel + bpffs; H1 resolved at DIVERGE, no in-slice SPIKE needed).
- Tier 2 triptych follows the `xdp_reverse_nat` existing test shape (crates/overdrive-bpf, `BPF_PROG_TEST_RUN`).
- The exact `ServiceFrontend` newtype shape that the gate projects to `BackendKey` is DESIGN's call (P1-Q2); the gate's set-equality logic is independent of the field names.
- Per `.claude/rules/development.md` § "Trait definitions specify behavior" — this is the DST equivalence test the contract demands.

---

## US-04: Submit a single UDP listener service and see the round-trip complete with VIP source (walking skeleton)

### Elevator Pitch
- **Before:** there is no end-to-end proof that a UDP service works both
  ways on Overdrive — the forward path is tested, the reverse path is
  not.
- **After:** Ana runs `overdrive deploy dns-resolver.toml`, sends a
  real UDP datagram to the VIP, and a `tcpdump` capture on the client
  veth shows the reply sourced from `10.96.0.10:5353` (the VIP), not the
  backend. Observable proof: the Tier 3 capture line
  `IP 10.96.0.10.5353 > 10.244.0.5.51000: UDP`.
- **Decision enabled:** Ana can confidently deploy a real UDP workload —
  she has run the exact command and seen the exact wire behavior.

### Problem
Ana is a platform engineer who needs to *verify*, not assume, that her
UDP service works. She finds it impossible today because no e2e exercises
the reverse path — the only way to discover the #163 asymmetry is to
deploy and watch a real client time out.

### Who
- Platform engineer (Ana) | running a real `overdrive deploy` for a UDP service | wants to see the round-trip on the wire.

### Solution
A Tier 3 e2e (real veth, behind `integration-tests`): submit a
single-UDP-listener Service through the real CLI → control-plane →
reconciler → EbpfDataplane chain, send a UDP datagram client→VIP, assert
the backend's reply is captured with the VIP as source.

### Domain Examples
#### 1: Happy Path — DNS resolver round-trip
`dns-resolver.toml` (udp/5353). Client at 10.244.0.5:51000 queries
10.96.0.10:5353. Reply captured with source 10.96.0.10:5353. Pass.

#### 2: Edge — multiple datagrams, same source rewrite
Client sends 3 datagrams; all 3 replies are sourced from the VIP
(UDP is connectionless — each reply independently rewritten).

#### 3: Error/Boundary — backend down, no response (not a rewrite bug)
Backend not bound on 5353 → no reply at all (distinct from "reply with
wrong source"). The test distinguishes "no response" from "response with
backend source" — only the latter is the #163 defect.

### UAT Scenarios (BDD)
#### Scenario: UDP service round-trip carries the VIP source
Given Ana submits dns-resolver.toml (udp/5353) and a backend bound on 5353
When a client sends a UDP datagram to the VIP and the backend replies
Then a wire capture on the client veth shows the reply sourced from the VIP

#### Scenario: Every UDP reply is independently source-rewritten
Given the same running UDP service
When the client sends three datagrams to the VIP
Then all three replies are captured with the VIP as source

#### Scenario: A missing backend response is distinguished from a wrong-source response
Given a UDP service whose backend is not bound on the listener port
When the client sends a datagram to the VIP
Then no reply is captured, and the test does NOT report a source-rewrite failure

### Acceptance Criteria
- [ ] Tier 3 e2e submits a single-UDP-listener Service through the real CLI→control-plane→reconciler→EbpfDataplane chain.
- [ ] A `tcpdump`/AF_PACKET capture on the client side shows the reply sourced from the VIP, not the backend.
- [ ] The test is gated behind `integration-tests` and runs via `cargo xtask lima run`.
- [ ] The test distinguishes "no response" from "response with backend source".

### Outcome KPIs
- **Who:** operators (Ana) deploying UDP services.
- **Does what:** verify the UDP round-trip works both ways with one command + one capture.
- **By how much:** the e2e passes deterministically across seeds (≥99/100), proving the reverse path for UDP.
- **Measured by:** the Tier 3 capture assertion (source address == VIP).
- **Baseline:** no such e2e exists; reverse path for UDP unverified.

### Technical Notes
- Follows `reverse_nat_e2e` / `service_map_forward` Tier 3 shape (crates/overdrive-dataplane/tests/integration/).
- Depends on US-01 + US-02. The walking-skeleton-by-journey-coverage slice.
- Uses the `overdrive-testing` netns/veth fixtures (`ThreeIfaceTopology`).

---

## US-05: Submit a multi-listener (TCP + UDP) service and have both protocols' paths work end-to-end

### Elevator Pitch
- **Before:** the ServiceMapHydrator emits a single `update_service`
  per service; a multi-listener service (TCP + UDP) cannot install both
  protocols' dataplane entries.
- **After:** Ana runs `overdrive deploy edge.toml` (tcp/8080 +
  udp/8081); the hydrator emits one `update_service` per listener with
  the spec-declared proto, and BOTH the TCP forward+reverse path AND the
  UDP forward+reverse path work. Observable proof: the accepted line
  shows both listeners, and two Tier 3 captures (one per protocol) show
  the VIP source.
- **Decision enabled:** Ana can run real-world services that speak both
  protocols on one VIP (e.g. a QUIC+HTTP edge endpoint) without splitting
  them into two workloads.

### Problem
Ana is a platform engineer who runs services that listen on both TCP and
UDP (DNS-over-TCP+UDP, QUIC+HTTP/2 edge). She finds it impossible today
because the hydrator collapses a multi-listener service to one
`update_service` call — one protocol wins, the other's dataplane entries
are never installed.

### Who
- Platform engineer (Ana) | running dual-protocol services on one VIP | needs every listener's path installed.

### Solution
`ServiceMapHydrator` (ADR-0042) reads `Vec<Listener>` from the intent
Service aggregate and emits one `update_service` call per listener, each
carrying the spec-declared Proto. Tier 3: a Service with two listeners
(TCP 8080 + UDP 8081) through the real chain; both protocols' forward +
reverse paths work.

### Domain Examples
#### 1: Happy Path — TCP+UDP edge endpoint
`edge.toml` (tcp/8080 + udp/8081, backend 10.244.0.30 on both ports).
Hydrator emits two `update_service` calls. Both captures show VIP source.

#### 2: Edge — two UDP listeners (different ports)
`multi-udp.toml` (udp/5353 + udp/5354). Hydrator emits two udp
`ServiceFrontend`s; both reverse paths work independently.

#### 3: Boundary — listener added on update
Ana edits `edge.toml` to add a third listener (udp/8082) and re-submits.
The hydrator reconciles to three `update_service` calls; the new udp
path works without disturbing the existing two.

### UAT Scenarios (BDD)
#### Scenario: A two-listener service installs both protocols' paths
Given Ana submits edge.toml with tcp/8080 and udp/8081 and a backend on both
When the ServiceMapHydrator reconciles the service
Then it emits one update_service per listener, each with the declared proto
And both the TCP and UDP forward+reverse paths work end-to-end

#### Scenario: Each listener's reverse path is source-rewritten to the VIP
Given the running two-listener service
When a client exercises both the tcp and udp listeners
Then both replies are captured with the VIP as source

#### Scenario: Adding a listener on re-submit converges without breaking existing paths
Given a running two-listener service
When Ana re-submits edge.toml with a third (udp) listener
Then the hydrator installs the third path and the existing two still work

### Acceptance Criteria
- [ ] `ServiceMapHydrator` emits one `update_service` per listener with the spec-declared proto.
- [ ] Tier 3 e2e: a TCP 8080 + UDP 8081 Service has both forward+reverse paths working through the real CLI→control-plane→reconciler→EbpfDataplane chain.
- [ ] Both protocols' replies are captured with the VIP as source.
- [ ] Re-submitting with an added listener converges without breaking existing paths.

### Outcome KPIs
- **Who:** operators (Ana) running dual-protocol services.
- **Does what:** deploy one service speaking both TCP and UDP on one VIP, both paths working.
- **By how much:** 100% of a multi-listener service's listeners have working forward+reverse paths (was: at most one protocol).
- **Measured by:** two Tier 3 captures (one per protocol), both showing VIP source.
- **Baseline:** multi-protocol on one VIP is unsupported (single `update_service` per service).

### Technical Notes
- `ServiceMapHydrator` (ADR-0042) is the emission site; reads `Vec<Listener>`.
- Depends on US-01 + US-02 + US-04. Composes onto everything above; lowest urgency (single-listener UDP already delivers core value).
- DESIGN open question: does each listener get its own VIP:port, or share a VIP across ports? (Existing SERVICE_MAP outer key is `(VIP, port)` per phase-2 architecture.md §5 Drift-3 — so per-(VIP,port) is natural.)

---

## Wave: DISCUSS / [HOW] Outcome KPIs (consolidated)

### Objective
Make UDP a first-class load-balanced protocol on Overdrive — the reverse
path works both ways and the Sim≡Ebpf lockstep makes the asymmetry
structurally impossible to reintroduce.

### Outcome KPI table

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| K1 (North Star) | UDP-service operators | get a working reverse path (response sourced from VIP) | 100% of UDP services install the `(ip,port,udp)` REVERSE_NAT entry; reverse-path success 0%→100% | 0% | Tier 3 wire capture (source==VIP) + REVERSE_NAT_MAP dump | Leading |
| K2 | dataplane authors | catch any Sim/Ebpf REVERSE_NAT divergence pre-merge | 100% of proto-fan-out divergences caught at PR time | 0% (#163 shipped undetected) | lockstep gate on per-PR critical path + deliberately-broken test | Leading |
| K3 | kernel reverse-NAT program | rewrite proto=17 response sources | 100% of UDP responses in the Tier 2 triptych rewritten to VIP | 0% (no udp entry to match) | Tier 2 `BPF_PROG_TEST_RUN` CHECK assertion | Leading |
| K4 | operators of dual-protocol services | deploy TCP+UDP on one VIP, both paths working | 100% of a multi-listener service's listeners have working paths | ≤50% (one proto at most) | two Tier 3 captures, both VIP-sourced | Leading |
| K5 | dataplane + reconciler code | read `(vip,port,proto)` from one typed surface | 100% of call sites read the `ServiceFrontend`; 0 positional reconstructions (`service_id` on the Action is permitted) | 0% (no proto on shipped trait) | review grep + TCP Tier 3 green | Leading (secondary) |

### Metric hierarchy
- **North Star:** K1 — UDP reverse-path success rate (0%→100%).
- **Leading indicators:** K2 (divergence caught pre-merge), K3 (kernel rewrite fires).
- **Guardrail metrics:** existing TCP forward+reverse paths must NOT regress (K5 green proxy); Tier 2/Tier 3 wall-clock grows ≤10%; no new top-level dependency; verifier instruction budget for `xdp_reverse_nat` stays ≤ baseline +5% (Tier 4).

### Measurement plan
| KPI | Data source | Collection method | Frequency | Owner |
|---|---|---|---|---|
| K1 | Tier 3 capture | AF_PACKET source-addr assertion | per-PR | DELIVER crafter |
| K2 | CI gate | lockstep invariant / acceptance test pass-fail | per-PR | DELIVER + reviewer |
| K3 | Tier 2 | `BPF_PROG_TEST_RUN` CHECK | per-PR | DELIVER crafter |
| K4 | Tier 3 | two captures | per-PR | DELIVER crafter |
| K5 | review + Tier 3 | grep + existing suite | per-PR | reviewer |

### Hypothesis
We believe that threading proto through a `ServiceFrontend` newtype and
fanning out REVERSE_NAT entries by `frontend.proto`, guarded by a
two-pronged lockstep gate (Tier-1 Sim set-equality + Tier-3 Ebpf
acceptance), for UDP-service operators will make the UDP reverse path
work both ways. We will know this is true when a UDP service's response
is captured sourced from the VIP (K1=100%) and the lockstep gate catches
a deliberately-dropped UDP fan-out (K2=100%).

---

## Wave: DISCUSS / [HOW] Definition of Ready validation

Per `nw-leanux-methodology` 9-item DoR (8-item skill list + item 9
Outcome KPIs per `nw-outcome-kpi-framework`).

### Story DoR matrix

| Story | 1. Problem clear | 2. Persona specific | 3. ≥3 examples w/ real data | 4. UAT 3-7 | 5. AC from UAT | 6. Right-sized | 7. Tech notes | 8. Deps tracked | 9. KPIs | Verdict |
|---|---|---|---|---|---|---|---|---|---|---|
| US-01 | PASS — protocol stranded in intent | PASS — Ana, mixed TCP/UDP | PASS — web/dns-resolver/edge .toml | PASS — 3 | PASS | PASS — ~1 day (refactor + proto plumbing; **8 sites** per DESIGN Correction 2 — DISCUSS est. was 5) | PASS — §5 C→ServiceFrontend noted | PASS — none (foundation) | PASS — K5 | **PASS** |
| US-02 | PASS — Step 4b Tcp-only | PASS — Ana, UDP services | PASS — dns-resolver/web/scale-to-0 | PASS — 3 | PASS | PASS — ~1 day | PASS — mirrors `reverse_nat_keys_for` | PASS — US-01 | PASS — K1 | **PASS** |
| US-03 | PASS — invariant Sim-only | PASS — Ana + teammates / CI | PASS — pass/regression/kernel-proof | PASS — 3 | PASS | PASS — ~1.5 days | PASS — retarget site named | PASS — US-02 | PASS — K2/K3 | **PASS** |
| US-04 | PASS — reverse path unverified | PASS — Ana, real submit | PASS — DNS/multi-datagram/backend-down | PASS — 3 | PASS | PASS — ~1 day | PASS — `reverse_nat_e2e` shape | PASS — US-01,02 | PASS — K1 | **PASS** |
| US-05 | PASS — hydrator collapses listeners | PASS — Ana, dual-proto | PASS — edge/multi-udp/add-listener | PASS — 3 | PASS | PASS — ~1.5 days | PASS — ADR-0042 named | PASS — US-01,02,04 | PASS — K4 | **PASS** |

**Aggregate DoR Status: PASSED (5 of 5 stories).** No gaps.

---

## Wave: DISCUSS / [HOW] Self-review (Dimension 0 — Elevator Pitch Test)

Per `nw-po-review-dimensions` Dimension 0 (BLOCKING, checked first).

| Story | Has section | Real entry point | Concrete output | Real decision | Verdict |
|---|---|---|---|---|---|
| US-01 | PASS | PASS — `overdrive deploy` | PASS — existing TCP e2e stays green; `ServiceFrontend` carries proto:Udp | PASS — "rely on one typed surface for protocol" | **PASS** |
| US-02 | PASS | PASS — `overdrive deploy` | PASS — `bpftool map dump` shows udp entry; capture shows VIP source | PASS — "deploy a UDP service and trust the reverse path" | **PASS** |
| US-03 | PASS | PASS — `cargo dst` + `cargo xtask bpf-unit` (the author's gate surface) | PASS — gate fails loudly on dropped fan-out | PASS — "trust the lockstep holds before merge" | **PASS** |
| US-04 | PASS | PASS — `overdrive deploy` + wire capture | PASS — `IP 10.96.0.10.5353 > ...` capture line | PASS — "confidently deploy a real UDP workload" | **PASS** |
| US-05 | PASS | PASS — `overdrive deploy edge.toml` | PASS — accepted line shows both listeners; two captures VIP-sourced | PASS — "run dual-protocol services on one VIP" | **PASS** |

**No `@infrastructure` stories.** US-01 is a refactor but it is NOT
infrastructure-only — its Elevator Pitch references a real operator entry
point (`overdrive deploy`) and an observable outcome (the existing
TCP e2e staying green is operator-visible behavior preservation), and it
enables a real decision. **Slice-composition hard gate: every slice
contains at least one operator-visible value-producing story.**
US-03's entry point is the *author's* CI surface (`cargo dst` /
`cargo xtask bpf-unit`) — legitimate because the "user" of a lockstep
gate is the dataplane author (J-PLAT-004 persona), and the decision
("trust the lockstep before merge") is real.

---

## Wave: DISCUSS / [HOW] Anti-pattern check

| Anti-pattern | Found? | Notes |
|---|---|---|
| "Implement X" framing | NO | Every story starts from operator/author pain (#163 silent asymmetry, stranded protocol, Sim-only invariant) |
| Generic data (user123) | NO | Ana, real TOML (`dns-resolver.toml`, `edge.toml`), real IPs (10.96.0.10, 10.244.0.20), real ports (5353/8080/8081) |
| Technical AC ("Use JWT") | NO | AC are observable outcomes (REVERSE_NAT entry present; capture source==VIP; gate fails on dropped fan-out) |
| Technical scenario titles | NO | Titles describe outcomes ("UDP response is source-rewritten to the VIP", "Dropping the production UDP fan-out fails the gate") |
| Oversized stories | NO | All 5 stories ≤3 scenarios, ≤1.5 days |
| Abstract requirements | NO | 3 concrete examples per story with real data |
| Ship abstraction first (taste test) | HANDLED | US-01 (`ServiceFrontend` newtype) sequenced first AS a behavior-preserving refactor (5 sites) — not a half-built abstraction; Priority Rationale documents this |

---

## Wave: DISCUSS / [HOW] Risks surfaced to DESIGN

| Risk | Probability | Impact | Mitigation owner |
|---|---|---|---|
| RESOLVED (H1): driving the real EbpfDataplane inside a Tier 1 DST invariant is infeasible (needs a kernel + bpffs) | — | — | RESOLVED at DIVERGE — the lockstep is Tier-1 Sim set-equality + Tier-3 Ebpf acceptance + Tier-2 triptych (US-03), NOT a both-adapter in-process retarget. No in-slice SPIKE. |
| US-01 narrows the Sim `reverse_nat_keys_for` `[Tcp,Udp]` hardcode to `frontend.proto` — a TCP service's Sim key set shrinks `{tcp,udp}→{tcp}` (production-true zero-change, Sim-corrected); existing two-proto-Sim assertions must be updated in the same single-cut PR (H2) | M | L | DELIVER — US-01 AC requires the assertion update in-PR; the change is the intended correction of an over-broad Sim fan-out |
| `update_service` C → `ServiceFrontend` trait migration (single-cut) | L | L | **DESIGN-corrected to 8 sites** (the DISCUSS estimate of 5 omitted the proto plumbing): trait, EbpfDataplane, SimDataplane, action-shim, lockstep invariant, **+ `Action::DataplaneUpdateService`, `ServiceDesired`, obs→desired projection**. The hydrator IS changed in US-01 (C3 requires proto end-to-end). See `design/upstream-changes.md` Correction 2 + ADR-0060. |
| Phase-2 architecture.md §5 Q-Sig C → `ServiceFrontend` (superseding the paper locked-A) needs an ADR amendment, not just a code change | H (certain) | L | DESIGN authors the ADR amendment (architect's job); DISCUSS forward-points only |
| Multi-listener VIP:port allocation semantics (US-05) underspecified — per-port VIP vs shared VIP | M | M | DESIGN — existing SERVICE_MAP outer key is `(VIP,port)`, so per-(VIP,port) is the natural shape; flag as P2 question |
| Empty-backend cross-service purge for udp entries could regress the existing tcp purge if not mirrored carefully | L | M | DELIVER — mirror the Sim adapter's `difference`/`live_keys` shape exactly (US-02 AC) |

---

## Wave: DISCUSS / [HOW] Hand-off package for DESIGN wave (solution-architect)

The architect should receive:

1. **This file** (`feature-delta.md`) — entry point with REF/WHY/HOW.
2. **`discuss/journey-udp-service-visual.md` + `.yaml`** — operator journey + embedded Gherkin.
3. **`slices/slice-01..05-*.md`** — per-slice machine briefs.
4. **The locked DISCUSS decision D1** (thread proto via `ServiceFrontend` newtype, C → `ServiceFrontend`; the DIVERGE-validated Option 6) plus `recommendation.md` + `diverge/taste-evaluation.md` — the architect authors the ADR amendment to phase-2 architecture.md §5 Q-Sig (C → `ServiceFrontend`, superseding the paper locked-A).
5. Cross-links: `crates/overdrive-core/src/traits/dataplane.rs` (`update_service`), `crates/overdrive-sim/src/adapters/dataplane.rs:266` (`reverse_nat_keys_for`), `crates/overdrive-sim/src/invariants/reverse_nat_lockstep.rs`, `crates/overdrive-bpf/src/programs/xdp_reverse_nat.rs:251`, `crates/overdrive-core/src/dataplane/backend_key.rs` (`BackendKey`/`Proto`), `docs/feature/phase-2-xdp-service-map/design/architecture.md` §5–§6, `.claude/rules/development.md` § "Trait definitions specify behavior" + § "Production code is not shaped by simulation".

### Anticipated DESIGN open questions (P1 + P2 — main has indicated "all priorities" by default)

| ID | Priority | Question | Why it matters at DESIGN |
|---|---|---|---|
| ~~P1-Q1~~ RESOLVED (DIVERGE/H1) | — | Lockstep mechanism: pure Tier-1 retarget against real Ebpf is infeasible. **Resolved:** Tier-1 Sim set-equality + Tier-3 Ebpf acceptance + Tier-2 triptych (US-03). | No longer an open question; the gate's tier split is locked. |
| P1-Q2 | P1 | `ServiceFrontend` newtype final detail: exact field names/derives, module location, and whether `port` is `NonZeroU16`. (The FAMILY is locked: newtype carries `(ServiceVip, port, Proto)`, `backends` separate, `service_id`/`correlation` on the Action.) | US-01 foundation; affects the 5 call sites |
| P1-Q3 | P1 | ADR amendment vs new ADR for the §5 Q-Sig **C → `ServiceFrontend`** reversal | Traceability; the paper locked-A decision must be visibly superseded |
| P2-Q4 | P2 | Multi-listener VIP allocation: per-(VIP,port) or shared VIP across ports? | US-05; existing SERVICE_MAP outer key is `(VIP,port)` — likely per-(VIP,port) |
| P2-Q5 | P2 | Does the hydrator emit ONE `update_service` per listener (current locked shape — multi-listener is a hydrator-fan-out concern, not a trait-surface one) or fold `Vec<Listener>` into the frontend? | Shapes US-05's hydrator emission; the locked decision says fan-out, not aggregate — re-open only if multi-listener becomes a trait-surface concern (Option-2 dissent condition) |
| P2-Q6 | P2 | Tier 2 triptych: reuse the existing `xdp_reverse_nat` test harness, or new fixture? | US-03 kernel-proof; affects test layout |

**DIVERGE artifacts PRESENT.** A scoped DIVERGE ran for the
`update_service` proto-threading decision: `recommendation.md`,
`diverge/taste-evaluation.md`, `diverge/wave-decisions.md`,
`diverge/options-raw.md`, `diverge/competitive-research.md`,
`diverge/job-analysis.md`, `diverge/review.yaml`. It scored 6 options on
a locked developer-tool taste matrix and selected **Option 6
(`ServiceFrontend` newtype, 4.17)** over the typed aggregate (Option 2,
3.57). D1 is now grounded in that scoring, not an unvalidated PO choice.

---

## Wave: DISCUSS / [HOW] Wave decisions

| ID | Decision | Status | Rationale |
|---|---|---|---|
| **D1** | **Thread the per-service L4 protocol through a `ServiceFrontend` newtype as a TYPED FIELD of the existing `update_service` call — NOT a whole-call aggregate.** New shape: `update_service(frontend: ServiceFrontend, backends: Vec<Backend>)` where `ServiceFrontend` carries `(ServiceVip, port, Proto)`. The frontend RE-ABSORBS `ServiceVip` (the typed home for the validated VIP that shipped-option-C dropped to a raw `Ipv4Addr`); `service_id` and `correlation` STAY on the `Action::DataplaneUpdateService` envelope (`validate.rs:288`) by design (action-routing, not dataplane-key — the SERVICE_MAP key is `(VIP,port)`, the REVERSE_NAT key is `BackendKey{ip,port,proto}`, neither keyed by `service_id`); `backends` stays a separate positional arg. Migration is FROM shipped **option C** (`update_service(vip: Ipv4Addr, backends)`, `dataplane.rs:101`) — locked-A was a paper decision never landed. DISCUSS locks the FAMILY (thread-proto-as-typed-field, frontend re-absorbs ServiceVip, service_id on Action); DESIGN picks the final detail (newtype field names, whether `port` is `NonZeroU16`). Multi-listener fan-out (US-05) is a HYDRATOR concern (one `update_service` per listener), NOT a trait-surface one. | **LOCKED (user-confirmed convergence on the DIVERGE recommendation, Option 6)** | Validated by the scoped DIVERGE taste matrix: **Option 6 (`ServiceFrontend` newtype) = 4.17**, in a statistical tie with Option 1 (positional proto) = 4.13, both decisively ahead of the typed aggregate (Option 2) = 3.57. Option 6 wins on concept-anchoring (the forward twin of the `BackendKey` the engineer already reads on the reverse side — Katran `VipKey` shape) and on making the lockstep set-equality trivial to express, at a blast radius the DISCUSS analysis estimated at 5 sites (trait + both adapters + action-shim + lockstep invariant), hydrator unchanged — **DESIGN corrected this to 8 sites** (the proto plumbing requires changing `Action::DataplaneUpdateService`, `ServiceDesired`, and the obs→desired projection; the hydrator IS touched in US-01 per C3; see `design/upstream-changes.md` Correction 2 + ADR-0060). **Mandatory dissent (Option 2 / full aggregate):** wins ONLY if (a) multi-listener becomes a trait-surface concern rather than hydrator fan-out, OR (b) the team commits to `update_service`-as-typed-SSOT (re-absorbing `service_id` too), OR (c) an explicit documented reweight for industry-alignment. None is established by J-OPS-004/J-PLAT-004. The final newtype shape + the architecture.md §5 Q-Sig amendment (C → `ServiceFrontend`) are forward-pointed to DESIGN; DISCUSS does not edit ADRs. See `recommendation.md` + `diverge/taste-evaluation.md` + `diverge/wave-decisions.md`. |
| D2 | Feature type: **Cross-cutting** (core trait + sim + bpf + dataplane + reconciler). | Locked (Wizard) | 3 bounded contexts; touches the trait surface and both adapters. |
| D3 | Walking skeleton: **single-UDP-listener forward+reverse e2e (US-04)**, brownfield — its prerequisites (US-01 `ServiceFrontend` newtype, US-02 production fan-out) land first. | Locked (Wizard 2) | Thinnest slice touching every backbone activity; the riskiest assumption (reverse path rewrites proto=17). |
| D4 | UX research depth: **Lightweight** (happy + key error paths). | Locked (Wizard 3) | Operator surface is small — #164 already ships the `protocol="udp"` declaration; this feature is dataplane-internal. |
| D5 | JTBD: **Lightweight, trace to existing jobs J-OPS-004 + J-PLAT-004; NO new job.** | Locked (Wizard 4 + this wave) | UDP is an extension of the operator-trust + dataplane-correctness jobs, not a new motivation. Minting a per-protocol job would fragment J-OPS-004. |
| D6 | Scope: **right-sized, no split.** | Locked (this wave) | `## Scope Assessment: PASS — 5 stories, 3 contexts, ~5–6 days.` |
| D7 | SSOT: **NEW `submit-a-udp-service.yaml` journey** (separate from `submit-a-service.yaml`); **no jobs.yaml change.** | Locked (this wave) | Dataplane-wire-path concern is orthogonal to the probe-lifecycle-signal concern in `submit-a-service.yaml`. |
| D8 | Density mode: **lean + ask-intelligent.** Tier-1 [REF] sections only. | Locked (contract) | See telemetry note below. |

### DIVERGE-convergence note (D1 revised)

A scoped DIVERGE wave **ran** for the `update_service` proto-threading
decision (the DIVERGE-absent risk from the original DISCUSS draft is now
closed). It scored 6 options on a locked developer-tool taste matrix and
selected **Option 6 (`ServiceFrontend` newtype) = 4.17** over Option 1
(positional proto) = 4.13 and the typed aggregate Option 2 = 3.57. The
user has now **LOCKED** the DIVERGE recommendation: thread proto as a
typed field of the existing call via `ServiceFrontend` (frontend
re-absorbs `ServiceVip`; `service_id`/`correlation` stay on the Action;
`backends` separate; multi-listener is hydrator fan-out). D1 above is
revised to this decision. The DIVERGE also resolved review finding H1
(lockstep is Tier-1 Sim set-equality + Tier-3 Ebpf acceptance + Tier-2
triptych — the in-process both-adapter retarget is infeasible, settled at
DIVERGE, not deferred into slice 03). Cross-references:
`docs/feature/udp-service-support/recommendation.md`,
`docs/feature/udp-service-support/diverge/wave-decisions.md`,
`docs/feature/udp-service-support/diverge/taste-evaluation.md`.

---

## Wave: DISCUSS / [HOW] Density telemetry note

The contract specifies running density telemetry via
`scripts/shared/telemetry.py:write_density_event(...)`. **That script
does not exist anywhere in this workspace** (verified via
`Glob **/telemetry*.py`, `**/*.py` under repo root, and `~/.claude`).
Per the no-aspirational-references discipline and the rule against
hand-writing JSONL, the telemetry event is NOT hand-fabricated. The
density decision is recorded here instead: **mode=lean,
expansion_prompt=ask-intelligent, Tier-1 [REF] sections emitted, no
expansion menu surfaced (no ask-intelligent trigger fired — see below).**
If the telemetry harness is added later, this wave's event is:
`{wave: DISCUSS, feature: udp-service-support, mode: lean,
expansion_prompt: ask-intelligent, expansion_surfaced: false,
trigger_fired: none}`.

### ask-intelligent trigger evaluation (wave end)
- DoR ambiguity? **No** — 5/5 PASS, no gaps.
- Vendor-neutrality risk? **No** — solution-neutral; kernel mechanics deferred to DESIGN.
- Oversized/split signal? **No** — Scope Assessment PASS.
- Unresolved cross-cutting decision needing user input? **No (now resolved)** — the D1 C → `ServiceFrontend` decision was validated by the scoped DIVERGE (Option 6) and LOCKED by the user. The original DIVERGE-absent risk is closed.

**Strict lean emitted. No expansion menu. Silent-lean skip recorded above.**

---

## Wave: DISCUSS / [REVIEW] Eclipse review (nw-product-owner-reviewer)

**Date:** 2026-06-02 · **Verdict:** NEEDS_REVISION → **resolved via DIVERGE + Option-6 lock; ready for re-review / DESIGN handoff.** (B1, B2, H1, H2, H3, M1 all RESOLVED; M2/M3 unchanged — already satisfied/model decisions.)

**Hard gates:** DoR 9/9 PASS · JTBD traceability PASS · Slice-composition PASS ·
Antipatterns 0 · Elevator Pitch 5/5 PASS · Bug patterns 0. The revision is
factual-accuracy on D1, not structural.

**B1, B2, and H2 independently verified against source by the orchestrator**
(`dataplane.rs:101`, `architecture.md §5:155`, `validate.rs:288`,
`sim/.../dataplane.rs:277`) — the findings are code-grounded, not speculative.

| ID | Sev | Summary | Disposition (this revision) |
|---|---|---|---|
| B1 | blocking | D1 misstated the "from" state as locked-A "three explicit args"; the shipped trait is option **C** (`update_service(vip: Ipv4Addr, backends)`, dataplane.rs:101) and locked-A was never landed. | **RESOLVED.** Corrected to "FROM shipped option C → `ServiceFrontend`; locked-A a paper decision never implemented; the frontend re-absorbs `ServiceVip` but leaves `service_id` on the Action envelope" in feature-delta.md header, D1, US-01 (Solution + Tech Notes), C6, and both journey files (`journey-udp-service-visual.md` shared-artifact spine; `journey-udp-service.yaml` `typed_service_descriptor` note). |
| B2 | blocking | C2/US-01 SSOT claim incomplete — `Action::DataplaneUpdateService` (validate.rs:288) carries `service_id` beside vip+backends; grep-AC could false-green. | **RESOLVED.** C2 + the US-01 AC restated with a defined pass condition: the `ServiceFrontend` is the single source of `(vip,port,proto)`; `service_id`/`correlation` stay on the Action by design (action-routing, not dataplane-key); no call site reconstructs the triple, `service_id` travelling separately is explicitly NOT a violation. |
| H1 | high | US-03 "drive BOTH adapters in Tier 1" infeasible (real Ebpf needs a kernel); AC's "OR Tier 3" deferred a KPI-affecting decision; resolving SPIKE sat inside the gated slice. | **RESOLVED via DIVERGE.** "OR" collapsed to the two-pronged pin: **Tier 1** Sim set-equality AND **Tier 3** Ebpf acceptance (`bpftool` dump + VIP-source capture) AND **Tier 2** triptych. K2 measurement restated (Tier-1 per-PR critical path; Tier-3 integration lane). Slice-03 SPIKE marked resolved — no in-slice SPIKE needed. |
| H2 | high | US-01 "zero behavior change" production-true but Sim-false: `reverse_nat_keys_for` (sim/dataplane.rs:277) hardcodes `[Tcp,Udp]`; narrowing to `frontend.proto` shrinks a TCP service's Sim key set {tcp,udp}→{tcp}. | **RESOLVED.** Reframed as "zero PRODUCTION behavior change; the Sim fan-out is CORRECTED from the over-broad `[Tcp,Udp]` hardcode to the declared `frontend.proto`." Added Domain Example 3 + AC clause ("existing two-proto-Sim assertions updated in the same single-cut PR") + a risk-table row. |
| H3 | high | Minimal-vs-aggregate cost never weighed; larger-blast-radius option presented as the only path. | **RESOLVED via DIVERGE taste matrix.** D1 now cites the scoring (Opt 6 = 4.17 / Opt 1 = 4.13 / Opt 2 = 3.57) and records the mandatory dissent: the full aggregate (Opt 2) wins ONLY if (a) multi-listener becomes a trait-surface concern, OR (b) the team commits to `update_service`-as-typed-SSOT, OR (c) an explicit industry-alignment reweight — none established by J-OPS-004/J-PLAT-004. Final newtype shape + §5 Q-Sig amendment forward-pointed to DESIGN. |
| M1 | med | `submit-a-udp-service.yaml` step 1 re-rendered submit+Stable from submit-a-service.yaml (drift risk). | **RESOLVED.** Step 1 now cross-references `submit-a-service.yaml` for the submit + Stable lifecycle; this journey picks up at the wire path. |
| M2 | med | NO-new-job is the right call; J-OPS-004/J-PLAT-004 trace is honest (model JTBD decision). | **Unchanged** (already satisfied; jobs.yaml NOT minted, rides J-OPS-004 + J-PLAT-004). |
| M3 | med | US-03 author-CI entry point legitimate (J-PLAT-004 persona); guard: never slice US-03 alone. | **Unchanged** (guard satisfied — US-03 bundled with US-02/US-04 in the gate sequence). |

**Re-review trigger satisfied:** B1 + B2 corrected (D1 + C2 + US-01 + both
journeys' "from"-state text + traceability); H1/H2/H3/M1 all dispositioned
via DIVERGE convergence + Option-6 lock. Verdict flips to APPROVED on
re-review; ready for DESIGN handoff.

**Praise (carried from review):** realistic data traced byte-identically across all
artifacts; ACs pin observable kernel side-effects (not branch reachability); emotional
arc fully defined with honest dual happy/sad terminal; DIVERGE-absent risk surfaced
rather than hidden.

---

# DESIGN wave (Morgan, 2026-06-02 — lean / Tier-1)

**Mode:** Propose (decisions locked by user, Phase A enumerated, Phase B
writes). **Density:** lean (Tier-1 [REF] only; no Tier-2 expansion — no
trigger fired). **SSOT:** ADR-0060 + `brief.md` § "UDP service support
extension" + `c4-diagrams.md` § "UDP service support". Back-prop
corrections to DISCUSS recorded in
`docs/feature/udp-service-support/design/upstream-changes.md`.

## Wave: DESIGN / [REF] DDD subdomain classification

| ID | Subdomain | Class | Note |
|---|---|---|---|
| D-DDD-1 | Dataplane reverse-NAT / service frontend | **Core** | The #163 correctness surface; the lockstep equality is the differentiator. |
| D-DDD-2 | Reconciliation (ServiceMapHydrator desired→Action) | Supporting | Existing reconciler (ADR-0042); gains a protocol dimension, not a new boundary. |
| D-DDD-3 | Intent/spec (`Listener`, `Proto`) | Generic (shipped #164) | Untouched by this feature; the proto source. |

No DDD skill load triggered — single bounded context (`overdrive-core`
trait surface + its two adapters), no aggregate redesign, no new
context map. `ServiceFrontend` is a value object (immutable `Copy`
newtype), not an aggregate.

## Wave: DESIGN / [REF] Component decomposition

| Component | Path | Disposition | Responsibility |
|---|---|---|---|
| `ServiceFrontend` | `crates/overdrive-core/src/dataplane/service_frontend.rs` | **CREATE NEW** | Typed `(ServiceVip [V4-by-construction], NonZeroU16 port, Proto)`; fallible `new()` (IPv4 validation), infallible `vip_v4()` narrow. |
| `Dataplane::update_service` | `crates/overdrive-core/src/traits/dataplane.rs:101` | EXTEND | Signature → `(frontend, backends)`; rustdoc contract per ADR-0060. |
| `Action::DataplaneUpdateService` | `crates/overdrive-core/src/reconcilers/mod.rs:440` | EXTEND | + protocol dimension; `service_id`/`correlation` retained. |
| `ServiceDesired` + obs→desired projection | `crates/overdrive-core/src/reconcilers/service_map_hydrator.rs:40,235,263` | EXTEND | + protocol dimension carried from observed `Listener`. |
| `SimDataplane` + `reverse_nat_keys_for` | `crates/overdrive-sim/src/adapters/dataplane.rs:266,289` | EXTEND | Narrow `[Tcp,Udp]`→`frontend.proto`; per-proto purge. |
| `EbpfDataplane::update_service` | `crates/overdrive-dataplane/src/lib.rs` | EXTEND | Step 4b per-`frontend.proto` REVERSE_NAT fan-out (US-02). |
| action-shim dispatch | `crates/overdrive-control-plane/src/action_shim/dataplane_update_service.rs:100,130,160` | EXTEND | Build `ServiceFrontend::new` (IPv6-reject site); call with frontend. |
| `ReverseNatLockstep` | `crates/overdrive-sim/src/invariants/reverse_nat_lockstep.rs` | EXTEND | Per-proto set-equality assertion. |
| `BackendKey` / `Proto` | `crates/overdrive-core/src/dataplane/backend_key.rs:137,66` | REUSE | REVERSE_NAT key + IANA proto enum; reused by `ServiceFrontend`. |
| `Listener` | `crates/overdrive-core/src/aggregate/workload_spec.rs:540` | REUSE (read-only) | The proto source; not modified. |

## Wave: DESIGN / [REF] Driving ports

| Port | Surface | Note |
|---|---|---|
| `ServiceMapHydrator.reconcile` | desired→Action emission | Pure sync (ADR-0035); emits `Action::DataplaneUpdateService` (+ proto). |
| action-shim `dispatch` | Action→`update_service` | Builds `ServiceFrontend`; the driving adapter into the dataplane port. |

No new driving port. No new external/REST surface (the CLI is unchanged —
shipped #164).

## Wave: DESIGN / [REF] Driven ports + adapters

| Driven port | Production adapter | Sim adapter | Probe (Earned Trust) |
|---|---|---|---|
| `Dataplane` (`update_service`) | `EbpfDataplane` (aya-rs, REVERSE_NAT_MAP via bpffs) | `SimDataplane` (in-memory `BTreeMap`) | `EbpfDataplane::probe()` already exists (ADR-0052 wire-then-probe-then-use); this feature adds **no new driven port** and no new probe surface — the existing dataplane probe at composition root covers the REVERSE_NAT_MAP FD. No new external dependency (filesystem/time/subprocess/SDK) is introduced. |

**Earned Trust note:** `ServiceFrontend` is a pure in-process value object;
the adapters it feeds (`EbpfDataplane`) already carry their composition-root
probe per ADR-0052. The IPv4-by-construction invariant is enforced at
`new()` (the action-shim), and the three-tier `ReverseNatLockstep` gate is
the empirical proof that both adapters honor the per-proto contract in their
real environment — Tier 3 exercises the actual kernel REVERSE_NAT_MAP, the
"does the substrate lie about proto=17 rewrite?" probe.

## Wave: DESIGN / [REF] Technology choices

| Choice | Decision | Rationale | License |
|---|---|---|---|
| `ServiceFrontend` derives | `Debug,Clone,Copy,PartialEq,Eq` only | Not wire, not persisted (D2); narrower than `BackendKey`/`Listener` deliberately. | (in-repo) |
| Port type | `NonZeroU16` | Matches `Listener.port`; port=0 unrepresentable (D1b). | std |
| Proto type | reuse `overdrive_core::…::Proto` | IANA tcp/udp enum already shipped (#164); no new type. | (in-repo) |
| Enforcement tooling | dst-lint (existing) + `cargo dst`/`bpf-unit`/`lima` gates | Rust-appropriate; `ReverseNatLockstep` is the DST-equivalence guard. | (in-repo) |

No new third-party crate. No proprietary dependency. No external API
integration (the dataplane boundary is an internal trait) — no
consumer-driven contract test warranted.

## Wave: DESIGN / [REF] Decisions table

| ID | Decision | Disposition |
|---|---|---|
| D1a | `ServiceFrontend { vip: ServiceVip, … }`, V4-guaranteed-by-construction via fallible `new()`; IPv6 rejected at the action-shim (existing operator-visible Failed row); adapters narrow infallibly. | LOCKED |
| D1b | `port: NonZeroU16` (matches `Listener.port`); project to `BackendKey.u16` via `.get()`. | LOCKED |
| D2 | Derives `Debug,Clone,Copy,PartialEq,Eq` only — no serde/utoipa/rkyv/Hash. | LOCKED |
| D3 | New file `crates/overdrive-core/src/dataplane/service_frontend.rs`. | LOCKED |
| D4 | Empty-backends purge is **per-proto** (only `frontend.proto`'s REVERSE_NAT keys; other protos + cross-service shared keys preserved). | LOCKED |
| D5 | ADR-0060 (next free number; supersedes phase-2 §5 Q-Sig locked-A paper). | LOCKED |
| D6 | Proto folds into **US-01** (NOT US-04); true blast radius = 8 sites (Action + ServiceDesired + obs→desired projection included). | LOCKED |
| D7 | No new endianness discipline (`Proto` is a single IANA byte; §11 governs ip/port only). | LOCKED |
| D8 | US-05 forward-key granularity (VIP-only per `validate.rs:218` vs `(VIP,port)` per architecture.md §5 Drift-3) **deferred to US-05 DESIGN** — disagreement flagged, not resolved. | DEFERRED |

## Wave: DESIGN / [REF] Reuse Analysis (HARD GATE)

| Touched component | Existing alternative considered | Decision | Justification |
|---|---|---|---|
| **`ServiceFrontend`** | (a) reuse `Listener { port, protocol }`; (b) reuse `BackendKey { ip, port, proto }`; (c) positional `(ServiceVip, NonZeroU16, Proto)` args | **CREATE NEW** | (a) `Listener` is the intent/spec **wire** type (serde+utoipa+rkyv, `deny_unknown_fields`) carrying no VIP — reusing it on the dataplane boundary would drag wire-schema-evolution coupling onto an ephemeral call argument and still lack the VIP. (b) `BackendKey` is the **backend-side** REVERSE_NAT *key* (`ip` = backend IP); `ServiceFrontend` is the **service-side** frontend (`vip` = service VIP) — semantically inverted; reusing it would conflate the two sides of the NAT and re-introduce a raw `Ipv4Addr` VIP. (c) positional args reintroduce the C2 sprawl the newtype exists to kill. No existing type is the `(service VIP, listener port, proto)` triple; CREATE NEW is justified. |
| `update_service` | keep shipped option-C `(vip, backends)` | EXTEND | The from-state's defect — no proto on the boundary; cannot be reused as-is. |
| `Action` / `ServiceDesired` | keep current shape (no proto) | EXTEND | C3 (no `Tcp` default) is satisfiable only if proto is carried end-to-end. |
| `SimDataplane` / `EbpfDataplane` / `ReverseNatLockstep` | reuse existing logic | EXTEND | Narrow the proto fan-out; purge logic shape unchanged. |
| `Proto` / `BackendKey` / `Listener` | — | REUSE | IANA enum + REVERSE_NAT key + proto source; no change needed. |

**Self-challenge passed:** `ServiceFrontend` is the only CREATE NEW; every
other site EXTENDs or REUSEs. The newtype earns its existence — no existing
type expresses the `(service VIP, listener port, proto)` triple, and the two
near-neighbours (`Listener`, `BackendKey`) are semantically wrong (wire-intent
type / backend-side key respectively).

## Wave: DESIGN / [REF] Open questions

| ID | Question | Owner | Status |
|---|---|---|---|
| OQ-1 | `SERVICE_MAP` forward-key granularity: VIP-only (`validate.rs:218`) vs `(VIP, port)` (architecture.md §5 Drift-3). | US-05 DESIGN | **RESOLVED 2026-06-03 (P2-Q4):** forward key is `(VIP, port, proto)` — see § "Wave: DESIGN / [REF] P2-Q4 resolution" below + ADR-0040 revision 2026-06-03. `validate.rs:218` write-key classifier widens to carry port+proto (DELIVER site). |
| OQ-2 | Action payload shape for the proto dimension: two scalar fields `(port, proto)` vs an embedded per-listener frontend payload. | US-01 DELIVER | OPEN (dimension locked; encoding is an implementation detail). |
| OQ-3 | Unconnected-UDP (`sendto(VIP, ...)` without `connect()`) needs a separate `sendmsg4` (`BPF_CGROUP_UDP4_SENDMSG`) hook — not implemented; connect4 path covers TCP + connected-UDP only. | user / orchestrator | DEFERRED — tracked as [#200](https://github.com/overdrive-sh/overdrive/issues/200). See ADR-0053 amendment § "Out of scope". |

---

## Wave: DESIGN / [REF] P2-Q4 resolution — proto in the service-LB map keys

**Date:** 2026-06-03 · **Architect:** Morgan · **Mode:** Propose
(core decision **user-locked**; only struct-layout / proto-source /
test-surface sub-choices are recommendations). **Resolves:** P2-Q4
(the open question slice-05 owned) and subsumes OQ-1/D8.

**Locked decision.** L4 protocol enters **both** eBPF service-LB map
keys, IPVS-style:
- `SERVICE_MAP` outer key `(ServiceVip, port)` → `(ServiceVip, port,
  Proto)` (wire-boundary XDP forward path; ADR-0040 revision
  2026-06-03).
- `LOCAL_BACKEND_MAP` key `(VIP, vip_port)` → `(VIP, vip_port, proto)`
  (same-host cgroup `connect4` path; ADR-0053 revision 2026-06-03).

**User rationale (verbatim):** *"we don't want to fix incorrect
architecture — do `(vip, port, proto)` as IPVS."*

**Relationship to the existing US-01 DESIGN (ADR-0060).** ADR-0060 put
proto on the **dataplane boundary** (`ServiceFrontend { vip, port,
proto }`) and into the **REVERSE_NAT** response-path key (`BackendKey
{ ip, port, proto }`). It explicitly **deferred** the SERVICE_MAP
*forward*-key granularity to US-05 (D8 / OQ-1). P2-Q4 closes that
deferral: the same proto dimension now also keys the forward maps. One
proto dimension, three maps (SERVICE_MAP forward, REVERSE_NAT response,
LOCAL_BACKEND_MAP same-host), end-to-end.

### [REF] Decisions table delta

| ID | Decision | Disposition |
|---|---|---|
| P2-Q4 | `SERVICE_MAP` outer key + `LOCAL_BACKEND_MAP` key both gain `proto` (IPVS `{protocol, addr, port}` shape). Proto byte absorbs one reserved `_pad` byte; 8-byte structs unchanged in width; trailing pad stays zeroed for deterministic BPF hashing. | LOCKED (user) |
| P2-Q4-a | cgroup_connect4 proto source = `bpf_sock_addr.protocol` (IANA byte, zero-translation; `bpf_sock_addr.type` SOCK_*→IPPROTO_* as documented fallback). Verified present in in-tree UAPI. | LOCKED (recommendation) |
| P2-Q4-b | `Action::DataplaneUpdateService` + `RegisterLocalBackend`/`DeregisterLocalBackend` carry `Proto`, sourced from a listener-bearing fact (ADR-0060 site #8), NEVER a `Tcp` default (C3). | LOCKED |
| P2-Q4-c | Migration = single-cut, reconciler-repopulated (key structs change, maps recreated on boot). No live migration, no dual-key shim, no deprecation. | LOCKED |
| P2-Q4-d | Unconnected-UDP (`sendmsg4`) is a separate undelivered hook — OUT of scope; tracked as OQ-3 deferral / [#200](https://github.com/overdrive-sh/overdrive/issues/200). | DEFERRED |

### [REF] Component decomposition delta (all EXTEND — zero CREATE NEW)

| Component | Path | Disposition | Change |
|---|---|---|---|
| `ServiceKey` (kernel) | `crates/overdrive-bpf/src/maps/service_map.rs:74-78` | EXTEND | `_pad: u16` → `proto: u8` + `_pad: u8`; 8 bytes unchanged. |
| `ServiceKey` (userspace mirror) | `crates/overdrive-dataplane/src/maps/service_map_handle.rs:59-66` | EXTEND | Mirror the proto byte; `from_vip_port` (`:73-84`) gains a `proto` param. |
| `xdp_service_map_lookup` | `crates/overdrive-bpf/src/programs/xdp_service_map.rs:247,268` | EXTEND | `proto` already read from the IPv4 header (`:247`); slot it into the key (`:268`). No new packet parse. |
| `LocalServiceKey` (kernel) | `crates/overdrive-bpf/src/maps/local_backend_map.rs:27-34` | EXTEND | `_pad: u16` → `proto: u8` + `_pad: u8`; 8 bytes unchanged. |
| `cgroup_connect4_service` | `crates/overdrive-bpf/src/programs/cgroup_connect4_service.rs:56-76` | EXTEND | Read `bpf_sock_addr.protocol`; key `LOCAL_BACKEND_MAP` on `(vip, port, proto)`. |
| `LocalBackendMapHandle` | `crates/overdrive-dataplane/src/maps/local_backend_map_handle.rs` | EXTEND | `upsert`/`remove` gain `proto: Proto`. |
| `Action::DataplaneUpdateService` | `crates/overdrive-core/src/reconcilers/mod.rs:440` | EXTEND | + proto (already covered by ADR-0060 site #7; the forward-key amendment confirms the consumer). |
| `Action::RegisterLocalBackend` / `DeregisterLocalBackend` | `crates/overdrive-core/src/reconcilers/mod.rs:485-509` | EXTEND | + `proto: Proto` field. |
| `ServiceMapHydrator` classifier | `crates/overdrive-core/src/reconcilers/service_map_hydrator.rs` | EXTEND | Thread per-listener proto into both forward (`DataplaneUpdateService`) and same-host (`RegisterLocalBackend`) actions. |
| `validate.rs` write-key classifier | `crates/overdrive-control-plane/src/action_shim/validate.rs:218` | EXTEND | Forward write-key widens from VIP-only (`port_opt: None`) to carry port + proto (closes OQ-1). |
| `Proto` | `crates/overdrive-core/src/dataplane/backend_key.rs:66` | REUSE | IANA enum (`as_u8()` → 6/17); reused, not modified. |

### [REF] Reuse Analysis (HARD GATE) — P2-Q4

| Touched component | Existing alternative considered | Decision | Justification |
|---|---|---|---|
| SERVICE_MAP / LOCAL_BACKEND_MAP key structs | (a) new proto-keyed map alongside the proto-less one; (b) widen the existing key in place | **EXTEND (widen in place)** | (a) A parallel map duplicates the lookup surface and forces the XDP/cgroup programs to consult two maps — strictly worse, and the single-cut posture makes the parallel-map "compat" benefit moot. (b) Widening the existing 8-byte POD by consuming a reserved pad byte is free (no byte-width change) and matches Cilium's own `lb4_key` tail. EXTEND. |
| `cgroup_connect4_service` proto read | new program / new hook | **EXTEND** | The existing connect4 program already reads `bpf_sock_addr`; adding a `.protocol` read is one field, not a new program. |
| `Proto` type | new `L4Proto` enum | **REUSE** | ADR-0060's `backend_key::Proto` already models tcp/udp with IANA `as_u8()`; a second enum would fork the vocabulary. REUSE. |
| `Action` variants | new proto-carrying action kinds | **EXTEND** | Add a `proto` field to the existing variants; a parallel action kind would duplicate dispatch. |

**Zero CREATE NEW.** Every P2-Q4 touched component EXTENDs an existing
map/struct/program/handle/action or REUSEs `Proto`. This is the
expected disposition: the feature widens existing keys to carry a
dimension that already exists at the boundary (ADR-0060), not a new
capability needing new structure.

### [REF] C4 — no diagram

P2-Q4 changes **map key tuples**, not the component topology: the same
SERVICE_MAP / LOCAL_BACKEND_MAP, the same XDP / cgroup programs, the
same hydrator → action-shim → adapter flow. No container or component
boundary moves. The existing US-01 C4 (L1/L2/L3 in
`design/c4-diagrams.md`) remains accurate. A new C4 diagram for "the
key got one byte wider" would be redundant — **explicitly not drawn**
per the deliverable instruction.

### [REF] Contradiction check vs ADR-0040 / 0053 / 0060

- **ADR-0040 Decision 1** (`SERVICE_MAP (VIP, port)`): the 2026-06-03
  amendment widens the key; no contradiction (amendment supersedes the
  tuple, preserves the map split + atomic-swap). ✓
- **ADR-0053 Decision 1** (`LOCAL_BACKEND_MAP (VIP, vip_port)`): the
  2026-06-03 amendment widens the key; preserves the cgroup mechanism.
  Decision 1's "SENDMSG/RECVMSG out of scope" is preserved & sharpened
  (connected-UDP in, unconnected-UDP out). ✓
- **ADR-0060** (`ServiceFrontend { vip, port, proto }`; REVERSE_NAT
  proto; D8 deferral): **no contradiction — complementary.** ADR-0060
  deferred the forward-key granularity (D8/OQ-1) explicitly; P2-Q4
  resolves it consistently (same `Proto`, same listener-bearing
  source, same C3 no-`Tcp`-default rule). ADR-0060 needs **no edit**. ✓
- **C3** (proto never defaulted to `Tcp`): honored — proto sourced from
  a listener-bearing fact on every path. ✓
- **C6** (single-cut migration): honored — key structs + maps recreated
  on boot, no shim. ✓

No contradiction found across ADR-0040 / 0053 / 0060.

No outcome-registry collision check (no `docs/product/outcomes/registry.yaml`
— Outcome Collision Check skipped, confirmed).

---

## Wave: DESIGN / [REVIEW] Atlas review (nw-solution-architect-reviewer)

**Date:** 2026-06-02 · **Verdict:** APPROVED (0 critical, 0 high, 1 medium, 1 low) · **Iteration:** 1

**Fidelity to locked decisions:** D1a (literal `ServiceVip` + V4-by-construction) FAITHFUL — `ServiceFrontend::new` replaces `ipv4_from_vip` at `dataplane_update_service.rs:110`, operator-visible Failed row preserved, `vip_v4()` narrow backed by sole-constructor invariant using `unreachable!()` (not `.expect()`). D4/D5/D6/D7/D8 all FAITHFUL. No drift from DIVERGE/DISCUSS; corrections back-propagated openly via `upstream-changes.md`. Trait contract complete (preconditions/postconditions/every edge case/cross-adapter invariant). Reuse Analysis a genuine self-challenge. C4 L1/L2/L3 match the 8-site decomposition.

| ID | Sev | Finding | Disposition |
|---|---|---|---|
| ATLAS-1 | medium | ADR-0060 site #8 (+ brief.md row 8, C4 L3 node) describe proto/port as carried from the `service_backends` observation — verified FALSE: `ServiceBackendRowV1` (`observation_store.rs:875`) and `ServiceDesired` (`service_map_hydrator.rs:40`) carry no port/proto; `hydrate_desired` (`reconciler_runtime.rs:1322-1348`) reads only `service_backends_rows`. Proto/port actually live on `ListenerRow` (`observation_store.rs:321`) + the `BackendDiscoveryBridge` per-listener projection (`reconciler_runtime.rs:2569`). Risk: a crafter implementing against the literal text could synthesize a `Tcp` default (C3 violation). | **Non-blocking for handoff** (dimension locked end-to-end; data exists). Correct the provenance text in ADR-0060/brief/C4; DISTILL pins proto provenance to a listener-bearing source + adds a C3 guard scenario ("unresolvable-listener desired projection is an error, never a silent `Tcp` default"). |
| ATLAS-2 | low | Existing `ServiceBackendRow` write path collapses listeners to the first (`reconciler_runtime.rs:2015-2019`: first-listener-only, port default 0, no proto). If a DELIVER crafter sources proto from `ServiceBackendRow` it becomes a hidden 9th site; correctly out of US-01 scope (US-05 owns multi-listener fan-out). | Flag to DISTILL/DELIVER so the first-listener collapse is not rediscovered as a surprise; confirm US-01 sources proto from the listener fact, not the proto-less `ServiceBackendRow`. |

**Gate:** CLEARED for DISTILL/DEVOPS handoff. The medium finding is a provenance-precision correction (artifact text + a DISTILL acceptance scenario), not a DESIGN re-spin.

---

# DISTILL wave (Sentinel, 2026-06-02 — lean / Tier-1)

**Mode:** acceptance-test authoring. **Density:** lean (Tier-1 [REF] only;
no Tier-2 expansion — no trigger fired). **Scenario SSOT:**
`distill/test-scenarios.md` (GIVEN/WHEN/THEN spec blocks — specification-only
per `.claude/rules/testing.md` § "No `.feature` files anywhere"; the executable
artifacts are the Rust RED scaffolds below). **Wave-Decision Reconciliation:**
PASSED — 0 contradictions across DISCUSS (D1–D8) / DESIGN (D1a–D8) / DEVOPS
(no `devops/` wave-decisions present — default environment matrix applied,
warning logged; no contradiction).

## Wave: DISTILL / [REF] Scenario list

23 scenarios. Error/edge ratio 10/23 = **43%** (≥40% met). Walking skeleton: 1
(S-04-A). `@property` (PBT-full, Tier 1 only): 5.

| Scenario | Tier | Tags | US |
|---|---|---|---|
| S-01-A IPv4 VIP constructs + accessors round-trip | 1 | `@property @in-memory @K5` | US-01 |
| S-01-B IPv6 VIP rejected | 1 | `@property @error @in-memory @D1a` | US-01 |
| S-01-C udp listener proto reaches dataplane as Udp | 1 | `@in-memory` | US-01 |
| S-01-D proto sourced from listener fact (not service_backends) | 1 | `@in-memory @C3` | US-01 |
| S-01-E unresolvable listener proto → structured Failed (NEGATIVE) | 1 | `@error @in-memory @C3` | US-01 |
| S-01-F IPv6 rejected at action-shim as operator-visible Failed | 1 | `@error @in-memory @D1a` | US-01 |
| S-02-A empty backends purge only this proto's keys | 1 | `@property @in-memory @D4` | US-02 |
| S-02-B cross-service shared key survives per-proto purge | 1 | `@in-memory @D4` | US-02 |
| S-02-C idempotent re-apply | 1 | `@property @in-memory` | US-02 |
| S-02-D IPv6 backend contributes no key | 1 | `@error @in-memory` | US-02 |
| S-02-E sctp rejected at parse boundary (confirm #164) | 1 | `@error @in-memory` | US-02 |
| S-03-A Sim installs exactly declared-proto key set | 1 | `@property @in-memory @K2` | US-03 |
| S-03-B dropped Sim fan-out key fails lockstep (NEGATIVE) | 1 | `@error @in-memory @K2` | US-03 |
| S-03-C phantom extra key fails lockstep orphan check (NEGATIVE) | 1 | `@error @in-memory @K2` | US-03 |
| S-03-D #163 shape (tcp-only for udp service) caught (NEGATIVE) | 1 | `@error @in-memory @K2` | US-03 |
| S-03-E xdp_reverse_nat rewrites proto=17 source to VIP | 2 | `@real-io @adapter-integration @K3` | US-03 |
| S-03-F REVERSE_NAT miss → XDP_PASS unmodified | 2 | `@error @real-io` | US-03 |
| S-04-A single-UDP-listener round-trip, VIP source (WS) | 3 | `@walking_skeleton @driving_adapter @real-io @adapter-integration @K1` | US-04 |
| S-04-B every UDP reply independently rewritten | 3 | `@real-io` | US-04 |
| S-04-C missing-backend response distinguished | 3 | `@error @real-io` | US-04 |
| S-05-A two-listener installs both protocol paths | 3 | `@real-io @adapter-integration @K4` | US-05 |
| S-05-B each listener reverse path VIP-sourced | 3 | `@real-io` | US-05 |
| S-05-C added listener on re-submit converges | 3 | `@real-io` | US-05 |

## Wave: DISTILL / [REF] Four-tier mapping (replaces WS-strategy A/B/C/D)

The generic skill's Walking-Skeleton-Strategy A/B/C/D is **retired** here in
favour of the project's four-tier model (`.claude/rules/testing.md`). Tier IS
the test taxonomy.

| Tier | Lane / runner | Scenarios | Input mode (Mandate 9) | Assertion (Mandate 8) |
|---|---|---|---|---|
| Tier 1 (DST / in-memory) | default lane, `cargo dst` / `cargo nextest run` | 15 | PBT-full allowed (layer 1–2) for `@property` | `BTreeSet<BackendKey>` set-equality (native universe guard — see infra policy § Mandate 8 mapping) |
| Tier 2 (BPF unit) | `cargo xtask bpf-unit`, `BPF_PROG_TEST_RUN` | 2 | example-only (Mandate 11) | kernel-side observable: verdict + `data_out` rewrite |
| Tier 3 (real veth) | `cargo xtask lima run --`, `integration-tests` | 6 | example-only (Mandate 11) | observable kernel side-effects: `bpftool map dump` + wire capture source |

## Wave: DISTILL / [REF] Adapter coverage (Mandate 6 — every driven adapter ≥1 real-IO)

| Driven adapter / port | Real-IO scenario | Tier | Covered by |
|---|---|---|---|
| `Dataplane` via `EbpfDataplane` (REVERSE_NAT_MAP) | YES | 3 | S-04-A (`bpftool map dump` + wire capture), S-05-A |
| `xdp_reverse_nat_lookup` (kernel program) | YES | 2 | S-03-E (`BPF_PROG_TEST_RUN` proto=17 rewrite) |
| `Dataplane` via `SimDataplane` (in-process driven-internal) | YES (in-memory, DST-real) | 1 | S-03-A set-equality + S-02-A/B/C purge |
| Driving adapter `overdrive deploy` (CLI subprocess) | YES | 3 | S-04-A (exit 0 + `Accepted.` + UDP reverse path) |

Zero "NO — MISSING" rows. No new external/non-deterministic driven port is
introduced (no clock/email/payment/LLM/third-party) — see infra policy
§ "Driven external".

## Wave: DISTILL / [REF] Scaffolds (RED-ready, project convention)

All test-side scaffolds use `#[should_panic(expected = "RED scaffold")]` (GREEN
at the bar — no `--no-verify` needed for sibling commits, per
`.claude/rules/testing.md`). The one production stub uses `todo!("RED scaffold:
…")` gated with `#[expect(clippy::todo, …)]`.

| File | Kind | Scenarios |
|---|---|---|
| `crates/overdrive-core/src/dataplane/service_frontend.rs` | **production stub** (`todo!`) — `ServiceFrontend` type only; the 8 production sites are DELIVER's job | (enables S-01-A/B) |
| `crates/overdrive-core/tests/service_frontend.rs` | test (`#[should_panic]`) | S-01-A, S-01-B |
| `crates/overdrive-core/tests/service_frontend_provenance.rs` | test (`#[should_panic]`) | S-01-C, S-01-D, S-01-E (C3 guard) |
| `crates/overdrive-control-plane/tests/acceptance/service_frontend_ipv6_rejected.rs` | test (`#[should_panic]`) | S-01-F (D1a) |
| `crates/overdrive-sim/tests/sim_dataplane_reverse_nat_per_proto.rs` | test (`#[should_panic]`) | S-03-A/B/C/D, S-02-A/B/C/D |
| `crates/overdrive-bpf/tests/integration/xdp_reverse_nat_udp.rs` | test (`#[should_panic]`) | S-03-E, S-03-F |
| `crates/overdrive-dataplane/tests/integration/reverse_nat_udp_e2e.rs` | test (`#[should_panic]`) | S-04-A (WS), S-04-B, S-04-C |
| `crates/overdrive-dataplane/tests/integration/multi_listener_tcp_udp_e2e.rs` | test (`#[should_panic]`) | S-05-A, S-05-B, S-05-C |

S-02-E (sctp boundary, confirms #164's `Proto` admission) has no new scaffold —
it is a confirmation against the shipped `Proto::try_from`/`FromStr`
(`backend_key.rs:100`); DELIVER adds it to the existing `backend_key.rs` test
or `sim_dataplane_reverse_nat_per_proto.rs` as a thin assertion.

Each test file is wired into its crate's entrypoint (`tests/integration.rs` /
`tests/acceptance.rs`) — no orphan modules. The production `ServiceFrontend`
stub is re-exported from `overdrive_core::dataplane`.

## Wave: DISTILL / [REF] Test placement (with precedent)

| Crate / dir | Scenarios | Precedent |
|---|---|---|
| `crates/overdrive-core/tests/*.rs` (standalone entrypoints) | S-01-A..E | `tests/service_vip.rs`, `tests/backend_key.rs` (newtype/proptest entrypoints) |
| `crates/overdrive-control-plane/tests/acceptance/` | S-01-F | `tests/acceptance/service_vip_submit_acceptance.rs`, `release_service_vip_dispatch.rs` |
| `crates/overdrive-sim/tests/*.rs` + `src/invariants/reverse_nat_lockstep.rs` | S-02-A..D, S-03-A..D | `tests/sim_dataplane_reverse_nat_cross_service.rs`; the invariant to retarget |
| `crates/overdrive-bpf/tests/integration/` | S-03-E/F | `tests/integration/xdp_reverse_nat_redirect_neigh.rs` (TCP triptych) |
| `crates/overdrive-dataplane/tests/integration/` | S-04-A..C, S-05-A..C | `tests/integration/reverse_nat_e2e.rs`, `service_map_forward.rs` (Tier 3, `ThreeIfaceTopology`) |

## Wave: DISTILL / [REF] Driving adapter coverage

| Driving surface (DESIGN) | Protocol-level exercise | Scenario |
|---|---|---|
| `overdrive deploy <spec.toml>` (CLI verb `Deploy`) | real **subprocess** of the built binary; assert exit code 0 + `Accepted.` stdout + UDP reverse wire path | S-04-A (WS) ; S-05-A (multi-listener) |
| `ServiceMapHydrator.reconcile` (in-process driving port) | direct call; assert emitted `Action`/`ServiceFrontend` proto | S-01-C/D/E |
| action-shim `dispatch` (Action → `update_service`) | direct call; assert operator-visible `Failed` row on IPv6 | S-01-F |

> **BLOCKER (resolved by instruction, surfaced for the record).** The product
> journey `docs/product/journeys/submit-a-udp-service.yaml` step 1 names
> `overdrive job submit dns-resolver.toml`. The **shipped CLI verb is
> `overdrive deploy`** (`crates/overdrive-cli/src/cli.rs:42` `Command::Deploy`,
> `main.rs:63`). DISTILL uses `overdrive deploy` (per the instruction and the
> verified CLI source) for the walking-skeleton driving adapter. The journey
> YAML's `job submit` text is a pre-existing artifact drift — **not** a
> wave-decision contradiction (the journey is product-level SSOT context, not
> a DISCUSS/DESIGN/DEVOPS wave-decision), so it does not trip the Reconciliation
> HARD GATE. Flagged for a follow-up journey-text fix (architect's territory;
> no GH issue created — surfaced for user/orchestrator to decide).

## Wave: DISTILL / [REF] Pre-requisites

- **DESIGN driving ports:** `ServiceMapHydrator.reconcile` + action-shim
  `dispatch` (DESIGN [REF] Driving ports). No new external/REST surface.
- **DESIGN driven ports/adapters:** `Dataplane` (`EbpfDataplane` prod /
  `SimDataplane` sim); existing `EbpfDataplane::probe()` covers REVERSE_NAT_MAP
  FD (no new probe). The `overdrive-testing` `ThreeIfaceTopology` netns/veth
  fixtures (real-infra, dev-dep) back the Tier 3 lane.
- **DEVOPS environment matrix:** no `docs/feature/udp-service-support/devops/`
  present — **default matrix applied** (warning logged). Tier 2/3 require Lima
  (`cargo xtask lima run --`) + the BPF object (`cargo xtask bpf-build` →
  `target/bpf/overdrive_bpf.o`) as a Tier-3/mutation prerequisite.
- **Project Infrastructure Policy:** bootstrapped this run at
  `docs/architecture/atdd-infrastructure-policy.md` (was absent) — Rust-native,
  no Python state-delta port (Mandate 8 satisfied via native `BTreeSet`
  set-equality; mapping documented in the policy file).
- **No `kpi-contracts.yaml` entry** maps to a new emittable metric event;
  `@kpi` observability scenarios are not warranted (soft gate — noted). KPIs
  K1–K5 are measured by the tier assertions (see test-scenarios.md § KPI links).
- **No `docs/product/outcomes/registry.yaml`** — Outcome registration SKIPPED
  (confirmed absent; this is a code-feature but the registry is not adopted in
  this project).

## Wave: DISTILL / [REF] DoD self-check

- [x] All scenarios authored + RED scaffolds created & wired (zero orphan modules).
- [x] Tier mapping complete (15 Tier 1 / 2 Tier 2 / 6 Tier 3); pyramid honoured.
- [x] Wave-Decision Reconciliation HARD GATE passed (0 contradictions).
- [x] Mandate 8 — Tier 1 set-mutating scenarios use `BTreeSet<BackendKey>`
      set-equality (native universe guard, fail-closed via orphan check);
      Tier 2/3 (layer 3+) traditional kernel-observable assertions.
- [x] Mandate 9 — PBT-full only on Tier 1 `@property` scenarios; Tier 2/3
      example-only.
- [x] Mandate 10 — Tier B state-machine PBT **NOT** warranted: the journey is
      a dataplane wire-path (the observable is set-equality + wire capture,
      not a ≥3-chained-scenario rich-input journey with a state-machine model);
      Tier A (example-only, production composition root) covers the space.
- [x] Mandate 11 — Tier 2/3 sad paths are named example-based tests (S-03-F,
      S-04-C); no PBT machinery at layer 3+.
- [x] Pillar 1 — scenario titles/steps use domain language (no HTTP/JSON/schema
      jargon; "reply sourced from the VIP", "the gate fails naming the missing
      udp key").
- [x] Pillar 2 — chained narrative within the US-04→US-05 journey (US-05 reuses
      the US-04 deploy+reverse-path setup).
- [x] Pillar 3 — Tier 3 WS uses the production composition root (real
      `overdrive deploy` subprocess → real control-plane → real EbpfDataplane);
      only the host netns/veth fixtures stand in for the cluster topology.
- [x] C3 guard (S-01-C/D/E) + D1a IPv6 (S-01-B/F) + per-proto purge (S-02-A/B) +
      #163 regression (S-03-D Tier 1 + S-04-A Tier 3) scenarios present.
- [x] Both `upstream-changes.md` corrections applied to the DISCUSS [REF]
      stories with `> Changed Assumptions (DISTILL, 2026-06-02):` annotations.

## Wave: DISTILL / [REF] Inherited commitments

| Origin | Commitment | DDD | Impact |
|--------|------------|-----|--------|
| DESIGN#ATLAS-1 | DISTILL pins proto provenance to a listener-bearing fact + adds a C3 guard scenario (unresolvable-listener → structured error, never silent `Tcp`) | n/a | S-01-C/D/E make a `Tcp`-default a failing test — the load-bearing C3 defense against the #163 class recurring at the projection layer |
| DESIGN#ATLAS-2 | Confirm US-01 sources proto from the listener fact, not the proto-less `ServiceBackendRow` (first-listener collapse) | n/a | S-01-D asserts the listener-fact source explicitly; the `ServiceBackendRow` write-path generalization stays US-05 scope |
| DESIGN#D4 | Per-proto purge | n/a | S-02-A/B pin per-proto purge + cross-service-shared survival; upstream-changes Correction 1 applied |
| DESIGN#D6 | 8-site blast radius; proto plumbed end-to-end in US-01 | n/a | upstream-changes Correction 2 applied to US-01 Technical Notes |
| DESIGN#D1a | V4-by-construction, IPv6 rejected at operator-visible action-shim site | n/a | S-01-B (newtype reject) + S-01-F (operator-visible Failed row) |
| ADR-0060 § Enforcement | Three-tier `ReverseNatLockstep` gate (T1 Sim set-equality + T2 triptych + T3 Ebpf acceptance) | n/a | S-03-A..D (T1) + S-03-E/F (T2) + S-04-A (T3) realise the gate |

---

## Wave: DISTILL / [REVIEW] Sentinel review (nw-acceptance-designer-reviewer)

**Date:** 2026-06-02 · **Verdict:** APPROVED (0 blocker, 0 high, 3 medium, 2 low) · **Iteration:** 1 of 2 (no second required) · **Hand to DELIVER:** yes.

**Verified REAL (not nominal):** C3 guard S-01-E asserts BOTH a structured `Failed` AND the absence of a `Tcp`-defaulted action; the #163 lockstep is pinned in BOTH directions (S-03-B missing-key + S-03-C orphan/phantom-key); per-proto purge S-02-A asserts the co-resident other-proto frontend SURVIVES. All scaffolds use the project `#[should_panic(expected = "RED scaffold")]` / `todo!` convention; zero `.feature` files; all 8 scaffold files wired into entrypoints; observable-behavior assertions throughout (`BTreeSet<BackendKey>` set-equality via port accessors, `bpftool` dump, wire-capture) — no internal-state coupling. Cross-wave fidelity to ADR-0060 high (8-site radius, per-proto purge, listener-fact provenance, no string-roundtrip proptest, rkyv mandate correctly N/A).

| ID | Sev | Finding | Disposition |
|---|---|---|---|
| M-1 | medium | 3 DISCUSS-internal sites (DoR matrix, Risk table, D1 wave-decision) still printed stale "5 sites / hydrator unchanged". | **RESOLVED** (this wave) — all three annotated to the locked **8 sites** with pointers to upstream-changes Correction 2 + ADR-0060. |
| M-2 | medium | S-04-A walking-skeleton driving-adapter half (subprocess `overdrive deploy` exit-0 + `Accepted.`) has no scaffold; placement left as "or" between overdrive-control-plane / overdrive-cli. Wire half IS scaffolded. | **DELIVER precondition** — create the subprocess-deploy scaffold in a definite crate (suggest `overdrive-cli/tests/integration/` per the `exec_spec_walking_skeleton` precedent) before closing S-04-A. |
| M-3 | medium | S-04-A asserts byte-exact stdout `"Accepted."` — couples a Tier-3 WS to the exact render string (cosmetic-drift red risk). | DELIVER judgment — prefer exit-0 + a structural "accepted" predicate over the literal. Tier-3 traditional assertions are permitted (not a Mandate-8 violation). |
| L-1 | low | S-02-E cites `backend_key.rs:100-110` (`TryFrom<u8>`, IANA byte) for rejecting the string token `"sctp"`; that token is rejected at the `Listener` proto `FromStr` boundary (#164), a different site. No-scaffold confirmation. | DELIVER points the assertion at the actual `FromStr` string-parse site shipped by #164. |
| L-2 | low | `ServiceFrontend::vip()` accessor scaffolded but no scenario drives it (S-01-A round-trips `vip_v4()`/`port()`/`proto()` only). | DELIVER: S-01-A GREEN adds a `vip()` assertion, or drop `vip()` if no consumer materialises (deletion discipline). |

**DELIVER preconditions carried:** (1) `cargo xtask bpf-build` must run first (`target/bpf/overdrive_bpf.o`) before Tier-2/3 scaffolds compile — environment caveat, not a scaffold defect; (2) M-2 — name the subprocess-deploy WS scaffold file before closing S-04-A.

**Gate:** CLEARED for DELIVER. Scores all ≥7; all mandates pass.
