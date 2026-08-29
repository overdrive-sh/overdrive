# DISTILL test scenarios — guest-stack-transparent-mtls-intercept (GH #222)

**Specification only; not parsed or executed.** This repository bans `.feature`
files. The Gherkin below is the behavioural contract; executable acceptance
tests are Rust. Scope is the egress-first VM Job slice. Inbound guest services
remain #257.

Authoritative upstream inputs are ADR-0088, ADR-0089, the DESIGN section of
`../feature-delta.md`, and `../design/wave-decisions.md`. The rule-hit amendment
is commit `cd12725159a6b2a92619f17aa4dc5f0ff621b842`; DESIGN review iteration 5
is **APPROVED**. The component reuse gate remains exactly **8 REUSE-AS-IS / 10
EXTEND / 1 CREATE-NEW**. DISTILL does not reclassify those nineteen rows.

## Reconciliation record

- DISCUSS `user-stories.md`, `story-map.md`, and `wave-decisions.md` are absent.
  The feature followed SPIKE → DESIGN, so each absence is a warning rather than
  an empty input.
- SPIKE remains WORKS for the routed guest topology. It does not prove the
  later lifecycle, diagnostic, or kernel-witness contracts.
- DESIGN Q7/Q9, including D7's mutation-aware exact-rule-hit oracle, supersedes
  the earlier generic “rule increment” wording.
- DEVOPS `wave-decisions.md` is absent. Native-metal qualification, the
  host-wide lease, and EDD evidence capture are therefore explicit roadmap /
  DEVOPS obligations before DELIVER may claim the metal gate.
- The product journey `run-a-vm-workload.yaml` still says an unreported guest
  death consumes retry/backoff. That conflicts with the later ratified Job
  run-once rule: pre-READY rejection and ordinary Job exit finalize without a
  restart; only platform reclamation while intent still stands re-drives the
  same allocation. The later Job rule is authoritative for this feature.
  Product/Journey ownership must remove the stale retry sentence; this
  DISTILL pass records the conflict instead of claiming zero contradictions.

**Reconciliation result:** one documented stale product-journey conflict,
resolved for implementation by the later Job run-once and platform-reclamation
decisions; no unresolved DESIGN ambiguity.

## Pinned lifecycle and observation model

The one legal success sequence is:

```text
deploy accepted
  -> C3 creates the allocation netns, host-veth and tap
  -> observer learns the real allocation/interface identities
  -> capture is armed on the exact tap and host-veth
  -> real VMM spawn
  -> guest initialization while the NIC is down
  -> READY (all guest network setup complete; guest blocked)
  -> production intercept install succeeds
  -> D7 stable `before` cut proves the exact rule is live
  -> existing asynchronous EXEC release
  -> operator command's first connection
  -> D7 exact packet/byte equality + original destination at leg-F
  -> TLS on the inter-agent wire, with no peer-path cleartext
  -> ordinary operator EXIT
```

The observation harness may be prepared before deploy, but no allocation id,
slot, netns inode, interface name/index, MAC, guest address, capture-ready edge,
or C3 result exists in a Given fixture. The harness learns those facts only
during the real deployment after C3, then arms capture before real VMM spawn.

Before READY, minimal-root initialization, token parsing, NIC-down admission,
IPv6 suppression and read-back, `arp_notify=0` and read-back, static address,
netmask, link-up, route, and resolver configuration must all succeed. Any
failure powers off before READY, emits no guest `EXIT`, reaches neither Running
nor EXEC, and is classified from the real VMM termination. After READY, every
`EXIT`, including status 78, is the operator command's result and never a boot
failure sentinel. Beacon Published Language, public describe fields, and
persistence schemas remain unchanged.

Platform reclamation is the sole same-allocation restart route for a VM Job in
this slice. An unclean control-plane restart with durable intent and a
non-terminal VM row but no fresh supervisor claim drives the existing
boot-epoch reclamation path; the platform-reclaimed row is then re-driven with
the same `AllocationId`. A Job's natural result/crash is run-once and finalizes.
`overdrive workload restart <id>` is generation replacement and mints a fresh
allocation, so it belongs to the fresh-start gate and is not S-GTI-06.

