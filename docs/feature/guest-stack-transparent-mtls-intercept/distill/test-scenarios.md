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

- DISCUSS `user-stories.md`, `story-map.md`, and `wave-decisions.md`:
  **missing**. This feature ran SPIKE → DESIGN after the spike's
  DISCARD-to-DESIGN decision. Each missing path is recorded as a WARNING, not
  silently treated as empty input.
- SPIKE `findings.md` and `wave-decisions.md`: present, read. The real-guest
  routed two-/30 verdict is **WORKS**; it proves the mechanism, not the later
  boot-order or diagnostic contracts.
- DESIGN `wave-decisions.md`, ADR-0088, ADR-0089, the effective architecture
  brief, and the DESIGN portion of `feature-delta.md`: present, read. The
  Q7/Q9 amendment commits `563fe26c`, `5590af67`, and `29ab0bf7` reconcile in
  that order; `design/review-q7-remediation.md` iteration 3 is **APPROVED**.
- DEVOPS `wave-decisions.md`: **missing** → WARNING. The execution surface is
  nevertheless fixed upstream: `kvm-tests` via `cargo xtask metal run --` on
  nested-KVM x86_64 metal, not Lima.
- Product journey reconciliation: `enforce-transparent-mtls-on-the-wire.yaml`
  supplies the one-universal-proxy commitment and `run-a-vm-workload.yaml`
  supplies the guest READY/exit honesty contract. `kpi-contracts.yaml` has no
  feature KPI applicable to this slice. The journeys' “unbuilt/staged” wording
  is delivery status, not an alternative lifecycle design.
- Contradiction scan (product journeys ↔ SPIKE ↔ approved DESIGN): the DESIGN
  keeps the spike-proven routed topology and universal proxy, then adds the
  approved pre-READY and closed-observer contracts. No live prior-wave
  statement requires setup `EXIT` or permits pre-intercept guest frames.

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
| Q7 | READY is the post-network-initialization barrier; setup failure powers off before READY | **PINNED:** minimal-root init, token parsing, NIC-down verification, per-interface IPv6 disable/read-back, `arp_notify=0` write/read-back, static IPv4/route apply, and resolver write all precede READY. Any failure powers off before READY, emits no guest `EXIT`, never reaches Running or EXEC, and resolves through the existing pre-READY `VmmExited` start-rejection arm. After READY, `EXIT` is exclusively the operator command's result. Beacon PL is unchanged. | S-GTI-08 + its supporting pure properties |
| Q8 | Sanctioned `KernelCmdline` compose/append surface for the ONE net parameter | Exact method name is a **DELIVER shape**; the ATs observe that the composed cmdline carries the platform net token (Q3) and the kernel never interprets it (`overdrive-init` does) | S-GTI-01 (behavioural) |
| Q9 | Born-captured invariant `capture-ready ≺ VMM-spawn ≺ network-ready ≺ READY ≺ intercept-live ≺ EXEC-release ≺ operator-first-connect` | **PINNED:** an observation-only decorator arms all-EtherType tap and host-veth capture before real VMM spawn/NIC-up and correlates allocation id, slot, netns inode, both interface names/ifindices, guest MAC, and guest address. From capture-ready through the exact observed outbound-rule-live edge, zero guest-originated L2 frames are allowed; drops/overflow, malformed/truncated records, unknown direction/time/order, unexpected MAC, or uncertain identity fail conservatively. Capture remains active through the existing async EXEC reply. The first operator five-tuple must increment the exact host-veth rule, reach leg-F, traverse TLS/kTLS, and have no cleartext peer-path copy. The decorator never supplies production networking or success. | S-GTI-01/-02 (behavioural) |

### The born-captured ordering state machine (C2 — documented here, pinned upstream)

