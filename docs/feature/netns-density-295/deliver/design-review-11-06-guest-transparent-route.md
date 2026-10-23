# GUEST-TRANSPARENT-ROUTE / STEP 11-06 independent DESIGN review

## Review metadata

| Field | Value |
|---|---|
| Review ID | `GUEST-TRANSPARENT-ROUTE-STEP11-06-ARCH-REVIEW-01` |
| Date | 2026-10-24 |
| Reviewer | `nw-solution-architect-reviewer` |
| Iteration | 1 |
| Scope | Only the native-falsified guest transparent-route premise, its exact bounded replacement in the #295 feature delta and ADRs, and its STEP 11-06 roadmap impact |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 0 critical, 1 high |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** |

The reviewer definition and the `nw-sar-critique-dimensions` and
`nw-roadmap-review-checks` skills were loaded. Repository rules requiring a
native Markdown artifact, production-path reachability, native falsification,
exact error mapping, no speculative scope, and no iteration cap govern this
review.

This review changes no production source, test body, DES event, execution log,
reviewed design artifact, model, lane, kernel state, or roadmap validation
status. It performs no new physical-host run, mutation run, commit, or
implementation review. The separate fresh DELIVER reviewer required after the
actual STEP 11-06 commit remains mandatory; this DESIGN review does not replace
it.

## Artifacts and evidence reviewed

- `docs/feature/netns-density-295/deliver/native-11-06-guest-transparent-route-falsification.md`.
- `.context/netns-density-295/11-06/design-route-current-plus-tcp-fwmark-accept-native.log`.
- `.context/netns-density-295/11-06/design-route-exact-spike-config-native.log`.
- `docs/feature/netns-density-295/feature-delta.md`, including the guest
  adaptation contract, DG-5 setup interface, typed-error table, D15 READY
  re-derivation, lifecycle matrix, Gate G-V7, and P4 test placement.
- `docs/product/architecture/adr-0150-guest-kernel-aya-adaptation-controlled-by-overdrive-init.md`.
- `docs/product/architecture/adr-0174-host-opened-guest-connections-appear-from-the-guest-gateway.md`.
- `docs/product/architecture/adr-0182-a-guest-reset-ends-the-vmm.md`.
- `docs/feature/netns-density-295/deliver/roadmap.json`, especially STEP
  11-06, its design references and validation state.
- The current production path in
  `crates/overdrive-init/src/transport/{mod.rs,kernel.rs}` and
  `crates/overdrive-init/src/guest_flow/{mod.rs,owner.rs,kernel_io.rs,real_kernel.rs}`,
  plus the existing in-guest wrapper in
  `crates/overdrive-init/tests/integration/in_guest_lane.rs`.
- The successful guest-capture lineage in
  `spike-scratch/netns-density-295-guest-vsock-capture/guest/init.sh` and
  `spike-scratch/netns-density-295-v5c/guest/init.sh`.
- Linux 7.0 source under `vendors/linux`: `include/net/route.h`,
  `net/core/fib_rules.c`, `net/ipv4/route.c`,
  `net/ipv4/fib_semantics.c`, `include/net/inet_sock.h`,
  `net/ipv4/tcp_input.c`, `net/ipv4/inet_connection_sock.c`,
  `include/net/ip.h`, `kernel/exit.c`, and `kernel/panic.c`.
- `AGENTS.md`, `.claude/rules/design.md`,
  `.claude/rules/development.md`, `.claude/rules/testing.md`, and
  `.claude/rules/verification.md`.

## Reproduced premise and necessity

The falsified state is reachable through the real STEP 11-06 production owner
path, rather than through a test-only override:

1. The existing Tier-3 cell starts a disposable guest through `GuestKit`, runs
   production `configure_guest_transport`, constructs `GuestFlowOwner` over
   `KernelGuestOwnerIo`, then drives a positive host-opened control flow after
   its aborted-flow branch
   (`guest_flow/real_kernel.rs:1083-1107,1174-1203,1315-1320`;
   `in_guest_lane.rs:290-312`).
