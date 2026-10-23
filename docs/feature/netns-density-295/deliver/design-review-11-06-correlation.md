# TCP-CORRELATION / STEP 11-06 independent DESIGN review

## Review metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CORRELATION-STEP11-06-ARCH-REVIEW-01` |
| Date | 2026-10-23 |
| Reviewer | `nw-solution-architect-reviewer` |
| Iteration | 1 |
| Scope | The proposed `TCP-CORRELATION` correction only: the pending feature-delta section, ADR-0150 pending correction, STEP 11-06 roadmap impact, Linux 7.0 source, current scaffolds, and the bounded guest-capture spike evidence |
| User authority | Pending; no approval of `TCP-CORRELATION`, a sixth guest program, or additional DISTILL coverage has been given |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 0 critical, 2 high |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** |

The reviewer definition and the `nw-sar-critique-dimensions` and
`nw-roadmap-review-checks` skills were loaded. The repository's native
Markdown, evidence, approval, exact-contract, and no-iteration-cap rules
override the reviewer definition's legacy YAML, Haiku, and two-iteration
defaults.

This review does not approve or implement the proposed sixth program. It
changes no production source, test, DES event, execution log, reviewed design
artifact, model, lane, or kernel state. It performs no physical-host test,
mutation run, KASAN run, commit, or roadmap approval.

## Artifacts and evidence reviewed

- `.context/netns-density-295/11-06-design/correlation.md`.
- `docs/feature/netns-density-295/feature-delta.md`, including the pending
  correction, the accepted `GuestProgram` contract, DG-5, G-V7, and the
  DISTILL placement table.
- `docs/product/architecture/adr-0150-guest-kernel-aya-adaptation-controlled-by-overdrive-init.md`.
- `docs/feature/netns-density-295/deliver/roadmap.json`, especially STEP
  11-06, STEP 12-01, and `validation`.
- `docs/feature/netns-density-295/spike/guest-vsock-capture-findings.md` and
  the bounded probe source under
  `spike-scratch/netns-density-295-guest-vsock-capture/`.
- Current RED scaffolds in `crates/overdrive-init/src/transport/mod.rs` and
  `crates/overdrive-init/src/guest_flow/mod.rs`, plus the existing P4 and
  in-guest cells.
- Linux 7.0 paths in `vendors/linux`: `net/ipv4/tcp_ipv4.c`,
  `net/ipv4/tcp_input.c`, `net/ipv4/tcp_minisocks.c`,
  `net/ipv4/inet_connection_sock.c`, `net/core/filter.c`, and
  `kernel/bpf/cgroup.c`.
- Aya 0.13.1's `EbpfError`, `ProgramError`, `SockOps::load`, and userspace
  hash-map operations from the locally installed crate source.
- `.claude/rules/design.md`, `.claude/rules/development.md`,
  `.claude/rules/spike.md`, and `.claude/rules/testing.md`.

## Verified premise and mechanism

### The accepted five-program contract is insufficient

The specification ambiguity is real and occurs on the production TCP-capture
path:

1. `GuestIoEvent::OutboundCaptured` requires the accepted child, the
   application's original destination, and the application socket's kernel
   cookie (`feature-delta.md` DG-5; `guest_flow/mod.rs:318-329`).
2. Linux runs `connect4` from `tcp_v4_pre_connect` at
   `vendors/linux/net/ipv4/tcp_ipv4.c:206-218`, before
   `inet_hash_connect` assigns the source port at `:301-308`. The hook can
   observe the socket cookie and original destination, but not the allocated
   local port later visible as the intake child's peer port.
3. `sock_release` cannot recover that port. Linux 7.0 permits
   `bpf_sock.src_port` only for the IPv4/IPv6 post-bind attach types, not
   `BPF_CGROUP_INET_SOCK_RELEASE`
   (`vendors/linux/net/core/filter.c:8960-9002`). `sendmsg4` and `recvmsg4`
   are UDP hooks, and `getpeername4` is application-driven.
4. Socket diagnostics after `accept()` cannot supply the required kernel
   cookie after an application reset/release. Treating that absence as a
   destination guess would violate the accepted event identity and fail-closed
   rules.

The current five variants in
`crates/overdrive-init/src/transport/mod.rs:361-389` therefore cannot, by
their accepted meanings, construct the event. DELIVER correctly stopped at
`SPECIFICATION_AMBIGUITY` instead of inventing an interface.

### Active-established SockOps is the evidenced narrow mechanism

The proposed sixth attachment uses a kernel fact at the point it becomes
available, without restoring the removed sockmap/parking datapath:

- Linux invokes `BPF_SOCK_OPS_ACTIVE_ESTABLISHED_CB` from
  `tcp_finish_connect` before the client emits the ACK that makes the server
  child accept-visible (`tcp_input.c:6753-6768`, `:6994-7022`;
  `tcp_minisocks.c:921-945`;
  `inet_connection_sock.c:1382-1442`).
- SockOps exposes the now-assigned local port from `skc_num`
  (`net/core/filter.c:10796-10805`) and the same socket cookie through
  `bpf_get_socket_cookie_sock_ops` (`:5187-5197`).
- The successful native-metal spike attached `flow_sockops` to the guest root
  cgroup and used the active-established callback to join the cookie-indexed
  destination to the local port
  (`bpf/src/main.rs:299-312`, `vfwd/src/owner.rs:618-622`); the intake owner
  then looked it up by the child's peer port (`owner.rs:1001-1017`).

This proves feasibility of the observation and ordering on Linux 7.0. The
proposed production restriction to active-established sockets with a
`Connect4` row is narrower than the spike's combined SockOps program: it adds
no passive-established branch, sockmap install, parking cell, or payload path.
No simpler one of the five accepted hooks supplies the missing post-allocation
join. The rejected socket-diagnostics and `SO_MARK` alternatives are
adequately distinguished.

### Error and readiness mapping is coherent

The setup error mapping uses the already-approved semantic set:

| Failure | Proposed typed representation | Assessment |
|---|---|---|
| Aya object/program load for the sixth program | `GuestTransportSetupError::ProgramLoad { program: GuestProgram::TcpCorrelation, source }` | Exact. Aya 0.13.1 `EbpfError` has `ProgramError(#[from] ProgramError)`, so `SockOps::load()` failures fit the existing carrier without a new variant or string-only distinction. |
| Guest-root-cgroup attach | `ProgramAttach { program: GuestProgram::TcpCorrelation, source: ProgramError }` | Exact. `SockOps::attach()` uses the existing `ProgramError` representation. |
| `Connect4` cannot retain original metadata | Return 0 before rewrite; application receives `EPERM`; no intake child | Supported by `kernel/bpf/cgroup.c:1649-1702`; fail-closed before capture. |
| Accepted child has no exact or has a conflicting correlation row | Reset that child; emit no `GuestIoEvent`; open no flow; send no control message; continue accepting | Correct owning domain. It is a per-connection capture refusal, not a guest-owner fatal or a new setup-error variant. |

Loading and attaching `TcpCorrelation` before `Connect4` and before READY is
the correct gate placement. It preserves READY's existing meaning: guest
network configuration is complete, including all required capture
attachments. It adds no host dependency and does not gate the control session,
Running, intercept-live, or Active. A setup failure remains the existing
pre-READY guest exit. The accepted lifecycle owner remains `overdrive-init`,
whose death ends the VM and the lifetime of its BPF links/maps.

The proposal weakens no accepted `must`, `never`, `exactly`, zero-state,
fail-closed, or no-guessing invariant. Roadmap validation was correctly moved
back to `pending`; STEP 11-06 must not resume from this proposal.

## Findings and dispositions

### F-01 — The new bounded-change state has no complete Contract Shape and lifecycle declaration

- **Severity:** High; blocks DESIGN approval.
- **Claim:** The proposal names a new cookie-indexed input row, a port-indexed
  correlation row, a conflict outcome, and one-time consumption, but does not
  declare the complete bounded-change universe and delta for those effects.
  In particular, it leaves the accepted contract silent about the finite map
  bound and its failure projection, the lifetime of a row when the application
  releases/resets before the child is consumed, and the atomicity required for
  an exact one-time consume while another active-established callback can
  publish.
- **Evidence:** The pending decision says the callback publishes by local port,
  an accept consumes the row, a conflict never guesses, and a row is consumed
  once (`feature-delta.md:908-931`). The handoff itself relies on the reachable
  case where the application resets before intake acceptance
  (`correlation.md:72-84`). The measured spike is weaker than the proposal:
  `FLOWP` is a fixed 65,536-entry `HashMap<u32, Dst>`
  (`bpf/src/main.rs:92-94`), the callback performs a `BPF_ANY` insert and
  discards its result (`:308-311`), the value carries no cookie, and the owner
  performs a non-consuming lookup (`owner.rs:1001-1017`). The handoff correctly
  identifies the added cookie as an inference rather than measured evidence
  (`correlation.md:52-56`). No proposed Contract Shape section closes the gap.
- **Reachability boundary:** This finding does not assert an unproven current
  production data race or demand a new public map API. The application-reset
  path and concurrent kernel/user owners are already in the accepted path;
  the exact collision/cleanup implementation is absent because STEP 11-06 is
  still RED. The blocking issue is that the proposed stronger guarantees
  exceed the measured spike without a complete state-transition contract.
- **Required revision:** Declare `TCP-CORRELATION` as bounded-change and name
  its universe and observable delta for `Connect4` retention, active-established
  publication, intake consumption, application release/reset before
  consumption, and `overdrive-init` teardown. Pin the bound or invariant that
  prevents correlation capacity from introducing an earlier unapproved TCP
  refusal, and pin the fail-closed result of publication/capacity failure.
  Require exact one-time consumption under concurrent publication and state
  that a stale row can never identify a later child. Leave private map types,
  keys beyond the already approved identity, helper signatures, and internal
  state threading to the crafter.
- **Disposition:** Open. The current prose is not sufficient for an exact
  implementation handoff.

### F-02 — “No new test changes” contradicts the mandatory SockOps Tier-3 gate

- **Severity:** High; blocks DESIGN completion and roadmap revalidation.
- **Claim:** The proposal categorically says no new test change is required
  and authorizes only changing five-to-six setup-order assertions. That cannot
  be the validation contract for a new SockOps program.
- **Evidence:** `correlation.md:115-116,127-135` and
  `feature-delta.md:966-972` exclude a new scenario. Repository testing rules
  state that SockOps has no meaningful `BPF_PROG_TEST_RUN` Tier-2 backstop and
  moves entirely to Tier 3 (`.claude/rules/testing.md:1588-1593`), and that
  every new eBPF program lands with hook-appropriate coverage or does not
  merge (`:1879-1890`). Existing `transport::real_kernel` proves setup
  postconditions, and existing `guest_flow::real_kernel` cells exercise the
  inbound owner path; neither drives an ordinary outbound TCP `connect()`
  through `Connect4` + active-established correlation + the production intake
  adapter and observes the required destination/cookie event. The successful
  throwaway spike is contract-shaping evidence, not a merge-gating production
  test (`.claude/rules/spike.md`). STEP 11-06's roadmap notes still enumerate
  only the five hooks and direct the crafter to spike lines 169-283, excluding
  the SockOps callback at lines 299-312 (`roadmap.json:973`).
- **Required revision:** Remove the “no new test changes” prohibition. Route
  the correction through DISTILL so the acceptance designer owns the exact
  executable shape. The required evidence is Tier 3 through the production
  guest-kit composition for the new callback and fail-closed miss/conflict
  outcome. The existing native guest-kit lane may host it; this finding does
  not authorize a new lane, test-support API, or model variable. Update STEP
  11-06 criteria, source references, and verification only after the user
  explicitly approves that bounded validation scope.
- **Disposition:** Open. No test or lane was added by this review.

### A-01 — Explicit user approval is still required for the interface change

- **Severity:** Authority gate; blocking, not counted as a technical finding.
- **Claim:** `GuestProgram` is Rust `pub(crate)`, but the sixth variant and the
  required guest-root-cgroup attachment still change the design-owned setup
  interface, READY prerequisite, BPF object/program set, and operator-visible
  typed setup-error label. The statement that there is no public API change is
  therefore inaccurate at the architecture-contract level.
- **Evidence:** The accepted canonical enum still has five variants
  (`feature-delta.md:6931-6966`; `transport/mod.rs:361-389`), and G-V7's READY
  promise still names those five (`feature-delta.md:11985-12008`). The pending
  proposal adds `GuestProgram::TcpCorrelation`,
  `as_str() == "tcp_correlation"`, a sixth attachment, and exact load/attach
  error payloads (`feature-delta.md:908-950`). The design rule requires a
  material port/action/lifecycle decision to be surfaced to the user before
  acceptance.
- **Required disposition:** Revise “no public/cross-crate Rust API” to say
  that no language-level `pub` or cross-crate Rust type, wire format, or
  persisted format changes, while the exact guest setup contract and READY
  attachment set do change. After F-01 and F-02 are resolved, surface the
  numbered decision below to the user. Silence, this review, or the pending
  artifact is not approval.

## Canonical and roadmap reconciliation required after approval

Keeping the accepted five-program sections unchanged while the proposal is
unapproved is correct. If the user approves the corrected decision, the design
owner must fold it into the current contract rather than leave a permanent
parallel pending section:

