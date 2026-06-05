<!-- markdownlint-disable MD024 -->
# Feature Delta — unconnected-udp-sendmsg4

**Feature-id:** `unconnected-udp-sendmsg4` · **GH:** [#200](https://github.com/overdrive-sh/overdrive/issues/200)
· **Waves present:** DIVERGE (accepted) → DISCUSS (approved) → **DESIGN** (this wave)
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

---

## Wave: DESIGN / [REF] Overview

**Architect:** Morgan. **Date:** 2026-06-05. **Mode:** GUIDE (framing pass
complete; all decisions user-locked). **Density:** lean + ask-intelligent.
**Paradigm:** object-oriented (project CLAUDE.md; `@nw-software-crafter` for
implementation — not re-asked).

**SSOT for this DESIGN wave:** **ADR-0053 revision 2026-06-05** +
`docs/product/architecture/brief.md` § "Unconnected-UDP sendmsg4 extension"
+ `docs/product/architecture/c4-diagrams.md` § "Unconnected-UDP sendmsg4 +
recvmsg4". This feature-delta section is the decision-and-reuse summary; it
does not supersede the ADR.

**Per-wave architect review:** DEFERRED to the mandatory consolidated review
at end of DISTILL (per the nWave per-wave-review-optional rule); NOT
self-invoked here.

---

## Wave: DESIGN / [REF] DDD decisions

| # | Decision | Verdict | One-line rationale |
|---|---|---|---|
| **DDD-1** | Reverse store = a second BPF map `REVERSE_LOCAL_MAP` (`BPF_MAP_TYPE_HASH`), written in ordered (reverse-first) sequence by `register_local_backend` (two map syscalls, not one transaction — an ordering guarantee, not atomicity). | LOCKED | A reverse scan is O(N)/datagram on the recvmsg hot path; conntrack models flow state UDP doesn't have; a second point-lookup map is the only stateless O(1) reply store. (ADR-0053 rev D1.) |
| **DDD-2** | Reverse key = `(backend_ip, backend_port, proto)` reusing the existing `BackendKey` newtype. | LOCKED | `backend_ip` alone is ambiguous when two services share a backend IP on different ports; `BackendKey` reuse buys byte-parity with the three existing keys + a free Sim mirror. (ADR-0053 rev D2.) |
| **DDD-3** | Reverse-miss = **pure no-op** (real source intact, `REVERSE_LOCAL_MISS_COUNTER` bumped only); HIT = rewrite source→VIP; recvmsg4 CANNOT deny (verifier `[1,1]`). **Corrected 2026-06-05b (UI-1)** — was "rewrite-to-sentinel `192.0.2.1`." | LOCKED (corrected) | recvmsg4 fires on ALL subtree unconnected UDP (cgroup-ancestor attach), so a miss = "not a service reply"; sentinel-ing it corrupts non-service traffic (Tier-3-observed, step 01-03). The map HIT is the discriminator; no-op-on-miss is Cilium-aligned. K5's no-leak holds via the D1 reverse-first dual-write (always-hit), not a sentinel. (ADR-0053 rev D3 sub-revision 2026-06-05b; research addendum UI-1 adjudication.) |
| **DDD-3a** | AC reframing US-01/US-03/K2/K5: wire-layer → application-sockaddr-layer. | LOCKED (back-prop REQUIRED) | recvmsg4 fires after the skb is on the socket queue; `tcpdump -i lo` shows the backend source regardless. Wire no-leak is XDP's domain, not recvmsg4's. Layer/wording correction, not scope change. (ADR-0053 rev D3; `design/upstream-changes.md`.) |
| **DDD-4** | Option 3 — ONE shared `#[inline(always)]` `build_local_service_key` helper (key-build + NBO ONLY) across connect4 + sendmsg4 + recvmsg4; ONE attach orchestration + ONE probe set. Map lookup + rewrite direction differ per hook and stay in each program body. | LOCKED (user override of Morgan's Option-2) | Single key-construction + NBO site across three hooks; connect4/sendmsg4 look up `LOCAL_BACKEND_MAP` (forward dest-rewrite), recvmsg4 looks up `REVERSE_LOCAL_MAP` (reverse source-rewrite) — one helper MUST NOT serve both rewrite directions. REFACTORS shipped connect4 (behavior-preserving, Tier-3-reverified, no Tier-2 backstop). (ADR-0053 rev D4.) |
| **DDD-5a** | `register_local_backend` writes BOTH maps reverse-first; NO new trait method; contract rustdoc amended (postcondition + observable invariant + edge case). | LOCKED | One logical write installs forward+reverse; the existing method body extends, the trait surface does not grow. (ADR-0053 rev D5a.) |
| **DDD-5b/c** | Probe attaches both new hooks + round-trips a `REVERSE_LOCAL_MAP` sentinel; the `attach()` syscall IS the below-floor preflight; `#[from]`-routed error variant(s), never `Internal(String)`. | LOCKED | The kernel-authoritative attach is the honest floor check; `/proc`/`uname` parsing would re-introduce the `unwrap_or_default` boundary-read footgun. (ADR-0053 rev D5b/c.) |
| **DDD-5d** | `SimDataplane` reply mirror `BTreeMap<BackendKey, Ipv4Addr>` under the SAME mutex acquisition as `local_backends`; `reply_source_for()` test accessor; models the observable contract only. | LOCKED | Tier-1 reply-path equivalence pin without a kernel; MUST NOT shape production (the production reverse-first dual-write is written to the contract; the Sim mirrors the post-state). (ADR-0053 rev D5d.) |
| **DDD-5e** | `user_port` low-16-NBO read idiom copied verbatim into the shared `build_local_service_key` helper; recvmsg4 writable fields confirmed = `user_ip4`/`user_port` (`msg_src_ip4` is sendmsg-only). | LOCKED | One correct read-side NBO site for all three hooks; the write-side (rewrite) NBO stays per-hook; research Q2 verified the recvmsg4 source-rewrite handles. (ADR-0053 rev D5e.) |

---

## Wave: DESIGN / [REF] Component decomposition

| Component | Path | Change type |
|---|---|---|
| `cgroup_sendmsg4_service` program | `crates/overdrive-bpf/src/programs/cgroup_sendmsg4_service.rs` | **CREATE NEW** |
| `cgroup_recvmsg4_service` program | `crates/overdrive-bpf/src/programs/cgroup_recvmsg4_service.rs` | **CREATE NEW** |
| `build_local_service_key` shared helper (key-build + NBO only; no lookup, no rewrite) | `crates/overdrive-bpf/src/shared/build_local_service_key.rs` | **CREATE NEW** |
| `REVERSE_LOCAL_MAP` kernel map | `crates/overdrive-bpf/src/maps/reverse_local_map.rs` | **CREATE NEW** |
| `REVERSE_LOCAL_MISS_COUNTER` (PERCPU_ARRAY) | `crates/overdrive-bpf/src/maps/` | **CREATE NEW** |
| `ReverseLocalMapHandle` userspace handle | `crates/overdrive-dataplane/src/maps/reverse_local_map_handle.rs` | **CREATE NEW** |
| `cgroup_connect4_service` key-build / NBO | `crates/overdrive-bpf/src/programs/cgroup_connect4_service.rs` | **EXTEND** (key-build + NBO refactored to call the shared helper; own `LOCAL_BACKEND_MAP` lookup + forward dest-rewrite stay in body — behavior-preserving, Tier-3-reverified) |
| `Dataplane::register_local_backend` / `deregister_local_backend` | `crates/overdrive-core/src/traits/dataplane.rs` | **EXTEND** (body: reverse-first dual-write; rustdoc contract amended). **Superseding note (2026-06-05 / DDD-5a, commit `3559e4e2`, GH #211):** `deregister_local_backend` later gained `backend: SocketAddrV4`; its reverse removal is keyed on the caller-supplied backend (NOT a read-back of the forward entry) so a partial-failure retry is safe. `Action::DeregisterLocalBackend` gained `backend` to match. See ADR-0053 § "Revision 2026-06-05 — DDD-5a". |
| `EbpfDataplane::register_local_backend` | `crates/overdrive-dataplane/src/lib.rs` | **EXTEND** (write `REVERSE_LOCAL_MAP` reverse-first; attach+probe both new hooks) |
| `SimDataplane` | `crates/overdrive-sim/src/adapters/dataplane.rs` | **EXTEND** (reply mirror + `reply_source_for()`) |
| `DataplaneError` / `DataplaneBootError` | `crates/overdrive-core/src/traits/dataplane.rs` (+ dataplane boot error home) | **EXTEND** (`CgroupSendRecvAttach`, `ReverseLocalProbe` `#[from]`-routed) |
| `LOCAL_BACKEND_MAP`, `BackendKey`, `Proto`, hydrator classifier, `Action::RegisterLocalBackend`, `cgroup_attach_path` | (shipped) | **REUSE** (unchanged) |

---

## Wave: DESIGN / [REF] C4 delta (Mermaid)

**L1 System Context: UNCHANGED.** No new external actor/system; the delta
(two cgroup hook types + one kernel map) is internal to the existing
`overdrive ↔ kernel` relationship. No L1 reproduced (redundant). **L3:** not
warranted — the shared-helper call graph is captured in the component
decomposition above. Canonical L2 lives in
`docs/product/architecture/c4-diagrams.md` § "Unconnected-UDP sendmsg4 +
recvmsg4"; reproduced here for the DELIVER reader:

```mermaid
C4Container
  title Container delta — Unconnected-UDP sendmsg4 + recvmsg4 (GH #200)

  Person(ana, "Platform Engineer (Ana)", "overdrive deploy + unconnected dig @<vip>")
  Container(dp, "overdrive-dataplane (Ebpf)", "adapter-host", "NEW ReverseLocalMapHandle; register_local_backend writes REVERSE_LOCAL_MAP reverse-first then LOCAL_BACKEND_MAP; probe attaches both new hooks")
  Container(bpf, "overdrive-bpf", "no_std BPF", "NEW sendmsg4 + recvmsg4 programs; NEW shared build_local_service_key helper (key-build + NBO only; per-hook lookup + rewrite stay in each program; connect4 refactored to call it)")
  Container(sim, "overdrive-sim", "adapter-sim", "reply mirror BTreeMap<BackendKey,Ipv4Addr> under one mutex; reply_source_for() accessor")

  System_Boundary(kern, "Linux kernel, overdrive.slice cgroup") {
    Container(connect4, "cgroup/connect4 (shipped, REFACTORED)", "BPF_CGROUP_INET4_CONNECT", "TCP + connected-UDP forward dst rewrite; key via shared helper, own LOCAL_BACKEND_MAP lookup")
    Container(sendmsg4, "cgroup/sendmsg4 (NEW, >=4.18)", "BPF_CGROUP_UDP4_SENDMSG", "Unconnected forward dst rewrite VIP->backend; key via shared helper, own LOCAL_BACKEND_MAP lookup")
    Container(recvmsg4, "cgroup/recvmsg4 (NEW, >=4.20)", "BPF_CGROUP_UDP4_RECVMSG", "Fires on ALL subtree unconnected UDP; map HIT (service reply) -> src rewrite backend->VIP; MISS (non-service) -> no-op + counter; [1,1] cannot-deny")
    ContainerDb(fwdmap, "LOCAL_BACKEND_MAP (shipped)", "HASH", "(vip,vip_port,proto)->backend")
    ContainerDb(revmap, "REVERSE_LOCAL_MAP (NEW)", "HASH", "BackendKey(ip,port,proto)->VIP")
    ContainerDb(xdp, "SERVICE_MAP / REVERSE_NAT (shipped, XDP)", "BPF maps", "DISTINCT wire path for remote/connected — untouched")
  }

  Rel(ana, sendmsg4, "Unconnected sendto(VIP:53)")
  Rel(dp, revmap, "1. upsert reverse (FIRST)")
  Rel(dp, fwdmap, "2. upsert forward")
  Rel(sendmsg4, fwdmap, "key via shared helper -> own lookup -> forward dst rewrite")
  Rel(recvmsg4, revmap, "key via shared helper -> own lookup -> HIT: reverse src rewrite to VIP / MISS: no-op + counter")
  Rel(connect4, fwdmap, "key via shared helper -> own lookup (REFACTORED)")
```

---

## Wave: DESIGN / [REF] Ports and adapters

**Driving ports (none new).** The operator surface is unchanged
`overdrive deploy <SPEC>`. The two new cgroup hooks are **driven** by the
kernel (the kernel invokes them at `sendmsg`/`recvmsg` syscall time); they
are not a driving port. No new CLI verb, no new HTTP route.

**Driven ports + adapters.**

| Port (trait) | Adapter(s) | Delta |
|---|---|---|
| `Dataplane` (`register_local_backend`/`deregister_local_backend`) | `EbpfDataplane` (host), `SimDataplane` (sim) | Bodies write `REVERSE_LOCAL_MAP` reverse-first (host) / the reply mirror under one lock (sim). NO new trait method. |
| `Dataplane` (boot/probe path) | `EbpfDataplane::probe` | Attaches sendmsg4 + recvmsg4 + round-trips `REVERSE_LOCAL_MAP` sentinel; attach IS the below-floor preflight. |
| (kernel-driven) `cgroup/sendmsg4`, `cgroup/recvmsg4` | the two new BPF programs | New driven adapters; each builds its key via the shared `build_local_service_key` helper, then does its own map lookup + its own rewrite direction. |

**External integrations:** none. No third-party API, webhook, or OAuth
provider — the only "external" surface is the Linux kernel BPF ABI, which is
not a consumer-driven-contract target. **No contract-test annotation.**

---

## Wave: DESIGN / [REF] Technology choices

| Choice | Value | Rationale | License |
|---|---|---|---|
| BPF userspace loader | `aya` (existing) | Already the workspace BPF loader; no new dep. | MIT/Apache-2.0 (OSS) |
| Reverse map type | `BPF_MAP_TYPE_HASH` | Point-access reply lookup; mirrors `LOCAL_BACKEND_MAP`; no HoM (no atomic-swap-of-set requirement). | kernel |
| Miss counter | `BPF_MAP_TYPE_PERCPU_ARRAY` | The shipped `DROP_COUNTER` precedent; per-CPU avoids contention. | kernel |
| Kernel floor | 4.18 (sendmsg4) / 4.20 (recvmsg4) | Both below the 5.10 LTS floor — **no matrix bump**. | — |
| Reverse key type | `BackendKey {ip,port,proto}` (in-repo) | REUSE; byte-parity with three existing keys; free Sim mirror. | — |
| ~~Sentinel value~~ | ~~`192.0.2.1` (RFC 5737 TEST-NET-1)~~ | **REMOVED (UI-1, ADR-0053 D3 sub-revision 2026-06-05b).** No sentinel is written on the miss path — recvmsg4 is a pure no-op on a miss. `SENTINEL_SOURCE_HOST` is dead code, deleted when S-03-01 lands. | — |

**Enforcement (architecture rules).** `dst-lint` (the existing crate-class
gate) keeps `overdrive-bpf`/`overdrive-host` out of `core` compile paths and
flags `std::fs` in async host bodies; the `BackendKey`/`Proto` newtype
discipline is enforced by the existing proptest roundtrips. No new
language-level architecture-test tool is warranted — the structural
boundaries (sim vs host vs core) are already `dst-lint`-enforced.

---

## Wave: DESIGN / [REF] Decisions table

| ID | Decision | SSOT |
|---|---|---|
| DDD-1 | Second map `REVERSE_LOCAL_MAP`, ordered (reverse-first) dual-write | ADR-0053 rev D1 |
| DDD-2 | Reverse key = `BackendKey (ip,port,proto)` | ADR-0053 rev D2 |
| DDD-3 | Reverse-miss = **no-op** (real source intact + counted miss); HIT = source→VIP; recvmsg4 `[1,1]` cannot-deny (corrected 2026-06-05b, UI-1 — was sentinel-on-miss) | ADR-0053 rev D3 sub-revision 2026-06-05b |
| DDD-3a | AC reframing wire → app-sockaddr | upstream-changes.md |
| DDD-4 | Option 3 shared `build_local_service_key` helper (key-build + NBO only; per-hook lookup + rewrite); refactors connect4 | ADR-0053 rev D4 |
| DDD-5a | dual-write in `register_local_backend`; no new method; contract amended | ADR-0053 rev D5a |
| DDD-5b/c | probe attaches both hooks; attach = below-floor preflight; `#[from]` errors | ADR-0053 rev D5b/c |
| DDD-5d | Sim reply mirror; test accessor; no production shaping | ADR-0053 rev D5d |
| DDD-5e | NBO idiom verbatim in shared helper; recvmsg4 fields confirmed | ADR-0053 rev D5e |

---

## Wave: DESIGN / [REF] Reuse Analysis (HARD GATE)

| Component | Disposition | Justification |
|---|---|---|
| `REVERSE_LOCAL_MAP` kernel map | **CREATE NEW** | No existing reply store for the same-host cgroup path; the XDP `REVERSE_NAT` is a different hook/class (remote/connected). |
| `ReverseLocalMapHandle` | **CREATE NEW** | Typed userspace handle for the new map; mirrors `LocalBackendMapHandle` shape. |
| `cgroup_sendmsg4_service` program | **CREATE NEW** | No unconnected-UDP forward hook exists today (Amendment 4 scoped it out). |
| `cgroup_recvmsg4_service` program | **CREATE NEW** | No reply-source-rewrite hook exists today. |
| `build_local_service_key` shared helper | **CREATE NEW** | Factored from connect4's inline key-build + NBO (Option 3); the single key-construction site. Does NOT perform a lookup or a rewrite — those stay per-hook (connect4/sendmsg4 → `LOCAL_BACKEND_MAP` forward dest-rewrite; recvmsg4 → `REVERSE_LOCAL_MAP` reverse source-rewrite). |
| `REVERSE_LOCAL_MISS_COUNTER` | **CREATE NEW** | New reply-path reason; NOT a `DropClass` variant (recvmsg4 does not drop). |
| `cgroup_connect4_service` | **EXTEND** | Lookup body refactored to call the shared helper (was UNCHANGED under DISCUSS DD6; now EXTEND per D4). |
| `register_local_backend` / `deregister_local_backend` (trait + both adapters) | **EXTEND** | Bodies gain the reverse map write / reply mirror; contract amended. NO new method. |
| `EbpfDataplane` probe/boot | **EXTEND** | Attach both new hooks + sentinel round-trip. |
| `DataplaneError`/`DataplaneBootError` | **EXTEND** | New `#[from]`-routed variant(s). |
| `BackendKey` | **REUSE** | The reverse key (D2). |
| `Proto` | **REUSE** | `backend_key::Proto` (IANA byte). |
| `LOCAL_BACKEND_MAP` (+ handle) | **REUSE** | sendmsg4 + connect4 forward lookup; unchanged shape. |
| `Action::RegisterLocalBackend` / hydrator classifier | **REUSE** | The reverse write is adapter-internal; no action field, no classifier change. |
| `cgroup_attach_path` config | **REUSE** | All three hooks attach to the same configured slice. |

**Net CREATE NEW = 6 components** (the new map, its handle, two programs,
the shared helper, the miss counter). Everything else EXTEND or REUSE.
**connect4 moved from UNCHANGED (DISCUSS DD6) to EXTEND** due to the D4
shared-helper refactor — this is the one item the framing pass flipped.

---

## Wave: DESIGN / [REF] Open questions (to DELIVER / Tier-3)

1. **`REVERSE_LOCAL_MISS_COUNTER` operational semantics (metric-semantics
   decision for DEVOPS / acceptance-designer, NOT a tracking issue).** Because
   recvmsg4 fires on ALL subtree unconnected UDP (cgroup-ancestor attach), the
   counter increments on every non-service recv (DNS clients, backend
   inbound-query recvs, unrelated same-host UDP) — its absolute value is
   dominated by non-service traffic and is NOT a "service reply failed to
   translate" alarm. It cannot isolate the should-never-happen evicted-reply
   case from routine non-service misses. Whether to keep, demote, or replace
   it (e.g. a control-plane reconciler comparing forward-vs-reverse map
   cardinality, or a `bpftool map dump` differential) is a metric-semantics
   decision. The **no-op-on-miss behavior is correct regardless** of how the
   counter is treated. Per research addendum "Residual Tier-3 open question";
   surfaced per `feedback_no_unilateral_gh_issues` (no `gh issue create`).
2. **Research Gap 1 (non-blocking citation).** The exact verifier
   `check_return_code` file:line for the recvmsg4 `[1,1]` range and the v5.10
   `udp_recvmsg` `RECVMSG_LOCK` call site were not pinned (Bootlin/raw fetch
   blocked). The *facts* are established by the selftest error string and the
   commit hunk; only the line citation is missing. Optional for the crafter to
   pin in a local 5.10 checkout.
3. **~~Sentinel resolver-rejection~~ — MOOT (UI-1, 2026-06-05b).** No sentinel
   is written on the miss path, so no resolver ever observes a sentinel-sourced
   reply. (Was research Gap 2; resolved by the no-op-on-miss correction.)

---

## Wave: DESIGN / [REF] Changed Assumptions (back-propagation)

Two DISCUSS-wave assumptions are corrected by DESIGN. Full verbatim
quotes + new wording + rationale in `design/upstream-changes.md`; summary:

| # | Prior-wave assumption (verbatim, abbreviated) | New | Rationale |
|---|---|---|---|
| CA-1 | DD6 / K4: *"Pure addition: connect4 … UNCHANGED"* / *"0 changes to connect4 / forward-map shape / hydrator classifier (pure addition; diff is additive only)"* | connect4 is **EXTEND** — its key-build + NBO is refactored to call the shared `build_local_service_key` helper (D4); its own `LOCAL_BACKEND_MAP` lookup + forward dest-rewrite stay in its body. Net-new connect4 *behavior* = 0; *diff* is non-zero, Tier-3-reverified. | The user overrode Morgan's Option-2 to Option-3 (shared helper). A shared key-build helper across three hooks necessarily refactors the shipped third hook (connect4). |
| CA-2 | US-01/US-03 + K2/K5 wire-layer ACs: *"tcpdump shows the reply source = the VIP"* / *"no backend-IP-sourced reply leaves the host"* | Application-sockaddr layer: *"the source the client app reads via recvfrom/msg_name is the VIP"* (K2) / *"on a reverse miss the app reads a non-backend sentinel, never the backend IP, and the miss is counted"* (K5). **The "sentinel on miss" half of this new wording is itself superseded by CA-3 below** — see CA-3 for the corrected K5. | recvmsg4 fires after the skb is on the socket queue; a `tcpdump -i lo` shows the backend source regardless (research Q4). Wire no-leak is XDP's domain; recvmsg4 cannot deny (research Q1). Layer/wording correction, intent preserved. |
| **CA-3** (DESIGN self-correction, back-prop from DELIVER UI-1; supersedes the CA-2/DDD-3 "sentinel on miss" wording) | DDD-3 / D3 (2026-06-05): *"on a `REVERSE_LOCAL_MAP` miss, recvmsg4 rewrites the reply source to a non-backend sentinel `192.0.2.1` + counted miss; strictly stronger than Cilium's pass-through-leak"* (and the CA-2 K5 "the app reads a non-backend sentinel" clause). | **No-op-on-miss:** recvmsg4 rewrites source→VIP on a **HIT**; on a **MISS** it leaves the real source intact and bumps the counter only. K5's no-leak guarantee holds via the **D1 reverse-first dual-write (always-hit)**, not a sentinel. Cilium-aligned, not Cilium-exceeding. | recvmsg4 attaches at a cgroup *ancestor* and fires on EVERY unconnected-UDP recv from any descendant, so a reverse-map miss = "not a service reply at all" (a backend's own inbound-query `recvfrom`; any unrelated UDP), NOT "a service reply with a lost reverse entry." Sentinel-ing every miss corrupts the source every non-service datagram's app reads — Tier-3-observed and fixed in DELIVER step 01-03 (commit `e71ad780`). The prior research Q5 "strictly stronger than Cilium" was a category error (it assumed a miss = lost service reply). Per `docs/research/dataplane/recvmsg4-reply-source-rewrite-and-miss-semantics-research.md` § "Addendum — UI-1 adjudication (2026-06-05)" (verdict: crafter CORRECT, Q5 WRONG); ADR-0053 D3 sub-revision 2026-06-05b; `deliver/upstream-issues.md` § UI-1. |

Non-blocking findings, all actioned in this revision: nitpick #1 (the
"≤1 day target" header overstated Slice 01's ~1–1.5d estimate — softened
above); nitpick #2 (the stale "peer review SKIPPED" line — corrected above);
suggestion #3 for DESIGN (pin the reverse-key *composition* — `backend_ip`
vs `(backend_ip, backend_port)` — explicitly in the ADR-0053 amendment;
folded into the DESIGN handoff above). No revision to user stories, ACs,
KPIs, slices, journeys, or persona was required.