2. `GuestFlowOwner` decodes the real `TcpAccept`, verifies the launch token and
   live V_g, then requests `gateway:0 -> workload_addr:listen_port`
   (`guest_flow/owner.rs:479-538`).
3. `KernelGuestOwnerIo::new_app_connect` creates the actual non-blocking IPv4
   socket, sets linger, `IP_TRANSPARENT`, `SO_REUSEADDR`, and `SO_MARK`, binds
   the gateway address and calls `connect`
   (`guest_flow/kernel_io.rs:616-647`).

The isolated contrast supports both the failure and the replacement:

| Configuration | Direct evidence | Assessment |
|---|---|---|
| Current `SO_MARK=0x1`, priority-32765 lookup of reserved table 255/local, with `tcp_fwmark_accept=1` added while retaining `fwmark_reflect=1` | The receipt prints the synchronized source configuration at lines 19-80. The owner then decodes flow `2147483650`, observes live V_g, issues `100.95.0.1:0 -> 100.95.32.54:8081`, and receives no connect-completion event at lines 543-556. The cell fails after 60.248 seconds at lines 102-121 and 570-572. | The rule-only premise remains false even after accepted-child mark inheritance is enabled. |
| Dedicated `SO_MARK=0x295`, priority-32765 table-295 rule, table-295 host-scope local-default route, `tcp_fwmark_accept=1`, and `fwmark_reflect=1` | The receipt prints the exact source constants, rule, route, both sysctls and socket mark at lines 19-92, rebuilds and stages the same production guest at lines 94-112, and the same cell passes in 30.280 seconds at lines 114-125. | The bounded replacement restores the existing host-opened flow outcome through the production owner. |

The successful contrast changes no test body or fixture. Targeted inspection of
the present source confirms the temporary probe constants, dedicated route and
combined sysctl edits are absent; the working tree is back on the current
falsified production configuration. Existing unrelated dirty work remains
untouched.

No simpler retained alternative is correct. Adding only
`tcp_fwmark_accept=1` failed. Adding a catch-all route to reserved table 255 or
`main` would broaden local delivery beyond marked owner traffic. Reusing the
host `0x1`/table-100 pair couples a guest-private route to host leg-C/leg-F
configuration, while the dedicated `0x295`/295 pair has the exact production-
owner success and the prior guest-capture lineage.

## Linux 7.0 source audit

The kernel claims used by the replacement match the vendored Linux 7.0 source:

- IPv4 socket route lookup copies `sk_mark` into the flow key
  (`include/net/route.h:151-156`), and FIB rules compare that mark under the
  rule mask (`net/core/fib_rules.c:278-310`). A nonzero mark with no explicit
  mask defaults to `0xffffffff` (`net/core/fib_rules.c:621-631`), matching the
  full-mask contract.
- An `RTN_LOCAL` result is sent through loopback
  (`net/ipv4/route.c:2814-2867`), and `RTN_LOCAL` has host scope
  (`net/ipv4/fib_semantics.c:88-100`). The dedicated table therefore provides
  the transparent local return path without changing unmarked lookup.
- With `tcp_fwmark_accept=1`, an unmarked listener obtains the incoming skb
  mark for the request (`include/net/inet_sock.h:105-112` and
  `net/ipv4/tcp_input.c:7508-7536`), and the accepted application child inherits
  it (`net/ipv4/inet_connection_sock.c:1204-1233`).
- `fwmark_reflect=1` preserves the triggering packet's mark for the guest
  kernel's own IPv4 reply (`include/net/ip.h:403-404`).
- Global PID 1 exit panics before `exit_files`
  (`kernel/exit.c:923-931,964-975`); with the accepted `panic=1` and RESET-EXIT
  path, the guest/VMM ends. The VM network namespace therefore owns the rule,
  route and sysctl lifetime; no live delete or cross-boot cleanup mechanism is
  needed.

These facts explain the native contrast. The contrast itself is the evidence
that the dedicated route is necessary.