1. Replace the accepted five-variant `GuestProgram` declaration at
   `feature-delta.md:6965` with the approved six-variant order and add the
   decision to the decision/approval index.
2. Update G-V7's READY promise at `feature-delta.md:11985-12008` to name
   `TcpCorrelation` before `Connect4` and retain the exact failure and
   unaffected-state clauses.
3. Fold the approved correction into ADR-0150 in present tense and remove its
   pre-implementation “pending correction” narration. This is correction of
   one unimplemented decision, not amendment history.
4. Update STEP 11-06 so the sixth program, SockOps source range, Contract
   Shape, and approved Tier-3 obligation are part of its criteria and
   verification. Keep roadmap `validation.status = pending` until independent
   post-approval review and roadmap revalidation pass.

## Architecture review dimensions

| Dimension | Assessment | Evidence |
|---|---|---|
| Architectural bias | Pass | The proposal adds the one kernel observation point proven necessary. It does not restore the removed sockmap/parking datapath or introduce a userspace proxy, token protocol, model, service, or lane. |
| ADR quality | Pass with reconciliation required | Context, chosen mechanism, alternatives, consequences, and pending status are present. After approval the correction must be folded into ADR-0150 rather than retained as pre-implementation amendment history. |
| Completeness | Fail | The new stateful bounded-change effect lacks a complete universe/delta and lifecycle/capacity contract (F-01), and the validation plan contradicts mandatory SockOps coverage (F-02). |
| Feasibility | Pass for the core mechanism | Linux 7.0 source and the native-metal spike prove the active-established cookie/local-port observation and callback-before-accept ordering. The stronger consume/conflict lifecycle remains to be specified and validated. |
| Priority/data justification | Pass | The five accepted hooks cannot provide the required join; the proposed callback directly closes that gap without changing payload forwarding. |

## Roadmap checks

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | Pass for the full roadmap | STEP 11-06 builds the additive half; STEP 12-01 invokes the guest setup from the production pre-READY path. The complete roadmap has an invocation path. |
| Acceptance-criterion coupling | Changes required | The exact program set and typed setup errors are legitimate DESIGN-owned contracts, but STEP 11-06 still describes only the five-hook baseline and omits the approved correlation outcome. |
| Step decomposition | Pass | The sixth program belongs to the existing guest-program/setup/owner step. It does not justify a new implementation step or subsystem. |
| Implementation code | Pass | The exact enum order belongs in the feature delta as interface contract. Private map representation and helper signatures remain unpinned. |
| Concision and precision | Pass for the correction | The pending section is bounded to the missing join; F-01 requires precise state outcomes, not a larger architecture. |
| Test boundary | Fail | A new SockOps hook requires Tier-3 production-composed evidence; the proposal currently forbids it (F-02). No Rust unit test may substitute for hook invocation. |

## Exact user decision required after revision

The user must explicitly approve or reject a revised numbered decision with
all of these clauses together:

> **TCP-CORRELATION:** Add `GuestProgram::TcpCorrelation` as a sixth,
> guest-root-cgroup SockOps attachment, loaded and attached before `Connect4`
> and before READY. On active establishment of a socket recorded by
> `Connect4`, publish the exact application cookie and original destination
> for the intake child; consume the exact correlation once. Any retention,
> publication, capacity, missing-row, conflict, stale-row, or consume failure
> fails closed without guessing, emitting `OutboundCaptured`, opening a flow,
> or sending control traffic, under the bounded state/lifecycle contract added
> for F-01. Setup failures use the existing `ProgramLoad` and `ProgramAttach`
> variants with `GuestProgram::TcpCorrelation`. Keep the existing five
> programs, event/wire/persisted types, flow lifecycle, and active-only/no-
> sockmap/no-parking boundary. Authorize DISTILL to add the mandatory
> correlation-specific Tier-3 evidence in the existing guest-kit lane and to
> update existing setup-order assertions; authorize no new lane, model, public
> test-support API, or unrelated mechanism.

The design owner must first make F-01's bounded contract concrete. The user
should not be asked to approve the current shorter wording because it also
asserts that no test change is required and does not expose the complete state
lifecycle/capacity decision.

## Verdict

**CHANGES_REQUESTED.** The sixth active-established SockOps attachment is the
technically justified and narrowly scoped correction to the proven five-hook
gap. Its Linux ordering, core feasibility, setup-error reuse, READY placement,
and fail-closed direction are sound. The proposal is not approval-ready until
the bounded correlation-state contract is complete and the mandatory Tier-3
validation scope is surfaced for explicit user approval. Roadmap validation
remains pending, STEP 11-06 remains paused, and no implementation or DISTILL
change is authorized by this review.

---

# Review iteration 2 — bounded state and Tier-3 remediation

## Iteration 2 metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CORRELATION-STEP11-06-ARCH-REVIEW-02` |
| Date | 2026-10-23 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | Only the iteration-1 remediation in the pending feature-delta section, ADR-0150, and the bounded handoff; the accepted roadmap body remains intentionally unchanged |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 2 critical, 1 high |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** |

Iteration 2 re-read the revised proposal, its exact user-decision wording,
the accepted D15 and D25 contracts, the current owner tests, the spike's BPF
maps and owner lookup, and the cited Linux 7.0 port-allocation, established-
tuple, bind-conflict, route, and `getpeername` paths. It did not change or run
production code, tests, DES state, the roadmap body, or any kernel fixture.

## Remediation that is complete

### Iteration-1 F-02 — mandatory Tier-3 ownership and lane

The revision now acknowledges that a new SockOps program needs a new
correlation-specific Tier-3 scenario. It places that scenario in the existing
disposable guest-kit owner lane, keeps the executable body under DISTILL
ownership, and authorizes no new lane, model, benchmark, production test branch,
or public test-support API (`feature-delta.md:1028-1054`;
`correlation.md:165-193`). This closes the original finding for the success,
uncorrelated-intake miss, and one-time-consume boundaries.

The conflict/stale portion is not yet acceptable evidence because it is
seeded rather than driven by its claimed production producer. That narrower
remaining issue is iteration-2 F-03 below.

### Iteration-1 A-01 — interface-change wording

The revision accurately distinguishes language visibility from architecture:
no language-level `pub` or cross-crate Rust type, wire type, persisted format,
event variant, flow lifecycle, or model variable changes, while the
design-owned Rust `pub(crate)` guest setup contract and READY attachment set do
change from five programs to six (`feature-delta.md:1020-1026`;
`correlation.md:115-118,165-170`). The wording defect is closed. Explicit user
approval remains an authority gate and is correctly still pending.

### Error representation and READY placement

The iteration-1 positive assessment stands. `ProgramLoad` can carry an Aya
`ProgramError` through `EbpfError::ProgramError`; `ProgramAttach` already
carries `ProgramError`. `Empty` / `Conflict` at intake is a per-connection
refusal, not a setup failure or `GuestOwnerFatal`. Attaching the sixth program
before `Connect4` and before READY preserves the existing state owner and
deadline. No new typed error is needed for the design as currently bounded.

## Iteration 2 findings

### F-01 — Terminal cleanup silently weakens the accepted per-socket original-peer contract

- **Severity:** Critical.
- **Claim:** The revised state lifecycle deletes `origin[cookie]` at every
  owner-terminal flow path and asserts that no peer-name promise exists after
  that flow ends. The accepted contract keeps the original destination **per
  socket** and uses `getpeername4` to restore it. Flow termination and socket
  release are different events; a cleanly ended application socket can remain
  open in a state where Linux still invokes `getpeername4`.
- **Accepted contract evidence:** ADR-0150 says the capture programs “keep the
  original destination per socket” and `getpeername4` restores it
  (`adr-0150:95-99`). D15 says programs see the original peers of outbound
  connections (`:38-42`). The successful spike implements that lifetime:
  `getpeername4` reads `ORIG[cookie]` and only `sock_release` removes it
  (`spike-scratch/.../bpf/src/main.rs:262-286`).
- **Revised contradiction:** The pending contract removes the retained origin
  on pending-limit refusal, attach refusal, control refusal, abort, pair end,
  or session loss and declares that no peer-name promise remains after flow
  end (`feature-delta.md:971-976`; ADR-0150 summarizes the same transition at
  `:108-115`). It simultaneously says the five accepted programs and their
  meanings stay unchanged (`feature-delta.md:908-920`). Both cannot hold.
- **Native reachability:** Linux `inet_getname` rejects peer lookup only for
  `TCP_CLOSE` and `TCP_SYN_SENT`; in every other TCP state it invokes
  `CGROUP_INET4_GETPEERNAME` (`vendors/linux/net/ipv4/af_inet.c:809-829`). A
  clean pair end clears linger and gives the application EOF while the owner
  removes its flow (`tcp_connect.rs:90-100`; `flow_end.rs:29-75`). The
  application can retain that socket after EOF and call `getpeername`; owner
  flow removal is not native evidence that the socket is released or that the
  hook cannot run.
- **Invariant impact:** This is an unapproved relaxation of D15, not private
  map cleanup. The combined user-decision wording says to preserve the five
  programs and existing contracts but does not quote the concrete weakening:
  `getpeername` may cease to restore the original peer after owner flow end and
  before socket release (`correlation.md:208-227`). It therefore cannot supply
  the explicit approval required by the contract-weakening gate.
- **Required revision:** Reconcile retained-origin lifetime with the accepted
  per-socket `getpeername4` promise. If the proposal intends to shorten that
  promise, the exact observable change and affected TCP states must be surfaced
  as a separate explicit user weakening. Do not describe owner-terminal cleanup
  as preserving the existing capture meaning. Private storage and cleanup
  mechanics remain the crafter's responsibility after the observable lifetime
  is pinned.
- **Disposition:** Open. Iteration-1 F-01 is not closed.

### F-02 — Linux serializes a full TCP tuple, not the port-only identity the proposal claims

- **Severity:** Critical.
- **Claim:** The revised contract says the two 65,536-entry collections add no
  admission bound below the 4,096-flow quota and cites Linux's established-
  tuple check to justify that no different socket can publish the same live
  port during a take. The cited kernel path compares the full address pair and
  port pair. Two sockets with different guest-local source addresses may hold
  the same source port and connect concurrently to the fixed intake, producing
  the same correlation-slot key.
- **Kernel evidence:** `__inet_check_established` builds its identity from
  `inet_rcv_saddr`, `inet_daddr`, destination port, and local port and rejects
  only an `inet_match` of that full tuple
  (`vendors/linux/net/ipv4/inet_hashtables.c:575-623`; `inet_match` compares
  `sk_addrpair` and `sk_portpair`,
  `include/net/inet_hashtables.h:345-368`). Explicit binds on distinct local
  addresses do not conflict merely because their ports match:
  `inet_csk_bind_conflict` reports a conflict only when
  `inet_rcv_saddr_equal` also holds
  (`net/ipv4/inet_connection_sock.c:227-265`). `tcp_v4_connect` preserves an
  explicitly bound source address (`tcp_ipv4.c:251-282`), and route lookup
  admits it when it is a local guest address
  (`include/net/route.h:319-355`; `net/ipv4/route.c:2766-2769`). The guest
  has both `lo` and `workload_addr`; the root-cgroup capture contract does not
  exclude an ordinary application socket explicitly bound to either.
- **Contract consequence:** With a slot keyed only by `u16` port, two such
  kernel-valid connections change the slot to `Conflict` and both can be reset
  even though two of 4,096 flow places are in use. That is an admission refusal
  below the accepted quota, directly contradicting
  `feature-delta.md:942-949`. The existing spike used default source selection
  and did not falsify this case.
- **Native falsification disposition:** The Linux source invalidates the
  proposal's port-serialization premise. A private seeded `Conflict` state
  does not repair that premise or establish whether the resulting refusal is
  an acceptable D15 residual. The dependent “no lower admission bound” claim
  is invalid and must remain pending under the native falsification gate.
- **Required revision:** Re-establish an exact identity/collision contract
  from production evidence. If a kernel-valid same-port/different-source
  connection is intentionally refused, surface that concrete observable D15
  weakening and its bound for explicit user approval. If the design claims the
  producer is unreachable, prove that through the real guest capture path
  before retaining `Conflict` machinery. This review does not select a map key,
  add a public API, or prescribe an implementation.
- **Disposition:** Open.

### F-03 — The seeded conflict fixture does not prove the production conflict transition

- **Severity:** High.
- **Claim:** The proposed Tier-3 scenario drives the success and empty-row
  paths through production, but permits a private fixture to seed
  `Conflict` / stale slot state. That proves the consumer's refusal after a
  test-only write; it does not prove the active-established producer's
  `Ready → Conflict` transition or the claimed production ordering.
- **Evidence:** The revision explicitly authorizes source-local seeded
  conflict/stale state (`feature-delta.md:1041-1048`;
  `correlation.md:177-188`). The current production scaffold contains no
  correlation map or publisher, and the native spike used a `BPF_ANY` write,
  discarded its result, carried no cookie in `FLOWP`, and never produced a
  `Conflict` state (`bpf/src/main.rs:92-94,299-312`).
- **Reachability rule:** The repository requires the real production producer
  and owner path before a failure state becomes a design requirement. F-02
  supplies a Linux-source candidate producer, but the revised validation does
  not drive it. A fixture-seeded state cannot substitute for that production
  path and must not be the evidence that justifies new collision machinery.
