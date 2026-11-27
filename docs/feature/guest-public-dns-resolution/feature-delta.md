# Feature-Delta — guest-public-dns-resolution (DISCUSS)

> GH #382. Lean density, Tier-1 `[REF]` sections only. Produced by Luna
> (nw-product-owner) on 2026-11-25. No DIVERGE artifacts exist (non-blocking
> risk; the direction was decided by the user, see Locked decisions). Slice
> briefs live under `slices/`. SSOT touched: `docs/product/jobs.yaml`
> (J-OPS-005 minted), `docs/product/journeys/run-a-vm-workload.yaml` (step 7),
> `docs/product/personas/ana-platform-engineer.yaml` (trigger + related job).

## Wave: DISCUSS / [REF] Reading checklist

- ✓ `.context/issue-382-full.md` — issue body + the metal-confirmation comment
- ✓ `.context/guest-public-dns-probe-findings.md` — probe findings (commit 46e80ccee, metal run b20bcb2fb8028a614e91755c, Linux 7.0.0-38 at fe0c3adcf)
- ✓ `docs/product/vision.md`, `docs/product/jobs.yaml` (lines 1-894 + changelog tail; the middle changelog was skimmed, not needed)
- ✓ personas: `ana-platform-engineer.yaml`, `maya-agent-developer.yaml`; ⊘ the other four personas (not relevant to this actor)
- ✓ journeys: `dial-a-mesh-peer-by-name.yaml`, `run-a-vm-workload.yaml`; ⊘ `reach-an-unconnected-udp-service.yaml` (not read; Ana's persona file summarises it and it concerns service VIP reachability, not name resolution)
- ◐ `docs/research/platform/herdr-dev-environment-in-microvm-research.md` — gap 7 row only (grep), not the whole document
- ✓ ADR-0072 (Status block through REV-3; DDN-8 read only through its references), ADR-0116, ADR-0154, ADR-0162 (Context/Decision)
- ✓ code: `dns_responder/responder.rs` (`answer_datagram`, now at line 462 at HEAD 30bdd98b0; the issue cites 461 at fe0c3adcf), `dns_responder/wire.rs` 100-121, `frontend_addr_allocator.rs` (`WORKLOAD_FRONTEND_BASE = 10.98.0.0/16`, line 67)
- ✓ house format: `docs/feature/dial-by-name-responder/feature-delta.md` + slice briefs
- ✓ CLAUDE.md and the `.claude/rules` set supplied in context (design.md, development.md, domain-naming.md, testing.md, verification.md)

## Wave: DISCUSS / [REF] Persona

**Ana Moreno** (`docs/product/personas/ana-platform-engineer.yaml`), operator
of VM-backed Service workloads on a single-node host, through a *workload
outbound reachability* lens. She is also the person building the herdr dev box
(a VM that runs agents, `git`, `cargo`, herdr self-updates). She does not
control the programs in the guest: they call `getaddrinfo` and expect it to work.
Today `overdrive deploy` succeeds, the VM reaches Running, outbound TCP works,
and every public lookup fails after about five seconds.

## Wave: DISCUSS / [REF] JTBD one-liner

**J-OPS-005 (minted, `relates_to: [J-MESH-001, J-OPS-003]`):** *When a program
inside a VM I deployed calls a public service by its name, I want the lookup to
just work through the guest's default resolver configuration, so the workload
does what it would do on any ordinary host without me installing resolvers,
routes or rules in the guest.*

Mint rationale: the progress (a guest reaches a public service by name) and the
failure mode (egress works, the name lookup times out) are distinct from
J-MESH-001 (mesh peer by name) and J-OPS-003 (lifecycle honesty). J-MESH-001's
situation is scoped to `<workload>.svc.overdrive.local`.

## Wave: DISCUSS / [REF] Locked decisions

| ID | Decision | Source |
|---|---|---|
| D-1 | The existing node DNS responder forwards non-mesh queries to the host's upstream resolvers. There is no second resolver. | User, 2026-11-25 |
| D-2 | The guest's resolver configuration is unchanged: one nameserver, the guest gateway (ADR-0154: no guest resolver stub). | User; ADR-0154 |
| D-3 | Forwarding is the non-mesh arm only and is independent of #303 D4. D4 changes only the mesh arm (where mesh-name resolution and load balancing live under the in-guest mTLS model). A future move of naming off the gateway responder must carry the forwarder. | User |
| D-4 | A forwarded answer that lands in an Overdrive-owned address range must not reach the guest as an ordinary answer. The guard is keyed on **one SSOT of owned ranges**. Today that SSOT would name the frontend subnet `10.98.0.0/16` (`WORKLOAD_FRONTEND_BASE`); the workload subnet `10.99.0.0/16` matters if D4 lands on DNS time. Recorded as a requirement; the mechanism and the on-hit result are DESIGN's (superseded: see DESIGN Changed Assumptions — guarded set is 10.98.0.0/16, the VIP pool and 100.95.0.0/16). | User |
| D-5 | Any cache for forwarded answers is separate from the mesh negative TTL (ADR-0072 DDN-8). The mesh answer contract is unchanged. Whether to cache at all is DESIGN's. | User |
| D-6 | Mesh names stay authoritative and local. A mesh name with no healthy backend still returns NXDOMAIN and is never sent to an upstream resolver. | Issue AC 2; ADR-0072 |
| D-7 | Stories describe observable behaviour only. Driving ports are `overdrive serve`, `overdrive deploy <SPEC>` and the guest's own resolver (`getaddrinfo`, UDP/TCP 53 to the gateway). No new API surface is named here. | CLAUDE.md |

## Wave: DISCUSS / [REF] User stories

All stories trace to **J-OPS-005**. Data used throughout: Ana deploys
`herdr-box.toml` (a VM Service); the guest gateway is `100.95.0.1`; public
names are `api.anthropic.com`, `github.com`, `crates.io`, `example.com`; the
mesh name is `payments.svc.overdrive.local`.

### US-GPD-1 — A guest resolves a public name and connects to it (walking skeleton)

**Job:** J-OPS-005

**Elevator Pitch:**
- **Before:** Ana's `herdr-box` VM reaches Running, and `git clone https://github.com/...` inside it fails with "Try again" after about 5 s. `curl` to `1.1.1.1:443` works; the name does not resolve.
- **After:** `overdrive serve` + `overdrive deploy herdr-box.toml`, then inside the guest `getaddrinfo("github.com")` returns addresses through the guest's default resolver configuration and a TCP connect to one of them succeeds, with no resolver, route or rule installed by the test or the operator.
- **Decision enabled:** Ana can decide the dev box is usable for agents, `git` and `cargo` and move on to the remaining blockers (persistent disk, operator tunnel) instead of debugging DNS.

**Problem:** Ana runs a VM workload whose programs reach public services by
name. The guest's only nameserver is the node's gateway, and the node
responder drops every query that is not a mesh name, so the guest's lookup
waits out its 5 s timeout and fails with EAI_AGAIN. Outbound connections by
address already work, so the failure looks like a broken program rather than a
missing resolver path.

**Domain Examples:**
1. **Happy path:** the `herdr-box` guest looks up `api.anthropic.com` and `github.com`; both return addresses promptly and a TCP connect to port 443 succeeds.
2. **Non-existent public name:** the guest looks up `no-such-host-7f3a.example.com`; it gets "Name does not resolve" promptly, not after 5 s.
3. **Mesh regression:** the guest looks up `payments.svc.overdrive.local` while `payments` has a healthy backend and gets the same stable frontend address as before; with `payments` stopped it gets NXDOMAIN, and that query is not seen by any upstream resolver.

**UAT Scenarios (BDD):**

```gherkin
Scenario: A guest resolves a public name through its default resolver and connects
  Given Ana has run "overdrive serve" on a node whose host resolves "github.com"
  And Ana has run "overdrive deploy herdr-box.toml" and the guest is Running
  When a program in the guest calls getaddrinfo for "github.com"
  Then the lookup returns at least one address
  And a TCP connection to that address on port 443 succeeds
  And the guest's resolver configuration is still the single gateway nameserver "100.95.0.1"

Scenario: The herdr dev box's usual public names all resolve
  Given the "herdr-box" guest is Running
  When the guest looks up "api.anthropic.com", "github.com", "crates.io" and "example.com"
  Then each lookup returns an address
  And none of them waits for the guest resolver's 5 second timeout

Scenario: A public name that does not exist fails promptly with a definite answer
  Given the "herdr-box" guest is Running
  When the guest looks up "no-such-host-7f3a.example.com"
  Then the lookup fails with "Name does not resolve"
  And it fails well before the guest resolver's 5 second timeout

Scenario: A mesh name still resolves as before
  Given "payments" is deployed with a healthy backend
  When the guest looks up "payments.svc.overdrive.local"
  Then the answer is the same stable address the node gave before this feature

Scenario: A mesh name with no healthy backend is NXDOMAIN and stays on the node
  Given "payments" is deployed with no healthy backend
  When the guest looks up "payments.svc.overdrive.local"
  Then the lookup fails with "Name does not resolve" promptly
  And no upstream resolver receives a query for that name

Scenario: A malformed name under the mesh namespace is not sent upstream
  Given the "herdr-box" guest is Running
  When the guest looks up "a.b.svc.overdrive.local"
  Then no upstream resolver receives a query for that name
```

**Acceptance Criteria:**
- [ ] AC-1: A guest deployed with `overdrive deploy <SPEC>` on a node started with `overdrive serve` resolves a public name through its default resolver configuration and connects to it, with no test-installed resolver, route or rule. (Issue AC 1)
- [ ] AC-2: Mesh names (`<workload>.svc.overdrive.local`) resolve as today, including NXDOMAIN for a mesh name with no healthy backend, and mesh queries never reach an upstream resolver. (Issue AC 2; D-6)
- [ ] AC-3: A non-mesh name that does not exist returns NXDOMAIN promptly, not a timeout. (Issue AC 3)
- [ ] AC-4: The guest's resolver configuration is unchanged by this feature (one nameserver, the gateway). (D-2)
- [ ] AC-5: No name under the mesh namespace, well-formed or not, is forwarded upstream. (D-6)

**Technical Notes:** drives through the production composition; no
integration test hand-installs a resolver, route or socket that production does
not. A `Sim*` upstream at a port boundary is acceptable; hand-installing a
missing production effect is not. The evidence pin is fe0c3adcf; re-measure at
the implementation SHA before citing the 5005 ms baseline.

### US-GPD-2 — A forwarded answer never points a guest into Overdrive-owned address space

**Job:** J-OPS-005

**Elevator Pitch:**
- **Before:** once the node forwards public lookups, the upstream resolver's answer is untrusted input. If a corporate or split-horizon resolver returns `10.98.0.7` for `build-cache.corp.example`, the guest connects to an address the node reserves for a mesh frontend and its traffic lands on an unrelated workload.
- **After:** `overdrive deploy herdr-box.toml`, then in the guest `getaddrinfo("build-cache.corp.example")` against an upstream that answers `10.98.0.7` does not return that address to the guest; an upstream answer of `10.20.4.7` (not Overdrive-owned) for the same name is returned unchanged.
- **Decision enabled:** Ana can decide it is safe to point the node at her office or cloud resolver without a name being able to alias a mesh frontend.

**Problem:** forwarding opens a new path by which an address the node did not
choose reaches a guest. Overdrive owns address ranges (the mesh frontend
subnet today). A real upstream can legitimately serve private addresses, so a
blanket "no private addresses" rule would break split-horizon DNS, but an
answer inside an owned range would be treated as a mesh frontend by the node's
own dataplane.

**Domain Examples:**
1. **Owned-range hit:** upstream answers `10.98.0.7` for `build-cache.corp.example`; the guest does not receive that address.
2. **Benign private answer:** upstream answers `10.20.4.7` for `git.corp.example`; the guest receives it unchanged.
3. **Mixed answer set:** upstream answers `140.82.121.4` and `10.98.0.9` for `github.com`; the guest does not receive `10.98.0.9` (what it does receive instead is DESIGN's decision, but it never receives the owned address).

**UAT Scenarios (BDD):**

```gherkin
Scenario: An upstream answer inside the mesh frontend subnet is not handed to the guest
  Given the node's upstream resolver answers 10.98.0.7 for "build-cache.corp.example"
  When the "herdr-box" guest looks up "build-cache.corp.example"
  Then the guest does not receive 10.98.0.7

Scenario: A private answer outside Overdrive-owned ranges is delivered unchanged
  Given the node's upstream resolver answers 10.20.4.7 for "git.corp.example"
  When the "herdr-box" guest looks up "git.corp.example"
  Then the guest receives 10.20.4.7

Scenario: One owned-range address in a mixed answer set is not delivered
  Given the node's upstream resolver answers 140.82.121.4 and 10.98.0.9 for "github.com"
  When the "herdr-box" guest looks up "github.com"
  Then the guest does not receive 10.98.0.9
```

**Acceptance Criteria:**
- [ ] AC-6: No forwarded answer containing an address in an Overdrive-owned range reaches the guest as an address. (D-4)
- [ ] AC-7: A forwarded answer outside Overdrive-owned ranges, including other private ranges, is delivered unchanged. (narrowness; D-4)
- [ ] AC-8: The set of owned ranges has one source of truth shared with the code that allocates those ranges; adding a range there extends the guard with no second edit. (D-4)
- [ ] AC-9: The guest never receives silence for a guarded answer; it gets a definite result: the whole reply's sections are emptied, the upstream rcode is kept, and EDE Blocked is added when the query carried EDNS (ADR-0359).

**Technical Notes:** grounded in production: forwarding to the host's
resolver (dhcpcd-generated on the metal box) passes whatever that resolver
returns. DESIGN ruling (user, 2026-11-25): the guarded ranges are the mesh
frontend range `10.98.0.0/16`, the configured service VIP pool, and the guest
prefix `100.95.0.0/16`; `10.99.0.0/16` is not guarded because no production
path steers a guest connection to it as mesh traffic (DESIGN [REF] Grounding
findings, 7). Adding a range is one line in the owned list.

### US-GPD-3 — A guest gets a definite, prompt failure when the node cannot reach any upstream resolver

**Job:** J-OPS-005

**Elevator Pitch:**
- **Before:** when the host's resolver is unreachable, a forwarding responder that waits silently reproduces today's symptom: every guest lookup hangs for the full 5 s and fails with a generic "Try again".
- **After:** `overdrive deploy herdr-box.toml`, then in the guest `getaddrinfo("github.com")` with the host's upstream resolvers unreachable fails with a definite error before the guest resolver's 5 s timeout; mesh lookups still answer during the outage; the next lookup after the upstream returns succeeds with no restart of the node or the guest.
- **Decision enabled:** Ana can tell "the node's upstream DNS is down" from "my guest program is broken", and the guest's own retry logic gets a fast, definite signal.

**Problem:** a forwarder adds a dependency the node did not have. If the
upstream is slow or down, the experience must not degrade to the same
5 s silent timeout that motivated this feature, and it must not take mesh
naming down with it. Under saturation (every in-flight forward slot held) a UDP query gets an immediate SERVFAIL and a TCP connection is closed unread; this behaviour is DESIGN-owned (ADR-0362).

**Domain Examples:**
1. **Upstream down:** Ana's host has lost its network; the guest's lookup of `crates.io` fails in under 5 s with a definite error.
2. **Mesh during the outage:** in the same window `payments.svc.overdrive.local` still resolves.
3. **Recovery:** the host network returns; the guest's next lookup of `crates.io` succeeds, with no `serve` restart.

**UAT Scenarios (BDD):**

```gherkin
Scenario: A lookup fails with a definite answer before the guest's timeout when no upstream is reachable
  Given the node's upstream resolvers are unreachable
  When the "herdr-box" guest looks up "crates.io"
  Then the lookup fails
  And it fails before the guest resolver's 5 second timeout elapses
  And the guest receives a DNS reply rather than silence

Scenario: Mesh names keep resolving while the upstream is down
  Given the node's upstream resolvers are unreachable
  And "payments" has a healthy backend
  When the guest looks up "payments.svc.overdrive.local"
  Then the guest receives the stable frontend address

Scenario: Lookups recover when the upstream returns, with no restart
  Given a lookup of "crates.io" failed because the upstream was unreachable
  When the upstream becomes reachable again
  Then the next lookup of "crates.io" from the guest returns an address
```

**Acceptance Criteria:**
- [ ] AC-10: With no usable upstream (every upstream unreachable, or the host resolver file absent, unreadable or empty), a guest lookup of a public name returns a definite DNS reply before the guest's 5 s resolver timeout. (**Requirement not in the issue; approved by the user 2026-11-25.** The 5 s figure is the measured baseline, 5005 ms.)
- [ ] AC-11: Mesh answers are unaffected by the state of the upstream. (DESIGN qualification: this holds for mesh answers over UDP. Under saturation a TCP connection, mesh or not, is closed at accept because its query class is not yet known; ADR-0362, ADR-0360.)
- [ ] AC-12: Recovery needs no restart of the node, the responder or the guest.

**Technical Notes:** which reply code (SERVFAIL vs other) and the node's own
upstream timeout budget are DESIGN's; the requirement is the guest-observed
promptness and definiteness.

## Wave: DISCUSS / [REF] Story map, slices, prioritization

**Backbone (Ana's journey, left to right):** Declare the VM → Deploy it → Guest
program looks up a name → Guest connects → Operator trusts the result.

| Declare | Deploy | Look up | Connect | Trust |
|---|---|---|---|---|
| `herdr-box.toml` (exists) | `overdrive deploy` (exists) | **public name resolves (US-GPD-1)** | TCP by address (exists, ADR-0162) | mesh unchanged (US-GPD-1) |
| | | owned-range guard (US-GPD-2) | | |
| | | prompt definite failure (US-GPD-3) | | |

| Slice | Stories | Hypothesis | Closes a loop through serve + deploy |
|---|---|---|---|
| 01 public name resolves and connects | US-GPD-1 | The node responder can forward non-mesh queries to the host's resolvers without disturbing mesh answers, so a default-configured guest resolves and connects. | yes: deploy a VM, guest lookup, connect |
| 02 owned-range guard | US-GPD-2 | A guard keyed on one SSOT of owned ranges keeps forwarded answers from aliasing mesh addressing while delivering other private answers unchanged. | yes: deploy a VM, guest lookup against a controlled upstream |
| 03 prompt failure when no upstream | US-GPD-3 | The forwarder fails definitely and promptly and recovers without a restart, so it never reproduces the 5 s silent timeout. | yes: deploy a VM, guest lookup with upstream down |

**Walking skeleton:** slice 01. The feature is brownfield (responder, gateway
resolver and egress path exist), so slice 01 is itself the thin end-to-end loop.

### Priority Rationale

1. Slice 01 first: it is the walking skeleton and the issue's three acceptance criteria. Value 5, urgency 5 (blocks the herdr dev box), effort 3.
2. Slice 02 second: forwarding turns upstream answers into untrusted input to mesh addressing; the feature should not be considered shippable without it. Value 4, urgency 4, effort 2.
3. Slice 03 third: availability and operator clarity; a degraded-but-honest outcome, not a correctness hazard. Value 3, urgency 3, effort 2.

Slices 02 and 03 are in the Definition of Done for this feature; ordering is
for delivery sequencing only.

## Wave: DISCUSS / [REF] Outcome KPIs

Measured at acceptance as pass/fail thresholds from a single recorded run per
case. No percentile or cohort claim is made (that would be a benchmark, per
`testing.md`).

| KPI | Who | Does what | Target | Measured by | Baseline |
|---|---|---|---|---|---|
| K-GPD-1 | A deployed VM guest | Resolves public names through its default resolver | 4 of 4 named public names (`api.anthropic.com`, `github.com`, `crates.io`, `example.com`) resolve and connect on port 443; each lookup completes in under 1000 ms | Guest-placed run through `serve` + `deploy`, recording per-lookup elapsed time | 0 of 2 tested; 5005 ms then EAI_AGAIN (metal run b20bcb2fb8028a614e91755c at fe0c3adcf) |
| K-GPD-2 | A deployed VM guest | Gets NXDOMAIN for a non-existent public name | NXDOMAIN in under 1000 ms | Same run, one name | Silence, then EAI_AGAIN at 5005 ms |
| K-GPD-3 | The node's mesh naming | Behaves identically for mesh names | 0 regressions in the existing mesh-name acceptance set; 0 mesh queries observed at the upstream | Existing dial-by-name acceptance plus an upstream-side observation in the new run | Mesh answers promptly (probe D: 7 ms) |
| K-GPD-4 | The guest | Receives owned-range addresses from forwarded answers | 0 of the owned-range answers delivered; 100% of non-owned private answers delivered | US-GPD-2 scenarios | N/A (no forwarding today) |
| K-GPD-5 | A guest with the upstream down | Gets a definite reply before the guest timeout | reply in under 5000 ms (strictly below baseline) | US-GPD-3 scenario 1 | 5005 ms silent |

North star: K-GPD-1. Guardrail: K-GPD-3.

## Wave: DISCUSS / [REF] Definition of Done

- US-GPD-1, 2 and 3 pass their UAT scenarios through `overdrive serve` + `overdrive deploy`, with no test-installed resolver, route or rule.
- The three issue acceptance criteria (AC-1, AC-2, AC-3) have a guest-level test each.
- The owned-range set has one SSOT (AC-8).
- Operator docs state the new behaviour and every scope limitation with its tracking issue (see [REF] Scope limitations).
- The EDD expectation in [REF] Verification expectations is captured on metal and reviewed by a different agent.

## Wave: DISCUSS / [REF] DoR validation (9 items plus J and E)

| # | Item | Status | Evidence |
|---|---|---|---|
| 1 | Problem statement in domain language | PASS | Each story opens from Ana's observed symptom (5005 ms EAI_AGAIN; untrusted upstream answer; silent outage) |
| 2 | Persona with specific characteristics | PASS | Ana Moreno, VM Service operator building the herdr dev box |
| 3 | 3+ domain examples with real data | PASS | Three per story with real names, addresses (`100.95.0.1`, `10.98.0.7`, `10.20.4.7`) |
| 4 | UAT in Given/When/Then (3-7) | PASS | 6, 3, 3 scenarios |
| 5 | AC derived from UAT | PASS | AC-1..12 map to scenarios; AC-1..3 are the issue's own |
| 6 | Right-sized | PASS | Each story one behaviour, 3-6 scenarios, estimated 1-2 days |
| 7 | Technical notes | PASS | Per-story notes; D-1..D-7; Questions for DESIGN |
| 8 | Dependencies resolved or tracked | PASS | No open dependency blocks DISCUSS. #381 and #348 are explicitly out of scope; #303 D4 is independent by D-3 |
| 9 | Outcome KPIs with targets | PASS | K-GPD-1..5; the 1000 ms and 5000 ms thresholds approved by the user 2026-11-25 |
| J | `job_id` traceability | PASS | J-OPS-005 minted in `jobs.yaml` |
| E | Elevator Pitch on every story | PASS | All three stories carry Before/After/Decision enabled with real entry points |

User rulings 2026-11-25: the numeric thresholds in K-GPD-1/2/5 and AC-10
(US-GPD-3's promptness requirement) are approved.

## Wave: DISCUSS / [REF] Out of scope

- Operator TCP tunnel into the guest (#381) and the persistent volume (#348).
- A guest resolver stub or any change to the guest's `resolv.conf` (ADR-0154).
- Mesh-name resolution and load balancing under the in-guest mTLS model (#303 D4). Only the non-mesh arm is in this feature.
- The guest egress path itself (ADR-0162): already works, unchanged.
- Scope limitations are recorded in [REF] Scope limitations with their rulings.

## Wave: DISCUSS / [REF] Scope limitations

User rulings 2026-11-25.

- **No per-workload resolver override — not a limitation.** A guest that wants a different resolver can already query it by IP over its ordinary egress (ADR-0162). The node-level source of the upstream set is a separate matter: see L-3 below ([#384](https://github.com/overdrive-sh/overdrive/issues/384)).
- **L-2 — No per-guest fairness.** One guest can use the whole in-flight bound and starve other guests' public lookups. Tracked by [#385](https://github.com/overdrive-sh/overdrive/issues/385).
- **L-3 — Upstreams come only from the host resolver file.** A stand-in until upstreams can be set in the worker node configuration. Tracked by [#384](https://github.com/overdrive-sh/overdrive/issues/384) (depends on [#327](https://github.com/overdrive-sh/overdrive/issues/327); Quad9 is noted there).
- **Operator health condition for upstream failure.** Follow-up tracked by [#386](https://github.com/overdrive-sh/overdrive/issues/386).
- **No allowlist or policy on non-mesh lookups.** Every non-mesh name a guest asks for is forwarded. Tracked by [#383](https://github.com/overdrive-sh/overdrive/issues/383); DESIGN Question 9 decides only whether this feature applies any existing control.
- **AAAA for non-mesh names: no limitation.** DESIGN chose pass-through for every non-mesh record type (GPD-6, ADR-0361, approved by the user 2026-11-25), so no IPv4-only stand-in is needed and [#308](https://github.com/overdrive-sh/overdrive/issues/308) is not cited by this feature.

## Wave: DISCUSS / [REF] Verification expectations

Approved by the user 2026-11-25: one EDD expectation, authored in DISTILL and captured on the metal box against the built binary. A VM guest deployed through `overdrive serve` + `overdrive deploy` resolves and connects to the herdr dev box's real lookup targets (`api.anthropic.com`, `github.com`, `crates.io`, and the herdr update host) and gets a prompt NXDOMAIN for a nonexistent public name, through its default resolver with nothing hand-installed. It is a point-in-time receipt, not a test.

## Wave: DISCUSS / [REF] Walking-skeleton strategy

Depends (brownfield). Slice 01 is the thin loop: `overdrive serve`, then
`overdrive deploy` of one VM Service, then a guest lookup of one public name,
then a TCP connect. Slices 02 and 03 each close their own loop through the same
entry points.

## Wave: DISCUSS / [REF] Driving ports

- `overdrive serve` (composition root) and `overdrive deploy <SPEC>`.
- The guest's own resolver: `getaddrinfo`, and UDP and TCP port 53 to the gateway `100.95.0.1`.
- Not driving ports: the responder's internal API, the upstream resolver adapter.

## Wave: DISCUSS / [REF] Pre-requisites

- Shipped: node `DnsResponder` serving the guest gateway (ADR-0116, ADR-0154); guest egress (ADR-0162); `overdrive-init` writing the guest `resolv.conf`.
- A VM Service spec whose guest program performs a lookup and a connect. The probe at `spike-scratch/guest-public-dns-probe/` (commit 46e80ccee, another branch) is evidence, not a fixture; DISTILL decides the fixture.
- A node host whose own resolver can resolve the test names (true on the metal box via dhcpcd).

## Wave: DISCUSS / [REF] Questions for DESIGN

All priorities (P1 and P2) are in scope for DESIGN. None of these is decided here.

**P1**
1. Where do the upstream resolvers come from on the appliance OS: the host's resolver configuration, `serve` configuration, or both? What if the host has none? How does the forwarder avoid ever forwarding to itself (the gateway address)?
2. Namespace boundary: the wire decoder today folds "not a valid mesh name" and "outside the mesh namespace" into one `NotMeshName` outcome. Which names are mesh-owned (never forwarded) versus forwardable? US-GPD-1 scenario 6 requires that a malformed name under `svc.overdrive.local` is not forwarded. What about other `overdrive.local` names and other `.local` names?
3. Upstream unreachable: reply code, node-side timeout budget, and how the budget composes with the guest's 5 s timeout and its retries (US-GPD-3).
4. Owned-range guard (D-4): mechanism, the SSOT shape, what the guest receives on a hit, whether `10.99.0.0/16` is in the set given #303 D4, and how a mixed answer set is treated.

**P2**
5. Caching: whether to cache forwarded answers; positive and negative lifetimes; kept separate from the mesh negative TTL (D-5). Resolver-side TTL honouring.
6. TCP/53 and truncation: does the responder serve TCP today? EDNS payload size; oversize answers; what the guest sees when UDP truncates.
7. DNSSEC passthrough: DO/AD bit handling.
8. Record types: AAAA and other types for non-mesh names (pass through or NODATA, given the guest's IPv4-only egress); CNAME chains.
9. Policy or egress control for non-mesh lookups: whether any applies, and how it relates to the ADR-0167 host-internal deny set.
10. Load and abuse bounds: one guest flooding the upstream; per-guest fairness; in-flight cap.
11. Observability: how forwarded outcomes appear next to the existing mesh query outcomes.
12. Reply source-pinning (`IP_PKTINFO`) for forwarded replies, per ADR-0116.

## Wave: DISCUSS / [REF] Scope Assessment result

**PASS** — 3 stories, 1 module (the node DNS responder) plus the guest-visible
behaviour, 3 slices, estimated 4-5 days. No oversize signal fires.

## Wave: DISCUSS / [REF] Wave decisions summary

- Job: minted J-OPS-005 (distinct progress and failure mode).
- Journey: extended `run-a-vm-workload.yaml` with step 7 rather than creating a new journey (the actor, lifecycle lens and VM guest context match; `dial-a-mesh-peer-by-name.yaml` is mesh-specific).
- Persona: extended Ana (no new persona).
- Slicing: 3 slices; guard before failure-honesty because forwarding makes upstream answers untrusted input.
- Evidence: the probe is spike evidence for the problem, not coverage.
- Risks: no DIVERGE artifacts; baseline measured at fe0c3adcf, not at HEAD.
- User rulings 2026-11-25: thresholds approved; per-workload upstream is not a limitation; lookup allowlist tracked by #383; AAAA is not a limitation (pass-through chosen in DESIGN); one EDD expectation added.

---

# DESIGN

> Status: user rulings A to E and the independent-review fixes of 2026-11-25 are applied; awaiting re-review (re-review findings F-1 to F-12 and M-1 to M-6 applied). ADRs 0356 to 0363 are accepted and unimplemented.

## Wave: DESIGN / [REF] Grounding findings that shape the design

1. **Responder today is UDP-only and wildcard-bound.** `bind_one(0.0.0.0)` first, gateway fallback (`responder.rs:283-298`); no TCP socket exists, so guest TCP to `gateway:53` (admitted by ADR-0167) reaches nothing and is refused by the host kernel.
2. **The serve loop is one blocking `recvmsg` per socket on `spawn_blocking`.** Waiting for an upstream inside that loop would stall mesh answers (violates AC-11). Forwarding must leave the receive thread.
3. **Every guest query reaches the responder with source = the gateway.** ADR-0152: the owner's socket connected to `gateway:53` sends from the gateway. The responder therefore cannot tell guests apart (no per-guest fairness is possible at this layer).
4. **Wildcard bind means the responder also receives datagrams addressed to any node address.** Forwarding unconditionally would make a node with a reachable address an open recursive resolver. This is reachable with no foreign software involved. The gate is the datagram's `ipi_spec_dst` (already read by the loop for source-pinning): forward only when it equals the gateway. Source reading: `fib_compute_spec_dst` returns the datagram's destination for a locally delivered unicast datagram (`net/ipv4/fib_frontend.c:283-285 @ v7.0`), `ipv4_pktinfo_prepare` stores it (`net/ipv4/ip_sockglue.c:1376-1408 @ v7.0`) and `udp_queue_rcv_one_skb` calls it per datagram (`net/ipv4/udp.c:2499 @ v7.0`). Measured (V-2): gateway-addressed gives 100.95.0.1, host-primary gives the host address, 127.0.0.1 gives 127.0.0.1, and real guest queries arrive with `spec_dst == 100.95.0.1` (`spike/findings-design-validation.md`).
5. **A forwarded query to the gateway would loop; a forward to any other host-local address terminates.** A socket connected to the gateway gets source = gateway (ADR-0152), so `spec_dst == gateway` and the gate would forward it again. A forward to any other host-local address (loopback, a node address) arrives with `spec_dst` equal to that address, the gate drops it, and the attempt times out inside the budget. So the one address to exclude is the gateway, in its IPv4 form and in its IPv4-mapped IPv6 form `::ffff:100.95.0.1`: an IPv6 socket that connects to a mapped address sends over IPv4 to that address (UDP `net/ipv6/datagram.c:186-197 @ v7.0`; TCP `net/ipv6/tcp_ipv6.c:219 @ v7.0`).
6. **Abuse capability is not new.** A guest already reaches any external resolver over ADR-0162 UDP/TCP egress. The forwarder adds no capability to flood or probe an upstream; it adds a node-resource exposure: descriptors and a task per in-flight query (one descriptor for a UDP forward, two for a TCP forward), bounded by ADR-0362.
7. **Owned ranges today have no single list; the guard needs only the ranges a guest connection is steered by.** The candidates are `WORKLOAD_FRONTEND_BASE` (`dns_responder/frontend_addr_allocator.rs:67`, `10.98.0.0/16`), `WORKLOAD_SUBNET_BASE` (`veth_provisioner.rs:249`, `10.99.0.0/16`), `GUEST_NODE_PREFIX` (`overdrive-core guest_transport/mod.rs:594`, `100.95.0.0/16`) and the operator-configurable `VipRange`. A guest TCP flow's destination is classified by `ServiceBackendsResolve` (`guest_network/owner_runtime.rs:1108` → `mtls_resolve_adapter.rs:576-633`):
   - a frontend hit, a VIP hit and any miss inside `10.98.0.0/16` are steered as mesh (arms 1 to 3, `:594-628`), so both ranges are guarded (together with `100.95.0.0/16`, below);
   - arm 4 steers an address as mesh only when a `service_backends` row carries it (`classify_by_addr`, `:632`, `:652`). Production rows carry a VM's `workload_addr`, leased from the guest pool (`action_shim/mod.rs:2063`), and `guest_network/pool_acceptance.rs:620-642` asserts no pool address lies in `10.99.0.0/16`. No production code assigns an address in `10.99.0.0/16` to a backend; the one production use of the range is the service-map hydrator's membership gate (`lib.rs:5760-5765`). **`10.99.0.0/16` is therefore not guarded: no production path steers a guest connection to it as mesh traffic.**
   - **`100.95.0.0/16` is guarded.** Guest `workload_addr`s are allocated in `GUEST_NODE_PREFIX` (`overdrive-core guest_transport/mod.rs:594`), and a guest TCP dial to another guest's `workload_addr:port` with a healthy service backend is classified Mesh by the exact-address match in arm 4 (`classify_by_addr`, `mtls_resolve_adapter.rs`, the fall-through after the `10.98.0.0/16` subnet check). A public answer in `100.95.0.0/16` would therefore alias into the mesh. ADR-0172's guest-prefix refusal covers only traffic not diverted to leg-C, so it does not cover this flow. The prefix also lies inside `100.64.0.0/10` shared address space, which Tailscale and CGNAT upstreams can legitimately return; that is the accepted cost of the guard (`dns-forwarder-answer-filtering-owned-ranges-research.md`).
   - `VipRange::contains` excludes reserved addresses, so it is the wrong containment test; `first_overlapping_range(/32)` is the right one and already exists.
8. **The appliance image's resolver configuration is not specified by any ADR** (ADR-0068 is silent). The metal box resolves through dhcpcd, but it is a provisioned test host, not the appliance.
9. **The DNS codec is `hickory-proto` behind `wire.rs` (DDN-3/DDN-4).** It stays the only crate naming hickory types. No resolver library is added.
10. **`serve`'s descriptor budget is its soft `RLIMIT_NOFILE`.** Descriptor allocation fails with `EMFILE` at the soft limit (`fs/file.c:622 @ v7.0`). The guest-flow owner reads it today live (`rustix::process::getrlimit`, `guest_network/flow_io/host.rs:334`) and refuses boot below its own requirement (`probe_guest_flow_startup`, `guest_network.rs:996-1000`); the live read is replaced by the injected value (ADR-0363). The raise happens once, at the `overdrive serve` binary entry, before the server starts (ADR-0363). Raising the soft limit to the hard limit needs no privilege but is not infallible: `do_prlimit` accepts `rlim_cur <= rlim_max` (`kernel/sys.c:1511 @ v7.0`), checks `rlim_max > sysctl_nr_open` for `RLIMIT_NOFILE` on **every** call, including one that leaves the maximum unchanged (`:1513-1515`), so EPERM is reachable if `nr_open` was lowered after the hard limit was set; it demands `CAP_SYS_RESOURCE` only when the maximum rises (`:1526-1527`); and it asks the security module (`security_task_setrlimit`, `:1530`), which can refuse. Hence a refused raise is handled, not treated as unreachable (below).
11. **The gateway route converges before the DNS responder is probed.** The gateway is a `local 100.95.0.1/32 dev lo table local` route, not an interface address (`converge_gateway_route` → `converge_ipv4_host_local_route`, `overdrive-netlink/src/client.rs:447-472`, called from `guest_network.rs:1325`). `run_server` calls `shared_guest_network.converge_shared()` at `lib.rs:4623` (a failure refuses boot, `:4623-4637`) and builds and probes the responder after it (`lib.rs:4737-4743`). A TCP `bind` to the gateway succeeds because `inet_bind` classifies the address through the local table and accepts `RTN_LOCAL` (`inet_addr_type_table`, `inet_addr_valid_or_nonlocal`, `net/ipv4/af_inet.c:501-513 @ v7.0`).

## Wave: DESIGN / [REF] Design decisions (DDD list)

User rulings of 2026-11-25, recorded as **approved**:

- **GPD-5 and GPD-6** approved as proposed (2026-11-25).
- **GPD-3**, option A (2026-11-25): upstreams are read from the host resolver file on each forwarded query, with an image-configuration requirement on #259 that the appliance image writes a usable resolver file. A stand-in until upstreams can be set in the worker node configuration ([#384](https://github.com/overdrive-sh/overdrive/issues/384), depends on #327).
- **Ruling A, GPD-1 (2026-11-25), now its own record GPD-8 / ADR-0362 (the bound and the shed) and ADR-0363 (the limit's raise and injection; independently reversible, so a separate record):** there is no fixed in-flight count. `serve` raises its soft `RLIMIT_NOFILE` to the hard limit and the forwarder's in-flight bound is derived from that limit minus a reserve. A query that cannot get a permit is shed at once: SERVFAIL on UDP; on TCP the accepted connection is closed unread (re-review F-1, direction of 2026-11-25: cheaper than reading and answering). Per-guest fairness stays [#385](https://github.com/overdrive-sh/overdrive/issues/385).
- **Ruling B, GPD-3 (2026-11-25):** no boot check of the resolver file. Each forwarded query reads it; an unreadable or empty file, or no usable upstream, gives SERVFAIL. The warn is per failure episode, not per query. A node health condition readable through the CLI is [#386](https://github.com/overdrive-sh/overdrive/issues/386), not designed here. The direction for startup checks on the appliance OS is [#376](https://github.com/overdrive-sh/overdrive/issues/376).
- **Ruling C, GPD-4 (2026-11-25, amended by the same-day user ruling below):** the guarded ranges are the mesh frontend range `10.98.0.0/16`, the configured service VIP pool, and the guest prefix `100.95.0.0/16` (`GUEST_NODE_PREFIX`); `10.99.0.0/16` is not guarded (Grounding findings, 7). The on-hit result is the dnsmasq shape: answer, authority and additional sections emptied, the upstream's rcode kept, and an Extended DNS Error "Blocked" (RFC 8914 code 15) when the query carried EDNS. Precedent: `docs/research/dataplane/dns-forwarder-answer-filtering-owned-ranges-research.md` (dnsmasq empties the whole reply and keeps the rcode; Unbound strips records; no surveyed system returns REFUSED). Accepted non-coverage is stated under [REF] Owned ranges.
- **Ruling C2, GPD-4 (2026-11-25):** the guard also covers `100.95.0.0/16`, for the reason in Grounding findings, 7. The owned-range-blocked event is logged at debug level per query, with no per-query warn; the `OwnedRangeBlocked` outcome count is test and benchmark observability only (the production observer records no outcomes, see Observer and events). The blocked reply's OPT advertises a UDP payload size of 1232 bytes (the DNS Flag Day 2020 value).
- **Ruling D, GPD-2 (2026-11-25):** all of `overdrive.local` stays reserved, unchanged.
- **Ruling E, GPD-7 (2026-11-25):** no synthesised FORMERR, NOTIMP or REFUSED-for-class replies and no `QueryOutcome::Malformed`.
- **Ruling F, GPD-8 (2026-11-26):** memory held at the in-flight bound is accepted with no second bound. The worst case and the per-permit costs (unmeasured estimates) are stated under the cost budget in [REF] Interface contracts; the TCP reply buffer grows only as bytes arrive, never pre-allocated to the length prefix (user ruling 2026-11-26; the ruling F decision, no second bound, is unchanged); flooding is [#385](https://github.com/overdrive-sh/overdrive/issues/385)'s concern.
- **Ruling G, GPD-7 and GPD-8 (2026-11-26):** shedding gets an episode warn, the same mechanism as the forward-failure episode warn: `dns.forward.shed_started` when shedding starts and `dns.forward.shed_cleared` when it stops, decided by one atomic flag whose swap return picks the line; one start and one clear per episode, no per-query cost (the end of an episode is defined in forwarder steps 8 and 9). Blocked answers stay debug-level (ruling C2).

| ID | Decision (one line) | ADR |
|---|---|---|
| GPD-1 | The responder forwards each gateway-addressed non-mesh query as a verbatim, per-query upstream exchange off the receive thread; sequential upstream failover inside a 4000 ms budget; an unanswered query gets SERVFAIL; no cache. | 0356 |
| GPD-2 | `overdrive.local` is a platform zone the node answers itself. One classifier over the decoded labels splits every query into mesh name, unknown platform name (NXDOMAIN, never forwarded), or forwardable. | 0357 |
| GPD-3 | Upstreams come from the host resolver file, read on each forwarded query; only the gateway is excluded; an unusable source is SERVFAIL, never a boot refusal. | 0358 |
| GPD-4 | A forwarded reply that carries an address in an Overdrive-owned range is emptied whole and marked Blocked; the owned set is one list: mesh frontend range, service VIP pool and guest prefix. | 0359 |
| GPD-5 | The responder also serves TCP/53, bound to the gateway, one query per connection, forwarding over TCP. | 0360 |
| GPD-6 | Every non-mesh query type (AAAA included), EDNS options and DNSSEC bits pass through unchanged; no synthesised answers. | 0361 |
| GPD-7 | `QueryOutcome` gains four variants so forwarded outcomes sit beside mesh outcomes; the only warn events are a forward-failure episode start and clear and a shed episode start and clear; an owned-range block is a per-query debug event. | in 0356 |
| GPD-8 | Forwards in flight run under permits derived from the process descriptor limit; TCP takes its permit at accept; excess is shed at once, with a shed episode warn (start and clear). The `serve` binary entry raises the limit and reads it back once, and injects the value through the guest-flow host seam. | 0362 (bound, shed), 0363 (raise, injection) |

## Wave: DESIGN / [REF] Component decomposition

All under `crates/overdrive-control-plane/src/dns_responder/` unless stated. No new crate. No new third-party dependency.

| Path | Change | Responsibility |
|---|---|---|
| `wire.rs` | EXTEND | Decode returns a classified question (`QueryClass`) or a `WireReject` (the datagram is dropped); `inspect_reply` projects an upstream reply into hickory-free `ReplyFacts`; reply-matching helper used by the exchange; encoders for SERVFAIL, the Blocked reply and the platform NXDOMAIN; `encode` generalised from `MeshServiceName` to the question name. `WireError::NotMeshName` is deleted (single cut). |
| `forwarder.rs` | CREATE NEW | `Forwarder`: select upstreams, run the budgeted failover, guard the reply, return the bytes to send. Owns the in-flight permit set, the failure-episode flag and the shed-episode flag. Never touches a guest socket. `forward_permits` (pure). |
| `upstream.rs` | CREATE NEW | The two driven ports (`UpstreamResolvers`, `UpstreamExchange`), `UpstreamSet` (pure selection), `DnsTransport`, error enums. |
| `upstream_host.rs` | CREATE NEW | Host adapters: `ResolvConfUpstreams` (reads a resolver file), `HostUpstreamExchange` (tokio UDP/TCP, one socket per exchange). |
| `owned_ranges.rs` | CREATE NEW | `OwnedAddressRanges`, `OwnedRangeKind`: the one list and the pure containment test. |
| `responder.rs` | EXTEND | Gateway-addressed gate; hand non-mesh work to the forwarder off the receive thread; send the forwarder's bytes through the existing source-pinned `sendmsg`; TCP listener at the gateway, accept loop, one query per connection; one cooperative cancellation token for the forward tasks and the accept loop. |
| `mod.rs` | EXTEND | `GuestDnsDeps.owned_ranges` and `GuestDnsDeps.forward_permits`; `HostGuestDnsFactory` carries the two upstream adapters; `QueryOutcome` +4 variants. |
| `lib.rs` (`run_server`, `clone_guest_dns_deps`) | EXTEND | Build `OwnedAddressRanges::platform(&config.vip_range)` and the permit count once (from the soft limit read through the guest-flow host-facts seam, `host_facts().nofile_limit`) and put them in the deps; log the permit count and the soft limit once (info). `run_server` does not touch the process limit. |
| `overdrive-cli` `main.rs` (serve dispatch) | EXTEND | Raise the soft `RLIMIT_NOFILE` to the hard limit, read the soft limit back, and pass it to `serve::run(args, nofile_soft)` (ADR-0363). The only place the process limit is changed. |
| `overdrive-cli` `commands/serve.rs` | EXTEND | `run` takes `nofile_soft` and passes it to `NodeAdapters::host`. Neither `ServeArgs` nor `run_with_adapters` changes; this file never changes the process limit (in-process tests call it). |
| `guest_network/flow_io/host.rs` | EXTEND | `HostGuestFlowIo::new(nofile_soft)` stores the value and returns it as `GuestHostFacts.nofile_limit` instead of calling `getrlimit` (`host.rs:334`); `impl Default for HostGuestFlowIo` (`host.rs:201`) is deleted (no caller, and a default has no value to inject); the sim keeps its own value (`flow_io/sim.rs:641`). |
| `guest_network.rs` | EXTEND | The guest-flow descriptor requirement becomes one function used by `probe_guest_flow_startup` and by the forwarder's reserve. `HostSharedGuestNetworkOwner::new` (`guest_network.rs:941`) is deleted: it has no caller (the boot uses `with_flow_io` over the bundle's `GuestFlowAdapter`) and it builds a `HostGuestFlowIo` with no injected value. |
| `node_adapters.rs` | EXTEND | `GuestFlowAdapter::host(nofile_soft)` and `NodeAdapters::host(config, cgroup_fs, kek, nofile_soft)` (below); `HostGuestDnsFactory::host()` replaces the unit-struct literal. |
| `overdrive-core/src/id.rs` | EXTEND | One const naming the platform zone beside `MeshServiceName::SUFFIX`. |
| `overdrive-sim/src/adapters/` | UNCHANGED | `SimGuestDns` unchanged. No sim adapter is added for the upstream ports; tests drive them with test-local scripted implementations (test-support surface is DISTILL's to shape). |

## Wave: DESIGN / [REF] Driving ports

Unchanged from DISCUSS: `overdrive serve`, `overdrive deploy <SPEC>`, and the guest's own resolver (UDP and TCP 53 to the gateway). The responder's UDP socket and the new TCP listener are the node-side entry. No CLI verb, flag, TOML key or HTTP route is added.

## Wave: DESIGN / [REF] Driven ports and adapters

| Port | Owner crate | Host adapter | Sim adapter | Probe |
|---|---|---|---|---|
| `UpstreamResolvers` (new) | control-plane | `ResolvConfUpstreams` | test-local scripted implementation | **None at boot** (ruling B). The file is read on every forwarded query; absent, unreadable, empty or no usable upstream is SERVFAIL with a distinct cause. Per-query fault scenarios for DISTILL: file absent, empty, unreadable (EACCES), replaced between reads, and recovery when restored. |
| `UpstreamExchange` (new) | control-plane | `HostUpstreamExchange` | test-local scripted implementation | None at boot. Reachability and socket creation are per-attempt runtime outcomes (`Unreachable`, `Io`); the world may legitimately have no network at boot. |
| `GuestDns` (existing) | control-plane | `DnsResponder` | `SimGuestDns` | `probe` also binds the TCP listener (bind failure is the existing `Bind` variant). `audit` is unchanged. |

Earned Trust: the boot refusal of ADR-0116 (bind plus name-index List-seed) is extended by the TCP bind only. A source that is read per use and can legitimately arrive late (DHCP) is not a boot fact; the per-use family of forwarders does not refuse at start (`docs/research/dataplane/dns-forwarder-upstream-config-failure-surfacing-research.md`, synthesis 1), and whether `serve`'s startup checks are needed at all on the appliance OS is [#376](https://github.com/overdrive-sh/overdrive/issues/376). The probe-contract enforcement is the repository's existing probe discipline; no new layer is added here.

## Wave: DESIGN / [REF] Technology choices

- `hickory-proto` 0.26 (Apache-2.0/MIT), already the DNS codec (DDN-3). The only new use is decoding an upstream reply; it stays inside `wire.rs`.
- `tokio` net and `tokio-util` `CancellationToken` (MIT), existing workspace dependencies, for the upstream exchange, the TCP listener and cooperative shutdown. No other runtime.
- Rejected: `hickory-resolver` or `hickory-server` as the forwarder. A resolver client re-synthesises answers (loses verbatim rcode, AD and RRSIG), brings its own cache and policy, and `hickory-server` was already rejected in DDN-3 for source-pinning. A hand-rolled codec is rejected by DDN-3.
- Per-query socket versus one shared multiplexed upstream socket: chosen on correctness, not measurement. A fresh kernel-assigned source port per exchange plus the guest's own id gives off-path-spoofing resistance with no id table, no shared mutable state, and nothing for a hazard model to cover. A shared socket needs an in-flight table and id rewriting. No cost comparison decides this, so no benchmark spike is listed.
- DNS stays in userspace (ADR-0116 stands); no kernel mechanism is proposed, so the kernel/userspace evidence rule does not apply to a new choice here.

## Wave: DESIGN / [REF] Interface contracts (pinned)

Contract shape per component (`.claude/rules/design.md` plus the effect-isolation mandate): classifier, reply inspection, guard, upstream selection, `forward_permits` and the reply encoders are **pure-function**. `Forwarder::forward` is **bounded-change**: its only effects are the in-flight permit, the failure-episode flag and exchanges through `UpstreamExchange`/`UpstreamResolvers`; it returns the reply bytes and never writes a guest socket (plan-value). The only writer to a guest socket stays the responder loop. `UpstreamResolvers` is read-only.

### Zone and classification (`overdrive-core::id`, `wire.rs`)

- `MeshServiceName::PLATFORM_ZONE: &str = "overdrive.local"`, next to `SUFFIX`. A unit-level const check keeps `SUFFIX` equal to `"svc." + PLATFORM_ZONE`.
- `QueryName`: hickory-free decoded label sequence of the question, ASCII-case-folded, root label implicit.
- `QueryClass { Mesh(MeshServiceName), PlatformUnknown, Forwardable }`.
- `classify(&QueryName) -> QueryClass` is the **one** predicate. It compares the **trailing labels** (`overdrive`, `local`), case-insensitively, never a rendered string. In zone (including the apex): `Mesh` iff `MeshServiceName::new` accepts it, else `PlatformUnknown`. Out of zone: `Forwardable`. Labels, not text, so `a\.svc.overdrive.local`, `evil-overdrive.local` and mixed case classify correctly.
- `decode(&[u8]) -> Result<DecodedQuery, WireReject>`; `DecodedQuery { id: u16, name: QueryName, qtype: RecordType, class: QueryClass }`. `WireReject` covers every datagram that is not one IN-class QUERY with one question (undecodable, QR set, opcode other than QUERY, QDCOUNT other than 1, class other than IN); it carries no reply.
- Inbound query receive limit: `RECV_BUF_LEN = 1500` (`responder.rs:86`) is kept. A stub resolver's single-question query is far below it; a longer datagram is truncated by the receive and then fails to decode or classify, which is a drop like every other `WireReject`.

| Inbound datagram (addressed to the gateway) | Result |
|---|---|
| `WireReject` (any case above) | dropped, as today |
| `Mesh` | exactly today's A / NODATA / NXDOMAIN-SOA (DDN-8 unchanged) |
| `PlatformUnknown` | NXDOMAIN with the zone SOA (negative TTL as DDN-8, 1 s) |
| `Forwardable` | forwarder outcome |

A datagram **not** addressed to the gateway keeps today's behaviour exactly: a `Mesh` name is answered, anything else (including `PlatformUnknown`) is dropped. No new reply is ever sent to a datagram addressed elsewhere.

### Upstream ports (`upstream.rs`)

```rust
#[async_trait] pub trait UpstreamResolvers: Send + Sync {
    /// Current nameserver candidates of the host, in file order.
    async fn candidates(&self) -> Result<Vec<IpAddr>, UpstreamConfigError>;
}
#[async_trait] pub trait UpstreamExchange: Send + Sync {
    async fn exchange(&self, server: IpAddr, transport: DnsTransport,
                      query: &[u8], timeout: Duration)
        -> Result<Vec<u8>, ExchangeError>;
}
pub enum DnsTransport { Udp, Tcp }
pub enum UpstreamConfigError { Absent, Unreadable { source: io::Error } }
pub enum ExchangeError { Timeout, Unreachable { source: io::Error },
                         Io { source: io::Error }, ReplyTooLarge }
```

Behaviour contract:
- `candidates`: re-reads the source on every call (no memo). A file that does not exist is `Absent`; a readable file with no `nameserver` line is `Ok(vec![])`; any other read failure is `Unreadable`. Lines other than `nameserver <ip>` are ignored; a `nameserver` token that is not a plain IPv4/IPv6 address (for example a scoped `fe80::1%eth0`) is skipped. Order preserved. Port is always 53.
- `exchange`: sends `query` to `server:53` over the chosen transport on a **fresh** socket (kernel-assigned source port) and returns the first reply that is a response (QR set) whose id and question (name case-insensitive, type, class) equal the query's, checked here once by the `wire` reply-matching helper; the forwarder does not re-check. Anything else is ignored until `timeout` (UDP) or fails the attempt as `Io` of kind `InvalidData` (TCP, which carries exactly one reply).
  - UDP: the socket is **connected** to `server:53`, so the kernel delivers only datagrams from that address and port (`compute_score` rejects a socket whose connected `inet_daddr` or `inet_dport` differs from the datagram's source, `net/ipv4/udp.c:368-396 @ v7.0`). One fixed receive buffer of `MAX_GUEST_DATAGRAM_PAYLOAD + 1` bytes (`overdrive-core guest_transport/wire.rs:661`, 59 000, the largest payload the guest transport can carry back); a datagram that fills it is `ReplyTooLarge`. `ReplyTooLarge` stays because it is a distinct failure mode with its own diagnostic; the forwarder treats it like any failed attempt.
  - TCP: RFC 1035 two-byte framing, one query per connection, connection closed on return; the length prefix bounds the reply at 65 535. The reply buffer grows only as bytes arrive and is never pre-allocated to the prefix's advertised size (user ruling 2026-11-26): memory held per TCP permit is proportional to the reply bytes actually received, capped by the prefix's length. The read loop's internals are the crafter's.
  - `timeout` bounds connect plus exchange. A refused connect or ICMP error is `Unreachable`, never `Timeout`. The socket is closed on every return path and when the future is dropped; dropping mid-exchange leaves no state to clean up.
- `UpstreamSet::select(candidates: Vec<IpAddr>, gateway: Ipv4Addr) -> Result<UpstreamSet, NoUsableUpstream>` (pure): drops only the gateway, in its IPv4 form and its IPv4-mapped IPv6 form `::ffff:<gateway>` (Grounding findings, 5); order preserved; no cap on the count (the budget bounds the attempts). An empty result is `NoUsableUpstream`. Every other host-local address terminates by the gate and the budget.
- `ForwardFailure` carries the cause and the last attempt's error, so the variants of `UpstreamConfigError` and `ExchangeError` reach the consumer of the episode warn: `ConfigAbsent`, `ConfigUnreadable { kind: io::ErrorKind }`, `NoNameserverListed`, `NoUsableUpstream`, `Attempts { last: Option<AttemptFailure>, budget_exhausted: bool }`. `Attempts` covers every case in which at least one upstream was tried or the deadline passed: `last` is the last attempt's failure (`None` only when the deadline passed before any attempt started), `budget_exhausted` says the deadline, not the end of the upstream list, ended the loop. `AttemptFailure` is `Timeout`, `Unreachable { kind: io::ErrorKind }`, `Io { kind: io::ErrorKind }`, `ReplyTooLarge`, `UndecodableReply`. Both enums have `as_str` for the label (for `Attempts`, `budget_exhausted` selects the label); the `io::ErrorKind` is carried as a value. Having no permit is not a `ForwardFailure`: the caller holds no permit and never calls `forward` (below).

### Forwarder (`forwarder.rs`)

```rust
pub const UPSTREAM_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(1500);
pub const FORWARD_BUDGET:           Duration = Duration::from_millis(4000);

pub struct Forwarder { /* resolvers, exchange, clock, owned, gateway, permit set, failure-episode flag and last-failure instant, shed-episode flag */ }
/// RAII in-flight permit. Not `Clone`. Released on drop.
pub struct ForwardPermit { /* ... */ }
pub struct ForwardRequest<'a> { pub wire: &'a [u8], pub transport: DnsTransport }
pub enum ForwardOutcome {
    Relayed(Vec<u8>),
    OwnedRangeBlocked { reply: Vec<u8>, address: Ipv4Addr, range: OwnedRangeKind },
    Failed { reply: Vec<u8>, cause: ForwardFailure },
}
impl Forwarder {
    pub fn new(resolvers: Arc<dyn UpstreamResolvers>, exchange: Arc<dyn UpstreamExchange>,
               clock: Arc<dyn Clock>, owned: OwnedAddressRanges, gateway: Ipv4Addr,
               permits: usize) -> Self;
    /// Takes one permit without waiting; `None` when all are held.
    pub fn try_acquire(&self) -> Option<ForwardPermit>;
    /// Runs one forward under a permit the caller already holds.
    pub async fn forward(&self, request: &ForwardRequest<'_>, permit: &ForwardPermit) -> ForwardOutcome;
}
/// Pure: the in-flight permit count for a soft descriptor limit and a reserve.
/// `nofile_soft.saturating_sub(reserve) / FDS_PER_PERMIT`, clamped to usize.
pub fn forward_permits(nofile_soft: u64, reserve: u64) -> usize;
pub const FDS_PER_PERMIT: u64 = 2;
```

`forward` contract:
1. The caller holds a permit; `forward` never acquires one. Where the permit is taken: a UDP query takes it with `try_acquire` after the query classifies `Forwardable` and passes the gateway gate; the TCP accept loop takes it when the connection is accepted, before reading, for every connection (see Responder lifecycle). With no permit available the caller sheds and never calls `forward`: UDP replies with a SERVFAIL built from the datagram (`QueryOutcome::Shed`); TCP closes the accepted connection unread (`QueryOutcome::Shed`). A shed query opens no upstream socket and reads no upstream configuration. The permit is held until the caller drops it: for UDP when the reply is sent, for TCP when the connection closes.
2. `deadline = clock.now() + FORWARD_BUDGET`. Read candidates, `UpstreamSet::select`. Config error or empty set: `Failed` carrying a SERVFAIL and the matching `ForwardFailure`.
3. For each upstream in order while time remains: `exchange(server, request.transport, request.wire, min(UPSTREAM_ATTEMPT_TIMEOUT, remaining))`. The query bytes are sent **unchanged** (guest id, RD, EDNS, DO, CD preserved).
4. `wire::inspect_reply` projects the validated reply into `ReplyFacts`. A reply it cannot decode counts as a failed attempt (`UndecodableReply`) and the next upstream is tried (a reply whose addresses cannot be inspected is never relayed).
5. An accepted reply of **any** rcode (NOERROR, NXDOMAIN, SERVFAIL, REFUSED, truncated) is final: no failover on an rcode. If any address in the answer section (A rdata; SVCB/HTTPS `ipv4hint`) is owned (`OwnedAddressRanges::owning_range`), return `OwnedRangeBlocked` carrying the **blocked reply**: the upstream reply with its answer, authority and additional sections emptied, the upstream's rcode and id and question kept, the AD and TC bits cleared, and, **only when the query carried EDNS** (an OPT record in `request.wire`), one OPT record (version 0, UDP payload size 1232, the DO flag copied from the query's OPT as RFC 3225 section 3 requires, no other flags) carrying one Extended DNS Error option, INFO-CODE 15 "Blocked" (RFC 8914), empty EXTRA-TEXT. Otherwise `Relayed(bytes)` verbatim, with no byte changed (AD bit, RRSIG, CNAME chain, TC preserved).
6. Every upstream failed or the deadline passed: `Failed` with SERVFAIL; cause `Attempts { last, budget_exhausted }`, carrying the last attempt's failure and whether the deadline ended the loop.
7. `forward` never returns later than `FORWARD_BUDGET` plus scheduling slack. It reads time only through the injected `Clock`.
8. Failure episode: the forwarder holds one flag and the instant of the last `Failed`, read through the injected `Clock`. Every line is decided by the return value of one atomic swap of the flag (a plain load may only skip a swap that cannot change it; it never decides). A `Failed` records its instant, swaps the flag to set, and reports an episode start when the swap found it clear. An accepted upstream reply (`Relayed` or `OwnedRangeBlocked`) first evaluates the threshold: at least `FORWARD_BUDGET` has passed since the last `Failed`. **The clearing swap itself is conditional on that threshold**: only when it holds does the reply swap the flag to clear, reporting an episode clear when the swap found it set; otherwise the flag is left untouched (an early accepted reply neither clears nor changes it). A clear is logged on the next qualifying accepted reply; a late or absent clear when traffic stops is intended, because a new episode cannot begin without further forwards. The check-then-swap can race with a concurrent `Failed` between the check and the swap; the effect is one early clear followed by a new start, a log-timing artifact only (staleness only affects when a line appears). Why that interval and no new constant: every forward is bounded by `FORWARD_BUDGET` (step 7), so once that long has passed since the last failure no forward that began before the upstream recovered can still fail; an earlier clear would be followed by those late failures and flap. Under intermittent failure closer together than `FORWARD_BUDGET` the episode stays open and is one start and one clear; failures further apart than that are separate episodes, so the warn pairs are at most one per `FORWARD_BUDGET`, never one per query. Nothing else changes the flag; a shed query never reaches `forward`, so it does not.
9. Shed episode (a second flag, same swap discipline): a query that finds no permit (`try_acquire` returns `None`) swaps the flag to set and reports `dns.forward.shed_started` when the swap found it clear. A `try_acquire` that succeeds first evaluates the threshold: at most half of the permits were held by others, measured before the acquire (`held * 2 <= permits`; the bound itself gives the threshold, there is no new constant). **The clearing swap itself is conditional on that threshold**: only when it holds does the acquire swap the flag to clear, reporting `dns.forward.shed_cleared` when the swap found it set; otherwise the flag is left untouched. A clear is logged on the next qualifying acquire; a late or absent clear when traffic stops is intended, because a new episode cannot begin without further acquires. The check-then-swap can race with a concurrent shed between the acquire and the swap; the effect is one early clear followed by a new start, a log-timing artifact only. At saturation a release followed by an acquire finds the permits all but one held, so it clears nothing; the episode ends only once the load has drained to half. With `permits = 1` the held count before the acquire is 0, so a single-permit bound still clears. Each line is emitted by exactly one caller and costs no per-query work beyond the load-and-swap on the shed and acquire paths (the held count is read from the permit set and may be stale by concurrent acquires; the only effect of staleness is when a log line appears). A shed query still opens no upstream socket and reads no upstream configuration.

Why these numbers (tuning, not shape). In-flight forwards equal the query rate times how long each holds a permit: healthy, that is the rate times the upstream round trip (1 000 queries per second at 10 ms is about 10 in flight); in a total outage it is the rate times 4 s (1 000 queries per second is about 4 000). The bound is therefore not a count chosen for load; it is the descriptor budget, because each in-flight forward holds descriptors (one for a UDP forward, two for a TCP forward) and one task. The guest resolvers observed are musl and glibc with a 5 s timeout. 4 000 ms leaves about 1 s for the association, the receive thread and the guest path. With one dead upstream the second is tried at 1 500 ms and the third at 3 000 ms; the budget, not a count, bounds the attempts. Under a total outage a glibc guest, which retries a SERVFAIL on its single nameserver, takes about 2 x 4 s before `EAI_AGAIN`; each reply is still definite and inside 5 s (AC-10). The 4 000 ms and 1 500 ms values are internal bounds with no operator setting; they are not a scope limitation.

**The descriptor limit (ADR-0363).**
- **The raise and the read-back are in `crates/overdrive-cli/src/main.rs` only.** The serve dispatch raises the soft `RLIMIT_NOFILE` to the hard limit, reads the soft limit back, and passes that value on. It is not in `commands/serve.rs` (`run_with_adapters` is called in-process by tests) and not in `run_server`, so in-process tests never mutate the process limit. There is no `ServerConfig` field for the value (ADR-0219: `ServerConfig` carries configuration only) and no `getrlimit` in `ServerConfig::new()` or anywhere in the control-plane library after this change.
- **Pinned signatures** (the value is the read-back soft limit, in descriptors):

  ```rust
  // overdrive-cli, commands/serve.rs   (the only caller is main.rs)
  pub async fn run(args: ServeArgs, nofile_soft: u64) -> Result<ServeHandle, CliError>;
  // overdrive-control-plane, node_adapters.rs
  impl GuestFlowAdapter { pub fn host(nofile_soft: u64) -> Self; }
  impl NodeAdapters {
      pub fn host(config: &ServerConfig, cgroup_fs: Arc<dyn CgroupFs>,
                  kek: Arc<dyn Kek>, nofile_soft: u64) -> Result<Self, ControlPlaneError>;
  }
  // overdrive-control-plane, guest_network/flow_io/host.rs (crate-private)
  impl HostGuestFlowIo { pub(crate) fn new(nofile_soft: u64) -> Self; }
  ```

  `HostGuestFlowIo` returns the stored value as `GuestHostFacts.nofile_limit`, replacing the live `getrlimit` at `flow_io/host.rs:334`; the sim keeps its own value (`flow_io/sim.rs:641`, 1,048,576). The existing descriptor probe (`probe_guest_flow_startup`, called through `probe_guest_network_startup`, defined at `lib.rs:3480` and called at `lib.rs:4005`) and the permit count both read `host_facts().nofile_limit`: one injected value, one seam, so `FdBudget` is decided on the same number the permits are derived from. `ServeArgs` and `run_with_adapters` are unchanged: the production `run` threads the value from `main.rs` into `NodeAdapters::host` in `commands/serve.rs:188`; a test composition passes its own value in the closure it already writes.
- **Call sites that change** (found by reading the tree; the compiler is the final list). `NodeAdapters::host` gains the fourth argument at: `overdrive-cli/src/commands/serve.rs:188` (production, the value from `main.rs`); `overdrive-sim/src/invariants/vm_reclamation.rs:639`; `overdrive-sim/src/invariants/netns_density_boot_order.rs:587`; `overdrive-sim/tests/vm_lifecycle_latency_283_spike.rs:304`; `overdrive-sim/tests/integration/convergence_admission_stop.rs:191`; `overdrive-bench-families/src/families/vm_lifecycle.rs:633`; `overdrive-cli/tests/acceptance/service_kind_vm_workloads.rs:110`; `overdrive-cli/tests/integration/http_client.rs:76`, `public_ingress_gateway.rs:326` and `:762`, `serve_fixture.rs:142`, `:170` and `:193`; `overdrive-control-plane/src/guest_listener_board_wiring_acceptance.rs:158`; `overdrive-control-plane/tests/common/node_adapters.rs:134`; `overdrive-control-plane/tests/acceptance/required_serve_ports_source_scan.rs:1685`; `overdrive-control-plane/tests/integration/guest_listener_real_boot.rs:164` and `guest_flow_through_serve.rs:314`; `tests/conformance/src/vm_lifecycle.rs:181`, `gateway.rs:1914` and `lib.rs:623`. `GuestFlowAdapter::host()` has one caller, `node_adapters.rs:436`. `HostGuestFlowIo::new()` is called at `node_adapters.rs:359`, `guest_network.rs:942` (deleted with `HostSharedGuestNetworkOwner::new`, which has no caller) and `guest_network/flow_io/accept_path_tier3.rs:218` and `:298` (these two never read host facts and pass the sim value). In `required_serve_ports_source_scan.rs` the allow-list entries at `:220` (`HostGuestFlowIo`, `new`), `:224` and `:228` match by name and do not change; the call at `:1685` gains the argument and the string fixture at `:1722` is unchanged. The `spike-scratch/` probes are separate builds and are not touched.
- **What the tests supply.** A test composition passes the sim seam's value, 1,048,576 (the number `SimGuestFlowIo` reports at `flow_io/sim.rs:641`); the test-support name for it is DISTILL's. `shared_guest_network_startup.rs:391` (`an_rlimit_nofile_below_the_owner_budget_refuses_boot`) stops lowering the process limit with `NofileGuard` and passes 1,024 to the bundle instead, so the refusal is still decided by the one seam and no process-global state changes. Production gets the value only from the read-back at the binary entry; test compositions pass the sim value.
- **A refused raise is handled.** `setrlimit` can fail with EPERM even for an unchanged hard limit (Grounding findings, 10: `nr_open` lowered after the hard limit was set, or a security-module refusal). The binary entry then keeps the current soft limit, reads it back (the read-back is authoritative either way), logs the refused raise at warn, and `serve` continues. The guest-flow probe already refuses boot when that limit is below its requirement, so a node that cannot raise and needs the raise fails at the existing gate, with `FdBudget` naming the available value, and not at the raise. A failure of the read-back itself ends `serve` with the error. The bound derives from the read-back value, so it never claims more than the kernel enforces.
- **Bounded effect.** Universe: `serve`'s soft `RLIMIT_NOFILE`. Delta: soft to hard only (the hard limit is unchanged). Inherited by the non-VMM children `serve` spawns (`ip`, `nft`, `bpftool`). VMMs are unaffected: the launcher sets their limit with `prlimit --nofile=<VmConfinement.rlimit_nofile>` (`crates/overdrive-core/src/vm/config.rs:508`, `:529`; the wrapper is built at `crates/overdrive-host/src/vmm.rs:120`).
- **No cap on the raise (decision).** A ceiling would be a number chosen without a measured reason (ruling A rejects that for the in-flight count and the same argument applies); the raise is already bounded by the hard limit the image sets, which is at most the kernel's `nr_open` (`sysctl_nr_open = 1024*1024` by default, `fs/file.c:96 @ v7.0`). The concern is a child that closes every descriptor up to the limit: `bpftool` has no such loop (no `RLIMIT_NOFILE`, `getdtablesize`, `sysconf(_SC_OPEN_MAX)` or `close_range` in `vendors/linux/tools/bpf/bpftool`); `serve` spawns children through Rust `Command`, which does not iterate descriptors and relies on close-on-exec. `ip` and `nft` are not vendored, so that they do not iterate is **not grounded**; the reachable cost if they did is bounded by the hard limit at one `close` per descriptor in the child.
**The permits and the shed (ADR-0362).**
- `permits = forward_permits(nofile_soft, reserve)` = `(nofile_soft - reserve) / FDS_PER_PERMIT`, saturating at 0. `FDS_PER_PERMIT = 2` is the descriptor cost of the most expensive forward: a TCP forward holds the connection accepted from the guest (a responder descriptor, outside the guest-flow owner's requirement) and its upstream socket; a UDP forward holds one (the responder's receive socket is shared and is part of `NODE_FD_RESERVE`). One uniform permit is chosen over weighted permits: it is a single integer, and over-counting a UDP forward only makes the bound conservative. It is computed once at boot and handed to `Forwarder::new` through `GuestDnsDeps.forward_permits`; a limit changed after boot is not tracked.
- `reserve = guest_flow_fd_requirement() + NODE_FD_RESERVE`. `guest_flow_fd_requirement()` is the one function holding the guest-flow owner's descriptor requirement (today the expression in `probe_guest_flow_startup`, `guest_network.rs:996-997`: `5 x GUEST_FLOW_MAX_PAIRS + 16,384 + 1 + 32,767`, with `GUEST_FLOW_MAX_PAIRS = 65,536` at `overdrive-core guest_transport/wire.rs:792`, so 376,832); the probe and the reserve both call it, so they cannot drift. `NODE_FD_RESERVE = 4096` is one fixed headroom for every other descriptor `serve` holds (stores, control-plane listeners, VMM children, the responder's own receive sockets). It is a round number, not measured: a larger reserve sheds earlier, a smaller one risks `EMFILE` on a non-forward path.
- **Arithmetic.** `reserve = 376,832 + 4,096 = 380,928`. At a hard limit of 1,048,576: `permits = (1,048,576 - 380,928) / 2 = 333,824`. At 524,288: `(524,288 - 380,928) / 2 = 71,680`. `permits = 0` (a limit between the guest-flow requirement and the reserve) sheds every forward; the guest-flow check already refuses a limit below its own requirement.
- **Shed.** A UDP query that cannot get a permit is answered SERVFAIL at once from the datagram. A TCP connection that cannot get a permit at accept is closed unread (cheaper than reading a framed query under the read deadline and writing a reply on a connection already refused a permit; the guest's resolver sees the connection end, as it does with no listener). A TCP connection takes a permit whatever the query class turns out to be, because the class is not known at accept; under saturation a mesh name asked over TCP is closed too, mesh over UDP is unaffected.
- **Memory budget (design.md cost budgets).** Per waiting permit: one socket (UDP: the upstream socket; TCP: accepted plus upstream), one task, and a copy of the query (at most `RECV_BUF_LEN` = 1,500 bytes; a TCP frame longer than that closes the connection, the same limit as UDP). **A UDP forward holds no receive buffer while waiting.** The upstream reply is received only after the socket is readable: one non-blocking receive into a scratch buffer of `MAX_GUEST_DATAGRAM_PAYLOAD + 1` = 59,001 bytes owned by the runtime worker thread and used inside one synchronous section with no `.await`, after which the reply is copied out at its exact length. UDP receive-buffer memory is therefore (worker threads x 59,001 bytes), not per forward; a held UDP reply costs its exact length (59,000 bytes at most) and only for the span between receive and send. **A TCP forward does hold a body buffer while waiting.** The reply's size is the two-byte length prefix (RFC 1035 section 4.2.2), up to 65,535 bytes, and the body is read across `.await` points, so the buffer the body is read into lives for the span between the prefix and the end of the body, per permit, not per worker thread. **The TCP reply buffer grows only as bytes arrive and is never pre-allocated to the prefix's advertised size (ruling 2026-11-26, see the TCP upstream exchange contract); held memory per TCP permit is proportional to the reply bytes actually received, capped by the prefix (65,535).** The read loop's internals are the crafter's. Estimates, **all unmeasured**: about 4 KiB per waiting UDP permit (kernel socket, task, query copy); about 8 KiB base per waiting TCP permit (accepted and upstream sockets, task, query copy), plus the reply bytes that permit has actually received and holds. Method: per-permit base cost times permits, plus received reply bytes; no measurement is planned (memory per permit is benchmark-shaped, `.claude/rules/testing.md`). Worst case at the example of 333,824 permits all waiting: about 1.3 GiB UDP-only (333,824 x 4 KiB), and about 2.6 GiB TCP-only (333,824 x 8 KiB) plus the reply bytes actually received and held. The earlier "every upstream stalls after a maximal length prefix, about 23 GiB" case no longer applies, because an upstream that sends a prefix and stalls costs no body memory. Both are reached by a total outage at about 83,000 queries per second for the 4 s budget. At 1,000 queries per second in an outage (about 4,000 permits) it is about 16 MiB UDP-only and about 31 MiB TCP-only (no body is received in an outage). The bound is the descriptor limit by ruling A; the memory it implies at a very high hard limit is the cost of that ruling and is stated here, not capped. **Accepted by the user 2026-11-26 (ruling F): no second memory bound.** The ruling itself is unchanged. **Ruling 2026-11-26 (reply buffer):** the TCP upstream reply buffer grows only as bytes arrive and is never pre-allocated to the length prefix's advertised size; this replaces the earlier maximal-prefix worst case. Flooding is [#385](https://github.com/overdrive-sh/overdrive/issues/385)'s concern.
- **Pinned shapes for the reserve (user's 2026-11-27 ruling approving the permit-derivation acceptance test; pinning its observable, not a new decision).** Both live in `overdrive-control-plane`'s existing `pub mod guest_network` (`guest_network.rs`), `pub` so the out-of-crate acceptance test (`tests/acceptance`) recomputes the expectation from the same items `serve` uses; `forward_permits` and `FDS_PER_PERMIT` are already `pub` in `dns_responder::forwarder`:

  ```rust
  // overdrive-control-plane, guest_network.rs
  /// The guest-flow owner's descriptor requirement; the one expression `probe_guest_flow_startup` also uses.
  pub fn guest_flow_fd_requirement() -> u64;
  /// Fixed headroom for every other descriptor `serve` holds.
  pub const NODE_FD_RESERVE: u64 = 4096;
  ```
- **Boot log (pinned).** After deriving the bound and before building the forwarder, the composition (`run_server`) emits exactly once per boot: `tracing::info!(name: "dns.forward.permits", permits = <usize>, nofile_soft = <u64>, reserve = <u64>)`. `permits` is the value passed to `GuestDnsDeps.forward_permits`; `nofile_soft` is `host_facts().nofile_limit`; `reserve` is `guest_flow_fd_requirement() + NODE_FD_RESERVE`. The acceptance test asserts `permits == (nofile_soft - reserve) / FDS_PER_PERMIT` (saturating) over the event's own fields, and `reserve` against the two pinned items.

### Owned ranges (`owned_ranges.rs`)

```rust
pub enum OwnedRangeKind { MeshFrontend, ServiceVip, GuestNode }   // as_str
pub struct OwnedAddressRanges { /* frontend net + VipRange clone */ }
impl OwnedAddressRanges {
    /// THE list. Adding an Overdrive-owned range is one line here.
    pub fn platform(vip_range: &VipRange) -> Self;
    pub fn owning_range(&self, address: Ipv4Addr) -> Option<OwnedRangeKind>;
}
```

`platform` reads `WORKLOAD_FRONTEND_BASE` and `GUEST_NODE_PREFIX` by reference to their defining consts and the service VIP pool through `VipRange::first_overlapping_range` with a /32 (reserved VIP addresses count as owned). The pool is `config.vip_range`, the same `VipRange` value the VIP allocator and `ServiceBackendsResolve` are built from (`lib.rs:4417-4419`). `platform` is built once at the composition root and passed in `GuestDnsDeps`. A range added to an allocator but not to `platform` is not detectable by type; a reviewer symptom, and see Open questions for the one test that would pin it.

Accepted non-coverage (user, 2026-11-25): the guard checks answer-section A records and SVCB/HTTPS `ipv4hint` only. It does not check AAAA, because a guest has only a link-local IPv6 address (V-3) and cannot connect to an aliased IPv6 address (the revisit trigger is guest IPv6 transport, [#308](https://github.com/overdrive-sh/overdrive/issues/308): once a guest can connect over IPv6 the guard must cover AAAA); and it does not check the authority or additional sections (glue), because stub resolvers ignore addresses outside the answer section.

### Wire reply contract (all synthesised replies)

Id and question echoed, QR set, RD copied from the query, RA set. SERVFAIL (from `Failed` and `Shed`) carries no records. NXDOMAIN for `PlatformUnknown` carries the existing synthetic SOA. The blocked reply is defined by forwarder step 5. Reply source is pinned to `ipi_spec_dst` through the existing `sendmsg` path for every reply kind, forwarded included (ADR-0116, Question 12): the loop captures `(peer, spec_dst)` at receive and hands them to the forward task together with a shared handle to the socket; `sendmsg` on a UDP socket is safe from another thread.

### Responder lifecycle and load (`responder.rs`)

- A datagram is handled on the receive thread only as far as classification and the gateway gate. A `Forwardable` gateway-addressed query takes a permit (`try_acquire`; none: SERVFAIL at once on the receive thread's send path) and is handed to a task on the serve task's runtime, which holds the permit until the reply is sent.
- **Shutdown is cooperative, by one cancellation token.** One `CancellationToken`, cancelled by the responder's `stop()`, is observed by every forward task and by the TCP accept loop (per-connection tasks hold child tokens). Each task's body selects on the token with `biased;`; an exchange in progress is dropped at that point, which leaves nothing to clean up (it holds only a socket). There is no task tracker and no `abort()`; task handles are detached (`.claude/rules/rust.md` § "Concurrency & async"). The blocking UDP receive loop keeps its existing stop flag.
- **TCP.** A listener bound to `gateway:53` only (never the wildcard). It binds after the gateway route has converged: `run_server` converges the shared guest network (`lib.rs:4623`) before it probes the responder (`lib.rs:4743`), and a converge failure has already refused boot. That order is a contract: the responder's probe MUST NOT be moved before `converge_shared`, because the bind relies on the gateway being a local-table address (Grounding findings, 11). No bind option (`IP_FREEBIND`) is needed or used. **The permit is taken at accept** (`Forwarder::try_acquire`, before any read) and held until the connection closes; the upstream exchange runs under that same permit. With none available the accepted connection is closed unread. **One query per connection**: the accept loop reads one two-byte-framed query under a read deadline of `TCP_READ_DEADLINE = 5 s` (covering the length prefix and the body), runs the same classify, answer-or-forward path with `DnsTransport::Tcp`, writes the framed reply and closes. A zero length, a short read or an expired deadline closes the connection without a reply. Connection count needs no cap of its own: every connection holds a permit, and the permits are the cap.
- `DnsResponderError` is unchanged: a TCP bind failure reuses `Bind`. There is no `Upstream` variant and no new boot-refusal reason.

### Observer and events (GPD-7)

`QueryOutcome` gains `Forwarded`, `ForwardFailed`, `OwnedRangeBlocked`, `Shed`. `Shed` is an answered SERVFAIL and is distinct from the existing `Dropped` (unanswered). A `PlatformUnknown` NXDOMAIN reports the existing `NameError`. The `overdrive-bench-families` DNS family and any exhaustive match on `QueryOutcome` update in the same change. Tracing events:
- Event field names are pinned (each emitted as `tracing::<level>!(name: "<event>", ...)`; `qname` is used because `name` is tracing's reserved metadata key). `dns.forward.failed` fields: `qname = %<query name>`, `cause = <&'static str label>`, `detail = %<detail>`. `dns.forward.recovered` fields: none. `dns.forward.owned_range_blocked` fields: `qname`, `address = %<Ipv4Addr>`, `range = <OwnedRangeKind::as_str>`, `upstream = %<server>`. `dns.forward.shed_started` fields: `permits = <usize>` only; `dns.forward.shed_cleared` fields: `permits = <usize>`, `held = <usize>` (the held count measured before the acquire). The forwarder knows only `permits`; the soft limit is carried by the boot event `dns.forward.permits`, not the shed events.
- `dns.forward.failed` (warn, **once per failure episode start**: name, cause label, and the cause's detail: the last attempt's failure and, for an unreadable resolver file, the `io::ErrorKind`) and `dns.forward.recovered` (warn, once when the episode clears: an accepted reply at least `FORWARD_BUDGET` after the last failure), per forwarder step 8; individual failed queries are counted through `QueryOutcome::ForwardFailed`, not logged. This is the dnsmasq pattern (`dns-forwarder-upstream-config-failure-surfacing-research.md`, Finding 4).
- `dns.forward.owned_range_blocked` (debug, per blocked query: name, address, range kind, upstream). There is no per-query warn.
- `dns.forward.shed_started` (warn, once when shedding starts: permit count) and `dns.forward.shed_cleared` (warn, once when a permit is acquired with at most half of the permits held by others), per forwarder step 9; individual shed queries are counted through `QueryOutcome::Shed`, not logged.
- No event for `Forwarded` or an individual `Shed`.
- **The outcome counts are test and benchmark observability, not an operator signal.** The production observer is `NoQueryObserver` (`dns_responder/mod.rs:107`, `:153-159`); `QueryObserver` counts reach only tests and benches. Operator visibility today is the failure-episode warn and the shed-episode warn; a CLI-readable condition waits on #386.
- A node health condition readable through the CLI is tracked by [#386](https://github.com/overdrive-sh/overdrive/issues/386); this feature designs none.

### Config surface

None added. The resolver file is `/etc/resolv.conf`, a const in the host adapter, with a `from_path` constructor for tests. The budgets are constants; the in-flight bound is derived (see above).

## Wave: DESIGN / [REF] Reuse Analysis

| Existing | Verdict | Justification |
|---|---|---|
| `DnsResponder` (`responder.rs`) | EXTEND | Owns the sockets, source-pin, probe/audit; the forwarder plugs into its loop. No second resolver (D-1). |
| `wire.rs` | EXTEND | The only hickory boundary; classification and reply inspection belong behind it. `NotMeshName` folding deleted. |
| `answer_for`, `NameIndex`, `FrontendAddrAllocator` | REUSE unchanged | Mesh arm untouched (D-6). |
| `WORKLOAD_FRONTEND_BASE`, `GUEST_NODE_PREFIX` | REUSE (referenced) | The owned list points at the defining consts. |
| `VipRange::first_overlapping_range` | REUSE | Containment test with reserved addresses counted. No new accessor. |
| `probe_guest_flow_startup` descriptor requirement | EXTEND | Becomes one function (`guest_flow_fd_requirement`) shared with the forwarder's reserve; the probe's behaviour is unchanged. |
| `GuestDns` / `GuestDnsFactory` / `GuestDnsDeps` | EXTEND | Two new deps, factory carries the two adapters. Port methods unchanged. |
| `QueryObserver` / `QueryOutcome` | EXTEND | Four variants (ADR-0307 observer). |
| `Clock` | REUSE | Forward budget runs on the injected clock. |
| core `Transport` | DO NOT REUSE | Connection/datagram carrier for the workflow engine; no request-reply exchange with deadline, source and id validation. |
| hickory-proto, tokio, tokio-util | REUSE | No new dependency. |
| `SimGuestDns` | REUSE unchanged | Socket-free supervisor double; forwarding is tested through `Forwarder` with the new sim ports. |
| `Forwarder`, `UpstreamResolvers`/`UpstreamExchange` + adapters, `OwnedAddressRanges` | CREATE NEW | No existing alternative: nothing in the tree exchanges DNS with an upstream or holds the owned-range list. Ports exist so the Tier-1 tests drive the real forwarding logic with a scripted upstream. |
| Upstream-port test doubles | NO NEW SIM ADAPTER | The upstream ports are driven in tests by test-local scripted implementations; test-support surface is DISTILL's to shape. |
| TCP listener in `responder.rs` | CREATE NEW (in an extended file) | GPD-5; none exists. |

## Wave: DESIGN / [REF] Decisions index and external integration

ADRs: 0356 forwarding, 0357 namespace classifier, 0358 upstream source, 0359 owned-range guard, 0360 TCP/53, 0361 non-mesh record types pass through, 0362 in-flight bound and shed, 0363 descriptor-limit raise and injection. ADR-0072 receives an explicit amendment (the responder contract is operative). ADR-0116, ADR-0154 and ADR-0167 are confirmed unchanged: ADR-0116 and ADR-0154 contain no wording that the responder drops or never forwards non-mesh queries, and ADR-0167's "non-mesh" wording concerns guest flows, not DNS names.

External integration: the host's upstream DNS resolvers (standard RFC 1035 wire). Consumer-driven contract testing (Pact) does not fit DNS; the boundary is covered by a test-local scripted `UpstreamExchange` plus the real-wire Tier-3 run against a controlled upstream. Recorded for platform-architect as: no contract-test tooling recommended for the resolver boundary.

## Wave: DESIGN / [REF] Lifecycle Gate Ownership

**Applicable in one respect**: the design extends the inputs of one existing boot gate. The forwarding conditions are answer-time conditions on one query, not gates on any lifecycle state (allocation, readiness, probe, driver).

| Signal or state | Owning component | Promise it makes | Inputs that may gate it | States it must not gate |
|---|---|---|---|---|
| Responder boot (`GuestDns::probe` Ok) | `DnsResponder`, via `run_server` | The node can answer mesh queries and serve DNS over UDP and, for the gateway, TCP | UDP bind, **TCP bind (new)**, name-index List-seed | Allocation `Running`, probe/readiness, guest launch, upstream reachability, the resolver file |

### Gate G-1 — responder probe also requires the TCP bind

- **Existing evidence:** ADR-0116 and ADR-0138: `run_server` calls `responder.probe()` and maps a failure to `health.startup.refused` (`lib.rs:4743-4764`, `DnsResponderError::boot_refusal_reason`).
- **Owner:** `DnsResponder::probe` (unchanged owner).
- **Promise:** every socket the responder serves on is bound, so a refusal is a boot fact and not a lazy failure.
- **Affected state:** boot admission only.
- **Failure projection:** TCP bind failure is the existing `Bind` reason. No new reason.
- **Explicitly unaffected:** upstream reachability, the resolver file (absent, unreadable or empty is a per-query SERVFAIL), any allocation or readiness state, mesh answers.
- **Ordering:** after `converge_shared` (`lib.rs:4623`, which converges the gateway route the TCP bind relies on) and after the UDP bind; before the name-index List-seed returns; consumes no new deadline.
- **Counterexample:** gating boot on a reachable upstream or a readable resolver file would refuse a node whose network is still coming up and would take mesh naming down with it (AC-11).
- **Evidence lane:** unit (reason mapping), Tier 3 on metal (bind refusal with the gateway TCP port held).

Boundary scenarios for DISTILL: (1) gate available, boot proceeds; (2) TCP port held, typed refusal; (3) resolver file absent, boot proceeds, query gets SERVFAIL with an episode warn, and recovery clears the episode once an accepted reply arrives at least `FORWARD_BUDGET` after the last failure, with a failure arriving after a first success but inside that interval leaving the one episode open (no second start); (4) saturation: with every permit held, a further UDP query gets an immediate SERVFAIL and a further TCP connection is closed unread, with no upstream socket opened and no upstream contacted; a mesh query over UDP is still answered; one `dns.forward.shed_started` warn is emitted for the whole shedding run (not one per shed query); releasing one permit and acquiring it again at saturation emits no `dns.forward.shed_cleared` (repeated release-and-acquire pairs at saturation emit no warn at all); one `dns.forward.shed_cleared` warn follows only when a permit is acquired with at most half of the permits held by others, and a second saturation after that starts a new episode; (5) feature disabled does not exist (the forwarder is composed unconditionally, like the responder, ADR-0138); (6) conditional clear, shed episode: saturated, then release and re-acquire with more than half of the permits held, gives exactly one `dns.forward.shed_started` and no `dns.forward.shed_cleared`, and the flag is left set (a later shed emits no second start); (7) conditional clear, failure episode: after a `Failed`, an accepted reply earlier than `FORWARD_BUDGET` gives exactly one `dns.forward.failed` start and no `dns.forward.recovered`, and the flag is left set (a later failure emits no second start).

No-obligation ruling (raise and read-back): the `RLIMIT_NOFILE` raise and read-back in `crates/overdrive-cli/src/main.rs` have no obligation. They are a thin syscall wrapper in the binary crate and are not reachable by in-process tests (tests never spawn the binary). Consequences are bounded by the existing guest-flow `FdBudget` boot refusal and the derived permit count; the refused-raise branch logs and continues. The code stays in `main.rs` only.

No-obligation ruling (late upstream reply): an upstream reply arriving after the budget has no obligation, because each exchange uses a fresh socket that is closed on timeout, so a late reply has no destination. No test is added.

## Wave: DESIGN / [REF] Formal protocol model

**Not applicable.** Evidence: each forward is an independent, single-owner, linear sequence (select, attempt 1..n, accept, guard, reply) with no shared mutable state; it uses a fresh socket and the guest's id, so there is no id table, no multiplexing and no ordering between queries. The shared objects are the in-flight permit set (an atomic acquire-or-shed) and two episode flags, the failure-episode flag (with the instant of the last failure) and the shed-episode flag. Each flag is flipped by one atomic swap whose return value alone decides whether a log line is emitted, and the hysteresis (forwarder steps 8 and 9) adds only a comparison read before that swap: a clock difference for the failure episode, a held count for the shed episode. Neither flag gates any query, reply, permit or state another owner reads. The worst a race can do is a misordered or one extra start-and-clear log pair (a read that is stale by a concurrent acquire or failure moves a line by one event). No safety or progress property depends on a flag, so no model is needed with the hysteresis either. There is no cache, no session, no lease and no crash-sensitive step (a lost forward is a retransmitted query). Timing and failover are covered by seeded unit tests over `SimClock` and test-local scripted upstream ports (DISTILL), not a model. **If the user chooses to add a forwarded-answer cache, this changes: shared state and TTL ordering would require a Quint spec through the `quint-modeler` dispatch.** No dispatch is listed.

## Wave: DESIGN / [REF] Validation items

| ID | Item | Class | Status |
|---|---|---|---|
| V-1 | **Appliance resolver source.** The appliance image provides a readable resolver file listing at least one non-loopback nameserver before guest traffic starts. | contract-shaping | Resolved by ruling (GPD-3 option A, 2026-11-25): the host resolver file plus an image-configuration requirement on #259. With no boot check (ruling B), a missing or empty file is a per-query SERVFAIL, so a falsification changes no contract. The check runs when #259 produces an image; the stand-in ends when [#384](https://github.com/overdrive-sh/overdrive/issues/384) lands. Whether `serve` needs startup checks at all on the appliance: [#376](https://github.com/overdrive-sh/overdrive/issues/376). |
| V-2 | **Gateway gate discriminates.** A datagram addressed to a non-gateway node address arrives with `ipi_spec_dst` equal to that address; a guest query arrives with `spec_dst == gateway`. | contract-shaping | Held (`spike/findings-design-validation.md` § V-2). No ADR-0116 change. |
| V-3 | **AAAA pass-through is harmless to an IPv4-only guest.** | contract-shaping | Held (`spike/findings-design-validation.md` § V-3). The sub-prediction "an IPv6 connect alone fails in under 10 ms" was not measured. |
| V-5 | **Guest TCP/53 reaches a gateway TCP listener.** | contract-shaping | Held (`spike/findings-design-validation.md` § V-5). |
| V-4 | Guest-observed latencies with the chosen constants (EDD receipt): K-GPD-1/2 under 1000 ms; outage reply under 5000 ms; **an owned-range blocked lookup** with its guest-observed time and the `getaddrinfo` result under glibc and under musl; **a saturation case** (every permit held) answered over UDP with an immediate SERVFAIL, guest-observed. This item measures latency only; it carries no memory measurement. | tuning | DELIVER (EDD receipt) |
| V-6 | `hickory-proto` with the current feature set decodes a DNSSEC-signed reply (RRSIG/NSEC) and HTTPS/SVCB records without error; an undecodable reply must not turn a legitimate answer into SERVFAIL. | internal | DISTILL scenario with a signed fixture |

## Wave: DESIGN / [REF] Scope limitations (user rulings 2026-11-25)

- **L-2 — No per-guest fairness.** Every guest query reaches the responder with source = gateway (finding 3), so one noisy guest can use the whole in-flight bound and starve other guests' public lookups. Mesh answers are not affected. Tracked by [#385](https://github.com/overdrive-sh/overdrive/issues/385) ("Per-guest fairness for forwarded non-mesh DNS queries").
- **L-3 — Upstreams come only from the host resolver file.** There is no node-configuration setting. A stand-in until upstreams can be set in the worker node configuration: tracked by [#384](https://github.com/overdrive-sh/overdrive/issues/384) (depends on #327). Per-workload override is not a limitation (DISCUSS ruling).
- **Lookup allowlist:** no allowlist or policy on non-mesh lookups, tracked by [#383](https://github.com/overdrive-sh/overdrive/issues/383).
- **AAAA: no limitation.** Pass-through approved (GPD-6); [#308](https://github.com/overdrive-sh/overdrive/issues/308) is not needed as a stand-in for this feature. #308 (guest IPv6 transport) is the revisit trigger for the owned-range guard's AAAA non-coverage (ADR-0359).
- Not limitations: the in-flight bound is derived, not fixed; the 4000 ms and 1500 ms budgets are internal bounds; one query per TCP connection is a property of the mechanism.

## Wave: DESIGN / [REF] Open questions for DISTILL and DELIVER

- DISTILL must record, per cross-boundary criterion, contract, owner path, fault stimulus, oracle and tier (testing.md). Suggested stimuli, not additions: upstream silent, upstream returns an owned address, upstream returns TC, resolver file absent or empty, every permit held, mesh query during an upstream outage, gateway-addressed versus other-addressed datagram.
- One pin worth deciding in DISTILL, not added by this design: a source scan that each owned-range const appears in `OwnedAddressRanges::platform`. Reachable failure: a future allocator range silently unguarded. Cost: one acceptance test. Cheaper alternative: a review symptom only.
- The EDD expectation (DISCUSS) still stands; no change.
- AC-3 (NXDOMAIN for a nonexistent public name) is satisfied by relaying the upstream's rcode.

## Wave: DESIGN / [REF] Changed Assumptions

- Quoted (`responder.rs`, `answer_datagram` doc): "a malformed or non-mesh query is silently dropped, never answered with a fabricated addr". Replaced: a gateway-addressed non-mesh query is forwarded, or answered with NXDOMAIN (unknown `overdrive.local` name) or SERVFAIL (no upstream answered; also a UDP query shed at saturation, while a shed TCP connection is closed unread); a datagram that is not a single-question IN-class query is still dropped; a non-gateway-addressed non-mesh query is still dropped.
- Quoted (`wire.rs`, `WireError::NotMeshName`): "The question name is not a valid `<workload>.svc.overdrive.local`". Replaced by `QueryClass`; a name under `overdrive.local` that is not a mesh name is NXDOMAIN, a change from silence. ADR-0072 carries the explicit amendment that retains the old contract.
- DISCUSS Technical Notes (US-GPD-2) wording "the workload subnet `10.99.0.0/16` matters if D4 lands on DNS time" is resolved: `10.99.0.0/16` is not guarded, because no production path steers a guest connection to it as mesh traffic (Grounding findings, 7); the DISCUSS note is updated.
- D-4 says "SSOT would name 10.98 (and 10.99 if D4 lands on DNS time)"; this design adds the service VIP pool and the guest prefix `100.95.0.0/16` and leaves 10.99 out; the user approved this under rulings C and C2 on 2026-11-25.

## Wave: DESIGN / [REF] Quality attribute notes

Security: open-resolver gate, owned-range guard, loop exclusion, no upstream capability beyond ADR-0162. Reliability: definite reply within the budget, under saturation an immediate SERVFAIL on UDP and a close on TCP, no state to recover, the mesh path over UDP independent of the forwarder (a TCP connection of any class is closed at saturation because its class is unknown at accept). Performance: no cache; one extra upstream round trip per lookup, bounded by `FORWARD_BUDGET`; in-flight bound scales with the descriptor limit. Maintainability: one classifier, one owned list, one hickory boundary, ports for the two effects. Testability: `Forwarder`, classifier, guard and selection are testable with no socket; saturation is reachable through the `permits` constructor parameter.

---

# DISTILL

> Quinn, 2026-11-27. Executable specification: `distill/test-scenarios.md`
> (GIVEN/WHEN/THEN, contract shape, owner path, fault, oracle, tier, placement,
> test path, pending or active, per scenario). No `.feature` files
> (`.claude/rules/testing.md`). Reconciliation passed: 0 contradictions across
> DISCUSS and DESIGN (no DEVOPS wave; default environment). Verified on metal
> under Linux 7.0.0-38.

## Wave: DISTILL / [REF] Inherited commitments

| Origin | Commitment | DDR | Impact |
|--------|------------|-----|--------|
| DESIGN#GPD-1..GPD-8 | The forwarder, classifier, upstream source, owned-range guard, TCP/53, pass-through types, outcomes and permits are the contract the scenarios assert | n/a | Every pinned behaviour has at least one scenario; the bodies are authored so DELIVER only removes markers and implements |
| DESIGN boundary scenarios 1-7 | The seven boundary scenarios are executable obligations | n/a | All six that exist are covered (scenario 5 does not exist by design); mapping in `distill/test-scenarios.md` |
| DISCUSS#K-GPD-1..5 | Pass/fail thresholds from single recorded runs, no cohort | n/a | K-GPD-1/2/5 are asserted by the guest-level scenarios; K-GPD-3/4 by the wire and tier-1 scenarios |
| DISCUSS#Verification expectations | One EDD expectation, point-in-time, captured at DELIVER | n/a | E17 is authored and `pending`; it is not a test |

## Wave: DISTILL / [REF] Scenario list with tags

65 new tests (61 pending, 4 active: W-3, W-6, W-14, G-4); 49 (75%) are error,
fault, edge or guard scenarios. Full table with test paths in `distill/test-scenarios.md`.

| Group | Count | Tags | Tier / lane |
|---|---|---|---|
| Guest through `serve` + `deploy` (G-1..G-4) | 4 | `@walking_skeleton` (G-1), `@driving_port`, `@real-io`, `@kpi` (K-GPD-1, 2, 4, 5) | Tier 3, `integration-tests,kvm-tests`, guest `cgroup-default`/`nested` |
| Responder on real UDP/TCP 53 (W-1..W-15) | 15 | `@driving_port`, `@real-io @adapter-integration`, `@error` (W-2, 4, 7..15), `@infrastructure-failure` (W-8..W-10, W-13) | Tier 3, `integration-tests`, guest `cgroup-default` |
| Driven adapters (U-1..U-8) | 8 | `@real-io @adapter-integration` | Tier 3, `integration-tests` |
| `Forwarder` contract (F-1..F-34, F-19 removed) | 33 | `@in-memory`, `@property` (F-12, F-26, F-33), `@error`, `@infrastructure-failure` | Tier 1, default lane |
| Owned-range list (O-1..O-4) | 4 | `@in-memory`, `@property` (O-1, O-2); O-4 is a source scan | Tier 1, default lane |
| `serve` boot permit derivation (P-1) | 1 | `@driving_port`, `@real-io` | Tier 3, `integration-tests`, guest `cgroup-default` |

Contract shape per scenario is recorded in the test docs
(`bounded-change` or `pure-function`); none is `unbounded-preservation`.

## Wave: DISTILL / [REF] WS strategy (Architecture of Reference)

One walking skeleton: G-1, a VM guest deployed through `serve` + `deploy`
resolves `github.com` through its default resolver and connects. Driving ports
and the host resolver file are real; the only non-production piece is the
scripted upstream resolver at the port boundary (the node's upstream is a
driven external port, faked with output capture) and a route-disjoint remote
peer that gives the connect something to reach. Nothing is installed in the
guest; nothing hand-installs a missing production effect. **Status: authored in
full, pending, RED proven on metal** (`f0db8cd7505ada0a31fcbbff`:
`resolve: nameserver=100.95.0.1`, `resolve: github.com error=EAI_AGAIN
ms=5006`, Job `Failed` where `Terminated` is asserted). It cannot be GREEN
before DELIVER: the capability is the feature.

## Wave: DISTILL / [REF] Adapter coverage table

| Driven adapter / boundary | Real-I/O scenario | Notes |
|---|---|---|
| Host resolver file (`ResolvConfUpstreams`, `/etc/resolv.conf`) | U-1..U-4 (real file, tempdir); W-9, W-10 (the guest's real `/etc/resolv.conf` rewritten in a guest group) | Absent, unreadable, empty, replaced, restored |
| Upstream exchange over UDP (`HostUpstreamExchange`) | U-5, U-6, U-7 (loopback sockets); W-1, W-2, W-7, W-8 (scripted upstream on the wire); G-1..G-4 | Wrong id, wrong question, timeout, refusal, oversize |
| Upstream exchange over TCP | U-8; W-11 | One query per connection; framing mismatch |
| Gateway TCP listener (`GuestDns::probe`, accept loop) | W-11, W-12, W-13 | Bind refusal, one query per connection, closed unread at saturation |
| Descriptor-limit seam (`host_facts().nofile_limit` to the permit count) | No new real-I/O scenario. The arithmetic is pure (F-26, F-27) and the bound is reachable through the `permits` constructor parameter (F-28..F-32, W-13). G-1 fails under a miswired `permits = 0` at boot (everything would be shed). The `RLIMIT_NOFILE` raise in `main.rs` has a DESIGN no-obligation ruling. Re-pointing `shared_guest_network_startup.rs:391` at the injected value is a DELIVER change named in the DESIGN | Proposal below |
| Guest resolver path (`getaddrinfo`, UDP/TCP 53 to `100.95.0.1`) | G-1..G-4 | Real guest, real egress |
| `Clock` | Faked (`SimClock`) in tier 1; real in tier 3 | Budget timing is deterministic in tier 1 |

## Wave: DISTILL / [REF] Scaffolds

Created so every test compiles and fails by assertion (`MISSING_FUNCTIONALITY`),
never by import. Each matches the pinned contract; bodies are `todo!` with a
file-level `#![allow]` carrying a reason.

| File | Items | Contract line it matches |
|---|---|---|
| `crates/overdrive-control-plane/src/dns_responder/forwarder.rs` | `UPSTREAM_ATTEMPT_TIMEOUT`, `FORWARD_BUDGET`, `FDS_PER_PERMIT`, `forward_permits`, `Forwarder::{new, try_acquire, forward}`, `ForwardPermit`, `ForwardRequest`, `ForwardOutcome`, `ForwardFailure`, `AttemptFailure` | Interface contracts, Forwarder block. `try_acquire` always grants until DELIVER lands the bound; `forward_permits` and `forward` are `todo!` |
| `.../dns_responder/upstream.rs` | `UpstreamResolvers`, `UpstreamExchange`, `DnsTransport`, `UpstreamConfigError`, `ExchangeError`, `UpstreamSet::select`, `NoUsableUpstream` | Interface contracts, Upstream ports block. `select` is `todo!` |
| `.../dns_responder/upstream_host.rs` | `ResolvConfUpstreams::from_path`, `HostUpstreamExchange` (unit struct) | "a `from_path` constructor for tests" (Config surface). The port bodies are `todo!` |
| `.../dns_responder/owned_ranges.rs` | `OwnedRangeKind` (+ `as_str`), `OwnedAddressRanges::{platform, owning_range}` | Interface contracts, Owned ranges block. `owning_range` is `todo!` |
| `.../dns_responder/mod.rs` | `GuestDnsDeps.owned_ranges`, `GuestDnsDeps.forward_permits`, `HostGuestDnsFactory::host()` | Component decomposition, `mod.rs` row; `NodeAdapters` row (`host()` replaces the unit-struct literal) |
| `crates/overdrive-control-plane/src/lib.rs` | the two new deps at the three `GuestDnsDeps` sites (`forward_permits: 0` marked SCAFFOLD) | `lib.rs` row: DELIVER derives the count from `host_facts().nofile_limit` |
| `crates/overdrive-core/src/id.rs` | `MeshServiceName::PLATFORM_ZONE` | Zone and classification block |
| `crates/overdrive-sim/src/adapters/guest_dns.rs` | test fixture gains the two deps | compile only |

`HostUpstreamExchange` is used in the tests as a unit-struct value, a
test-support use of the pinned type. DESIGN pins `ResolvConfUpstreams::from_path`
and the `UpstreamExchange` port, not how `HostUpstreamExchange` is constructed;
note for DELIVER: if it needs state, the construction change routes back to the
architect rather than to an improvised constructor.

Test support, DISTILL's to shape (`overdrive-testing`): `dns_upstream`
(`ScriptedDnsUpstream`, `HostResolverFile`, `Behaviour`, `Transport`; Linux
only), the guest program subcommands `resolve` and `resolve-connect` with
`exit::RESOLVE` and the console line contract, and the `guest_peer` handler for
them.

Not scaffolded because no test observes them through a port: `QueryClass`,
`QueryName`, `classify`, the `wire.rs` extensions, `QueryOutcome` +4 variants,
the responder's TCP listener and forward tasks. (No sim upstream adapters exist: DESIGN drives the upstream ports in tests with test-local scripted implementations.)

## Wave: DISTILL / [REF] Test placement

Directory choice follows the existing layout: default-lane tests in
`crates/overdrive-control-plane/tests/acceptance/` (precedent
`dns_name_index.rs`, `dns_frontend_assigner.rs`); integration tests under
`tests/integration/` (precedent `dns_responder_bind.rs`); the guest-level tests
in `crates/overdrive-cli/tests/integration/` (precedent
`vsock_walking_skeleton.rs`).

| File | Placement |
|---|---|
| `acceptance/dns_forwarder.rs`, `acceptance/dns_owned_ranges.rs` | `placement!(host_pure)`: in-process, scripted ports, `SimClock`, no socket and no kernel state |
| `integration/dns_upstream_host.rs` file tests | `#[overdrive_test(host_pure)]`: a tempdir file, no node-global effect |
| `integration/dns_upstream_host.rs` exchange tests | `guest = "cgroup-default", profile = "normal-gate"` (binds sockets) |
| `integration/dns_forwarding_real_wire.rs` | `guest = "cgroup-default", profile = "normal-gate"` (binds `:53`, rewrites the guest's `/etc/resolv.conf`) |
| `guest_public_dns.rs` | `guest = "cgroup-default", profile = "nested"` (boots `serve` and launches a VM) |

Nothing product-facing is `native_launcher`; nothing that mutates kernel state
is `host_pure`. `cargo xtask placement-lint` is clean. No lane, nextest group,
feature or xtask command was added.

## Wave: DISTILL / [REF] Driving-adapter coverage

| Driving port | Scenario |
|---|---|
| `overdrive serve` (composition root) | G-1..G-4 boot it in-process through the production composition; W-1..W-13 build the responder through `HostGuestDnsFactory::host()`, the factory `serve` uses |
| `overdrive deploy <SPEC>` | G-1..G-4 deploy a VM Service and read its end state; tests never spawn the binary (`testing.md`) |
| Guest resolver, UDP 53 to the gateway | G-1..G-4 (real `getaddrinfo`); W-1..W-10, W-13 (real datagrams) |
| Guest resolver, TCP 53 to the gateway | W-11, W-12, W-13; F-5 and U-8 for the forwarder and exchange |
| `GuestDns::{probe, serve, stop}` | W-12 and every W test |

## Wave: DISTILL / [REF] Pre-requisites

- A Linux 7.0 native-metal host with KVM and the guest kit prepared (`cargo xtask guest-kit-prepare`, `kmod-load`), for tiers with `guest =` placement.
- The host's resolver file is not read by the tests: they write their own inside a guest, or a tempdir file. E17 reads the metal host's real one.
- DESIGN driving ports as listed; no DEVOPS wave.

## Wave: DISTILL / [REF] RED classification

All 61 pending tests are `MISSING_FUNCTIONALITY`; the 4 active tests (W-3,
W-6, W-14, G-4) pass against today's responder. Every long pending body was
walked to its GREEN end against the pinned contract; one defect (F-30's extra
permit at saturation) was fixed. Per group (review-fix run): default lane (35),
run `fb50c37a0613b080f2629e85`, 35 passed, plus the two later additions F-34 and
O-4, run `6287f4a5d7989b21f12d127f`, 2 passed; P-1 (`serve` boot permit derivation), run `f2416a8f35e1d7effe93f718`, 1 passed; integration (30, including the
existing `dns_responder_bind` regression set), run `e3ce508b20c17e855d135326`,
30 passed; Tier 3 guest (4), run `3569a6d3cc1eef699cfd9586`, 4 passed; real RED
evidence for G-1 in `f0db8cd7505ada0a31fcbbff`. `cargo clippy -D warnings`
over the workspace with `integration-tests` (`02681b1eb276f2fc43175eba`) and
over `overdrive-cli` and `overdrive-control-plane` with
`integration-tests,kvm-tests` (`c564866300091c9e9cefd6c1`) is clean;
`placement-lint` is clean (`305db135d470664902e16f02`). Detail and the semantic
RED of each group: `distill/test-scenarios.md` § "RED classification".

## Wave: DISTILL / [REF] Verification expectation

E17 (`verification/expectations/E17-vm-guest-resolves-public-names-through-its-default-resolver/`,
indexed in `verification/expectations/INDEX.md`): authored, `pending`, anchored
to AC-1, AC-3, AC-4, K-GPD-1/2, V-4 and ADR-0356. A point-in-time capture on
metal at DELIVER by a different reviewer; not a test, not a cohort.

## Wave: DISTILL / [REF] Wave decisions

- Pending tests are `should_panic(expected = ...)` over complete bodies, not placeholders. Tier-1 and `dns_upstream_host` REDs name the scaffold's `todo!` message, which proves the body reached the production call; the shed and wire scenarios name an assertion.
- Tests key tracing events on `meta.name()`: production tracing macros MUST set `name:` explicitly for the `dns.forward.*` events (`failed`, `recovered`, `shed_started`, `shed_cleared`, `owned_range_blocked`); `owned_range_blocked` is emitted by the forwarder, which alone knows the upstream the event names.
- G-4 asserts a definite error result only, with no `ms < 5000` bound, and is active: the guest-observed time depends on the guest resolver library's SERVFAIL handling (musl does not accept a SERVFAIL as an answer and resends a bounded number of times; glibc retries once), which DESIGN V-4 measures at DELIVER. The reply-level AC-10 oracle is W-8 and F-8.
- Mandate-12 (Python-pilot types module, step-reuse ratio) does not apply to Rust tests; the equivalent is one fixture per tier (`World`, `Node`) and no business logic in test bodies beyond the oracle.
- The one pin DESIGN left to DISTILL (a source scan that each owned-range const appears in `OwnedAddressRanges::platform`) is added as O-4 by user ruling 2026-11-27, with the concurrency test for the permit bound (F-34). The user's third approved test, the permit derivation at boot, is added as P-1 once DESIGN pinned its observable (`dns.forward.permits`, `guest_flow_fd_requirement`, `NODE_FD_RESERVE`); the two pinned items are scaffolded in `guest_network.rs` (`guest_flow_fd_requirement` is a `todo!` scaffold, `NODE_FD_RESERVE` is final).
- Open proposals (not added): `ForwardFailure::as_str` and `AttemptFailure::as_str` are pinned but no test reads the label, so they are DELIVER's with no pending test.