## Interface, error, lifecycle and scope assessment

### Exact configuration and contract shape

The detailed guest adaptation contract pins one interpretation:

- owner socket `SO_MARK=0x295`;
- `ip rule priority 32765 fwmark 0x295/0xffffffff lookup 295`;
- `ip route add local 0.0.0.0/0 dev lo table 295 proto boot scope host`;
- `net.ipv4.tcp_fwmark_accept=1`; and
- `net.ipv4.fwmark_reflect=1`.

Its Contract Shape is correctly classified as bounded-change. The mutation
universe is the one full-mask rule, one local-default route, and the two named
IPv4 sysctls inside one disposable VM network namespace. The existing P4
transport real-kernel cell is the read-back boundary, and the existing owner
positive control observes successful `Paired`. Aligning those existing cells
adds no scenario, lane, test-support API, or model.

The replacement changes no public or cross-crate Rust API, wire or persisted
format, product configuration, CLI, owner, state, timeout, retry, repair,
adoption, runtime reconciliation, or payload path. The two added
`GuestTransportOps` methods are crate-private setup stages. Their exact names
and order are pinned without prescribing private helper structure.

### Typed error mapping

The semantic setup failures and their interface representations are complete:

| Designed failure | Exact representation | Projection |
|---|---|---|
| Installing the priority-32765 full-mask rule to table 295 | `GuestTransportSetupError::FwmarkRule { source: std::io::Error }` | One cause-specific console line; guest exits before READY |
| Installing the table-295 host-scope local-default route on `lo` | `GuestTransportSetupError::FwmarkRoute { source: std::io::Error }` | One cause-specific console line; guest exits before READY |
| Writing `net.ipv4.tcp_fwmark_accept=1` | `GuestTransportSetupError::TcpFwmarkAccept { source: std::io::Error }` | One cause-specific console line; guest exits before READY |
| Writing `net.ipv4.fwmark_reflect=1` | `GuestTransportSetupError::FwmarkReflect { source: std::io::Error }` | One cause-specific console line; guest exits before READY |

The interface and display table distinguish all four semantic stages. Neither
new failure is hidden in `FwmarkRule`, `FwmarkReflect`, a generic diagnostic,
or a string-only subtype. The per-flow `SO_MARK` sets the same already-owned
application-connect socket option; failure continues through the existing
`start_connect_app` `io::Error` to the accepted per-flow `Refused { errno }`
projection and adds no setup error class.

### Ownership, lifecycle and cleanup

`overdrive-init` remains the sole setup owner. It installs the rule, route and
sysctls before intake binding, capture attachment, application-side transparent
connect and READY. A setup failure remains the existing pre-READY guest exit.
Later PID-1 loss ends the VM under RESET-EXIT; network-namespace destruction
removes every partial or complete route/rule/sysctl effect. No new lifecycle
gate, timeout, cancellation, reconnect, replay, or recovery protocol exists,
so no new Quint or seeded-control-plane invariant is warranted. The effect is
a real-kernel adapter result, for which the existing native P4 lane is the
correct evidence boundary.

The accepted `TcpAccept` outcomes, gateway peer identity, no-payload boundary,
non-blocking connect, `GUEST_INBOUND_CONNECT_TIMEOUT`, READY meaning, control
session ownership, allocation Running/Active, and Service Stable semantics are
unchanged. The full-mask dedicated table is narrower than using table 255 or
`main`; unmarked guest traffic is unaffected. No accepted `zero`, `never`,
`must`, `exactly`, fail-closed, forbidden-state, cleanup, or security invariant
is relaxed.

### Authority classification

This correction does not introduce a user-visible outcome or a new security,
ownership, lifecycle, compatibility, persistence, wire, lane, or runtime
mechanism decision. Native evidence disproved the internal configuration that
was supposed to implement already-approved D15, INB-1 and D8-PEER. The exact
replacement is necessary to make those outcomes true and is confined to the
existing private G-V7 setup boundary. Under the governing direction to stop
only for a gap that changes an outcome and otherwise carry evident design
intent, no additional user approval is needed for the `0x295`/table-295
correction, the two crate-private setup stages, or alignment of existing P4
tests. Independent DESIGN approval and canonical reconciliation remain
required before DELIVER resumes.