## D7 exact unchanged-rule-hit oracle

S-GTI-01 and S-GTI-02 use the complete ratified D7 contract. “A rule counter
increased,” tag+handle equality, a partial dump, or leg-F arrival alone is not
enough.

1. The production allocation-scoped outbound rule contains one anonymous,
   non-terminal `counter` after the unchanged host-veth and TCP matches and
   before the byte-identical TPROXY/mark/accept tail. Shared, inbound,
   output-divert, and sibling rules remain counter-free unless independently
   selected by their own allocation identity.
2. The selected rule is unique only after a strict complete multipart
   `GETRULE` succeeds. Its full userdata, handle, and normalized ordered
   expression program must equal the production encoder. Normalization retains
   every expression kind, register, operand, address, port, mark, verdict, and
   order; only the live counter values become a typed placeholder. Missing,
   duplicate, extra, reordered, malformed, or unknown expressions fail.
3. `GETRULE` uses a dedicated socket and one absolute deadline. Every message
   must have the kernel sender, request sequence, expected rule reply type and
   family, valid message/attribute/nesting length and alignment, and no trailing
   bytes. Success requires exactly one zero-status `NLMSG_DONE`. Error,
   `NLM_F_DUMP_INTR`, overrun, timeout, EOF, extra/missing DONE, malformed data,
   or a partial dump fails before uniqueness is evaluated.
4. Strict `GETGEN` accepts exactly one complete kernel `NEWGEN` reply with the
   request sequence, expected family, and full nonzero generation id. After
   production install and before the first generation read, a read-only socket
   joins `NFNLGRP_NFTABLES` with loss reporting. Every snapshot is bracketed
   `GETGEN(G) -> complete GETRULE -> GETGEN(G)`. All brackets and the final
   drain must retain the initial nonzero `G`; any notification, generation
   change/decrease/wrap, replacement, delete/reinsert, unrelated transaction,
   `ENOBUFS`, overrun, or notification loss fails.
5. Two equal generation-bracketed snapshots separated by a capture-confirmed
   quiet interval define `before`; two equal guarded snapshots after the
   round-trip define `after`. The observer never installs, replaces, resets, or
   deletes a production rule.
6. The exact host-veth ingress witness is `AF_PACKET/SOCK_DGRAM`, armed before
   VMM spawn, and retains direction, ifindex, and protocol. It uses
   `recvmsg(MSG_TRUNC)` with a 65,535-byte L3 buffer, and closing
   `PACKET_STATISTICS` must report zero drops. A rule-eligible record is exactly
   a kernel-valid unfragmented IPv4 packet on that ifindex whose protocol is
   TCP. Fragmentation, malformed/truncated input, loss, or offload ambiguity
   fails. Each record contributes one packet and validated IPv4 `tot_len`, the
   `skb->len` domain at the priority -150 prerouting counter; L2 length, snap
   length, TCP payload length, or guessed bytes are forbidden substitutes.
7. Let `C` be the complete eligible-packet count and `L` the checked sum of
   those `tot_len` values. Require `C > 0`, `L > 0`,
   `after.packets.checked_sub(before.packets) == Some(C)`,
   `after.bytes.checked_sub(before.bytes) == Some(L)`,
   `before.packets.checked_add(C) == Some(after.packets)`, and
   `before.bytes.checked_add(L) == Some(after.bytes)`. The first eligible
   packet is the expected SYN and every eligible packet has the same
   directional tuple. Regression, reset, wrap, competing traffic, or
   incomplete capture cannot false-pass.
8. The same original destination must arrive at leg-F, the inter-agent path
   must carry TLS, and no cleartext request/response copy may reach the peer
   path. Same-tag adoption keeps accumulated counts but takes a fresh baseline;
   normal stop deletes the exact handle; boot recovery sweeps before reinstall;
   no comparison crosses a restart. Quiescent siblings retain their exact
   tag, handle, normalized program, counter state, and position.

## Native-metal execution and global lease

Runtime metal examples execute only on a **native, non-virtualized x86_64 KVM
host** through `cargo xtask metal run --`. Nested KVM is forbidden. Lima and
other virtualized hosts may compile the gated Rust tests with `--no-run`, but a
compile is not runtime evidence.