## Wave: DESIGN / [REF] Peer review

**Reviewer:** Atlas (nw-solution-architect-reviewer, run on inherited Opus
per `rigor.reviewer_model = inherit`) · **Date:** 2026-06-05 ·
**Verdict: APPROVED** (pre-handoff architecture gate cleared; 0 blocking,
0 critical, 0 high).

All 9 validation axes PASS. Praise: (1) the D3 `[1,1]` verifier
cannot-deny claim is VERIFIED-PRIMARY (kernel selftest verbatim diagnostic
+ commit `983695fa6765` + Cilium `SYS_PROCEED`), with the inferred
mechanism honestly separated from the verified range — the design rests
only on the verified half; (2) the D4 connect4-refactor honesty (Tier-3-only
regression surface named, same-PR mitigation pinned, K4/DD6 back-propagated
verbatim).

Four `low`, non-blocking findings — all DELIVER-actionable, none gate the
DISTILL handoff:

| # | Finding | Action site |
|---|---|---|
| F-1 (most important for the crafter) | Helper named `local_backend_lookup` + "consumed by all three hooks" oversells the shared surface: recvmsg4 shares the **key-build + NBO primitive** but looks up a *different* map (`REVERSE_LOCAL_MAP`), and does a **reverse source-rewrite**, not the forward dest-rewrite connect4/sendmsg4 do. ADR substance is correct (rewrites kept per-hook); the *naming* invites an implementer to write one function doing both. | **ACTIONED in-revision (2026-06-05).** Helper renamed `build_local_service_key` (key-build + NBO ONLY — no lookup, no rewrite) across ADR D4/D5e, brief, c4-diagrams, and this section; per-hook map lookup (`LOCAL_BACKEND_MAP` forward vs `REVERSE_LOCAL_MAP` reverse) and per-hook rewrite direction (forward dest vs reverse source) now stated explicitly. DELIVER: implement accordingly — one helper MUST NOT serve both rewrite directions. |
| F-2 | "atomically reverse-first" prose overstates — the guarantee is an **ordering** guarantee (two BPF map syscalls), not atomicity. The trait contract (5a) already states it correctly as a one-directional implication ("any visible forward entry implies a visible reverse entry"). | **ACTIONED in-revision (2026-06-05).** "atomically/atomic reverse-first" replaced with "ordered (reverse-first)" in ADR D1/D3/D5d, brief, and this section; the trait-contract (5a) one-directional-implication wording left as-is (already correct). |
| F-3 | The two-VIPs→one-identical-backend-socket collision (reverse slot last-writer-wins) is implied by "single VIP per backend key" but not named as the operator-misconfig it is. | **ACTIONED in-revision (2026-06-05).** One sentence added to ADR D2 naming two-VIPs→one-identical-backend-socket as last-writer-wins operator misconfiguration / unsupported topology, not a silent assumption; key design unchanged. |
| F-4 | Sentinel (`192.0.2.1`, RFC 5737 TEST-NET-1) resolver-rejection — correctly deferred to Tier-3 as an open question, surfaced not assumed; no tracking issue (per `feedback_no_unilateral_gh_issues`). | DELIVER Tier-3 empirical check; swap sentinel with no design change if needed. |