## Finding and disposition

### F-01 — The canonical READY/G-V7 contract still names the falsified rule-only setup

- **Severity:** High; blocks DESIGN approval.
- **Claim:** The detailed guest adaptation contract, typed-error table, DG-5
  interface, ADR-0150 consequences, and lifecycle matrix require the new rule,
  route and both sysctls before READY. Two canonical lifecycle sections still
  define READY's exact input as only “the fwmark rule and
  `fwmark_reflect`.” That omission leaves two contradictory versions of the
  G-V7 promise.
- **Evidence:** The exact replacement is at `feature-delta.md:7121-7154`, the
  setup interface/order at `:11230-11265`, and the lifecycle matrix at
  `:11899-11903`. In contrast, the D15 READY re-derivation says “Guest network
  configuration is: ... the fwmark rule and
  `net.ipv4.fwmark_reflect=1`” and then “READY waits for exactly that” at
  `:1274-1285`. Gate G-V7 likewise says its promise consists of the guest
  address, “the fwmark rule and `fwmark_reflect`” at `:12270-12288`. Neither
  section names the table-295 local-default route or
  `tcp_fwmark_accept=1`.
- **Reachability:** This is not a hypothetical runtime failure or an expansion
  request. The production-owner failure is reproduced above, and both omitted
  effects are the ones that distinguish the failing and passing native runs.
  A crafter following either exact lifecycle section could reproduce the
  falsified configuration while believing G-V7 and READY were complete.
- **Required revision:** Reconcile the D15 READY re-derivation and Gate G-V7
  promise to the same exact guest-private mark, priority/full-mask rule,
  table-295 host-scope local-default route, `tcp_fwmark_accept=1`, and
  `fwmark_reflect=1` already pinned in the guest adaptation contract. Preserve
  the existing owner, ordering, failure projection, unaffected states and
  evidence lane. This is documentation reconciliation only; it authorizes no
  new gate, state, scenario, lane, model, API, retry or cleanup mechanism.
- **Disposition:** Open.

No other critical, high, or medium issue is proven. In particular, adding a
runtime route auditor, retry, repair loop, persisted ownership record, explicit
live delete, new test cell, new lane, new model state, or host-side route is
unsupported by a reachable failure and would expand the accepted scope.

## Architecture review dimensions

| Dimension | Assessment | Evidence |
|---|---|---|
| Architectural bias | Pass | The replacement restores the previously proven guest-capture route shape and adds no subsystem, userspace dataplane, runtime manager or fashionable technology. |
| ADR quality | Pass | ADR-0150 records the falsified premise, exact replacement, rejected alternatives, consequences and Linux/native evidence; ADR-0174 preserves peer identity. |
| Completeness | **Fail pending F-01** | Exact configuration, errors, owner and cleanup are present, but the two canonical READY/G-V7 summaries still omit the route and `tcp_fwmark_accept`. |
| Feasibility and testability | Pass | The same production-owner native cell fails under the retained rule-only shape and passes under the exact replacement; existing P4 cells and lane are sufficient after bounded assertion alignment. |
| Priority/data justification | Pass | The reproduced inbound `TcpAccept` failure is the actual STEP 11-06 blocker, and the replacement changes only the kernel configuration shown necessary by the isolated contrast. |

