<!-- markdownlint-disable MD024 -->
# Feature Delta — unconnected-udp-sendmsg4

**Feature-id:** `unconnected-udp-sendmsg4` · **GH:** [#200](https://github.com/overdrive-sh/overdrive/issues/200)
· **Waves present:** DIVERGE (accepted) → **DISCUSS** (this wave)
· **Density:** lean + ask-intelligent (resolved from `~/.nwave/global-config.json`)

> Single narrative file per the nw-discuss Outputs contract. DISCUSS
> findings live as `## Wave: DISCUSS / [REF|WHY|HOW] <Section>` headings.
> Slice briefs are the only separate machine artifacts
> (`slices/slice-NN-*.md`). DIVERGE artifacts remain at the feature root
> (`recommendation.md`, `wave-decisions.md`) and under `diverge/`.

---

## Wave: DISCUSS / [REF] Persona

**Ana Moreno** — Overdrive platform engineer running UDP-bearing services
on a single-node dev host. She does NOT control the clients that talk to
her services: they are third-party DNS resolvers (`dig`, glibc
`getaddrinfo`, musl) that call `sendto(VIP)` per query and never
`connect()`. SSOT: `docs/product/personas/ana-platform-engineer.yaml`
(created this wave — Ana now anchors two UDP journeys). Secondary actor:
the dataplane author (rides J-PLAT-004) who needs Sim and the real kernel
path provably equivalent on the reply-path source identity.

---

## Wave: DISCUSS / [REF] JTBD one-liner

When a same-host client sends a datagram to a service VIP **without first
connecting** (the dominant DNS-resolver idiom), Ana — the operator who
declared that UDP service — wants the platform to deliver the datagram to
a healthy backend and make the reply appear to originate from the VIP, so
the service is reachable from real clients she does not control.

**Job is ALREADY VALIDATED in DIVERGE — JTBD was NOT re-run this wave.**
Source: `diverge/job-analysis.md` (physical-level job + O1–O5 ODI
outcomes). Rides **J-OPS-004** (operator-trust reachability) +
**J-PLAT-004** (Sim≡kernel equivalence) in `docs/product/jobs.yaml`. No
new job (per the udp-service-support D5 precedent; the 2026-06-05 jobs.yaml
changelog entry was added in DIVERGE).

---

## Wave: DISCUSS / [REF] Journey decision — SIBLING journey (extend-with-cross-ref rejected)

**Decision: add a SIBLING SSOT journey
`docs/product/journeys/reach-an-unconnected-udp-service.yaml`, with an
explicit `related_journeys` cross-reference added at BOTH ends.**

Rationale (why NOT extend `submit-a-udp-service.yaml`):

- The existing journey's central shared artifact is **REVERSE_NAT_MAP** —
  the CONNECTED-UDP / **XDP** wire path for **remote** backends (forward
  XDP SERVICE_MAP rewrite + reverse-NAT over `(backend_ip, backend_port,
  proto) → vip`). It fires for the client that calls `connect()`.
- THIS feature's central artifacts are **LOCAL_BACKEND_MAP** (forward) +
  the NEW **REVERSE_LOCAL_MAP** (reply) — the UNCONNECTED-UDP /
  **cgroup** path for **same-host** backends (`cgroup/sendmsg4` forward +
  `cgroup/recvmsg4` reply). It fires for the client that NEVER calls
  `connect()`.
- These are **different kernel paths and different reverse maps**.
  Branching the unconnected path inside the connected journey would invite
  a reader to install REVERSE_NAT_MAP semantics on the cgroup recvmsg4 hook
  (or vice versa) — a wrong-map-on-wrong-hook error. The DIVERGE dispatch
  preferred extend-with-cross-ref over duplication; here the artifacts
  diverge enough that a sibling is LESS duplicative than a branch, because
  the only shared SSOT is `service_proto` + the `ServiceFrontend` newtype
  (both cross-referenced, not copied).
- The connected-vs-unconnected distinction is made explicit at both ends
  via `related_journeys` blocks + dated changelog notes.

---

## Wave: DISCUSS / [REF] Locked decisions (DISCUSS)

| ID | Decision | Verdict | Source |
|---|---|---|---|
| **DD1** | Journey = SIBLING `reach-an-unconnected-udp-service.yaml`, not a branch of `submit-a-udp-service.yaml`; cross-ref at both ends. | LOCKED | this wave; § Journey decision |
| **DD2** | Persona = first-class `ana-platform-engineer.yaml` (Ana now anchors two UDP journeys). | LOCKED | this wave; existing journey reused "Ana" with no persona file |
| **DD3** | recvmsg4 + `REVERSE_LOCAL_MAP` are IN SCOPE with sendmsg4 (no sendmsg4-first split). | LOCKED (user-confirmed) | `recommendation.md` Option 2; dispatch |
| **DD4** | Walking skeleton = ONE same-host DNS-shape UDP service, ONE unconnected `sendto`/`recvfrom` round-trip, reply VIP-sourced (Slice 01). Forward-only is an internal milestone, NOT a shipped slice. | LOCKED | dispatch Decision 2; DIVERGE dissent verdict |
| **DD5** | Three elephant-carpaccio slices: 01 round-trip (WS) → 02 equivalence lockstep → 03 error hardening. | LOCKED | § Story map |
| **DD6** | Pure addition: connect4, `LOCAL_BACKEND_MAP` forward shape, action variants (proto-carrying, ADR-0053 Amd 3), hydrator classifier all UNCHANGED. Single-cut, no shims. | LOCKED | `recommendation.md` blast radius; `feedback_single_cut_greenfield_migrations.md` |
| **DD7** | ADR-0053 amendment (sendmsg4+recvmsg4 + REVERSE_LOCAL_MAP) is the ARCHITECT's job in DESIGN. DISCUSS forward-points only; edits NO ADR. | LOCKED | dispatch; CLAUDE.md "delegate to architect" |

---

## Wave: DISCUSS / [REF] System constraints (cross-cutting)

These bind every story and AC below.

- **Kernel floors:** `cgroup/sendmsg4` since 4.18; `cgroup/recvmsg4` since
  4.20. Both below the 5.10 LTS floor — no matrix bump (verified
  2026-06-05, `recommendation.md` kernel-viability table).
- **proto source:** `bpf_sock_addr.protocol`, zero-translation (ADR-0053
  Amd 2). Never default to TCP on this path.
- **`user_port` hazard:** low-16-NBO in a u32. Read/write must cast to u16
  then `from_be`/`to_be`; never byte-swap the full u32
  (`.claude/rules/development.md`).
- **No Tier-2 backstop:** `BPF_PROG_TEST_RUN` returns ENOTSUPP for
  `cgroup_sock_addr` on kernel ≤ 6.8. sendmsg4 + recvmsg4 correctness is a
  **Tier-3-only** gate; the structural defense below Tier-3 is the Tier-1
  `SimDataplane` equivalence invariant (J-PLAT-004).
- **Reverse mapping = atomic second write, NOT conntrack** (D7). UDP is
  stateless; `REVERSE_LOCAL_MAP` is written alongside the forward
  `register_local_backend` write (one logical write, two entries).
- **Fixture collision:** Tier-3 fixtures bind off systemd-resolved's UDP
  5353 (`.claude/rules/debugging.md` § 11).
- **No new GitHub issue:** #200 covers this. New sub-deferrals are surfaced
  for user approval, never `gh issue create`d by an agent (CLAUDE.md).

---

## Wave: DISCUSS / [REF] User stories with elevator pitches + acceptance criteria

### US-01: Reach a same-host UDP service from a client that never connects (WALKING SKELETON)

**Job:** J-OPS-004, J-PLAT-004 · **Slice:** 01

#### Problem

Ana deploys a same-host DNS-shape UDP service. `overdrive deploy` succeeds,
`overdrive alloc status` shows Running, and a hand-written connected-UDP
test even passes. Yet `dig @<vip> example.com` hangs, because `dig` uses
the unconnected `sendto` idiom and connect4 (which fires only at
`connect()` time) never intercepts the datagram — so `LOCAL_BACKEND_MAP` is
never consulted and the VIP→backend rewrite never happens. The service is a
half-working service: healthy upstream, unreachable from the client that
matters.

#### Who

Ana, platform engineer | single-node dev host | wants a real DNS service
reachable from unmodified resolvers, not a demo that needs a bespoke client.

#### Solution

A `cgroup/sendmsg4` program rewrites the unconnected request VIP→backend
over `LOCAL_BACKEND_MAP`, AND a `cgroup/recvmsg4` program rewrites the reply
source backend→VIP over a new `REVERSE_LOCAL_MAP` (written atomically with
the forward entry), so the resolver's source validation accepts the reply.

#### Elevator Pitch

Before: Ana cannot resolve through a same-host UDP service with a real
resolver — `dig @<vip>` hangs because the unconnected datagram is never
intercepted.
After: run `dig @<vip> example.com` → sees a correct DNS answer, and a
`tcpdump` capture shows the reply sourced from the VIP (not the backend IP).
Decision enabled: Ana decides the service is production-real — she can put
`<vip>` in `resolv.conf` and `getaddrinfo` will resolve through it.

#### Domain Examples

1. **Happy path** — Ana deploys `dns-resolver.toml` (one `[[listener]]`
   `protocol = "udp"`, one local backend bound to `10.244.0.7:5300`,
   VIP `10.96.0.10:53`). `dig @10.96.0.10 example.com` returns
   `example.com. 300 IN A 93.184.216.34`. `tcpdump` shows the reply
   `src 10.96.0.10:53` — the VIP, not `10.244.0.7`.
2. **Edge — second query reuses the same mapping** — Ana runs `dig
   @10.96.0.10 example.org` immediately after; the same `LOCAL_BACKEND_MAP`
   /`REVERSE_LOCAL_MAP` entries serve it (no per-flow state; stateless).
3. **Boundary — forward-rewrite-only (internal milestone, NOT shipped)** —
   during development, with sendmsg4 landed but recvmsg4 absent, `dig
   @10.96.0.10 example.com` hangs and `tcpdump` shows a reply
   `src 10.244.0.7` that `dig` discards on source validation — empirically
   reproducing the trap recvmsg4 closes (kernel `983695fa6765`).

#### UAT Scenarios (BDD)

##### Scenario: A same-host resolver query through the VIP returns an answer

```
Given Ana has deployed a same-host UDP DNS service on VIP 10.96.0.10:53
  with one local backend, via `overdrive deploy dns-resolver.toml`
When Ana runs `dig @10.96.0.10 example.com` (an unconnected sendto, no connect)
Then dig returns a correct A record for example.com
```

##### Scenario: The reply is sourced from the VIP, never the backend

```
Given the same deployed service and a Tier-3 wire capture on overdrive.slice
When an unconnected sendto/recvfrom round-trip completes to 10.96.0.10:53
Then the captured reply packet's source address is 10.96.0.10 (the VIP)
  And no reply ever leaves with the backend IP 10.244.0.7 as its source
```

##### Scenario: Forward and reverse mappings are present together after one registration

```
Given Ana has deployed the same-host UDP service
When the platform registers the local backend
Then `bpftool map dump LOCAL_BACKEND_MAP` shows (10.96.0.10, 53, udp) -> backend
  And `bpftool map dump REVERSE_LOCAL_MAP` shows backend -> 10.96.0.10
  And there is no observable window where the forward entry exists alone
```

#### Acceptance Criteria

- [ ] `dig @<vip> example.com` against a single same-host DNS-shape UDP
      service returns a correct answer (unconnected `sendto`, no `connect`).
- [ ] Tier-3 `tcpdump` shows the reply source = the VIP, never the backend IP.
- [ ] `bpftool map dump` shows both the forward `LOCAL_BACKEND_MAP` and the
      reverse `REVERSE_LOCAL_MAP` entries after one `register_local_backend`.
- [ ] Forward + reverse entries are written by one atomic action (no
      forward-without-reverse window).
- [ ] connect4 / forward-map shape / hydrator classifier UNCHANGED (pure
      addition).

#### Technical Notes

- Tier-3-only correctness gate (no `BPF_PROG_TEST_RUN` for
  cgroup_sock_addr). `user_port` low-16-NBO handling required. Fixture
  avoids UDP 5353. DESIGN owns the `REVERSE_LOCAL_MAP` shape + atomic-write
  contract (ADR-0053 amendment).

---

### US-02: Trust the VIP-sourced reply guarantee won't silently regress

**Job:** J-PLAT-004 (primary), J-OPS-004 · **Slice:** 02

#### Problem

The dataplane author cannot tell, from a green test suite alone, whether
the reply-path source rewrite is actually asserted or merely incidentally
passing. A forward-only or asymmetric change (the #163 class) can pass
every existing test and silently ship a half-working service — exactly the
failure recvmsg4 was added to prevent.

#### Who

The dataplane author / Ana | reasons about Sim≡kernel equivalence | wants a
mechanical guarantee, not discipline, that the reply leg stays symmetric.

#### Solution

A Tier-1 `SimDataplane` invariant pins the reply-path source identity
(reply source = VIP) alongside the forward rewrite, meeting a Tier-3
real-kernel acceptance at the shared backend identity — the two-pronged
pin (the in-process both-adapter retarget is infeasible).

#### Elevator Pitch

Before: a forward-only / asymmetric reply-path regression passes the suite
and ships silently.
After: run the per-PR gate (`cargo xtask lima run -- cargo nextest run
--features integration-tests`) → sees the unconnected-UDP reply-path
equivalence invariant assert the Sim reply source is the VIP; a forced
forward-only mutation turns it RED.
Decision enabled: the team decides the VIP-sourced-reply guarantee is safe
to depend on — it cannot regress to the backend-sourced-reply trap undetected.

#### Domain Examples

1. **Happy path** — the Tier-1 invariant evaluates the SimDataplane for
   VIP `10.96.0.10` and asserts the reply source it would present is
   `10.96.0.10`; green on the per-PR critical path.
2. **Edge — kernel-side pin** — the Tier-3 acceptance drives the real
   recvmsg4 path and asserts `tcpdump` shows `src 10.96.0.10`; green on the
   integration lane.
3. **Mutation — forward-only Sim** — a mutation dropping the Sim reply
   rewrite makes the invariant assert `src 10.244.0.7 ≠ VIP` → RED (the
   mutation this slice exists to kill).

#### UAT Scenarios (BDD)

##### Scenario: The reply-path source identity is asserted in the Sim layer

```
Given a SimDataplane configured with the unconnected-UDP service from US-01
When the reply-path equivalence invariant evaluates the declared frontend
Then it asserts the Sim reply source equals the VIP, not the backend
  And the invariant runs on the per-PR critical path
```

##### Scenario: A forward-only regression fails the gate loudly

```
Given the reply-path rewrite is removed from the Sim adapter (a mutation)
When the per-PR gate runs
Then the reply-path equivalence invariant turns RED
  And the regression is caught at PR time, not in production
```

#### Acceptance Criteria

- [ ] A Tier-1 invariant asserts the SimDataplane unconnected-UDP reply
      source = VIP for the declared frontend, on the per-PR critical path.
- [ ] Removing the Sim reply rewrite FAILS the Tier-1 invariant (verified
      by RED scaffold or mutation target, not inspection).
- [ ] Removing the kernel reply rewrite FAILS the Tier-3 acceptance.
- [ ] Both adapters derive the reverse mapping from the same forward
      registration (no independent reply-path source of truth).

#### Technical Notes

- Mirrors the `ReverseNatLockstep` two-pronged shape from
  submit-a-udp-service.yaml step 4, retargeted to the cgroup same-host
  reply path. Tier-1 stays in the default lane; Tier-3 in the
  integration lane.

---

### US-03: Diagnose a misconfigured reply path without a backend-IP leak

**Job:** J-OPS-004 (primary), J-PLAT-004 · **Slice:** 03

#### Problem

When the reverse mapping is missing or the host kernel is below floor, Ana
needs the failure to be clean and diagnosable. The worst outcome is a reply
that leaks the backend IP (the client discards it and the query times out
with no signal) or a silent hang she cannot distinguish from a platform
bug.

#### Who

Ana | diagnosing a non-answering service | wants to tell HER
misconfiguration from a platform bug using `dig`/`tcpdump`/`bpftool`.

#### Solution

`recvmsg4` on a `REVERSE_LOCAL_MAP` miss fails safe (no backend-IP-sourced
reply ever reaches a client; the miss is counted/observable); a host below
the recvmsg4 floor refuses/warns observably rather than delivering a
forward-only half-working service.

#### Elevator Pitch

Before: a missing reverse entry leaks the backend IP, the resolver
discards the reply, and `dig` hangs with no diagnosable signal.
After: run `dig @<vip> example.com` against a service whose reverse entry is
absent → sees a clean failure, `bpftool map dump REVERSE_LOCAL_MAP` shows
the missing entry, and a `tcpdump` shows NO backend-IP-sourced reply left
the host.
Decision enabled: Ana decides "this is my misconfiguration, not a platform
bug" — and fixes the spec instead of filing a false dataplane report.

#### Domain Examples

1. **Reverse miss** — forward entry present, reverse forced absent: `dig`
   fails cleanly; no `src 10.244.0.7` reply on the wire; a miss counter
   increments.
2. **Below floor** — a host on kernel 4.15 (below recvmsg4's 4.20): attach
   preflight refuses/warns observably; no forward-only half-working
   service is delivered.
3. **Fixture collision** — the Tier-3 stub resolver binds off UDP 5353 and
   asserts a clean `bind`; an `EADDRINUSE` fails the test loudly.

#### UAT Scenarios (BDD)

##### Scenario: A missing reverse entry never leaks the backend IP

```
Given the forward LOCAL_BACKEND_MAP entry is present but REVERSE_LOCAL_MAP
  has no entry for the backend
When an unconnected reply traverses recvmsg4
Then no reply reaches the client sourced from the backend IP
  And the miss is observable via a counter or log, not silent
```

##### Scenario: A below-floor kernel refuses observably

```
Given a host whose kernel predates recvmsg4 (< 4.20)
When the platform attaches the same-host UDP hooks
Then the attach/preflight refuses or warns observably
  And the platform does not deliver a forward-only half-working service
```

#### Acceptance Criteria

- [ ] With the forward entry present and the reverse entry forced absent,
      no client-bound reply is sourced from the backend IP (Tier-3
      `tcpdump`); the miss is observable.
- [ ] A below-floor host refuses or warns observably at attach/preflight.
- [ ] The Tier-3 fixture binds off UDP 5353 and asserts a clean `bind`
      (collision fails loudly).

#### Technical Notes

- Floor check mirrors the cgroup-preflight refusal precedent (ADR-0028 /
  ADR-0034). No-leak miss handling uses the `DropClass`-counted discipline
  (`crates/overdrive-core/src/dataplane/drop_class.rs`). Drop-vs-pass on
  miss is a DESIGN decision (ADR-0053 amendment); the AC pins only
  "no backend-IP-sourced reply reaches a client."

---

## Wave: DISCUSS / [REF] Story map + walking skeleton + elephant carpaccio

**Backbone (operator activities, left→right):**
Declare a same-host UDP service → Register backend (forward + reverse maps)
→ Reach the backend (unconnected request) → Receive a VIP-sourced reply →
Trust it won't regress / diagnose it when it breaks.

**Walking skeleton (minimum end-to-end value):** Slice 01 — ONE same-host
DNS-shape UDP service, ONE backend, ONE unconnected `sendto`/`recvfrom`
round-trip, reply VIP-sourced. Spans the full backbone thinly. Forward-only
is an internal milestone within Slice 01, **never a shipped slice** (the
DIVERGE dissent verdict: shipping forward-only is the J-OPS-004
operator-trust violation).

**Slices (each ~1 day target — Slice 01 flagged ≤1.5d, see brief —
end-to-end, value-bearing — none `@infrastructure`-only):**

| # | Slice | Story | Learning hypothesis (disproves if it fails) | WS? |
|---|---|---|---|---|
| 01 | Unconnected round-trip | US-01 | Disproves "forward rewrite is sufficient" — reproduces the half-working trap, proving recvmsg4 is load-bearing | **YES** |
| 02 | Reply-path equivalence lockstep | US-02 | Disproves "the equivalence surface covers the reply leg" if a forward-only Sim mutation does NOT turn it red | no |
| 03 | Reply-path error hardening | US-03 | Disproves "the reply path fails safe" if a forced reverse-miss leaks the backend IP | no |

**Carpaccio taste tests:**

- *4+ new components per slice?* — No. Slice 01 adds 2 programs + 1 map +
  1 atomic-write extension (the irreducible WS; flagged as the one
  >1-day risk). Slices 02/03 add a test surface + an error path each.
- *Every slice depends on a new abstraction?* — No new shared abstraction;
  this is a pure addition over the shipped connect4 surface (DD6). (Option
  3's shared helper was explicitly NOT adopted — it would modify shipped
  connect4 code.)
- *Does any slice disprove a pre-commitment?* — YES. Slice 01 disproves
  "forward-only suffices" (the central DIVERGE bet); Slice 02 disproves
  "the equivalence invariant is decorative." Not decoration.
- *Synthetic-data-only?* — No. Slice 01 uses a real `dig`/`sendto`
  round-trip against a real stub resolver (production-shape).
- *2+ slices identical except for scale?* — No; each is a distinct
  behaviour (deliver → guard → harden).

**Priority rationale:** 01 first (highest learning leverage — it
empirically confirms or disproves the whole feature bet, and is the
dogfood moment: `dig @<vip>` answers). 02 second (the regression guard is
worthless before the behaviour exists, and cheap once it does). 03 last
(error hardening presupposes both the happy path and the equivalence
surface to assert error-path source identity against). Dependency chain:
01 → 02 → 03, strictly.

---

## Wave: DISCUSS / [REF] WS strategy

**Strategy: B — Vertical thin slice** (per Mandate 5). The walking skeleton
(Slice 01) is one thin end-to-end vertical: a single same-host UDP service
carried from `overdrive deploy` through both kernel hooks to a VIP-sourced
`dig` answer. Not A (no greenfield scaffold needed — pure addition over the
shipped connect4 surface), not D (no env-switching), not C (no horizontal
layer-by-layer build — the value is the round-trip, which is inherently
vertical).

---

## Wave: DISCUSS / [REF] Outcome KPIs

### Objective

A same-host UDP service is reachable from the canonical unconnected
resolver and answers with a VIP-sourced reply — with the symmetry pinned
so it cannot silently regress. Timeboxed to the #200 DELIVER completion.

### Outcome KPIs (mapped to O1–O5)

| # | ODI | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|---|
| K1 | O1 | A same-host unconnected-UDP client (`dig`/`sendto`) | resolves through a same-host UDP service VIP | 100% of unconnected `dig @<vip>` round-trips return a correct answer (Slice 01 acceptance) | 0% (unreachable via unconnected path today) | Tier-3 acceptance: real `dig`/`sendto` round-trip through `overdrive.slice` | Leading (Activation) |
| K2 | O2 | The same client | receives a reply sourced from the VIP | 100% of replies are VIP-sourced; 0 backend-IP-sourced replies reach a client | n/a (connect4 path never sees these datagrams) | Tier-3 `tcpdump` wire capture on the reply path | Leading (primary — the discriminator) |
| K3 | O3 | The dataplane | keeps Sim and the real kernel equivalent on the reply-path source identity | 0 asymmetry escapes; the Tier-1 reply-path equivalence invariant present + green on every PR; a forward-only mutation is killed | n/a (no hook, no equivalence surface today) | Tier-1 `SimDataplane` invariant on the per-PR critical path + Tier-3 meet-at-backend-identity acceptance | Leading (Guardrail — the regression gate) |
| K4 | O4 | The dataplane author | adds the unconnected path WITHOUT re-migrating connect4 call sites | 0 changes to connect4 / forward-map shape / hydrator classifier (pure addition; diff is additive only) | n/a | PR diff review: no connect4/forward-shape/classifier modification | Leading (Secondary) |
| K5 | O5 | The reply path | fails safe when the reverse mapping is missing or the kernel is below floor | 0 backend-IP-sourced replies leak on a reverse miss; below-floor hosts refuse/warn observably (Slice 03) | n/a | Tier-3 forced-miss + below-floor acceptance; miss counter | Leading (Guardrail) |

### Metric hierarchy

- **North Star:** K2 — % of replies sourced from the VIP (the
  reply-path-correctness outcome that separates a real service from a
  half-working one). Target 100%.
- **Leading indicators:** K1 (reachability), K3 (equivalence-green rate).
- **Guardrail metrics (must NOT degrade):** connect4 / connected-UDP / TCP
  delivery unchanged (K4); no backend-IP leak on miss (K5).

### Measurement plan

| KPI | Data source | Collection method | Frequency | Owner |
|---|---|---|---|---|
| K1 | Tier-3 acceptance run | `dig`/`sendto` round-trip in Lima | per-PR (integration lane) | dataplane author |
| K2 | Tier-3 wire capture | `tcpdump` source-address assertion | per-PR (integration lane) | dataplane author |
| K3 | Tier-1 invariant + Tier-3 acceptance | `cargo nextest run` (default + integration) | per-PR (critical path) | dataplane author |
| K4 | PR diff | reviewer check (additive-only) | per-PR | reviewer |
| K5 | Tier-3 forced-miss / below-floor acceptance | `tcpdump` + miss counter | per-PR (integration lane) | dataplane author |

### Hypothesis

We believe that adding `cgroup/sendmsg4` + `cgroup/recvmsg4` +
`REVERSE_LOCAL_MAP` (Option 2) for Ana's same-host UDP services will make
them reachable from unconnected resolvers with VIP-sourced replies. We will
know this is true when an unconnected `dig @<vip>` returns a correct answer
(K1=100%) and every reply is VIP-sourced (K2=100%), with the Sim≡kernel
reply-path equivalence invariant green (K3, 0 escapes).

---

## Wave: DISCUSS / [REF] Definition of Ready — 9-item validation

| # | DoR item | Status | Evidence |
|---|---|---|---|
| 1 | Problem statement clear, domain language | ✓ PASS | Each story's Problem section frames operator pain (half-working service, backend-IP leak) in Ana's vocabulary; no solution-prescription in the problem. |
| 2 | User/persona with specific characteristics | ✓ PASS | `ana-platform-engineer.yaml` created — name, role, context, motivations, emotional arc, frustrations, success signals. |
| 3 | 3+ domain examples with real data | ✓ PASS | Each story has 3 examples with real data (VIP 10.96.0.10:53, backend 10.244.0.7:5300, `example.com` A 93.184.216.34). |
| 4 | UAT in Given/When/Then (3–7 scenarios) | ✓ PASS | US-01: 3 scenarios; US-02: 2; US-03: 2 — all business-outcome-titled (no implementation-detail titles). |
| 5 | AC derived from UAT | ✓ PASS | Each story's AC checklist maps 1:1 to its scenarios; ACs verify the elevator-pitch "After" command end-to-end. |
| 6 | Right-sized (1–3 days, 3–7 scenarios) | ✓ PASS (with flag) | 3 slices, each ≤1 day target; Slice 01 flagged as the one >1-day risk (the irreducible WS round-trip + Tier-3 fixture) with a conditional pre-slice SPIKE — surfaced, not hidden. |
| 7 | Technical notes: constraints/dependencies | ✓ PASS | § System constraints + per-story Technical Notes (kernel floors, NBO hazard, no-Tier-2, atomic reverse write, fixture collision). |
| 8 | Dependencies resolved or tracked | ✓ PASS | Shipped deps named (connect4, LOCAL_BACKEND_MAP, register_local_backend, hydrator); DESIGN dep (ADR-0053 amendment) forward-pointed to the architect; #200 tracks the work. |
| 9 | Outcome KPIs defined with measurable targets | ✓ PASS | K1–K5 mapped to O1–O5, each with numeric target + measurement method + type. |

**DoR verdict: 9/9 PASS.** Requirements completeness: see § Completeness.

---

## Wave: DISCUSS / [REF] Requirements completeness

**Score: 0.97 (> 0.95 gate).** Rationale: 3 stories, all with job
traceability (J-OPS-004 + J-PLAT-004), complete elevator pitches, 3+ real
examples, 2–3 BDD scenarios each, AC-from-UAT, KPIs mapped to ODI
outcomes, and a DESIGN-owned residual (the ADR-0053 amendment + the
drop-vs-pass-on-miss decision) that is explicitly forward-pointed rather
than left ambiguous. The 0.03 gap is the single genuine open question
(reverse store = second map vs reverse scan; drop vs pass on miss) which is
correctly a DESIGN decision, not a DISCUSS gap.

---

## Wave: DISCUSS / [REF] Out of scope (explicit non-goals)

- Reopening connect4 / the connected-UDP path (ADR-0053 locked, shipping).
- The CONNECTED-UDP / XDP REVERSE_NAT_MAP wire path
  (`submit-a-udp-service.yaml` / GH #163 — a different kernel path).
- A conntrack / per-flow state table (UDP is stateless; D7 rejects it).
- Option 3's shared-helper refactor of connect4 (a DESIGN-time refinement
  *of* Option 2, only if the architect judges the connect4-refactor risk
  worth the marginal-surface win — not a DISCUSS commitment).
- A sendmsg4-first ship split (the DIVERGE dissent path) — user confirmed
  recvmsg4 is in scope; would require a new user-approved tracking issue.
- Multi-backend weighted selection / health-driven removal (rides the
  existing hydrator + `Backend.healthy`; not a reply-path concern).
- The ADR-0053 amendment text itself (architect's job in DESIGN).

---

## Wave: DISCUSS / [REF] Wave decisions summary

### Key decisions

- **[DD1]** Sibling journey, not a branch (different kernel path + reverse
  map) — see § Journey decision; `reach-an-unconnected-udp-service.yaml`.
- **[DD2]** First-class Ana persona (anchors two UDP journeys).
- **[DD3–DD4]** recvmsg4 + REVERSE_LOCAL_MAP in scope; WS = the full
  unconnected round-trip (forward-only is an internal milestone only).
- **[DD5]** Three carpaccio slices: 01 round-trip (WS) → 02 equivalence →
  03 error hardening.
- **[DD6]** Pure addition; single-cut, no shims.
- **[DD7]** ADR-0053 amendment is the architect's DESIGN job.

### Requirements summary

- **Primary need:** a same-host UDP service reachable from the canonical
  unconnected resolver, with a VIP-sourced reply, pinned against silent
  reply-path asymmetry.
- **Walking-skeleton scope:** Slice 01 (one service, one backend, one
  unconnected round-trip, reply VIP-sourced).
- **Feature type:** Backend (eBPF dataplane delivery path; one
  operator-observable surface).

### Constraints established

- Kernel floors below 5.10 (no bump); proto zero-translation; `user_port`
  NBO hazard; no Tier-2 backstop (Tier-3-only + Tier-1 equivalence);
  atomic reverse write (no conntrack); fixture avoids UDP 5353; no agent
  GitHub-issue creation.

### Upstream changes (back-propagation)

- None to DISCOVER (no `discover/` for this feature — DIVERGE-first).
- SSOT: created `ana-platform-engineer.yaml`; created
  `reach-an-unconnected-udp-service.yaml`; cross-referenced
  `submit-a-udp-service.yaml`. `jobs.yaml` already carried the 2026-06-05
  DIVERGE changelog entry (no new job — verified, unchanged this wave).

---

## Wave: DISCUSS / [REF] Handoff

- **To DESIGN (nw-solution-architect):** US-01/02/03 + slice briefs + the
  two SSOT journeys + Ana persona + outcome KPIs. DESIGN owns the ADR-0053
  amendment (REVERSE_LOCAL_MAP shape, atomic-write contract, drop-vs-pass
  on reverse miss, AND the reverse-key *composition* — `backend_ip` alone
  vs `(backend_ip, backend_port)`, which matters the moment two services
  share a backend IP on different ports; pin it explicitly per the peer
  review's finding #3) and may adopt Option 3's shared helper as a
  refinement of Option 2 if the connect4-refactor risk is judged
  worthwhile.
- **To DEVOPS (nw-platform-architect):** outcome KPIs K1–K5 only (drive
  Tier-3 instrumentation: wire-capture source-address assertions, miss
  counters, equivalence-green tracking).
- **To DISTILL (nw-acceptance-designer):** the BDD scenarios + integration
  points (the Tier-1 equivalence invariant + Tier-3 round-trip / forced-miss
  / below-floor acceptances) + KPIs.
- **Per-wave peer review:** EXECUTED (Eclipse / nw-product-owner-reviewer,
  2026-06-05, **APPROVED** — see § Peer review below). The mandatory
  consolidated review at end of DISTILL (Eclipse + Architect + Forge +
  Sentinel against the full feature-delta) still covers this wave in
  aggregate.

## Wave: DISCUSS / [REF] Peer review

**Reviewer:** Eclipse (nw-product-owner-reviewer, run on inherited Opus per
`rigor.reviewer_model = inherit`) · **Date:** 2026-06-05 ·
**Verdict: APPROVED** (pre-DESIGN gate cleared; no blocking issues).

All 9 scrutiny dimensions PASS: job traceability (every story → J-OPS-004 /
J-PLAT-004, no orphans, no new job); elevator-pitch test (every "After" is a
real operator-invocable command with concrete observable output);
slice-composition hard gate (no `@infrastructure`-only slice); carpaccio
discipline (forward-only correctly an internal milestone of Slice 01, never
shipped; SPIKE framing honest, not a hidden oversized slice); journey
coherence + emotional arc (the silent-asymmetry "Uneasy" beat is earned, all
error paths mapped); shared-artifact tracking (connected-XDP-REVERSE_NAT vs
unconnected-cgroup-LOCAL_BACKEND+REVERSE_LOCAL kept provably distinct at both
journey ends — the wrong-map-on-wrong-hook trap defused); outcome KPIs
(K1–K5 numeric targets + real measurement methods, honest about the
no-Tier-2-backstop reality); DoR 9/9 with completeness 0.97 judged justified,
not inflated; scope hygiene (connect4 not reopened, pure-addition accurate,
single-cut, reverse-store question correctly deferred to DESIGN, no ADR edit,
no GH issue, #200 cited by number).

Praise: (1) the sibling-journey decision prevents a class of bug rather than
merely documenting it; (2) the walking skeleton refuses to ship forward-only
and frames the Tier-3-fixture SPIKE honestly.

Non-blocking findings, all actioned in this revision: nitpick #1 (the
"≤1 day target" header overstated Slice 01's ~1–1.5d estimate — softened
above); nitpick #2 (the stale "peer review SKIPPED" line — corrected above);
suggestion #3 for DESIGN (pin the reverse-key *composition* — `backend_ip`
vs `(backend_ip, backend_port)` — explicitly in the ADR-0053 amendment;
folded into the DESIGN handoff above). No revision to user stories, ACs,
KPIs, slices, journeys, or persona was required.