F-1, F-2, and F-3 are prose/naming accuracy fixes — per CLAUDE.md they
route through the architect, not inline crafter edits; all three are now
**actioned in-revision** (architect pass 2026-06-05) across ADR-0053 rev,
brief, and c4-diagrams, with F-1's implementation guidance carried into
DELIVER. F-4 stays a DELIVER Tier-3 open question. No revision to the
locked decisions (D1–D5) was required — these are wording/naming
corrections to the frozen SSOT, not decision changes.

---

## Wave: DISTILL / [REF] Overview

**Acceptance designer:** Quinn. **Date:** 2026-06-05. **Density:** lean +
ask-intelligent. **Lang:** Rust (project marker `Cargo.toml`; `[lang-mode]
rust`). **Reconciliation:** PASSED — the one known DISCUSS↔DESIGN
divergence (wire-layer US-01/US-03/K2/K5 → app-sockaddr) is **resolved** by
DESIGN DDD-3a + `design/upstream-changes.md` CA-2 (back-prop), and DD6/K4 "0
connect4 changes" → connect4 EXTEND is resolved by CA-1/DDD-4. Both are
documented supersessions, not live contradictions. Scenarios are specced
against the **reframed app-sockaddr ACs** throughout — zero `tcpdump`/wire
assertions for the recvmsg4 reply path.

