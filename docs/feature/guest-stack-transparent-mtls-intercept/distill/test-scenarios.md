# DISTILL test-scenarios — guest-stack-transparent-mtls-intercept (GH #222)

**Specification only — NOT parsed, NOT executed.** Per `.claude/rules/testing.md`
§ "Testing" this repo BANS `.feature` files: the Gherkin GIVEN/WHEN/THEN blocks
below are the human-readable behavioural spec; the EXECUTABLE acceptance tests
are Rust `#[test]`/`#[tokio::test]` under `crates/*/tests/` (see § "Executable
AT map" at the foot of this file). Scope: EGRESS-first #222 (inbound +
service-kind = #257, OUT). Companion ADRs: ADR-0088 (topology + addressing),
ADR-0089 (provisioning boundary + CH net attach). Spike: `../spike/findings.md`
(verdict WORKS, kernel 7.0.0-29, CH v53.0, no toggle).

## Wave-Decision Reconciliation (HARD GATE) — PASSED

- DISCUSS `wave-decisions.md`: **absent** (this feature ran SPIKE → DESIGN; the
  spike DISCARD-to-DESIGN skipped a DISCUSS wave) → WARNING, not a blocker.
- DESIGN `wave-decisions.md`: present, read.
- DEVOPS `wave-decisions.md`: **absent** → WARNING; default env matrix
  (clean / with-pre-commit / with-stale-config) not materially applicable — the
  execution surface is fixed by the `kvm-tests` metal gate, not an env matrix.
- Contradiction scan (SPIKE ↔ DESIGN): the DESIGN implements the spike's WORKS
  topology (routed two-/30) **verbatim** — zero contradictions.

**Result: Reconciliation passed — 0 contradictions.**

---

## DISTILL shape pins (Q1–Q9) — models pinned upstream, exact shapes pinned here

Per CLAUDE.md § "Implement to the design": these pin ONLY the shapes ADR-0088 /
ADR-0089 / feature-delta Q1–Q9 already SANCTION. No new public API is invented
beyond them. The exact struct field names / method names of Q1/Q2/Q3/Q8 remain
DELIVER implementation shapes (the ATs observe them behaviourally, not by name);
where a name IS pinned it is transcribed from the design, not chosen. **No
underspecified-and-unsanctioned shape was found — no BLOCKER surfaced.**

| # | Design model (upstream) | DISTILL pin | Observed by |
|---|---|---|---|
| Q1 | Guest-net channel on `AllocationSpec` (pure in-memory, no-serde, sibling of `netns`/`host_veth`/`workload_addr`) | Exact field family name is a **DELIVER shape**; the ATs never name it — they observe `workload_addr` (= guest addr) at the operator surface and the composed egress behaviour | S-GTI-07 (behavioural) |
| Q2 | `VmConfig` fold so "netns without NIC" is unrepresentable for mesh VM allocs | Exact struct shape (fold `netns`+net-attach into one `Option`) is a **DELIVER shape**; ATs observe the composed CH launch behaviourally | S-GTI-01 (behavioural) |
| Q3 | ONE platform-owned cmdline parameter `(guest_addr/prefix, gateway, dns)`, parsed by `overdrive-init`, never the kernel | Grammar is a **DELIVER shape** (proposed `overdrive.net=<addr>/<prefix>,gw=<gw>,dns=<dns>`, single space-free token, opaque to the kernel); ATs observe that the guest is addressed + resolves by name | S-GTI-01 (behavioural) |
| Q4 | MAC = locally-administered unicast, pure fn of the slot | **PINNED (pure-derive property):** byte0 has the unicast bit clear (`b0 & 0x01 == 0`) and the locally-administered bit set (`b0 & 0x02 == 0x02`); the low bytes encode the slot; distinct slots ⇒ distinct MAC. Exact 6-byte layout beyond those invariants is a DELIVER shape | S-GTI-11 (pure PBT) |
| Q5 | Tap name `ovd-tp-<4hex-slot>` | **PINNED** — 11 chars, IFNAMSIZ-safe, sibling of `ovd-hv-`/`ovd-wl-`; `ovd-tp-` prefix + `NetSlot::to_hex4()` | S-GTI-09 (pure PBT) |
| Q6 | Guest /30 = `WORKLOAD_SUBNET_BASE + 0x8000 + slot*4`; add the symmetric guest-carve const guard | **PINNED:** guest network = `base.network() + 0x8000 + slot*4`; tap gateway = 1st usable; guest addr = 2nd usable. **DELIVER const guard** (compile-time, DELIVER authors it — NOT this wave): `const _: () = assert!((0x8000 + NET_SLOT_MAX as u32*4 + 3) < base_span)` beside the S6 guard (`veth_provisioner.rs:518`). S-GTI-10 is its runtime companion | S-GTI-10 (pure PBT) |
| Q7 | Pre-exec net-apply `EXIT` must be host-distinguishable from a normal non-zero operator exit | **PINNED (model, no new PL field):** the host beacon session distinguishes structurally — an `EXIT` received while the session is in the **post-READY / pre-EXEC** state is a provisioning/net-apply failure, NOT a crashed operator command. Reconciles ADR-0088's "beacon PL unchanged": `BeaconMessage` is UNCHANGED; the disambiguation is the host state-machine arm (which EXEC-release makes reachable, see Q9). Exact host-arm wiring = DELIVER shape | S-GTI-08 (behavioural) |
| Q8 | Sanctioned `KernelCmdline` compose/append surface for the ONE net parameter | Exact method name is a **DELIVER shape**; the ATs observe that the composed cmdline carries the platform net token (Q3) and the kernel never interprets it (`overdrive-init` does) | S-GTI-01 (behavioural) |
| Q9 | Born-captured ORDERING INVARIANT `intercept-install-success ≺ beacon-EXEC-release` | **PINNED (mechanism, no new vsock connect):** realise EXEC-release as the **REPLY on the guest-INITIATED beacon connection** (the existing `BeaconSession.write_half` + `BeaconMessage::Exec`), **deferred** until `start_alloc` succeeds. TODAY (`vm_driver.rs:917-951`) the `EXEC` write fires INSIDE `driver.start()`'s beacon-win arm, BEFORE the action-shim `Running` arm runs `start_alloc` — that is the first-connect window. DELIVER moves the `EXEC` reply to AFTER intercept-install-success; on install `Err` the reply is never sent (session drops → guest never execs → no cleartext). No host-initiated `connect()` into the guest (the microVM spike flagged host→guest vsock as unproven). Exact wiring = DELIVER shape | S-GTI-02 (behavioural) |

### The born-captured ordering state machine (C2 — documented here, pinned upstream)

```
                       guest dials beacon (vsock)              host accepts
  [boot] ───────────────────────────────────────────────────▶ [READY-received]
                                                                     │
                          guest applies cmdline net-addressing (Q3, fail-closed)
                                                                     │
                   net-apply FAILS  ┌──────────────────────────────┐ net-apply OK
                   guest sends EXIT  │                              │ guest waits for EXEC
                   (pre-EXEC)        ▼                              ▼
              [EXIT-before-EXEC] ── Q7 ──▶ terminal          [post-READY/pre-EXEC]
              provision/boot failure                                │
              (NOT restart-looped)                    action-shim: start_alloc
                                                       (install egress TPROXY, D6)
                                                                     │
                                    install Err  ┌──────────────────┤ install OK
                                    (D-MTLS-18)   ▼                  ▼
                                             [terminal Failed]  host sends EXEC reply
                                             guest never execs  (Q9 deferred release)
                                             no cleartext              │
                                                                       ▼
                                                            guest execs operator cmd
                                                            first connect() is BORN captured
```

Legal transitions, illegal-events, and terminal exits: every arm above has an
AT below (S-GTI-02, -05, -06, -08).

---

## Contract-shape + Outcome Elevator Pitch per scenario (2026-05-15 mandate)

Every scenario carries a `@contract-shape:<...>` tag. The Elevator Pitch uses
ubiquitous-language verbs (no "returns 200"/"exit 0"/"calls X once") and
propagates verbatim into the Rust test name.

| ID | Tags | Contract shape | Outcome Elevator Pitch (domain verbs) |
|---|---|---|---|
| S-GTI-01 | `@walking_skeleton @driving_port @real-io @kvm` | bounded-change | A microVM workload dials a mesh peer BY NAME and receives the peer's reply |
| S-GTI-02 | `@walking_skeleton @driving_port @real-io @kvm @property` | unbounded-preservation | The guest's very first mesh dial is born intercepted — no cleartext reaches the mesh |
| S-GTI-03 | `@real-io @kvm @wire-assertion` | unbounded-preservation | The guest's mesh traffic travels the peer wire as mTLS, never in the clear |
| S-GTI-04 | `@real-io @kvm` | bounded-change | The same guest reaches a non-mesh destination in the clear, unchanged |
| S-GTI-05 | `@real-io @kvm @error` | bounded-change | When the mesh guard cannot be installed, the workload is refused, never run in the clear |
| S-GTI-06 | `@real-io @kvm @error @restart` | bounded-change | A restarted microVM workload is re-enrolled in the mesh before it runs again |
| S-GTI-07 | `@real-io @kvm` | bounded-change | The operator sees the microVM workload's own mesh address, not its transit hop |
| S-GTI-08 | `@real-io @kvm @error` | bounded-change | A microVM that cannot address its network is refused as a boot failure, not retried forever |
| S-GTI-09 | `@property @in-memory` | pure-function | Each microVM slot names its own tap device, collision-free |
| S-GTI-10 | `@property @in-memory` | pure-function | Each microVM slot owns a mesh address disjoint from its transit hop and inside the mesh block |
| S-GTI-11 | `@property @in-memory` | pure-function | Each microVM slot carries its own locally-administered NIC identity |
| S-GTI-12 | `@real-io @kvm @teardown` | bounded-change | A stopped microVM workload's egress mesh guard is torn down, never left behind |

`@property` (Mandate 9) means: at layers 1-2 (S-GTI-09/-10/-11 pure derivation)
DELIVER GREEN-phase converts the scaffold to a Hypothesis-equivalent `proptest!`
over the `NetSlot` domain. At layers 3+ (S-GTI-01..08, real KVM boot) the
`@property` on S-GTI-02 is example-pinned, NOT PBT-generated (Mandate 11 — layer
3+ sad/invariant paths stay example-based).

---

## Tier-3 metal scenarios (S-GTI-01..08) — EGRESS through real `serve` + `deploy`

**Execution surface (ADR-0088/0089, iteration-1 HIGH):** all S-GTI-01..08 boot a
REAL Cloud-Hypervisor microVM — a netns cannot model "no host `struct sock`"
(the spike's whole point) — so they require **nested KVM** and run under
`cargo xtask metal run --` on the x86_64 metal box, gated behind
`#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]`. **NOT
Lima** (arm64 Lima has no nested KVM; a Lima run returns no signal). This wave
CANNOT execute them — they are scaffolded RED and **Tier-3 metal-deferred** (see
`red-classification.md`).

**East-west mTLS corollary (CLAUDE.md — a known trap with an RCA):** in
S-GTI-01/02/03 the guest/test **DIALER speaks PLAINTEXT** (`TcpStream` connect,
byte-distinct REQUEST/RESPONSE litmus). The mTLS is proven on the INTER-AGENT
(leg-B ↔ leg-C) wire (`0x17` TLS 1.3 application_data, zero cleartext), NOT on
the client handshake. DELIVER MUST NOT copy the inbound keystone's "client
presents TLS" dial shape onto this egress test — leg-F is the plaintext
workload-facing leg; a rustls dial would open a peerless TLS session leg-F never
terminates and stall → RST.

```gherkin
@walking_skeleton @driving_port @real-io @kvm @contract-shape:bounded-change
Scenario: S-GTI-01 — A microVM workload dials a mesh peer by name and receives the reply
  Given an mTLS-composed "overdrive serve" is running with the DNS responder up
    And a mesh "[service]" peer is deployed on the node and reachable by its name
  When the operator deploys a "[vm]"+"[job]" whose guest command dials that peer BY NAME
    And the guest sends a byte-distinct plaintext REQUEST over an ordinary socket
  Then the guest receives the peer's byte-distinct plaintext RESPONSE
    And guest dial-by-name resolved over the routed hops (guest resolv.conf -> responder -> resolve -> dial)
    And the workload's allocation reaches Running

@walking_skeleton @driving_port @real-io @kvm @property @contract-shape:unbounded-preservation
Scenario: S-GTI-02 — The guest's very first mesh dial is born intercepted (no cleartext escapes)
  Given an mTLS-composed "overdrive serve" is running
  When the operator deploys a "[vm]"+"[job]" whose guest command's FIRST action is a mesh dial by name
  Then the guest's first connection is captured by the egress intercept
    And ZERO cleartext SYN for the mesh destination ever leaves for the peer before the intercept rule is live
    And the ordering invariant install-success-precedes-EXEC-release held (the guest could emit no egress before the rule)

@real-io @kvm @wire-assertion @contract-shape:unbounded-preservation
Scenario: S-GTI-03 — The guest's mesh traffic travels the peer wire as mTLS, never in the clear
  Given a microVM guest is dialing a mesh peer through the composed egress intercept
  When the connection carries the guest's request and the peer's reply
  Then the inter-agent (leg-B to leg-C) wire carries TLS 1.3 application_data records
    And that wire carries ZERO cleartext of the guest's plaintext REQUEST/RESPONSE litmus
    And the kTLS legs report kernel-TLS installed

@real-io @kvm @contract-shape:bounded-change
Scenario: S-GTI-04 — The same guest reaches a non-mesh destination in the clear
  Given a microVM guest whose mesh dials are intercepted
  When the guest dials a NON-mesh destination (an address outside the mesh membership block)
  Then the connection passes through in the clear (classified NonMesh)
    And the guest receives the non-mesh peer's reply unchanged

@real-io @kvm @error @contract-shape:bounded-change
Scenario: S-GTI-05 — When the mesh guard cannot be installed, the workload is refused
  Given an mTLS-composed "overdrive serve" where the egress intercept install will fail for a VM alloc
  When the operator deploys a "[vm]"+"[job]"
  Then the allocation is driven to a terminal Failed state (fail-closed, D-MTLS-18 extended to VM kind)
    And the guest never runs the operator's command (EXEC-release never fired)
    And no cleartext egress ever left the guest

@real-io @kvm @error @restart @contract-shape:bounded-change
Scenario: S-GTI-06 — A restarted microVM workload is re-enrolled in the mesh before it runs again
  Given a Running mesh "[vm]"+"[job]" allocation whose egress intercept is installed
  When the allocation is restarted (crash-recovery / restart budget / "overdrive workload restart")
  Then the restarted allocation re-installs the egress intercept (the :1880 restart gate fired for VM kind)
    And when the re-install fails the restarted allocation is driven terminal fail-closed
    And a restarted VM alloc never runs cleartext fail-open

@real-io @kvm @contract-shape:bounded-change
Scenario: S-GTI-07 — The operator sees the microVM workload's own mesh address, not its transit hop
  Given an mTLS-composed "overdrive serve"
  When the operator deploys a "[vm]"+"[job]" and runs "overdrive workload describe"
  Then the workload's canonical address is the guest address (the upper-block guest /30 host)
    And it is NOT the transit /30 address (which carries no workload endpoint)

@real-io @kvm @error @contract-shape:bounded-change
Scenario: S-GTI-08 — A microVM that cannot address its network is refused as a boot failure
  Given an mTLS-composed "overdrive serve" where the guest's net-apply will fail before exec
  When the operator deploys a "[vm]"+"[job]"
  Then the allocation is driven to a terminal state classified as a provision/boot failure
    And it is NOT misattributed as a crashed operator command
    And it is NOT restart-looped (the restart budget is not consumed by a pre-exec net-apply failure)
    And the guest never ran the operator's command

@real-io @kvm @teardown @contract-shape:bounded-change
Scenario: S-GTI-12 — A stopped microVM workload's egress mesh guard is torn down, never left behind
  Given a Running mesh "[vm]"+"[job]" allocation deployed via real "overdrive serve" + "overdrive deploy"
    And its egress intercept is installed (the VM alloc's "overdrive-mtls" nft rule is PRESENT)
  When the operator stops it via the real stop driving port ("overdrive workload stop")
  Then the VM alloc's "overdrive-mtls" nft rule is GONE (teardown fired for VM kind at the ungated-by-DriverType stop site :2038)
    And no OTHER alloc's intercept rule is disturbed (observed via a host nft list of the overdrive-mtls table, not by instrumenting the teardown line)
    And adding a DriverType::Exec gate to the teardown sites (:1269 / :2038) would red this AT — the teardown-ungated invariant is locked (mirror of S-GTI-06's :1880 install lock)
```

## Layer 1-2 pure-derivation scenarios (S-GTI-09..11) — `VmTapPlan` (default lane)

These reference the design-sanctioned CREATE-NEW value `VmTapPlan` (feature-delta
§ Component decomposition) + its slot-derive (sibling of
`derive_workload_netns_plan`). DELIVER GREEN-phase converts each scaffold to a
`proptest!` over the `NetSlot` domain (C1 boundary 0 / NET_SLOT_MAX; C3
cardinality). Runnable in Lima / the default lane once GREEN.

```gherkin
@property @in-memory @contract-shape:pure-function
Scenario: S-GTI-09 — Each microVM slot names its own tap device, collision-free
  Given any valid net slot in 0..=NET_SLOT_MAX
  When the VmTapPlan is derived for that slot
  Then the tap name is "ovd-tp-" followed by the slot's 4-hex form (11 chars, IFNAMSIZ-safe)
    And distinct slots yield distinct tap names

@property @in-memory @contract-shape:pure-function
Scenario: S-GTI-10 — Each microVM slot owns a mesh address disjoint from its transit hop
  Given any valid net slot in 0..=NET_SLOT_MAX
  When the VmTapPlan (guest /30) and the WorkloadNetnsPlan (transit /30) are derived for that slot
  Then the guest /30 is "WORKLOAD_SUBNET_BASE + 0x8000 + slot*4" (upper-half carve)
    And the tap gateway is the guest /30 first usable and the guest addr is the second usable
    And the guest /30 is strictly disjoint from the transit /30 for that slot
    And the guest /30 is strictly inside WORKLOAD_SUBNET_BASE (the /16 mesh-membership block)

@property @in-memory @contract-shape:pure-function
Scenario: S-GTI-11 — Each microVM slot carries its own locally-administered NIC identity
  Given any valid net slot in 0..=NET_SLOT_MAX
  When the VmTapPlan MAC is derived for that slot
  Then the MAC is unicast (byte0 low bit clear) and locally-administered (byte0 second bit set)
    And distinct slots yield distinct MACs
```

**DELIVER const-guard obligation (Q6 / Finding 4):** S-GTI-10 is the RUNTIME
companion to a compile-time guard DELIVER MUST add beside the S6 transit guard
(`veth_provisioner.rs:518`):
`const _: () = assert!((0x8000 + NET_SLOT_MAX as u32 * 4 + 3) < base_span)`.
This wave does NOT author the const (it is production code the crafter owns); it
authors the scenario that motivates it.

---

## AT-completeness self-audit (Phase 2.5 — 15-item mechanical checklist)

| Item | Verdict | Evidence / rationale |
|---|---|---|
| C1a empty/zero/min | PASS | S-GTI-09/10/11 boundary slot 0 |
| C1b partition boundary | PASS | S-GTI-10 slot NET_SLOT_MAX; `NetSlot::new` rejects > MAX (max+1); guest-carve const guard motivated |
| C2a state machine in docstring | PASS | born-captured ordering state machine documented above + in the metal AT module docstring |
| C2b illegal-event-per-state | PASS | S-GTI-08 (EXIT-before-EXEC from post-READY/pre-EXEC), S-GTI-05 (install-Err from post-READY) |
| C3 cardinality 0/1/N | PASS | S-GTI-09/10/11 PBT over the whole slot domain (0/1/N slots); S-GTI-04 (non-mesh) vs S-GTI-01 (mesh) dial classes |
| C4a apply-twice/idempotency | PASS | S-GTI-06 restart = re-install (`start_alloc` idempotent — tears prior down) |
| C4b inverse-op-without-prereq | PASS | **S-GTI-12 locks the teardown (the inverse op):** a stopped VM alloc's `overdrive-mtls` rule is GONE and no other alloc's rule is disturbed — a state-mutating teardown, NOT the earlier no-op stop-without-install framing. The ungated-by-`DriverType` invariant is regression-locked: adding a `DriverType::Exec` gate to `:1269`/`:2038` reds S-GTI-12 (feature-delta § [REF] D6 MIRROR hazard — the install-side mirror is S-GTI-06's `:1880` lock) |
| C5a mode-flag combos | PASS | mesh (S-GTI-01) vs non-mesh (S-GTI-04); Exec-vs-Vm driver-type gate exercised by S-GTI-01 (Vm) alongside the shipped exec path |
| C5b flag orthogonality | PASS | D6 gate flips at BOTH install sites (S-GTI-01 fresh `:1584` + S-GTI-06 restart `:1880`); teardown stays ungated-by-`DriverType` and is now concretely locked by **S-GTI-12** (`:1269`/`:2038` — a teardown `DriverType` gate reds S-GTI-12, the teardown twin of the S-GTI-06 install lock) |
| C6a malformed input | **GAP (documented)** | a malformed guest-addressing cmdline token → `overdrive-init` fail-closed is tied to the Q3 grammar (a DELIVER shape); the fail-closed OUTCOME is covered by S-GTI-08. AT_GAP_IN_DELIVERY_SCOPE, not spec-ambiguity |
| C6b each declared error triggered | PASS | S-GTI-05 (install Err → terminal), S-GTI-08 (net-apply Err → terminal boot-failure) |
| C6c closed error set | PASS (partial) | S-GTI-05/-08 assert the fail-closed arms reach terminal with no cleartext escape; the closed-set completeness is bounded by the two named failure modes |
| C7a degraded resource | **GAP (documented)** | resource-starvation under a real guest boot is metal-deferred; out of the egress-first slice, no production degraded-resource path in scope |
| C7b interruption mid-op | PASS | S-GTI-06 (restart / crash-recovery is the interruption class); Q9 install-before-EXEC-release is interruption-safe by construction |
| C7c concurrent actors | PASS | S-GTI-09/10/11 prove per-slot disjointness (tap name, /30, MAC) — the collision-free-by-construction property the net-slot registry relies on for concurrent allocs |

**Passing: 13 / 15 → verdict COMPLETE.** C4b flipped GAP→PASS in this
follow-up: S-GTI-12 locks the teardown-ungated invariant (feature-delta § [REF]
D6 MIRROR hazard — the teardown twin of S-GTI-06's `:1880` install lock). The two
remaining gaps (C6a malformed guest-addressing cmdline token, C7a
degraded-resource) are both `AT_GAP_IN_DELIVERY_SCOPE` (DELIVER-fillable or
out-of-egress-slice), NOT `SPECIFICATION_AMBIGUITY`: the C2 state machine
(born-captured ordering), C5 mode flags (D6 gate), C6 error contract (D-MTLS-18
fail-closed), and C7 concurrency (net-slot registry) are all specified upstream
in ADR-0088/0089 + feature-delta. C6a is recorded as an explicit DELIVER
carry-forward (F4, feature-delta § Wave: DISTILL). **No upstream routing /
CLARIFICATION_NEEDED.**

---

## Executable AT map (Rust — the REAL tests)

| Scenario | Rust test file | Test fn (name = Elevator Pitch) | Tier / gate | RED shape |
|---|---|---|---|---|
| S-GTI-01 | `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs` | `microvm_dials_a_mesh_peer_by_name_and_receives_the_reply` | Tier-3 metal (`kvm-tests`) | `#[should_panic(expected = "RED scaffold")]` |
| S-GTI-02 | (same) | `the_guests_first_mesh_dial_is_born_intercepted_no_cleartext_escapes` | Tier-3 metal | `#[should_panic]` |
| S-GTI-03 | (same) | `the_guests_mesh_traffic_travels_the_peer_wire_as_mtls_never_in_the_clear` | Tier-3 metal | `#[should_panic]` |
| S-GTI-04 | (same) | `the_same_guest_reaches_a_non_mesh_destination_in_the_clear` | Tier-3 metal | `#[should_panic]` |
| S-GTI-05 | (same) | `when_the_mesh_guard_cannot_be_installed_the_workload_is_refused` | Tier-3 metal | `#[should_panic]` |
| S-GTI-06 | (same) | `a_restarted_microvm_workload_is_re_enrolled_in_the_mesh_before_it_runs_again` | Tier-3 metal | `#[should_panic]` |
| S-GTI-07 | (same) | `the_operator_sees_the_microvm_workloads_own_mesh_address_not_its_transit_hop` | Tier-3 metal | `#[should_panic]` |
| S-GTI-08 | (same) | `a_microvm_that_cannot_address_its_network_is_refused_as_a_boot_failure` | Tier-3 metal | `#[should_panic]` |
| S-GTI-12 | (same) | `a_stopped_microvm_workloads_egress_mesh_guard_is_torn_down_never_left_behind` | Tier-3 metal (`kvm-tests`) | `#[should_panic(expected = "RED scaffold")]` |
| S-GTI-09 | `crates/overdrive-control-plane/src/veth_provisioner.rs` (`#[cfg(test)] mod guest_tap_plan_distill_scaffold`) | `each_microvm_slot_names_its_own_tap_device_collision_free` | layer-1 default lane | `#[should_panic]` + `VmTapPlan`/`derive_vm_tap_plan` `todo!()` |
| S-GTI-10 | (same) | `each_microvm_slot_owns_a_mesh_address_disjoint_from_its_transit_hop` | layer-1 default lane | `#[should_panic]` |
| S-GTI-11 | (same) | `each_microvm_slot_carries_its_own_locally_administered_nic_identity` | layer-1 default lane | `#[should_panic]` |

Every test fn name carries the Outcome Elevator Pitch verbatim (DISCUSS → DISTILL
scenario name → DELIVER test name trace, 2026-05-15 mandate).