The preflight fails closed unless all of these agree: `uname -m` is `x86_64`;
`systemd-detect-virt --vm --quiet` reports no VM; `/proc/cpuinfo` has no
`hypervisor` flag; `/dev/kvm` is an openable character device; KVM API version
is 12; cgroup v2, Cloud Hypervisor, the kernel, and the selected rootfs exist.
Missing tools, contradictory signals, permission errors, or an unknown
virtualization result are a block, never a skip or pass.

All guest-stack metal/EDD commands share a host-wide advisory lease at
`/run/lock/overdrive-guest-stack-transparent-mtls-intercept.lock`. Acquisition
uses `flock` with a finite 120-second timeout. Once acquired, the holder writes
PID, UTC start, scenario/expectation id, workspace, and commit SHA to the lock
file; a timeout reports the current owner metadata. The file descriptor stays
held across preflight, command execution, evidence collection, bounded cleanup,
and final residue probes, and is released only when the remote command has
completed. This serializes independent worktrees as well as one test process.
The next roadmap and DEVOPS handoff must assign the harness owner and commit
the lease/preflight implementation before any runtime metal acceptance step.

## Acceptance examples and budget

There are **15** acceptance examples. Splitting restart success/failure,
pre-/post-READY exit semantics, and teardown with/without a rule is intentional;
they are mutually exclusive outcomes, not one scenario with contradictory
Then clauses. Exactly one scenario carries the walking-skeleton marker:
S-GTI-01.

| ID | Tags | Contract shape | Outcome |
|---|---|---|---|
| S-GTI-01 | `walking-skeleton @driving_port @real-io @kvm` | bounded-change | a VM Job dials a mesh peer by name and receives its reply |
| S-GTI-02 | `@driving_port @real-io @kvm @property` | unbounded-preservation | the first guest mesh dial is born captured and exactly accounted |
| S-GTI-03 | `@real-io @kvm @wire-assertion` | unbounded-preservation | inter-agent traffic is TLS and peer-path cleartext is absent |
| S-GTI-04 | `@real-io @kvm` | bounded-change | non-mesh egress remains plaintext and functional |
| S-GTI-05 | `@real-io @kvm @error` | bounded-change | fresh intercept-install failure refuses execution |
| S-GTI-06a | `@real-io @kvm @restart` | bounded-change | platform reclamation re-enrols the same allocation before EXEC |
| S-GTI-06b | `@real-io @kvm @restart @error` | bounded-change | failed same-allocation reinstall remains terminal and closed |
| S-GTI-07 | `@real-io @kvm` | bounded-change | describe shows the guest address, not the transit hop |
| S-GTI-08a | `@real-io @kvm @error @cleanup` | bounded-change | a real resolver-apply failure is terminal, truthful, and residue-free |
| S-GTI-08b | `@real-io @kvm` | bounded-change | post-READY operator exit 78 is an ordinary command result |
| S-GTI-09 | `@property @in-memory` | pure-function | slots derive collision-free tap names |
| S-GTI-10 | `@property @in-memory` | pure-function | slots derive bounded, disjoint guest networks |
| S-GTI-11 | `@property @in-memory` | pure-function | slots derive unique locally-administered unicast MACs |
| S-GTI-12a | `@real-io @kvm @teardown` | bounded-change | Job stop removes exactly the installed allocation rule |
| S-GTI-12b | `@real-io @kvm @teardown` | unbounded-preservation | Job stop without an installed rule is idempotent and preserves siblings |

### Stakeholder scenarios