**SSOT scaffold spec:** `distill/test-scenarios.md` (GIVEN/WHEN/THEN, never
executed — no `.feature` files per `.claude/rules/testing.md`). RED
classification: `distill/red-classification.md`.

**DEVOPS delta:** ABSENT (no `docs/feature/unconnected-udp-sendmsg4/devops/`).
WARN, default env applied: Lima VM, kernel ≥ 5.10 (both new hooks' floors
4.18/4.20 sit below it — no matrix bump). No DEVOPS contradiction possible
(nothing to contradict).

**KPI contracts:** `docs/product/kpi-contracts.yaml` is the **docs-platform**
feature's DEVOPS instrumentation contract (KPI-1..6 for the website) — it does
NOT carry this feature's K1–K5. The UDP K1–K5 live in this file's § DISCUSS
Outcome KPIs and are linked to scenarios via `@kpi-KN` tags below. Injecting
UDP KPIs into the docs-platform contract would corrupt an unrelated SSOT;
**not done** (soft-gate warning honored, not a blocker).

---

## Wave: DISTILL / [REF] Scenario list with tags

9 scenarios across 3 slices. Walking skeleton = **S-01-01** (flagged).

| ID | Slice | Story | Tier | Tags | Class |
|---|---|---|---|---|---|
| **S-01-01** | 01 | US-01 | T3 | `@walking_skeleton @US-01 @kpi-K1 @kpi-K2 @tier3 @real-io @driving_adapter @property` | happy (WS) |
| S-01-02 | 01 | US-01 | T3 | `@US-01 @kpi-K2 @tier3 @real-io` | happy |
| S-01-03 | 01 | US-01 | T3 | `@US-01 @tier3 @real-io @error` | edge |
| S-02-01 | 02 | US-02 | T1-DST | `@US-02 @kpi-K3 @tier1-dst @in-memory @property` | happy |
| S-02-02 | 02 | US-02 | T1-DST | `@US-02 @kpi-K3 @tier1-dst @in-memory @error` | error |
| S-02-03 | 02 | US-02 | T3 | `@US-02 @kpi-K3 @tier3 @real-io` | happy |
| S-03-01 | 03 | US-03 | T3 | `@US-03 @kpi-K5 @tier3 @real-io @error` | error |
| S-03-02 | 03 | US-03 | T3 | `@US-03 @kpi-K5 @tier3 @real-io @error` | error |
| S-03-03 | 03 | US-03 | T3 | `@US-03 @tier3 @real-io @error` | error |