```
 C3 provision complete
          │
          ▼
 [capture-ready on exact tap + host-veth]
          │ release real VMM create
          ▼
 [guest boot / NIC still down]
          │
          ├─ init, token, NIC prerequisite, suppression/read-back,
          │  static-apply, or resolver failure
          │       ▼
          │  [poweroff before READY; no guest EXIT]
          │       ▼
          │  [existing pre-READY VmmExited start rejection]
          │       ▼
          │  [Failed: VmGuestExitUnreported] ──▶ [FinalizeFailed only]
          │
          └─ all guest setup succeeds with zero emitted L2 frames
                  ▼
              [READY; guest blocked awaiting EXEC]
                  │ action-shim start_alloc (D6)
                  ├─ install Err ──▶ [terminal Failed; no EXEC]
                  └─ install success + exact host-veth rule observed
                              ▼
                        [intercept-live]
                              │ existing async EXEC reply
                              ▼
                        [operator command]
                              │ first expected five-tuple
                              ▼
                        [rule increment → leg-F → TLS/kTLS]
                              │
                              └─ later EXIT = operator result only
```

There is no setup-failure `EXIT` arm. Legal transitions, illegal events, and
terminal exits are covered by S-GTI-01/-02/-05/-06/-08; S-GTI-12 locks the
inverse teardown transition.

Compatibility pins: Beacon PL and `BeaconMessage` remain byte-for-byte
unchanged; no new `ExitKind`, `VmmExit`, describe, or observation field is
allowed. The approved step 02-01/02-02 production mechanics remain: the
existing asynchronous guest-initiated EXEC reply, both D6 install sites, one
`start_alloc`, and driver-type-ungated teardown. Q9 strengthens only the
observation contract around the real VMM and exact live rule.

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
3+ sad/invariant paths stay example-based). S-GTI-08 additionally requires two
source-local Rust properties over pure helpers: total suppression admission and
exact terminal classification. Each live property carries the exact rustdoc
line `/// CONTRACT_SHAPE: pure-function.`; these support the metal example and
do not replace it.

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
    And the observation-only metal decorator is capture-ready on the exact allocation tap and host-veth before the real VMM is spawned
  When the operator deploys a "[vm]"+"[job]" whose guest command dials that peer BY NAME
    And the guest sends a byte-distinct plaintext REQUEST over an ordinary socket
  Then the guest receives the peer's byte-distinct plaintext RESPONSE
    And guest dial-by-name resolved over the routed hops (guest resolv.conf -> responder -> resolve -> dial)
    And the first operator five-tuple incremented the exact allocation's intercept rule and arrived at leg-F
    And the workload's allocation reaches Running

@walking_skeleton @driving_port @real-io @kvm @property @contract-shape:unbounded-preservation
Scenario: S-GTI-02 — The guest's very first mesh dial is born intercepted (no cleartext escapes)
  Given an mTLS-composed "overdrive serve" is running
    And C3 has provisioned one allocation's netns, tap, and host-veth
    And before real VMM spawn or guest NIC-up an observation-only decorator reports capture-ready on BOTH the tap and host-veth
    And the witness correlates the exact allocation id, slot, netns inode, tap name and ifindex, host-veth name and ifindex, guest MAC, and guest address
  When the operator deploys a "[vm]"+"[job]" whose guest command's FIRST action is a mesh dial by name
  Then from capture-ready through intercept-live the witness observes ZERO guest-originated L2 frames across every EtherType, VLAN shape, protocol, destination, payload size, and unicast/multicast/broadcast class
    And an unexpected source MAC, capture drop or overflow, malformed or truncated record, unknown direction, timestamp or ordering, missing readiness edge, or uncertain interface identity FAILS the test rather than being ignored
    And intercept-live requires BOTH start_alloc success and the exact outbound rule observed on that same host-veth
    And capture continues across the existing asynchronous EXEC release
    And the first post-release TCP SYN is exactly the operator's expected guest-address-to-mesh-VIP-and-port five-tuple
    And that exact original destination increments the correlated host-veth rule and arrives at leg-F
    And the inter-agent path carries TLS 1.3 over kTLS while ZERO cleartext copy reaches the external peer path
    And the complete ordering capture-ready-precedes-VMM-spawn-precedes-network-ready-precedes-READY-precedes-intercept-live-precedes-EXEC-release-precedes-operator-first-connect held

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
    And the capture-ready witness observes no guest-originated frame before failure
    And no cleartext egress ever left the guest