- **Required revision:** DISTILL may keep a private source-local helper for a
  lower-level transition check, but the contract-shaping Tier-3 evidence must
  drive any claimed conflict through the real `Connect4` + active-established
  publisher + `KernelGuestOwnerIo` path, or the design must retain it only as
  an unproven hypothesis and remove it from the approval/merge claim. No new
  lane or production test seam is authorized by this finding.
- **Disposition:** Open. Iteration-1 F-02 is closed only for success, empty,
  and one-time consumption; conflict/stale evidence remains incomplete.

## Iteration 2 architecture and roadmap assessment

| Dimension | Assessment | Evidence |
|---|---|---|
| Architectural bias | Pass for the sixth hook | Active-established SockOps remains the narrow evidenced observation point. The new `Conflict` machinery is not approved until its real producer and consequence are resolved. |
| ADR quality | Fail | ADR-0150 simultaneously preserves per-socket original-peer restoration and removes the record at owner flow end. The material D15 consequences are absent from the user decision. |
| Completeness | Fail | The bounded universe is now enumerated, but two lifecycle/identity premises contradict accepted contracts and Linux source (F-01, F-02). |
| Feasibility | Partial | The callback and success join are feasible. Port-only collision handling and terminal cleanup are implementable but do not yet preserve the approved observable contract. |
| Priority/data justification | Partial | The sixth callback addresses the proven gap. Fixture-seeded conflict handling adds mechanism whose real producer is not exercised by the proposed evidence. |

| Mandatory roadmap check | Iteration 2 result | Evidence |
|---|---|---|
| External validity | Pass | STEP 12-01 remains the production call-site cut. |
| Acceptance-criterion coupling | Pending | After approval the roadmap must carry the corrected behavior, but the exact behavior is not yet approved. Keeping its body unchanged now is correct. |
| Step decomposition | Pass | No new implementation step or lane is justified. |
| Implementation code | Pass | The proposal leaves private map representation and helpers to DELIVER. |
| Concision and precision | Fail | “No bound below 4,096” and “no peer-name promise after flow end” conceal material observable changes rather than precisely reconciling them. |
| Test boundary | Fail for conflict/stale only | The existing Tier-3 lane is correct, but a seeded private state does not prove its production producer. |

## Iteration 2 authority gate

The proposal must not be sent for the current combined approval. Before the
user decides, the architect must resolve F-01 through F-03 and rewrite the
decision around the resulting exact behavior. If either relaxation remains,
the user-facing decision must say explicitly that:

1. a still-open TCP socket may stop receiving original-peer restoration from
   `getpeername4` after the owner flow ends but before `sock_release`; and/or
2. two otherwise kernel-valid captured TCP connections that share a local port
   across different local source addresses may both be reset as a correlation
   conflict below `GUEST_TCP_PENDING_MAX`.

Approval of “terminal cleanup,” “Conflict,” or generic fail-closed wording is
not explicit approval of those observable weakenings. If the revised design
preserves both accepted outcomes, neither waiver is required. The sixth
attachment, its setup errors, the corrected bounded state, and its DISTILL
authorization still require one explicit user decision after independent
review.

## Iteration 2 verdict

**CHANGES_REQUESTED.** The revision correctly adds a bounded-change section,
recognizes the interface-contract change, and authorizes a Tier-3 scenario in
the existing lane. It does not yet close the design gap. Terminal owner cleanup
conflicts with the accepted per-socket `getpeername4` lifetime, and Linux's
full-tuple uniqueness falsifies the claim that a port-only slot creates no
refusal below the 4,096-flow quota. The seeded conflict fixture does not prove
the real producer transition. Roadmap validation remains pending, STEP 11-06
remains paused, and no implementation, test change, user approval, or commit is
authorized by this iteration.

---

# Review iteration 3 — full-tuple and child-cookie handoff

## Iteration 3 metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CORRELATION-STEP11-06-ARCH-REVIEW-03` |
| Date | 2026-10-23 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | The revised two-callback full-tuple → accepted-child-cookie handoff, its bounded state, and its production-driven Tier-3 obligation |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 0 critical, 1 high |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** |

Iteration 3 did not implement or run the proposal. It re-read the revised
feature delta, ADR-0150, handoff, accepted guest-owner seam, relevant native
cells, Aya 0.13.1 / aya-ebpf 0.1.1 SockOps and cookie interfaces, and Linux
7.0's active/passive handshake, accept queue, child locking, tuple identity,
bind, route, and cookie paths.

## Iteration 2 findings resolved

### Iteration-2 F-01 — per-socket original-peer lifetime

**Closed.** The accepted origin fact now remains keyed by application socket
until `sock_release`, regardless of owner flow end
(`feature-delta.md:953-956,981-984,994-996`; ADR-0150:98-116). This matches
the accepted `getpeername4` contract and the spike's `ORIG` lifetime. The
combined user decision expressly preserves that lifetime
(`correlation.md:220-240`). No D15 peer-restoration weakening remains.

### Iteration-2 F-02 — port-only collision

**Closed.** The active handoff is keyed by the full client tuple, including
the source IPv4 address and source port; the passive callback re-keys under
the exact accepted child's kernel cookie (`feature-delta.md:921-940`). The
identity now matches Linux's full address-pair/port-pair uniqueness. Two
same-port sockets on distinct local addresses remain distinct and the
production-driven Tier-3 obligation explicitly exercises both
(`:1065-1069`). `Conflict` and the below-quota refusal it introduced are
removed.

### Iteration-2 F-03 — manufactured conflict state

**Closed.** The seeded state and `Conflict` representation are gone. The one
parameterized Tier-3 scenario now drives success,
same-port/different-source, reset/release before accept plus admitted tuple
reuse, direct uncorrelated intake, and one-time consumption through the real
`Connect4` → active SockOps → passive SockOps → `KernelGuestOwnerIo` path
(`feature-delta.md:1052-1084`; `correlation.md:184-203`). It adds no lane,
model, benchmark, production branch, or public test-support API.

## Verified two-callback ordering and identity

The revised kernel mechanism is feasible and closes the previous aliasing
window:

1. `Connect4` records the application's origin by application cookie before
   the destination rewrite. Active-established runs after source-port
   allocation and before the client emits the final ACK
   (`tcp_ipv4.c:206-218,301-308`; `tcp_input.c:6753-6768,6994-7022`). Its
   SockOps context exposes the full local/remote IPv4 tuple, and the cookie
   helper returns the same application socket cookie.
2. The server creates and queues its child while processing that ACK
   (`tcp_minisocks.c:921-945`;
   `inet_connection_sock.c:1407-1443`). `tcp_child_process` then runs
   passive-established before calling the listening parent's
   `sk_data_ready` (`tcp_input.c:7268-7296`;
   `tcp_minisocks.c:992-1007`).
3. Queue insertion before the passive callback does not let userspace return
   the child early. The child is cloned with its bottom-half socket lock held;
   `tcp_child_process` releases that lock only after the passive callback
   (`tcp_minisocks.c:992-1017`; `net/core/sock.c:2470-2508`). Even if another
   readiness event lets `inet_csk_accept` dequeue it, `inet_accept` must
   acquire `lock_sock(child)` before returning the accepted socket
   (`inet_connection_sock.c:650-706`; `af_inet.c:788-802`). The child-cookie
   row is therefore written before `KernelGuestOwnerIo` can obtain the fd and
   read `SO_COOKIE`.
4. Passive SockOps and `SO_COOKIE` use `__sock_gen_cookie` / `sock_gen_cookie`
   on the same accepted child (`filter.c:5187-5197`;
   `net/core/sock.c:2109-2113`). The userspace lookup key is the exact cookie
   the passive callback wrote.

This is a metadata-only join. The passive branch installs no socket, parks no
bytes, and changes no EVMOD-PAIR lifecycle. Missing/capacity state remains a
per-connection reset with no event, flow, or control message. The existing
setup error taxonomy and pre-READY ownership remain exact.

## Iteration 3 finding

### F-01 — The active-transient release transition is contradictory and its non-passive lifetime is not exact

- **Severity:** High; blocks approval of the bounded-change contract.
- **Claim:** The canonical proposal and its handoff give different
  `sock_release` deltas. The feature delta says release removes **only** the
  per-socket origin and never mentions removal of the active full-tuple row;
  the handoff says release may conditionally remove that socket's
  still-unconsumed active transient. At the same time the feature delta says
  every handoff row lives only between establishment and userspace accept,
  while its later-active transition and teardown rule allow an active row
  that never reaches passive-established to survive until same-tuple
  replacement or owner teardown.
- **Evidence:** `feature-delta.md:965-977` declares rows live only until
  userspace accept. Its normative delta 2 permits replacement of an
  unconsumed active row, delta 4 says `sock_release` removes only origin, and
  delta 6 drops remaining state at teardown (`:985-1002`). The bounded handoff
  instead says `SockRelease` “may conditionally remove” that socket's active
  transient (`correlation.md:139-155`). The exact combined user decision does
  not choose either release behavior (`:218-240`).
- **Reachability:** The design itself admits missing active/passive handoffs
  and capacity refusal. An active callback can publish before the server has
  completed the passive side; a connection whose final ACK never produces a
  child, or whose server request is dropped before passive establishment,
  leaves precisely the unconsumed active state whose lifetime must be pinned.
  This is part of the proposed failure model, not a new recovery subsystem.
- **Why it matters:** Both choices can fail closed, but they define different
  bounded state and capacity behavior over a long guest boot. “May remove” is
  not an exact transition a crafter can implement or a reviewer can check. If
  release removes by full tuple, it must also be defined so an old socket's
  release cannot delete a later socket's row after admitted tuple reuse. If it
  does not remove, the contract must state the row survives until passive
  take, same-tuple replacement, or teardown and that such rows count toward
  the 65,536 bound.
- **Required revision:** Select one observable state transition and make the
  feature delta, ADR summary, handoff, combined user decision, and Tier-3
  obligation consistent. Pin the conditional identity rule if release may
  remove a row, or pin the exact stale-row lifetime and capacity consequence
  if it may not. This is a private-state/lifecycle contract; no new public API,
  map, timer, audit, retry, or test lane is authorized by the finding.
- **Disposition:** Open.

## Iteration 3 architecture and roadmap assessment

| Dimension | Assessment | Evidence |
|---|---|---|
| Architectural bias | Pass | The two callbacks use the kernel's existing connection identities and add no payload or forwarding machinery. |
| ADR quality | Pass except F-01 | Context, decision, alternatives, consequences, and validation are coherent; the active-row release delta must be reconciled. |
| Completeness | Fail narrowly | Full-tuple and child-cookie identity, bounds, failures, teardown, and tests are explicit. One active-row lifetime has contradictory owners/transitions. |
| Feasibility | Pass | Linux ordering, child locking, SockOps tuple fields, and shared child cookie support the mechanism. |
| Priority/data justification | Pass | The sixth program closes the proven five-hook gap; the second callback is required to bind a queued/reset child to its immutable kernel identity. |

| Mandatory roadmap check | Iteration 3 result | Evidence |
|---|---|---|
| External validity | Pass | STEP 12-01 remains the production pre-READY call-site cut. |
| Acceptance-criterion coupling | Pending | Keeping STEP 11-06 unchanged before approval is correct; it must receive the final two-callback criterion and counts after approval. |
| Step decomposition | Pass | Both callbacks are one correlation program in the existing guest setup step. |
| Implementation code | Pass | Public/interface identity and transitions are pinned; private map representation remains DELIVER-owned. |
| Concision and precision | Fail narrowly | `sock_release` “only” versus “may conditionally” is not single-interpretation. |
| Test boundary | Pass | Every contract-shaping case uses the real producer path in the existing Tier-3 lane; no seeded state substitutes for production. |

## Iteration 3 authority gate

The sixth active/passive SockOps attachment remains a material, unapproved
guest setup and READY-contract change. Once F-01 is reconciled and independently
re-reviewed, the exact combined decision may be surfaced to the user. The user
must approve the sixth attachment, the two bounded handoffs and capacity
failure, the chosen active-row release/lifetime rule, existing error mappings,
and the specified DISTILL changes together. Reviewer agreement and the pending
proposal are not user authority.

## Iteration 3 verdict

**CHANGES_REQUESTED.** Iteration 3 resolves the accepted-peer lifetime,
port-collision, and manufactured-test-state findings. The full-tuple →
accepted-child-cookie join is technically sound, preserves the five accepted
program meanings, and has an honest production-driven Tier-3 plan. Approval is
blocked only by the contradictory lifetime of an active transient that never
reaches passive establishment. Roadmap validation remains pending, STEP 11-06
remains paused, and no implementation, test change, user approval, or commit is
authorized by this iteration.

---

# Review iteration 4 — conditional active-row release

## Iteration 4 metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CORRELATION-STEP11-06-ARCH-REVIEW-04` |
| Date | 2026-10-23 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | The one selected active-transient `sock_release` rule and its interaction with passive establishment, tuple reuse, and the required Tier-3 outcome |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 1 critical, 0 high |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** |

Iteration 4 verified the revised wording across the feature delta, ADR-0150,
bounded handoff, and combined decision. Those artifacts now consistently state
the selected compare-by-application-cookie cleanup rule. The review then traced
the rule through Linux 7.0's actual `sock_release`, active connect completion,
final-ACK/passive establishment, and Aya eBPF hash-map operations. No code or
test was run or changed.