Error/edge ratio: **5/9 = 56%** (≥ 40% ✓). `@property` (S-01-01, S-02-01) is
example-pinned per Mandate 9 (S-01-01 at Tier 3 → one canonical round-trip;
S-02-01 is the DST `evaluate_*` install-walk-assert shape, not Hypothesis
`@given`). Full GIVEN/WHEN/THEN in `distill/test-scenarios.md`.

---

## Wave: DISTILL / [REF] WS strategy

Per the Architecture of Reference (port-class → treatment, decided once per
project; the retired A/B/C/D choice): **driving** port = `overdrive deploy`
(real CLI) + the unconnected `sendto`/`recvfrom` round-trip; **driven-internal**
= the real BPF maps (`LOCAL_BACKEND_MAP`, `REVERSE_LOCAL_MAP`) via the real
kernel; **driven non-deterministic** = none (the only "external" is the Linux
kernel BPF ABI — not a CDC target). The Sim reply mirror is the **in-memory
double** for the Tier-1 equivalence pin (the `InMemoryComposition`-equivalent
honoring the same `Dataplane` interface). No project Infrastructure Policy
file exists or is bootstrapped — this is a kernel-dataplane feature whose
treatment is fixed by the crate-class split (`overdrive-sim` vs
`overdrive-host` vs real kernel), already `dst-lint`-enforced; a generic
`atdd-infrastructure-policy.md` would be redundant noise here (no DB /
Testcontainers / HTTP fake mechanism to record).

