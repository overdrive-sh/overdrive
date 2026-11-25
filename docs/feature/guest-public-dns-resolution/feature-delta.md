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
| D-4 | A forwarded answer that lands in an Overdrive-owned address range must not reach the guest as an ordinary answer. The guard is keyed on **one SSOT of owned ranges**. Today that SSOT would name the frontend subnet `10.98.0.0/16` (`WORKLOAD_FRONTEND_BASE`); the workload subnet `10.99.0.0/16` matters if D4 lands on DNS time. Recorded as a requirement; the mechanism and the on-hit result are DESIGN's. | User |
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
- [ ] AC-9: The guest never receives silence for a guarded answer; it gets a definite result (which result is DESIGN's).

**Technical Notes:** grounded in production: forwarding to the host's
resolver (dhcpcd-generated on the metal box) passes whatever that resolver
returns. Whether `10.99.0.0/16` is guarded depends on where #303 D4 puts
mesh-name resolution; DESIGN decides, the SSOT must make it a one-line change.

### US-GPD-3 — A guest gets a definite, prompt failure when the node cannot reach any upstream resolver

**Job:** J-OPS-005

**Elevator Pitch:**
- **Before:** when the host's resolver is unreachable, a forwarding responder that waits silently reproduces today's symptom: every guest lookup hangs for the full 5 s and fails with a generic "Try again".
- **After:** `overdrive deploy herdr-box.toml`, then in the guest `getaddrinfo("github.com")` with the host's upstream resolvers unreachable fails with a definite error before the guest resolver's 5 s timeout; mesh lookups still answer during the outage; the next lookup after the upstream returns succeeds with no restart of the node or the guest.
- **Decision enabled:** Ana can tell "the node's upstream DNS is down" from "my guest program is broken", and the guest's own retry logic gets a fast, definite signal.

**Problem:** a forwarder adds a dependency the node did not have. If the
upstream is slow or down, the experience must not degrade to the same
5 s silent timeout that motivated this feature, and it must not take mesh
naming down with it.

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
- [ ] AC-10: With every upstream unreachable, a guest lookup of a public name returns a definite DNS reply before the guest's 5 s resolver timeout. (**Proposed requirement, not in the issue; needs user confirmation.** The 5 s figure is the measured baseline, 5005 ms.)
- [ ] AC-11: Mesh answers are unaffected by the state of the upstream.
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

## Wave: DISCUSS / [REF] DoR validation (9 items)

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

- **One node-wide upstream set, no per-workload resolver override — not a limitation.** A guest that wants a different resolver can already query it by IP over its ordinary egress (ADR-0162). No issue.
- **No allowlist or policy on non-mesh lookups.** Every non-mesh name a guest asks for is forwarded. Tracked by [#383](https://github.com/overdrive-sh/overdrive/issues/383); DESIGN Question 9 decides only whether this feature applies any existing control.
- **IPv4 answers only for non-mesh names (if DESIGN chooses IPv4-only / NODATA for AAAA).** Stand-in until guest IPv6 transport lands: tracked by [#308](https://github.com/overdrive-sh/overdrive/issues/308). No new issue.

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
- User rulings 2026-11-25: thresholds approved; per-workload upstream is not a limitation; lookup allowlist tracked by #383; AAAA limitation (if any) tracked by #308; one EDD expectation added.