## Iteration 3 documentary finding resolved

**Closed as a consistency finding.** All reviewed proposal artifacts now say:

- `Ready` and `Reject` active values carry `application_cookie`;
- active establishment records its full-tuple key in cookie-addressable private
  per-socket correlation state;
- `sock_release` removes the accepted origin and removes the active row only
  when the row still carries the releasing cookie;
- a later publisher with another cookie survives an older release;
- passive child-cookie rows are never removed by application release; and
- active and passive lifetimes and their 65,536-entry bounds are explicit.

The exact contract appears at `feature-delta.md:953-1018`, ADR-0150:103-118,
`correlation.md:117-166`, and the combined decision at `:223-249`. The prior
“only” versus “may conditionally” contradiction is gone.

## Iteration 4 finding

### F-01 — Conditional `sock_release` cleanup can delete the handoff before passive establishment

- **Severity:** Critical.
- **Claim:** The selected release rule is internally consistent but does not
  preserve the proposal's required reset/release-before-accept outcome. Linux
  runs the cgroup `sock_release` hook at application file release, before
  `tcp_close`. There is no kernel ordering that requires the server's passive-
  established callback to have consumed the active row before the application
  can close the connected socket. If release wins that race, it removes the
  same-cookie active row; the later passive callback observes an empty active
  handoff, publishes `Reject`, and the owner resets the child without emitting
  the exact `OutboundCaptured` the proposal requires.
- **Client-side source evidence:** Active completion sets `TCP_ESTABLISHED` and
  runs the active SockOps callback while processing the SYNACK
  (`vendors/linux/net/ipv4/tcp_input.c:6753-6768`). The connecting task is
  woken by `sk_state_change` after `tcp_finish_connect`; the final ACK is sent
  in that same path (`:6994-7022`). The server processes that ACK separately
  to create the child and run passive-established
  (`tcp_minisocks.c:921-945`; `tcp_input.c:7268-7296`). Sending the ACK does
  not synchronize application file release with the server callback.
- **Release-hook source evidence:** `inet_release` invokes
  `BPF_CGROUP_RUN_PROG_INET_SOCK_RELEASE(sk)` before calling the protocol's
  `close` operation (`vendors/linux/net/ipv4/af_inet.c:419-444`). The hook can
  therefore run as soon as the application closes after `connect()` returns,
  while the final ACK is still awaiting passive processing.
- **Contract contradiction:** The revised delta requires application
  reset/release after establishment but before userspace accept to preserve the
  queued child's exact handoff, and makes that a real-producer Tier-3 case
  (`feature-delta.md:1080-1082`). Its selected delta 4 instead removes the
  unconsumed active row on same-cookie release (`:1000-1005`). If release
  precedes passive take, both requirements cannot hold.
- **Tuple-reuse atomicity:** Cookie comparison also does not by itself make a
  compare-and-delete atomic. Aya eBPF exposes hash-map lookup and delete as
  separate operations (`aya-ebpf-0.1.1/src/maps/hash_map.rs:50-84`), and Linux
  exposes `bpf_map_delete_elem` as a separate helper
  (`kernel/bpf/helpers.c:77-84`). Once an old socket is `TCP_CLOSE` and
  unhashed, a later same-tuple active publisher can race between an old
  release's lookup and delete; the old delete can then remove the new row
  despite the earlier comparison. The proposal pins an atomic semantic result
  without naming a kernel primitive or reproduced ordering that makes it
  implementable.
- **Evidence-gate disposition:** This review does not claim a production
  implementation failure—the implementation is still RED. It identifies an
  unproven ordering premise in the proposed contract. The existing spike never
  implemented same-cookie release cleanup, and no bounded native result proves
  that release always follows passive take or that compare-and-delete is
  atomic. A future Tier-3 test after approval cannot retroactively make an
  ambiguous contract safe enough to approve now.
- **Required revision:** Preserve the already-approved reset-before-accept and
  tuple-reuse outcomes with an exact active-row lifetime whose kernel ordering
  is supported by Linux 7.0 source or a bounded native spike. If conditional
  release cleanup is retained, prove and record the ordering/atomic primitive
  that prevents both early deletion and deletion of a later publisher. If the
  design chooses another lifecycle, state its capacity consequence exactly.
  Do not add a public API, timer, audit, retry, extra map, production seam, or
  lane without explicit user authority.
- **Disposition:** Open. The selected rule is not ready for user approval.

## Iteration 4 architecture and roadmap assessment

| Dimension | Assessment | Evidence |
|---|---|---|
| Architectural bias | Pass for the two-callback join | The core identity mechanism remains narrow; the release cleanup is the only unresolved part. |
| ADR quality | Fail | The consequences promise reset-before-accept exactness while the selected release transition can erase its producer state. |
| Completeness | Fail | State names and nominal lifetimes are complete, but their cross-CPU ordering and conditional deletion are unsupported. |
| Feasibility | Fail for release cleanup | Linux calls `sock_release` before `tcp_close`; no atomic cookie-checked delete or passive-before-release proof is provided. |
| Priority/data justification | Pass | The sixth program still addresses the proven correlation gap. |

| Mandatory roadmap check | Iteration 4 result | Evidence |
|---|---|---|
| External validity | Pass | The full roadmap still invokes the guest setup at STEP 12-01. |
| Acceptance-criterion coupling | Pending | The final lifecycle must be approved before STEP 11-06 is reconciled. |
| Step decomposition | Pass | No new step or lane follows from this finding. |
| Implementation code | Pending | The interface invariant is not yet implementable from a proven kernel primitive; private mechanics remain unchosen. |
| Concision and precision | Pass syntactically | The selected rule is stated consistently, but consistency does not establish correctness. |
| Test boundary | Pass in shape, pending premise | The proposed scenario uses the real path, but its expected reset-before-accept result currently contradicts the release delta. |

## Iteration 4 authority gate

The current combined decision must not be sent for approval. It asks the user
to approve both same-cookie active-row deletion and exact handoff after release,
which are not jointly established. Once F-01 is corrected and independently
reviewed, the user must still explicitly approve the sixth active/passive
SockOps attachment, both bounded handoffs, the final release/capacity rule,
existing error mappings, and the bounded DISTILL changes. Roadmap validation
must remain pending until the post-approval fold and independent roadmap review.

## Iteration 4 verdict

**CHANGES_REQUESTED.** The proposal is textually consistent and the core
full-tuple → accepted-child-cookie join remains sound. The chosen
`sock_release` cleanup is not: it can run before passive establishment and
erase the handoff required for the accepted reset-before-accept path, while its
cookie-check/delete is not shown atomic against later tuple reuse. STEP 11-06
remains paused. No implementation, test change, approval, roadmap mutation, or
commit is authorized by this iteration.

---

# Review iteration 5 — simplified bounded lifetime

## Iteration 5 metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CORRELATION-STEP11-06-ARCH-REVIEW-05` |
| Date | 2026-10-23 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | The simplified no-release-cleanup lifetime, its explicit capacity consequence, and readiness for the user decision |
| Technical verdict | **APPROVED FOR USER DECISION** |
| Formal verdict | **CHANGES_REQUESTED — USER_DECISION_REQUIRED** |
| Blocking technical findings | 0 critical, 0 high |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** |

The formal verdict follows the repository approval boundary: an independently
reviewed material design proposal remains `CHANGES_REQUESTED` until the user
explicitly approves its exact public contract. The technical proposal is ready
to be surfaced. This iteration does not approve the decision on the user's
behalf, fold it into the accepted contract, revalidate the roadmap, or authorize
implementation.

## Iteration 4 finding resolved

### Iteration-4 F-01 — release-before-passive deletion and non-atomic compare/delete

**Closed.** Release-time handoff cleanup is removed rather than synchronized:

- `sock_release` removes only the accepted application-socket origin fact and
  never either correlation handoff;
- active rows live until passive take, later same-full-tuple replacement, or
  owner teardown;
- passive rows live until userspace accept or owner teardown;
- immediate application close/reset cannot erase pre-passive metadata;
- no cookie-checked lookup/delete sequence exists, so no compare/delete race
  against tuple reuse remains; and
- no timer, audit, retry, cleanup map, LRU eviction, or additional state was
  added.

The feature delta, ADR-0150, handoff, and combined decision agree
(`feature-delta.md:953-1018`; ADR-0150:103-118;
`correlation.md:117-166,223-249`).

## Final technical assessment

### Identity and ordering

The approved-for-decision mechanism has one exact chain:

1. `Connect4` retains the original destination by application socket cookie.
2. Active-established publishes `{ application_cookie,
   original_destination }` by the complete client tuple after source-port
   allocation.
3. Passive-established takes that row while holding the accepted child's
   kernel socket lock and republishes it by that child's full kernel cookie.
4. `accept()` cannot return the child before that lock is released;
   `KernelGuestOwnerIo` reads the same cookie through `SO_COOKIE` and takes the
   child row once.

The full tuple preserves same-port/different-source connections. The child
cookie prevents reset-before-accept and later tuple reuse from aliasing an old
child to a new application identity. The accepted per-socket origin remains
until `sock_release`, so D15 `getpeername4` restoration is unchanged.

### Bounded state and capacity

The added state is exactly two pre-provisioned 65,536-row collections. No row
is evicted or guessed. A successful active row normally moves to passive and
then leaves at accept. An active row that never reaches passive remains charged
until the same tuple replaces it or the owner tears down.

The proposal does not hide the availability consequence: after 65,536 distinct
non-passive tuples remain, another connection needing an active row may be
reset even while fewer than `GUEST_TCP_PENDING_MAX` flows are held. The exact
combined decision says this explicitly (`correlation.md:223-249`). This is a
material new bound and therefore needs the user's approval; it is no longer an
unacknowledged contradiction or an implementation choice.

Capacity, missing-active, missing-passive, and direct-uncorrelated outcomes all
fail closed: reset that child, emit no `GuestIoEvent`, open no flow, and send no
control message. The state dies with `overdrive-init`; nothing survives the
guest boot or VM. The existing `ProgramLoad` / `ProgramAttach` representations
remain exact for pre-READY setup failure.

### Validation

The proposed DISTILL authorization is proportionate and stays in the existing
guest-kit lane. Its one parameterized Tier-3 scenario drives every
contract-shaping identity case through the real programs and production
adapter: ordinary success, same-port/different-source connections,
reset/release before accept plus admitted tuple reuse, direct uncorrelated
intake, and one-time child-cookie consumption. No seeded state, new lane,
model, benchmark, production test branch, or public test-support surface is
authorized.

The bounded 65,536-row exhaustion consequence is an explicitly approved-or-
rejected design outcome rather than a new state the test must manufacture.
Normal verifier/load gates still apply to both callbacks when DELIVER is
authorized; this review adds no separate lane or per-step mutation work.

## Architecture review dimensions

| Dimension | Iteration 5 assessment | Evidence |
|---|---|---|
| Architectural bias | Pass | One kernel program uses the two existing establishment callbacks; no userspace proxy, mark protocol, parking path, or recovery subsystem is added. |
| ADR quality | Pass | Context, decision, alternatives, consequences, bounds, failures, lifecycle, and pending authority are explicit and linked to the canonical feature contract. |
| Completeness | Pass | Identity, both callback orders, origin and handoff lifetimes, capacity, every fail-closed outcome, setup errors, READY ownership, teardown, and validation are pinned. |
| Feasibility | Pass | Linux 7.0 exposes the tuple and cookies, provides the callback/accept locking order, and the prior spike proves both callback types load and fire on the target substrate. |
| Priority/data justification | Pass | The sixth program closes the proven five-hook correlation gap without restoring removed payload machinery. |

## Roadmap checks

| Mandatory check | Iteration 5 result | Evidence |
|---|---|---|
| External validity | Pass | STEP 11-06 builds the additive guest half and STEP 12-01 invokes it through the production pre-READY path. |
| Acceptance-criterion coupling | Pass for the proposed reconciliation | The exact program order, error labels, and observable correlation outcomes are DESIGN-owned contracts. Private maps and helpers remain DELIVER-owned. |
| Step decomposition | Pass | Both callbacks and the accept adapter belong to the existing guest program/setup step. |
| Implementation code | Pass | The proposal specifies interface state and deltas, not method bodies or map implementation code. |
| Concision and precision | Pass | The simplified lifetime has one interpretation and names its availability cost. |
| Test boundary | Pass | The new callback is assigned production-adapter Tier-3 evidence after approval; neutral five-to-six assertions remain existing-test corrections. |

Keeping the current STEP 11-06 body unchanged and roadmap validation `pending`
before user approval is correct. After approval, the design/DISTILL owners must
fold the six-program and Tier-3 obligations into the roadmap and independently
revalidate it before DELIVER resumes.

## Exact user authority required

The combined decision in `.context/netns-density-295/11-06-design/correlation.md`
§ *Exact combined user decision after re-review* is technically ready to
surface as one numbered approve/reject decision. Its material clauses are:

- the sixth active/passive SockOps attachment before `Connect4` and READY;
- active full-tuple and passive child-cookie handoffs, each bounded at 65,536;
- accepted per-socket origin lifetime unchanged through `sock_release`;
- no release-time handoff cleanup;
- the explicit possibility of a reset below the owner flow quota after 65,536
  distinct non-passive rows remain;