The WS (S-01-01) closes the full backbone thinly: `overdrive deploy` →
register dual-write → sendmsg4 forward → recvmsg4 reply → VIP-sourced
`recvfrom`. Demo-able to Ana: `dig @<vip>`-shape answer whose source the app
reads is the VIP.

---

## Wave: DISTILL / [REF] Adapter coverage table

Every NEW driven adapter maps to a `@real-io @tier3` scenario; the reply-path
identity additionally carries the `@tier1-dst` equivalence invariant (the
no-Tier-2-backstop structural defense).

| Driven adapter / surface | @real-io (Tier 3) | @tier1-dst | Covered by |
|---|---|---|---|
| `cgroup/sendmsg4` program | YES | (via mirror) | S-01-01, S-01-03 |
| `cgroup/recvmsg4` program | YES | (via mirror) | S-01-01, S-02-03, S-03-01 |
| `REVERSE_LOCAL_MAP` (map + handle) | YES | YES (reply mirror) | S-01-02, S-02-01, S-02-03, S-03-01 |
| `register_local_backend` dual-write (host) | YES | — | S-01-02, S-02-03 |
| `register_local_backend` reply mirror (sim) | — | YES | S-02-01, S-02-02 |
| `REVERSE_LOCAL_MISS_COUNTER` | YES | — | S-03-01 |
| `EbpfDataplane::probe` (attach both; below-floor preflight) | YES | — | S-03-02 |
| `build_local_service_key` shared helper | YES (exercised via all 3 hooks) | — | S-01-01 (+ connect4 re-run) |
| `cgroup/connect4` (EXTEND — helper refactor) | YES (shipped acceptance re-run) | — | `local_backend_proto_connect.rs` (D4 risk mitigation) |
| Driving adapter `overdrive deploy` + unconnected round-trip | YES | — | S-01-01 |