## Roadmap review

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | Pass for the complete roadmap | STEP 11-06 builds the additive guest setup/owner half; the existing attachment-cut step invokes it before READY. The native cell already drives the production setup and owner path. |
| Acceptance-criterion coupling | Pass for this correction | Exact route values and typed setup stages are design-owned configuration/error contracts. Private netlink helper structure remains with the crafter. |
| Step decomposition | Pass | The correction belongs inside existing STEP 11-06. It adds no step, component or subsystem. |
| Implementation code | Pass | The feature delta pins observable kernel objects, order and error semantics; it does not prescribe private helper bodies. |
| Concision and precision | Pass for the amendment | One roadmap note identifies the exact replacement and its bounded DISTILL alignment. F-01 concerns canonical lifecycle consistency, not roadmap growth. |
| Test boundary | Pass | Existing source-local transport readback plus the existing production-owner native positive control cover the adapter effect. No unit test substitutes for the real-kernel route result, and no new lane or cell is needed. |
| Mechanical validity | Pass | `roadmap.json` parses; the reviewed design/roadmap diff passes `git diff --check`; validation remains `pending`. |

The roadmap must remain pending while F-01 is open. After the canonical
lifecycle text is reconciled and this reviewer approves the same iteration,
the design owner may fold the correction from proposed to accepted, remove the
now-obsolete explicit-user-approval gate, record independent DESIGN approval,
and revalidate the roadmap. DISTILL still owns bounded alignment of the
existing P4 transport/owner cells before DELIVER resumes.

## Iteration 1 verdict

**CHANGES_REQUESTED.** The native evidence proves the accepted rule-only
premise false and proves the dedicated `0x295`/table-295 local-default route
with both mark sysctls sufficient through the real production owner. The
replacement is necessary, bounded, fail-before-READY, guest-namespace-local,
fully typed, and preserves every accepted outcome and invariant. No additional
user approval, test cell, lane, model, retry, repair or cleanup subsystem is
required. Approval is blocked only because the D15 READY re-derivation and Gate
G-V7 still state the falsified rule-only setup as the exact READY input. Reconcile
those two lifecycle declarations, then return this same review for iteration 2.

---

# Review iteration 2 — canonical READY/G-V7 reconciliation

## Iteration 2 metadata

| Field | Value |
|---|---|
| Review ID | `GUEST-TRANSPARENT-ROUTE-STEP11-06-ARCH-REVIEW-02` |
| Date | 2026-10-24 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | Only F-01's D15 READY and Gate G-V7 reconciliation; all iteration-1 native, interface, error, cleanup, scope and roadmap assessments remain incorporated |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |
| Roadmap state during review | **PENDING / NON-EXECUTABLE** |

No production source, test body, model, lane, DES event, execution log,
roadmap validation field, or commit was changed by this review iteration. No
new physical-host run was needed: F-01 was a canonical-document consistency
defect, and the retained native evidence was already sufficient.

## F-01 remediation verification

The two formerly contradictory lifecycle declarations now match the detailed
guest adaptation contract exactly.

### D15 READY re-derivation

`feature-delta.md:1274-1289` now states that READY's unchanged guest-network
precondition includes, in order:

- the dummy device with `workload_addr/32`;
- the guest-private route infrastructure for the owner's later
  `SO_MARK=0x295` application sockets;
- `ip rule priority 32765 fwmark 0x295/0xffffffff lookup 295`;
- `ip route add local 0.0.0.0/0 dev lo table 295 proto boot scope host`;
- `net.ipv4.tcp_fwmark_accept=1`, then
  `net.ipv4.fwmark_reflect=1`;
- the existing capture-program, forwarder and inbound-listener requirements.

It retains “READY waits for exactly that and nothing on the host,” keeps the
control session outside READY, and leaves host activation at G-V3. The
correction therefore completes the existing READY promise without changing
its owner or meaning.

### Gate G-V7

`feature-delta.md:12274-12305` now pins the same mark, priority, full mask,
table, host-scope local-default route, protocol, and sysctl order. It also maps
the four setup stages explicitly:

| Stage | Exact typed representation |
|---|---|
| Priority-32765 full-mask rule to table 295 | `GuestTransportSetupError::FwmarkRule { source }` |
| Table-295 host-scope local-default route on `lo` | `GuestTransportSetupError::FwmarkRoute { source }` |
| `net.ipv4.tcp_fwmark_accept=1` | `GuestTransportSetupError::TcpFwmarkAccept { source }` |
| `net.ipv4.fwmark_reflect=1` | `GuestTransportSetupError::FwmarkReflect { source }` |