- existing typed setup errors and the no-sockmap/no-parking/no-payload
  boundary; and
- the bounded DISTILL changes in the existing guest-kit lane.

Only an explicit user approval of that complete decision adds it to scope. If
approved, the architect must fold it into the current accepted feature
contract and ADR in present tense, record the approval, update the roadmap and
DISTILL counts/criterion, and submit the exact folded result and roadmap to
their required independent reviews. Silence or this technical verdict is not
approval.

## Iteration 5 verdict

**CHANGES_REQUESTED — USER_DECISION_REQUIRED.** No critical or high technical
finding remains. The simplified full-tuple → accepted-child-cookie mechanism
is coherent, feasible, bounded, fail-closed, preserves the accepted original-
peer contract, and has a production-driven validation plan. The proposal is
approved for presentation to the user, not accepted into the architecture.
Roadmap validation remains pending and STEP 11-06 remains paused. No
implementation, test change, roadmap-body edit, DES event, approval, or commit
is authorized by this iteration.

---

# Review iteration 6 — SYN-option / child-storage replacement

## Iteration 6 metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CORRELATION-STEP11-06-ARCH-REVIEW-06` |
| Date | 2026-10-23 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | The invariant-preserving replacement authorized by `TCP-CORRELATION-NO-WEAKEN`: 18-byte SYN option, passive child SK_STORAGE, exact setup errors, and the existing-lane Tier-3 plan |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 2 critical, 2 high |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** |

The user explicitly rejected the iteration-5 below-quota capacity relaxation
and authorized bounded DESIGN remediation that preserves admission below
`GUEST_TCP_PENDING_MAX`, connection identity, and reset-before-accept. This
iteration therefore reviews the SYN-option replacement against those accepted
outcomes; it does not treat the prior user-decision-ready verdict as authority
for a superseded mechanism.

The review read the new spike finding and standalone Aya sources, the v20
SK_STORAGE probe and raw map operations, Linux 7.0's SYN option allocator,
syncookie fallback, header-option helpers and SK_STORAGE operations, Aya
0.13.1 / aya-ebpf 0.1.1 map support, ADR-0159, and the revised canonical
contracts. It ran no code and changed no reviewed artifact.

## Findings

### F-01 — TCP Fast Open leaves insufficient SYN option space and causes a valid below-quota reset

- **Severity:** Critical.
- **Claim:** The exact 18-byte correlation option is not available on every
  kernel-valid captured TCP connect. Linux allocates ordinary SYN options,
  including TCP Fast Open, before the BPF header-option reservation. With the
  default timestamp/window-scale/MSS set, any Fast Open option leaves fewer
  than 18 bytes. `bpf_reserve_hdr_opt` then returns `-ENOSPC`; the proposal
  emits no option, passive establishment creates `Reject`, and an otherwise
  valid connection is reset below `GUEST_TCP_PENDING_MAX`.
- **Kernel path and arithmetic:** Linux gives a SYN at most 40 option bytes
  (`include/net/tcp.h:73`). With the normal configuration it consumes 4 bytes
  MSS, 12 bytes timestamp, and 4 bytes window scale before optional features
  (`net/ipv4/tcp_output.c:981-1008`), leaving 20. Fast Open is processed before
  BPF and consumes at least its 2-byte base rounded to 4 bytes, and more when a
  cookie is present (`:1010-1023`; `include/net/tcp.h:238-240`). Only then does
  Linux call `bpf_skops_hdr_opt_len` (`tcp_output.c:1025-1054`). The reserve
  helper returns `-ENOSPC` when requested length exceeds the remaining bytes
  (`net/core/filter.c:7933-7947`). Thus even the no-cookie Fast Open request
  leaves 16 bytes, less than the proposed 18.
- **Reachable producer evidence:** TCP Fast Open is an ordinary socket path,
  driven with `MSG_FASTOPEN` in the repository's v20 native probe
  (`spike-scratch/netns-density-295-v20/probe/src/main.rs:206-225,864-887`);
  it produced 1,001/1,001 tagged accepted children on Linux 7.0
  (`spike/findings-v20.md:24-25`). The new option spike explicitly limits its
  verdict to default ordinary SYN options and does not exercise Fast Open
  (`spike/findings-tcp-correlation-option.md:107-122`).
- **Contract violation:** The canonical proposal converts reserve/store
  failure into `Reject` and reset (`feature-delta.md:978-989`) while its
  normative audit claims correlation adds no admission bound below
  `GUEST_TCP_PENDING_MAX` (`:972-976,1004-1013`). Under the user's
  `TCP-CORRELATION-NO-WEAKEN` ruling, those clauses cannot both stand. Fast
  Open is not in the feature's deferred list and D15 preserves kernel-allowed
  ordinary socket behavior.
- **Required revision:** Rework the exact carrier/option-space contract so a
  kernel-valid captured connect below quota does not become a correlation
  refusal. Any claimed solution must be supported by Linux 7.0 source and a
  bounded real-kernel producer case; do not silently disable Fast Open or add
  a new workload restriction. This review does not select a replacement ABI or
  authorize another mechanism.
- **Disposition:** Open. The no-weakening audit is falsified.

### F-02 — Syncookie children have no saved SYN, and the proposal does not replay the option in the final ACK

- **Severity:** Critical.
- **Claim:** `TCP_SAVE_SYN` does not make the proposed correlation available
  when the intake listener uses syncookies. In that path passive-established
  has no saved SYN. Linux's own header-option selftest handles this by asking
  the active side to repeat the option in the final ACK; the proposed program
  writes only the outgoing SYN. It therefore produces `Reject` and resets a
  valid connection under SYN-backlog pressure, before the owner flow quota is
  the deciding bound.
- **Kernel evidence:** Linux's selftest handles
  `bpf_load_hdr_opt(..., BPF_LOAD_HDR_OPT_TCP_SYN) == -ENOENT` by loading the
  option from the just-received final ACK, explicitly because “saved_syn is not
  found” in syncookie mode
  (`tools/testing/selftests/bpf/progs/test_tcp_hdr_options.c:462-478`). The
  proposed state machine arms/writes the option only for the outgoing SYN and
  treats absent saved option as `Reject` (`feature-delta.md:930-945,978-989`).
- **Reachable producer evidence:** The repository's v20 real-kernel probe
  forced the Linux syncookie path and observed `SyncookiesRecv +2000` with
  2,000/2,000 accepted children (`spike/findings-v20.md:24,29-34`;
  `spike-scratch/netns-density-295-v20/probe/src/main.rs:840-862`). In
  production `tcp_syncookies=1` reaches the same path when the SYN backlog is
  pressured; pending-flow quota is applied only after userspace accepts and
  classifies the child, so it does not prevent this kernel path.
- **Contract violation:** A syncookie connection can be admitted by TCP yet
  reset solely because correlation metadata is absent. This contradicts the
  explicit user ruling and the proposal's statements that correlation adds no
  below-quota admission bound and preserves reset-before-accept identity.
- **Required revision:** Pin and validate a correlation path that survives the
  Linux 7.0 syncookie handshake without guessing or weakening admission. Add
  no raw-packet-only production seam; use a real kernel producer. If the
  replacement requires another observable protocol clause, surface it for
  review and user approval before changing DISTILL or DELIVER.
- **Disposition:** Open.

### F-03 — Callback arming is assigned to `Connect4`, which cannot call the SockOps flag helper

- **Severity:** High.
- **Claim:** The normative transition says `Connect4` records the origin and
  arms SockOps header callbacks. A `cgroup/connect4` program receives
  `SockAddrContext`; Linux exposes `bpf_sock_ops_cb_flags_set` only to
  SockOps programs. The executed spike correctly arms in
  `BPF_SOCK_OPS_TCP_CONNECT_CB`, so the proposal's exact owner is wrong.
- **Evidence:** `feature-delta.md:930-932,978-983` and
  `correlation.md:140-146` assign arming to `Connect4`. Aya's
  `SockAddrContext` has no callback-flag operation, while `SockOpsContext`
  exposes `set_cb_flags`
  (`aya-ebpf-0.1.1/src/programs/{sock_addr,sock_ops}.rs`). Linux returns
  `bpf_sock_ops_cb_flags_set` only from `sock_ops_func_proto`
  (`net/core/filter.c:8555-8597`). The new spike performs the feasible sequence:
  `BPF_SOCK_OPS_TCP_CONNECT_CB` looks up the origin and calls
  `ctx.set_cb_flags(...)`
  (`spike-scratch/netns-density-295-tcp-correlation-option/bpf/src/main.rs:143-148`);
  Linux invokes that callback at `tcp_output.c:4302`.
- **Required revision:** Assign the option-arm transition to the sixth
  program's `TCP_CONNECT_CB` after `Connect4` has recorded the origin. Reconcile
  the feature delta, ADR summary, handoff, combined decision, and Tier-3
  oracle. This changes no public API and adds no mechanism; it makes the
  existing proposal implementable.
- **Disposition:** Open.

### F-04 — SK_STORAGE provisioning is not reconciled with the accepted unpinned, process-owned lifecycle

- **Severity:** High.
- **Claim:** Aya 0.13.1 / aya-ebpf 0.1.1 cannot directly create the required
  BTF-backed SK_STORAGE map from the object. The cited v20 precedent precreates
  and pins the map, then Aya reuses the pin. The proposal maps create/pin/reuse
  failures but never states whether the pin is temporary or removed. A retained
  bpffs pin contradicts D19's accepted rule that every guest forwarding object
  is unpinned and process-owned, and contradicts the proposed teardown claim.
- **Executed evidence:** V20 finding F-2 records that SK_STORAGE requires BTF,
  Aya 0.13.1 refuses map type 24 without `allow_unsupported_maps`, and legacy
  creation lacks BTF; its working path performs raw BTF/map creation, pins the
  map, and lets the object reuse it
  (`spike/findings-v20.md:39-40`; `spike-scratch/netns-density-295-v20/ebpf/src/main.rs:20-46`;
  `probe/src/main.rs:60-114`). The replacement relies on that precedent and
  explicitly includes create/pin/reuse failures in `ProgramLoad`
  (`feature-delta.md:1028-1037`; ADR-0150:113-120).
- **Accepted lifecycle:** ADR-0159 requires every forwarding object to live
  exactly as long as its owner process, with “no bpffs pin” and no boot
  adoption (`adr-0159:44-56,64-71`). The pending proposal says teardown drops
  the link and every socket storage value, but does not own or remove the map
  pin (`feature-delta.md:993-997`).
- **Required revision:** Pin the exact SK_STORAGE map-object lifecycle at the
  interface/invariant level: any load-time pin used for Aya reuse must not
  survive into the running READY state or owner teardown, and its failure must
  retain the proposed typed `ProgramLoad` representation. Name the bounded
  compiler/caller fallout needed to create/reuse and release the map. Do not
  add runtime adoption, audit, recovery, or a persistent pin.
- **Disposition:** Open.

## Error-contract and fallout assessment

The proposed setup error reuse is otherwise type-complete:

| Designed failure | Proposed representation | Review assessment |
|---|---|---|
| SockOps object/program or SK_STORAGE create/reuse/ELF-map load | `ProgramLoad { program: TcpCorrelation, source: EbpfError }`, with map failures preserved as `EbpfError::MapError` | Sound, subject to F-04 pin lifecycle. |
| Root-cgroup SockOps attach | `ProgramAttach { program: TcpCorrelation, source: ProgramError }` | Sound. |
| `TCP_SAVE_SYN` setup in the existing intake step | `IntakeBind { addr, source: io::Error }` | Semantically acceptable at the existing listener-setup boundary; all such failures exit before READY. |
| Runtime option/storage absence or allocation failure | Per-connection reset, no event/flow/control message | Typed setup error not required; however ordinary Fast Open and syncookie absence are forbidden weakening, not acceptable “failure” cases (F-01/F-02). |

The bounded production fallout correctly identifies the sixth enum variant,
load/attach dispatch, callback logic, intake option, private option ABI,
SK_STORAGE, and `KernelGuestOwnerIo`. F-03 and F-04 must be reflected in that
fallout before it is executable. No host wire, persisted schema, owner-event
variant, flow lifecycle, model variable, new lane, or public test-support API
is justified.

## Architecture and roadmap checks

| Dimension | Assessment | Evidence |
|---|---|---|
| Architectural bias | Pass | A guest-local kernel handshake carrier is narrower than global correlation/cleanup state and adds no payload path. |
| ADR quality | Fail | The no-weakening consequence is contradicted by Fast Open and syncookie paths; map ownership is incomplete. |
| Completeness | Fail | Default-SYN success is proven, but two kernel-valid handshake classes and the map pin lifecycle are unresolved. |
| Feasibility | Partial | Default SYN option write/load and SK_STORAGE separately work. The composed production mechanism needs correct callback ownership and lifecycle. |
| Priority/data justification | Pass | The mechanism targets the proven correlation gap and removes the rejected abandoned-row capacity. |