**No-Tier-2-backstop note:** `BPF_PROG_TEST_RUN` → ENOTSUPP for
`cgroup_sock_addr` ≤ 6.8. The two new programs get **NO Tier-2 triptych**
(deliberately not scaffolded). Tier-3 is THE gate; Tier-1
`reply-source-rewrite-lockstep` is the structural defense below it.

---

## Wave: DISTILL / [REF] Scaffolds

Test-side (RED-ready) + production-side (so imports resolve, Mandate 7).
Markers: `__SCAFFOLD__` / `#[should_panic(expected = "RED scaffold")]` /
`todo!("RED scaffold: …")`. Discover via
`grep -rn '__SCAFFOLD__\|RED scaffold' crates/`.

**Tier-1 DST invariant (default lane — load-bearing J-PLAT-004 piece):**
- `crates/overdrive-sim/src/invariants/reply_source_rewrite_lockstep.rs` —
  `evaluate_reply_source_rewrite_lockstep` (real body; RED-fails via
  `InvariantResult::Fail` until the Sim mirror write lands). Wired into
  `Invariant::ReplySourceRewriteLockstep` (`invariants/mod.rs` variant +
  `as_canonical` + `ALL`) and the harness dispatch arm (`harness.rs`).

**Tier-3 acceptance (integration-tests feature, Lima-only, `#[should_panic]`):**
- `crates/overdrive-dataplane/tests/integration/unconnected_udp_roundtrip.rs`
  — S-01-01 (WS) / S-01-02 / S-01-03 / S-02-03.
- `crates/overdrive-dataplane/tests/integration/unconnected_udp_reply_hardening.rs`
  — S-03-01 / S-03-02 / S-03-03.
- Wired into `tests/integration.rs` inline `mod integration { … }` block.

**Production-side RED scaffolds:**
- `crates/overdrive-bpf/src/maps/reverse_local_map.rs` (`__SCAFFOLD__`, `#[map]`
  absent), `…/reverse_local_miss_counter.rs` (same).
- `crates/overdrive-bpf/src/shared/build_local_service_key.rs` (`todo!` +
  `#[expect(clippy::todo)]`).
- `crates/overdrive-bpf/src/programs/cgroup_sendmsg4_service.rs`,
  `…/cgroup_recvmsg4_service.rs` (`#[cgroup_sock_addr(...)]` attribute absent —
  the kernel-side RED signal; returns the non-denying verdict 1).
- `crates/overdrive-dataplane/src/maps/reverse_local_map_handle.rs` (`todo!` +
  `#[expect(clippy::todo)]`).
- `crates/overdrive-sim/src/adapters/dataplane.rs` — Sim reply mirror field +
  `reply_source_for()` + `reply_mirror_entries()` (REAL accessors); the mirror
  WRITE in `register_local_backend` is the GREEN target (commented scaffold,
  not `todo!` — so existing forward-path tests stay green; RED is carried by
  the Tier-1 invariant).

mod-wiring touched: `overdrive-bpf` `maps/mod.rs`, `programs/mod.rs`,
`shared/mod.rs`; `overdrive-dataplane` `maps/mod.rs`,
`tests/integration.rs`; `overdrive-sim` `invariants/mod.rs`, `harness.rs`.

---

## Wave: DISTILL / [REF] Test placement

Precedent-justified:
- Tier-1 invariant → `crates/overdrive-sim/src/invariants/` (the
  `reverse_nat_lockstep.rs` template — same install-walk-assert + `Invariant`
  enum + harness-dispatch wiring).
- Tier-3 acceptance → `crates/overdrive-dataplane/tests/integration/<scenario>.rs`
  behind the `integration-tests` feature, declared inside the inline `mod
  integration { … }` block in `tests/integration.rs` (the
  `local_backend_proto_connect.rs` precedent — same cgroup-attach-at-
  `/sys/fs/cgroup`, `cargo xtask lima run --` root harness, veth + per-test
  bpffs pin-dir, fixture binding off systemd-resolved ports).
