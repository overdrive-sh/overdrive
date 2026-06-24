# Test scenarios — `canonical-workload-address-inbound-tproxy` (GH #241)

**Wave:** DISTILL · **Paradigm:** OOP Rust · **Density:** lean ·
**Model:** inherits (DESIGN locked; three Tier-3 spikes settled every
load-bearing question)

> **Specification only — NEVER parsed or executed.** Per
> `.claude/rules/testing.md` § "Testing": this project has **no `.feature`
> files**, no pytest-bdd, no cucumber-rs. The GIVEN/WHEN/THEN blocks below are
> the human-readable specification companion. The crafter translates each
> scenario into a Rust `#[test]` / `#[tokio::test]` in the placement named per
> scenario. DISTILL ships the RED scaffolds as Rust
> `#[should_panic(expected = "RED scaffold")]` placeholders; DELIVER replaces
> the panic bodies with the real assertions.

This is the **keystone slice** of the transparent-mtls-enrollment arc. It
productionises the inbound nft-TPROXY install ADR-0071's `start_alloc` deferred
(`tproxy_guard = None`) and flips the `BackendDiscoveryBridge` advertise addr to
the canonical per-workload `workload_addr`. The acceptance gate (S-WS) drives the
**production composition root in-process** — real `run_server` boot + the real
in-process deploy submit handler for two mesh workloads — with **no
test-installed inbound rule and no synthetic loopback virt**; the production
`start_alloc` install captures the dial. (See the reconciliation note under S-WS
for why this is in-process rather than `serve`/`deploy` subprocesses.)

---

## Driving ports (where the scenarios enter)

| Driving port | Adapter | Scenarios |
|---|---|---|
| Operator CLI — `overdrive serve` | `commands::serve` → `run_server` (boot composition root; the gated hydrator + reused shared-routing infra stand up here) | S-WS |
| Operator CLI — `overdrive deploy <SPEC>` | `commands::deploy` → action-shim `StartAllocation` (C3 seam injects `workload_addr`; `WorkloadLifecycle` injects `service_ports`; `start_alloc` installs the inbound rule(s)) | S-WS, S-NRULES, S-DPORT, S-JOB0 |
| `BackendDiscoveryBridge::reconcile` (pure reconciler driving port — the function signature IS the port, per `nw-tdd-methodology` § "Hexagonal Architecture Testing → Domain Layer") | direct call, in-process Tier-1 DST | S-BRIDGE, S-PORTSET |
| `ServiceMapHydrator::reconcile` (pure reconciler driving port) | direct call, in-process Tier-1 DST | S-GATE, S-PORTSET |
| `AllocStatusRowEnvelope` codec (rkyv archive/access/deserialize) | direct call, default-lane | S-V2 |

No new driving ports — `serve` + `deploy` are existing verbs gaining the
inbound-capture behaviour they were missing.

---

## Scenario set

### S-WS — Keystone walking skeleton (Tier-3, mandatory #241 acceptance gate)

`@walking_skeleton @driving_port @real-io @tier3 @kpi-none`

```gherkin
Scenario: A workload reached at its canonical address terminates mTLS end to end
  Given the node has been brought up through its production boot composition root
  And a server workload offering a service on its declared port has been deployed through the production deploy submit handler
  And a client workload in the same mesh has been deployed through the production deploy submit handler
  When the client workload dials the server workload at its canonical workload address and declared service port — directly, with no name lookup
  Then the node's own production inbound capture (installed when the server workload started) diverts the dial to the server's transparent listener
  And the connection is authenticated with mutual TLS
  And the client's request bytes arrive at the server workload byte-for-byte
  And the server's reply bytes return to the client byte-for-byte
```

- **Observable outcome:** the client's application request reaches the server
  workload and its reply returns — a complete mTLS-terminated round-trip reached
  *by canonical workload address*, driven through the **production composition
  root in-process** (real `run_server` boot + real in-process deploy submit
  handler) with **no test-only wiring**. The keystone drives the **real
  production `EbpfDataplane`** — there is **no `dataplane_override`**
  (`dataplane_override = None`). This is load-bearing, not incidental: the
  production boot gate `compose_mtls = config.dataplane_override.is_none() ||
  config.mtls_probe_fault.is_some()` (`overdrive-control-plane/src/lib.rs:1824`)
  switches the **mTLS worker OFF** whenever a `dataplane_override` is present,
  and the only override-compatible compose path (`mtls_probe_fault`) forces a
  fail-closed boot refusal — so a `dataplane_override` and a working mTLS worker
  cannot coexist. The keystone needs the composed mTLS worker for the leg-B
  handshake, so it MUST use the real dataplane. The **only** test seam injected
  at the in-process composition boundary is the `mtls_identity_override` test-PKI
  source (read on the `compose_mtls = true` path for the leg-B handshake at
  `lib.rs:1841-1845`); everything else is production wiring.
