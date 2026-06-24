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
      with the running-and-healthy backend's IPv4 address read as a sibling
      name-keyed reader over the SAME service_backends rows MtlsResolve folds
      (NOT the addr-keyed intercept-index struct; refined by DESIGN ADR-0072) (the
      v1 substrate is IPv4 — the MtlsResolve addr is SocketAddrV4, the per-netns
      responder addr is Ipv4Addr), returning the SAME address MtlsResolve
      recognizes (headless, no VIP, D-TME-10); answer AAAA as NODATA (the name is
      currently resolvable, no IPv6 record — a real IPv6 backend story is out of
      v1 scope); and NXDOMAIN when no backend is running-and-healthy (declared-but-empty and
      unknown are indistinguishable — the responder reads the running-and-healthy resolve
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
| **D-TME-11** | Read mechanism = a **sibling name-keyed reader over the SAME `service_backends` rows** (same `ObservationStore`, same List-then-Watch + relist-on-`Lagged` pattern as `ServiceBackendsResolve`) — **NOT** the addr-keyed intercept-index *struct*. This feature makes the `ObservationStore` `service_backends` **surface** **"one source, THREE readers"** (outbound resolve + inbound install + **name answers**). Byte-consistency = same rows, not a shared struct (DESIGN DDN-1; the security-critical `ServiceBackendsResolve` struct stays untouched). | **Surface exists; this feature adds the 3rd reader (a sibling, not a widening)** |
| **v1 address family** | The shipped resolve/intercept substrate is **IPv4**: `ResolvedBackend.addr` is `SocketAddrV4` (`mtls_resolve.rs`), and `responder_addr` + the netns addrs are `Ipv4Addr` (`veth_provisioner.rs`). **v1 answers `A` with the running-AND-healthy IPv4 backend addr; answers `AAAA` as NODATA** when the name is currently resolvable; **a name with 0 running-and-healthy backends → NXDOMAIN** (declared-but-not-running, unhealthy, and unknown all collapse — the responder reads only the running-AND-healthy set; DESIGN DDN-2). See § *The v1 DNS answer contract* above for the canonical table. A real IPv6 backend story (widening the `SocketAddrV4`/`Ipv4Addr` substrate) is **OUT of v1 scope**. | **Pinned (forced by the shipped types)** |
| **Implement-to-design** | This feature describes **behavior + the pinned contracts**. It does NOT invent the responder's API surface, new public types, or the `MtlsResolve` shape — those are DESIGN-wave decisions. | **Hard constraint (CLAUDE.md)** |

**Today's gap (the thing this feature closes):** nothing answers on `responder_addr`.
`getaddrinfo` reaches an injected resolver with no responder behind it → name
resolution fails in a deploy → an unmodified workload cannot initiate a by-name
connection at all.

### `[REF]` The v1 DNS answer contract (canonical — every artifact matches this)

| Query | Name has ≥1 running-AND-healthy IPv4 backend | Name has 0 running-and-healthy backends\* |
|---|---|---|
| `A` | **NOERROR + A** (the running-and-healthy IPv4 addr) | **NXDOMAIN** |
| `AAAA` | **NOERROR / NODATA** (resolvable name, no IPv6 record — v1 is IPv4) | **NXDOMAIN** |

\* *0 running-and-healthy backends covers declared-but-not-running, unhealthy /
not-ready, AND unknown names alike — v1 does **not** distinguish them, because the
responder reads only the **running-AND-healthy** set (the `by_name` index gates on
`Backend.healthy == true`, matching the intercept's `Mesh` set — DESIGN DDN-2), so
all collapse to **NXDOMAIN**. A stale / cached / guessed / unhealthy address is
**never** returned. The NXDOMAIN carries a short negative-TTL so a retrying dialer
re-resolves promptly once a backend reaches running-and-healthy (DESIGN pins the
exact TTL — 1 s, DDN-8). A future refinement could return NODATA for a
declared-but-empty service IF the responder gains a declared-service view distinct
from the running-and-healthy index — that is **not** v1.*

- **NODATA** = the name **is** currently resolvable (≥1 running-and-healthy
  backend) but has no record of the queried type (only `AAAA` in v1, since the
  substrate is IPv4).
- **NXDOMAIN** = no currently-resolvable (running-and-healthy) backend for the name.

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

### US-DBN-2 — Walking skeleton: responder answers ONE name → ONE running-and-healthy backend, end-to-end through serve+deploy

**Job:** `J-MESH-001`

**Elevator Pitch:**
- **Before:** An operator deploys a mesh workload that another workload must dial by name; the dialing workload's `getaddrinfo("<peer>.svc.overdrive.local")` reaches the injected resolver, **nothing answers**, resolution fails, and the connection never starts.
- **After:** `overdrive serve` (one node) + `overdrive deploy <server-spec>` + `overdrive deploy <client-spec>` → the client workload's `getaddrinfo("<server>.svc.overdrive.local")` resolves to the server's **running-and-healthy** `service_backends` addr, the client connects, and the connection is **intercepted + mTLS'd end-to-end** (TLS 1.3 `0x17` records on the peer wire) — the dial-by-name path closes through the production entry points.
- **Decision enabled:** Sam can tell a reviewer "an unmodified workload reaches its mesh peer by name and the hop is encrypted, proven through `serve` + `deploy`, not a `#[test]` seam."

**Problem:** Sam (and Ana's unmodified workload) need name resolution to actually work
in a deploy. The injection is shipped; the answer is missing.

**Solution (behavior; DESIGN owns the API):** The in-agent name-answering listener
answers `A` for `<job>.svc.overdrive.local` with the **running-AND-healthy IPv4**
backend addr by reading `service_backends ∩ running-and-healthy` as a **sibling
name-keyed reader over the SAME `service_backends` rows** (same `ObservationStore`,
same List-then-Watch pattern as `ServiceBackendsResolve`, D-TME-11) — NOT the
addr-keyed intercept-index struct — and returning a running-and-healthy backend addr
— the **same** address `MtlsResolve.resolve` recognizes and classifies `Mesh`
(`SocketAddrV4`, D-TME-10); it answers `AAAA` as **NODATA** (name exists, no IPv6
record — the v1 substrate is IPv4). Thin: the **A→B direction only** first (one
name, one running-and-healthy backend), driven end-to-end through `overdrive serve`
+ `overdrive deploy`, proven by the intercept landing.

**Domain Examples:**
1. **Happy path (A→B):** Sam deploys `server` (`server.toml`, replicas=1) and `client` (`client.toml`). `server` reaches running-and-healthy with backend addr `10.x.y.2:8080`. `client`'s `getaddrinfo("server.svc.overdrive.local")` → `10.x.y.2`; `client` connects; tcpdump on the peer leg shows TLS 1.3 records.
2. **Headless single-source:** the addr the responder returns for `server.svc.overdrive.local` is **byte-identical** to the addr `MtlsResolve.resolve` recognizes for the same flow (the name answer and the intercept read the same single source — two of the one-source / **three**-readers contract) — no VIP, no translation layer.
3. **Boundary (not-yet-running / not-yet-healthy):** Sam deploys `server` and *immediately* (before it reaches running-and-healthy) `client` queries `server.svc.overdrive.local` → NXDOMAIN (no running-and-healthy backend; covered fully by US-DBN-4), never a half-provisioned, unhealthy, or guessed addr.

**UAT Scenarios (BDD):**

```gherkin
Scenario: An unmodified workload resolves its mesh peer by name and the hop is encrypted
  Given Sam has run "overdrive serve" on a single node
  And Sam has run "overdrive deploy server.toml" and the server allocation is Running-AND-HEALTHY with a service_backends addr
  When Sam runs "overdrive deploy client.toml" and the client workload calls getaddrinfo("server.svc.overdrive.local")
  Then the query resolves to the server's running-and-healthy service_backends addr (the same addr MtlsResolve recognizes)
  And the client's subsequent connection is intercepted and the peer wire carries TLS 1.3 application_data records

Scenario: The name answer is byte-consistent with the intercept path's source
  Given a server allocation is Running-AND-HEALTHY with a service_backends addr A classified Mesh
  When the responder answers "server.svc.overdrive.local"
  Then the answer addr equals A byte-for-byte
  And no VIP and no #167 allocator is involved
```

**Acceptance Criteria:**
- [ ] Driven through production `overdrive serve` + `overdrive deploy` — NOT a hand-rolled harness. No test installs a rule / binds a socket / supplies an address production does not itself install/bind/supply (CLAUDE.md vertical-slice rule).
- [ ] A deployed workload's `getaddrinfo("<server>.svc.overdrive.local")` resolves to the server's `running`-and-healthy `service_backends` addr.
- [ ] The resolved addr is byte-identical to the addr `MtlsResolve.resolve` recognizes AND classifies `Mesh` (D-TME-10 single-source; an unhealthy addr would classify `MeshUnreachable`, so it is never answered).
- [ ] The subsequent connection is intercepted + mTLS'd (Tier-3 capture: TLS 1.3 `0x17`, zero payload cleartext on the peer leg).
- [ ] The resolve read is a **sibling name-keyed reader over the SAME `service_backends` rows** (the 3rd reader of the `ObservationStore` surface, D-TME-11) — no second source of backend truth, and the addr-keyed intercept-index struct is untouched.
- [ ] `AAAA` for a name with a running-and-healthy (IPv4) backend returns **NODATA** (NOERROR, no IPv6 record) — NOT NXDOMAIN, NOT a fabricated v6 addr (the v1 substrate is IPv4).

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

### US-DBN-4 — Empty-candidate honesty: no running-and-healthy backend → honest NXDOMAIN, never a stale address

**Job:** `J-MESH-001`

**Elevator Pitch:**
- **Before:** A name query for a service with **no running-and-healthy backend** risks returning a stale or last-known address — pointing an unmodified workload at a dead instance, producing a confusing connection failure two layers downstream.
- **After:** `overdrive deploy <server-spec>` then querying `<server>.svc.overdrive.local` **before it reaches running-and-healthy** (or after all backends stop) → **NXDOMAIN** (no running-and-healthy backend) — the workload sees "no such name right now," not a wrong address. (No new operator verb; the observable is the query result inside a deployed workload + the absence of a bogus connection.)
- **Decision enabled:** Sam can tell a reviewer "the name layer is fail-honest — it never points a workload at a backend that isn't running-and-healthy," matching the arc's fail-closed discipline.

**Problem:** A stale address is worse than no address — it sends an unmodified workload
to a dead instance. The arc's whole posture is fail-closed/fail-honest; the name layer
must match it.

**Solution (behavior):** When `service_backends ∩ running-and-healthy` is empty for
the queried name (no backend running-and-healthy, OR all backends unhealthy / not-ready, OR
unknown name), the responder returns **NXDOMAIN** (per § *The v1 DNS answer
contract*) — **never** a stale, cached, unhealthy, or last-known address. (Mirrors
the arc's "never absorb a fallible read into a default" discipline and the
K8s-headless/Fly `.internal` empty-endpoint-set shape.)

**Domain Examples:**
1. **No backend yet:** `server` deployed but Pending; query `server.svc.overdrive.local` → NXDOMAIN; the workload retries later and succeeds once `server` is running-and-healthy.
2. **All backends stopped:** `server` was running-and-healthy, then `overdrive job stop` removes it; a subsequent query → NXDOMAIN, NOT the old `10.x.y.2`.
3. **Unknown name:** query `nonexistent.svc.overdrive.local` → NXDOMAIN.

**UAT Scenarios (BDD):**

```gherkin
Scenario: No running-and-healthy backend yields NXDOMAIN, never a stale address
  Given a "server" service has been deployed but has no running-and-healthy backend
  When a workload queries "server.svc.overdrive.local"
  Then the responder returns NXDOMAIN
  And no previously-known or guessed address is returned

Scenario: A name that drops all backends stops resolving
  Given "server" was running-and-healthy and resolved to addr A
  When all of "server"'s backends stop
  And a workload queries "server.svc.overdrive.local"
  Then the responder returns NXDOMAIN
  And it never returns the stale addr A
```

**Acceptance Criteria:**
- [ ] Empty `running-and-healthy` candidate set → **NXDOMAIN** (never a stale/cached/unhealthy/guessed addr).
- [ ] After all backends stop (or go unhealthy), the name stops resolving (no stale addr).
- [ ] Unknown name → NXDOMAIN.
- [ ] Proven through a deployed workload's query against `overdrive serve` + `overdrive deploy` (Tier-3), consistent with the index's `running-and-healthy` filter — no second source of liveness truth.

---

## `[REF]` System Constraints (cross-cutting)

- **Single-node, Phase 2.** No multi-node, no cross-node name resolution. One node's workloads.
- **Headless only.** No VIP, no `fdc2::/16`, no XDP `SERVICE_MAP`, no #167/#61 dependency (D-TME-10).
- **IPv4 substrate (v1).** The resolve/intercept addr is `SocketAddrV4`; the responder + netns addrs are `Ipv4Addr`. `A` answers carry the running-AND-healthy IPv4 backend; `AAAA` answers are **NODATA**. A real IPv6 story (widening the substrate) is out of v1 scope.
- **In-agent, userspace.** Same process as the agent-light L4 proxy + the `ServiceBackendsResolve` index. NOT a separate daemon, NOT in-kernel (D-TME-11 / arc reframe).
- **One source, three readers.** The responder is the THIRD reader of the `ObservationStore` `service_backends` **surface** — a sibling name-keyed reader over the SAME rows (outbound resolve + inbound install + name answers), NOT a widening of the addr-keyed `ServiceBackendsResolve` intercept-index struct. No second source of backend truth.
- **Implement-to-design.** Behavior + pinned contracts only; the responder API surface, the listener type, and the resolve accessor signatures are DESIGN-wave decisions. Surface gaps as blockers, never improvise API (CLAUDE.md).
- **Vertical slices through production entry points.** Every slice closes a real loop through `overdrive serve` + `overdrive deploy`. No slice ships if it only composes in a `#[test]` (CLAUDE.md).

---

## `[REF]` Definition of Done

- US-DBN-1 spike verdict + promotion-gate decision recorded; the one-listener-many-netns assumption is PROMOTED (or the design pivots before the walking skeleton).
- US-DBN-2 walking skeleton: a deployed workload resolves ONE peer name → ONE running-and-healthy backend through `serve` + `deploy`, proven by the intercept landing.
- US-DBN-3 ping-pong demo: `examples/dial-by-name-responder/{a,b}.toml` + program; two `overdrive deploy`s produce an observable advancing counter/date, each hop mTLS'd; graduated to EDD.
- US-DBN-4 empty-candidate honesty: no running-and-healthy backend → NXDOMAIN, never stale; proven Tier-3 through a deployed workload.
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
<server-spec>` (reaches running-and-healthy, gets a `service_backends` addr) + `overdrive deploy
<client-spec>` whose workload does `getaddrinfo("<server>.svc.overdrive.local")` →
resolves to the server's running-and-healthy backend addr → connects → the existing intercept path
mTLS's the hop. **A→B direction only** (one name, one running-and-healthy backend) — the thinnest
slice that closes a real dial-by-name loop through production entry points and is
proven by the intercept landing. The bidirectional ping-pong (US-DBN-3) and
empty-candidate honesty (US-DBN-4) build outward from this spine.

**Gated by Slice 00 (the spike).** The skeleton cannot be designed until the
one-listener-many-netns routing assumption is validated — that probe is the BLOCKING
first slice (`spike.md`).

---

## `[REF]` Driving ports (for DESIGN — named, not designed)

- The **name-answering listener** that receives `getaddrinfo` queries on each per-netns gateway addr (the new surface; its concrete type/shape is a DESIGN decision).
- The shared **`ObservationStore` `service_backends`** read surface (`subscribe_all_events()`, `all_service_backends_rows()`, the same surface `ServiceBackendsResolve` reads) — EXISTS; this feature adds a third **sibling** reader over the SAME rows, NOT a widening of the addr-keyed intercept-index struct.
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
| `service_backends_running_set` | The `ObservationStore` `service_backends` **surface** (`service_backends ∩ running-AND-healthy`, D-TME-11) — the SAME rows `ServiceBackendsResolve` folds, read via the same List-then-Watch + relist-on-`Lagged` pattern; the responder keeps its OWN name-keyed `by_name` index (a sibling reader, NOT the addr-keyed struct) | outbound resolve · inbound install · **name answers (this feature)** | transparent-mtls arc owns the addr-keyed struct; this feature is a 3rd sibling **reader** over the same rows | a second source of backend truth (a cache, a stale snapshot), OR widening the addr-keyed intercept struct, would let the name layer drift from / couple into the security path | K-DBN-1 / K-DBN-4 (Tier-3): name answers drawn ONLY from the surface's running-and-healthy rows; no separate cache; `mtls_resolve_adapter.rs` untouched |
| `answered_backend_addr` | the `running`-AND-healthy `service_backends` row (`SocketAddrV4`, headless D-TME-10) | the workload's `getaddrinfo` + `connect`; then `MtlsResolve.resolve` (the intercept) | this feature (the responder reads + answers) | a non-byte-identical addr, OR an unhealthy addr (→ `MeshUnreachable`), vs what `MtlsResolve` recognizes as `Mesh` → the resolved peer is not the intercepted peer | K-DBN-4 single-source oracle: feed the answered addr into `resolve`; assert byte-equality AND `Mesh` classification |
| `responder_addr` | `WorkloadNetnsPlan.host_addr` (per-netns gateway, `Ipv4Addr`, D-TME-9) — written to `resolv.conf` by `veth_provisioner.rs` `WriteResolvConf` (SHIPPED) | the workload's stub resolver (the `nameserver` it queries); the in-agent listener (the addr it must answer on, for EVERY netns) | transparent-mtls arc (injection shipped); this feature answers on it | one listener may NOT be able to answer on N per-netns gateway addrs (the load-bearing unvalidated routing assumption) | **Slice 00 (the spike)** — real-kernel one-listener-many-netns probe; BLOCKING |
| `mesh_dns_name` | the `<job>.svc.overdrive.local` grammar (job name ← the deploy spec `[service].id`) | the workload's query; the responder's name→backend lookup | this feature (the responder parses + matches the suffix) | name-grammar drift (suffix, case, label limits) vs what workloads dial | US-DBN-2 / US-DBN-4 ACs: `getaddrinfo("<server>.svc.overdrive.local")` resolves; unknown name → NXDOMAIN |
| `ping_pong_command_path` | the staged tiny Rust ping-pong bin's on-disk path in the deploy env (decided 2026-06-24); referenced by `examples/dial-by-name-responder/{a,b}.toml` `[exec].command` | `overdrive deploy` (the two specs); the workloads at runtime | this feature (the demo) | a phantom `command` path → the alloc never reaches Running → the demo silently can't run (the `dns-resolver.toml` collision class) | US-DBN-3 AC: `command` points at a real on-disk binary present in the deploy env, verified before the demo runs |
| `edd_ping_pong_evidence` | the `verification/expectations/` capture of the demo (proposed `E05-dial-by-name-ping-pong-mtls`), black-box against the built `overdrive` binary under Lima | EDD different-fox review; the operator-surface proof (K-DBN-3) | this feature (the EDD expectation) | a fabricated / narrated capture (forbidden by `verification.md`); a stale capture vs current HEAD | honest `pending` until the full-system EDD harness (#227/#75) lands (mirrors E04); captured + different-fox-reviewed, never self-stamped |

---

## `[REF]` Outcome KPIs (numeric targets + measurement method)

| KPI | Who | Does what (behavior change) | By how much (target) | Measured by | Baseline |
|---|---|---|---|---|---|
| **K-DBN-1 — name resolves to a live backend** | A deployed mesh workload | `getaddrinfo("<peer>.svc.overdrive.local")` resolves to a **running-AND-healthy** backend addr | **100%** of queries where ≥1 backend is running-AND-healthy resolve to a running-and-healthy backend addr (0 stale, 0 unhealthy, 0 timeout) across the Tier-3 acceptance matrix | Tier-3 test: query from inside a deployed workload's netns; assert resolved addr ∈ running-and-healthy `service_backends`; assert byte-equal to the `MtlsResolve`-recognized addr AND that `resolve` classifies it `Mesh` | Today: **0%** — nothing answers; resolution fails in every deploy |
| **K-DBN-2 — empty-candidate honesty** | A deployed workload querying a name with no running-and-healthy backend | Receives **NXDOMAIN** (no running-and-healthy backend) | **0** stale/unhealthy/guessed addresses returned across the no-backend / all-stopped / all-unhealthy / unknown-name cases; **100%** NXDOMAIN | Tier-3: deploy-then-query-before-running-and-healthy, stop-then-query, all-unhealthy, unknown-name — assert NXDOMAIN, assert never the prior addr | Today: N/A (no answer at all) — target is "honest NXDOMAIN," not "wrong addr" |
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
    name-answering listener (a sibling name-keyed reader over the SAME service_backends rows the ServiceBackendsResolve index reads — NOT the addr-keyed struct, D-TME-11)
    answers A with a running-AND-healthy IPv4 service_backends addr (headless, D-TME-10) — the
    same addr MtlsResolve recognizes and classifies Mesh (SocketAddrV4) — and AAAA as NODATA (v1 substrate
    is IPv4), or NXDOMAIN when no running-and-healthy backend exists (declared-but-not-running,
    unhealthy, and unknown collapse in v1); the existing intercept path then mTLS's the hop. Operator-observable
    via the dial-by-name-responder ping-pong demo.
  persona: >
    Sam (sam-platform-security-engineer.yaml) — reachability lens. Verifies an
    unmodified workload reaches its mesh peer by name and lands at a live instance,
    and that the name layer never returns a stale address.
  emotional_arc:
    start: "Skeptical — 'the wire is enforceable, but can an unmodified workload even FIND its peer by name in a deploy, and will the platform ever hand it a dead address?'"
    middle: "Reassured — the walking skeleton resolves one name to one running-and-healthy backend through serve+deploy and the hop goes TLS 1.3; the ping-pong demo advances counters he watches himself."
    end: "Confident — 'an ordinary workload dials by name and lands at a LIVE peer the mesh then mTLS's, and a no-backend query returns NOTHING, never a stale addr. I ran the proof myself with two overdrive deploys.'"
  steps:
    - id: 1
      name: "An unmodified workload resolves its peer by name to a running-and-healthy backend"
      command: "(workload getaddrinfo; in-agent responder answers from service_backends ∩ running-and-healthy as a sibling name-keyed reader over the SAME ObservationStore rows — the third reader of the surface, NOT the addr-keyed intercept struct)"
    - id: 2
      name: "The resolved addr is the one the intercept path recognizes (headless, one source)"
      command: "(internal — D-TME-10 single-source; no VIP)"
    - id: 3
      name: "The ping-pong demo advances — two services dial each other by name, each hop mTLS'd"
      command: "overdrive deploy examples/dial-by-name-responder/a.toml + b.toml"
  error_paths:
    - step: 1
      failure: "No running-and-healthy backend for the queried name"
      recovery: "NXDOMAIN (no running-and-healthy backend; declared-but-not-running, unhealthy, and unknown collapse in v1) — never a stale/cached/unhealthy/guessed addr (mirrors the arc's fail-honest discipline)."
  related_jobs: [J-MESH-001]
  related_features:
    - id: dial-by-name-responder
      role: "Ships the in-agent name responder (GH #243). The reachability leg of the mesh; a third sibling reader of the ObservationStore service_backends surface (the SAME rows ServiceBackendsResolve folds, NOT the addr-keyed struct)."
    - id: transparent-mtls-enrollment
      role: "Ships the intercept + MtlsResolve + resolve index this journey's name answers feed into (one source, three readers)."
```

### Anti-pattern scan (clean)
- No "Implement X" stories (all open from user pain). ✓
- No generic data (real names: Sam, Ana, server/client, a/b, `b.svc.overdrive.local`, `/usr/bin/socat`). ✓
- No technical AC / technical scenario titles (outcomes: "resolves to a running-and-healthy backend," "ping-pong advances," "honest NXDOMAIN"). ✓
- No oversized story (each ≤7 scenarios, single behavior). ✓
- No abstract requirements without examples (3+ per story). ✓

### Risks / notes
- **No DIVERGE artifacts** for this feature (`docs/feature/dial-by-name-responder/diverge/` absent) — consistent with the jobs.yaml header precedent (JTBD distilled from issue/arc, not interviews). Noted as a non-blocking risk; the parent arc's DISCUSS/DESIGN/spike history grounds the contracts.
- The EDD ping-pong capture may be `pending` until #227/#75 (full-system Lima EDD harness) — mirror E04's honest `pending` posture rather than fabricate a capture.

---

# Wave: DESIGN (ADR-0072, GH #243) — Morgan, 2026-06-25

> The single authoritative DESIGN narrative for this feature, compact `[REF]`
> form (no separate `design/wave-decisions.md` — the wave-decisions are folded
> here per the feature's compact-form posture). The Pass-1 user-ratified decision
> points (A1/B/C1/D1/E1/F/H1) are carried here under the stable `DDN-1..DDN-8`
> scheme (one ID per concern; see the Decisions table); this DESIGN pass pins the
> EXACT signatures, the running-AND-healthy gate, and the fallback source. Full
> decision record + alternatives: **ADR-0072**. SSOT
> components/C4: `docs/product/architecture/brief.md` § "Phase 2
> dial-by-name-responder extension (ADR-0072, GH #243)".

## Wave: DESIGN / [REF] DDD — strategic + tactical

- **D-DBN-1 (bounded context).** The name layer is a NEW reader bounded context
  over the EXISTING `service_backends` observation surface — NOT a new write
  surface and NOT a widening of the intercept (enforcement) context. The
  responder is the **third reader** (outbound resolve · inbound install · name
  answers); the addr-keyed `ServiceBackendsResolve` index is an *anti-corruption
  sibling*, untouched (A1).
- **D-DBN-2 (ubiquitous language).** `MeshServiceName` (the dialed
  `<job>.svc.overdrive.local` grammar) is the name layer's core domain term —
  modeled as a validated newtype, never a raw `String`. `NameAnswer` is the
  domain result of a query (`Records | NoData | NxDomain`), not a wire `Message`.
- **D-DBN-3 (aggregate / consistency boundary).** The `by_name` index is the
  read model; its consistency boundary is the List-then-Watch contract over the
  ObservationStore (eventually consistent, relist-on-`Lagged`). No transactional
  invariant spans the responder and the writer (`BackendDiscoveryBridge`).
- **D-DBN-4 (verified mapping).** `<job>` ← the SVID job segment of
  `Backend.alloc: SpiffeId` = `WorkloadId` = deploy `[service].id`. The
  "∩ running" filter is satisfied by construction (the bridge builds rows from
  `actual.actual.running` only); the `by_name` index additionally gates on
  `Backend.healthy == true` (running-AND-healthy, DDN-2) so an answered addr is
  always the `Mesh` set the intercept recognizes — the name layer derives no
  second liveness truth.
- **D-DBN-5 (anti-corruption to the wire).** `hickory-proto` is the wire codec
  behind a `wire.rs` translation boundary; the domain core (`answer_for`,
  `NameAnswer`, `MeshServiceName`) never names a hickory type in its signature
  (the QType crosses the boundary in `answer_for`; `NameAnswer` is hickory-free).

## Wave: DESIGN / [REF] Component decomposition

| Component | Path | Change | Responsibility |
|---|---|---|---|
| `MeshServiceName` newtype | `crates/overdrive-core/src/id.rs` | **CREATE NEW** | The `<job>.svc.overdrive.local` grammar; validated, case-insensitive, label ≤ `LABEL_MAX` |
| `NameAnswer` enum | `crates/overdrive-core/src/` (`id.rs` or small `dns` module) | **CREATE NEW** | Pure query result: `Records(Vec<SocketAddrV4>) \| NoData \| NxDomain` |
| `dns_responder` module root | `crates/overdrive-control-plane/src/dns_responder/mod.rs` | **CREATE NEW** | Module wiring + re-exports |
| `name_index.rs` | `…/dns_responder/name_index.rs` | **CREATE NEW** | `by_name: BTreeMap<MeshServiceName, BTreeSet<SocketAddrV4>>`; List-then-Watch + relist-on-`Lagged` + single-owner drain + `probe()` (mirror `ServiceBackendsResolve`) |
| `answer.rs` | `…/dns_responder/answer.rs` | **CREATE NEW** | Pure `answer_for(name, qtype, &index) -> NameAnswer` (the mutation-gate target) |
| `wire.rs` | `…/dns_responder/wire.rs` | **CREATE NEW** | `hickory-proto` decode (query) + encode (`NameAnswer` → bytes: A / NODATA-SOA / NXDOMAIN-SOA); separately proptested |
| `responder.rs` | `…/dns_responder/responder.rs` | **CREATE NEW** | `DnsResponder` host adapter: bind (wildcard→per-addr fallback), `IP_PKTINFO` recv/sendmsg loop, `probe()`, `serve()` |
| `DnsResponderError` | `…/dns_responder/` | **CREATE NEW** | Typed `thiserror`; no `Internal(String)` |
| composition root | `crates/overdrive-control-plane/src/lib.rs` (`run_server_with_obs_and_driver`, ~1893-1957) | **EXTEND** | Construct after `resolve.probe()`; `responder.probe()`; `tokio::spawn(serve)`; hold `JoinHandle`; same `mtls_worker.is_some()` gate; `health.startup.refused` on failure |
| `hickory-proto` dep | root `Cargo.toml` `[workspace.dependencies]` | **ADD** | Apache-2.0/MIT |
| `nix` features | `overdrive-control-plane` + workspace `nix` (`socket`, `uio`) | **EXTEND** | `recvmsg`/`sendmsg`/`ControlMessage::Ipv4PacketInfo` (no new public API) |

## Wave: DESIGN / [REF] Driving + driven ports

**Driving (inbound):**
- The **UDP DNS socket** (`0.0.0.0:53` wildcard, or N per-gateway-addr on
  fallback) — the workload's `getaddrinfo` query arrives here. NOT a Rust port
  trait (D4 / C1: the socket is irreducibly Tier-3, no Tier-2 backstop; a Sim
  adapter would simulate exactly the substrate the spike proved cannot be
  honestly simulated). The driving "seam" for test is the pure
  `answer_for(name, qtype, &index)` + the `wire.rs` codec.

**Driven (outbound):**
- `Arc<dyn ObservationStore>` — `all_service_backends_rows()` (List at probe +
  relist) and `subscribe_all_events() -> LagAwareSubscription` (single-owner
  drain). EXISTING port; the responder is its third reader.
- `Arc<dyn Clock>` — for the SOA `SERIAL`. EXISTING port, injected.
- The live gateway-set source (for the per-addr bind fallback only) — PINNED
  (DDN-5) as `veth_provisioner::NetSlotAllocator` (`state.net_slot_allocator`,
  the SAME map that owns every live `alloc → NetSlot` binding). The responder
  derives each gateway via the existing pure `responder_addr_for_slot(slot)`.
  Read ONLY if the wildcard bind `EADDRINUSE`s; never read on the wildcard path.
  `NetSlotAllocator` exposes no change-subscription (`snapshot()` only), so the
  fallback **re-derives the desired gateway set on the converge tick** and diffs
  it against the bound per-addr socket set (add-if-missing / drop-if-absent —
  `reconcilers.md` Bar-1 converge), keeping sockets tracking the live slot set.

**Required-deps discipline:** `DnsResponder::new(store, clock, slots:
NetSlotAllocator)` — all mandatory constructor params, no builder, no default
(`development.md` § "Port-trait dependencies"). The `NetSlotAllocator` handle is
a cheap `Arc`-shared clone; it is NOT a port trait (it is concrete host state,
the single source of slot truth — a second source would be the anti-pattern).

## Wave: DESIGN / [REF] Pinned signatures (implement to the design — do NOT invent surface)

```rust
// overdrive-core::id — NEW newtype, full completeness + proptest round-trip
pub struct MeshServiceName(/* validated <job> label */);
impl MeshServiceName {
    pub const SUFFIX: &'static str = "svc.overdrive.local";
    pub fn new(raw: &str) -> Result<Self, IdParseError>; // validates + canonicalises
    pub fn as_str(&self) -> &str;                         // canonical <job> label
}
// + Display (lowercase canonical), FromStr (case-insensitive), Serialize/
//   Deserialize (matching Display/FromStr), TryFrom<String>/<&str>.
//   Exact internal shape (store the <job> label vs the full name) is a DELIVER
//   detail; the public surface above is the contract.

// overdrive-core — NEW pure result type (PINNED — variant names are the contract)
pub enum NameAnswer {
    Records(Vec<SocketAddrV4>), // ≥1 running-AND-healthy IPv4 backend → A
    NoData,                     // live name, no record of the queried type (AAAA v1)
    NxDomain,                   // 0 running-and-healthy backends (declared-not-running OR unhealthy OR unknown)
}

// dns_responder::answer — pure, the mutation-gate target.
// qtype is PINNED to hickory_proto::rr::RecordType (reuse the codec's vocabulary —
// no redundant local QType enum; NameAnswer itself stays hickory-free).
pub fn answer_for(
    name: &MeshServiceName,
    qtype: hickory_proto::rr::RecordType,
    index: &NameIndex,
) -> NameAnswer;

// dns_responder::responder — host adapter
impl DnsResponder {
    pub fn new(
        store: Arc<dyn ObservationStore>,
        clock: Arc<dyn Clock>,
        slots: veth_provisioner::NetSlotAllocator, // PINNED gateway source for the
                                                   // per-addr fallback (DDN-5):
                                                   // re-derive responder_addr_for_slot
                                                   // over snapshot() on the converge tick
    ) -> Self;                                    // required deps, no builder
    pub async fn probe(&self) -> Result<(), DnsResponderError>; // bind + List-seed + watch; refuse-boot on failure
    pub async fn serve(self: Arc<Self>);          // IP_PKTINFO recv/answer_for/encode/sendmsg loop
}

// dns_responder — typed errors; NO Internal(String); each → distinct
// health.startup.refused reason (mirrors MtlsResolveError::{Probe,StoreUnreadable})
pub enum DnsResponderError {
    Bind { addr: SocketAddr, source: std::io::Error },
    ListSeed { reason: String },
    Probe { reason: String },
    Socket { source: std::io::Error },
}
```

## Wave: DESIGN / [REF] Technology choices (OSS-first)

| Choice | Pin | License | Rationale |
|---|---|---|---|
| DNS wire codec | **`hickory-proto`** (`hickory-proto.workspace = true`; add to `[workspace.dependencies]`) | Apache-2.0/MIT | OSS-first, mature, well-maintained; removes the DNS-encoding bug class (name compression, EDNS, SOA RDATA). Hand-rolled DNS REJECTED. |
| DNS server | `hickory-server` **REJECTED** | — | No per-packet reply-source control on a multi-homed wildcard socket; cannot satisfy the spike-mandatory `ipi_spec_dst` source-pin. |
| Socket / `IP_PKTINFO` | **own loop** via `nix` (`socket`, `uio` features) + `libc` (both already in workspace) | (nix MIT) | The spike-validated `ipi_spec_dst` source-pinning shape on one wildcard `0.0.0.0:53` socket; `getaddrinfo` rejects wrong-source replies. |
| Index collection | `BTreeMap`/`BTreeSet` | — | Iteration observed under test → deterministic across seeds (`development.md` § "Ordered-collection choice"). |
| Errors | `thiserror` typed `DnsResponderError` | — | No `Internal(String)` flatten; each variant → a distinct `health.startup.refused` reason. |

## Wave: DESIGN / [REF] Decisions table

> IDs are the stable `DDN-*` scheme (one per concern, consistent with ADR-0072).
> The user-ratified Pass-1 decision point each implements is shown in parens; the
> `F` point spanned two concerns (mapping + newtype), split into DDN-2 and DDN-7.

| ID | Decision | Alternatives rejected (see ADR-0072 for full rationale) |
|---|---|---|
| **DDN-1** (A1) | NEW sibling name-keyed reader over the `ObservationStore` `service_backends` surface (own `by_name` index, same rows + same List-then-Watch pattern as `ServiceBackendsResolve`) — byte-consistency is the shared rows, not a shared struct | A2: extend the addr-keyed intercept index struct (couples the name layer to the security-critical enforcement path) |
| **DDN-2** (F, mapping) | `<job>` ← SVID job segment; `by_name` index gates on **`Backend.healthy == true`** (running-AND-healthy), matching the intercept's `Mesh` set — mandatory, since an unhealthy addr is `MeshUnreachable` and answering it breaks byte-consistency | a declared-service view keyed by `[service].id` (needs a second observation surface to split declared-empty from unknown — not v1) |
| **DDN-3** (B) | `hickory-proto` codec + OWN `IP_PKTINFO` socket loop | hand-rolled DNS; `hickory-server` `RequestHandler` (no per-packet reply-source control — spike-verified) |
| **DDN-4** (C1) | Pure `answer_for` + separately-proptested encoder; NO port trait / NO Sim adapter | a `NameResponder` port + `SimNameResponder` (false confidence — sims the irreducibly-Tier-3 substrate; no second prod impl / no scheduling concern) |
| **DDN-5** (D1) | Bind `0.0.0.0:53` wildcard first; per-gateway-addr fallback on `EADDRINUSE`, source = `NetSlotAllocator` + `responder_addr_for_slot`, re-derived on the converge tick (add/drop sockets as slots come/go) | wildcard-only (node-image coupled); N per-addr only (wasteful, scales with allocs) |
| **DDN-6** (E1) | `run_server` owns it: after `resolve.probe()`, `responder.probe()`, spawn, hold `JoinHandle`; same `mtls_worker.is_some()` gate; `health.startup.refused` on failure | lazy spawn outside the composition root (breaks wire→probe→use); a standalone daemon (second source of truth; D-TME-11 in-agent reframe) |
| **DDN-7** (F, newtype) | NEW `MeshServiceName` newtype (`SUFFIX = svc.overdrive.local`, single `<job>` label v1, full completeness + proptest) | raw `String` parse (newtype violation); reuse `WorkloadId` as the name key (it is the job label, not the dialed name grammar) |
| **DDN-8** (H1) | NXDOMAIN(+1s-MINIMUM SOA) for 0-running-and-healthy; NODATA(+same SOA) for AAAA-on-live; pinned SOA fields (SERIAL via `Clock`) | no-SOA negative answers (implementation-default negative cache; stale negative window); longer negative TTL (delays deploy-then-dial re-resolve) |

## Wave: DESIGN / [REF] Reuse Analysis (mandatory hard gate — carried from Pass 1)

| Capability needed | Existing? | Verdict | Evidence |
|---|---|---|---|
| List-then-Watch + relist-on-`Lagged` + single-owner-drain + `probe()` over `ObservationStore` | YES — `ServiceBackendsResolve` (`mtls_resolve_adapter.rs`) | **REUSE the PATTERN** (mirror it as a sibling), do not REUSE the struct (A1 keeps the security path untouched) | `all_service_backends_rows`, `subscribe_all_events`, `LagAwareSubscription`, `SubscriptionEvent::{Row,Lagged}` consumed identically |
| `service_backends ∩ running-AND-healthy` rows | YES — `BackendDiscoveryBridge` builds rows from `actual.actual.running` only; `Backend.healthy` is on the row | **REUSE** (read the rows; ∩-running holds by construction; the index gates `healthy == true` per DDN-2) | `backend_discovery_bridge.rs` ~351; `mtls_resolve_adapter.rs:124-135` (the `Mesh`/`MeshUnreachable` healthy split) |
| Name→backend mapping (`<job>` ← SVID) | PARTIAL — `SpiffeId::path()` exists; NO job-segment accessor | **EXTEND** (surface OQ-1: pin the accessor in DISTILL/DELIVER, do NOT improvise) | `id.rs` `SpiffeId::path() -> "/job/<wk>/alloc/<id>"`; `for_allocation`; `WorkloadId` = `[service].id` |
| Label-shaped newtype machinery | YES — `define_label_newtype!` macro, `LABEL_MAX`, `validate_label` | **REUSE** (model `MeshServiceName` on it; suffix grammar needs a bespoke `FromStr`, so likely a hand-written newtype using the same validators) | `id.rs` lines 65-214 |
| DNS wire codec | NO | **CREATE NEW** (via `hickory-proto`, OSS-first) | no DNS dep in workspace |
| `IP_PKTINFO` recv/sendmsg socket | NO (but `nix`/`libc` present; spike has the shape) | **CREATE NEW** (own loop) | spike `increment-a` validated `ipi_spec_dst` |
| Composition-root probe-then-spawn-then-hold-handle | YES — the `resolve.probe()` + `mtls_worker.is_some()` block | **EXTEND** | `lib.rs` ~1893-1957 |
| `Clock` injection | YES — `Arc<dyn Clock>` on `AppState` (`config.clock`) | **REUSE** | `lib.rs` `config.clock.clone()` |

**Gate verdict: PASS.** Every capability is REUSE/EXTEND except the two genuinely
novel surfaces (DNS codec via OSS `hickory-proto`; the `IP_PKTINFO` socket loop),
each justified by "no existing alternative." The security-critical intercept
index is provably untouched.

## Wave: DESIGN / [REF] Open questions (deferred to DISTILL/DELIVER)

- **OQ-1 — the `SpiffeId` → `<job>` accessor signature.** The mapping is verified
  (D-DBN-4 / DDN-2), but no existing `SpiffeId` accessor returns the job segment. The
  exact accessor (a new `SpiffeId::job_segment() -> Option<&str>` on the newtype,
  or a parse helper local to the index) is a small surface decision left to
  DISTILL/DELIVER per CLAUDE.md "Implement to the design — never invent API
  surface." The crafter MUST surface and pin it, not improvise. **This is the
  one remaining gap the DESIGN decisions did not cover; it is named, not improvised.**

> **Resolved in this DESIGN pass (no longer open):**
> - **`NameAnswer` variant names + the `answer_for` qtype type are PINNED** —
>   `enum NameAnswer { Records(Vec<SocketAddrV4>), NoData, NxDomain }` and
>   `qtype: hickory_proto::rr::RecordType` (see Pinned signatures + ADR DDN-4).
> - **The per-addr-fallback gateway-set source is PINNED** —
>   `veth_provisioner::NetSlotAllocator` + `responder_addr_for_slot`, re-derived
>   on the converge tick (see Driven ports + ADR DDN-5).

## Wave: DESIGN / [REF] DEVOPS / Tier-3 obligation (for platform-architect)

- **Re-confirm the spike verdict on the 6.18 appliance kernel (ADR-0068) in the
  DELIVER Tier-3 matrix.** The Slice-00 PROMOTE is pinned to dev-Lima
  `7.0.0-22-generic`; the exercised surfaces (`IP_PKTINFO`, multi-homed UDP,
  per-netns `resolv.conf`, `SO_REUSEADDR` wildcard coexistence) are long-stable
  (well pre-6.18), so the verdict is expected to hold but is not separately
  confirmed there.
- **Acceptance SIGNAL is `getaddrinfo`/`getent`, never `dig @gw` alone** —
  `dig @gw` is lenient and masks a missing `ipi_spec_dst` source-pin.
- **`ip_forward=1` prerequisite** (already modeled as the converge-on-boot
  `EnableIpForward` step).
- **No external third-party API** — no consumer-driven contract tests apply
  (the only "external" surface is the kernel UDP/`IP_PKTINFO` substrate, covered
  by Tier-3, not Pact).

## Wave: DESIGN / [REF] Wave-decisions (DESIGN — folded, compact form)

1. **All ratified decision points carried into the stable `DDN-1..DDN-8` scheme**
   (one ID per concern; the Pass-1 points A1/B/C1/D1/E1/F/H1 map in, with `F`
   split across DDN-2 mapping + DDN-7 newtype — eight concerns, no duplicate
   labels). This DESIGN pass pinned signatures + the healthy gate + the fallback
   source; no decision re-opened.
2. **ADR-0072 minted** (`docs/product/architecture/adr-0072-dial-by-name-responder-node-local-dns.md`),
   next free platform-track number (0071 was the prior highest).
3. **brief.md SSOT** extended with a NEW `## Phase 2 dial-by-name-responder
   extension (ADR-0072, GH #243)` section (§36) + an ADR-0072 index row — the
   per-feature-section convention, NOT a rewrite of `## Application Architecture`.
4. **No separate `design/wave-decisions.md`** — folded here per the feature's
   compact-form posture (consistent with the DISCUSS compact form).
5. **One gap surfaced, not improvised** (OQ-1: the `SpiffeId` → `<job>`
   accessor) — per CLAUDE.md "Implement to the design — never invent API surface."
   The two former open questions (the `NameAnswer`/qtype signatures and the
   fallback gateway source) are now PINNED in DESIGN, not deferred.
6. **Ready for DISTILL handoff** (acceptance-designer): the contract table
   (running-AND-healthy throughout), the pinned signatures, the C4, and the Reuse
   gate are complete; OQ-1 is the sole named gap for the crafter to pin (not a
   blocker to DISTILL).

---

# Wave: DISTILL (GH #243) — Quinn, 2026-06-25

> The executable acceptance specification for this feature. Compact `[REF]`
> form, matching the file's lean-density posture. The GIVEN/WHEN/THEN scenario
> SSOT lives in `docs/feature/dial-by-name-responder/distill/test-scenarios.md`
> (the 26-scenario executable spec — **no `.feature` files**, per
> `.claude/rules/testing.md`); the RED-classification PLAN in
> `distill/red-classification.md`. These `[REF]` sections are the pointers +
> structured summaries. Wave-Decision Reconciliation HARD GATE: **PASS — 0
> contradictions** across DISCUSS / DESIGN (no DEVOPS delta dir; the Tier-3
> obligation is folded into the DESIGN § DEVOPS/Tier-3 section). Lang: Rust
> (`[lang-mode] rust`). Policy: `inherit` (`docs/architecture/atdd-infrastructure-policy.md`
> exists; dial-by-name rows appended below).

## Wave: DISTILL / [REF] Inherited commitments

| Origin | Commitment | DDD | Impact |
|--------|------------|-----|--------|
| DESIGN/DDN-1 | Sibling name-keyed reader over the SAME `service_backends` rows; the addr-keyed `ServiceBackendsResolve` intercept struct is provably untouched | DDN-1 | Scenarios assert through `answer_for` / the index's public read only; S-DBN-IDX-04 + S-DBN-SINGLE-SRC prove no second source of backend truth and `mtls_resolve_adapter.rs` is not modified |
| DESIGN/DDN-2 | `by_name` index gates `Backend.healthy == true` (running-AND-healthy), matching the intercept's `Mesh` set | DDN-2 | S-DBN-ANSWER-04 + S-DBN-IDX-02 + S-DBN-SINGLE-SRC make the healthy gate a structural mutation target — an unhealthy-only name MUST yield NXDOMAIN, and the answered addr MUST classify `Mesh` |
| DESIGN/DDN-3 | `hickory-proto` codec + own `IP_PKTINFO` socket loop (`hickory-server` rejected — no per-packet reply-source control) | DDN-3 | `wire.rs` is proptested in isolation (S-DBN-WIRE-*); the socket loop is irreducibly Tier-3 (S-DBN-BIND-*), acceptance via `getent` source-pin, never `dig` |
| DESIGN/DDN-4 | Pure `answer_for` + separately-proptested encoder; NO port trait, NO Sim adapter | DDN-4 | `answer_for` is THE mutation-gate target (S-DBN-ANSWER-*); the project policy records "no new Sim adapter for the socket" — the socket has no Tier-2 backstop, a Sim would give false confidence |
| DESIGN/DDN-5 | Wildcard `0.0.0.0:53` first; per-gateway-addr fallback on `EADDRINUSE`, source = `NetSlotAllocator` + `responder_addr_for_slot`, re-derived on the converge tick | DDN-5 | S-DBN-BIND-01 (wildcard coexists) + S-DBN-BIND-02 (forced-`EADDRINUSE` fallback re-derive lifecycle) |
| DESIGN/DDN-6 | `run_server` owns the responder: probe-then-spawn-then-hold-handle, gated by `mtls_worker.is_some()`, `health.startup.refused` on failure | DDN-6 | S-DBN-WS litmus (delete the spawn → `getent` times out) + S-DBN-BIND-03 (Earned-Trust refuse-boot, cause-distinct `DnsResponderError` → distinct refusal reason) |
| DESIGN/DDN-7 | NEW `MeshServiceName` newtype (`SUFFIX = svc.overdrive.local`, single `<job>` label v1, full completeness + proptest) | DDN-7 | S-DBN-NAME-01..04 (mandatory round-trip proptest + case-insensitive + suffix grammar + label-limit rejection) in the core newtype-test suite |
| DESIGN/DDN-8 | NXDOMAIN(+1s-MINIMUM SOA) for 0-running-and-healthy; NODATA(+same SOA) for AAAA-on-live; SERIAL via `Clock` | DDN-8 | S-DBN-WIRE-02/03/04 pin the SOA shape + the deterministic-per-`Clock` SERIAL; S-DBN-NXDOMAIN-01 confirms the 1s TTL lets a retry land |
| DISCUSS/D-TME-10 | Headless return shape — the answered `A` addr is byte-identical to the addr `MtlsResolve.resolve` recognizes; NO VIP, NO #167/#61 | n/a | S-DBN-SINGLE-SRC (K-DBN-4 oracle): feed the answered addr into `resolve`, assert byte-equality + `Mesh` classification |
| DISCUSS/D-TME-11 | One source, THREE readers — the responder is the third sibling reader over the `ObservationStore` `service_backends` surface (same rows, same List-then-Watch + relist-on-`Lagged`) | DDN-1 | S-DBN-IDX-01/03 mirror the `ServiceBackendsResolve` List-then-Watch + relist behaviour against `SimObservationStore` |
| DISCUSS/D-TME-9 | `resolv.conf` injection SHIPPED — each per-netns gateway = `plan.host_addr`, the addr the responder answers on | n/a | The Tier-3 fixtures (S-DBN-WS / S-DBN-BIND-*) rely on the shipped injection; the responder answers on each per-netns gateway (one root-netns wildcard listener) |
| SPIKE/Slice-00 | `getaddrinfo`/`getent`, never `dig @gw` alone (the source-pin litmus); `ip_forward=1` prerequisite; root-netns wildcard listener | n/a | K2 fixture knob — every name-path Tier-3 scenario asserts on `getent`; a `dig`-only assertion is a reviewer-flagged defect |
| CLAUDE.md | Implement to the design — never invent API surface; surface OQ-1, do not improvise | n/a | OQ-1 (the `SpiffeId` → `<job>` accessor, CONFIRMED-ABSENT this pass) is NAMED as the one open surface decision the crafter pins in DELIVER; DISTILL picks NO signature |

## Wave: DISTILL / [REF] Scenario list with tags

26 scenarios (16 Tier 1 pure / in-memory, 10 Tier 3 real-kernel Lima).
Full GIVEN/WHEN/THEN in `distill/test-scenarios.md`. Error-path coverage
**14/26 = 54%** (≥40% target met — the fail-honest NXDOMAIN posture is
the load-bearing US-DBN-4 leg).

| Scenario | Tags | Tier | US |
|---|---|---|---|
| S-DBN-NAME-01..04 | `@property`/`@error_path` `@in-memory` | 1 | US-DBN-2 |
| S-DBN-ANSWER-01..05 | `@property`/`@error_path` `@in-memory` `@kpi` | 1 | US-DBN-2/4 |
| S-DBN-WIRE-01..04 | `@property`/`@error_path` `@in-memory` | 1 | US-DBN-2/4 |
| S-DBN-IDX-01..04 | `@property`/`@error_path` `@in-memory` `@kpi` | 1 | US-DBN-2/4 |
| S-DBN-WS | `@walking_skeleton` `@driving_adapter` `@real-io` `@kpi` | 3 | US-DBN-2 |
| S-DBN-SINGLE-SRC | `@real-io` `@kpi` | 3 | US-DBN-2 |
| S-DBN-PINGPONG | `@walking_skeleton` `@real-io` `@edd` `@kpi` | 3 | US-DBN-3 |
| S-DBN-NXDOMAIN-01..03 | `@real-io` `@error_path` `@kpi` | 3 | US-DBN-4 |
| S-DBN-BIND-01..03 | `@boot` `@real-io` (`@error_path` on 02/03) | 3 | US-DBN-2 |

## Wave: DISTILL / [REF] WS strategy (Architecture of Reference)

Per the project Architecture of Reference (port class → treatment), NOT a
per-feature A/B/C/D choice:

- **Driving** (entry points) = real adapters: `overdrive serve`
  (`run_server_with_obs_and_driver`) + `overdrive deploy` (`POST /v1/jobs`,
  in-process per the keystone) + `getaddrinfo`/`getent` (the workload's
  real stub-resolver path). The walking skeleton (S-DBN-WS) and the
  ping-pong demo (S-DBN-PINGPONG) close the loop through these, mirroring
  `canonical_address_inbound_walking_skeleton.rs`.
- **Driven internal** (`ObservationStore` `service_backends` surface) =
  real: Tier-3 uses the real `LocalObservationStore`; Tier-1 uses
  `SimObservationStore` (the `adapter-sim` "real" in-process adapter
  honouring the same trait) for the watch/relist logic (S-DBN-IDX-*).
- **The UDP `:53` socket + `IP_PKTINFO`** = **irreducibly Tier-3 real, NO
  Sim** (DDN-4). The spike proved this substrate cannot be honestly
  simulated (no Tier-2 `BPF_PROG_TEST_RUN`-equivalent for multi-homed
  `IP_PKTINFO`); a Sim adapter would simulate exactly the part the spike
  proved lies. The DST seam is the pure `answer_for` + the proptested
  `wire.rs` encoder, NOT a fake socket.
- **Driven external / non-deterministic** = none new (`Clock` is the only
  injected non-determinism, reused; no email/SMS/payment/LLM/3rd-party).

## Wave: DISTILL / [REF] Adapter coverage table

Every driven adapter the responder adds or consumes → a `@real-io`
scenario (Mandate 6). Full table in `distill/test-scenarios.md` §
"Adapter coverage table". Summary: UDP `:53` socket loop (S-DBN-WS,
S-DBN-BIND-01/02) · `getaddrinfo`/`getent` consuming adapter (S-DBN-WS,
S-DBN-NXDOMAIN-*, S-DBN-BIND-01/02) · `ObservationStore` reader (S-DBN-WS
real, S-DBN-IDX-* Sim) · `Clock` for SOA SERIAL (S-DBN-WIRE-04) ·
`NetSlotAllocator` fallback source (S-DBN-BIND-02) · `MtlsResolve.resolve`
oracle (S-DBN-SINGLE-SRC) · `EbpfDataplane`/intercept (reused, S-DBN-WS +
S-DBN-PINGPONG) · `overdrive deploy` handler (S-DBN-WS, S-DBN-PINGPONG,
S-DBN-NXDOMAIN-*). **Empty rows: none.** The pure `answer_for` + `wire.rs`
are NOT adapters (no port trait, DDN-4) — they are the Tier-1 proptest
seams.

## Wave: DISTILL / [REF] Scaffold MANIFEST

**SCOPE DECISION**: DISTILL produces this MANIFEST, NOT landed `.rs`
files. The `answer_for`/`wire.rs` scaffolds NAME `hickory_proto` types, so
a compilable RED scaffold REQUIRES the `hickory-proto` workspace dep +
the `nix` `socket`/`uio` features — which are DELIVER's wiring step
(ADR-0072 § Components: "ADD"/"EXTEND"). Landing a half-built module + a
new workspace dep mid-DISTILL would perturb the workspace build for
everyone and is out of scope. **NO file is written under `crates/` this
wave.** DELIVER's RED phase materialises each file below with the
`todo!("RED scaffold: …")` / `#[should_panic(expected = "RED scaffold")]`
markers, adds the dep, and runs the fail-for-right-reason gate
(`distill/red-classification.md`).

### Production scaffolds (DELIVER materialises; all `todo!("RED scaffold: …")` + `#[expect(clippy::todo, …)]`)

| Path | Stubs (the PINNED signature) | Scenarios it RED's |
|---|---|---|
| `crates/overdrive-core/src/id.rs` (+`MeshServiceName`) | `MeshServiceName(/*…*/)`; `const SUFFIX = "svc.overdrive.local"`; `new(&str) -> Result<Self, IdParseError>`; `as_str(&self) -> &str`; `+ Display` (lowercase) `+ FromStr` (case-insensitive) `+ Serialize/Deserialize` (matching) `+ TryFrom<String>/<&str>` (model on `define_label_newtype!` + `validate_label` + `LABEL_MAX`; bespoke suffix `FromStr`) | S-DBN-NAME-01..04 |
| `crates/overdrive-core/src/` (`NameAnswer`, in `id.rs` or a small `dns` module) | `enum NameAnswer { Records(Vec<SocketAddrV4>), NoData, NxDomain }` (variant names ARE the contract) | (consumed by all `answer_for` scenarios) |
| `crates/overdrive-control-plane/src/dns_responder/mod.rs` | module wiring + re-exports | (module root) |
| `crates/overdrive-control-plane/src/dns_responder/answer.rs` | `pub fn answer_for(name: &MeshServiceName, qtype: hickory_proto::rr::RecordType, index: &NameIndex) -> NameAnswer` | S-DBN-ANSWER-01..05 |
| `crates/overdrive-control-plane/src/dns_responder/name_index.rs` | `NameIndex` = `by_name: BTreeMap<MeshServiceName, BTreeSet<SocketAddrV4>>`; List-then-Watch + relist-on-`Lagged` + single-owner-drain + `probe()` (mirror `ServiceBackendsResolve`); `Backend.healthy == true` gate (DDN-2); the OQ-1 `<job>` grouping | S-DBN-IDX-01..04, S-DBN-ANSWER-04 |
| `crates/overdrive-control-plane/src/dns_responder/wire.rs` | `hickory-proto` decode (query) + encode (`NameAnswer → Vec<u8>`: A / NODATA-SOA / NXDOMAIN-SOA, MINIMUM=1, SERIAL via `Clock`) | S-DBN-WIRE-01..04 |
| `crates/overdrive-control-plane/src/dns_responder/responder.rs` | `impl DnsResponder { new(store: Arc<dyn ObservationStore>, clock: Arc<dyn Clock>, slots: veth_provisioner::NetSlotAllocator) -> Self; async fn probe(&self) -> Result<(), DnsResponderError>; async fn serve(self: Arc<Self>) }` (wildcard→per-addr fallback, `IP_PKTINFO` recv/send loop) | S-DBN-BIND-01..03, S-DBN-WS, S-DBN-SINGLE-SRC, S-DBN-NXDOMAIN-*, S-DBN-PINGPONG |
| `crates/overdrive-control-plane/src/dns_responder/` (`DnsResponderError`) | `enum DnsResponderError { Bind { addr: SocketAddr, source: std::io::Error }, ListSeed { reason: String }, Probe { reason: String }, Socket { source: std::io::Error } }` — typed, NO `Internal(String)`, each → distinct `health.startup.refused` reason | S-DBN-BIND-03 |
| `crates/overdrive-control-plane/src/lib.rs` (`run_server_with_obs_and_driver` ~1893-1957) | EXTEND: construct after `resolve.probe()`; `responder.probe()`; `tokio::spawn(serve)`; hold `JoinHandle`; same `mtls_worker.is_some()` gate; `health.startup.refused` on failure | S-DBN-WS (litmus), S-DBN-BIND-03 |

### Test scaffolds (DELIVER materialises; `#[should_panic(expected = "RED scaffold")]`)

| Path | Tier | Scenarios |
|---|---|---|
| `crates/overdrive-core/tests/acceptance/core_newtype_roundtrip.rs` (EXTEND) | 1 | S-DBN-NAME-01/02 (mandatory proptest round-trip) |
| `crates/overdrive-core/tests/acceptance/core_newtype_validation.rs` (EXTEND) | 1 | S-DBN-NAME-03/04 |
| `crates/overdrive-control-plane/tests/acceptance/dns_answer_for.rs` (NEW; wire into `acceptance.rs`) | 1 | S-DBN-ANSWER-01/02/03/05 |
| `crates/overdrive-control-plane/tests/acceptance/dns_name_index.rs` (NEW) | 1 | S-DBN-ANSWER-04, S-DBN-IDX-01..04 |
| `crates/overdrive-control-plane/tests/acceptance/dns_wire.rs` (NEW) | 1 | S-DBN-WIRE-01..04 |
| `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs` (NEW; `mod integration { mod … }` in `integration.rs`) | 3 | S-DBN-WS, S-DBN-SINGLE-SRC |
| `crates/overdrive-control-plane/tests/integration/dns_responder_ping_pong.rs` (NEW) | 3 | S-DBN-PINGPONG (+ the `E05` EDD expectation, honest `pending`) |
| `crates/overdrive-control-plane/tests/integration/dns_responder_nxdomain.rs` (NEW) | 3 | S-DBN-NXDOMAIN-01..03 |
| `crates/overdrive-control-plane/tests/integration/dns_responder_bind.rs` (NEW) | 3 | S-DBN-BIND-01..03 |
| `examples/dial-by-name-responder/{a,b}.toml` + the staged ping-pong bin | 3 | S-DBN-PINGPONG fixtures (real on-disk `command` path) |

**No `__SCAFFOLD__` / `// SCAFFOLD: true` marker sweep this wave** — the
Rust RED convention is `todo!("RED scaffold: …")` (production) +
`#[should_panic(expected = "RED scaffold")]` (test), discoverable via
`grep -rn 'RED scaffold' crates/`, per `.claude/rules/testing.md` § "RED
scaffolds". DELIVER injects a file-level
`#![cfg_attr(not(test), expect(clippy::todo, reason = "RED scaffold; lands GREEN slice 01"))]`
on the `dns_responder` module during the active slice (the DISTILL scaffold
clippy-discipline note in project memory) and strips it once Slice 03
closes the last scaffold.

## Wave: DISTILL / [REF] Test placement

| Surface | Directory | Precedent justification |
|---|---|---|
| `MeshServiceName` / `NameAnswer` | `crates/overdrive-core/tests/acceptance/` | `core_newtype_roundtrip.rs` + `core_newtype_validation.rs` already test newtypes here (the mandatory round-trip + rejection suites) |
| `answer_for` / `wire.rs` / `NameIndex` | `crates/overdrive-control-plane/tests/acceptance/` | Default-lane Tier-1 in-process tests live here (e.g. `eval_broker_collapse.rs`, `listener_fact_*`); `SimObservationStore`-driven, no real I/O |
| Tier-3 responder + intercept | `crates/overdrive-control-plane/tests/integration/` (gated `#![cfg(feature="integration-tests")]`, inline `mod integration { mod … }`) | `canonical_address_inbound_walking_skeleton.rs` is the direct sibling (in-process `run_server` + real `EbpfDataplane`); the `mod integration` inline trick is the documented Cargo lookup-base shift (`.claude/rules/testing.md` § Mechanics) |
| `examples/dial-by-name-responder/{a,b}.toml` | `examples/<feature>/` | Introduces the per-feature subdir convention; `command` → a real on-disk staged bin (the `coinflip-as-service.toml` / `dns-resolver.toml` precedents) |

## Wave: DISTILL / [REF] Driving-adapter coverage

The user-facing driving surfaces: (1) `getaddrinfo`/`getent` (the
name-path acceptance SIGNAL — **never `dig` alone**, the spike litmus),
(2) `overdrive deploy` (`POST /v1/jobs`, in-process per the keystone),
(3) `overdrive serve` (`run_server`, the boot driving surface for the
Earned-Trust gate). Each has ≥1 Tier-3 scenario exercising it via its
real protocol: `getent` (S-DBN-WS, S-DBN-NXDOMAIN-*, S-DBN-BIND-01/02),
`POST /v1/jobs` (S-DBN-WS, S-DBN-PINGPONG, S-DBN-NXDOMAIN-*), `run_server`
refuse-boot (S-DBN-BIND-03). Full detail in `distill/test-scenarios.md` §
"Driving-adapter verification".

## Wave: DISTILL / [REF] Project Infrastructure Policy rows (appended)

The policy at `docs/architecture/atdd-infrastructure-policy.md` exists
(inherit mode). Dial-by-name introduces these port rows (DELIVER appends
them to the policy file; recorded here for the audit trail):

- **Driving** — `getaddrinfo`/`getent` (glibc stub resolver) → real
  resolution from inside a deployed workload's netns under Lima
  (`cargo xtask lima run --`); the name-path acceptance signal, never
  `dig @gw` alone.
- **Driven internal** — `ObservationStore` `service_backends` reader →
  `SimObservationStore` (Tier 1, the watch/relist logic) + real
  `LocalObservationStore` (Tier 3). The responder is a sibling reader; no
  new struct.
- **Driven internal (NO Sim)** — the UDP `:53` socket + `IP_PKTINFO` →
  real kernel only (DDN-4; no Tier-2 backstop). Explicitly NOT a port
  trait, NOT a Sim adapter — the DST seam is the pure `answer_for` + the
  proptested `wire.rs` codec.
- **Driven external / non-deterministic** — none new (`Clock` reused for
  the SOA SERIAL; no email/SMS/payment/LLM/3rd-party).

## Wave: DISTILL / [REF] Outcomes registered

Four OUT-DBN rows registered to `docs/product/outcomes/registry.yaml`
(`feature: dial-by-name-responder`) — see § "Outcome registration" in the
DISTILL summary. `OUT-DBN-ANSWER-FOR` (the v1 DNS answer contract,
`specification`), `OUT-DBN-MESH-SERVICE-NAME` (the name grammar newtype,
`specification`), `OUT-DBN-RESPONDER-SERVE` (the name-answering operation
through serve+deploy, `operation`), `OUT-DBN-SINGLE-SOURCE` (the answered
addr == the `MtlsResolve` `Mesh` addr, `invariant`).

## Wave: DISTILL / [REF] Pre-requisites

- **SHIPPED**: `resolv.conf` injection (D-TME-9), the `ServiceBackendsResolve`
  index (D-TME-11), the `MtlsResolve` consumer + intercept path
  (transparent-mtls arc), the canonical-address inbound walking-skeleton
  test shape (the boot/deploy/netns fixture this feature mirrors).
- **DONE**: Slice 00 PROMOTE (`spike/wave-decisions.md`) — the
  one-listener-many-netns assumption is validated.
- **DELIVER's RED-phase deps (NOT added this wave — see Scaffold MANIFEST
  SCOPE DECISION)**: `hickory-proto.workspace = true` (root `Cargo.toml`
  `[workspace.dependencies]` — the one new workspace dep the `answer_for`/
  `wire.rs` scaffolds need to compile) + the `nix` `socket`/`uio`
  features; the staged tiny Rust ping-pong bin at a real on-disk `command`
  path (decided 2026-06-24, the `coinflip-helper` precedent).
- **Tier-3 obligation**: re-confirm the spike verdict on the pinned-6.18
  appliance kernel in the DELIVER Tier-3 matrix (ADR-0068; the MERGE
  GATE). Dev-Lima `7.0.0-22-generic` is necessary-but-not-sufficient.

## Wave: DISTILL / [REF] Wave-decisions (DISTILL — folded, compact)

1. **Reconciliation HARD GATE: PASS — 0 contradictions** across DISCUSS /
   DESIGN. DISCUSS's D-TME-9/10/11 + the v1 DNS answer contract table are
   carried into DESIGN's DDN-1..8 with no inversion; the scenarios
   reference the running-AND-healthy contract throughout. No DEVOPS delta
   dir exists (the Tier-3 obligation is folded into DESIGN § DEVOPS);
   default infra used, warning logged, not a blocker.
2. **26 scenarios, 54% error-path.** Tier 1 (16) = the pure seams
   (`MeshServiceName`, `answer_for`, `wire.rs`, `NameIndex`) under
   proptest + `SimObservationStore`; Tier 3 (10) = the socket + boot + the
   reused intercept under Lima as root. No Tier 2 (DDN-4, no
   `BPF_PROG_TEST_RUN` surface).
3. **OQ-1 surfaced, NOT resolved.** `SpiffeId` confirmed-absent a
   job-segment accessor (`id.rs:267-282` exposes only `as_str`/
   `trust_domain`/`path`). DISTILL names it as the one open surface
   decision; DELIVER pins the signature (CLAUDE.md "never invent API
   surface"). No scenario depends on a specific accessor shape —
   S-DBN-IDX-01 asserts the `<job>`-grouping behaviour, not the accessor.
4. **No `crates/` files written this wave** (the Scaffold MANIFEST SCOPE
   DECISION) — the `hickory-proto` dep is DELIVER's wiring step.
5. **EDD graduation**: S-DBN-PINGPONG → `verification/expectations/`
   `E05-dial-by-name-ping-pong-mtls`, honest `pending` until #227/#75
   (mirror E04). DELIVER authors the expectation stub + `runner.sh`; the
   capture is different-fox-reviewed, never self-stamped.
6. **Pillar compliance**: Pillar 1 (domain language — titles use
   `resolve by name`, `running-and-healthy`, `stale addr`, `NXDOMAIN`, no
   DNS/socket jargon in titles); Pillar 2 (chained narrative — S-DBN-IDX-01
   → IDX-02 read as one name's lifecycle; S-DBN-WS → S-DBN-NXDOMAIN-02
   chain a resolved name into its stop); Pillar 3 (production composition
   root — S-DBN-WS uses real `run_server`; no Tier B state-machine PBT —
   the journey is rich but the socket is irreducibly Tier-3, so Tier B's
   in-memory composition would simulate the substrate the spike proved
   cannot be simulated; the pure `answer_for`/`NameIndex` PBT at Tier 1
   covers the domain-rich input space instead).