```gherkin
@walking_skeleton @driving_port @real-io @kvm @contract-shape:bounded-change
Scenario: S-GTI-01 — A microVM workload dials a mesh peer by name and receives the reply
  Given an mTLS-composed "overdrive serve" and a named mesh peer are available
    And the observation harness is prepared but has no allocation identity
  When the operator deploys a "[vm]"+"[job]" whose command dials the peer by name
  Then the real deployment learns and captures its interfaces before VMM spawn
    And the guest receives the peer's byte-distinct reply
    And the ratified D7 exact unchanged-rule-hit oracle passes
    And the allocation reaches Running

@driving_port @real-io @kvm @property @contract-shape:unbounded-preservation
Scenario: S-GTI-02 — The guest's first mesh dial is born captured and exactly accounted
  Given an mTLS-composed "overdrive serve" is available
    And the observation harness is prepared but has no allocation identity
  When the operator deploys a "[vm]"+"[job]" whose first action is a mesh dial
  Then after real C3 provisioning the harness binds the exact allocation, netns, tap, host-veth, MAC, and guest address
    And capture is armed on both exact interfaces before real VMM spawn
    And every guest-originated L2 frame before the D7 before-cut fails the example
    And capture loss, ambiguity, malformed data, or uncertain ordering fails the example
    And EXEC is released only after the generation-stable exact rule baseline exists
    And the first eligible SYN has the expected guest-address-to-mesh-VIP tuple
    And checked packet and IPv4-total-length deltas equal the complete eligible capture
    And the original destination reaches leg-F over a TLS-protected path with no peer-path cleartext

@real-io @kvm @wire-assertion @contract-shape:unbounded-preservation
Scenario: S-GTI-03 — The guest's mesh traffic travels the peer wire as mTLS, never in the clear
  Given a deployed microVM is dialing a mesh peer through its live egress guard
  When byte-distinct request and response payloads cross the connection
  Then the inter-agent wire carries TLS application-data records in both directions
    And neither plaintext marker appears on the external peer path
    And the kernel reports TLS installed on the protected legs

@real-io @kvm @contract-shape:bounded-change
Scenario: S-GTI-04 — The same guest reaches a non-mesh destination in the clear
  Given a deployed microVM whose mesh egress is guarded
  When its command dials an address outside mesh membership
  Then the non-mesh peer receives the plaintext request
    And the guest receives the unchanged reply

@real-io @kvm @error @contract-shape:bounded-change
Scenario: S-GTI-05 — A fresh mesh-guard installation failure refuses the workload
  Given the native host is arranged so the real production nft install returns a kernel error
  When the operator deploys a "[vm]"+"[job]"
  Then describe reaches terminal Failed with an actionable guard-install detail
    And the operator command never runs
    And no guest-originated frame or cleartext egress escapes
    And bounded cleanup leaves no allocation-scoped residue

@real-io @kvm @restart @contract-shape:bounded-change
Scenario: S-GTI-06a — Platform reclamation re-enrols the same allocation before it runs again
  Given a VM Job is Running and its durable intent and allocation row exist
  When the operator ends "overdrive serve" uncleanly and restarts it on the same data directory
  Then "overdrive workload describe <id>" reports the platform-reclamation ending for the unsupervised VM
    And its next attempt reuses the same allocation identity
    And the replacement guest remains blocked until its new guard satisfies D7
    And its first post-reclamation mesh flow is protected with no peer-path cleartext

@real-io @kvm @restart @error @contract-shape:bounded-change
Scenario: S-GTI-06b — Failed re-enrolment after platform reclamation stays closed
  Given a VM Job is Running and its durable intent and allocation row exist
    And the native kernel environment will reject the guard reinstall after reclamation
  When the operator ends "overdrive serve" uncleanly and restarts it on the same data directory
  Then "overdrive workload describe <id>" shows the next attempt reused the same allocation identity
    And describe reaches terminal Failed with the reinstall detail
    And EXEC is never released for the replacement guest
    And no cleartext egress escapes

@real-io @kvm @contract-shape:bounded-change
Scenario: S-GTI-07 — The operator sees the guest address, not its transit hop
  Given an mTLS-composed "overdrive serve" is available
  When the operator deploys a "[vm]"+"[job]" and runs "overdrive workload describe <id>"
  Then the canonical workload address is the guest /30 host address
    And it is not the transit /30 address

@real-io @kvm @error @cleanup @contract-shape:bounded-change
Scenario: S-GTI-08a — A real resolver-apply failure is terminal, truthful, and residue-free
  Given a second independent allocation is Running with a quiescent egress rule
    And the VM spec selects a custom rootfs whose resolver target makes the production resolver write fail
  When the operator deploys the failing "[vm]"+"[job]" and runs "overdrive workload describe <id>"
  Then describe shows one terminal Failed attempt with the resolver-stage detail
    And the exact observed VMM exit code is preserved when present
    And the durable restart count and budget are unchanged
    And the allocation never reports READY or Running and the operator command never runs
    And no operator marker, guest EXIT frame, or guest-originated network frame is observed
    And within the cleanup deadline no VMM, cgroup, rootfs clone, clone index, run directory, netns, tap, veth, route, nft rule, capture process, socket, or file descriptor remains for the failed allocation
    And the independent allocation and its exact rule identity and counter state are unchanged

@real-io @kvm @contract-shape:bounded-change
Scenario: S-GTI-08b — Operator exit 78 after READY is an ordinary result
  Given a VM Job has completed network setup and reported READY
  When EXEC runs an operator command that exits with status 78
  Then "overdrive workload describe <id>" reports the ordinary Job result with exit code 78
    And it does not report a setup rejection or an unreported guest exit
    And the result consumes no restart attempt

@property @in-memory @contract-shape:pure-function
Scenario: S-GTI-09 — Each microVM slot names its own tap device collision-free
  Given any valid slot, including zero, one, maximum minus one, and maximum
  When its guest tap plan is derived
  Then the tap name is "ovd-tp-" plus the four-hex slot and fits IFNAMSIZ
    And a distinct slot has a distinct name
    And maximum plus one is rejected

@property @in-memory @contract-shape:pure-function
Scenario: S-GTI-10 — Each microVM slot owns a bounded guest network disjoint from transit
  Given any valid slot, including zero, one, maximum minus one, and maximum
  When guest and transit plans are derived
  Then the guest /30 begins at the mesh base plus 0x8000 plus slot times four
    And gateway and guest are its first and second usable addresses
    And guest and transit /30s are disjoint and remain within the mesh block
    And maximum plus one is rejected

@property @in-memory @contract-shape:pure-function
Scenario: S-GTI-11 — Each microVM slot carries a unique local unicast NIC identity
  Given any valid slot, including zero, one, maximum minus one, and maximum
  When its guest MAC is derived
  Then the multicast bit is clear and the locally-administered bit is set
    And a distinct slot has a distinct MAC
    And maximum plus one is rejected

@real-io @kvm @teardown @contract-shape:bounded-change
Scenario: S-GTI-12a — Job stop removes exactly the installed guest egress guard
  Given two independent VM Jobs are Running with generation-stable exact rule snapshots
  When the operator runs "overdrive job stop <target-id>"
  Then the target allocation's exact egress rule is absent after a complete guarded dump
    And every sibling rule retains its exact tag, handle, normalized program, counter, and order
    And bounded cleanup leaves no target allocation residue

@real-io @kvm @teardown @contract-shape:unbounded-preservation
Scenario: S-GTI-12b — Job stop without a guest egress guard is idempotent
  Given a VM Job has a terminal pre-READY attempt with no installed egress rule
    And an independent VM Job remains Running with a generation-stable rule snapshot
  When the operator runs "overdrive job stop <target-id>" twice
  Then the results are Stopped followed by AlreadyStopped
    And no target rule appears
    And every sibling rule retains its exact tag, handle, normalized program, counter, and order
```