@real-io @kvm @error @restart @contract-shape:bounded-change
Scenario: S-GTI-06 — A restarted microVM workload is re-enrolled in the mesh before it runs again
  Given a Running mesh "[vm]"+"[job]" allocation whose egress intercept is installed
  When a genuine RestartAllocation reuses that allocation id through crash-recovery, restart budget, or "overdrive workload restart" (not stop plus fresh deploy)
  Then the restarted allocation re-installs the egress intercept (the :1880 restart gate fired for VM kind)
    And on successful re-install the first post-restart mesh five-tuple is intercepted and ZERO cleartext reaches the peer
    And when the re-install fails the restarted allocation is driven terminal fail-closed
    And EXEC is not released on that failed re-install
    And a restarted VM alloc never runs cleartext fail-open

@real-io @kvm @contract-shape:bounded-change
Scenario: S-GTI-07 — The operator sees the microVM workload's own mesh address, not its transit hop
  Given an mTLS-composed "overdrive serve"
  When the operator deploys a "[vm]"+"[job]" and runs "overdrive workload describe"
  Then the workload's canonical address is the guest address (the upper-block guest /30 host)
    And it is NOT the transit /30 address (which carries no workload endpoint)

@real-io @kvm @error @contract-shape:bounded-change
Scenario: S-GTI-08 — A microVM that cannot address its network is refused as a boot failure
  Given an mTLS-composed "overdrive serve" with nonzero private and durable restart counts
    And a REAL Cloud-Hypervisor guest will encounter a malformed platform network token before READY through the production deploy path
  When the operator deploys a "[vm]"+"[job]"
  Then the guest powers off before READY and sends no guest EXIT
    And the allocation never reaches Running and the host never sends EXEC
    And the operator command never runs and no guest-originated L2 frame is emitted
    And the typed reason is VmGuestExitUnreported with the exact VMM Option<i32> exit code and signal
    And the primary detail is the bounded final 8 KiB / five line fragments of PID 1 console.log, retaining an unterminated final fragment and rendering invalid UTF-8 lossily
    And bounded hypervisor stderr is used only when console.log is absent, empty, or unreadable, with a stable bounded diagnostic when neither source exists
    And the selected detail names the platform network token parse failure
    And the terminal claim is exactly Failed with the same Option<i32> exit code
    And FinalizeFailed is the only lifecycle action and RestartAllocation is absent
    And the returned private WorkloadLifecycleView equals its input and the durable restart_count is unchanged