Every setup failure still exits before READY through the existing start-failure
projection. `overdrive-init` remains the owner; ordering stays after `lo` and
before READY under the existing READY deadline; the control session, Running,
intercept-live and Active remain explicitly unaffected. No new lifecycle gate,
state transition, timeout, retry, repair or cleanup protocol entered the
contract.

The corrected declarations agree with the detailed guest adaptation contract,
DG-5 method/order contract, typed-error display table, ADR-0150 consequences,
lifecycle matrix, P4 placement, and the native falsification record. F-01 is
closed.

## No-weakening and scope recheck

The remediation adds only the configuration and error details omitted from
the two summaries. It does not change any accepted `TcpAccept`, gateway-peer,
non-blocking-connect, timeout, no-payload, READY, control-session,
Running/Active, Service Stable, fail-before-READY, namespace-cleanup, or
security outcome. The dedicated full-mask guest rule still leaves unmarked
traffic unaffected. VM network-namespace destruction remains the sole cleanup
boundary after pre-READY refusal or PID-1/VMM loss.

No public or cross-crate API, wire or persisted format, CLI/config field,
test-support surface, scenario, lane, model variable, runtime task, persistence,
reconciliation or host route was introduced. Existing P4 transport/owner cells
remain the complete bounded DISTILL alignment surface.

No further user decision is required. The native evidence corrected a private
guest-kernel configuration needed to satisfy already-approved D15, INB-1 and
D8-PEER outcomes. The remediation preserves all user-approved outcomes and
owners and makes no new security, lifecycle, compatibility, API, validation-
scope or mechanism choice. Under the iteration-1 authority assessment, the
independently reviewed exact correction may be folded into the accepted
contract without another approval prompt.

## Mechanical and roadmap recheck

| Check | Result | Evidence |
|---|---|---|
| Canonical consistency | Pass | D15 READY and G-V7 now name the same exact mark/rule/route/sysctl set as the guest adaptation contract. |
| Typed error gate | Pass | All four semantic setup failures have distinct pinned variants and retain the existing pre-READY projection. |
| Lifecycle ownership | Pass | `overdrive-init`, the existing READY gate, existing deadline and explicitly unaffected states are unchanged. |
| External validity | Pass | STEP 11-06 remains the additive setup/owner step and the attachment-cut step remains its production invocation; the existing native cell drives the production path. |
| Test boundary | Pass | Bounded alignment stays in existing P4 source-local and native guest-kit cells; no new scenario or lane is introduced. |
| JSON syntax | Pass | `jq` parses `roadmap.json`. |
| Whitespace/patch validity | Pass | `git diff --check` passes for the reviewed design, roadmap and review artifact. |
| Validation state | Pass | `roadmap.validation.status` remains `pending` with `approved_at: null`; no premature self-approval occurred. |

The design owner/orchestrator may now perform the mechanical acceptance fold:
replace the temporary proposed/pending and explicit-user-approval wording with
the independently approved status, preserve the exact contract reviewed here,
and revalidate the roadmap. That reconciliation must not alter the mechanism,
outcomes, error set, test boundary or scope. DELIVER remains gated until the
already-required bounded DISTILL alignment of the existing P4 cells is complete
and independently reviewed.

## Iteration 2 final verdict

**APPROVED.** F-01 is fully remediated. The exact dedicated
`SO_MARK=0x295`, priority-32765 full-mask rule to table 295, table-295
host-scope local-default route, `tcp_fwmark_accept=1` and `fwmark_reflect=1`
contract is consistent across the authoritative READY and G-V7 declarations;
its four setup failures are fully typed; owner, order, cleanup and unaffected
states remain unchanged. The correction is the smallest native-proven
mechanism that restores the accepted host-opened guest-flow outcome. There are
no open technical or authority findings and no further user decision is
required within this scope.