## Supporting source-local and component examples

These examples carry implementation-sensitive assertions that do not belong in
metal Gherkin. Pure properties use the exact rustdoc declaration
`/// CONTRACT_SHAPE: pure-function.` on every live property.

### Guest-init closed failure partitions

Malformed token parsing and static application are distinct families. Each row
forces only the named stage, returns its typed stage-specific diagnostic, and
forbids all later address/route/resolver/READY/EXEC operations.

| Family | Named case |
|---|---|
| token | missing token |
| token | malformed guest address |
| token | non-integer or out-of-range prefix |
| token | malformed gateway |
| token | malformed DNS address |
| suppression | NIC unexpectedly up |
| suppression | IPv6-disable write failure |
| suppression | IPv6 read-back not disabled |
| suppression | `arp_notify=0` write failure |
| suppression | `arp_notify` read-back not zero |
| static apply | address application failure |
| static apply | netmask application failure |
| static apply | link-up failure |
| static apply | default-route failure |
| resolver | resolver write failure |

The token parser property ranges over arbitrary malformed bytes and field
boundaries. The suppression property ranges over NIC flags and read-back values.
The static-apply examples are separate because one collapsed “static apply
failed” case cannot prove later operations are suppressed at every stage.

### Diagnostic and cleanup totality

Bounded examples cover a console tail over 8 KiB, over five fragments, an
unterminated final fragment, invalid UTF-8, and nonempty-console precedence over
VMM stderr. Absence, empty content, unreadable metadata, open/read failure, and
mid-read error are separate cases. Every case must still return the original
start rejection, select bounded stderr or the stable neither-source detail, and
run the same cleanup; diagnostic collection may never replace or mask the
rejection or cleanup error. Cleanup examples poll to a finite deadline for the
same complete residue set asserted by S-GTI-08a and preserve an independent
allocation and nft rule.