| Mandatory roadmap check | Result | Evidence |
|---|---|---|
| External validity | Pass | The complete roadmap still invokes guest setup at STEP 12-01. |
| Acceptance-criterion coupling | Pending | The final invariant-preserving handshake and map lifecycle must be approved before STEP 11-06 is reconciled. |
| Step decomposition | Pass | The sixth program, listener setup, storage, and adapter belong to STEP 11-06. |
| Implementation code | Pass | The option ABI and state outcomes are DESIGN-owned; private syscall wrappers remain DELIVER-owned. |
| Concision and precision | Fail | The no-weakening audit says no below-quota refusal while reserve/saved-SYN failures produce one. |
| Test boundary | Fail | The proposed Tier-3 cell omits Fast Open and syncookie real-producer cases that falsify the current contract. |

Keeping the roadmap body unchanged and validation `pending` is correct. No
roadmap update, DISTILL authoring, or DELIVER resumption is authorized from this
proposal.

## Normative no-weakening audit

**FAIL.** The replacement preserves per-socket origin lifetime, removes global
correlation capacity, and keeps missing/malformed metadata fail-closed. It
nevertheless adds two new below-quota resets for connections Linux accepts:
insufficient SYN option space on Fast Open and absent saved SYN in syncookie
mode. The user's exact `TCP-CORRELATION-NO-WEAKEN` ruling rejects those
outcomes. Generic approval of “reserve failure resets” or “missing option
resets” cannot waive them; any weakening would require a new, concrete user
decision quoting the exact behavior, and the current user direction requires
the invariant-preserving alternative instead.

## Iteration 6 verdict

**CHANGES_REQUESTED.** The SYN-option replacement is promising and its default
Aya/Linux 7.0 spike is valid evidence, but it is not the approved
invariant-preserving design yet. Linux source plus existing executed probes show
that Fast Open exhausts the 18-byte reservation and syncookies remove the saved
SYN, producing valid below-quota resets. The canonical transition also assigns
callback arming to the wrong program, and the SK_STORAGE pin lifecycle is not
reconciled with D19. Roadmap validation remains pending and STEP 11-06 remains
paused. No production code, test body, roadmap body, DES state, user approval,
or commit is authorized by this review.

---

# Review iteration 7 — final-ACK / ordinary-map replacement

## Iteration 7 metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CORRELATION-STEP11-06-ARCH-REVIEW-07` |
| Date | 2026-10-23 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | The replacement final-ACK carrier, ordinary active/child hash maps, Fast Open `Pending -> Ready`, combined pending count, setup errors and bounded DISTILL fallout |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 1 critical |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** |

This iteration reviewed the replacement recorded in the pending
`TCP-CORRELATION` sections of the feature delta and ADR-0150, the exact handoff
in `.context/netns-density-295/11-06-design/correlation.md`, the rewritten
finding and standalone Aya source under
`spike-scratch/netns-density-295-tcp-correlation-option/`, Linux 7.0 source,
the current crate-private setup/owner seams, and the still-pending roadmap.
The review also ran two bounded, source-independent socket checks in the
repository Lima VM. It changed no reviewed design, production source, test,
roadmap body, DES state or kernel setting.

## Iteration 6 remediation dispositions

| Prior finding | Replacement disposition | Iteration 7 assessment |
|---|---|---|
| F-01 — the 18-byte SYN option does not fit with Fast Open | The option is now absent from every SYN and is written on the handshake-completing non-SYN ACK; the rewritten spike drove an actual data-in-SYN Fast Open child through `Pending -> Ready` and observed exact data | **Closed.** The carrier no longer competes with Fast Open's SYN options. |
| F-02 — syncookie children have no saved SYN | Ordinary and syncookie passive establishment load the current final ACK; the run observed `SyncookiesRecv +1` and exact metadata | **Closed.** This is the Linux selftest's supported current-packet path. |
| F-03 — `Connect4` cannot arm SockOps callbacks | `Connect4` only retains origin and rewrites; `BPF_SOCK_OPS_TCP_CONNECT_CB`, after port allocation, copies the origin and arms write/parse/state callbacks | **Closed.** The canonical owner and the executed Aya source now agree. |
| F-04 — SK_STORAGE required an unreconciled bpffs pin | SK_STORAGE is gone. Both transients are ordinary preallocated Aya hash maps, unpinned and owned by `overdrive-init` | **Closed.** D19 teardown and pre-READY ownership are coherent. |

## Finding

### F-01 — 65,536 is not the complete active-cookie universe, so the no-earlier-bound proof is invalid

- **Severity:** Critical.
- **Claim:** `ACTIVE_CORRELATION` is keyed by application socket cookie, but
  the proposed capacity is justified as “the complete 16-bit local-port
  universe for the fixed workload-address/intake pair.” The source address is
  not fixed by that path. An ordinary application may bind different local
  IPv4 addresses before `connect()`. `Connect4` rewrites only the destination,
  and Linux permits the same local port on those different source addresses
  for the same local intake destination. Consequently one local-port universe
  does not bound the number of live application-cookie rows.
- **Canonical conflict:** The feature delta fixes exactly 65,536 active rows
  and calls that the complete universe, then concludes fixed-capacity
  exhaustion is unreachable below the accepted quota
  (`feature-delta.md:1003-1015,1023-1027`). The handoff repeats the premise and
  says the active capacity cannot become an earlier bound
  (`correlation.md:137-148,195-207`). The exact user decision includes that
  size (`correlation.md:251-263`). Those are interface/invariant claims, not a
  private implementation choice.
- **Production path:** `Connect4` retains the original destination by socket
  cookie and changes `user_ip4`/`user_port`; it does not rewrite the local
  source address. Linux bind stores the application's chosen local address in
  `inet_rcv_saddr` (`net/ipv4/af_inet.c:501-513,522-563 @ v7.0`). TCP
  established uniqueness includes both local address and local port
  (`net/ipv4/inet_hashtables.c:575-603`), and connect-time port selection relies
  on that established check rather than a port-only bound (`:1093-1126`). Each
  such socket reaches `TCP_CONNECT_CB` after port allocation and therefore
  requests a distinct cookie row before any passive confirmation.
- **Executed falsification of the capacity premise:** On the same repository
  Lima VM (`7.0.0-38-generic`), a bounded ordinary-socket check bound two
  clients to `127.0.0.2:45232` and `127.0.0.3:45232` and connected both to the
  same non-loopback local destination `192.168.5.15:44491`. Both connections
  succeeded simultaneously and retained the identical source port:

  ```text
  dst=('192.168.5.15', 44491)
  sources=[('127.0.0.2', 45232), ('127.0.0.3', 45232)]
  same_port=True
  ```

  This is the post-rewrite tuple shape claimed to be single-address. The guest
  contract retains `lo` as well as the dummy workload address, and D15 preserves
  ordinary kernel-allowed bind behavior. The result directly disproves the
  statement that 65,536 local-port values exhaust the possible active cookie
  keys; the accepted guest exposes many local loopback source addresses.
- **Cleanup does not supply the missing bound:** Passive confirmation and
  exact-socket `TCP_CLOSE` prevent sequential dead-row accumulation, which is a
  real improvement over iteration 5. They do not bound simultaneous live rows.
  `TCP_CONNECT_CB` publishes before the SYN. Listener/accept pressure can keep
  an application socket without a passive confirmation until a later retry or
  its close; Linux checks accept-queue fullness before creating the passive
  child (`net/ipv4/tcp_ipv4.c:1722-1724`) and the default overflow path can
  leave the connection attempt to retry rather than creating that child
  (`net/ipv4/tcp_minisocks.c:947-955`). Such a socket retains its active row by
  the proposed lifecycle. No accepted contract caps these pre-passive sockets
  at 65,536 across all legal source addresses.
- **Invariant impact:** When the active map is full, the next
  `TCP_CONNECT_CB` cannot publish/arm the metadata. The rewritten connection
  still targets the intake, but its passive side cannot receive the required
  option and fails closed. The proposed proof that correlation can never cause
  an admission failure while the owner has room therefore does not follow from
  the stated capacity. The final 65,537-row exhaustion was not executed; the
  exact bounded gap is that the design supplies neither a valid kernel bound
  nor real-producer evidence for its normative 65,536 maximum.
- **Required revision:** Do not enlarge the permanent test surface or invent a
  public API. Rework or prove the active transient so its capacity cannot
  become a correlation-only refusal below `GUEST_TCP_PENDING_MAX`. If retaining
  a fixed map, identify the real production quantity that bounds every live
  row and reproduce that bound through the captured multi-source/backlog path;
  local-port cardinality alone is not that quantity. Reconcile the feature
  delta, ADR, handoff, combined decision and no-weakening audit. This review
  does not select another map type, cleanup protocol or carrier.
- **Disposition:** Open. The explicit `TCP-CORRELATION-NO-WEAKEN` proof remains
  incomplete.

## Capacity, lifecycle and identity assessment

| Concern | Assessment | Evidence |
|---|---|---|
| Correlation necessity | Pass | The accepted child still lacks both the pre-rewrite destination and application cookie under the five accepted hooks. The sixth program remains necessary on the proven path. |
| Callback ownership | Pass | `Connect4` retains/rewrites; `TCP_CONNECT_CB` runs after allocation and owns callback arming. The executed source matches Linux `tcp_connect`. |
| Ordinary and syncookie identity | Pass | The exact cookie and destination travel together on the current final ACK and materialize under the accepted child cookie. The native run observed both ordinary and forced-syncookie success. |
| Fast Open identity | Pass | No SYN option is added. Passive establishment creates `Pending`; the final-ACK parse callback creates `Ready`. The native run observed `TCPFastOpenPassive +1`, the exact metadata and the exact SYN data. |
| Immediate release / reset-before-accept | Pass | Application `sock_release` removes the accepted origin but not the active retransmission copy. Passive processing is self-contained in the ACK, and the native immediate-close case retained exact metadata. |
| Active cleanup | Partial | Passive consumption and exact active `TCP_CLOSE` are coherent and the spike observed both. F-01 shows that exact cleanup does not establish the claimed maximum number of simultaneously live rows. |
| Child capacity | Pass | `listen(4096)` admits at most 4,097 queued children under Linux's `>` fullness check; 4,096 adapter-held Fast Open `Pending` children plus one classification boundary gives 8,194. Ready rows are taken at classification and exact child release is the cleanup complement. |
| Combined pending quota | Pass | The crate-private adapter count starts at accept and ends only at successful attach or consuming abort/close; it includes internally held `Pending` children and emitted `CapturedConn` handles. This preserves one 4,096-place pre-pairing total without a new trait, event or model variable. |
| Pending wake and audit | Pass | A private best-effort wake affects latency only. Scans before blocking and after every poll wake plus the already-approved fixed-rate one-second audit provide the bounded fallback without a new timer or host event. |
| Teardown | Pass | Ordinary maps and the sixth link are unpinned and die with `overdrive-init`; no state, pin or adoption protocol survives the VM/owner. |

## Typed error and contract-surface assessment

| Designed failure or change | Exact proposed representation | Assessment |
|---|---|---|
| SockOps object/program, active/child/wake map create or ELF-map load | `GuestTransportSetupError::ProgramLoad { program: GuestProgram::TcpCorrelation, source }`; map failures remain `aya::EbpfError::MapError(..)` | Pass. Aya 0.13.1 has the exact `EbpfError::MapError` carrier and no new error variant is needed. |
| Root-cgroup attachment | Existing `ProgramAttach { program: GuestProgram::TcpCorrelation, source }` | Pass. |
| `listen(4096)` or `TCP_FASTOPEN(4096)` | Existing `IntakeBind { addr, source }` | Pass at the existing intake-setup boundary. |
| Missing/malformed runtime metadata | Reset/drop that connection; no event, flow or control record | Pass as a runtime fail-closed outcome, subject to F-01: capacity exhaustion cannot be relabelled as malformed input. |
| Program set | Add crate-private `GuestProgram::TcpCorrelation`, label `tcp_correlation`, `GuestProgram::ALL` entry and pre-READY load/attach dispatch | Complete and explicitly user-decision-requiring. It changes the exact guest setup contract even though no language-level `pub`, cross-crate, wire or persisted type changes. |
| Owner adapter | Private pending-child table, combined count, one-time child-map take and wait/audit scans | Complete private HOW under the existing `GuestOwnerIo` and `GuestIoEvent` contracts. |

The bounded compiler fallout is otherwise complete: the enum and exhaustive
matches, setup call counts/order, production Aya dispatch, intake backlog/TFO
setup and crate-private adapter state are named. The proposal does not require a
host protocol, persisted schema, owner-event variant, flow lifecycle, model
state, new lane or public test-support surface.

## DISTILL and roadmap assessment

The proposed neutral five-to-six corrections are ordinary compiler and
existing-assertion fallout. One new parameterized Tier-3 cell in the existing
disposable guest-kit owner lane is still the correct minimum for the observable
new mechanism: production `Connect4` plus final-ACK callbacks plus
`KernelGuestOwnerIo`, covering ordinary one-time take, immediate release,
forced syncookie, actual Fast Open `Pending -> Ready`, direct uncorrelated
intake, and transient cleanup. No second lane, model, raw-packet fixture,
seeded private state or public fixture API is justified by this review.

F-01 is a DESIGN/evidence correction before that DISTILL authorization becomes
effective. A bounded spike may prove the revised active bound; it does not by
itself authorize an additional permanent test cell. The roadmap body correctly
remains unreconciled and its validation remains `pending`; STEP 11-06 is not
executable.