- Production scaffolds → alongside their shipped siblings
  (`maps/`, `programs/`, `shared/` in `overdrive-bpf`; `maps/` in
  `overdrive-dataplane`; `adapters/dataplane.rs` in `overdrive-sim`).

---

## Wave: DISTILL / [REF] Driving-adapter coverage

The operator driving surface is unchanged `overdrive deploy <SPEC>` (DESIGN §
Ports: no new driving port). S-01-01 exercises it end-to-end via the real CLI
deploy → the unconnected `sendto`/`recvfrom` round-trip (the user's actual
invocation path), tagged `@driving_adapter @walking_skeleton`. The two new
cgroup hooks are **kernel-driven** (the kernel invokes them at
`sendmsg`/`recvmsg` syscall time) — driven adapters, NOT a driving port; no
new CLI verb / HTTP route to cover.

---

## Wave: DISTILL / [REF] Pre-requisites

- **DESIGN driving ports / contract:** ADR-0053 rev 2026-06-05 (D1–D5e);
  `brief.md` § "Unconnected-UDP sendmsg4 extension"; `design/upstream-changes.md`
  (the app-sockaddr reframe + connect4-EXTEND back-prop).
- **DEVOPS env (default):** Lima VM, kernel ≥ 5.10 (sendmsg4 ≥ 4.18, recvmsg4
  ≥ 4.20 both below floor — no matrix bump). Tier-3 runs Lima-only
  (`cargo xtask lima run -- cargo nextest run --features integration-tests`).
- **No Tier-2 backstop** for `cgroup_sock_addr` (ENOTSUPP ≤ 6.8) — Tier-3 is
  THE correctness gate; Tier-1 `reply-source-rewrite-lockstep` is the defense
  below it.
- **Shipped deps (REUSE):** `BackendKey`, `Proto`, `LOCAL_BACKEND_MAP` (+
  handle), `register_local_backend` action (proto-carrying), hydrator
  classifier, `cgroup_attach_path`.
- **DELIVER open question (Tier-3, NOT a tracking issue per
  `feedback_no_unilateral_gh_issues`):** confirm `dig`/glibc/musl cleanly
  reject a `192.0.2.1`-sourced reply; swap the sentinel (no design change) if
  not (DESIGN open-Q 1 / F-4).

---

## Wave: DISTILL / [REF] Mandate compliance evidence

- **CM-A (hexagonal boundary):** acceptance tests enter through the driving
  path (`overdrive deploy` + unconnected round-trip) and the `Dataplane`
  driving-port methods (`register_local_backend`); the Tier-1 invariant drives
  the `SimDataplane` adapter through the trait surface. No internal-component
  instantiation.
- **CM-B (business language):** scenario titles + GIVEN/WHEN/THEN use Ana's
  domain (deploy a UDP service, unconnected query, reply source the app reads);
  technical detail (NBO, `bpf_sock_addr`, map names) lives in Notes, not titles.
- **CM-C (user journeys):** every scenario is a complete journey with operator
  value (reachability, VIP-sourced reply, diagnosable failure) — not isolated
  technical ops.
- **CM-E (Mandate 8 universe at layers 1-3):** the Tier-1 invariant asserts on
  port-exposed observables (`reply_source_for`, `reply_mirror_entries`,
  `local_backends`) — never internal struct fields. Tier-3 (layer 4+) uses
  traditional `recvfrom`/`bpftool`-equivalent assertions per Mandate 8 (layers
  4+ may).
- **CM-F (Mandate 9 layer-dependent PBT):** PBT-shape (`@property`) at Tier 1-2
  is the DST `evaluate_*` install-walk; layer 3+ scenarios are example-only
  (`#[should_panic]` single-example), no PBT machinery imported.
- **CM-G (Mandate 10 Tier B):** NOT added. The journey is ≥3 chained scenarios
  but the input space is config-shaped (one VIP, one backend, fixed
  proto=UDP) — the reply-source identity is a single invariant, not a
  domain-rich generative space. Tier A (the production-composition-root Tier-3
  round-trip) + the Tier-1 DST invariant cover it; a `RuleBasedStateMachine`
  would add ceremony with no new coverage (Mandate 10 "when NOT worth it":
  config-shaped, the observable is identity not a rich mutation space).
- **CM-H (Mandate 11 layer-3+ sad paths example-based):** S-01-03, S-02-02,
  S-03-01/02/03 are named example-based tests; no PBT at Tier 3.

---

## Wave: DISTILL / [REF] Handoff to DELIVER

- **Acceptance suite:** 9 scenarios (`distill/test-scenarios.md`) + 1 Tier-1
  DST invariant + 7 Tier-3 `#[should_panic]` scaffolds + production-side RED
  scaffolds for every imported NEW module.
- **Walking skeleton:** S-01-01 (flagged) — the unconnected round-trip,
  reply VIP-sourced at the app sockaddr layer.
- **One-at-a-time sequence:** Slice 01 (WS round-trip + dual-write + both maps
  + helper extraction → flips S-01-01/02/03 + the Tier-1 invariant GREEN) →
  Slice 02 (the Tier-1 invariant is already wired; Slice 02 confirms the
  mutation kill + S-02-03 Tier-3 meet) → Slice 03 (sentinel-miss + below-floor
  + fixture → S-03-01/02/03). Strict 01 → 02 → 03 dependency.
- **Mandate evidence:** CM-A/B/C/E/F/G/H above.
- **Peer review:** the mandatory consolidated 4-reviewer Final Wave Review
  Gate (Eclipse + Atlas + Forge + Sentinel against the full feature-delta) is
  run by the ORCHESTRATOR after this DISTILL artifact returns — NOT
  self-invoked here (subagent cannot dispatch parallel reviewers).