@real-io @kvm @teardown @contract-shape:bounded-change
Scenario: S-GTI-12 — A stopped microVM workload's egress mesh guard is torn down, never left behind
  Given a Running mesh "[vm]"+"[job]" allocation deployed via real "overdrive serve" + "overdrive deploy"
    And its egress intercept is installed (the VM alloc's "overdrive-mtls" nft rule is PRESENT)
  When the operator stops it via the real stop driving port ("overdrive workload stop")
  Then the VM alloc's "overdrive-mtls" nft rule is GONE (teardown fired for VM kind at the ungated-by-DriverType stop site :2038)
    And no OTHER alloc's intercept rule is disturbed (observed via a host nft list of the overdrive-mtls table, not by instrumenting the teardown line)
    And adding a DriverType::Exec gate to the teardown sites (:1269 / :2038) would red this AT — the teardown-ungated invariant is locked (mirror of S-GTI-06's :1880 install lock)
```

### S-GTI-08 supporting properties and bounded examples (Rust, source-local)

The real-metal scenario samples `network_token_parse_malformed`; the remaining
finite partitions are source-local table-driven examples so test strength does
not require ten duplicate microVM boots:

| Failure name | Forced condition | Required read-back / diagnostic observable |
|---|---|---|
| `minimal_root_init` | minimal-root bootstrap fails | selected detail names minimal-root initialization; no NIC operation occurs |
| `network_token_missing` | platform network token absent | selected detail names missing token; no suppression/apply operation occurs |
| `network_token_parse_malformed` | address/prefix/gateway/DNS token malformed | selected detail names the malformed field; no static apply occurs |
| `nic_down_prerequisite` | non-loopback NIC flags report UP | selected detail names NIC-down prerequisite and the observed UP state |
| `ipv6_disable_write` | per-interface `disable_ipv6` write fails | selected detail names the interface and IPv6 write stage |
| `ipv6_disable_readback` | `disable_ipv6` read-back is not disabled | selected detail names the expected and observed read-back values |
| `arp_notify_write` | per-interface `arp_notify=0` write fails | selected detail names the interface and `arp_notify` write stage |
| `arp_notify_readback` | `arp_notify` read-back is not `0` | selected detail names the expected and observed read-back values |
| `static_ipv4_apply` | address, netmask, link-up, or default-route apply fails | selected detail names the exact static-apply stage; resolver/READY do not occur |
| `resolver_write` | guest resolver write fails | selected detail names resolver write; READY does not occur |

- The pure Job classifier generates every `Option<i32>` VMM exit code and an
  arbitrary signal for `VmGuestExitUnreported`, then asserts exact preservation
  into `TerminalCondition::Failed { exit_code }`. Its property has the exact
  declaration `/// CONTRACT_SHAPE: pure-function.`.
- A pure guest-network suppression/admission helper generates NIC-up/down and
  arbitrary IPv6/`arp_notify` read-back values. It admits static apply only for
  NIC-down + IPv6-disabled + `arp_notify == 0`; every other combination yields
  its named typed setup failure and schedules no address, route, resolver,
  READY, or EXEC operation. Its property also has the exact declaration
  `/// CONTRACT_SHAPE: pure-function.`.
- Bounded examples cover console tails over 8 KiB, over five line fragments,
  an unterminated final fragment, invalid UTF-8, empty/missing/unreadable
  console, nonempty console precedence over VMM stderr, bounded stderr fallback,
  and the stable neither-source fallback. These are deterministic unit examples,
  while the real-metal S-GTI-08 example proves the production path.

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
| C2a state machine in docstring | PASS | the approved pre-READY state machine is documented above and is required in the metal AT module docstring; it contains no setup `EXIT` state |
| C2b illegal-event-per-state | PASS | S-GTI-08 rejects every setup failure after READY and any READY/Running/EXEC on failure; S-GTI-05 rejects EXEC after install `Err`; post-READY `EXIT` remains operator-only |
| C3 cardinality 0/1/N | PASS | S-GTI-09/10/11 PBT over the whole slot domain (0/1/N slots); S-GTI-04 (non-mesh) vs S-GTI-01 (mesh) dial classes |
| C4a apply-twice/idempotency | PASS | S-GTI-06 restart = re-install (`start_alloc` idempotent — tears prior down) |
| C4b inverse-op-without-prereq | PASS | **S-GTI-12 locks the teardown (the inverse op):** a stopped VM alloc's `overdrive-mtls` rule is GONE and no other alloc's rule is disturbed — a state-mutating teardown, NOT the earlier no-op stop-without-install framing. The ungated-by-`DriverType` invariant is regression-locked: adding a `DriverType::Exec` gate to `:1269`/`:2038` reds S-GTI-12 (feature-delta § [REF] D6 MIRROR hazard — the install-side mirror is S-GTI-06's `:1880` lock) |
| C5a mode-flag combos | PASS | mesh (S-GTI-01) vs non-mesh (S-GTI-04); Exec-vs-Vm driver-type gate exercised by S-GTI-01 (Vm) alongside the shipped exec path |
| C5b flag orthogonality | PASS | D6 gate flips at BOTH install sites (S-GTI-01 fresh `:1584` + S-GTI-06 restart `:1880`); teardown stays ungated-by-`DriverType` and is now concretely locked by **S-GTI-12** (`:1269`/`:2038` — a teardown `DriverType` gate reds S-GTI-12, the teardown twin of the S-GTI-06 install lock) |
| C6a malformed input | PASS | S-GTI-08 explicitly covers missing/malformed platform token and malformed address/prefix/gateway/DNS; the result is pre-READY poweroff, not an infrastructure skip |
| C6b each declared error triggered | PASS | S-GTI-05 (fresh install), S-GTI-06 (restart install), and S-GTI-08's named init/token/NIC/suppression/static-apply/resolver/diagnostic cases cover every declared #222 error class |
| C6c closed error set | PASS | the pre-READY table is closed over each sanctioned setup stage; every arm has the same forbidden READY/Running/EXEC/operator/frame effects plus its named observable |
| C7a degraded resource | PASS | S-GTI-02 treats capture drop/overflow and malformed/truncated records conservatively; S-GTI-08 covers missing/unreadable console and resolver-write failure without losing the typed rejection |
| C7b interruption mid-op | PASS | S-GTI-06 covers genuine restart/re-install; S-GTI-05 covers install interruption before EXEC; S-GTI-08 covers shutdown during guest initialization |
| C7c concurrent actors | PASS | S-GTI-09/10/11 prove per-slot disjointness (tap name, /30, MAC) — the collision-free-by-construction property the net-slot registry relies on for concurrent allocs |

**Passing: 15 / 15 → verdict COMPLETE.** The approved Q7/Q9 amendment closes
the former malformed-token and degraded-observer gaps with explicit named
examples and fail-conservative behavior. No item is waived as an infrastructure
limitation; nested KVM is the specified execution tier, while each RED reason
remains missing feature behavior. **No upstream routing /
CLARIFICATION_NEEDED.**

### Test-budget reasoning

The budget stays at twelve acceptance scenarios because each scenario owns a
distinct operator-visible outcome or safety invariant: one happy journey, the
closed boot/install invariant, wire encryption, non-mesh pass-through, two
install-failure lifecycle modes, address projection, pre-READY boot failure,
three independent slot-derivation properties, and teardown. The S-GTI-08 named
failure rows are one state-machine partition with a common complement and are
cheaper and clearer as table-driven examples, not ten duplicate acceptance
scenarios. The classifier and suppression properties exhaust unbounded input
spaces at the pure source-local layer. Console truncation/precedence cases are
bounded examples because their partitions are finite. Metal remains reserved
for S-GTI-01/-02/-03/-05/-06/-08/-12 behavior that cannot be represented by a
host-netns substitute.

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
| S-GTI-08 classifier property | `crates/overdrive-reconcilers/src/workload_lifecycle.rs` (source-local) | exact `VmGuestExitUnreported` mapping over every `Option<i32>` + arbitrary signal | layer-1 property | `MISSING_FUNCTIONALITY`; exact `/// CONTRACT_SHAPE: pure-function.` required |
| S-GTI-08 setup/suppression properties | `crates/overdrive-init/src/main.rs` (source-local) | total admission/sequencing plus named failure and read-back partitions | layer-1 properties + bounded examples | `MISSING_FUNCTIONALITY`; exact `/// CONTRACT_SHAPE: pure-function.` on every property |
| S-GTI-08 diagnostic-selection examples | `crates/overdrive-worker/src/vm_driver.rs` (source-local) | bounded console selection, precedence, and fallback matrix | layer-1 bounded examples | `MISSING_FUNCTIONALITY`; no public-field addition |
| S-GTI-12 | (same) | `a_stopped_microvm_workloads_egress_mesh_guard_is_torn_down_never_left_behind` | Tier-3 metal (`kvm-tests`) | `#[should_panic(expected = "RED scaffold")]` |
| S-GTI-09 | `crates/overdrive-control-plane/src/veth_provisioner.rs` (`#[cfg(test)] mod guest_tap_plan_distill_scaffold`) | `each_microvm_slot_names_its_own_tap_device_collision_free` | layer-1 default lane | `#[should_panic]` + `VmTapPlan`/`derive_vm_tap_plan` `todo!()` |
| S-GTI-10 | (same) | `each_microvm_slot_owns_a_mesh_address_disjoint_from_its_transit_hop` | layer-1 default lane | `#[should_panic]` |
| S-GTI-11 | (same) | `each_microvm_slot_carries_its_own_locally_administered_nic_identity` | layer-1 default lane | `#[should_panic]` |

Every acceptance-test fn name carries the Outcome Elevator Pitch verbatim
(DISCUSS → DISTILL scenario name → DELIVER test name trace, 2026-05-15
mandate). The three S-GTI-08 supporting rows are source-local properties or
bounded examples, not extra acceptance scenarios; their names describe the
specific invariant they exhaust.