## Normative no-weakening audit

The replacement closes the four iteration-6 weakenings. It keeps D15's
per-socket origin through `sock_release`, uses the final ACK for ordinary,
syncookie and Fast Open paths, keeps early bytes only in the intake receive
queue, applies one combined pending count, resets missing identity, and uses
process-owned teardown.

The audit nevertheless **fails on F-01**. “Exactly 65,536” is based on a
single-source premise that Linux 7.0 and the accepted guest address model
falsify. Exact cleanup proves absence of dead sequential accumulation; it does
not prove that every simultaneously live cookie fits. The review cannot accept
the claim that correlation introduces no earlier refusal until the active
universe is bounded by a production fact or the mechanism removes that
capacity consequence. The user has not approved a relaxation.

## User authority and iteration 7 verdict

No exact user decision is ready to present from this revision. The sixth
`GuestProgram` variant, final-ACK option ABI, transient state, pending-child
adapter behavior and test authorization all remain proposed public/design
contract changes. The existing `TCP-CORRELATION-NO-WEAKEN` ruling authorizes
DESIGN remediation but does not approve this exact replacement.

**CHANGES_REQUESTED.** The final-ACK carrier is technically supported for the
ordinary, syncookie and data-in-SYN Fast Open cases, and the child lifecycle,
combined quota, typed errors and bounded caller fallout are coherent. The
active-map capacity proof is not: ordinary Linux sockets can share one local
port across multiple legal source addresses, so 65,536 is not the complete
application-cookie universe. The canonical design must close that one bounded
gap before independent approval or a user decision. Roadmap validation remains
pending and STEP 11-06 remains paused. No implementation, test-body change,
roadmap-body edit, DES event, user approval or commit is authorized by this
iteration.

---

# Review iteration 8 — active socket-storage replacement

## Iteration 8 metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CORRELATION-STEP11-06-ARCH-REVIEW-08` |
| Date | 2026-10-23 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | Capacity-free active socket storage, raw-BTF/Aya staging lifecycle, final-ACK retransmission, the unchanged child handoff/quota, typed failures and minimum validation |
| Technical verdict | **APPROVED FOR EXPLICIT USER DECISION** |
| Workflow verdict | **CHANGES_REQUESTED — USER_DECISION_REQUIRED** |
| Blocking technical findings | 0 |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** |

This iteration reviewed the replacement recorded in the pending
`TCP-CORRELATION` sections of the feature delta and ADR-0150, the exact handoff
in `.context/netns-density-295/11-06-design/correlation.md`, the rewritten
finding and standalone Aya source under
`spike-scratch/netns-density-295-tcp-correlation-option/`, Linux 7.0 socket
storage/destruction and TCP callback paths, Aya 0.13.1's map/program lifecycle,
ADR-0159, the existing crate-private setup and owner seams, and the pending
roadmap. It ran no additional kernel experiment and changed no reviewed design,
production source, test, roadmap body, DES state or kernel setting.

## Iteration 7 remediation disposition

### F-01 — fixed-capacity active-cookie universe

**Closed.** `ACTIVE_CORRELATION` is no longer a cookie-keyed hash map and no
local-address/port cardinality is used. It is `BPF_MAP_TYPE_SK_STORAGE` on the
exact active socket with `max_entries = 0`. Linux requires zero
`max_entries` for local-storage maps
(`kernel/bpf/bpf_local_storage.c:788-801 @ v7.0`), allocates a value against
that socket when `bpf_sk_storage_get(..., CREATE)` succeeds, and removes all of
the socket's storage in `__sk_destruct`
(`net/core/bpf_sk_storage.c:46-61`; `net/core/sock.c:2343-2363`). There is no
global active-row limit, alias, eviction choice or dead-row class. General
kernel memory failure remains a per-operation fail-closed kernel failure; it
is not the proposal's earlier fixed admission bound.

The rewritten probe used the actual storage map and helper calls on Linux
`7.0.0-38-generic`. Four established cases deleted storage after later peer
input, and the refused connection observed its storage at `TCP_CLOSE`
immediately before socket-lifetime destruction. This directly exercises both
declared cleanup complements. The iteration-7 multi-source-port falsification
therefore does not apply to the replacement.

## Active-storage state and retransmission review

| Transition or invariant | Assessment | Evidence |
|---|---|---|
| Create on the exact active socket | Pass | `TCP_CONNECT_CB` runs after local-port allocation. The probe copies the existing per-socket origin into `bpf_sk_storage_get(..., BPF_SK_STORAGE_GET_F_CREATE)` and rolls the value back if callback arming fails. |
| Emit phase | Pass | Active establishment changes only that socket's storage from armed to emit before Linux builds the final ACK. Header-length and write callbacks read the same socket-local value and write the exact 18-byte option only on non-SYN ACKs. |
| Lost final ACK / retransmission | Pass | A lost final ACK causes another SYN-ACK. The deletion predicate excludes every SYN packet, so the duplicate SYN-ACK cannot erase storage; the responding non-SYN ACK still carries the exact option. Application file release removes the origin map fact but storage remains until the orphaned TCP socket is destroyed. |
| First later peer packet | Pass on the production path | Ordinary/syncookie passive sockets cannot produce a post-handshake non-SYN response until the final ACK established the child. A Fast Open child exists from the SYN, but `KernelGuestOwnerIo` holds it internally while `Pending` and writes no payload; its SYN-ACK and retransmissions retain SYN. The first later non-SYN packet is therefore after the final-ACK opportunity the contract needs. |
| Immediate application close | Pass | The executed ordinary, syncookie and Fast Open cases dropped the application fd before userspace accept and still recovered exact metadata. Linux frees socket storage at socket destruction, not at file release. |
| Explicit/automatic deletion | Pass | The active parse callback deletes on the first later non-SYN peer packet. If no such packet exists, `__sk_destruct` destroys the value. The refused-connect state callback observed the latter lifetime in the run. |
| Capacity and identity | Pass | Each value is owned by one `struct sock`; `max_entries = 0` supplies no global row ceiling. Cookie and destination remain in one immutable value and one emitted option, so another address, port, tuple or socket cannot alias them. |

The phase field is necessary state, not speculative machinery: without it a
SYN-ACK parse could delete the value before active establishment, while the
contract must retain it until the final-ACK writer is eligible. The exact
armed/emit/delete/destruct state machine is bounded to the reproduced
correlation path.

## Aya and D19 lifecycle review

The executed loader establishes the required Aya 0.13.1 sequence:

1. create minimal BTF and the `BPF_MAP_TYPE_SK_STORAGE` fd with the exact
   32-bit key BTF, 24-byte value BTF, `max_entries = 0` and
   `BPF_F_NO_PREALLOC`;
2. pin that fd under a process-private, unique bpffs staging directory;
3. call `EbpfLoader::allow_unsupported_maps().map_pin_path(..)` so
   `load_file` reuses the map and the resulting `Ebpf` owns its fd;
4. unlink the map pin and remove the directory;
5. drop the raw creation/BTF fds, then call `SockOps::load()` and attach the
   resulting BPF link; and
6. retain only the runtime fds held by `overdrive-init`, with no pin or
   adoption path during service or teardown.

The spike performs the unlink at
`probe/src/main.rs:346-365`, before `SockOps::load()` and attachment, and then
runs the complete ordinary/syncookie/Fast-Open/direct/refused matrix. This is
stronger than separate feasibility probes: the same `Ebpf` instance that
reused the temporary pin loads and attaches the program and supplies the maps
for the successful run.

The operational D19 outcome is preserved. No attached program, runtime map or
link is pinned; no object can be adopted by a later owner; all runtime objects
remain fd-owned; and PID-1 failure before READY ends the VM. A normal returned
failure closes the raw/Aya fds and removes staging residue before returning.
If unlink itself fails, setup returns the declared fatal `ProgramLoad`, READY is
not emitted, and PID-1 failure ends the VM rather than allowing another owner
to adopt the residue. No runtime audit or recovery protocol is introduced.

There is one explicit authority consequence. ADR-0159 currently says “no
bpffs pin” without a setup qualifier. This proposal permits one private
pre-program-load staging pin. Its service/ownership invariant is equivalent,
but the literal accepted contract changes from no pin to a bounded temporary
pin. The combined decision names the exact staging sequence and prohibition on
runtime pin/adoption, so it is technically complete for an explicit user
decision. It is not already authorized by `TCP-CORRELATION-NO-WEAKEN`. The
approval request must identify it as a bounded amendment to ADR-0159, and an
approval must be folded into ADR-0159 as well as ADR-0150 and the feature
delta. Calling the change merely “private HOW” would be inaccurate.

## Child handoff, quota and lifecycle review

| Concern | Assessment | Evidence |
|---|---|---|
| Ordinary and syncookie child identity | Pass | Passive establishment loads the current final ACK and creates `Ready` under the exact child cookie. The run observed exact ordinary metadata and `SyncookiesRecv +1`. |
| Fast Open child identity | Pass | Passive establishment on the data-bearing SYN creates `Pending`; the final-ACK parse callback creates `Ready`. The run observed `TCPFastOpenPassive +1`, exact metadata and exact SYN data. |
| Child capacity | Pass | `listen(4096)` permits at most 4,097 queued children under Linux's `>` fullness check. At most 4,096 accepted Fast Open children are held internally as `Pending`; the one classification boundary gives exactly 8,194. |
| Combined pending quota | Pass | The adapter count starts at every accept and ends only at successful `attach_outbound` or consuming abort/close. It covers internally held `Pending` children and emitted `CapturedConn` handles without summing two independent quotas. |
| One-time classification | Pass | `Ready` is lookup-and-deleted once. Missing, malformed, ended and second-consume state emits no event, flow or control record. Exact child `sock_release` is the destruction cleanup complement. |
| Wake loss | Pass | The private wake is only a latency hint. Pending rows are scanned before blocking and after every poll wake; the existing fixed-rate one-second owner audit is the idle fallback. No new timer, event variant or recovery model is needed. |
| Early bytes | Pass | The correlation callbacks move metadata only. Fast Open and ordinary bytes remain solely in the intake child's receive queue until EVMOD-PAIR attaches the pair. |

No concrete production path weakens reset-before-accept or admits a guessed
identity. The final-ACK carrier remains valid under the stock guest's option
space, forced syncookies and actual data-in-SYN Fast Open. The proposal neither
restores sockmap parking nor changes host wire, persistence or flow lifecycle.

## Typed error and bounded-fallout assessment

| Designed condition | Exact representation | Assessment |
|---|---|---|
| Raw BTF/map creation, staging directory/pin, Aya reuse or unlink; child/wake map creation or ELF-map load | `GuestTransportSetupError::ProgramLoad { program: GuestProgram::TcpCorrelation, source: aya::EbpfError::MapError(..) }` | Pass. Aya's `MapError` can retain syscall, pin and `io::Error` causes. Callers branch only on fatal setup stage/program, so no missing semantic distinction is hidden in a string. |
| SockOps program load | Existing `ProgramLoad { program: TcpCorrelation, source: EbpfError::ProgramError(..) }` | Pass. |
| Root-cgroup BPF-link attach | Existing `ProgramAttach { program: TcpCorrelation, source: ProgramError }` | Pass. Linux 7.0/Aya use `bpf_link_create`, satisfying D19's attachment ownership. |
| `listen(4096)` or `TCP_FASTOPEN(4096)` | Existing `IntakeBind { addr, source }` | Pass at the existing intake-setup boundary. |
| Missing/malformed runtime metadata or allocation failure for one socket | Reset/drop that connection; no owner event, flow or control record | Pass. It is a per-connection fail-closed outcome, not a new owner-fatal error. |

The compiler and caller fallout is complete and bounded: the design-owned
crate-private `GuestProgram` variant/`ALL` order/string label, the existing
load/attach dispatch, the shared guest Aya object's private ABI/map/loader
state, intake TFO/backlog setup, and private `KernelGuestOwnerIo` pending table
and scans. No language-level `pub` or cross-crate type, trait method, owner
event, host protocol, persisted schema, model variable or public test-support
API is required.

## Minimum validation and roadmap assessment

The proposed validation remains the minimum honest production evidence:

- neutral corrections update the existing program list, label, setup order and
  existing load/attach error cases from five to six;
- one parameterized Tier-3 cell in the existing disposable guest-kit owner lane
  composes production `Connect4`, the final-ACK callbacks and
  `KernelGuestOwnerIo` for ordinary one-time take, immediate release, forced
  syncookie, actual data-in-SYN Fast Open `Pending -> Ready`, direct
  uncorrelated intake and transient cleanup; and
- its transient-cleanup oracle includes the active-storage delete/destruct
  complement, child-map return to baseline, and absence of the staging pin and
  directory during service/after READY. This stays inside the same cell and
  needs no stable public path or test-only production branch; DISTILL owns the
  source-local inspection.

The standalone Lima spike is sufficient DESIGN feasibility evidence for the
loader order and callback mechanism. The approved Tier-3 cell, if authorized,
is still required because it replaces userspace origin seeding and direct map
inspection with the production `Connect4` and owner adapter on the supported
guest-kit lane. No second cell, new lane, raw-packet fixture, seeded private
state, Sim variable, benchmark or KASAN run is justified.