### Reconciler/action-shim component example

`C-GTI-08-RECONCILE` maps to the existing workload-lifecycle reconciler and
action shim. Given a Job allocation row carrying an unreported pre-READY VMM
exit, a seeded private View, and a nonzero durable restart count, reconcile must
emit exactly one finalization action with the exact Failed exit code, emit no
restart action, and return a View identical to its input. Dispatch must persist
the same terminal claim while retaining the durable count. This component
example owns the private action vector, View equality, and no-restart checks;
S-GTI-08a owns only port-visible state and cleanup.

`C-GTI-08-EXIT78` proves the complement: a READY-observed operator `EXIT 78`
maps to the ordinary Job result and never to the pre-READY rejection reason.

### Illegal-event state matrix

| State | One forbidden event and expected disposition |
|---|---|
| harness prepared, no C3 identity | claiming capture-ready or spawning the VMM fails the witness |
| C3 complete / capture armed | any guest-originated L2 frame before the D7 before-cut fails |
| guest initializing | READY before every required setup/read-back succeeds is rejected |
| READY, blocked | EXEC before a stable exact rule baseline is forbidden |
| intercept live | any nft notification, generation change, reset ambiguity, loss, or program mismatch fails |
| operator command running | a setup-failure classification is forbidden; status 78 remains an operator result |
| terminal | later READY, EXEC, or a second finalization cannot reopen or duplicate the attempt |

### Netlink/capture error closure

Source-local parser/property cases cover zero target, one exact target, and
duplicate targets; counter absent/partial/duplicate/wrong-width; expression
missing/extra/reordered/unknown; wrong sender/sequence/type/family; malformed or
misaligned messages/attributes; missing/extra/error DONE; `NLM_F_DUMP_INTR`;
timeout/EOF/overrun/trailing/partial dump; zero/extra/malformed `GETGEN` reply;
generation change/decrease/wrap; notification and notification loss; counter
reset/regression/wrap/overflow; capture truncation/drop/fragment/offload
ambiguity; and packet/byte equality mismatch. Counter-free siblings remain
valid. These are decoder/oracle properties, not duplicated metal boots.

### Interruption and concurrent-actor supporting examples

`M-GTI-INTERRUPT-BOOT` is a focused native-metal example. After real C3 has
produced the allocation identity and capture is armed, it terminates the real
VMM before READY. The production start path must report the same pre-READY
rejection class, never release EXEC, and complete the full bounded residue
cleanup while an independent allocation remains unchanged. This is an actual
external interruption during the operation, distinct from a stage returning an
expected error.

`M-GTI-CONCURRENT-DEPLOY` starts two built-binary VM Job deploy commands in
parallel inside one host-lease holder. Both real C3 paths must derive distinct
allocation ids, netns/tap/veth identities, guest networks, MACs, and exact nft
rules; each capture may accept only its own tuple, and both Jobs must converge
without cross-observation. Cleanup is delta-scoped per allocation. This is the
parallel multi-actor case; the host-wide lease serializes competing test runs,
not the two allocations inside this example.

## Canonical AT-completeness audit

This is **specification coverage**, not execution status. The immutable-baseline
execution classification is in `red-classification.md` and currently records
`NOT_EXECUTED` for this remediation.

