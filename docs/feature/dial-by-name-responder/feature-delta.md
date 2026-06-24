# Feature-Delta — dial-by-name-responder (DISCUSS · DRAFT)

> **Status: DISCUSS reviewed (2026-06-24) — gated to Slice 00.** The single
> authoritative DISCUSS narrative for this feature — the compact `feature-delta.md`
> form mandated by the `nw-discuss` Outputs contract + `validate_feature_layout.py`;
> the legacy split `discuss/*.md` files (user-stories, story-map, dor-validation,
> outcome-kpis, wave-decisions) are intentionally **not** produced, their content
> lives here. Lean density, Tier-1 `[REF]` sections. Produced by Luna
> (nw-product-owner) on 2026-06-24 for `dial-by-name-responder` (GH #243); revised
> per `review-discuss.md` (2026-06-24). **Cleared for Slice 00 (the spike) only —
> full responder DESIGN is BLOCKED until the spike records `PROMOTE`** (see the Gate
> verdict). Slice briefs under `slices/`.

## Reading checklist

- ✓ `docs/feature/dial-by-name-responder/intake.md` — primary source (GH #243 body, pinned contracts, ping-pong demo requirement, grounded code locations)
- ✓ `docs/product/jobs.yaml` — J-SEC-003 (parent enforcement arc), J-OPS-004 (operator-trust reachability family), header precedent ("JTBD skipped — distilled from whitepaper/issue, not interviews")
- ✓ `docs/product/personas/sam-platform-security-engineer.yaml` — Sam, the actor
- ✓ `docs/product/journeys/enforce-transparent-mtls-on-the-wire.yaml` — parent enforcement journey (pure enforcement, no operator verb)
- ✓ `docs/evolution/2026-06-22-transparent-mtls-enrollment.md` — finalized arc; D-TME-9/10/11 + responder reframe (#61 → #243)
- ✓ `examples/coinflip-as-service.toml` — `[service]`/`[exec]`/`[resources]`/`[[listener]]` schema + `/tmp`-staged-binary precedent
- ✓ `examples/dns-resolver.toml` — `command` = real on-disk binary (`/usr/bin/socat`); port-collision honesty (avoid 5353)
- ✓ `verification/expectations/E04-workload-reachable-at-canonical-address-mtls/README.md` — EDD expectation shape this feature graduates a sibling into

---

## `[REF]` Persona

**Sam — platform/security engineer** (`docs/product/personas/sam-platform-security-engineer.yaml`).
Defends the mesh story to a security reviewer. For this feature his lens shifts
from *enforcement* ("is the wire actually encrypted?") to *reachability* ("can an
**unmodified** workload even **find** its mesh peer by name, and does the platform
**never** hand it a stale address that points at a dead instance?"). Sam runs the
proof himself — `overdrive deploy` two specs, watch the ping-pong counter advance,
confirm each hop went TLS 1.3 on the peer wire.

Secondary actor (relates-to, not added to `personas/`): **Ana — application developer** (the workload author).
She writes an *ordinary* program that does `getaddrinfo("b.svc.overdrive.local")`
and connects — no SDK, no SVID, no mesh awareness. Her job is that this Just Works.
(Ana is referenced as a relates-to actor; this feature is authored through Sam's lens
per the codebase's single-persona-per-feature precedent.)

---

## `[REF]` JTBD — one-liner + proposed job entry

**One-liner (the job this feature serves):** *When I run two mesh workloads that
must talk to each other, I want an **unmodified** workload to dial a peer **by its
name** (`<job>.svc.overdrive.local`) and land at a **live** instance the mesh then
mTLS's — so the platform's "every flow is identity-bearing" promise is reachable
from ordinary code, not just enforceable once a connection already exists.*

### Job-tracing decision: MINT a new job `J-MESH-001` (reachability/mesh family)

**Decision: (a) mint a NEW job**, `relates_to: [J-SEC-003]`, actor Sam (+ Ana).
**Justification (one line):** dial-by-name has a **distinct progress** (an unmodified
workload **reaches** a mesh peer **by name** and lands at a **running** instance) and
a **distinct failure mode** (name resolution returns nothing / a stale address → the
workload **cannot even initiate** the connection) — orthogonal to J-SEC-003's
enforcement progress (cleartext on the wire / handshake that doesn't fail closed).
The two are independently satisfiable and independently failable.

This follows the **J-SEC-002 mint precedent** (jobs.yaml changelog 2026-06-08:
minted distinct from J-SEC-001 on "different progress + failure mode"), NOT the
**udp-sendmsg4 elevation precedent** (2026-06-05: elevated under J-OPS-004 because it
was the *same* reachability job at finer granularity). Dial-by-name is genuinely
distinct from enforcement — collapsing it under J-SEC-003 would over-correct exactly
as the J-SEC-002 analysis warned.

> **Why not elevate under J-OPS-004?** J-OPS-004 is *operator-trust in the
> Service-submit wire signal* (does `Stable`/`Failed` reflect real liveness). This
> job is *peer-to-peer name reachability inside the mesh* — a different progress,
> different actor-circumstance (a workload dialing a peer, not an operator reading a
> submit stream). A J-MESH-* family is the honest home; J-OPS-004 would fragment.

**APPLIED jobs.yaml addition** (user-approved 2026-06-24 — see Wave-Decisions
§ Applied SSOT diffs; the entry below is the source of the landed `jobs.yaml` text):

```yaml
  - id: J-MESH-001
    title: "Let an unmodified workload reach a mesh peer by name and land at a live instance — no SDK, no mesh awareness"
    relates_to: J-SEC-003
    situation: >
      When a workload I run must open a connection to another mesh workload by
      its logical name (<job>.svc.overdrive.local) — and the workload is
      identity-unaware and unmodified (it just calls getaddrinfo + connect like
      it would anywhere) — and the platform has already SHIPPED resolv.conf
      injection (each per-netns /etc/resolv.conf points at the per-netns gateway,
      D-TME-9) but NOTHING answers there yet, so name resolution fails in a deploy,
    motivation: >
      I want an in-agent name-answering listener (in the same process that owns
      the agent-light L4 proxy and the ServiceBackendsResolve index — NOT a
      separate daemon, NOT in-kernel) to answer A for <job>.svc.overdrive.local
      with the running backend's IPv4 address from the shared resolve index (the
      v1 substrate is IPv4 — the MtlsResolve addr is SocketAddrV4, the per-netns
      responder addr is Ipv4Addr), returning the SAME address MtlsResolve
      recognizes (headless, no VIP, D-TME-10); answer AAAA as NODATA (the name is
      currently resolvable, no IPv6 record — a real IPv6 backend story is out of
      v1 scope); and NXDOMAIN when no backend is running (declared-but-empty and
      unknown are indistinguishable — the responder reads the running resolve
      index) — never a stale address,
    outcome: >
      so an unmodified workload can DIAL a mesh peer BY NAME and land at a live
      instance the existing intercept path then mTLS's — closing the dial-by-name
      leg the transparent-mtls arc deferred (#236), making the mesh reachable from
      ordinary code rather than only enforceable on a connection that already
      exists; and so the resolve index becomes "one source, THREE readers"
      (outbound resolve + inbound install + name answers) with the name layer
      byte-consistent with what the intercept path enforces.
    source: >
      GH #243; D-TME-9/10/11 (docs/evolution/2026-06-22-transparent-mtls-enrollment.md);
      whitepaper §11. Distilled from the issue + the finalized arc, not from user
      interviews (per the jobs.yaml header precedent).
    served_by_phase: 2
    status: active
```

---

## `[REF]` Locked decisions / pinned contracts (settled inputs — DO NOT re-litigate)

| ID | Contract | Status |
|---|---|---|
| **D-TME-9** | `resolv.conf` injection SHIPPED — each per-netns `/etc/resolv.conf` carries `nameserver <responder_addr>`; `responder_addr` = per-netns gateway (`plan.host_addr`, Fly `fdaa::3` model, collision-free by construction). `veth_provisioner.rs` `WriteResolvConf`. | **Shipped, do not touch** |
| **D-TME-10** | Return shape = **HEADLESS** — answer with a `running` `service_backends` addr, the **same** address `MtlsResolve.resolve` recognizes (one source, byte-consistent). **NO VIP, NO #167, NO #61.** | **Pinned** |
| **D-TME-11** | Read mechanism = the shared **`ServiceBackendsResolve`** index (in-RAM, address-keyed, ownership-aware reverse index, List-then-Watch + relist-on-`Lagged`). This feature makes it **"one source, THREE readers"** (outbound resolve + inbound install + **name answers**). | **Exists; this feature adds the 3rd reader** |
| **v1 address family** | The shipped resolve/intercept substrate is **IPv4**: `ResolvedBackend.addr` is `SocketAddrV4` (`mtls_resolve.rs`), and `responder_addr` + the netns addrs are `Ipv4Addr` (`veth_provisioner.rs`). **v1 answers `A` with the running IPv4 backend addr; answers `AAAA` as NODATA** when the name is currently resolvable; **a name with 0 running backends → NXDOMAIN** (declared-but-empty and unknown both collapse — the responder reads only the running index). See § *The v1 DNS answer contract* above for the canonical table. A real IPv6 backend story (widening the `SocketAddrV4`/`Ipv4Addr` substrate) is **OUT of v1 scope**. | **Pinned (forced by the shipped types)** |
| **Implement-to-design** | This feature describes **behavior + the pinned contracts**. It does NOT invent the responder's API surface, new public types, or the `MtlsResolve` shape — those are DESIGN-wave decisions. | **Hard constraint (CLAUDE.md)** |

**Today's gap (the thing this feature closes):** nothing answers on `responder_addr`.
`getaddrinfo` reaches an injected resolver with no responder behind it → name
resolution fails in a deploy → an unmodified workload cannot initiate a by-name
connection at all.

### `[REF]` The v1 DNS answer contract (canonical — every artifact matches this)

| Query | Name has ≥1 running IPv4 backend | Name has 0 running backends\* |
|---|---|---|
| `A` | **NOERROR + A** (the running IPv4 addr) | **NXDOMAIN** |
| `AAAA` | **NOERROR / NODATA** (resolvable name, no IPv6 record — v1 is IPv4) | **NXDOMAIN** |

\* *0 running backends covers BOTH a declared-but-not-running service AND an
unknown name — v1 does **not** distinguish them, because the responder reads only
the running `ServiceBackendsResolve` set, so both collapse to **NXDOMAIN**. A
stale / cached / guessed address is **never** returned. The NXDOMAIN carries a
short negative-TTL so a retrying dialer re-resolves promptly once a backend reaches
Running (DESIGN pins the exact TTL). A future refinement could return NODATA for a
declared-but-empty service IF the responder gains a declared-service view distinct
from the running index — that is **not** v1.*

- **NODATA** = the name **is** currently resolvable but has no record of the queried
  type (only `AAAA` in v1, since the substrate is IPv4).
- **NXDOMAIN** = no currently-resolvable backend for the name.

This table is the single contract US-DBN-2 / US-DBN-4, slice-03, the KPIs, and the
product journey all reference. (Resolves the empty-answer ambiguity flagged in
`review-discuss.md` Blocking #1.)

---

## `[REF]` The load-bearing unvalidated mechanism (→ SPIKE-FIRST, do NOT design here)

**How does ONE in-agent listener answer DNS queries sent to N different per-netns
gateway addresses?** A query is emitted from inside each workload netns toward *that
netns'* gateway addr; one host-side listener must receive and answer all of them.
This is a real-kernel **netns / routing / binding** question with **no Tier-2
backstop** (`spike.md` "no synthetic harness" case). It MUST be validated in a
timeboxed probe (Slice 00) **before** the walking skeleton. **This document names it
as a dependency; it does NOT solve it.** The arc's own precedent spiked throughout
(increment-a/b/c/d/i).

---

## `[REF]` User stories

> All stories trace to **J-MESH-001**. Each carries an Elevator Pitch
> whose "After" is a real operator entry point (`overdrive serve` / `overdrive
> deploy <SPEC>`). ACs are embedded + testable. `@infrastructure`-tagged stories
> (the spike) carry an infrastructure rationale in lieu of an Elevator Pitch.

### US-DBN-1 `@infrastructure` `@spike` — One listener, many netns: validate the routing assumption

**Job:** `J-MESH-001` · **Infrastructure rationale:** This is a timeboxed PROBE of an
unvalidated real-kernel mechanism (`spike.md`), not a shippable behavior. It produces
a WORKS/DOESN'T-WORK verdict + a promotion-gate decision, not operator-observable
value. No Elevator Pitch (per the `@infrastructure` exemption).

**Problem:** Sam cannot trust the walking skeleton until he knows a single host-side
listener can actually *receive and answer* DNS sent to N distinct per-netns gateway
addresses. If it can't, the whole headless-in-agent design is wrong and must pivot
before any production code lands.

**Solution (behavior to probe — NOT to design):** In gitignored
`spike-scratch/increment-a/` (per `spike.md`), provision ≥2 per-workload netns+veth
(reuse the shipped `veth_provisioner` topology shape), each with `resolv.conf`
pointing at its own gateway; run ONE host-side listener; from *inside each netns*
emit a real `getaddrinfo`/`dig`-shape query toward that netns' gateway; confirm the
one listener receives and answers all of them on a real kernel under Lima as root.

**Acceptance Criteria (probe gate):**
- [ ] Probe runs **for real under Lima as root** (`cargo xtask lima run -- …`), NOT `--no-run` / compile-only.
- [ ] Findings record a binary verdict (WORKS / DOESN'T-WORK) with **pasted** command output (not narrated), `uname -r`, and the binding/routing shape that worked (or the wall that blocked).
- [ ] Promotion-gate decision (PROMOTE / DISCARD / PIVOT) recorded in `spike/wave-decisions.md`.
- [ ] Probe code is in `spike-scratch/`, **never** in `crates/`; eBPF (if any) is aya-rs Rust, never C.

### US-DBN-2 — Walking skeleton: responder answers ONE name → ONE running backend, end-to-end through serve+deploy

**Job:** `J-MESH-001`

**Elevator Pitch:**
- **Before:** An operator deploys a mesh workload that another workload must dial by name; the dialing workload's `getaddrinfo("<peer>.svc.overdrive.local")` reaches the injected resolver, **nothing answers**, resolution fails, and the connection never starts.
- **After:** `overdrive serve` (one node) + `overdrive deploy <server-spec>` + `overdrive deploy <client-spec>` → the client workload's `getaddrinfo("<server>.svc.overdrive.local")` resolves to the server's **running** `service_backends` addr, the client connects, and the connection is **intercepted + mTLS'd end-to-end** (TLS 1.3 `0x17` records on the peer wire) — the dial-by-name path closes through the production entry points.
- **Decision enabled:** Sam can tell a reviewer "an unmodified workload reaches its mesh peer by name and the hop is encrypted, proven through `serve` + `deploy`, not a `#[test]` seam."

**Problem:** Sam (and Ana's unmodified workload) need name resolution to actually work
in a deploy. The injection is shipped; the answer is missing.

**Solution (behavior; DESIGN owns the API):** The in-agent name-answering listener
answers `A` for `<job>.svc.overdrive.local` with the running **IPv4** backend addr by
reading `service_backends ∩ running` from the shared `ServiceBackendsResolve` index
(D-TME-11) and returning a `running` backend addr — the **same** address
`MtlsResolve.resolve` recognizes (`SocketAddrV4`, D-TME-10); it answers `AAAA` as
**NODATA** (name exists, no IPv6 record — the v1 substrate is IPv4). Thin: the
**A→B direction only** first (one name, one running backend), driven end-to-end
through `overdrive serve` + `overdrive deploy`, proven by the intercept landing.

**Domain Examples:**
1. **Happy path (A→B):** Sam deploys `server` (`server.toml`, replicas=1) and `client` (`client.toml`). `server` reaches Running with backend addr `10.x.y.2:8080`. `client`'s `getaddrinfo("server.svc.overdrive.local")` → `10.x.y.2`; `client` connects; tcpdump on the peer leg shows TLS 1.3 records.
2. **Headless single-source:** the addr the responder returns for `server.svc.overdrive.local` is **byte-identical** to the addr `MtlsResolve.resolve` recognizes for the same flow (the name answer and the intercept read the same single source — two of the one-source / **three**-readers contract) — no VIP, no translation layer.
3. **Boundary (not-yet-running):** Sam deploys `server` and *immediately* (before it reaches Running) `client` queries `server.svc.overdrive.local` → NXDOMAIN (no running backend; covered fully by US-DBN-4), never a half-provisioned or guessed addr.

**UAT Scenarios (BDD):**

```gherkin
Scenario: An unmodified workload resolves its mesh peer by name and the hop is encrypted
  Given Sam has run "overdrive serve" on a single node
  And Sam has run "overdrive deploy server.toml" and the server allocation is Running with a service_backends addr
  When Sam runs "overdrive deploy client.toml" and the client workload calls getaddrinfo("server.svc.overdrive.local")
  Then the query resolves to the server's running service_backends addr (the same addr MtlsResolve recognizes)
  And the client's subsequent connection is intercepted and the peer wire carries TLS 1.3 application_data records

Scenario: The name answer is byte-consistent with the intercept path's source
  Given a server allocation is Running with a service_backends addr A
  When the responder answers "server.svc.overdrive.local"
  Then the answer addr equals A byte-for-byte
  And no VIP and no #167 allocator is involved
```

**Acceptance Criteria:**
- [ ] Driven through production `overdrive serve` + `overdrive deploy` — NOT a hand-rolled harness. No test installs a rule / binds a socket / supplies an address production does not itself install/bind/supply (CLAUDE.md vertical-slice rule).
- [ ] A deployed workload's `getaddrinfo("<server>.svc.overdrive.local")` resolves to the server's `running` `service_backends` addr.
- [ ] The resolved addr is byte-identical to the addr `MtlsResolve.resolve` recognizes (D-TME-10 single-source).
- [ ] The subsequent connection is intercepted + mTLS'd (Tier-3 capture: TLS 1.3 `0x17`, zero payload cleartext on the peer leg).
- [ ] The resolve read goes through the shared `ServiceBackendsResolve` index (the 3rd reader, D-TME-11) — no second source of backend truth.
- [ ] `AAAA` for a name with a running (IPv4) backend returns **NODATA** (NOERROR, no IPv6 record) — NOT NXDOMAIN, NOT a fabricated v6 addr (the v1 substrate is IPv4).

### US-DBN-3 — Runnable ping-pong demo: two services dial each other by name

**Job:** `J-MESH-001`

**Elevator Pitch:**
- **Before:** There is no operator-runnable proof of dial-by-name; the behavior is only assertable inside a Tier-3 test. An operator cannot *see* the mesh resolve names and ping-pong.
- **After:** `overdrive deploy examples/dial-by-name-responder/a.toml` + `overdrive deploy examples/dial-by-name-responder/b.toml` → an observable ping-pong: A calls `b.svc.overdrive.local`, B calls `a.svc.overdrive.local`, each call **increments a counter and stamps a fresh date** on a ~10s cadence, each hop resolved through the responder then intercepted + mTLS'd.
- **Decision enabled:** Sam (or a reviewer, or a new teammate) can *watch* the mesh work by name end-to-end with two `overdrive deploy` commands — the operator-runnable proof of dial-by-name.

**Problem:** Sam needs a proof he can run with his own hands and watch advance, not a
green test he must trust. The demo cannot run until the responder answers — so it is
scoped **inside** this feature.

**Solution (behavior):** Two specs `examples/dial-by-name-responder/{a,b}.toml`
(`[service]`/`[exec]`/`[resources]`/`[[listener]]` — the schema `overdrive deploy`
accepts). A small ping-pong workload program: resolve peer by name → call on a ~10s
loop; on inbound call, increment a counter + set a fresh date + reply. `command` MUST
point at a **real on-disk binary** in the deploy env (no phantom paths). Introduces the
`examples/<feature>/` subdir convention. **Program shape DECIDED (user, 2026-06-24): a
tiny Rust bin staged into the VM** (the `coinflip-helper` precedent — clean HTTP/TCP +
counter/date), built and staged at a real on-disk `command` path before the demo runs.

**Domain Examples:**
1. **Bidirectional (A↔B):** A deploys, B deploys; within ~10s A's `getaddrinfo("b.svc.overdrive.local")` resolves and A calls B (counter `b=1`, date stamped); within ~10s B calls A (counter `a=1`). Counters advance roughly every 10s.
2. **Real binary:** `a.toml`'s `command` is a real path present in the deploy env (e.g. a `/tmp`-staged `dial-pong` helper, per the `coinflip-helper` precedent, or `/usr/bin/socat`+shell per `dns-resolver.toml`) — verified to exist before the demo runs.
3. **Port honesty:** the listener ports avoid the dev-VM collisions documented in `dns-resolver.toml` (do not bind 5353 — `systemd-resolved` owns it).

**UAT Scenarios (BDD):**

```gherkin
Scenario: Two services ping-pong by name, each hop intercepted and mTLS'd
  Given Sam has run "overdrive serve" on a single node
  When Sam runs "overdrive deploy examples/dial-by-name-responder/a.toml"
  And Sam runs "overdrive deploy examples/dial-by-name-responder/b.toml"
  Then within ~10 seconds A resolves "b.svc.overdrive.local" and calls B, B's counter increments and its date refreshes
  And within ~10 seconds B resolves "a.svc.overdrive.local" and calls A, A's counter increments and its date refreshes
  And each call's hop is intercepted and carries TLS 1.3 records on the peer wire
  And the counters continue advancing on a ~10s cadence
```

**Acceptance Criteria:**
- [ ] `examples/dial-by-name-responder/a.toml` and `b.toml` exist with the `[service]`/`[exec]`/`[resources]`/`[[listener]]` schema and `command` pointing at a real on-disk binary.
- [ ] A calls `b.svc.overdrive.local` and B calls `a.svc.overdrive.local`, each resolved through the in-agent responder.
- [ ] Each call increments a counter and stamps a fresh date; cadence ≈ 10s.
- [ ] Each hop is intercepted + mTLS'd (observable via tcpdump/`ss -tie` on the peer leg).
- [ ] The demo is driven by `overdrive deploy` (two commands) against `overdrive serve` — a real serve+deploy loop, not a `#[test]`.
- [ ] Graduated to a `verification/expectations/` EDD expectation (see Outcome KPIs / EDD).

### US-DBN-4 — Empty-candidate honesty: no running backend → honest NXDOMAIN, never a stale address

**Job:** `J-MESH-001`

**Elevator Pitch:**
- **Before:** A name query for a service with **no running backend** risks returning a stale or last-known address — pointing an unmodified workload at a dead instance, producing a confusing connection failure two layers downstream.
- **After:** `overdrive deploy <server-spec>` then querying `<server>.svc.overdrive.local` **before it reaches Running** (or after all backends stop) → **NXDOMAIN** (no running backend) — the workload sees "no such name right now," not a wrong address. (No new operator verb; the observable is the query result inside a deployed workload + the absence of a bogus connection.)
- **Decision enabled:** Sam can tell a reviewer "the name layer is fail-honest — it never points a workload at a backend that isn't running," matching the arc's fail-closed discipline.

**Problem:** A stale address is worse than no address — it sends an unmodified workload
to a dead instance. The arc's whole posture is fail-closed/fail-honest; the name layer
must match it.

**Solution (behavior):** When `service_backends ∩ running` is empty for the queried
name, the responder returns **NXDOMAIN** (per § *The v1 DNS answer contract*) — **never** a stale, cached, or
last-known address. (Mirrors the arc's "never absorb a fallible read into a default"
discipline and the K8s-headless/Fly `.internal` empty-endpoint-set shape.)

**Domain Examples:**
1. **No backend yet:** `server` deployed but Pending; query `server.svc.overdrive.local` → NXDOMAIN; the workload retries later and succeeds once `server` is Running.
2. **All backends stopped:** `server` was Running, then `overdrive job stop` removes it; a subsequent query → NXDOMAIN, NOT the old `10.x.y.2`.
3. **Unknown name:** query `nonexistent.svc.overdrive.local` → NXDOMAIN.

**UAT Scenarios (BDD):**

```gherkin
Scenario: No running backend yields NXDOMAIN, never a stale address
  Given a "server" service has been deployed but has no running backend
  When a workload queries "server.svc.overdrive.local"
  Then the responder returns NXDOMAIN
  And no previously-known or guessed address is returned

Scenario: A name that drops all backends stops resolving
  Given "server" was Running and resolved to addr A
  When all of "server"'s backends stop
  And a workload queries "server.svc.overdrive.local"
  Then the responder returns NXDOMAIN
  And it never returns the stale addr A
```

**Acceptance Criteria:**
- [ ] Empty `running` candidate set → **NXDOMAIN** (never a stale/cached/guessed addr).
- [ ] After all backends stop, the name stops resolving (no stale addr).
- [ ] Unknown name → NXDOMAIN.
- [ ] Proven through a deployed workload's query against `overdrive serve` + `overdrive deploy` (Tier-3), consistent with the resolve-index `running` filter — no second source of liveness truth.

---

## `[REF]` System Constraints (cross-cutting)

- **Single-node, Phase 2.** No multi-node, no cross-node name resolution. One node's workloads.
- **Headless only.** No VIP, no `fdc2::/16`, no XDP `SERVICE_MAP`, no #167/#61 dependency (D-TME-10).
- **IPv4 substrate (v1).** The resolve/intercept addr is `SocketAddrV4`; the responder + netns addrs are `Ipv4Addr`. `A` answers carry the running IPv4 backend; `AAAA` answers are **NODATA**. A real IPv6 story (widening the substrate) is out of v1 scope.
- **In-agent, userspace.** Same process as the agent-light L4 proxy + the `ServiceBackendsResolve` index. NOT a separate daemon, NOT in-kernel (D-TME-11 / arc reframe).
- **One source, three readers.** The responder is the THIRD reader of `ServiceBackendsResolve` (outbound resolve + inbound install + name answers). No second source of backend truth.
- **Implement-to-design.** Behavior + pinned contracts only; the responder API surface, the listener type, and the resolve accessor signatures are DESIGN-wave decisions. Surface gaps as blockers, never improvise API (CLAUDE.md).
- **Vertical slices through production entry points.** Every slice closes a real loop through `overdrive serve` + `overdrive deploy`. No slice ships if it only composes in a `#[test]` (CLAUDE.md).

---

## `[REF]` Definition of Done

- US-DBN-1 spike verdict + promotion-gate decision recorded; the one-listener-many-netns assumption is PROMOTED (or the design pivots before the walking skeleton).
- US-DBN-2 walking skeleton: a deployed workload resolves ONE peer name → ONE running backend through `serve` + `deploy`, proven by the intercept landing.
- US-DBN-3 ping-pong demo: `examples/dial-by-name-responder/{a,b}.toml` + program; two `overdrive deploy`s produce an observable advancing counter/date, each hop mTLS'd; graduated to EDD.
- US-DBN-4 empty-candidate honesty: no running backend → NXDOMAIN, never stale; proven Tier-3 through a deployed workload.
- The resolve index is a verified "one source, three readers" — the name layer is byte-consistent with the intercept path.
- All four DoR-passing stories trace to J-MESH-001.

---

## `[REF]` Out of scope (cite existing issues only)

- **VIP path** (`<job>.svc.overdrive.local → fdc2::/16` VIP + XDP `SERVICE_MAP`) — **#61** (depends on #167). D-TME-10 headless choice avoids it.
- **Backend addressing / inbound install** — **#241** (the production inbound nft-TPROXY rule's `virt` source; the leg-C listener + accept loop ARE production).
- **Expected-SVID / intended-peer pinning** — **#178** (split → #242). v1 is authn-only; the responder returns an addr, not an expected identity.
- **Cross-node / multi-node name resolution, gossiped name state** — OUT of Phase-2 single-node scope. No forward-pointer issue (#36 is generic node enrollment, not this).
- **The agent-light kTLS enforcement substrate itself** — shipped by the transparent-mtls arc (#26/#236); this feature is a READER of the resolve index + a name-answerer, never an enforcer. MUST NOT duplicate `MtlsResolve` / `MtlsEnforcement`.

---

## `[REF]` Walking-skeleton strategy

**The thinnest serve+deploy loop:** `overdrive serve` (one node) + `overdrive deploy
<server-spec>` (reaches Running, gets a `service_backends` addr) + `overdrive deploy
<client-spec>` whose workload does `getaddrinfo("<server>.svc.overdrive.local")` →
resolves to the server's running backend addr → connects → the existing intercept path
mTLS's the hop. **A→B direction only** (one name, one running backend) — the thinnest
slice that closes a real dial-by-name loop through production entry points and is
proven by the intercept landing. The bidirectional ping-pong (US-DBN-3) and
empty-candidate honesty (US-DBN-4) build outward from this spine.

**Gated by Slice 00 (the spike).** The skeleton cannot be designed until the
one-listener-many-netns routing assumption is validated — that probe is the BLOCKING
first slice (`spike.md`).

---

## `[REF]` Driving ports (for DESIGN — named, not designed)

- The **name-answering listener** that receives `getaddrinfo` queries on each per-netns gateway addr (the new surface; its concrete type/shape is a DESIGN decision).
- The shared **`ServiceBackendsResolve`** read surface (`subscribe_all_events()`, `all_service_backends_rows()`) — EXISTS; this feature adds the third reader.
- **`overdrive serve`** (composition root / `run_server`) and **`overdrive deploy <SPEC>`** — the production entry points every slice drives through.

---

## `[REF]` Pre-requisites

- **SHIPPED:** `resolv.conf` injection (D-TME-9), the `ServiceBackendsResolve` index (D-TME-11, `01-03`), the `MtlsResolve` consumer + intercept path (transparent-mtls arc).
- **BLOCKING (Slice 00):** the one-listener-many-netns routing assumption (spike, no Tier-2 backstop).
- **For the demo (US-DBN-3):** the ping-pong program — DECIDED (2026-06-24) as a tiny Rust bin staged into the VM (the `coinflip-helper` precedent) — built and staged at a real on-disk `command` path in the deploy env.

---

## `[REF]` Shared-artifact registry

Registry-grade tracking of every value that must be single-source and byte-consistent
across the name layer and the intercept path — source of truth, consumers, owner,
integration risk, validation. (Addresses `review-discuss.md` High #3.)

| Artifact | Source of truth | Consumers | Owner | Integration risk | Validation |
|---|---|---|---|---|---|
| `service_backends_running_set` | The shared `ServiceBackendsResolve` index (`service_backends ∩ running`, D-TME-11) — in-RAM, ownership-aware, List-then-Watch + relist-on-`Lagged` | outbound resolve · inbound install · **name answers (this feature)** | transparent-mtls arc owns the index; this feature is the 3rd **reader** | a second source of backend truth (a cache, a stale snapshot) would let the name layer drift from the intercept path | K-DBN-1 / K-DBN-4 (Tier-3): name answers drawn ONLY from this index's running rows; no separate cache |
| `answered_backend_addr` | the `running` `service_backends` row (`SocketAddrV4`, headless D-TME-10) | the workload's `getaddrinfo` + `connect`; then `MtlsResolve.resolve` (the intercept) | this feature (the responder reads + answers) | a non-byte-identical addr vs what `MtlsResolve` recognizes → the resolved peer is not the intercepted peer | K-DBN-4 single-source oracle: feed the answered addr into `resolve`; assert byte-equality |
| `responder_addr` | `WorkloadNetnsPlan.host_addr` (per-netns gateway, `Ipv4Addr`, D-TME-9) — written to `resolv.conf` by `veth_provisioner.rs` `WriteResolvConf` (SHIPPED) | the workload's stub resolver (the `nameserver` it queries); the in-agent listener (the addr it must answer on, for EVERY netns) | transparent-mtls arc (injection shipped); this feature answers on it | one listener may NOT be able to answer on N per-netns gateway addrs (the load-bearing unvalidated routing assumption) | **Slice 00 (the spike)** — real-kernel one-listener-many-netns probe; BLOCKING |
| `mesh_dns_name` | the `<job>.svc.overdrive.local` grammar (job name ← the deploy spec `[service].id`) | the workload's query; the responder's name→backend lookup | this feature (the responder parses + matches the suffix) | name-grammar drift (suffix, case, label limits) vs what workloads dial | US-DBN-2 / US-DBN-4 ACs: `getaddrinfo("<server>.svc.overdrive.local")` resolves; unknown name → NXDOMAIN |
| `ping_pong_command_path` | the staged tiny Rust ping-pong bin's on-disk path in the deploy env (decided 2026-06-24); referenced by `examples/dial-by-name-responder/{a,b}.toml` `[exec].command` | `overdrive deploy` (the two specs); the workloads at runtime | this feature (the demo) | a phantom `command` path → the alloc never reaches Running → the demo silently can't run (the `dns-resolver.toml` collision class) | US-DBN-3 AC: `command` points at a real on-disk binary present in the deploy env, verified before the demo runs |
| `edd_ping_pong_evidence` | the `verification/expectations/` capture of the demo (proposed `E05-dial-by-name-ping-pong-mtls`), black-box against the built `overdrive` binary under Lima | EDD different-fox review; the operator-surface proof (K-DBN-3) | this feature (the EDD expectation) | a fabricated / narrated capture (forbidden by `verification.md`); a stale capture vs current HEAD | honest `pending` until the full-system EDD harness (#227/#75) lands (mirrors E04); captured + different-fox-reviewed, never self-stamped |

---

## `[REF]` Outcome KPIs (numeric targets + measurement method)

| KPI | Who | Does what (behavior change) | By how much (target) | Measured by | Baseline |
|---|---|---|---|---|---|
| **K-DBN-1 — name resolves to a live backend** | A deployed mesh workload | `getaddrinfo("<peer>.svc.overdrive.local")` resolves to a **running** backend addr | **100%** of queries where ≥1 backend is Running resolve to a running backend addr (0 stale, 0 timeout) across the Tier-3 acceptance matrix | Tier-3 test: query from inside a deployed workload's netns; assert resolved addr ∈ running `service_backends`; assert byte-equal to the `MtlsResolve`-recognized addr | Today: **0%** — nothing answers; resolution fails in every deploy |
| **K-DBN-2 — empty-candidate honesty** | A deployed workload querying a name with no running backend | Receives **NXDOMAIN** (no running backend) | **0** stale/guessed addresses returned across the no-backend / all-stopped / unknown-name cases; **100%** NXDOMAIN | Tier-3: deploy-then-query-before-Running, stop-then-query, unknown-name — assert NXDOMAIN, assert never the prior addr | Today: N/A (no answer at all) — target is "honest NXDOMAIN," not "wrong addr" |
| **K-DBN-3 — operator-runnable ping-pong advances** | Sam (operator) | Watches the demo advance via two `overdrive deploy`s | Both counters increment **≥1** within **~15s** of the second deploy and continue advancing on a **~10s ±5s** cadence over a 60s observation window | EDD expectation capture: deploy a.toml + b.toml, observe counter/date advancing in the workload output/logs; different-fox adversarial review of captured evidence | Today: **0** — demo cannot run (responder absent) |
| **K-DBN-4 — single-source consistency** | The mesh data plane | Name answers match the intercept path's backend truth | **100%** of name-answered addrs are byte-identical to the addr `MtlsResolve.resolve` recognizes for the same flow (one source, three readers) | Tier-3 single-source oracle: feed the responder's answered addr into `resolve`; assert equality (the 05-02 single-source discipline) | Today: N/A — no name answers exist |

**EDD graduation (per `.claude/rules/verification.md`):** US-DBN-3's ping-pong is the
operator-surface proof → graduates to a `verification/expectations/` `O`/`E`-surface
expectation (proposed id, e.g. `E05-dial-by-name-ping-pong-mtls`), anchored to the
US-DBN-3 walking-skeleton scenario + the K-DBN-3 KPI, captured black-box against the
built `overdrive` binary under Lima, different-fox-reviewed. (Like sibling `E04`, the
capture may be `pending` until the full-system EDD harness #227/#75 lands — surface
that as a dependency, mirror E04's honest `pending` posture.)

---

## `[REF]` DoR validation (9-item hard gate)

| # | Item | Status | Evidence |
|---|---|---|---|
| 1 | Problem statement clear, domain language | ✅ | Each story opens from user pain (Sam/Ana can't reach a peer by name; stale-addr hazard) in mesh/reachability vocabulary |
| 2 | User/persona with specific characteristics | ✅ | Sam (`sam-platform-security-engineer.yaml`), reachability lens; Ana as relates-to actor |
| 3 | 3+ domain examples with real data | ✅ | Each story carries 3 examples with concrete names (`server`/`client`, `a`/`b`), addrs (`10.x.y.2:8080`), names (`b.svc.overdrive.local`), real binaries (`/usr/bin/socat`, `/tmp`-staged helper) |
| 4 | UAT in Given/When/Then (3–7 scenarios) | ✅ | 7 scenarios across US-DBN-2/3/4 (US-DBN-1 is a spike → probe-gate ACs) |
| 5 | AC derived from UAT | ✅ | Each story's AC list maps to its scenarios |
| 6 | Right-sized (1–3 days, 3–7 scenarios) | ✅ | 4 slices, each a single behavior; ping-pong demo is the largest and is still one deliverable. See Scope Assessment in wave-decisions |
| 7 | Technical notes: constraints/dependencies | ✅ | System Constraints + Pre-requisites + Driving Ports sections; pinned contracts D-TME-9/10/11 |
| 8 | Dependencies resolved or tracked | ⚠️ | Resolved: D-TME-9/10/11, resolve index, intercept path; ping-pong program shape (tiny Rust bin, decided 2026-06-24). **Tracked/blocking: the spike (Slice 00)**; the EDD-harness #227/#75 (for full black-box capture) |
| 9 | Outcome KPIs with measurable targets | ✅ | K-DBN-1..4 with numeric targets + measurement method + baseline |

**Gate verdict:** **DISCUSS approved to run / design Slice 00 (the spike) ONLY.**
DoR is met for the spike, but **full responder DESIGN is BLOCKED until Slice 00
records `PROMOTE`** — if it records `PIVOT` / `DISCARD`, the DISCUSS artifacts are
revised before continuing. The spike is an in-feature *blocking* dependency, not a
parallel track; do **not** describe full DESIGN as ready until the spike result
exists. (Item 8: the spike is blocking; the ping-pong-program shape is decided — a
staged Rust bin; the EDD-harness dependency mirrors E04's honest `pending`.) No
invented issues. (Addresses `review-discuss.md` Blocking #2.)

---

## `[REF]` Wave-Decisions (DISCUSS)

### Decisions taken
1. **Job:** MINT `J-MESH-001` (reachability/mesh family, `relates_to: [J-SEC-003]`) — distinct progress + failure mode vs J-SEC-003 (J-SEC-002 mint precedent, NOT the udp-sendmsg4 elevation precedent).
2. **Journey:** PROPOSE a new journey `dial-a-mesh-peer-by-name.yaml` — do NOT extend `enforce-transparent-mtls-on-the-wire.yaml` (pure enforcement, no operator verb; this leg HAS an operator-observable surface via the demo).
3. **Scope:** the ping-pong demo is IN scope (operator-runnable proof, cannot run until the responder answers → scoped inside this feature, not built standalone).
4. **Slicing:** 4 slices (00 spike → 01 walking skeleton A→B → 02 bidirectional ping-pong → 03 empty-candidate honesty). Spike-first per `spike.md`.
5. **Ping-pong program shape (user, 2026-06-24):** a tiny **Rust bin staged into the VM** (the `coinflip-helper` precedent — clean HTTP/TCP + counter/date), NOT a shell+`curl`/`socat` loop — built and staged at a real on-disk `command` path before the demo runs.

### Scope Assessment: PASS — 4 stories, 1–2 modules (the in-agent responder + the `examples/` demo), estimated ~4–6 days incl. spike
- Stories: 4 (≤10 ✅). Bounded contexts/modules: the in-agent name responder reading the existing resolve index (1 new surface) + the demo (`examples/` + a small program) (≤3 ✅). Walking skeleton integration points: serve + deploy + the resolve index (≤5 ✅). Multiple independent outcomes that could ship separately? No — all serve the single dial-by-name reachability outcome. **Right-sized; no split needed.**

### Applied SSOT diffs (user-approved 2026-06-24)
- **`docs/product/jobs.yaml`:** ✅ APPLIED — `J-MESH-001` added (after J-SEC-003) + a changelog entry dated 2026-06-24 recording the mint + the mint-vs-elevate justification.
- **`docs/product/journeys/dial-a-mesh-peer-by-name.yaml`:** ✅ APPLIED — NEW journey file written (the draft below is the source).
- **`docs/product/personas/sam-platform-security-engineer.yaml`:** NOT modified (per user) — Ana referenced inline only.

### Journey written to `docs/product/journeys/dial-a-mesh-peer-by-name.yaml` (source draft below; the written file expands this to the sibling-journey shape)

```yaml
journey:
  name: "Dial a mesh peer by name and land at a live instance"
  goal: >
    An unmodified, identity-unaware workload calls
    getaddrinfo("<job>.svc.overdrive.local") and connects; an in-agent
    name-answering listener (sharing the ServiceBackendsResolve index, D-TME-11)
    answers A with a running IPv4 service_backends addr (headless, D-TME-10) — the
    same addr MtlsResolve recognizes (SocketAddrV4) — and AAAA as NODATA (v1 substrate
    is IPv4), or NXDOMAIN when no backend is running (declared-but-empty and unknown
    collapse in v1); the existing intercept path then mTLS's the hop. Operator-observable
    via the dial-by-name-responder ping-pong demo.
  persona: >
    Sam (sam-platform-security-engineer.yaml) — reachability lens. Verifies an
    unmodified workload reaches its mesh peer by name and lands at a live instance,
    and that the name layer never returns a stale address.
  emotional_arc:
    start: "Skeptical — 'the wire is enforceable, but can an unmodified workload even FIND its peer by name in a deploy, and will the platform ever hand it a dead address?'"
    middle: "Reassured — the walking skeleton resolves one name to one running backend through serve+deploy and the hop goes TLS 1.3; the ping-pong demo advances counters he watches himself."
    end: "Confident — 'an ordinary workload dials by name and lands at a LIVE peer the mesh then mTLS's, and a no-backend query returns NOTHING, never a stale addr. I ran the proof myself with two overdrive deploys.'"
  steps:
    - id: 1
      name: "An unmodified workload resolves its peer by name to a running backend"
      command: "(workload getaddrinfo; in-agent responder answers from service_backends ∩ running via the shared resolve index)"
    - id: 2
      name: "The resolved addr is the one the intercept path recognizes (headless, one source)"
      command: "(internal — D-TME-10 single-source; no VIP)"
    - id: 3
      name: "The ping-pong demo advances — two services dial each other by name, each hop mTLS'd"
      command: "overdrive deploy examples/dial-by-name-responder/a.toml + b.toml"
  error_paths:
    - step: 1
      failure: "No running backend for the queried name"
      recovery: "NXDOMAIN (no running backend; declared-but-empty and unknown collapse in v1) — never a stale/cached/guessed addr (mirrors the arc's fail-honest discipline)."
  related_jobs: [J-MESH-001]
  related_features:
    - id: dial-by-name-responder
      role: "Ships the in-agent name responder (GH #243). The reachability leg of the mesh; reads the ServiceBackendsResolve index as the third reader."
    - id: transparent-mtls-enrollment
      role: "Ships the intercept + MtlsResolve + resolve index this journey's name answers feed into (one source, three readers)."
```

### Anti-pattern scan (clean)
- No "Implement X" stories (all open from user pain). ✓
- No generic data (real names: Sam, Ana, server/client, a/b, `b.svc.overdrive.local`, `/usr/bin/socat`). ✓
- No technical AC / technical scenario titles (outcomes: "resolves to a running backend," "ping-pong advances," "honest NXDOMAIN"). ✓
- No oversized story (each ≤7 scenarios, single behavior). ✓
- No abstract requirements without examples (3+ per story). ✓

### Risks / notes
- **No DIVERGE artifacts** for this feature (`docs/feature/dial-by-name-responder/diverge/` absent) — consistent with the jobs.yaml header precedent (JTBD distilled from issue/arc, not interviews). Noted as a non-blocking risk; the parent arc's DISCUSS/DESIGN/spike history grounds the contracts.
- The EDD ping-pong capture may be `pending` until #227/#75 (full-system Lima EDD harness) — mirror E04's honest `pending` posture rather than fabricate a capture.