- **Litmus is the transitive round-trip — no map inspection.** A successful
  canonical-address mTLS round-trip IS the proof that the LB gate fell through to
  nft-TPROXY: a gated mesh backend MISSES `LOCAL_BACKEND_MAP` (so the dial is not
  short-circuited by the cgroup LB path) and the production-installed inbound
  rule diverts it. The keystone asserts on the round-trip, NOT on
  `LOCAL_BACKEND_MAP` contents — exactly the transitive-proof model the adapter
  coverage section already states (see "their **real** kernel consequence is
  proven transitively by S-WS …" in § "Adapter coverage" below). That paragraph
  stays as the load-bearing proof model; this bullet is its statement at the
  keystone.
- **What #241 REMOVES from the existing skeleton
  (`bidirectional_walking_skeleton.rs`):** the test-installed
  `install_inbound_tproxy(virt, leg_c_port)` redirect AND the synthetic
  loopback `INBOUND_VIRT_IP`/`INBOUND_VIRT_PORT` virt. The keystone captures on
  the **production-installed** rule keyed on `ip daddr <workload_addr> tcp dport
  <service_port>` — the rule `start_alloc` now installs from
  `spec.{workload_addr, service_ports}`.
- **The gate (CLAUDE.md § "Build vertical slices through production entry
  points"):** no integration test installs the inbound rule, supplies the
  address, or stands in for the production call site. The C3 seam supplies
  `workload_addr`; `WorkloadLifecycle::project_service_listen_ports` supplies
  `service_ports`; `start_alloc` installs the rule. If the production install
  is missing, the dial is **not captured** and the round-trip fails — exactly
  the signal the keystone exists to give.
- **Pinned-6.18 Tier-3 matrix (DELIVER obligation #3 — MERGE-BLOCKING):** the
  spike verdicts are on dev-Lima kernel 7.0; the authoritative signal is the
  pinned-6.18 appliance kernel (ADR-0068). The DELIVER roadmap AC must assert
  the bidirectional mesh loop passes the **pinned-6.18 Tier-3 matrix**, not
  merely "tests pass." Dev-Lima 7.0 is necessary-but-not-sufficient (the
  built-in-ca-operator-composition cold-boot regression is the precedent for an
  "expected to work on 6.18" change that did not).
- **`E`-surface verification-catalogue note (per `.claude/rules/verification.md`):**
  S-WS graduates into the verification catalogue as
  `verification/expectations/E04-workload-reachable-at-canonical-address-mtls/`
  (an `E`-surface expectation: "a workload is reachable at its canonical address
  over mTLS"). The catalogue entry **IS authored in DELIVER** (03-02) — but only
  as a `pending` stub: the `README.md` (scenario + `- Anchor:` lines + a
  `verification` block + `Status: pending`), a `runner.sh` skeleton, and the
  `INDEX.md` row. Its real **black-box `overdrive serve` + `overdrive deploy`
  subprocess evidence capture is DEFERRED** — and the deferral STANDS, but its
  technical grounds are corrected here. The earlier premise ("`overdrive
  serve`'s `EbpfDataplane` XDP attach to `lo` fails at boot on dev-Lima") is
  **FALSE** per committed O03 evidence: post-ADR-0061, `serve` no longer attaches
  XDP to `lo` — it auto-provisions a `ovd-veth-cli`/`ovd-veth-bk` veth pair and
  binds there, so `serve` boots cleanly and `deploy` exits 0 on dev-Lima. The
  honest deferral grounds are that the **black-box** mesh-mTLS `E`-surface
  capture (real `serve` + real `deploy`×2, **no test PKI**) requires a converged
  full-system two-workload deployment AND the production CA→SVID→leg-C mTLS path
  proven black-box (no `mtls_identity_override` seam) — which is precisely what
  **GH #227** (EDD harness: a disposable full-system Lima VM on the immutable OS
  for end-to-end captures) provides, on **GH #75** (the Image Factory MVP that
  produces the immutable node OS image #227 needs). DELIVER authors the stub
  `pending` and does NOT capture/satisfy it; the subprocess capture lands when
  #227/#75 unblock the EDD harness.
- **Placement:** `crates/overdrive-control-plane/tests/integration/canonical_address_inbound_walking_skeleton.rs`, wired into `crates/overdrive-control-plane/tests/integration.rs`. The keystone lands in the **control-plane** test tree because in-process `run_server` / `ServerConfig` / `mtls_identity_override` all live in `overdrive-control-plane` (`src/lib.rs`), and `overdrive-control-plane` depends-on `overdrive-worker` — a reverse edge is a Cargo-rejected cycle, so a worker-crate test physically **cannot** reach in-process `run_server`. The direct in-repo precedent (`backend_discovery_bridge/walking_skeleton.rs`) lives in this same control-plane test tree. The **synthetic-virt removal** (deleting the test-installed `install_inbound_tproxy(virt, leg_c_port)` and the `INBOUND_VIRT_IP`/`INBOUND_VIRT_PORT` loopback virt) STAYS in `crates/overdrive-worker/tests/integration/bidirectional_walking_skeleton.rs` (worker tree); the existing skeleton's body is otherwise **not modified** beyond de-wiring the moved scaffold.
- **Strategy:** the **production composition root in-process** — real `run_server`
  boot + the real in-process deploy submit handler for the two mesh workloads,
  capturing on the 03-01 production-installed inbound rule. Direct in-repo
  precedent: `crates/overdrive-control-plane/tests/integration/backend_discovery_bridge/walking_skeleton.rs`
  (drives the production boot composition root in-process, not a subprocess).
  This is NOT a `#[test]` that hand-assembles `start_alloc` — `run_server` + the
  deploy submit handler ARE the production composition root; the litmus is
  preserved (delete the 03-01 production install and the keystone goes RED —
  the dial is not captured and the round-trip fails). The keystone runs the
  **real production `EbpfDataplane`** (NO `dataplane_override`) so the boot gate
  `compose_mtls` composes the mTLS worker; the **only** test seam injected at the
  in-process composition boundary is the `mtls_identity_override` test PKI.
  Requires root + `CAP_NET_ADMIN`/`CAP_SYS_ADMIN`; a non-root run SKIPs.
  `uname -r` recorded; the merge-blocking signal is the pinned-6.18 appliance-kernel
  Tier-3 matrix (DELIVER obligation #3).

> **Reconciliation note (2026-06-23 — corrects the original subprocess mandate).**
> S-WS was authored mandating real `overdrive serve` + `overdrive deploy`
> subprocesses (grounded in the RCA-P1 driving-adapter requirement + the spike
> increment-c precedent). At execution time that collided with two
> higher-priority project rules, so the keystone is reshaped to drive the
> production composition root **in-process** (Option A):
>
> 1. **`crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess"** —
>    a firm rule: do not spawn `overdrive` as a subprocess in tests; call the CLI
>    command handlers directly as Rust functions. The "invoke the binary via
>    `Command::spawn`" pattern is explicitly rejected for this crate.
> 2. **`CLAUDE.md` § "Implement to the design — never invent API surface"** — the
>    test-PKI seam (`mtls_identity_override` on `run_server` / `ServerConfig`)
>    that makes the mesh mTLS round-trip work is reachable **only in-process**. A
>    real `serve` subprocess would use the production workload CA, against which
>    the test workloads hold no SVID, and wiring a test trust bundle into
>    `overdrive serve` would require inventing test-only production CLI surface
>    (forbidden).
>
> The in-process composition root honours both rules, invents **zero** new
> production API, and PRESERVES THE LITMUS. The full black-box `serve` + `deploy`
> subprocess proof is not dropped — it graduates into the `verification/`
> catalogue as `E04` (authored `pending` in DELIVER; subprocess capture deferred
> to #227/#75, per the verification-catalogue note above).
>
> **R1 (2026-06-23 — keystone relocates to the control-plane test tree).** The
> in-process choice above surfaced a hard crate-graph fact at execution: in-process
> `run_server` / `ServerConfig` / `mtls_identity_override`
> all live in `overdrive-control-plane` (`src/lib.rs`), and `overdrive-control-plane`
> depends-on `overdrive-worker` (runtime dep). A reverse edge is a Cargo-rejected
> cycle, so a **worker-crate test physically cannot reach in-process `run_server`**.
> The keystone test therefore relocates from
> `crates/overdrive-worker/tests/integration/` to
> `crates/overdrive-control-plane/tests/integration/canonical_address_inbound_walking_skeleton.rs`
> — the ONLY crate from which in-process `run_server` is reachable, and the same
> tree as the named precedent
> `crates/overdrive-control-plane/tests/integration/backend_discovery_bridge/walking_skeleton.rs`.
> The **synthetic-virt REMOVAL** (the test-installed `install_inbound_tproxy(virt,
> leg_c_port)` + the `INBOUND_VIRT_IP`/`INBOUND_VIRT_PORT` loopback virt) STAYS in
> the worker tree (`bidirectional_walking_skeleton.rs`), where that code lives.
> Nothing else about Option A changes — same litmus, same `mtls_identity_override`
> test-PKI injection (and NO `dataplane_override` — the keystone runs the real
> `EbpfDataplane` so `compose_mtls` composes the mTLS worker; see the S-WS
> observable-outcome bullet), same E04 `pending` stub.
>
> **R2 (2026-06-23 — keystone uses the real `EbpfDataplane`; drop the
> contradictory `dataplane_override`).** Option A's original wording injected
> BOTH `mtls_identity_override` AND a `dataplane_override` at the in-process
> composition boundary. That is **contradictory with the production code**: the
> boot gate `compose_mtls = config.dataplane_override.is_none() ||
> config.mtls_probe_fault.is_some()` (`overdrive-control-plane/src/lib.rs:1824`)
> switches the **mTLS worker OFF** when a `dataplane_override` is present, and the
> only override-compatible compose path (`mtls_probe_fault`) forces a fail-closed
> boot refusal — so a `dataplane_override` and the working mTLS worker the
> keystone depends on cannot coexist. The fix: the keystone runs with the **real
> production `EbpfDataplane`** (`dataplane_override = None`) so `compose_mtls =
> true` composes the mTLS worker, and KEEPS only `mtls_identity_override =
> Some(TestPki)` (read on the compose-true path for the leg-B handshake). The
> litmus is proven **transitively** — the design already states this in the
> adapter-coverage section (~L424-432): a gated mesh backend must MISS
> `LOCAL_BACKEND_MAP` so the dial falls through to nft-TPROXY, which is exactly
> what a successful canonical-address capture requires. So a successful mTLS
> round-trip IS the proof; no `LOCAL_BACKEND_MAP` inspection, no
> `dataplane_override`. This invents ZERO production API.

---

### S-NRULES — N listeners install exactly N inbound rules (Tier-3, real nft)

`@real-io @tier3 @us-A1`

```gherkin
Scenario: A service with two declared ports installs an inbound capture for each
  Given a server workload deployed with two declared service ports
  When the workload starts on the node
  Then the node installs exactly two inbound capture rules
  And each rule matches the workload's canonical address on one of the two declared ports
  And both capture rules are released when the workload is torn down
```

- **Observable outcome:** the live nft ruleset carries exactly 2 per-virt
  capture rules for the 2-listener Service, keyed `ip daddr <workload_addr> tcp
  dport <port_i>`; 2 RAII guards retained, dropped on teardown (no leftover nft
  state after the alloc ends).
- **D-A1 mapping:** N listeners → N inbound rules (the per-port
  `install_inbound_tproxy` loop in `start_alloc`).
- **Placement:** `crates/overdrive-worker/tests/integration/inbound_rules_per_listener.rs`.

---

### S-DPORT — Capture rule keys on the declared service port, not the ephemeral leg-C port (Tier-3, error/edge)

`@real-io @tier3 @us-BLOCKER1 @error`

```gherkin
Scenario: The inbound capture matches the port a peer actually dials
  Given a server workload deployed with one declared service port
  When the workload starts on the node
  Then the installed capture rule matches the declared service port
  And it does NOT match the agent's own ephemeral transparent-listener port
  And a peer dialing the workload's canonical address on the declared service port is captured
```

- **Observable outcome:** the installed nft rule's match `dport` is the
  **declared service port** (D-BLOCKER1, D-TME-10 one-source/two-readers), NOT
  the ephemeral `leg_c_addr.port()`. The inert self-referential shape the design
  rejected (a rule matching the agent's own leg-C port, which no real inbound
  connection targets) is structurally absent. A dial to
  `workload_addr:service_port` is captured; the rule's `tproxy to` target is the
  ephemeral leg-C port (the redirect destination, not the match key).
- **Why this is the error/edge guard:** it pins the negative — the rule must NOT
  key on the wrong (ephemeral) port. A mutant that keys the rule on
  `leg_c_addr.port()` instead of the declared port passes a naive "a rule was
  installed" check but fails this scenario.
- **Placement:** `crates/overdrive-worker/tests/integration/inbound_rule_keys_declared_port.rs`.

---

### S-JOB0 — A Job-kind alloc (0 listeners) installs 0 inbound rules (Tier-3, error/edge)

`@real-io @tier3 @error`

```gherkin
Scenario: A workload that offers no service installs no inbound capture
  Given a Job-kind workload deployed with no declared service ports
  When the workload starts on the node
  Then the node installs no inbound capture rules
  And no spurious capture diverts unrelated traffic
```

- **Observable outcome:** the live nft ruleset carries **zero** per-virt capture
  rules for the Job alloc (empty `service_ports` / `None` `workload_addr` → the
  host-netns/Job path, unchanged). No `TproxyInterceptGuard` retained.
- **Why this is the error/edge guard:** the `project_service_listen_ports`
  mirror returns `Vec::new()` for `Job`/`Schedule` (matching
  `project_probe_descriptors`); a mutant that installs an all-TCP or
  hardcoded-port rule for a Job fails this scenario.
- **Placement:** `crates/overdrive-worker/tests/integration/job_kind_installs_no_inbound_rule.rs`.

---

### S-BRIDGE — Bridge advertises the canonical address when present, host address otherwise (Tier-1 DST)

`@in-memory @us-B2`

```gherkin
Scenario: The bridge advertises a mesh workload by its canonical address
  Given a running allocation whose canonical workload address is known
  When the backend-discovery bridge reconciles
  Then the advertised backend address is the canonical workload address on the listener's port
  And the advertised service's virtual address is unchanged

Scenario: The bridge falls back to the host address for a host-netns workload
  Given a running allocation whose canonical workload address is absent
  When the backend-discovery bridge reconciles
  Then the advertised backend address is the host address on the listener's port
  And the advertised service's virtual address is unchanged
```

- **Observable outcome (driving port = `BackendDiscoveryBridge::reconcile`):**
  - `Some(workload_addr)` → emitted `Backend.addr == workload_addr:listener_port`.
  - `None` → emitted `Backend.addr == host_ipv4:listener_port` (fallback UNCHANGED).
  - `ServiceBackendRow.vip` UNCHANGED in **both** arms (the dialable-VIP path is
    #61 territory, orthogonal).
- **Universe (Mandate 8 — port-exposed observable names only; the
  reconcile-returned `Vec<Action>` + `View`, never the bridge's private
  fields):**
  `{actions.emitted.backend_addr, actions.emitted.service_vip, view.advertised_fingerprint}`.
  The `None`-fallback arm is the error/edge coverage (host-netns workload).
- **PBT mode (Mandate 9):** Tier-1 in-memory acceptance → PBT-eligible. The
  crafter MAY express this as a `proptest` over `{Some(addr) | None} ×
  listener_port` with `assert_state_delta`-shaped universe assertions; an
  `@example`-pinned canonical case (the `Some(10.99.0.6)` mesh row + the `None`
  host row) is preserved for the reviewer. Single-example fallback is acceptable
  if the proptest generator cannot express the two-arm split cleanly.
- **Placement:** `crates/overdrive-core/tests/canonical_address_bridge_advertise.rs` (default-lane standalone test binary, sibling to `backend_discovery_bridge_types.rs`).

---

### S-GATE — Hydrator gates mesh backends, leaves local and remote arms unchanged (Tier-1 DST)

`@in-memory @us-GATE`

```gherkin
Scenario: A mesh-subnet backend is programmed into neither load-balancer path
  Given a backend whose address is inside the workload subnet
  When the service-map hydrator reconciles
  Then it emits no local-backend registration
  And it emits no dataplane service update
  (nft-TPROXY owns delivery for this mesh workload)

Scenario: A host-address backend is still registered as a local backend
  Given a backend whose address equals the host address
  When the service-map hydrator reconciles
  Then it emits a local-backend registration (unchanged)

Scenario: A non-mesh, non-host backend still drives a dataplane service update
  Given a backend whose address is neither the host address nor inside the workload subnet
  When the service-map hydrator reconciles
  Then it emits a dataplane service update (unchanged)
```

- **Observable outcome (driving port = `ServiceMapHydrator::reconcile`),
  three-way split applied BEFORE the existing LOCAL/REMOTE partition (D-GATE,
  D-GATE-PRED):**
  - `addr.ip() ∈ WORKLOAD_SUBNET_BASE (10.99.0.0/16)` → emits **neither**
    `RegisterLocalBackend` **nor** `DataplaneUpdateService` (mesh → skip).
  - `addr == host_ipv4` → `RegisterLocalBackend` (UNCHANGED LOCAL arm).
  - otherwise → `DataplaneUpdateService` (UNCHANGED REMOTE arm).
- **Universe (Mandate 8):** `{actions.emitted.register_local_backend_count,
  actions.emitted.dataplane_update_service_count, view.programmed_fingerprint}`.
  The two non-mesh arms are the error/edge coverage — they prove the gate does
  **not over-fire** (a mutant that gates everything, or gates nothing, fails
  here).
- **PBT mode (Mandate 9):** Tier-1 → PBT-eligible over the three address
  classes; `@example`-pin a representative addr per arm
  (`10.99.0.6` mesh / `host_ipv4` local / `10.96.0.50` remote).
- **Placement:** `crates/overdrive-core/tests/mesh_backend_lb_gate.rs`.

---

### S-PORTSET — The capture port-set equals the advertise port-set (Tier-1 DST, property — DELIVER obligation #1)

`@in-memory @us-portset @property`

```gherkin
Property: Every port a workload is captured on is a port it is advertised on
  Given a service declaring an arbitrary non-empty set of listener ports (N ≥ 2)
  When the listen-port projection and the bridge advertise path both read that service's listeners
  Then the inbound-capture port-set equals the advertised port-set
  And no captured port is missing from the advertised set
  And no advertised port is missing from the captured set
```

- **Observable outcome:** for an N≥2-listener Service, the inbound-rule port-set
  (`project_service_listen_ports(intent)` → `AllocationSpec.service_ports`)
  **equals** the advertise port-set (the bridge reading `desired.listeners`
  ports). Assert **byte-set equality** (DELIVER obligation #1 — same intent
  source, two code paths, latent drift risk).
- **Universe (Mandate 8):** `{projection.service_ports_set, advertise.listener_ports_set}` with
  the invariant `projection == advertise`.
- **PBT mode (Mandate 9):** Tier-1 `@property` → PBT full. The crafter generates
  an arbitrary non-empty set of `NonZeroU16` listener ports (N ≥ 2) and asserts
  set equality across both read paths. This is the canonical "property over a
  domain-rich input space" case the `@property` tag signals.
- **Placement:** `crates/overdrive-core/tests/capture_advertise_port_set_equality.rs`.

---

### S-V2 — `AllocStatusRow` V2 envelope: V1 decodes through, V2 roundtrips (default-lane, schema-evolution)

`@property @schema-evolution`

```gherkin
Scenario: A pre-V2 stored allocation status still reads back correctly
  Given the pinned V1 golden bytes of a stored allocation status
  When the bytes are decoded through the current envelope and projected to the latest shape
  Then the projection carries no canonical workload address (absent by additive default)
  And every other field matches the canonical V1 payload

Scenario: A V2 allocation status carrying a canonical address roundtrips intact
  Given a V2 allocation status whose canonical workload address is present
  When it is archived, accessed, deserialized, and projected to the latest shape
  Then the projection equals the original byte-for-byte
```

- **Observable outcome (driving port = `AllocStatusRowEnvelope` codec):**
  - `FIXTURE_V1` golden bytes decode through the envelope + `into_latest()` to a
    V2 with `workload_addr: None` (additive `From<V1> for V2`).
  - A V2 payload with `Some(addr)` roundtrips archive → access → deserialize →
    `into_latest()` equal to the original.
- **Mandatory per `.claude/rules/testing.md` § "Archive schema-evolution
  roundtrip"** and `development.md` § "rkyv schema evolution" 6-step procedure:
  - `FIXTURE_V1` pinned **untouched** (existing fixture stays verbatim).
  - `FIXTURE_V2` added in the same commit (DELIVER fills the bytes via the
    `print_fixture_v1_bytes`-shaped regeneration aid).
  - `GOLDEN_DISCRIMINANT_OFFSET_V1` re-pinned via the triangulation test (adding
    `Option<Ipv4Addr>` — 4 bytes behind the `Option` discriminant — shifts the
    trailing root footprint; the offset is empirical, re-pinned on the bump).
- **Layer note (Mandate 9):** this is a default-lane codec roundtrip, not a
  layer-3 real-I/O test — PBT-eligible for the V2 `Some(addr)` arm (generate an
  arbitrary `Ipv4Addr`), example-pinned for the V1 golden-bytes arm (the fixture
  IS the pinned example).
- **Placement:** `crates/overdrive-core/tests/schema_evolution/alloc_status_row.rs` (the **existing** schema-evolution fixture file — the V2 scaffold is appended there; the file is wired via the existing `tests/schema_evolution.rs` `mod alloc_status_row;` entry).

---

## Error / edge ratio

| Scenario | Class |
|---|---|
| S-WS | happy (keystone) |
| S-NRULES | happy (multi-listener) |
| **S-DPORT** | **error/edge** (rule must NOT key on the ephemeral port) |
| **S-JOB0** | **error/edge** (0 listeners → 0 rules; no spurious capture) |
| S-BRIDGE (Some arm) | happy |
| **S-BRIDGE (None arm)** | **error/edge** (host-netns fallback) |
| **S-GATE (local arm)** | **error/edge** (gate must NOT over-fire — host arm unchanged) |
| **S-GATE (remote arm)** | **error/edge** (gate must NOT over-fire — remote arm unchanged) |
| S-GATE (mesh arm) | happy (the gate itself) |
| S-PORTSET | property (invariant) |
| S-V2 (V1-decodes) | error/edge (backward-compat — old bytes must still read) |
| S-V2 (V2-roundtrip) | happy |

Counting the discrete observable behaviours: **6 error/edge of 13 ≈ 46%** —
above the 40% mandate floor. (S-DPORT, S-JOB0, the S-BRIDGE `None` arm, the two
non-mesh S-GATE arms, and the S-V2 V1-backward-compat arm are all negative /
must-not-break / boundary cases.)

---

## Adapter coverage (Mandate 6 — every driven adapter mapped to ≥1 `@real-io` or `@property` scenario)

| Driven adapter | Path | Covered by | Tag |
|---|---|---|---|
| Inbound nft-TPROXY install (`install_inbound_tproxy` → `nft` / `ip` CLI) | `mtls_intercept.rs` (REUSE), called per-port from `start_alloc` | S-WS (real capture end-to-end), S-NRULES (N rules), S-DPORT (rule key), S-JOB0 (0 rules) | `@real-io @tier3` |
| Gated `Dataplane` / `ServiceMapHydrator` (the `register_local_backend` / `update_service` NOT called for mesh) | `service_map_hydrator.rs` (EXTEND) | S-GATE (three-way split, both non-mesh arms exercised) | `@in-memory` (Tier-1 reconciler) |
| `BackendDiscoveryBridge` advertise (`Backend.addr` source) | `backend_discovery_bridge.rs` (EXTEND) | S-BRIDGE (both arms), S-PORTSET | `@in-memory` / `@property` |
| `AllocStatusRow` rkyv codec (`AllocStatusRowEnvelope::V2`) | `observation_store.rs` (EXTEND) | S-V2 (V1 golden decode + V2 roundtrip) | `@property @schema-evolution` |

The inbound nft-TPROXY adapter is exercised with **real I/O** end-to-end by the
keystone (S-WS) and observed directly (real nft ruleset) by S-NRULES / S-DPORT /
S-JOB0 — the "real" bar (`.claude/rules/testing.md` Tier-3): the test would FAIL
if the production install were absent. The reconciler-logic adapters
(hydrator/bridge gate) are Tier-1 in-memory DST (the `reconcile()→(Vec<Action>,
View)` purity contract); their **real** kernel consequence is proven
transitively by S-WS (a gated mesh backend must MISS `LOCAL_BACKEND_MAP` so the
dial falls through to nft-TPROXY — exactly what S-WS's successful capture
requires).

---

## Driving-adapter coverage (production composition root / CLI entry — RCA P1)

| Driving adapter | Protocol | Scenario |
|---|---|---|
| `run_server` (the `overdrive serve` boot composition root) | in-process production boot composition root | S-WS |
| in-process deploy submit handler (the `overdrive deploy <SPEC>` handler, called directly as a Rust function — `overdrive-cli/CLAUDE.md` § "no subprocess") | in-process production deploy submit handler | S-WS (×2 deploys), S-NRULES / S-DPORT / S-JOB0 (deploy a spec, observe the installed rule) |

S-WS exercises the full operator invocation path through the **production
composition root in-process** (real `run_server` boot + the real in-process
deploy submit handler), not a `#[test]` that assembles `start_alloc` by hand —
satisfying CLAUDE.md § "Build vertical slices through production entry points"
and the RCA-P1 driving-adapter requirement (an in-process `run_server` + deploy
submit handler IS a production composition root, not hand-assembled
`start_alloc`). The subprocess shape was relaxed to honour
`overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess" and CLAUDE.md
§ "never invent API surface" (the `mtls_identity_override` test-PKI seam is
in-process-only) — see the reconciliation note under S-WS. The Tier-3 supporting
scenarios (S-NRULES/S-DPORT/S-JOB0) MAY observe the installed rule via a real
in-process deploy + nft dump, or (crafter's discretion under the determinism
contract) drive `start_alloc` directly through the production worker seam if a
full boot + deploy per scenario is too costly — but the **rule install itself
must be the production call site**, never a test-installed
`install_inbound_tproxy`.

---

## DELIVER-obligation → scenario map (the 5 obligations from `design/wave-decisions.md`)

| # | Obligation | Scenario(s) / note |
|---|---|---|
| **1** | Port-set equality AC — `project_service_listen_ports` set **equals** the bridge advertise set for an N≥2 Service | **S-PORTSET** (`@property`, byte-set equality) |
| **2** | Pin two internal wiring seams in the crafter dispatch: (a) `hydrate_actual` `RunningAllocSet.running` `BTreeSet → BTreeMap<…, Option<Ipv4Addr>>` population; (b) `service_ports` threaded at the identical site/shape as `probe_descriptors` (confirm `obs.alloc_status_rows()` already carries the V2 row → no new `ObservationStore` method) | Not a test obligation — a **dispatch-pinning** note carried into DELIVER. S-BRIDGE exercises the `RunningAllocSet` map read transitively (the bridge reads `actual.running[alloc]`); S-V2 confirms the V2 row is the observation surface. **Flagged for the crafter dispatch — pin both seams, do not improvise.** |
| **3** | Pinned-6.18 Tier-3 AC — the bidirectional mesh loop passes the **pinned-6.18 appliance-kernel Tier-3 matrix** (ADR-0068), MERGE-BLOCKING, not merely "tests pass" | **S-WS** (noted in the scenario: dev-Lima 7.0 is necessary-but-not-sufficient; the DELIVER roadmap AC must name the pinned-6.18 matrix) |
| **4** | Crate-path nit — `mtls_resolve_adapter.rs` is in `overdrive-control-plane`, not `overdrive-worker` | FIXED in DESIGN; no test obligation. Recorded so a scaffold doc-comment does not re-introduce the wrong crate qualifier. |
| **5** | Rustdoc on `AllocStatusRowV2.workload_addr` naming it a materialized `slot × base-at-provision` join + the #239 single-cut constraint (a base change is a redeploy, not a live re-tune) | Production-code doc obligation (DELIVER, `src/`). **NOT a DISTILL deliverable** (DISTILL does not touch `src/`). S-V2's scaffold doc-comment flags the obligation so DELIVER carries it onto the field. |

---

## Prerequisites

- **`integration-tests` feature** on `overdrive-worker` (Tier-3 scenarios
  S-WS/S-NRULES/S-DPORT/S-JOB0 are gated behind it, wired through
  `tests/integration.rs`). Already declared on the crate.
- **Pinned-6.18 Tier-3 matrix** (ADR-0068) — the merge-blocking signal for S-WS
  (DELIVER obligation #3). Dev-Lima execution
  (`cargo xtask lima run -- cargo nextest run -p overdrive-worker --features
  integration-tests`) is the inner loop; the appliance-kernel matrix is the gate.
- **Root + `CAP_NET_ADMIN`/`CAP_SYS_ADMIN`** for the Tier-3 scenarios (nft, `ip
  netns`, `ip rule`, `IP_TRANSPARENT`); a non-root run SKIPs.
- **Default lane** (no feature gate) for the Tier-1 DST scenarios
  (S-BRIDGE/S-GATE/S-PORTSET) and the schema-evolution scenario (S-V2) — pure
  in-process Rust, `cargo nextest run -p overdrive-core` (Lima-routed per
  `.claude/rules/testing.md`).

---

## Reconciliation (HARD GATE) result

This feature has **no `discuss/` or `devops/` dir** (it started at SPIKE per the
dispatch). The only wave artifacts are DESIGN (`design/wave-decisions.md` +
`feature-delta.md`) and the three `spike/findings*.md`. There is no
DISCUSS↔DESIGN↔DEVOPS triad to cross-check. The three spikes are mutually
reconciled (increment-b: the LB cgroup hook FIRES → cannot retire; increment-c:
the VIP/LB path is INERT → GATE is sufficient, TEACH unnecessary; increment-a:
the inbound capture recipe is the existing production triple → no new
primitive). **Zero contradictions — Reconciliation passed.**