| Item | Verdict | Concrete evidence |
|---|---|---|
| C1a zero/min | PASS | S-GTI-09/10/11 explicitly include slot 0 and 1 |
| C1b partition boundary | PASS | S-GTI-09/10/11 include max-1, max, and rejected max+1 |
| C2a state machine documented | PASS | legal lifecycle sequence is explicit above |
| C2b illegal event per state | PASS | seven-state illegal-event matrix names one forbidden event and result per state |
| C3 0/1/N cardinality | PASS | netlink properties cover 0/1/duplicate targets; slot properties cover one and many allocations |
| C4a apply twice/idempotency | PASS | same-tag adoption takes a fresh baseline; S-GTI-12b repeats Job stop |
| C4b inverse without prerequisite | PASS | S-GTI-12b stops an allocation for which no guard was installed |
| C5a mode combinations | PASS | mesh/non-mesh, fresh/reclamation, success/failure are separate examples |
| C5b flag orthogonality | PASS | fresh and reclamation gates are independent; teardown is covered with and without a guard |
| C6a malformed input | PASS | address, prefix, gateway, and DNS token cases are distinct |
| C6b each declared error | PASS | suppression, address, netmask, link, route, resolver, install, diagnostics, dump, generation, and capture failures are individually forced |
| C6c closed error set | PASS | guest-init and D7 error tables enumerate every sanctioned stage and forbid later effects |
| C7a degraded resource | PASS | console read variants, resolver failure, capture loss, netlink loss, and strict timeout paths fail conservatively |
| C7b interruption mid-operation | PASS | M-GTI-INTERRUPT-BOOT terminates the real VMM after capture-ready and before READY, then proves rejection and total cleanup |
| C7c concurrent actors | PASS | M-GTI-CONCURRENT-DEPLOY runs two deploys in parallel and proves distinct identities/captures/rules; S-GTI-12 additionally preserves a live sibling during stop |

**Specified: 15/15 → COMPLETE.** No infrastructure waiver is counted as
coverage. This score does not assert that all examples are implemented or have
run.

## Adapter, test, and EDD map

| Contract | Production seam / driving port | Executable location or obligation |
|---|---|---|
| real deploy + describe | built `overdrive deploy`; `overdrive workload describe` | `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs`; E07/E08 |
| exact Job stop | built `overdrive job stop <id>` → `commands::deploy::stop` | S-GTI-12a/b; E09 |
| same-allocation re-drive | unclean serve restart → boot VM reclamation → standing-intent lifecycle re-drive | S-GTI-06a/b; E09 |
| fresh and restarted install | existing allocation start/restart dispatch into the production mTLS worker | S-GTI-01/05/06a/06b |
| guest token and static apply | production kernel cmdline → `overdrive-init` | source-local parser/stage properties; S-GTI-08a |
| terminal/no-restart classification | workload-lifecycle reconciler + action shim | `C-GTI-08-RECONCILE` source/component example |
| post-READY status 78 | real READY/EXEC/EXIT path | S-GTI-08b + `C-GTI-08-EXIT78`; E08 |
| exact rule-hit witness | production rule encoder/installer + read-only strict netlink/capture observer | D7 parser/oracle properties; S-GTI-01/02; E07 |
| failed-start cleanup | production VMM/worker guard drops and host observation surfaces | cleanup totality examples; S-GTI-05/08a; E08 |
| interruption and concurrent allocations | real VMM termination; two parallel built-binary deploys | M-GTI-INTERRUPT-BOOT / M-GTI-CONCURRENT-DEPLOY supporting metal examples |
| tap/network/MAC derivation | existing veth provisioner and the one CREATE-NEW `VmTapPlan` | S-GTI-09/10/11 source-local properties |
| native host qualification | `cargo xtask metal run --` with non-virtualized preflight and global lease | roadmap/DEVOPS implementation obligation; E07/E08/E09 stubs |

The EDD stubs are `E07-guest-first-mesh-dial-born-captured`,
`E08-vm-guest-boot-failure-truthful-and-clean`, and
`E09-vm-guest-reclamation-and-stop-preserve-rules`. They remain pending until
their built-binary commands, state/wire/kernel evidence, bounded cleanup, and
native-host preflight are executed and independently reviewed.