The roadmap body correctly remains unchanged and validation remains `pending`.
After exact user approval, DESIGN and DISTILL must fold this contract and the
one-cell criterion into STEP 11-06, then submit the roadmap to its independent
review. STEP 11-06 is not executable before that sequence completes.

## Normative no-weakening audit

**Pass for product outcomes; explicit contract amendment pending authority.**
The capacity-limited active collection is gone. Active state belongs to and
dies with one socket; child capacity covers the complete queued/held universe;
the same 4,096 pending bound is enforced across internal and emitted handles;
ordinary, syncookie and Fast Open retain exact cookie/destination identity;
immediate release preserves retransmission metadata; missing metadata never
opens a flow; and early bytes remain in the child receive queue.

The private staging pin does not weaken service ownership, fail-closed teardown
or no-adoption outcomes, but it is a literal exception to ADR-0159's accepted
“no bpffs pin” wording. It therefore requires the exact explicit approval named
above. This review does not treat operational equivalence as authority to
silently rewrite that `no`.

## Exact user authority required

The combined decision in `.context/netns-density-295/11-06-design/correlation.md`
§ *Exact combined user decision after re-review* is technically ready to be
presented as one approve/reject decision, provided the request explicitly says
that it makes these contract changes:

- the exact guest setup and READY attachment set changes from five programs to
  six through crate-private `GuestProgram::TcpCorrelation` before `Connect4`;
- the guest-internal final-ACK option ABI, active socket-storage phases and
  child `Pending | Ready` lifecycle become normative;
- ADR-0159's literal no-pin rule gains the exact private pre-program-load
  create/pin/Aya-reuse/unlink staging exception, with no runtime pin or
  adoption;
- the existing typed setup errors represent the pinned failure set;
- the intake enables Fast Open with backlog 4,096 and the adapter privately
  counts Pending children in the same 4,096-place total; and
- DISTILL may make only the neutral five-to-six corrections and add the one
  parameterized Tier-3 cell in the existing lane.

Only the user's explicit approval of that complete decision can move
`TCP-CORRELATION` from proposed to accepted. If approved, the architect must
record the authority, rewrite the accepted clauses in present tense, reconcile
ADR-0159, update DISTILL/roadmap counts and criteria, and obtain the required
roadmap review. Silence, `TCP-CORRELATION-NO-WEAKEN`, the spike verdict or this
technical review is not approval.

## Iteration 8 verdict

**CHANGES_REQUESTED — USER_DECISION_REQUIRED.** No technical finding remains.
The capacity-free active socket storage closes iteration 7; its exact
armed/emit/delete/destruct lifecycle preserves final-ACK retransmission and
immediate release. The combined loader run proves the raw-BTF/Aya staging
sequence, and the child capacity, combined quota, typed failures, bounded
fallout and one-cell validation plan are coherent. The proposal is approved
for presentation to the user, not accepted into the architecture. Roadmap
validation remains pending and STEP 11-06 remains paused. No implementation,
test-body change, roadmap-body edit, DES event, user approval or commit is
authorized by this iteration.

---

# Review iteration 9 — approved-contract fold and roadmap revalidation

## Iteration 9 metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CORRELATION-STEP11-06-ARCH-REVIEW-09` |
| Date | 2026-10-23 |
| Reviewed at | `2026-10-23T11:44:26Z` |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | The user-approved iteration-8 contract folded into the feature delta, ADR-0150, ADR-0159 and reconciled STEP 11-06 roadmap |
| Verdict | **APPROVED** |
| Blocking findings | 0 |
| Roadmap disposition | **APPROVE after recording this review; DELIVER remains gated on the authorized DISTILL work** |

The exact user authority is recorded without inference. The orchestrator asked:

> Approve the reviewed correction
> [`.context/netns-density-295/11-06-design/correlation.md`], including that
> exception, the sixth correlation program, and one test in the existing lane?

The user replied:

> approved. continue

This iteration reviewed only the resulting canonical fold and STEP 11-06
reconciliation. It did not reopen the technically approved mechanism, add an
adjacent requirement, edit production/test/DES state, execute a kernel test or
commit any file.

## Fold-remediation dispositions

The first pass found two bounded status/audit wording defects. Both are closed:

| Finding | Remediation | Disposition |
|---|---|---|
| ADR-0150 still called the feature-delta correlation section pending | ADR-0150 now points to the accepted § *[REF] TCP capture correlation* | **Closed.** The authority/status chain is consistent. |
| The feature-delta no-weakening audit's “only additive contract” list omitted the approved ADR-0159 exception | The audit now names the private pre-program-load BTF/map create-pin-Aya-reuse-unlink staging exception and repeats unlink before program load, attach and READY with no runtime pin/adoption | **Closed.** The previously absolute no-pin contract is not concealed. |

No mechanism, lifecycle, typed error, test count or roadmap boundary changed in
that remediation.

## Exact authority and canonical-fold review

| Approved clause | Folded location | Assessment |
|---|---|---|
| Sixth `GuestProgram::TcpCorrelation` before `Connect4` and READY; the five prior meanings remain | Feature delta § *[REF] TCP capture correlation*; ADR-0150 status/decision; ADR-0159 guest-program list; decision index | Exact. The setup/READY set is expressly changed from five to six. |
| `Connect4` retains origin/rewrites; `TCP_CONNECT_CB` creates exact-socket capacity-free storage; active establishment enables the final-ACK option | Feature delta lines 931-941 and state deltas; ADR-0150 decision | Exact. Callback ownership is unchanged from the reviewed contract. |
| Exact 18-byte final-ACK ABI; ordinary and syncookie current-ACK load; Fast Open `Pending -> Ready` | Feature delta option/state clauses; ADR-0150 decision | Exact. No SYN carrier or saved-SYN premise returns. |
| Active `BPF_MAP_TYPE_SK_STORAGE`, `max_entries = 0`, delete after first later non-SYN peer packet or socket destruction; immediate file release preserves retransmission copy | Feature delta Contract Shape/state deltas; ADR-0150 decision | Exact. Iteration-7's fixed active collection remains removed. |
| Child-cookie `Pending | Ready` map, exactly 8,194 entries and one-time classification/destruction cleanup | Feature delta Contract Shape; ADR-0150 decision; roadmap criterion | Exact. The 4,097 + 4,096 + 1 arithmetic is unchanged. |
| One combined 4,096 count across adapter-held Pending children and emitted `CapturedConn` handles | Feature delta owner clause; ADR-0150 decision; roadmap criterion | Exact. No second quota or model variable is introduced. |
| Private best-effort wake, pre/post-wait scans and existing one-second audit fallback | Feature delta/ADR-0150 | Exact. No new timer, public event or recovery subsystem appears. |
| Private pre-program-load BTF/map create-pin-Aya-reuse-unlink staging exception; no runtime pin/adoption | Approval record; feature delta; ADR-0150; ADR-0159; decision index; roadmap notes | Exact. The pin is required to be absent before `SockOps::load`, attach and READY. |
| Exact setup errors | Feature delta and ADR-0150; ADR-0159 failure clause; roadmap notes | Exact. Staging/map failures are `ProgramLoad { TcpCorrelation, EbpfError::MapError(..) }`, attach is `ProgramAttach`, and listen/TFO setup is `IntakeBind`. |
| No sockmap, parking or payload path | Feature delta/ADR-0150 and roadmap avoidance notes | Exact. EVMOD-PAIR remains intact. |

The feature-delta approval record quotes the user's authority and enumerates the
approved scope. The decision index contains `TCP-CORRELATION` and amends D19's
status with the bounded 2026-10-23 exception. ADR-0150 is accepted in present
tense, and ADR-0159 narrows its ownership wording to no *runtime* pin while
pinning the sole pre-program-load exception and its fail-closed cleanup.

No approved `zero`, `never`, `must`, `exactly`, fail-closed or required outcome
was removed. Admission below `GUEST_TCP_PENDING_MAX`, exact identity,
reset-before-accept, per-socket original-peer restoration, process ownership,
no runtime adoption and receive-queue-only early-byte storage all remain.

## ADR-0159 lifecycle and failure review

ADR-0159 now records the exact six-program set and the only permitted pin
sequence:

1. `overdrive-init` creates minimal BTF and the active socket-storage fd;
2. it pins at a unique process-private bpffs path;
3. Aya 0.13.1 reuses it through
   `allow_unsupported_maps().map_pin_path(..)`;
4. it unlinks the map and removes the directory before `SockOps::load`, link
   attach or READY; and
5. service and teardown retain only process-owned fds, with no adoption path.

Normal returned failures clean staging state. An unlink-cleanup failure maps to
the approved typed `ProgramLoad`, blocks READY and ends the VM through PID-1
failure, so no later owner can adopt residue. Every attachment remains a BPF
link. No runtime audit, repair, persistent pin or restart adoption was added.

This is equal to the exception the user explicitly approved. ADR-0159's prior
literal no-pin wording was not silently reinterpreted.

## DISTILL authorization and test-count review

The fold authorizes exactly the reviewed test delta:

- existing transport bodies receive neutral five-to-six corrections for the
  enum list, `tcp_correlation` label, setup order/call count and existing
  `ProgramLoad` / `ProgramAttach` cases;
- `guest_flow::real_kernel` changes from 7 to 8 cells;
- `in_guest_lane` changes from 11 to 12 cells through its existing wrapper;
- the one added behavior is one parameterized source-local
  `TCP-CORRELATION` cell in the existing disposable guest-kit owner lane; and
- that cell covers production `Connect4` + `TcpCorrelation` +
  `KernelGuestOwnerIo`: ordinary exact one-time capture, immediate close,
  forced syncookie, actual data-in-SYN Fast Open `Pending -> Ready`, direct
  uncorrelated reset, combined-place release, active delete/destruct, child-map
  baseline and absence of staging residue during service.

The wrapper is the existing lane's invocation of that one cell, not a second
behavioral scenario. No new lane, model, benchmark, seeded private state,
raw-packet fixture, production test branch or public test-support API is
authorized. This matches the user's “one test in the existing lane” approval
and the exact artifact incorporated by reference in the approval question.

## Bounded compiler and caller fallout

The roadmap identifies the compiler-required changes without widening the
contract: the crate-private enum/`ALL` order and exhaustive matches,
load/attach dispatch, private Aya ABI/map/loader state, intake TFO/backlog
setup, and private `KernelGuestOwnerIo` pending table/scans. The existing
`GuestOwnerIo` and `GuestIoEvent` contracts, host wire, persisted state, flow
lifecycle and model variables remain unchanged. The roadmap's file list stays
guidance under workspace rules and does not prohibit tightly required compiler
fallout.

## Roadmap revalidation

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | Pass | The added Tier-3 cell composes the production capture programs and `KernelGuestOwnerIo` in the supported native-metal guest-kit lane; the Lima spike is reference evidence rather than the product test boundary. |
| Acceptance-criterion coupling | Pass | STEP 11-06 names the exact six-program order, real producer matrix, active/child cleanup, combined quota and absence of staging residue. |
| Step decomposition | Pass | The correction belongs to the existing guest program/transport/owner step; no cross-wave or unrelated step is added. |
| Implementation code | Pass | The roadmap pins interface, lifecycle, observable outcomes and evidence. Private helper/function structure remains the crafter's HOW. |
| Concision and precision | Pass | Counts are exact (`8`, `12`, one parameterized cell); typed failures and callback/map ownership have one interpretation. |
| Test boundary | Pass | DISTILL owns the new cell and neutral corrections before the crafter resumes; the crafter is expressly forbidden to author/edit test bodies or invent API. |
| No unrelated expansion | Pass | No new lane/model/public test API, host protocol, persistence, timer, recovery or payload mechanism is included. |
| Mechanical validity | Pass | `roadmap.json` parses, `git diff --check` passes for the folded artifacts, STEP 11-06 carries scenario id `TCP-CORRELATION`, and validation remained `pending` through this review. |

The roadmap is approved as the implementation plan for the folded contract.
Its execution order remains gated: DISTILL must first author and independently
review the authorized neutral corrections and one parameterized cell. Only
then may DELIVER resume STEP 11-06 with a fresh crafter under the repository's
strict sequence. Roadmap approval does not claim that the DISTILL prerequisite
already exists.

## Exact roadmap validation record

Record the following values in
`docs/feature/netns-density-295/deliver/roadmap.json`:

```json
{
  "status": "approved",
  "reviewer": "Independent TCP-CORRELATION folded-contract and STEP 11-06 roadmap review iteration 9 approved on 2026-10-23; DELIVER remains gated until DISTILL authors and independently reviews the authorized neutral five-to-six corrections and one parameterized cell in the existing guest-kit lane.",
  "approved_at": "2026-10-23T11:44:26Z"
}
```

This reviewer does not write that status itself; the DESIGN owner owns the
scoped fold/roadmap mutation and commit.

## Iteration 9 verdict

**APPROVED.** The canonical fold equals the user-approved iteration-8 contract,
including the bounded ADR-0159 exception. The two review-time wording defects
are remediated. STEP 11-06 preserves below-quota admission and exact identity,
authorizes only the neutral five-to-six corrections and one existing-lane
parameterized cell, and adds no unapproved mechanism or relaxation. The roadmap
may now record the exact approved validation block above. DELIVER remains
paused until the authorized DISTILL work and its independent review complete.
No production source, test body, DES state or unrelated dirty work was changed
by this review.
