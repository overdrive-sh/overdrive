# DISTILL review — STEP 11-06 TCP-CORRELATION correction

## Review metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CORRELATION-STEP11-06-DISTILL-REVIEW-01` |
| Feature | `netns-density-295` |
| Roadmap step | `11-06` |
| Scenario ID | `TCP-CORRELATION` |
| Review date | 2026-10-23 |
| Reviewer | Fresh isolated `nw-acceptance-designer-reviewer` |
| Design authority | Commit `9f7176260`; approved TCP-CORRELATION fold and design-review iteration 9 |
| Review scope | The authorized neutral five-to-six corrections and one parameterized source-local Tier-3 cell plus its existing-lane wrapper |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 1 |

## Review boundary and authority

This review is limited to the acceptance-test changes in:

- `crates/overdrive-init/src/transport/tests.rs`;
- `crates/overdrive-init/src/guest_flow/real_kernel.rs`; and
- `crates/overdrive-init/tests/integration/in_guest_lane.rs`.

The controlling contract is the accepted TCP-CORRELATION section of
`docs/feature/netns-density-295/feature-delta.md`, ADR-0150, ADR-0159, the
exact decision in `.context/netns-density-295/11-06-design/correlation.md`,
design-review iteration 9, and the approved STEP 11-06 roadmap criterion.
The user explicitly approved the sixth `TcpCorrelation` program, the private
pre-program-load create/pin/Aya-reuse/unlink staging exception, and one test in
the existing disposable guest-kit owner lane.

The authorized test delta is exact: neutral assertions move from five to six
programs, `guest_flow::real_kernel` moves from seven to eight cells, and
`in_guest_lane` moves from eleven to twelve cells. No new lane, model,
benchmark, seeded correlation state, raw-packet fixture, production branch,
public test-support API, KASAN run, mutation run, or physical-host kernel
mutation belongs to this review.

Concurrent removal of existing STEP 11-06 pending markers, changes in other
STEP 11-06 files, `execution-log.json`, and the unrelated dirty `AGENTS.md` are
outside this DISTILL author's body and outside this review artifact. They were
preserved.

## Strengths

- The neutral transport correction is exact. `ALL_PROGRAMS` now declares
  `TcpCorrelation, Connect4, Sendmsg4, Getpeername4, Recvmsg4, SockRelease`;
  the lower-snake-case labels match; the setup count changes from ten to
  twelve program calls; and both load and attach failure mappings name
  `TcpCorrelation`.
- The new behavior remains one table-driven Rust cell with one existing-lane
  wrapper. The counts are exactly eight source-local real-kernel cells and
  twelve in-guest integration cells.
- The wrapper uses existing owner-lane slot 11 and the disposable Cloud
  Hypervisor guest. The sysctl writes, BPF map inspection, socket traffic and
  bpffs inspection occur in the guest cell, not on the native metal host.
- The cell builds the approved production components directly:
  `configure_guest_transport` over `KernelGuestTransportOps`, followed by
  `KernelGuestOwnerIo`. Its application connects traverse production
  `Connect4`; no origin or correlation row is seeded by the fixture.
- Ordinary, immediate-close and syncookie cases compare the emitted full
  application `SO_COOKIE` and original destination. The syncookie case also
  requires `SyncookiesRecv` to increase.
- The Fast Open case primes the real client cookie cache and requires
  `TCPFastOpenPassive` to increase, so its payload is genuinely sent in the
  SYN rather than represented by a test-only state.
- The direct intake case supplies no origin, requires no `GuestIoEvent`, and
  observes `ECONNRESET`. Successful cases lookup-and-delete the child row,
  reject a second capture during a quiet bound, and return the child map to
  baseline.
- Private map inspection is source-local and opens kernel maps by ID. It adds
  no production/test-support API. Active socket storage is identified as
  `BPF_MAP_TYPE_SK_STORAGE` with `max_entries = 0`; the child map is identified
  by its exact 8,194-entry bound.
- The ordinary path observes active storage before capture, sends real
  post-handshake traffic, and waits for the exact socket's storage to be
  absent. Setup and service compare the entire bpffs tree to the pre-setup
  disposable-guest baseline, covering the approved absence of staging residue.
- Both the source-local cell and wrapper carry the exact
  `/// CONTRACT_SHAPE: bounded-change.` declaration. The Tier-3 table is
  example-based as required; no PBT or repeated quota-filling workload was
  added to a real-I/O lane.

## Contract coverage map

| Approved outcome | Authored evidence | Assessment |
|---|---|---|
| Sixth exact program order, labels and setup call count | `transport/tests.rs:62-69`, `:408-413`, `:660-670` | **Covered.** |
| Existing `ProgramLoad` and `ProgramAttach` mappings name `TcpCorrelation` | `transport/tests.rs:294-307`, `:707-721` | **Covered.** |
| Production `Connect4` + `TcpCorrelation` + `KernelGuestOwnerIo` | `guest_flow/real_kernel.rs:591-604` and each real application connect | **Covered by the intended RED boundary.** |
| Ordinary exact cookie/destination and one-time take | `guest_flow/real_kernel.rs:512-520`, `:396-430`, `:449-460` | **Covered.** |
| Immediate application release before userspace accept | `guest_flow/real_kernel.rs:522-534` drops the application socket before `captured` calls `next_event` | **Covered.** |
| Forced syncookie exact capture | `guest_flow/real_kernel.rs:536-549` | **Covered.** |
| Actual data-in-SYN Fast Open | `guest_flow/real_kernel.rs:551-575`, including a priming traversal and `TCPFastOpenPassive` delta | **Covered.** |
| Fast Open `Pending -> Ready` with one combined place held then released | `guest_flow/real_kernel.rs:568-574` observes only one child key before asking the owner for an event | **Not covered; blocker F-01.** |
| Direct uncorrelated reset with no event/flow/control | `guest_flow/real_kernel.rs:501-510` | **Covered at the approved `KernelGuestOwnerIo` boundary.** No event means the owner has no input from which to create a flow or control record. |
| Active delete/destruct and child-map baseline | `guest_flow/real_kernel.rs:433-460`, `:499`, `:578` | **Covered for active post-handshake deletion and every child complement.** The immediate-close case exercises socket-lifetime destruction through the exact `SK_STORAGE` map type; no separate collection exists. |
| Staging pin/directory absent after setup and during service | `guest_flow/real_kernel.rs:594-606` | **Covered on the fresh disposable guest.** |
| Exactly one new behavior and existing-lane wrapper at slot 11 | `guest_flow/real_kernel.rs:581-608`; `tests/integration/in_guest_lane.rs:450-464` | **Covered.** |

## Findings

| ID | Severity | Disposition |
|---|---|---|
| F-01 | **Blocker** | Open — the actual Fast Open case can pass without production `KernelGuestOwnerIo` ever accepting or counting a `Pending` child. |

### F-01 — Fast Open does not prove the approved Pending-owner transition or combined-place release

The Fast Open branch sends with a blocking socket:

```rust
app.fast_open(destination, b"data-in-syn");
let before_owner = maps.child_entries();
assert_eq!(before_owner, 1, "Fast Open owns one child row before owner classification");
let child = captured(io, destination, cookie);
```

`ApplicationSocket::fast_open` calls blocking `sendto(MSG_FASTOPEN)` and
returns only after it has written the complete payload
(`guest_flow/real_kernel.rs:310-323`). Linux's pinned implementation passes no
`MSG_DONTWAIT` flag to `tcp_sendmsg_fastopen`; `tcp_sendmsg_fastopen` therefore
calls blocking `__inet_stream_connect` (`vendors/linux/net/ipv4/tcp.c:1065-1109`),
whose connected return follows the wait while the socket is `SYN_SENT` or
`SYN_RECV` (`vendors/linux/net/ipv4/af_inet.c:702-736`). The cell does not call
`KernelGuestOwnerIo::next_event` until after that return
(`guest_flow/real_kernel.rs:568-574`, through `captured` at `:396-418`).

The pre-owner assertion cannot close the gap. `child_entries` deliberately
iterates keys without reading the private value (`guest_flow/real_kernel.rs:201-236`),
so `before_owner == 1` is equally true for `Pending` and `Ready`. The later
exact capture proves that the final ACK produced `Ready`; it does not prove
that the production adapter accepted and held the child while it was
`Pending`, charged one place to the shared 4,096 total, rescanned it to
`Ready`, and released that same place through the consuming abort.

**Passing counterexample:** an implementation can leave the data-in-SYN child
queued until the final ACK has already changed its row to `Ready`, then have
the first `KernelGuestOwnerIo::next_event` accept it as an ordinary ready
child. It returns the exact cookie/destination, deletes the row, emits once and
passes every current assertion without exercising the approved internal
Pending table or combined-count path.

This is a missing required transition, not a request for broader coverage.
The approved roadmap and design review expressly require actual data-in-SYN
Fast Open `Pending -> Ready` **with one combined pending place held then
released**. A real data-in-SYN counter and a key-count delta do not establish
that owner transition.

**Required remediation:** keep the same one source-local cell, existing lane,
production `Connect4`/`TcpCorrelation`/`KernelGuestOwnerIo`, private inspection
boundary and example-only Tier-3 mode. Add executable evidence that the
production adapter accepts and holds the actual data-in-SYN child while its
correlation row is `Pending`, later observes `Ready`, emits exactly once and
releases the same combined place when the returned child is consumed. Do not
add a lane, public or test-support API, seeded state, raw-packet fixture, model,
benchmark, quota-sized brute-force expansion, production branch or new
scenario. Preserve all currently covered ordinary, immediate-close,
syncookie, direct-refusal, cleanup and staging-pin outcomes.

The exact finding was sent to the original DISTILL author for bounded
remediation.

## Acceptance-review dimensions

| Dimension | Score | Assessment |
|---|---:|---|
| Happy-path bias | 9/10 | The exact approved matrix contains ordinary and three handshake alternatives plus a direct fail-closed refusal. The generic 40% heuristic does not authorize new failures beyond this user-approved one-cell scope. |
| Given-When-Then form | 9/10 | The Rust table has one setup, one connection stimulus per case and bounded outcome checks. The repository forbids executable `.feature` files. |
| Business/domain language | 9/10 | Names preserve the exact owner, program, callback, correlation state and cleanup terms from the accepted design. |
| Coverage completeness | 4/10 | All approved cases are named, but the Fast Open case omits the required owner-held `Pending -> Ready` and combined-place transition. |
| Walking-skeleton user-centricity | 10/10 | Not a walking skeleton; this is the explicitly approved focused Tier-3 kernel/adapter cell. |
| Priority validation | 10/10 | The cell targets the reproduced five-hook correlation gap and adds no adjacent mechanism or test program. |
| Observable behavior assertions | 8/10 | Exact event identity, reset, kernel counters, kernel map lifecycle and bpffs complement are observable Tier-3 effects. The missing Pending-owner transition is scored under completeness. |
| Traceability coverage | 10/10 | `TCP-CORRELATION` traces from the approved feature delta and ADRs through roadmap 11-06 to the source-local cell and slot-11 wrapper. |
| Walking-skeleton boundary proof | 10/10 | Not applicable as a walking skeleton; the production-composition boundary is independently verified below. |

The approval score gate fails because coverage completeness is below 5 and a
blocker remains.

## Test-design mandates and three pillars

| Gate | Result | Evidence |
|---|---|---|
| CM-A — hexagonal/production boundary | PASS | The cell uses production `configure_guest_transport`, `KernelGuestTransportOps`, real kernel sockets and `KernelGuestOwnerIo`; it does not call BPF program helpers or seed their maps. |
| CM-B — domain-language abstraction | PASS | Scenario/case names express exact correlation outcomes. Syscalls and map iteration remain fixture mechanics. |
| CM-C — complete journey | **FAIL** | The Fast Open journey begins with actual data in SYN and ends with exact capture, but skips proof of the contract-bearing owner-held Pending state and its release. |
| CM-D — pure logic/fixture isolation | PASS | No new business algorithm or environment matrix is embedded in fixtures; the parameter table varies only real adapter handshake modes. |
| CM-E — bounded-change universe | **FAIL** | The declared combined pending place is absent from the observed transition universe. Other active, child and bpffs complements are bounded and fail-closed. |
| CM-F — layer-dependent PBT mode | PASS | This Layer-4+/Tier-3 real-I/O cell is example-only. No PBT machinery or repeated load matrix is introduced. |
| CM-G — two-tier rich journey | PASS / N/A | The user approved one focused existing-lane cell. A new state-machine tier would be unapproved scope. |
| CM-H — real-I/O sad paths | PASS | Direct uncorrelated refusal is an explicit named case, not generated input. |

Pillar 1, exact domain language, passes. Pillar 2, the table's state narrative,
fails only at the omitted owner-held Pending transition. Pillar 3, production
composition, passes.

## Completeness taxonomy

| Category | Result | Evidence |
|---|---|---|
| C1 — equivalence and boundary | PASS | The six-program setup contract and exact identity boundary are asserted. |
| C2 — state and transition | **FAIL** | Fast Open proves data in SYN and eventual capture but not `Pending -> Ready` through the adapter-held state. |
| C3 — count/cardinality | **FAIL** | The one shared pending place is neither shown held nor shown released; no brute-force capacity expansion is required or authorized. |
| C4 — lifecycle and idempotency | PASS | One-time child consumption, post-handshake active deletion, exact-map destruction path, child cleanup and staging complement are exercised. |
| C5 — mode/decision table | PASS | Direct, ordinary, immediate close, forced syncookie and Fast Open modes match the approved exhaustive table. |
| C6 — negative and robustness | PASS | Missing correlation fails closed with reset and no event; no guessed identity or seeded state is accepted. |
| C7 — environment/interruption/concurrency | PASS | The cell runs only in the existing disposable native-metal Cloud Hypervisor guest and uses real guest-kernel hooks. |

The selected scenario is otherwise self-contained. Adding more scenarios,
property generators, raw packets, quota-sized connection bursts or a second
lane would not repair F-01 and is outside the approved test budget.

## Test honesty and production-path assessment

The intended RED is honest. The Lima all-target check fails on the missing
production `GuestProgram::TcpCorrelation` variant and the existing
five-element `GuestProgram::ALL`; those failures are the exact first
design-owned surface the crafter must add. The check reports no fixture setup,
import or unrelated type failure before that boundary.

The real-kernel body does not manufacture the disputed identity. Each
application socket obtains its kernel `SO_COOKIE`; each non-local destination
is submitted through production `Connect4`; and the only accepted identity is
the existing `GuestIoEvent::OutboundCaptured` produced by
`KernelGuestOwnerIo`. The direct connection deliberately bypasses `Connect4`
by targeting the local intake and therefore tests the missing-correlation
complement through the real owner seam.

The test's private BPF-map reads are valid Tier-3 kernel-side observations
under `.claude/rules/testing.md`. They do not become public API. F-01 arises
because the Fast Open map observation is key-only and occurs before the owner
call but after the blocking handshake completes; it is not a criticism of
private map inspection itself.

## Mechanical scope and verification

| Check | Result |
|---|---|
| Changed DISTILL body files | PASS — only the three authorized files contain this correction; concurrent marker removals and other STEP 11-06 work remain separately attributable. |
| Cell counts | PASS — 8 `guest_flow::real_kernel` tests and 12 `in_guest_lane` tests. |
| Contract Shape declarations | PASS — exact bounded-change rustdoc line on the new cell and wrapper. |
| New lane/model/public test support/raw packet/seeded state/benchmark | PASS — none added. |
| Host-effect boundary | PASS — the new kernel/sysctl/bpffs effects execute only inside the disposable guest reached through `cargo xtask metal run`; the wrapper does not mutate the physical host kernel. |
| `cargo fmt --all -- --check` | PASS. |
| Scoped `git diff --check` | PASS. |
| `cargo xtask lima run -- cargo xtask bpf-build` | PASS — the pinned nightly BPF build completed and wrote the existing target artifact. |
| `cargo xtask lima run -- cargo check -p overdrive-init --all-targets --features integration-tests,kvm-tests` | Expected RED — 8 diagnostics, all from missing `GuestProgram::TcpCorrelation` or `[GuestProgram; 5]` versus `[GuestProgram; 6]`. |
| Native-metal real-guest execution | NOT RUN — production TCP-CORRELATION does not exist yet; only the disposable Cloud Hypervisor lane can provide signal after GREEN. |
| KASAN | NOT RUN — outside this bounded correction. |
| Mutation testing | NOT RUN — correctly reserved for the single final DELIVER-wave gate. |

## Remediation disposition

| Item | Disposition |
|---|---|
| Neutral five-to-six program assertions | Preserve unchanged. |
| One source-local table and slot-11 existing-lane wrapper | Preserve unchanged. |
| Real producer, exact cookie/destination, direct reset and one-time consume | Preserve unchanged. |
| Active/child/staging complement checks | Preserve unchanged. |
| Fast Open data-in-SYN counter only plus post-handshake key count | Insufficient for the owner-held Pending contract; remediate F-01. |
| Additional scenario, lane, PBT, raw packet, seeded state, quota burst or API | Reject as unauthorized and unnecessary. |
| Production/design/roadmap/DES changes | Not authorized by this DISTILL finding. |

# CHANGES_REQUESTED

STEP 11-06 remains gated. The authored correction is tightly scoped and most
of its test design is exact, but the one approved Fast Open transition is not
yet executable evidence: the cell can pass while `KernelGuestOwnerIo` never
holds or counts a `Pending` child. After bounded remediation of F-01, the same
fresh reviewer must re-review the corrected body and record the next iteration
in this native Markdown artifact before DELIVER resumes.

---

# Iteration 2 — F-01 remediation re-review

## Iteration 2 metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CORRELATION-STEP11-06-DISTILL-REVIEW-02` |
| Review date | 2026-10-23 |
| Reviewer | Same fresh isolated `nw-acceptance-designer-reviewer` |
| Scope | Only the bounded remediation of iteration-1 finding F-01 in the same source-local TCP-CORRELATION cell |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 2 |

This section is additive. It preserves iteration 1's evidence and verdict,
then records the independent re-review of the remediation. The authority and
file boundary are unchanged. No new lane, scenario, model, raw-packet fixture,
seeded state, quota-sized burst, public API, test-support accessor, benchmark,
KASAN run or mutation run is authorized.

## Remediation inspected

The remediation makes four material changes to the Fast Open case:

1. the primed application uses nonblocking `MSG_FASTOPEN`;
2. source-local raw map inspection reads a child row's `Pending` or `Ready`
   tag;
3. an immediate `KernelGuestOwnerIo::next_event(Some(Instant::now()))` poll is
   followed by direct reads of an owner-held pending table and combined count;
   and
4. up to 16 sequential attempts are allowed, with complete child/event/count
   cleanup after every attempt that reaches `Ready` before the desired held
   `Pending` observation.

When the desired attempt occurs, the body requires one BPF child row in
`Pending`, no owner state before the poll, one adapter-held child and one
combined place after the poll, exact `OutboundCaptured` after the later scan,
zero held children with the combined place retained by the emitted handle,
then combined count zero after consuming `abort`. Child-row baseline and the
no-replay quiet bound remain.

This sequence directly targets F-01 and preserves the previous ordinary,
immediate-close, syncookie, direct-refusal, active-delete, child-complement and
staging-residue assertions. The retries are sequential and each unsuccessful
attempt is cleaned; they do not approach the 4,096 quota and are not a load or
benchmark expansion.

## Iteration 1 finding disposition

| Finding | Iteration-2 assessment | Disposition |
|---|---|---|
| F-01 — blocking Fast Open could reach `Ready` before the first owner poll and never exercise the held-Pending/count path | The new intended sequence observes `Pending`, immediately polls the production adapter, and checks hold/emission/abort count transitions. That would close the original false-positive if its observation boundary were valid and deterministic. | **Superseded by F-02 and F-03; not approvable in the current form.** |

## Iteration 2 findings

| ID | Severity | Disposition |
|---|---|---|
| F-02 | **Blocker** | Open — the acceptance body prescribes private production field names/storage and private BPF map value layout that the accepted interface contract leaves to the crafter. |
| F-03 | **Blocker** | Open — success depends on userspace winning an uncontrolled local-kernel Pending window in one of 16 attempts; the cap bounds work but does not make the evidence deterministic. |

### F-02 — The remediation turns private implementation structure into acceptance surface

`correlation_owner_test_observation` directly requires these production
members (`guest_flow/real_kernel.rs:528-539`):

```rust
held_pending_children: io.pending_tcp_children.len(),
combined_pending_places: io.tcp_pending_count,
```

Neither field exists in the accepted scaffold. The Lima all-target check now
fails with `E0609` for both exact names before a crafter has chosen the private
representation. A conforming implementation may derive the combined count
from existing socket ownership, store emitted and held children in one table,
use two collections with different names, wrap the state in another private
type, or maintain an invariant without a stored scalar. All satisfy the
approved semantic contract. The test rejects them before observing behavior.

The raw child-map read adds the same problem at the BPF/userspace boundary.
`child_phase` requires the value's first four bytes to be a native-endian
`u32`, with `Pending = 1` and `Ready = 2`
(`guest_flow/real_kernel.rs:166-179`, `:239-297`). The accepted design pins
`CHILD_CORRELATION[child_cookie] = Pending | Ready { application_cookie,
original_destination }`; it does not pin field order, tag width, numeric
discriminants or native-memory layout. Those remain private implementation
choices. The exact option ABI is pinned; the private child-map memory layout
is not.

This violates the workspace's interface-ownership rule in `AGENTS.md`:
architecture pins interface contracts while private functions, private types
and how internal state is threaded belong to the crafter. Source-local and
read-only access does not turn a production field or map layout into
DISTILL-owned test support. The approved no-public/no-test-support-API boundary
also does not authorize prescribing production fields as a substitute for an
accessor.

**Required remediation:** remove every requirement for exact private owner
field names/container shape/stored-count strategy and private child-map byte
layout/discriminants. Prove the accepted semantic transition through existing
observable behavior and permitted kernel effects without adding public or
test-support surface. If the approved boundary cannot expose the required
fact, report that bounded testability conflict to the orchestrator rather than
inventing the crafter's HOW. Do not route this internal/test-support issue back
to DESIGN and do not weaken the Pending/count criterion.

### F-03 — Sixteen live scheduling attempts are bounded but not deterministic

`fast_open_capture_through_owner_pending` sends a real nonblocking Fast Open
connection and succeeds only when userspace observes the child map while it is
`Pending` and the immediate owner poll accepts it before the final ACK makes it
`Ready` (`guest_flow/real_kernel.rs:605-674`). A `Ready` observation or direct
`OutboundCaptured` is cleaned and retried. After 16 such outcomes, the test
panics.

That is a bounded schedule search, not a deterministic acceptance stimulus.
The local TCP stack may process the SYN, SYN-ACK and final ACK before the test
thread runs after `sendto`, or between the raw map read and the immediate owner
poll. A completely conforming implementation can therefore produce `Ready` on
all 16 attempts and fail this test solely because of guest CPU and softirq
scheduling. Increasing a finite retry count changes probability, not the
contract or the schedule control.

The cleanup is honest: every unsuccessful attempt consumes the event, aborts
the child, returns the child map and combined count to zero, and does not fill
the quota. That prevents cross-attempt contamination but does not make the
required held-Pending observation reproducible.

**Required remediation:** replace the schedule lottery with a deterministic
sequence at the existing approved boundary that distinguishes an
adapter-held `Pending` child from a child still queued until `Ready`. Keep the
one-cell, real producer, no-seeded-state, no-raw-packet, no-new-lane and
no-new-API constraints. A fixed retry cap may remain only for a bounded kernel
wait whose success follows from a controlled precondition; it cannot be the
only mechanism for eventually winning an uncontrolled interleaving.

## Re-review of acceptance dimensions and mandates

| Gate | Iteration-2 result | Evidence |
|---|---|---|
| Coverage completeness | **FAIL** | The intended Pending/count sequence is asserted, but only through unapproved private representation and a nondeterministic schedule witness. |
| Observable behavior | **FAIL** | Exact event and kernel lifecycle assertions remain sound; direct production-field and private byte-layout assertions are implementation observations, not stable contract observations. |
| CM-A — production/port boundary | **FAIL** | The real production adapter is still driven, but its internal fields are now acceptance inputs. |
| CM-C — complete journey | **FAIL** | The complete path is represented, but a conforming run can fail before demonstrating it because the schedule is uncontrolled. |
| CM-E — bounded-change universe | **FAIL** | Count complements are explicit, but the universe is coupled to a stored scalar and exact private container/layout rather than the semantic state. |
| CM-F — layer-dependent PBT mode | PASS | The 16 attempts are a bounded real-kernel witness search, not PBT; no generator was added. Its determinism defect is F-03. |
| Scope/test budget | PASS | Still one cell and one wrapper; no brute quota expansion or second scenario. |

The remaining iteration-1 passes continue to hold for domain language,
priority, traceability, actual data-in-SYN proof, direct refusal, exact identity,
one-time emission, cleanup complements and disposable-guest isolation.

## Iteration 2 verification

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS. |
| Scoped `git diff --check` | PASS. |
| Lima all-target check | Expected RED with 10 diagnostics: the prior eight `TcpCorrelation`/five-to-six diagnostics plus two `E0609` failures for required private fields `pending_tcp_children` and `tcp_pending_count`. The two new diagnostics are evidence for F-02, not an acceptable semantic RED. |
| New lane/scenario/model/raw packet/seeded state/public or test-support API/quota burst | PASS — none added. |
| Attempt cleanup and bound | PASS — at most 16 sequential attempts, each unsuccessful accepted child consumed and counts returned to zero. |
| Deterministic Pending-owner witness | **FAIL** — no mechanism controls the SYN-to-final-ACK interleaving. |
| Native-metal real-guest execution | NOT RUN — production TCP-CORRELATION and the disputed private fields do not exist. |
| KASAN/mutation | NOT RUN — outside this correction/final-wave only, respectively. |

# ITERATION 2 — CHANGES_REQUESTED

F-01's semantic intent is now explicit, and the remediation stays within the
one-cell test budget. It cannot pass the DISTILL gate because it makes exact
private owner fields and private BPF value bytes part of the acceptance
contract, then relies on an uncontrolled 16-attempt scheduling race. STEP
11-06 remains gated pending a bounded, deterministic remediation that proves
the same semantic Pending/Ready/count lifecycle without prescribing the
crafter's private implementation.

---

# Iteration 3 — deterministic real-producer re-review

## Iteration 3 metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CORRELATION-STEP11-06-DISTILL-REVIEW-03` |
| Review date | 2026-10-23 |
| Reviewer | Same fresh isolated `nw-acceptance-designer-reviewer` |
| Scope | Only the bounded remediation of iteration-2 findings F-02 and F-03 in the existing TCP-CORRELATION cell |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 1 |

The authority, file boundary and one-cell test budget remain unchanged. This
iteration reviews the replacement evidence, not production code. No private
owner field, private child-map value layout, numeric state tag, accessor,
trait/API, seeded state, raw packet, quota burst, second scenario, new lane,
model or benchmark is present.

## Remediation inspected

The iteration-2 retry search is replaced with one controlled real-producer
sequence:

1. a normal production traversal primes the Linux Fast Open client cookie;
2. a scoped owner thread enters production
   `KernelGuestOwnerIo::next_event` and its sleeping state is observed before
   the application stimulus;
3. the application performs one blocking `MSG_FASTOPEN` with data;
4. during the kernel delayed-final-ACK window, the cell simultaneously
   requires one child-map key, the exact intake/client tuple in `SYN_RECV`, no
   returned owner event, and one additional process socket fd;
5. the same blocked `next_event` later returns the exact
   `OutboundCaptured`; the child map is empty and the socket-fd delta remains
   one; and
6. consuming `abort` returns the socket-fd count to the pre-owner baseline.

The Fast Open passive counter must still increase, preserving the independent
proof that the payload traveled in the SYN. Ordinary, immediate-close,
syncookie, direct-refusal, active-storage, child-map, no-replay and staging
complements are unchanged.

## Iteration-2 finding dispositions

| ID | Iteration-2 finding | Iteration-3 evidence | Disposition |
|---|---|---|---|
| F-02 | Test prescribed private owner fields and child-map value bytes | All direct owner-field reads, field names, stored scalar assumptions, child-value reads and numeric Pending/Ready tags are removed. Map inspection is key-count only. The Lima check contains no private-field/layout diagnostic. | **CLOSED.** |
| F-03 | Sixteen attempts searched an uncontrolled Pending schedule | One owner wait is established before the stimulus. Pinned Linux holds the final ACK behind `sk_write_pending`/`TCP_DELACK_MAX`, and the test requires the concrete `SYN_RECV`/key/fd/no-event conjunction in that window. No retry exists. | **CLOSED.** |

## Determinism and kernel-premise review

The delayed-ACK premise matches the pinned Linux 7.0 source:

- blocking `MSG_FASTOPEN` calls `tcp_sendmsg_fastopen`, which passes no
  `O_NONBLOCK` flag to `__inet_stream_connect`
  (`vendors/linux/net/ipv4/tcp.c:1065-1109`);
- because the Fast Open request carries data, `__inet_stream_connect` passes
  `writebias = 1` into `inet_wait_for_connect`
  (`vendors/linux/net/ipv4/af_inet.c:702-712`);
- `inet_wait_for_connect` increments `sk_write_pending` before sleeping and
  decrements it after the connect wait (`af_inet.c:604-625`); and
- the SYN-ACK path sees `sk_write_pending`, schedules the final ACK on
  `ICSK_TIME_DACK` for `TCP_DELACK_MAX`, and does not send the immediate ACK
  (`vendors/linux/net/ipv4/tcp_input.c:6994-7022`; `TCP_DELACK_MAX = HZ/5` in
  `include/net/tcp.h`).

The cell confirms the owner is already sleeping in `next_event` before the
blocking send. The conjunction at `guest_flow/real_kernel.rs:568-570` has a
specific meaning on the fresh disposable guest:

- one child-map key proves the real passive producer owns correlation state;
- the exact server tuple in state `3` proves that child is still
  `TCP_SYN_RECV`, before the delayed final ACK;
- the owner event channel being empty proves no `Ready` event escaped; and
- exactly one new socket fd, with the application fd already included in the
  baseline, proves production `KernelGuestOwnerIo` accepted and retains the
  intake child rather than leaving it on the listener queue.

After the timer-driven final ACK, the same blocked call returns exact
cookie/destination identity. This closes the prior schedule false positive:
the test cannot pass merely by accepting an already-Ready queued child. The
150 ms observation bound is inside the kernel's 200 ms maximum delayed-ACK
window and is a bounded wait on a source-pinned precondition, not a repeated
lottery.

## Iteration 3 finding

| ID | Severity | Disposition |
|---|---|---|
| F-04 | **Blocker** | Open — process socket-fd cardinality proves accepted-child ownership and lifetime, but not the required shared pending admission count. |

### F-04 — A socket fd is not evidence that the shared pending place was charged

The new sequence observes one additional process socket fd while the child is
internally held, the same cardinality after `OutboundCaptured`, and baseline
after `abort` (`guest_flow/real_kernel.rs:532`, `:568-570`, `:588-594`,
`:699-709`). These are sound observable facts about socket ownership:
production accepted one child, retained one child capability across the
Pending-to-emitted transition, and closed it on consuming abort.

They do not observe the design's distinct admission-accounting invariant.
The accepted contract requires `KernelGuestOwnerIo` to charge every accepted
child, including an internally held Fast Open `Pending` child, to the one
shared `GUEST_TCP_PENDING_MAX = 4,096` total; the charge remains while the
emitted `CapturedConn` is outstanding and is released by successful attach or
consuming abort/close. A file descriptor is the owned socket resource, not the
logical quota place.

**Passing counterexample:** production can accept and retain the exact Pending
child, wake on the final ACK, emit exact `OutboundCaptured`, keep the same fd,
and close it on abort while omitting the internal-Pending count increment and
decrement. Every child-map, `SYN_RECV`, event, identity, fd-count and cleanup
assertion passes. At scale, internally held Pending children then bypass the
shared 4,096 admission bound, violating the approved contract. The existing
event-side guard for emitted handles does not repair the uncounted internal
Pending interval.

Calling the `+1` fd the “same combined pending place” therefore assumes the
implementation relationship the acceptance test must prove. It is not an
observable equivalent of the count.

**Disposition and boundary:** do not restore the private owner-field or map
layout assertions rejected by F-02. Do not add a public/test-support API, raw
packet mechanism, seeded state, extra lane/scenario, or 4,096-connection quota
burst; all are outside the exact authorization. Under those constraints the
logical one-place charge currently has no independent observation. This is a
bounded DISTILL testability conflict for the orchestrator/user to resolve,
not authority to weaken “one combined pending place” into “one socket fd” and
not authority for the reviewer to invent production structure.

## Iteration 3 acceptance assessment

| Gate | Result | Evidence |
|---|---|---|
| Real producer path | PASS | Production `Connect4`, `TcpCorrelation` setup and `KernelGuestOwnerIo`; no map state is seeded. |
| Actual data-in-SYN Fast Open | PASS | Primed blocking Fast Open plus increasing `TCPFastOpenPassive`. |
| Pending-to-Ready adapter lifecycle | PASS | Owner asleep before stimulus; one key + exact `SYN_RECV` tuple + no event + accepted socket; same wait later emits exact capture. |
| One-time consume and socket lifecycle | PASS | Child map zero after emission, no replay, one retained socket until consuming abort, then fd baseline. |
| Shared combined-count lifecycle | **FAIL** | No assertion distinguishes a charged internal Pending place from an uncounted held socket. |
| CM-A — production/port boundary | PASS | No private production field/layout is referenced; observations are `/proc`, BPF key cardinality and the existing event port. |
| CM-C — complete journey | **FAIL** | Kernel/adapter/socket journey is complete, but the contract-bearing admission-accounting effect remains unproved. |
| CM-E — bounded-change universe | **FAIL** | Socket and child-map complements close; the combined count complement is inferred rather than observed. |
| Determinism | PASS | One source-pinned delayed-ACK schedule replaces the retry search. |
| Scope/test budget | PASS | One cell, one wrapper, no new surface or brute expansion. |

## Iteration 3 verification

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS. |
| Scoped `git diff --check` | PASS. |
| Lima all-target check | Expected RED with exactly the original 8 diagnostics: missing `GuestProgram::TcpCorrelation` and `[GuestProgram; 5]` versus `[GuestProgram; 6]`. No fixture/private-HOW diagnostic remains. |
| F-02 forbidden surface grep/review | PASS — no private owner field/accessor, child value read, numeric phase tag, public/test-support API or runtime branch. |
| Retry/seed/raw/quota-burst review | PASS — one real connection after priming; no attempt loop, seed, raw packet or capacity expansion. |
| Pinned kernel premise | PASS — `sk_write_pending` and `TCP_DELACK_MAX` path verified in vendored Linux 7.0 source. |
| Combined pending-place observation | **FAIL** — fd ownership is not quota accounting. |
| Native-metal real-guest execution | NOT RUN — production TCP-CORRELATION is still absent. |
| KASAN/mutation | NOT RUN — outside this correction/final-wave only, respectively. |

# ITERATION 3 — CHANGES_REQUESTED

The deterministic real-kernel Pending-to-Ready path is now honestly specified,
and the iteration-2 private-HOW and schedule defects are closed. The cell still
does not prove that the internally held Pending child consumes the shared
admission count: it proves one socket is owned. Because a no-count
implementation passes, the accepted combined-place invariant remains
unverified. STEP 11-06 stays gated while the orchestrator resolves this bounded
testability conflict without private-structure pinning, brute-force expansion
or contract weakening.

---

# Iteration 4 — combined admission-boundary re-review

## Iteration 4 metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CORRELATION-STEP11-06-DISTILL-REVIEW-04` |
| Review date | 2026-10-23 |
| Reviewer | Same fresh isolated `nw-acceptance-designer-reviewer` |
| Scope | Only the bounded remediation of iteration-3 finding F-04 in the existing TCP-CORRELATION cell |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 1 |

The orchestrator clarified that exercising the exact 4,096 admission boundary
inside this one approved parameterized cell is necessary observable coverage,
not a forbidden benchmark, new scenario or new lane. That clarification
supersedes iteration 3's statement that a quota-sized boundary proof was out
of scope. It does not authorize private production fields/layout, a public or
test-support API, seeded correlation state, raw packets, another test, another
lane, a model or a benchmark.

## Remediation inspected

The Fast Open case now adds one capacity-boundary sequence around the already
approved deterministic Pending witness:

1. hold `GUEST_TCP_PENDING_MAX - 1 = 4,095` ordinary production
   `OutboundCaptured` handles and their real application sockets;
2. run the actual data-in-SYN Fast Open path, including the source-pinned
   internally held `Pending` observation and exact later capture;
3. require the next production application connection, the 4,097th held
   child, to reset with no `OutboundCaptured`;
4. consume the Fast Open `CapturedConn` by `abort`;
5. require one ordinary replacement connection to produce exact
   `OutboundCaptured` while the original 4,095 handles remain; and
6. abort every retained handle, drop every application socket and require the
   exact pre-boundary process socket-fd baseline.

The implementation uses `GUEST_TCP_PENDING_MAX` rather than a duplicated
literal for the held count. The only literals are the accepted destinations
for distinct real-producer traversals. No map value is read and no owner field
or container is named.

## Capacity arithmetic and lifecycle assessment

The arithmetic is exact:

- 4,095 ordinary emitted handles occupy 4,095 shared pending places;
- the Fast Open child is the 4,096th accepted child;
- the subsequent connection is the first one beyond the bound and must reset;
- aborting the Fast Open handle returns the count to 4,095;
- one replacement is then admitted as the 4,096th place; and
- aborting the replacement returns to 4,095 before the retained set is fully
  consumed.

The test's fd accounting is also internally consistent. Before the capacity
setup it records a baseline. Each retained ordinary flow adds its application
socket and captured intake child, so the expected delta is
`2 × (GUEST_TCP_PENDING_MAX - 1)`. The TFO helper records its own baseline
after creating the TFO application socket; one accepted child adds one. The
over-limit probe's application socket is dropped on return. After TFO abort,
its child fd is gone and the application is then explicitly dropped. The
replacement's application and child are both consumed. Finally every retained
ordinary child is aborted and its application socket dropped, returning the
process to the exact pre-boundary socket count.

The guest launcher raises `RLIMIT_NOFILE` to 65,536 before executing the cell,
so the approximately 8,192 held application/child descriptors fit the
existing disposable-guest lane. The owner-cell timeout is five minutes; the
host-side script waits 60 seconds for `DONE`. Those are existing lane bounds,
not a benchmark contract. Runtime has not yet been measured because the
production sixth program is absent, but no new timeout or lane is introduced.

## Iteration-3 finding disposition

| ID | Iteration-3 finding | Iteration-4 evidence | Disposition |
|---|---|---|---|
| F-04 | A socket-fd delta did not prove the shared admission count | The 4,096/4,097 behavioral boundary now distinguishes counted from uncounted handles after emission, and abort plus one replacement proves release of the emitted TFO place. | **Partially closed; F-05 remains for the internally held Pending interval.** |

## Iteration 4 finding

| ID | Severity | Disposition |
|---|---|---|
| F-05 | **Blocker** | Open — the over-capacity refusal is driven after the TFO child has become `Ready` and emitted, so it does not prove that the earlier internally held `Pending` interval was charged. |

### F-05 — The boundary probe occurs after emission, leaving the Pending-count mutant alive

The deterministic helper does prove the TFO child was accepted and held while
the BPF row existed, the server tuple remained `SYN_RECV`, and no event had
returned. It then waits for the final ACK and returns the exact
`OutboundCaptured` (`guest_flow/real_kernel.rs:527-597`). Only after that
helper returns does the case call
`assert_capture_refused_at_combined_max`
(`guest_flow/real_kernel.rs:756-764`).

The refusal therefore proves that the TFO handle occupies the 4,096th shared
place **after `Ready`/emission**. The later abort/replacement proves that this
emitted place is retained and released. Neither observation reaches back to
the preceding internally held `Pending` interval.

**Passing counterexample:** `KernelGuestOwnerIo` can accept and hold the real
TFO child while it is `Pending` without incrementing the shared count, then
increment the count only when the final ACK changes the row to `Ready` and it
emits `OutboundCaptured`. With 4,095 ordinary handles, the later capacity
probe still sees 4,096 and resets. Aborting the emitted TFO handle decrements
to 4,095, so the replacement is admitted. All map, tuple, event, fd,
refusal/recovery and final-cleanup assertions pass. The accepted contract is
still violated because an arbitrary number of internally held Pending
children could bypass the bound before they become Ready.

The feature delta is explicit: “A Fast Open `Pending` child reserves one
`GUEST_TCP_PENDING_MAX` place immediately” and is held without an owner event
(`feature-delta.md:952-961`). Counting only at emission is not an equivalent
implementation.

**Required remediation:** exercise the over-capacity behavioral refusal while
the deterministic TFO child is still in the verified internally held
`Pending`/`SYN_RECV` window, before its final-ACK event returns. Preserve a
post-emission boundary or equivalent continuity check if needed, and preserve
the current abort-then-replacement proof for release. This is the same
authorized 4,096-boundary work inside the same cell. It requires no private
field/layout, API, seed, raw packet, additional test, lane, model or benchmark.

## Iteration 4 acceptance assessment

| Gate | Result | Evidence |
|---|---|---|
| Necessity and scope | PASS | The exact boundary is the minimum observable proof of shared admission accounting; root expressly confirmed it belongs in the existing cell. |
| Capacity arithmetic | PASS | `4,095 + TFO = 4,096`; next resets; TFO abort permits one replacement. |
| Post-emission count continuity | PASS | A 4,097th connection is refused while the emitted TFO handle is outstanding. |
| TFO place release | PASS | After consuming TFO abort, one replacement is admitted with the 4,095 ordinary handles retained. |
| Pending-immediate charge | **FAIL** | No capacity stimulus is processed before the TFO final ACK and event emission. |
| Lifecycle cleanup | PASS | Every child is consumed, every application socket is dropped, child-map baseline and exact socket-fd baseline are restored. |
| Private-HOW boundary | PASS | No owner field/container/count member or BPF value layout/discriminant is referenced. |
| One-cell count | PASS | Still eight source-local cells and twelve lane wrappers; the capacity rows are data within the existing Fast Open case. |
| No extra coverage mechanism | PASS | No new lane/test/model/benchmark, raw packet, seed or API. |

## Iteration 4 verification

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS. |
| Scoped `git diff --check` | PASS. |
| Lima all-target check | Expected RED with exactly the original 8 diagnostics: missing `GuestProgram::TcpCorrelation` and `[GuestProgram; 5]` versus `[GuestProgram; 6]`. |
| Required constant/source | PASS — uses production `GUEST_TCP_PENDING_MAX`; no duplicated capacity literal drives the boundary. |
| Fixture fd capacity | PASS — existing guest launcher sets `RLIMIT_NOFILE = 65,536`. |
| Full cleanup code path | PASS by inspection — every retained tuple is consumed and final socket count is the captured baseline. Runtime awaits GREEN. |
| Internally held Pending count mutant | **SURVIVES** — defer the TFO increment until emission and every current assertion still passes. |
| Native-metal real-guest execution | NOT RUN — production TCP-CORRELATION remains absent. |
| KASAN/mutation | NOT RUN — outside this correction/final-wave only, respectively. |

# ITERATION 4 — CHANGES_REQUESTED

The one-cell capacity proof is necessary, correctly authorized, arithmetically
exact and fully cleaned. It proves the TFO place is counted after emission and
released by abort. It still does not prove the contract's defining addition:
the child must consume that place immediately while internally held Pending.
Because a count-at-emission implementation passes, STEP 11-06 remains gated
pending the same boundary refusal during the verified Pending window.

---

# Iteration 5 — in-Pending admission-boundary re-review

## Iteration 5 metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CORRELATION-STEP11-06-DISTILL-REVIEW-05` |
| Review date | 2026-10-23 |
| Reviewer | Same fresh isolated `nw-acceptance-designer-reviewer` |
| Scope | Only the bounded remediation of iteration-4 finding F-05 in the existing TCP-CORRELATION cell |
| Verdict | **APPROVED** |
| Blocking findings | 0 |

This section preserves all prior iterations as review history and records the
final independent verdict for the current DISTILL body. The exact authority,
three-file body scope, eight/twelve cell counts and existing-lane wrapper are
unchanged.

## Remediation inspected

The first over-capacity connection now occurs inside
`fast_open_capture_through_owner_pending`, after the test has established all
four facts of the deterministic held-Pending window and before it waits for
the final-ACK event:

- one real child-correlation key exists;
- the exact intake/client tuple is still `TCP_SYN_RECV`;
- the already-blocked owner wait has returned no event; and
- production owns one additional accepted socket fd.

With 4,095 ordinary emitted handles already retained, the overflow
application performs one real production connect. It must observe
`ECONNRESET`. After that refusal, the owner wait must still have returned no
event, the TFO child map must still contain exactly one row, and production
must still own exactly the one accepted TFO child. The test then receives the
TFO's exact `OutboundCaptured` after the delayed final ACK.

The iteration-4 post-emission overflow remains. Consuming TFO abort still
permits one replacement capture, and the final cleanup still returns every
socket to the pre-boundary baseline.

## Finding dispositions

| ID | Finding | Iteration-5 disposition |
|---|---|---|
| F-01 | Blocking Fast Open skipped the held-Pending owner path | **CLOSED** — source-pinned delayed ACK plus pre-sleeping owner and exact `SYN_RECV`/map/fd/event observations prove the real held-Pending transition. |
| F-02 | Acceptance pinned private owner fields and BPF value layout | **CLOSED** — no private field, container, count member, accessor, value read or numeric phase tag remains. |
| F-03 | Sixteen uncontrolled schedule attempts | **CLOSED** — one controlled blocking-Fast-Open delayed-ACK sequence; no retry. |
| F-04 | Fd ownership alone did not prove combined admission accounting | **CLOSED** — exact 4,096/4,097 behavior now observes admission, continuity and release. |
| F-05 | First capacity probe occurred only after emission | **CLOSED** — overflow is reset while the TFO child is still the verified internally held Pending child. |

## Exact mutant-kill assessment

The current sequence rejects each material count defect independently:

| Defect | Observable failure |
|---|---|
| The 4,095 ordinary emitted handles are not counted | The in-Pending overflow is admitted/emitted instead of reset. |
| The internally held TFO Pending child is not charged immediately | The in-Pending overflow sees only 4,095 places and survives or produces `OutboundCaptured`; `expect_reset`, the empty event channel and exact fd count fail. |
| The count is added only at `Ready`/emission | Same in-Pending overflow failure occurs before the final-ACK event. |
| The Pending charge is dropped at `Ready`/emission | The second, post-emission overflow is admitted instead of reset. |
| Consuming TFO abort does not release its place | The ordinary replacement is reset instead of producing exact `OutboundCaptured`. |
| Overflow refusal damages or consumes the held TFO child | The one-row/fd/no-event complement fails before the final ACK, or the later exact TFO event is absent. |
| Any retained ordinary or replacement socket leaks from final cleanup | The final process socket count differs from the captured pre-boundary baseline. |

This closes the exact counterexample from F-05. A count-at-emission
implementation cannot use the later boundary to hide the missing Pending
charge because the first refusal must complete before the test receives the
TFO event.

## Timing and production-path assessment

The kernel premise approved in iteration 3 remains unchanged. Blocking
`MSG_FASTOPEN` gives the client `sk_write_pending = 1`; the SYN-ACK path
schedules the final ACK for `TCP_DELACK_MAX` rather than sending it
immediately. The owner thread is already blocked in production `next_event`
before the application sends. On the listener wake it can accept and hold the
TFO child, then process the overflow listener wake while that child remains
Pending. The overflow is a local production connection and its reset is the
designed capacity outcome, not a fixture-injected error.

The post-refusal checks retain teeth against timing drift. If the final ACK
wins before the in-Pending boundary is processed, the owner returns the TFO
event, the event-channel-empty assertion fails, or the TFO child no longer
matches the exact one-row/one-fd held complement. The test therefore cannot
silently relabel a post-Ready refusal as Pending evidence.

No raw packet, timer override, seeded map state or simulated owner state is
used. The kernel-defined delayed-ACK window and normal listener readiness are
the only scheduling mechanisms.

## Scope, necessity and cleanup

The 4,095-row setup is the minimum black-box input that distinguishes one
charged Pending place from one uncharged Pending socket at a 4,096 bound. Root
confirmed that this exact boundary is required coverage within the single
approved cell. It is not a throughput sample, repeated cohort, soak,
benchmark, new scenario or new lane.

The resource envelope is bounded and supported by existing lane setup:

- `GUEST_TCP_PENDING_MAX - 1` is read from the production constant;
- every setup connection travels through production `Connect4`,
  `TcpCorrelation` and `KernelGuestOwnerIo`;
- the guest launcher already raises `RLIMIT_NOFILE` to 65,536;
- 4,095 application sockets plus 4,095 captured children remain below that
  limit and the destination's ephemeral-port range;
- the source-local owner cell already has a five-minute process bound; and
- every handle is aborted, every application socket dropped, the child map
  returns to zero, no capture replays, and the exact socket-fd baseline is
  restored before the cell reports `DONE`.

There is no private production representation dependency. Map inspection
remains key-cardinality only; `/proc/net/tcp`, `/proc/self/fd`, socket terminal
behavior and the existing `GuestIoEvent` port are observable real-kernel
effects.

## Final acceptance assessment

| Gate | Result | Evidence |
|---|---|---|
| Neutral five-to-six corrections | PASS | Exact program order, labels, call count and existing load/attach failures. |
| Real producer and exact identity | PASS | Real application cookies/destinations through production setup and adapter; no seeded correlation state. |
| Immediate release, ordinary and syncookie | PASS | Exact capture and independent kernel-path counters where applicable. |
| Actual data-in-SYN Fast Open | PASS | Primed real TFO plus increasing `TCPFastOpenPassive`. |
| Pending-to-Ready transition | PASS | Exact `SYN_RECV`/child-key/no-event/accepted-fd held state followed by the same wait's exact capture. |
| Shared count immediate charge | PASS | In-Pending overflow reset at `4,095 + 1`; a missing or deferred increment fails. |
| Count continuity at emission | PASS | Second overflow remains reset while the TFO `CapturedConn` is outstanding. |
| Count release | PASS | TFO abort admits one replacement while the original 4,095 remain. |
| Direct missing-correlation complement | PASS | Direct intake resets with no event/flow/control input. |
| Active/child/staging complements | PASS | Active deletion/socket lifetime, one-time child take, child baseline, no replay and bpffs baseline. |
| CM-A — production/port boundary | PASS | Production composition and real kernel observations only; no internal field/layout API. |
| CM-B — domain language | PASS | Exact accepted TCP-correlation, owner, admission and cleanup vocabulary. |
| CM-C — complete journey | PASS | Every approved handshake/refusal/lifecycle outcome reaches its bounded observable complement. |
| CM-D — fixture isolation | PASS | Fixture code orchestrates real adapters and performs no copied business algorithm. |
| CM-E — bounded-change universe | PASS | Program set, active/child state, emitted handles, shared admission places, fds and staging complement are all closed. |
| CM-F/H — real-I/O input mode | PASS | One explicit table/capacity boundary, no PBT or generated sad-path expansion. |
| Traceability and test count | PASS | TCP-CORRELATION → STEP 11-06 → one source-local cell → one slot-11 wrapper; counts remain 8 and 12. |

## Iteration 5 verification

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS. |
| Scoped `git diff --check` | PASS. |
| Lima all-target check | Expected semantic RED with exactly 8 diagnostics, all from missing `GuestProgram::TcpCorrelation` or `[GuestProgram; 5]` versus `[GuestProgram; 6]`. No fixture, private-HOW or unrelated compile failure. |
| New lane/scenario/model/API/layout/seed/raw packet/benchmark | PASS — none added. |
| Exact boundary source | PASS — production `GUEST_TCP_PENDING_MAX`, with 4,095 retained plus the real TFO child. |
| In-Pending mutant kill | PASS by executable oracle — overflow must reset before the TFO event returns. |
| Emission continuity/release mutant kills | PASS by executable oracle — second refusal, then replacement admission after abort. |
| Full cleanup | PASS by executable oracle — child map/no-replay and exact fd baseline. |
| Native-metal real-guest run | Deferred correctly until GREEN; current production lacks the sixth program. The existing wrapper routes the eventual run through the disposable Cloud Hypervisor guest only. |
| KASAN/mutation | Not run; outside this correction and reserved for the final DELIVER gate, respectively. |

# ITERATION 5 — APPROVED

The authored DISTILL correction now matches the exact user-approved
TCP-CORRELATION contract. It preserves the six-program setup shape, drives the
real producer and owner, proves ordinary/immediate-close/syncookie/Fast-Open
identity, witnesses a real internally held Fast Open Pending child, proves its
immediate shared-count charge, continuity through emission and release on
abort, and closes every active/child/staging/socket complement. It does so in
one existing-lane cell with no invented public or private interface surface.

The dedicated DISTILL review gate is complete. STEP 11-06 may resume with its
fresh isolated DELIVER crafter under the repository's strict sequence.

---

# Compatibility iteration 6 — pre-authored STEP 11-06 bodies

## Compatibility review metadata

| Field | Value |
|---|---|
| Review ID | `NETNS-DENSITY-295-STEP11-06-DISTILL-COMPAT-01` |
| Review date | 2026-10-23 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| Scope | Compatibility of existing STEP 11-06 acceptance bodies and their source-local sim fixture with accepted K-A4, DG-5-Q, QPROJ-5/7/8 and EVMOD contracts |
| Body files reviewed | `guest_flow/tests/tcp_accept.rs`, `guest_flow/tests/udp_slots.rs`; remaining incompatible body in `guest_flow/tests/session.rs` |
| Fixture file reviewed | `guest_flow/sim.rs` |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 1 |

This is a new bounded compatibility gate after the approved TCP-CORRELATION
correction. It does not reopen iteration 5, add a scenario, change production,
authorize a new seam/API, or alter DES state. Its question is whether the
already-authored STEP 11-06 bodies honestly express the accepted event and
projection contracts once the crafter's production path becomes executable.

## Correction 1 — TCPACCEPT-END and K-A4

### Reviewed change

`a_v_g_the_host_closed_after_the_request_ends_with_no_c_and_no_message` now:

1. queues V_g with its complete request;
2. releases and settles `InboundAccepted`, so the owner accepts V_g and starts
   the request read;
3. releases the completed request `Read` event;
4. ends host V_h before either released event is dispatched; and
5. settles in queue order, so the `Read` event is handled first and the
   owner's post-read `inbound_ended` check sees the accepted V_g ended.

Previously the body ended host V_h while V_g was still an unaccepted queue
entry. K-A4 correctly removed that queue entry and recorded
`reset_in_queue`; the later `release_inbound_accept` therefore attempted to
accept an object that no longer existed. That sequence contradicted the very
K-A4 sibling case the suite already owns.

### Assessment

**PASS.** The correction distinguishes the two accepted orders:

- queued V_g closed before guest accept: torn down in the accept queue and
  never reaches the owner; and
- accepted V_g whose request was already buffered: the request read succeeds,
  then the owner observes the end and creates no C or control message.

The corrected body drives only existing sim actions and owner dispatch. It
adds no polling seam or invented kernel state. Its assertion on
`InboundEnded { ended: true }`, absence of `StartConnectApp`, empty accepted
table and empty outbox is the exact TCPACCEPT-END complement.

## Correction 2 — UDP SlotReady ownership boundary

### First-send pre-readiness evidence

After `app_send`, the body now reads the QPROJ-7
`guest_socket_observation()` and requires:

- the slot's parked kernel queue contains the exact `(SocketCookie,
  destination)` origin;
- one undispatched readiness fact names the slot; and
- QPROJ-5 still reports `owner = None` and no association.

Only `Guest::associate` releases `SlotReady` and settles the owner, after which
the `DatagramStream` request and owner projection are asserted.

**PASS.** This restores the accepted projection boundary. `app_send` mutates
the slot socket and readiness arbiter; it does not directly mutate owner state.
The old assertion that QPROJ-5 already owned the slot before `SlotReady` was
released collapsed QPROJ-7 into QPROJ-5.

### Three-slot identity evidence

`slots_are_held_per_socket_and_destination` now releases the three existing
held `SlotReady` events before settling and reading QPROJ-5. The repeated send
to the first `(socket, destination)` still returns the first slot and does not
create a fourth readiness event.

**PASS.** The body observes kernel slot identity before owner dispatch and
owner ownership after dispatch, without adding a polling hook or weakening the
one-slot-per-origin rule.

## Correction 3 — EVMOD-END reset callback in the sim fixture

`SimGuestOwnerIo::app_reset` still calls the real sim socket's `peer_reset`.
For an unattached captured intake child it preserves the accepted `Hangup {
Error(ECONNRESET) }` delivery path. For an attached half it now runs the
forwarder's queued callback in the reset's own fixture step.

**PASS.** This matches the method's existing rustdoc and EVMOD-END: a real
attached socket's `sk_error_report` runs the installed forwarder callback as
part of the reset effect. The acceptance body
`an_application_reset_while_paired_is_a_socket_error_record` can therefore
take the resulting end record without an unrelated second driver stimulus.
The later explicit `run_forwarder` in the FIN-then-reset case is a harmless
idle run and retains the separate FIN transition.

This is fixture fidelity, not production behavior embedded in the fixture.
Classification and outcome remain owned by `SimFlowForwarder`, the QPROJ-8
arbiter paired with the real module evidence.

## Compatibility finding

| ID | Severity | Disposition |
|---|---|---|
| C-01 | **Blocker** | Open — the session-loss body asserts full QPROJ-5 preservation after intentionally consuming three `Detached` records, contradicting D-11's two cumulative projection fields. |

### C-01 — Session-loss preservation oracle includes two fields that must change

`session_loss_aborts_every_flow_in_the_observing_dispatch_before_any_reconnect`
captures `obs = g.obs()` immediately after the dispatch that observes session
loss. At that point the owner has removed its paired TcpAccept, paired
TcpConnect and attached datagram association and has called
`abort_forward` for pair ids `p1`, `p2` and `p3`. Their three `Detached`
records are queued in the forwarder, not yet consumed by the owner.

The body then calls `g.run_and_settle()`, which runs forwarder work, releases
all queued records and dispatches them through the owner. It finally asserts:

```rust
assert_eq!(g.obs(), obs, "the aborts' Detached records change nothing (the flows are ended)");
```

That assertion is too broad. The functional session, flow and slot tables
must remain as they were after the observing dispatch because the flows are
already ended. D-11 and QPROJ-5 separately require every consumed end record
to mutate the observation:

- `consumed_ends` is cumulative and increments once per record; and
- `last_consumed_end` is replaced by each consumption and ends at the final
  `Detached` record.

The accepted `GuestSideObservation` contract pins these fields at
`feature-delta.md:7247-7252`; the `fwd_pair` conformance projection reads
`rec = read`, outcome, cause and errno from them. Treating the three records as
changing nothing would erase accepted D-11 evidence.

**Required body correction:** preserve the current assertion that session,
accepted flows, paired flows, awaiting-paired flows, slot ownership/
association/attachment and attach counters do not change while the already
ended flows' records are consumed. Exclude the two D-11 fields from that
preservation assertion, and assert the exact allowed delta:
`consumed_ends == obs.consumed_ends + 3` and `last_consumed_end` equals the
last consumed `Detached` record under the pinned queue order. Do not change
production to keep these fields unchanged and do not remove or weaken either
projection field.

## Ownership and disposition of C-01

C-01 is a blocking additional **DISTILL body correction** in
`guest_flow/tests/session.rs`. It is not crafter production remediation, a
DESIGN gap, a new scenario or optional cleanup.

The initial compatibility dispatch named `tcp_accept.rs`, `udp_slots.rs` and
the necessary `sim.rs` fixture. The repository's roadmap and file lists are
guidance rather than restrictive allowlists, and the acceptance designer owns
every pre-authored acceptance body. Once executable evidence proves that an
existing body contradicts the accepted QPROJ contract, leaving it outside the
correction merely because it was absent from the initial list would hand the
crafter an impossible choice: weaken D-11 or edit a test body DELIVER does not
own. The bounded correction must therefore return to DISTILL before this
compatibility gate can approve.

## Compatibility verification

| Check | Result |
|---|---|
| Focused Lima selector | PASS — nextest run `33fd3f02-defa-45cf-a042-fa486392d2bb`; 4/4 passed for TCPACCEPT-END, both UDP projection bodies and attached application reset. |
| Scoped `rustfmt --check` | PASS for `tcp_accept.rs`, `udp_slots.rs` and `sim.rs`. |
| Scoped `git diff --check` | PASS for the three correction files and this review artifact. |
| Full guest-flow handoff run | Handoff evidence: 57/58 with only the session-loss preservation assertion failing after three `Detached` consumptions; the logged projection delta was limited to `last_consumed_end` and `consumed_ends`. |
| Reviewer full guest-flow run in the concurrently changing shared worktree | 58/58 (`2abce0ae-272a-41ea-9020-8a36f5edeb60`). This does not close C-01: the current in-progress owner path can ignore records after removing their table entries, while accepted D-11 requires their consumption to update the two projection fields. A stale test that passes a non-projecting implementation is still incompatible. |
| Whole-workspace `cargo fmt --check` | Not attributable in this review: concurrent STEP 11-06 production files are incomplete/unformatted and one declared module is temporarily absent. The three scoped files pass independently. |
| New scenario/API/polling seam/model | PASS — none added. |
| Production/DES/commit mutation by reviewer | PASS — none. |

## Compatibility acceptance assessment

| Gate | Result | Evidence |
|---|---|---|
| K-A4 queued versus accepted V_g | PASS | Correct distinct sequences and complements. |
| DG-5-Q released-event discipline | PASS | Test drivers release events explicitly; settle dispatches only released work/deadlines. |
| QPROJ-7 kernel/socket facts | PASS | Parked origin and readiness read before SlotReady dispatch. |
| QPROJ-5 owner facts | **FAIL overall** | UDP boundary is corrected; session body still forbids required D-11 fields from changing. |
| QPROJ-8 forwarder facts | PASS | Reset callback is executed by the forwarder arbiter in the reset fixture step. |
| EVMOD-END fixture fidelity | PASS | Attached reset queues its classified record without a second external stimulus; unattached path stays Hangup. |
| Observable bounded-change complement | **FAIL** | Session body declares the two required cumulative record projections unchanged. |
| Scope and test count | PASS | Existing bodies only; no scenario/test/lane count change. |

# COMPATIBILITY ITERATION 6 — CHANGES_REQUESTED

The TCPACCEPT-END, UDP SlotReady/QPROJ and sim reset-callback corrections are
approved as exact compatibility fixes. The gate remains blocked by one stale
session-loss oracle that contradicts accepted D-11/QPROJ-5. DISTILL must make
that one additional body correction and return it for re-review; DELIVER must
not change production to satisfy the stale full-observation equality.

---

# Compatibility iteration 7 — session projection re-review

## Iteration 7 metadata

| Field | Value |
|---|---|
| Review ID | `NETNS-DENSITY-295-STEP11-06-DISTILL-COMPAT-02` |
| Review date | 2026-10-23 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| Scope | Only the bounded remediation of compatibility finding C-01, plus final disposition of the four already-approved compatibility corrections |
| Verdict | **APPROVED** |
| Blocking findings | 0 |

This section is additive and preserves compatibility iteration 6 as history.
No production code, scenario, test count, lane, model, API, DES state or commit
is added by this re-review.

## C-01 remediation

The session-loss body now reads the post-record observation once, saves its
two D-11 fields, resets only those two fields to their pre-record values and
compares the entire normalized observation with the snapshot taken after the
session-loss dispatch:

```rust
let after_records = g.obs();
let last_consumed = after_records.last_consumed_end;
let consumed_ends = after_records.consumed_ends;
let mut without_d11_delta = after_records;
without_d11_delta.last_consumed_end = obs.last_consumed_end;
without_d11_delta.consumed_ends = obs.consumed_ends;
assert_eq!(without_d11_delta, obs, ...);
```

It then requires:

- `consumed_ends == obs.consumed_ends + 3`; and
- `last_consumed_end` is pair `p3`'s
  `Aborted { cause: Detached }` record with `moved_ab = 0` and
  `moved_ba = 0`.

### Assessment

**C-01 CLOSED.** Only the two accepted D-11 deltas are normalized. Full
equality continues to protect the session flag, accepted/paired/awaiting flow
sets, every slot's owner/association/attachment, and both attach counters.
The correction therefore does not weaken the original preservation oracle.

The `+3` cardinality is exact: session loss aborts paired TcpAccept `p1`,
paired TcpConnect `p2` and attached datagram pair `p3`. The two unpaired flows
have no forwarder pair and produce no end record. The queue order is pinned by
the owner sequence: TCP pairs are aborted first; the attached datagram
association is ended afterward. `p3` is therefore the final consumed record.
No payload was advanced for that association before loss, so its moved counts
are exactly zero.

The body now rejects both error classes:

- production that fails to expose all three consumed records fails the
  `+3`/last-record assertions; and
- production that mutates any state beyond the two D-11 fields fails the
  normalized full-observation equality.

## Final sim reset-step review

The source-local fixture correction remains exact. `app_reset` identifies the
open outer socket and calls `peer_reset`. When its stage is `Captured`, the
socket is the unattached TcpConnect intake child; the fixture queues the
accepted `Hangup { Error(ConnectionReset) }` event and does not run a
forwarder callback. That preserves
`an_application_reset_before_paired_aborts_and_sends_abort`.

For an attached outer half the stage is no longer `Captured`; its installed
forwarder hook represents the real socket's `sk_error_report`. The fixture
runs queued forwarder work in that reset step, producing the EVMOD-END
`SocketError` record before the acceptance body calls `release_forward_ends`.
The FIN-then-reset body's later explicit run is idle after the reset callback
and does not create another record. Classification, errno, side and one-record
semantics remain owned by `SimFlowForwarder`/QPROJ-8.

No polling seam, test-only production branch or copied end-classification
algorithm is introduced.

## Final disposition of the five compatibility failures

| Item | Original incompatibility | Final disposition |
|---|---|---|
| TCPACCEPT-END body | Closed an unaccepted queue entry, then attempted to accept it | **CLOSED** — accept and arm read first; queue Read, then host end; Read dispatch observes accepted V_g ended. |
| UDP first-send body | Read QPROJ-5 ownership before SlotReady reached the owner | **CLOSED** — reads QPROJ-7 parked origin/readiness and asserts QPROJ-5 absent until explicit release. |
| UDP three-slot body | Settled without releasing its three held SlotReady facts | **CLOSED** — releases exactly the three existing events before owner projection. |
| Attached application reset fixture | Required an unrelated second driver stimulus to run the real reset callback effect | **CLOSED** — attached reset runs queued forwarder work in the reset fixture step; unattached captured reset remains Hangup. |
| Session-loss body | Declared all QPROJ-5 fields unchanged after consuming three Detached records | **CLOSED** — preserves every other field and asserts exact D-11 `+3`/last-p3-Detached deltas. |

All changes are compatibility corrections to existing bodies or their
source-local fixture. No scenario, test, lane or architectural mechanism was
added.

## Iteration 7 verification

| Check | Result |
|---|---|
| Scoped `rustfmt --check` | PASS for `tcp_accept.rs`, `udp_slots.rs`, `session.rs` and `sim.rs`. |
| Scoped `git diff --check` | PASS for all four compatibility files and this review artifact. |
| Focused compatibility run | Prior executable evidence remains 4/4 (`33fd3f02-defa-45cf-a042-fa486392d2bb`) for TCPACCEPT-END, both UDP bodies and attached reset. |
| Full-suite evidence before C-01 correction | 57/58; only the stale session full-observation equality failed, with the exact two D-11 fields changed. |
| Current exact rerun | Environment-limited by the concurrent crafter's temporarily absent `transport/kernel.rs` module. The compile interruption is outside these four files and does not alter the static contract review. |
| New scenario/test/lane/API/model | PASS — none. |
| Production/DES/commit mutation by reviewer | PASS — none. |

## Final compatibility assessment

| Gate | Result |
|---|---|
| K-A4 queued/accepted end distinction | PASS. |
| DG-5-Q released-event order | PASS. |
| QPROJ-5 owner projection | PASS, including exact D-11 cumulative fields. |
| QPROJ-7 socket/readiness arbiter | PASS. |
| QPROJ-8 forwarder classification and record order | PASS. |
| EVMOD-END reset-step fidelity | PASS. |
| Bounded-change complements | PASS; only named deltas are allowed and every other field is preserved. |
| Test count and authorized scope | PASS; existing bodies only. |

# COMPATIBILITY ITERATION 7 — APPROVED

All five discovered STEP 11-06 compatibility failures are corrected at their
proper DISTILL or source-local fixture boundary. The current bodies preserve
K-A4 event ordering, separate QPROJ-7 kernel facts from QPROJ-5 owner state,
model EVMOD-END's attached reset callback in the reset step, and retain D-11's
exact consumed-record projection. No stale acceptance oracle now requires the
crafter to weaken conforming production behavior.

---

# Iteration 8 — completed STEP 11-06 bounded additions

## Iteration 8 metadata

| Field | Value |
|---|---|
| Review ID | `NETNS-DENSITY-295-STEP11-06-DISTILL-REVIEW-08` |
| Review date | 2026-10-23 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| Scope | The user-approved thirteenth UDP-slot cell and sim mapping; existing Quint/model compatibility corrections; disposable-guest dummy-module fixture HOW; neutral real-kernel constructor fallout |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 1 |

This review does not reopen the approved TCP-CORRELATION body or compatibility
iteration 7. It reviews only the bounded additions and corrections completed
after that gate. The reviewer changed no source, test, spec, fixture or DES
state.

## 1. Guest UDP slot attach-refusal cell

### Authority and count

`SLOT-ATTACH-REFUSAL-SHAPE` is accepted in the feature delta and ADR-0210.
The user explicitly approved one additional source-local UDP-slot cell. The
suite now contains exactly 13 `#[test]` cells; no lane or public test support
was added.

### Contract mapping

The new
`a_v_g_ending_during_paired_refuses_the_attach_and_the_slot_reassociates`
cell covers the complete approved path:

- the first datagram claims resident S and starts association `first`;
- V_g is ended after host `Paired` is in flight but before guest attach;
- `attach_slot` returns the typed `GuestSlotAttachRefused { error, vsock }`
  shape, represented in the sim by `AttachSlot { accepted: None }` and the
  unchanged error from `SimFlowForwarder::attach`;
- `forwarder_attach_ended` increments exactly once, which is specific to the
  `HalfEnded` semantic branch;
- the returned V_g is closed before the existing end-of-association order
  discards and disconnects S;
- S remains resident and owned by the exact application cookie/destination;
- the refusing guest context has no live pair and no queued end record and
  the guest writes no control message;
- the already-attached host pair independently produces exactly its normal
  `PeerAbort { side: A }` end record when the guest close reaches V_h; and
- the next application datagram reuses S but starts a fresh association id.

The sim adapter maps every attach error without converting borrowed S into an
owned handle: success stores the pair; failure returns only the original
`HostConn` V_g in `GuestSlotAttachRefused`. No `GuestOwnedSocket::Slot`, fake
handle or optional stream outer half exists.

### Assessment

**PASS.** The cell is one exact stateful example for a single prescribed race,
not a combinatorial input property. The accepted contract owns the error
taxonomy and lifecycle; the body observes each externally meaningful
complement and does not duplicate the forwarder's attach algorithm.

Focused Lima evidence
`.context/netns-density-295/11-06/udp-slot-refusal-pass.log` passed 1/1,
nextest run `4b09d427-8e32-424f-9bda-691d4c1c8a28`.

## 2. Quint and model compatibility

### UDP owner/kernel projection split

`udp_slots.qnt` now separates the two accepted facts:

- `AppClaim` atomically writes `kernelSlots` and parks the datagram, expressing
  the QPROJ-7 capture-program/kernel fact while QPROJ-5 remains unowned; and
- `GuestReady` consumes that readiness and atomically publishes owner
  `slots.owner`, claims the flow id and writes the request.

Release clears both projections. The safety predicates compare kernel
identity before readiness and require every published owner to match its
kernel claim. Named runs now insert `GuestReady` before expecting QPROJ-5
ownership. This aligns the model with the compatibility-approved UDP bodies;
it does not weaken slot uniqueness, frame ownership, release, stale
reassociation or late-Paired invariants.

The REC-only guards have a bounded purpose:

- conformance omits an audit stutter while the earlier pairing deadline is
  pending and no orphan is released; and
- conformance represents one pairing deadline only when no simultaneous
  earlier orphan and no second due pending association exists.

Both guards are `not(REC) or ...`; every model-checking, liveness and hazard
instance has `REC = false` and retains the full abstract-time state space.
They remove driver steps that production's settle/clock boundary cannot
represent, not failure states or formal schedules.

The UDP driver changes decode the ITF encoder's singleton-set unwrapping and
empty-set/list ambiguity using the action's known type. They do not coerce a
nonempty list or a value of another domain.

**PASS.** The recorded 13/13 UDP named-plus-random run predates the last
REC-only guard edit, so this reviewer reran the current tree. Current Lima run
`7c3b7598-a89c-4224-98b3-f4d59c700a2a` passed 13/13. The formal safety and
release-liveness records passed; the later guards are false only with
`REC = true`, so they do not alter those formal instances.

### Exact disabled-action expectations

The owner-flow spec extracts unchanged action guards into `canActivate`,
`canGuestOpenSession`, `canDialOpen`, `canDialNoSession`,
`canDialUnreachable` and `canHostDialDeadline`. Named runs that previously
ended in `.then(Action).fail()` now end in exact `not(canAction)`
expectations.

**PASS.** quint-connect cannot deserialize the meta-only terminal state
created by `.fail()`; the replacement asserts the same precondition is false
in the preceding executable state. The guarded action definitions call those
same predicates, so no disabled action is made enabled and no behavioral
outcome is removed. The current evidence is 46/46 owner named-plus-random
tests passing (`quint-owner-pass.log`).

The guest owner driver's pre-first-dispatch `GuestOpenFlow` no-op is also
bounded: in that named stale-host-flow state, the action is a host-side queued
workload stimulus and the guest owner has not started; no guest projection
exists to drive. After the first dispatch the same handler executes the real
guest capture path.

### Forwarder sim primitive

`SimSocket::peer_shutdown` now preserves the first recorded end class rather
than overwriting it on a later transition. **PASS.** This strengthens
`ClassFixedAtTransition` and matches the kernel callback contract. It does not
drop later peer bits or suppress the hook; only the immutable classification
fact remains first-writer-wins.

### Quint evidence retained

- fwd-pair named plus random: 38/38 (`quint-fwd-pass2.log`);
- owner-flow named plus random: 46/46 (`quint-owner-pass.log`);
- current UDP named plus random: 13/13 (review rerun above);
- UDP safety TLC: PASS (`udp-slots-safety-tlc.log`); and
- UDP release liveness TLC: PASS (`udp-slots-release-liveness.log`).

These green results establish mechanical compatibility. They do not override
the projection-source blocker below.

## 3. Guest-kit dummy-module fixture HOW

The stock Tier-3 kernel ships the dummy-link driver as a module. The fixture
now locates `drivers/net/dummy`, decompresses `.ko.zst` when necessary, and
stages it at a test-only guest path. A `GuestInit::OverdriveInit` image boots
the existing scripted peer as a short-lived module-loading PID 1, loads the
dummy module inside that disposable guest with the existing `finit_module`
helper, then `exec`s the staged production init. `exec` preserves PID 1.

The branch occurs before the peer's optional forwarder-module loop, so
production `overdrive-init` still validates the kit module release, invokes
`finit_module` for `ovd_fwd.ko`, opens/probes the context and owns every READY
decision. A missing or refused dummy prerequisite powers off the guest rather
than skipping the production cell. The physical metal host is only the source
of the stock module bytes; the fixture never loads dummy there.

The peer's pre-existing proc/sys/dev, loopback and vsock setup means production
exercises documented idempotent/already-loaded outcomes for those prerequisites
on this fixture. That does not bypass the STEP 11-06 forwarder validation/load
or guest transport creation under review.

**PASS as fixture HOW.** No product config, CLI, production test branch or
runtime daemon surface is added. `cargo check -p overdrive-testing
--all-targets` passed in `guest-kit-dummy-check.log`.

## 4. Neutral real-kernel constructor fallout

`KernelGuestOwnerIo::new(ops)` now returns the configured correlation-map
result. The two source-local real-kernel construction sites add `expect` with
the exact setup meaning before using the same production adapter. No alternate
constructor, default map, fallback path or test-only state is introduced.

**PASS.** This is compiler-required fallout from the accepted typed setup
path, not a new test contract.

## Iteration 8 finding

| ID | Severity | Disposition |
|---|---|---|
| Q-01 | **Blocker** | Open — the guest fwd-pair driver fabricates QPROJ-8 socket facts from action bookkeeping, so green conformance can hide a broken sim/kernel effect. |

### Q-01 — Driver flags are used as socket projection sources

The accepted fwd-pair map pins these variables to the retained `SimSocket`
arbiter:

- `vs(g).sent` → `SimSocket::sent_shutdown()`;
- `vs(g).sshut` → `SimSocket::own_send_shut()`;
- `dir(hg).rst` → the destination/far-reader reset fact; and
- `pr(g).closed` → the flow left the owner table **and** the retained vsock
  half is closed.

`feature-delta.md:10218-10224` names these sources exactly. Driver action or
stimulus bookkeeping is not a substitute for the socket fact.

The current `quint_fwd_pair.rs` adds `owner_close_applied`,
`owner_abort_applied`, `refused_close_applied` and
`context_release_applied`. `sockets_view` combines those flags with an owner
effect-log check to create `projected_owner_end`, then injects `{S, R}` into
`vs(g).sent` and forces `vs(g).sshut = true` even when the retained
`SimSocket` reports neither (`quint_fwd_pair.rs:1355-1383`). The same flags
force the `dir(hg).rst` projection (`:1442-1448`) and help satisfy
`pr(g).closed` (`:1293-1298`).

These are not harmless relabellings. They copy the expected consequence of
the model action into the driver's observed state.

**Concrete false pass:** let `SimGuestOwnerIo` record its expected
`Close`/`Abort` effect and remove the flow while failing to shut/reset the
retained V_g `SimSocket`. Or let a context row become released while its
socket release side effects are absent. The new flags become true, the driver
manufactures the expected `sent`, `sshut`, reset and closed values, and the
fwd-pair suite can remain 38/38 green. The exact kernel/socket behavior the
conformance driver claims to compare is never observed.

**Required remediation:** project every S-class socket field from the named
SimSocket/far-end arbiter. If the source-local fixture does not update the
retained socket on owner close, abort, refusal or context release, correct the
fixture/socket lifecycle so the documented read-back reports the real effect.
An explicitly classified owner action result may be asserted as its own
effect, but it must not be converted into QPROJ-8 socket state. Remove the
action-derived socket synthesis and rerun the full 38-test fwd-pair selection.

Cached previously observed pair/socket handles may preserve identity after an
owner table releases them, but cached values must remain observations; they
must not be overwritten from the model step or an “action applied” flag.

## Iteration 8 verification

| Check | Result |
|---|---|
| UDP-slot refusal focused Lima | PASS — 1/1, run `4b09d427-8e32-424f-9bda-691d4c1c8a28`. |
| fwd-pair named plus random | PASS mechanically — 38/38, run `cc4e62a7-e80e-441e-adc0-6476802621dc`; blocked semantically by Q-01. |
| owner-flow named plus random | PASS — 46/46, run `c3d9f9c6-5125-4685-b837-ddce62bc4462`. |
| UDP named plus random, current tree | PASS — 13/13, run `7c3b7598-a89c-4224-98b3-f4d59c700a2a`. |
| UDP formal safety/liveness | PASS — recorded TLC checks; REC-only guard edits leave formal instances unchanged. |
| overdrive-testing all-target check | PASS — `guest-kit-dummy-check.log`. |
| Scoped `rustfmt --check` | PASS for all reviewed Rust files. |
| Scoped `git diff --check` | PASS for reviewed Rust/spec files and this artifact. |
| UDP test count | PASS — exactly 13. |
| New scenario/lane/public test API | PASS — none. |

# ITERATION 8 — CHANGES_REQUESTED

The thirteenth UDP-slot cell, its typed refusal mapping, UDP/owner Quint
alignment, exact disabled-action expectations, dummy-module guest fixture and
constructor fallout are bounded and accepted. The fwd-pair conformance driver
still fails the test-honesty gate because it derives socket outputs from its
own action bookkeeping. STEP 11-06 remains gated until Q-01 is remediated and
the 38 fwd-pair cases pass against genuine arbiter observations.

---

# Iteration 9 — separate bounded dependency verdicts

## Purpose and global disposition

This section records independent final acceptance decisions for two bounded
dependencies that do not depend on the open Quint driver finding. It does not
supersede iteration 8's `CHANGES_REQUESTED` verdict for the full DISTILL gate.
Q-01 remains open, no branch is globally GREEN, and STEP 11-06 must not advance
past the repository's global review sequence until the Quint projection is
remediated and re-reviewed.

## Dependency A — typed UDP-slot refusal and thirteenth cell

| Field | Value |
|---|---|
| Subscope | `GuestSlotAttachRefused`, `GuestOwnerIo::attach_slot` sim mapping, and the one approved source-local UDP-slot refusal body |
| Authority | SLOT-ATTACH-REFUSAL-SHAPE design review iteration 3 and the user's exact “Approve the UDP-slot test” decision |
| Verdict | **APPROVED** |
| Blocking findings | 0 |

The final body equals the accepted iteration-3 contract:

- datagram refusal carries exactly `GuestSlotAttachRefused { error:
  ForwarderError, vsock: HostConn }`;
- stream refusal remains the two-owned-half `GuestAttachRefused`;
- sim `attach_slot` borrows resident S, consumes V_g and returns V_g unchanged
  on every forwarder error;
- no slot socket enters `GuestOwnedSocket`, no fake socket/id is synthesized,
  and no optional TCP outer weakens the stream carrier;
- the owner recognizes `HalfEnded` through the exact returned error, counts it
  once, closes V_g, then runs discard/disconnect on S with `release = false`;
- S remains resident and owned, the local association/pair state is absent,
  the refusing guest context owns no pair/id/end record and no guest control
  message is written;
- the separately attached host pair ends through its ordinary one-record path;
  and
- a later datagram reuses S and starts a new association id.

The source-local body is exactly the user-authorized thirteenth cell. It adds
no lane, fixture, Quint state, public test support or product API. Its focused
Lima run passed 1/1 (`4b09d427-8e32-424f-9bda-691d4c1c8a28`), the suite count
is exactly 13, and scoped formatting/diff checks pass.

**Bounded handoff consequence:** the original STEP 11-06 crafter may rely on
this approved test/carrier dependency while completing the already-approved
native path. This approval does not waive Q-01 or authorize the crafter to edit
the DISTILL body.

## Dependency B — disposable-guest dummy prerequisite fixture

| Field | Value |
|---|---|
| Subscope | GuestKit staging/decompression of the stock dummy module and test PID-1 loading before `exec` of production `overdrive-init` |
| Authority | Bounded DISTILL fixture-HOW disposition recorded after the native owner lane reproduced `DummyDevice`/`EOPNOTSUPP`; appliance contract already requires `CONFIG_DUMMY` |
| Verdict | **APPROVED** |
| Blocking findings | 0 |

The fixture correction is narrowly environmental:

- `host_module_file` selects the running Tier-3 kernel's
  `drivers/net/dummy.ko` or `.ko.zst`, decompressing only into the fixture work
  directory;
- the module is copied into the disposable guest image at the test-only
  module path;
- only the guest's short-lived fixture PID 1 invokes the existing
  `finit_module` helper for dummy;
- failure to locate, decompress or load dummy fails the image/guest path
  closed;
- the metal host's module state is never changed, and no reboot, unload or
  identity substitution occurs;
- the helper then `exec`s the staged production init, preserving PID 1;
- production init still parses the production transport token, creates the
  dummy link/address/rule, validates and loads the exact guest-kit
  `ovd_fwd.ko`, opens/probes its context and exclusively owns READY; and
- the missing/other-release/other-build forwarder refusal paths remain
  production paths because the fixture does not preload the forwarder before
  the `testinit` exec branch.

The peer's existing proc/sys/dev, loopback and vsock setup yields documented
idempotent or `EEXIST` outcomes when production init repeats those operations;
it does not bypass forwarder identity/validation or the guest transport setup
being exercised. The fixture carries the same stock kernel release and exact
module bytes used by the guest; it does not substitute another kernel or
module identity.

Mechanical evidence:

- `cargo check -p overdrive-testing --all-targets` passed
  (`guest-kit-dummy-check.log`);
- native guest-kit preparation completed for stock kernel
  `7.0.0-38-generic`;
- the post-correction native lane advanced beyond the reproduced
  `DummyDevice/EOPNOTSUPP` boundary into the later TCP-correlation map setup,
  demonstrating that the dummy prerequisite loaded and production dummy-link
  creation ran; its later map-staging failure is outside this fixture
  dependency verdict; and
- scoped formatting/diff checks pass.

**Bounded handoff consequence:** the original crafter may use this approved
fixture dependency for the disposable native guest path. It does not authorize
loading dummy on the physical host, changing product image identity, bypassing
production forwarder validation, or calling the native lane GREEN before its
later production failures are resolved.

## Separate dependency verdict summary

| Dependency | Verdict | May unblock bounded crafter work? |
|---|---|---|
| Typed UDP-slot refusal + sim mapping + thirteenth body | **APPROVED** | Yes, within the accepted STEP 11-06 native implementation scope. |
| GuestKit dummy staging + guest-only preload/exec fixture HOW | **APPROVED** | Yes, for the disposable guest prerequisite only. |
| Existing fwd-pair Quint compatibility | **CHANGES_REQUESTED** | No; Q-01 remains the global DISTILL blocker. |

# ITERATION 9 — BOUNDED DEPENDENCIES APPROVED; GLOBAL GATE STILL BLOCKED

These approvals are final for their stated subscopes. They permit dependency
ordering inside the active crafter's bounded work while preserving iteration
8's Quint blocker and the repository rule that no global GREEN, review
completion or next roadmap step may be reported until every required review
returns `APPROVED`.

---

# Iteration 10 — QPROJ-8 arbiter remediation re-review

## Iteration 10 metadata

| Field | Value |
|---|---|
| Review ID | `NETNS-DENSITY-295-STEP11-06-DISTILL-REVIEW-10` |
| Review date | 2026-10-23 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| Scope | Only Q-01 remediation: genuine retained-socket projection, fixture lifecycle facts, and the accepted guest `TakeEnd`/`OwnerClose` dispatch split |
| Verdict | **APPROVED** |
| Blocking findings | 0 |

This iteration closes the one global blocker retained by iterations 8 and 9.
The bounded dependency approvals in iteration 9 remain unchanged.

## Q-01 remediation

### Driver projection sources

All action-derived projection fields have been removed from
`quint_fwd_pair.rs`:

- no `owner_close_applied`, `owner_abort_applied`,
  `refused_close_applied` or `context_release_applied` remains;
- no `projected_owner_end` injects shutdown/reset state;
- no `last_pair_state` fabricates a current forwarder observation; and
- the driver no longer builds socket queue state from moved counters.

The current driver reads the exact named arbiters:

| Model fact | Current source |
|---|---|
| `vs(g).sent` | retained V_g `SimSocket::sent_shutdown()` |
| `vs(g).sshut` | retained V_g `SimSocket::own_send_shut()` |
| `vs(g).ps`, `.cls`, `.st`, `.err` | retained V_g received shutdown, first recorded class, vsock state and `sk_err` |
| `dir(gh).q0` | outer `SimSocket::stream_input()` |
| `dir(hg).q1` | V_g `SimSocket::stream_input()` |
| `dir(hg).q2` | outer `SimSocket::stream_output()` |
| `dir(hg).got` | outer far peer's `peer_received()` |
| `dir(hg).rst` | outer `SimSocket::peer_was_reset()` |
| `pr(g).closed` | consumed record, flow absent and retained V_g reporting `SHUTDOWN{S,R}` |

These sources match the accepted QPROJ-8/QPROJ-7 map. The driver retains
socket identity after the owner table drops it, but the retained object is the
arbiter itself; no expected value is cached or replayed from a model action.

### Fixture facts

`SimSocket::close_from_owner(reset)` now records the direct local-close effect:

- TCP reports reset or EOF to a still-live far peer;
- `peer_was_reset` remains true when a local reset follows an earlier EOF,
  preserving ResetNeverEndsClean evidence;
- a TCP peer that already reset (`sk_err` present) does not receive a
  synthetic second local reset;
- vsock STREAM records the owner's `SHUTDOWN{S,R}`; and
- forwarder pair/context abort uses the same close primitive.

`SimGuestOwnerIo::end_socket` invokes that primitive before recording its
owner effect, so a missing socket mutation can no longer be hidden by the
effect log. Datagram slots retain the most recent association V_g handle,
allowing the D-5 read-back after `end_association` clears the live slot record.
Attached application reset still runs its installed callback in the reset
fixture step; unattached captured reset remains the accepted Hangup path.

`SimSocket::peer_shutdown` preserves the first recorded vsock class while
continuing to accumulate the strongest peer shutdown flags. This keeps the
classification fact immutable without suppressing later transitions.

**Q-01 CLOSED.** The concrete false-pass from iteration 8 is no longer
possible: a Close/Abort effect without actual V_g shutdown, or a context
release without actual half reset, leaves the SimSocket projection wrong and
fails conformance.

## Guest TakeEnd and OwnerClose ordering

The accepted fwd-pair action map distinguishes two guest owner steps:

1. `TakeEnd(g)` consumes the forwarder record and removes the flow from the
   owner projection; and
2. `OwnerClose(g)` closes the retained descriptors in the next dispatch.

Production now represents that order directly. `handle_forward_end` queues a
private `DeferredGuestClose` after updating D-11 and removing the flow.
`next_deadline()` returns `now` while that queue is nonempty, and the next
`dispatch_next` applies exactly one deferred stream/datagram close before
polling another event. Multiple closes remain serialized one per owner step.

The guest fwd-pair driver's `TakeEnd(g)` performs only the explicit record
dispatch and no longer settles the pending close behind the model's back.
`OwnerClose(g)` performs the later settle/dispatch. Actions for side `h` no
longer drive guest work.

The existing flow-end cell now pins this production boundary:

- after the released `Completed` record, the first `g.step()` removes the
  flow but observes no linger-clear or close effect and V_g still reports only
  its prior `SHUT_WR`;
- the second `g.step()` clears linger before close and closes exactly both
  halves; and
- application bytes/EOF and the absence of a control message remain exact.

This is an acceptance-body correction to the already-designed two-action
order, not a new scenario or architecture mechanism.

## Evidence

| Check | Result |
|---|---|
| Current fwd-pair named plus random | PASS — 38/38, nextest run `fde4c9d5-a6d2-45a1-b4e7-700b50fdc583` (`quint-fwd-qproj8-pass.log`). |
| Current TakeEnd/OwnerClose focused cell | PASS — reviewer rerun `d5109a1f-43f4-4f6c-b26f-43b7b9eb2e06`; 1/1 after the final `next_deadline` addition. |
| Prior focused record | PASS — `flow-end-deferred-close.log`, 1/1. |
| Forbidden projection bookkeeping search | PASS — no action-derived QPROJ flags or `projected_owner_end`; no `last_pair_state`. |
| Scoped `rustfmt --check` | PASS for driver, owner, sim fixture, flow-end body and `overdrive-sim` socket arbiter. |
| Scoped `git diff --check` | PASS for the same files and this review artifact. |
| New scenario/lane/public API | PASS — none. The deferred close queue and socket read-backs are private implementation/test-support required by the accepted action map. |

## Final finding disposition

| ID | Prior severity | Final disposition |
|---|---|---|
| Q-01 | Blocker | **CLOSED** — every compared socket/queue/reset fact comes from the retained SimSocket arbiter, and the production owner exposes the exact two dispatches the model names. |

## Global DISTILL disposition

Iterations 1–7 approved the TCP-CORRELATION and compatibility bodies.
Iteration 9 separately approved the UDP-slot refusal dependency and the
guest-only dummy prerequisite fixture. This iteration closes iteration 8's
sole global Quint blocker. No reviewed DISTILL finding remains open.

# ITERATION 10 — APPROVED

The full STEP 11-06 DISTILL handoff is approved. The executable
specifications now use exact production/model action boundaries and genuine
observable arbiters; passing conformance no longer depends on driver-authored
expected socket state. The original crafter may continue the active STEP
11-06 DELIVER cycle. This verdict does not claim production GREEN, waive the
dedicated DELIVER reviewer, authorize mutation testing before the final wave
gate, or permit advancement to the next roadmap step before STEP 11-06's
required implementation review returns `APPROVED`.

---

# Iteration 11 — deferred cleanup and final compatibility re-review

## Iteration 11 metadata

| Field | Value |
|---|---|
| Review ID | `NETNS-DENSITY-295-STEP11-06-DISTILL-REVIEW-11` |
| Review date | 2026-10-23 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| Scope | Corrections after iteration 10: honest QPROJ-8 action boundaries, deferred stream/datagram owner close, exact session/UDP cleanup oracles, and regression re-check of the already-approved UDP refusal and disposable-guest fixtures |
| Source/body verdict | **APPROVED** |
| Blocking findings | 0 |
| Full formal-record evidence | **PENDING** — the 279-check `--record` run is still executing; no terminal summary exists yet |

No iteration-10 finding reopened. The current corrections preserve the accepted
`TakeEnd(g)` then `OwnerClose(g)` action boundary for every reviewed stream and
datagram owner-driven abort path, and the Quint projection no longer advances
private guest-owner work during a foreign-side stimulus.

## QPROJ-8 source honesty and action boundaries

The current `quint_fwd_pair.rs` projection derives compared state from the
retained production-simulation arbiters:

- `pr(g).closed` requires the exact consumed record, absence from the owner
  table, and `SimSocket::closed_by_owner()` on retained V_g;
- shutdown, class, vsock state and error facts come from retained V_g;
- stream input/output and delivered bytes come from the retained half sockets;
- reset truth comes from `SimSocket::peer_was_reset()`; and
- phase and record location remain the owner's observation plus the forwarder
  context/pair arbiters.

The forbidden projection caches remain absent: there is no
`owner_*_applied`, `context_release_applied`, `projected_owner_end`, or
`last_pair_state`. `SimSocket::close_from_owner` records the actual local
descriptor close separately from the forwarder's earlier transport shutdown,
retains reset truth after an earlier EOF, suppresses a second local reset after
the peer itself reset, and retains the first recorded vsock class. These are
source-local test-support facts owned by DISTILL; they add no production API or
runtime branch.

The driver's dispatch map is now exact. `TakeEnd(g)` releases one queued record
and calls precisely one `dispatch_next`; it cannot consume the pending private
close. `OwnerClose(g)` is the later owner action. Forwarder runs, socket
stimuli, host-side actions and other foreign-side actions do not call the
guest-owner settling loop. Consequently a pending `OwnerClose(g)` cannot be
silently consumed behind `Hop*`, `VCtl`, host `TakeEnd`, or another foreign
model action. The current all-init run exercises all 38 fwd-pair named/random
tests after this final mapping; the two reproduced owner-close seeds also pass.

## Stream and datagram deferred cleanup

Production's private deferred-close machinery matches the accepted action map:

1. an owner-driven abort stores the exact stream pair or datagram slot cleanup
   under its forwarder id and calls the real forwarder abort;
2. `handle_forward_end` consumes the resulting real record, updates the D-11
   fields and moves that cleanup onto the private dispatch queue;
3. the following `dispatch_next` performs one `OwnerClose`; and
4. `next_deadline()` reports `now` while such a close is pending, so the owner
   cannot strand it while otherwise idle.

The clean stream cell observes the exact intermediate state: `TakeEnd` removes
the flow with no close or linger-clear and retained V_g still open, then the
next dispatch clears linger before closing both halves. The owner-abort cell
requires both descriptors to remain open after the actual `AbortForward` and
to be closed only after the real `Detached` record path. The fwd-pair
conformance trace compares the additional intermediate `TakeEnd` state before
its distinct `OwnerClose`, so an implementation that closes in the record
dispatch fails even though eventual cleanup still occurs.

The UDP and session-loss cells now make the same ordering observable for a
resident slot:

- after `AbortForward`, the slot's owner, association and attached pair remain
  exact and S remains connected;
- the end record is real and retains its accepted outcome, cause, moved counts
  and queue order;
- only the following deferred owner close discards S's queue, disconnects it
  and clears association/attach;
- a retained slot keeps its owner, while an actual release clears it; and
- the session-loss oracle permits only the exact three D-11 consumptions and
  the corresponding association/attach cleanup, including p3's final
  zero-move `Detached` record.

This closes the compatibility gap without weakening QPROJ-5, QPROJ-7,
QPROJ-8, D-11, DG-5-Q, K-A4 or EVMOD. No synthetic record, driver action flag,
seeded production state, polling seam, new scenario, new lane or public
test-support API was introduced.

## Rechecked bounded dependencies

The iteration-9 bounded approvals remain valid:

- UDP `AppClaim` still publishes only the kernel-side parked origin and held
  readiness; QPROJ-5 owner/flow remain absent until `GuestReady`, which
  atomically publishes the owner and new flow id. The current formal safety and
  release-liveness checks both pass.
- The thirteenth UDP-slot cell still exercises the designed
  `GuestSlotAttachRefused { error, vsock }` representation. It verifies the
  exact returned V_g is closed, resident S is discarded/disconnected but keeps
  its owner, the refusing guest context creates no pair or end record, no
  control message escapes, the host pair follows its ordinary one-record path,
  and a later datagram re-associates.
- GuestKit still stages the stock running-kernel dummy module, decompresses it
  only in the fixture work directory when needed, and loads it only from the
  disposable guest's fixture PID 1 before `exec` of production init. Production
  init continues to own forwarder validation/loading and READY. The fixture
  does not mutate the physical host's module state or bypass kernel/module
  identity.

## Verification evidence

| Check | Result |
|---|---|
| Deferred session/UDP owner-close focus | PASS — 4/4, nextest run `15f75eda-a202-4eb2-a2c1-d3436f0cbde9` (`deferred-udp-owner-close-focused.log`). |
| Current complete `overdrive-init` integration-feature suite | PASS — 187/187 with 3 filtered/skipped, nextest run `e3803c56-623c-4fe9-b6bd-333debf9a7d9` (`init-suite-after-deferred-cleanup-oracles.log`). This run post-dates every reviewed fwd-pair, owner, session and UDP source edit. |
| Reproduced owner-close seed | PASS — 1/1, nextest run `4c2010e7-aadd-45eb-a8a8-0dd9705e63bb` (`quint-fwd-seed-112-final.log`). |
| UDP formal safety | PASS — `udp-slots-safety-tlc-final.log`. |
| UDP formal release liveness | PASS — `udp-slots-release-liveness-final.log`. |
| Workspace clippy | PASS — workspace/all-targets/integration-tests with `-D warnings` (`workspace-clippy-final-distill.log`). |
| Scoped whitespace check | PASS — `git diff --check` on every file in this iteration's bounded review. |
| Full guest-flow-owner formal record | **PENDING** — `cargo xtask quint check --subsystem guest-flow-owner --record --jobs 3 --mem-budget 12g` is still running (`quint-record-existing-model-alignment-retry.log`); the observed partial log contains passes but no final result. |

The full `--record` run is a separate formal evidence gate because the Quint
specifications changed. Its pending state does not create a source/body
finding, and this review does not pretend it passed. Global STEP 11-06 evidence
closure must wait for its terminal all-pass summary; a failure in that run
reopens the affected formal compatibility result.

## Iteration 11 disposition

No blocking test-design, test-honesty, contract-shape, API-shape or invariant
finding remains in the reviewed source and executable specifications. All
previously bounded UDP-refusal and GuestKit fixture approvals remain in force.

# ITERATION 11 — SOURCE/BODY APPROVED; FORMAL RECORD EVIDENCE PENDING

The bounded DISTILL corrections are approved. The global evidence gate remains
pending solely on the already-running full guest-flow-owner formal record; the
active workflow must not report that gate complete until the run produces its
terminal all-pass summary.

---

# Iteration 12 — unhooked classifier and native Pending-premise audit

## Iteration 12 metadata

| Field | Value |
|---|---|
| Review ID | `NETNS-DENSITY-295-STEP11-06-DISTILL-REVIEW-12` |
| Review date | 2026-10-23 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| Scope | Two post-iteration-11 changes only: retained-socket classifier honesty and the first completed native-metal run of the approved Fast Open Pending recipe |
| Verdict | **CHANGES_REQUESTED — NATIVE TEST RECIPE BLOCKED** |
| Blocking findings | 1 (`F-06`) |
| Roadmap validation | **PENDING** — mechanically reopened after native falsification of the approved deterministic fixture premise |

This iteration does not reopen the accepted TCP-correlation production
mechanism, the separately approved Quint pending-place model, or any unrelated
STEP 11-06 body. It reopens only the dependent native test recipe and the
roadmap's claim that the authorized one-cell boundary is executable as
specified.

## Unhooked classifier correction

**APPROVED.** `SimSocket::peer_shutdown` still records the actual received
shutdown bits for the retained socket, but it records a first
`recorded_class` only while the forwarder hook is installed. Attach-time
classification still comes from the socket's already-observed shutdown state;
a live hooked callback can still classify its real transition. Once
`finish_item` has removed the hook, a later foreign `VCtl` can strengthen the
socket's received bits but cannot manufacture a classifier result for a pair
that no longer exists.

This restores the accepted QPROJ-8 boundary: `recorded_class` is a retained
arbiter fact produced by the real simulated forwarder lifecycle, not a model
action or projection cache. No driver flag, expected-value snapshot, model
guard or invariant was added or weakened.

Evidence:

- the reproduced seed `0x1942cd2e` passes, nextest run
  `f426a34f-9859-47a5-b6b3-fd8a58f0390e`
  (`quint-fwd-seed-1942-final.log`);
- the complete fwd-pair named/random suite passes 38/38, nextest run
  `707b07a8-06e3-42b6-909f-4e096e99c0b9`
  (`quint-fwd-unhooked-classifier-final.log`); and
- the later complete `overdrive-init` integration-feature run passes 187/187,
  nextest run `7644908d-2c55-480c-881c-e22339c008b6`
  (`init-suite-after-all-latest-sim-and-libc-fixes.log`).

## F-06 — native execution falsifies the deterministic observable-Pending premise

| Field | Value |
|---|---|
| Severity | **Blocker** |
| Affected approval | Iterations 3 and 5's deterministic Fast Open fixture premise; iteration 9's roadmap external-validity/test-boundary conclusion that depends on that recipe |
| Unaffected | The production `Pending -> Ready` mechanism, exact identity contract, no-weakening outcome, separately approved model, and the other native cases |
| Disposition | **OPEN — replace and natively validate the fixture recipe; do not weaken the oracle** |

The accepted cell relies on a blocking `MSG_FASTOPEN` producing a
userspace-observable interval in which all four facts hold together: one real
child key, an owner-held child fd, no returned owner event, and the exact
intake/client tuple still in `TCP_SYN_RECV`. Iterations 3 and 5 treated
`sk_write_pending` and `TCP_DELACK_MAX` as a deterministic source-pinned reason
that this interval would remain observable for the 150 ms bound.

The first production-composed disposable Cloud Hypervisor run disproves that
recipe. During the complete bound it repeatedly observed:

```text
child_entries=1, tcp_state=Some(1), socket_fds=9220, before_owner=9219
```

Linux `/proc/net/tcp` state `1` is `ESTABLISHED`; the required `SYN_RECV` state
is `3`. A separate experimental concurrent ordinary Fast Open producer was
also unable to preserve the state and produced the same exact observation.
That experiment was discarded from the body; the current source contains only
the original sequential recipe and neutral diagnostic values.

The two native failures are concrete production-path evidence:

- `in-guest-correlation-tfo-diagnostic.log` — the sequential approved recipe;
- `in-guest-correlation-concurrent-tfo.log` — the discarded concurrent
  attempt; and
- both execute Linux `7.0.0-38-generic` inside the disposable guest through
  production `Connect4`, `TcpCorrelation` and `KernelGuestOwnerIo`.

The key/fd conjunction is still useful but insufficient. It proves that a
real child-correlation row exists and that production owns one additional
socket. With the tuple already `ESTABLISHED`, it cannot distinguish any of
these current paths:

1. the owner accepted while the row was `Pending`, but userspace observed only
   after the final ACK changed it to `Ready`;
2. the owner accepted only after `Ready`; or
3. a broken implementation charged the shared place only at emission.

Accepting key count plus fd count as “Pending” would revive the passing mutant
rejected in iterations 3–5. Enlarging the 150 ms bound, adding retries or
adding another producer also cannot restore the missing state: the observed
state has already advanced rather than merely arriving late.

The current failure is therefore honest. The diagnostic-only change in
`real_kernel.rs` is neutral and approved, but the native cell remains RED and
STEP 11-06 cannot report GREEN.

## Bounded replacement-fixture feasibility

A faithful replacement does not inherently require a raw-packet exception.
The narrow viable direction is a private guest-local PRIO/NETEM gate over
packets generated by the ordinary TFO application socket:

1. install a root `prio` qdisc on `lo` and attach `netem` only at class `1:3`;
2. set the real TFO application socket's `SO_PRIORITY` to the direct class id
   `0x0001_0003`; Linux copies that `sk_priority` to every IPv4 skb and
   `sch_prio` routes the skb directly when its class major matches the qdisc;
3. start the production owner wait and perform the same blocking
   `MSG_FASTOPEN` with data — only that client socket's SYN and final ACK enter
   NETEM, while the server SYN-ACK and the default-priority overflow socket
   bypass the delayed class;
4. after `MSG_FASTOPEN` returns, require the exact child-key, `SYN_RECV`,
   owner-fd and no-event conjunction, then drive the existing overflow refusal
   during the controlled delayed-ACK interval;
5. delete the root qdisc, which resets the NETEM child and discards its queued
   ACK, then make an ordinary send on the same connected application socket;
   the kernel generates the releasing ACK/data packet through the same
   final-ACK option callbacks, after which the exact one-time event,
   continuity, release and cleanup outcomes already authored must hold; and
6. use an unwind-safe guard to delete the qdisc on every exit, require the
   captured fresh-guest qdisc baseline afterward, and fail the cell if install,
   release or cleanup fails. VM death is the final complement for loaded
   fixture modules and staged files.

This is packet gating, not raw-packet generation: it uses no `AF_PACKET`, raw
socket, constructed TCP header, injected SYN/ACK, seeded BPF state, new BPF
program, production branch, public/test-support API, lane, scenario or model.
It controls delivery of the real kernel-generated final ACK, which is the
state transition the existing oracle needs.

The required kernel/tool substrate exists for the pinned metal kernel but is
not present in the current guest image:

- the completed read-only metal audit confirms `CONFIG_NET_SCH_NETEM=m`, the
  stock matching dependency-free `sch_netem.ko.zst`, and `/usr/sbin/tc`;
- the dedicated `sch_prio` availability command was blocked by the canonical
  metal lease and timed out before producing its config/module lines
  (`tfo-gating-prio-metal-readonly.log`), so the matching
  `CONFIG_NET_SCH_PRIO`/`sch_prio.ko` prerequisite remains a mandatory
  pre-spike check rather than an assumed fact;
- Linux copies `sk_priority` from `SO_PRIORITY` to every IPv4 skb
  (`vendors/linux/net/ipv4/ip_output.c:191,531`), `sch_prio` honors a matching
  direct class id before its fallback priority map
  (`vendors/linux/net/sched/sch_prio.c:34-63`), and `sch_netem` queues to a
  configured send time (`net/sched/sch_netem.c:558-603`);
- the metal host provides `/usr/sbin/tc`; its complete dynamic closure is the
  loader plus libc, libm, libbpf, libelf, libmnl, libcap, libxtables, libz and
  libzstd (`tfo-gating-tc-deps-metal-readonly.log`); and
- the existing GuestKit already has stock-module discovery/decompression,
  guest-only `finit_module`, arbitrary fixture-file staging and dynamic-library
  staging mechanisms.

The delay must be longer than the bounded in-Pending overflow operation, and
the owner wait must be correspondingly longer than the one delayed SYN plus
that observation interval while remaining inside the existing cell/lane wall
clock. That relationship is an explicit fixture bound to validate in the
native spike; the remedy must not rely on merely hoping to win the delay.

These are read-only feasibility facts, not replacement execution evidence.
Before F-06 can close, the chosen gate must be spiked in the same disposable
guest and show all of the following in one bounded run:

- the selected direct qdisc class demonstrably queues the real final ACK;
- the accepted child is simultaneously key-present, owner-held and
  `SYN_RECV`, with no owner event;
- the different-port overflow is refused during that interval;
- releasing the gate produces exact `OutboundCaptured`, after which the
  existing post-emission refusal, abort/replacement and all cleanup complements
  pass; and
- the loopback qdisc, child rows, sockets and bpffs staging paths all return to
  their fresh-guest baselines; the stock fixture modules and staged tool files
  die with the disposable guest, as the existing dummy prerequisite does.

The traffic gate fits the existing one-cell, existing-lane fixture-HOW boundary
and does not relax any user-approved prohibition. Staging the two stock
matching qdisc modules plus the host `tc` binary's exact dynamic closure is a
bounded guest-only fixture dependency, not a production/runtime dependency or
a public test-support API. Under the repository's correctness precedence and
DISTILL ownership of fixture HOW, it is authorized as necessary fallout of the
already-approved one-cell outcome; no raw-packet exception or new user-facing
contract is required. This authorization does not approve an unexecuted
recipe. Its exact implementation and native result still require independent
DISTILL re-review and independent roadmap revalidation before validation may
return to `approved`.

## Iteration 12 evidence and disposition

| Check | Result |
|---|---|
| Unhooked classifier source review | PASS — classification is recorded only at attach or while the actual hook exists. |
| Reproduced classifier seed | PASS — 1/1. |
| Complete fwd-pair suite | PASS — 38/38. |
| Sequential native Fast Open Pending recipe | **FAIL as required evidence** — child key 1 and owner fd +1, but exact tuple `ESTABLISHED`, never observed `SYN_RECV`. |
| Concurrent-producer hypothesis | **FAIL and reverted** — same exact established observation; no body/scope expansion retained. |
| Key/fd accepted as Pending | **REJECTED** — would weaken the oracle and permit the count-at-emission mutant. |
| Replacement traffic gate | FEASIBLE IN PRINCIPLE, **NOT YET EXECUTED** — native spike and independent review required. |
| Current roadmap validation | PENDING — root applied the mandatory native-falsification gate; no contract, DES or production mechanism was changed. |
| Full 279-check formal record | Still independent and in progress; it cannot close this native testability finding. |

# ITERATION 12 — CHANGES_REQUESTED

The SimSocket classifier correction is approved with no QPROJ-8 weakening.
F-06 blocks the native TCP-CORRELATION cell because real production execution
falsifies its deterministic observable-Pending premise. Preserve the failing
`SYN_RECV` oracle. Replace the recipe with a natively proven private
guest-local gate over ordinary kernel-generated TCP traffic, then independently
re-review that exact fixture and revalidate the roadmap. STEP 11-06 remains
non-GREEN while this finding is open.

---

# Iteration 13 — terminal formal evidence and native-gate classification

## Iteration 13 metadata

| Field | Value |
|---|---|
| Review ID | `NETNS-DENSITY-295-STEP11-06-DISTILL-REVIEW-13` |
| Review date | 2026-10-23 |
| Reviewer | Same isolated `nw-acceptance-designer-reviewer` |
| Formal/model verdict | **APPROVED** |
| Classifier/source-group verdict | **APPROVED** |
| Native TCP-correlation verdict | **CHANGES_REQUESTED — F-06 remains open** |
| Roadmap validation | **PENDING** |

This iteration separates three facts that the previous provisional remedy had
conflated:

1. PRIO/NETEM can create and release a real, deterministic Fast Open
   `SYN_RECV` interval using only ordinary socket-generated TCP;
2. the current production correlation implementation does not promote the
   held child after that interval is released; and
3. the complete formal/model evidence is now terminal and clean.

The native failure therefore does not justify weakening the `SYN_RECV` oracle,
discarding the proven traffic gate, or inventing a new DESIGN mechanism before
the current implementation is debugged against the accepted working spike.

## Classifier disposition remains approved

Iteration 12's classifier approval is unchanged. The retained SimSocket
records its first forwarder classification only at attach or while its real
hook exists. A foreign `VCtl` after unhook may still update received shutdown
bits but cannot synthesize `recorded_class`. The exact reproduced seed, the
38/38 fwd-pair suite and the later 187/187 integration-feature suite remain
passing. No projection cache, driver-authored expected state or model weakening
has returned.

## PRIO/NETEM boundary is natively proven

The standalone disposable-Cloud-Hypervisor spike validates the exact traffic
mechanics proposed in iteration 12:

- the real application uses blocking `MSG_FASTOPEN`; no packet is constructed
  or injected;
- direct `SO_PRIORITY` selects the NETEM child under the loopback PRIO qdisc;
- the real Fast Open child remains `SYN_RECV` while the client packet is
  queued;
- default-priority overflow traffic bypasses the selected class;
- deleting the qdisc and performing an ordinary send establishes the held
  connection; and
- qdisc cleanup restores the fresh-guest state.

The successful results are recorded at 5 s, 2 s and 1 s delay bounds. The
supplied `tfo-qdisc-spike-metal-5.log` is an earlier infrastructure failure
before guest execution (`GuestCid` outside its leased range), so it is not
counted as evidence. The corrected successful 5 s run is
`tfo-qdisc-spike-metal-6.log`; the 2 s and 1 s successes are
`tfo-qdisc-spike-metal-2s.log` and `tfo-qdisc-spike-metal-1s.log`. Each
successful log ends with:

```text
VERDICT=WORKS pending=SYN_RECV overflow=bypass release=ordinary-send cleanup=qdisc-delete
```

This closes iteration 12's feasibility uncertainty. The qdisc boundary is a
valid private fixture mechanism within the one approved cell and existing
lane. It adds no raw-packet generator, seeded state, BPF program, production
API, public test-support API, model variable or new scenario.

## Production-composed result is an implementation RED

When the same gate was temporarily folded into the production-composed cell,
it reached the exact strong pre-release witness and the in-Pending capacity
stimulus. After release, however:

- `/proc/net/tcp` reported the exact child tuple as `ESTABLISHED`;
- the child-correlation key remained present;
- production retained the accepted child fd;
- no `OutboundCaptured` event appeared after repeated owner/audit waits; and
- both qdisc deletion plus an ordinary send and timed delivery of the queued
  packet failed to complete the approved `Pending -> Ready -> event` path.

The temporary private-layout diagnostic read the current implementation row as
state `1` (`Pending`) after TCP had become established:

```text
child_entries=1, child_state_debug=Some((12298, 1)),
tcp_state=Some(1), socket_fds=9220, before_owner=9219
```

That read is diagnostic evidence only. It has correctly been removed from the
acceptance body and does not pin the private map representation. The stable
oracle remains public/behavioral: after a proven real `SYN_RECV` hold and gate
release, the same production wait must emit exact `OutboundCaptured` once.

This result is not evidence that the PRIO/NETEM boundary is invalid: the
standalone spike proves its TCP mechanics at three bounds, and the composed run
itself reaches `SYN_RECV`, owner-held fd and the overflow refusal before
release. It is also not yet a DESIGN falsification. The accepted reference
spike exercised Fast Open `Pending -> Ready` with the approved SockOps carrier;
the current production BPF/owner implementation is the first suspect. The
original STEP 11-06 crafter must compare the implementation with that accepted
working spike and fix the proven production path before any new architecture,
testability surface or weakened outcome is proposed.

The temporary qdisc fixture, module/tool staging, spike source and private
debug read were removed from the current body. That cleanup is appropriate for
the private-layout read and scratch spike, but removing the proven deterministic
gate leaves the current acceptance body on the iteration-12-falsified
ungated timing premise. F-06 therefore remains open at DISTILL: the final body
must restore an independently reviewed form of the proven private gate, without
the private-state read, so the production implementation fix is judged against
the strong public oracle.

## Terminal formal/model evidence

The full guest-flow-owner record is approved:

| Evidence field | Verified result |
|---|---|
| Command | `cargo xtask quint check --subsystem guest-flow-owner --record --jobs 3 --mem-budget 12g` |
| Run | `24389-1792766001.740` |
| Interval | `2026-10-23T14:33:21.739793664Z` to `2026-10-23T15:59:56.295217152Z` |
| Result | **279/279 expected outcomes matched**, wall clock 5372.9 s |
| Tool identity | Quint 0.32.0; Apalache 0.56.1 build `70cdaf4`; OpenJDK 21.0.12.1; Linux 7.0.0-38-generic |
| Git identity | `e08e27be7a1c9881fdb3bbb3da42da52cdbad07a`, dirty worktree recorded explicitly |
| Input closure | 24 SHA-256 entries recorded; current `checks.toml`, `owner_flows.qnt`, `udp_slots.qnt` and `fwd_pair.qnt` hashes exactly match the summary |
| Subsystem verification | PASS — `quint-verify-evidence-final.log` |
| Independent reviewer global verification | PASS — `cargo xtask quint verify-evidence` verified `guest-flow-owner`, `probe-count` and `tcp-capture-pending`; every recorded outcome matches its current spec |

The record includes holds and expected-violation hazards rather than treating
every check as a positive invariant. All 279 `expect` values equal their
recorded outcomes. The reviewed D-11 end-record projection, QPROJ-5 owner
projection, QPROJ-7 kernel claim, AppClaim/GuestReady split, QPROJ-8 retained
socket arbiters and deferred OwnerClose action map remain exact in the hashed
inputs and source-honest drivers. The formal pass therefore closes iteration
11's pending evidence gate; it does not mask or resolve the separate native
Fast Open failure.

## Iteration 13 finding disposition

| Finding/group | Final iteration-13 disposition |
|---|---|
| Unhooked SimSocket classifier | **APPROVED.** Honest retained arbiter; seed and suites pass. |
| Guest-flow-owner formal/model record | **APPROVED.** 279/279, subsystem verification pass, global three-subsystem verification pass. |
| QPROJ-5/QPROJ-7/D-11 compatibility corrections | **APPROVED.** Hash-closed formal evidence and previously reviewed driver/body evidence agree. |
| PRIO/NETEM fixture mechanics | **APPROVED AS THE DETERMINISTIC TEST BOUNDARY.** Three successful native bounds; no raw packet or new interface. |
| Current production-composed Fast Open path | **RED / crafter remediation required.** Exact Pending witness succeeds; release reaches TCP ESTABLISHED but no correlation event. |
| F-06 | **OPEN.** Restore the proven gate without private-layout inspection, retain the exact public oracle, and rerun after the original crafter fixes the implementation. |
| Roadmap | **PENDING.** It may be independently revalidated only after the final gated body and native result are reviewed. |

# ITERATION 13 — FORMAL/SOURCE GROUP APPROVED; NATIVE F-06 OPEN

The classifier, Quint compatibility work and complete formal evidence are
approved. STEP 11-06 remains non-GREEN solely on the native TCP-correlation
path. The next action is implementation debugging against the accepted working
spike and the proven deterministic PRIO/NETEM boundary, followed by restoration
and independent review of the gated acceptance body. Do not accept
key/fd/`ESTABLISHED` as Pending, remove the capacity oracle, add a public
test-support API, or route an unproven implementation defect into new DESIGN.

---

# Iteration 14 — handshake-only seven-program/build correction review

## Iteration 14 metadata

| Field | Value |
|---|---|
| Review ID | `NETNS-DENSITY-295-STEP11-06-DISTILL-REVIEW-14` |
| Review date | 2026-10-23 |
| Reviewer | Fresh independent DISTILL acceptance-test reviewer for the post-candidate fold |
| Canonical design basis | Commit `74e6eca8f2f356873bdcbb93aac72256aa6d5e2b`; approved `feature-delta.md` TCP-CORRELATION section; ADR-0038, ADR-0150 and ADR-0159 |
| Authored scope reviewed | Candidate corrections in `transport/tests.rs`; existing assertion bodies in `xtask/src/guest_kit_prepare.rs` and `xtask/tests/integration/dev_setup_bpf_linker.rs` |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 1 |

This review is bounded to the post-design-fold DISTILL correction. The already
reviewed parameterized TCP-CORRELATION body, PRIO/NETEM fixture, thirteenth UDP
slot cell, formal/model inputs and native-lane counts are unchanged. No new
scenario, cell, lane, model, public test seam, mutation run, KASAN run or host
kernel effect is required.

The shared worktree also contains original-crafter marker removals and a
production `guest_kit_prepare::run` edit. They are outside this acceptance-body
review and receive no disposition here. This iteration neither adopts nor
reverts them.

## Canonical contract applied

The approved design pins these observable setup/build obligations:

- exact READY order `TcpCorrelation`, `TcpCorrelationCandidate`, `Connect4`,
  `Sendmsg4`, `Getpeername4`, `Recvmsg4`, `SockRelease`;
- labels `tcp_correlation` and `tcp_correlation_candidate`;
- one crate-private `prepare_guest_programs()` call after all intake binds and
  before any program load or attach;
- preparation of both embedded objects and temporary ACTIVE/CHILD staging,
  followed by seven loads and seven attaches;
- active/Rust/shared-staging failures attributed to
  `ProgramLoad(TcpCorrelation, EbpfError)`; candidate object/map/CO-RE/BTF/
  symbol/prototype/FEntry-load failures attributed to
  `ProgramLoad(TcpCorrelationCandidate, EbpfError)`; and candidate attach
  attributed to `ProgramAttach(TcpCorrelationCandidate, ProgramError)`;
- exact stable candidate artifact, build-script export, Clang/LLVM 21,
  `bpftool`, libbpf headers, target-kernel BTF, `-g -O2 -target bpf` and
  x86/arm64 target selection at the existing build/setup surfaces; and
- guest-kit preparation against the same kernel/BTF shipped by that kit.

These are interface and operator/build outcomes, so DISTILL may pin them. The
design does not require those outcomes to be proved by searching particular
Rust/YAML source files for disconnected string literals.

## Approved transport group

The `transport/tests.rs` correction is **APPROVED** within this iteration.

| Contract | Evidence in the authored body | Assessment |
|---|---|---|
| Seven-program identity and order | `ALL_PROGRAMS` lists the exact seven variants in canonical order (`transport/tests.rs:62-70`); `GuestProgram::ALL` is compared against it and labels are zipped against the exact lower-snake-case sequence (`:461-478`). | Pass. |
| Preparation position and cardinality | `expected_order` places one `PrepareGuestPrograms` after all 1,024 UDP binds and before the load/attach loop (`:266-282`); the clean-run oracle compares the entire call vector, exact count `6 + 1_024 + 1 + 14 + 3`, and explicitly orders preparation before first load (`:706-730`). | Pass. One exact call and all fourteen load/attach calls are observed. |
| Stop-on-first-failure | The finite error matrix includes both correlation programs' load/attach boundaries and compares the complete prefix through the failing step (`:763-844`). | Pass. No later setup operation can be hidden. |
| Preparation typed identity | The scripted preparation port maps Rust/active-side failure to `TcpCorrelation` and candidate-object failure to `TcpCorrelationCandidate`, preserving `EbpfError::NoBTF`; both must stop before any load/attach (`:844-875`). | Pass. Exact existing envelopes, no string-only semantic. |
| Candidate load/attach typed identity | Candidate load and attach cases match the precise `ProgramLoad`/`ProgramAttach` variants and program identity (`:781-797`); display cases pin the approved labels (`:340-355`). | Pass. |
| Test boundary | The body invokes production `configure_guest_transport` through its crate-private `GuestTransportOps` port and uses a recording adapter. | Pass. It asserts the composition/setup contract rather than private loader fields or map layout. |

The transport RED is honest. The supplied Lima log fails on the absent approved
`prepare_guest_programs` trait member, `TcpCorrelationCandidate` enum variant
and six-versus-seven `ALL` shape. It reports no fixture or unrelated semantic
failure.

## Build assertion review

The two build tests name the correct contract terms, but their observation
method does not prove the contract:

- `the_artefact_paths_are_the_pinned_contract` reads `xtask/src/main.rs` and
  `overdrive-init/build.rs` with `include_str!`, then checks that artifact,
  environment, tool, flag and architecture substrings occur anywhere in those
  files (`xtask/src/guest_kit_prepare.rs:282-306`); and
- `dev_setup_plans_bpf_linker_install_with_locked_when_missing` keeps using the
  real pure `plan(&ProbeContext)` only for the old Rust/bpf-linker assertion,
  then reads `dev_setup.rs`, the Lima YAML and CI YAML as strings and checks
  four substrings anywhere in each file
  (`xtask/tests/integration/dev_setup_bpf_linker.rs:40-80`).

The supplied build RED is mechanically scoped—five selected tests run, three
pass and these two fail—but the two failures are source-presence failures, not
failures of candidate artifact production, command construction, tool probing,
provisioning or build-script export.

## Finding I14-F01 — source-string assertions neither prove the approved build behavior nor preserve private HOW freedom

| Field | Value |
|---|---|
| Severity | **Blocker** |
| Kind | Test honesty / hexagonal-boundary violation |
| Files | `xtask/src/guest_kit_prepare.rs:282-306`; `xtask/tests/integration/dev_setup_bpf_linker.rs:70-80` |
| Disposition | **OPEN** |

A comment or unreachable string containing
`overdrive_guest_tcp_correlation_candidate_bpf.o`, `clang-21`, `bpftool`,
`libbpf` and `/sys/kernel/btf/vmlinux` makes these assertions green without:

- `cargo xtask bpf-build` generating `vmlinux.h` or the candidate object;
- applying the required flags or target macro;
- `overdrive-init/build.rs` actually tracking/exporting the object;
- `guest-kit-prepare` invoking the build against the kit kernel/BTF;
- `dev-setup` probing or planning any C/CO-RE prerequisite; or
- the Lima/CI BPF-build path installing or validating those prerequisites.

Conversely, a conforming implementation may move the bpf-build command planner
from `xtask/src/main.rs` into a private module, or make Lima and CI invoke one
shared provisioning script. The approved CLI, artifacts, exact commands and
provisioned tools would remain identical, yet the tests fail because the
literal is no longer duplicated in the selected file. That makes file-local
source placement part of the acceptance contract even though DESIGN leaves it
to the crafter.

The dev-setup test is especially misleading: its `ProbeContext` and `Plan`
contain only the original bpf-linker/nightly/rust-src states. The new C/CO-RE
assertion does not drive a missing Clang, bpftool, libbpf-header or BTF state
through `plan`; it only proves that each source/config document contains four
words. A `dev-setup` implementation that returns an empty plan while every new
requirement is absent can pass after those words are placed in comments.

### Required bounded remediation

Replace the source-substring portions of these same existing bodies with
assertions over the approved behavior:

- the effective bpf-build command/plan contains the exact target BTF input,
  `bpftool` generation, Clang-21 flags/architecture selection and stable output;
- the build-script contract actually requires/tracks both artifacts and exports
  the approved candidate path;
- the guest-kit preparation plan invokes that build before compiling/staging
  the guest binary against its kernel/BTF; and
- missing C/CO-RE prerequisites are represented in the effective dev-setup,
  Lima and CI provisioning paths, with their actual command/config association
  checked rather than global substring presence.

This can be done through source-local pure planners, parsed declarative config,
or the existing command-runner test boundary. The reviewer does not prescribe
which private helper or module owns it. Do not add a new scenario, lane, public
API or production test branch; correct the existing bodies and any private
test-support shape they need. Continue asserting the exact design-pinned
artifact names, environment name, tools, flags and architecture values—the
finding rejects AST/file-placement coupling and false-positive observation,
not those approved contracts.

## Mandate and completeness disposition

| Check | Result |
|---|---|
| Hexagonal/observable boundary | **FAIL for the build group** — source files are inspected as implementation artifacts instead of driving the build/setup behavior. Transport group passes. |
| Domain language | Pass — candidate/program/preparation/artifact/tool terms are the accepted infrastructure ubiquitous language. |
| Contract Shape declarations | Pass — live authored tests carry the required declaration. |
| Typed error contract | Pass for transport — every new semantic setup distinction uses the approved existing envelope and exact program identity. |
| Scenario density | Pass — no new scenario/cell/lane/model was added; the existing matrix remains unchanged. |
| PBT/parametrize discipline | Pass — the correction is a finite exact order/error/build contract; no slow-path generative expansion is warranted. |
| Test honesty | **Fail** — the two build tests can pass without the designed effects and reject conforming private refactors. |

The canonical C1-C7 scenario inventory is unchanged by this neutral correction.
No missing boundary or error class justifies additional tests. The blocker is
how two existing assertions observe already-approved C6/C7 build/setup
requirements.

## Iteration 14 disposition

| Group | Verdict |
|---|---|
| Seven-program order, labels and call count | **APPROVED** |
| `prepare_guest_programs` placement, stop behavior and typed mapping | **APPROVED** |
| Candidate load/attach display and typed identity | **APPROVED** |
| Existing native/formal/Tier-3/UDP bodies | **UNCHANGED; NOT REOPENED** |
| Candidate artifact/tooling assertions | **CHANGES_REQUESTED — I14-F01** |

# ITERATION 14 — CHANGES_REQUESTED

The transport correction is exact and ready. The build correction is blocked
only by I14-F01: replace source-string/AST presence checks with honest
assertions over the approved build, embedding, guest-kit and provisioning
behaviors. No broader DISTILL coverage or architecture change is required.
STEP 11-06 must not resume DELIVER until this bounded remediation is
independently re-reviewed and approved.

---

# Iteration 15 — I14-F01 remediation re-review

## Iteration 15 metadata

| Field | Value |
|---|---|
| Review ID | `NETNS-DENSITY-295-STEP11-06-DISTILL-REVIEW-15` |
| Review date | 2026-10-23 |
| Reviewer | Same independent DISTILL reviewer |
| Scope | Only I14-F01 remediation in the two existing build/setup test bodies and their private fixtures |
| Transport verdict | **APPROVED — unchanged from iteration 14** |
| Build/setup verdict | **CHANGES_REQUESTED** |
| Blocking findings | 1 |

No production feature, canonical design, TCP-CORRELATION/UDP body, lane, model,
public runtime API, DES state or commit is reviewed or changed here. The three
private shell fixtures and `CCoreProbe`/`ProbeContext` compiler fallout are
within the authorized test-support boundary.

## I14-F01 remediation disposition

The remediation removes every source-global `include_str!().contains(...)`
assertion identified in iteration 14. Most of I14-F01 is closed:

| Required observation | Remediated evidence | Assessment |
|---|---|---|
| Build script requires both embedded objects | `the_artefact_paths_are_the_pinned_contract` compiles and executes the real `overdrive-init/build.rs`; guest-only input must fail, then both objects must succeed (`guest_kit_prepare.rs:286-329`). | **Pass.** The test observes executable behavior rather than text presence. |
| Build script tracks and exports exact paths | The successful run requires exact `cargo:rerun-if-changed` and `cargo:rustc-env` directives for both artifacts (`:330-348`). | **Pass.** Exact approved names and paths are observable output. |
| Real bpf-build entry point and source/header causality | The integration body launches the real `xtask bpf-build` binary in a private workspace. Fake rustup produces only the pre-existing Rust artifacts; fake bpftool produces `vmlinux.h`; fake Clang refuses unless it receives the candidate source, generated header, libbpf headers and output (`dev_setup_bpf_linker.rs:181-270`; fixture scripts). | **Pass subject to I15-F01's exact-argv defect.** This is command/plumbing evidence only; the separate native receipt remains the compiler/verifier evidence. |
| Stable artifacts | The real CLI must write exact host, guest and candidate stable outputs with distinct fixture contents (`:236-246`). | **Pass.** A stale/prewritten candidate cannot satisfy the path because the private workspace starts empty. |
| Guest-kit invokes bpf-build before later setup | The body removes the candidate, runs real `guest-kit-prepare`, and requires recreation before the deliberately incomplete workspace reaches its later failure (`:273-282`). | **Pass.** This proves actual entry-point order without a production branch. |
| Lima and CI association | A source-local decoder selects only Lima `provision` and CI `jobs.bpf-build`, reads executable `script`/`run` blocks, ignores comments and reconstructs continued commands before checking provisioning (`:58-179`). | **Pass for the current declarative surfaces.** Comments and unrelated jobs cannot satisfy it. |
| Test isolation | All tools are private executable fixtures under `xtask/tests/integration/fixtures`; no install, compiler, BPF load, qdisc or host-kernel mutation occurs. | **Pass.** No dependency or public runtime seam was added. |

The supplied RED is semantic and correctly bounded at its first observable
failures: five selected tests execute; three pass; dev-setup lacks a C-tool
plan and the real build script accepts a missing candidate. These are current
implementation omissions, not collection, fixture or parser failures.

## Finding I15-F01 — two remaining oracles can pass without the exact C/CO-RE contract

| Field | Value |
|---|---|
| Severity | **Blocker** |
| Kind | Test honesty / decision causality |
| Files | `xtask/tests/integration/dev_setup_bpf_linker.rs:284-354`; `:236-269` |
| Disposition | **OPEN** |

### A. `CCoreProbe` does not have to affect the plan

The only missing-C input sets all four C/CO-RE prerequisites absent **and** sets
`bpf_linker_on_path = false` (`dev_setup_bpf_linker.rs:289-300`). The opposite
row sets bpf-linker and every C prerequisite present (`:361-372`). Therefore a
production planner can ignore `ProbeContext::c_core` completely and append the
Clang/bpftool/libbpf/BTF commands only inside its existing
`if !bpf_linker_on_path` branch. Every new assertion passes, but a developer
who already has bpf-linker and lacks bpftool, libbpf headers, readable target
BTF or exact Clang 21 receives an empty plan.

The `clang_major: Option<u32>` boundary is also unexercised: `None` and exact
`Some(21)` appear, but an installed wrong major such as `Some(20)` may be
accepted as satisfied. The canonical contract requires major 21, not merely
some Clang.

This is a direct reachable dev-setup failure, not a hypothetical internal
schedule. `run()` feeds the real probe into `plan`; on an existing developer
machine bpf-linker can be present independently of any newly added C/CO-RE
prerequisite.

### B. The exact `-target bpf` argv is not actually asserted

The Clang log assertion checks each required string with
`observed.contains(argument)` across the entire tab-separated line
(`dev_setup_bpf_linker.rs:261-267`). The candidate source path itself contains
`overdrive-bpf`, so it satisfies the required string `"bpf"` even when the
argument following `-target` is absent or is `wasm`. The fake Clang fixture
checks source/header/output causality but does not reject a wrong/missing `-g`,
`-O2`, `-target bpf` or architecture argument. A command with
`-target wasm`, the correct source path and the architecture macro therefore
produces the candidate and passes the current oracle.

That is precisely the approved build contract the fake-tool layer is meant to
prove. It does not require a real compiler claim: the tab-delimited invocation
already exposes exact argv.

### Required bounded remediation

Keep the same existing test body and private fixtures:

1. Drive each C prerequisite while bpf-linker/nightly/rust-src and the other C
   prerequisites are satisfied, or use an equivalent compact decision table.
   Include a wrong non-21 Clang major. Each state must cause the effective plan
   to handle that exact missing requirement. This may be a loop/table inside
   the existing test; no new scenario is required.
2. Parse the fake Clang log into exact argv tokens and require the exact
   `-g`, `-O2`, adjacent `-target`, `bpf`, correct architecture macro, candidate
   source, include inputs and output relation. Alternatively make the private
   fake compiler reject any wrong shape. Do not infer the target value from a
   source-path substring.

No native compilation, new dependency, production API, new test function,
scenario, lane or model is required. The real native receipt remains the
separate compiler/verifier evidence layer.

## Iteration 15 mandate disposition

| Check | Result |
|---|---|
| Original source-string/AST blocker | Substantially closed — real entry points and structured executable config replaced global text presence. |
| Refactor freedom | Pass for bpf-build/build-script implementation internals; exact designed outputs/commands remain pinned. |
| Actual entry-point wiring | Pass — real build script, `xtask bpf-build` and `guest-kit-prepare` execute. |
| Source/header causality | Pass — fake tools require candidate source, generated BTF header and libbpf headers. |
| Exact argv | **Fail** — substring matching does not prove `-target bpf`. |
| Decision-table causality | **Fail** — C prerequisite handling can be accidentally conditioned on missing bpf-linker; wrong Clang major is uncovered. |
| No fake-native claim | Pass — fixture validates plumbing only and relies on the frozen native receipt for real compilation/verifier behavior. |
| Scope | Pass — same tests, private fixtures, neutral `ProbeContext` compiler fallout, no new product surface. |

## Iteration 15 disposition

| Group | Verdict |
|---|---|
| Transport correction | **APPROVED** |
| Real build-script behavior | **APPROVED** |
| Real bpf-build/guest-kit entry-point composition | **APPROVED subject to exact-argv remediation** |
| Structured Lima/CI executable configuration checks | **APPROVED** |
| Dev-setup C-prerequisite decision causality | **CHANGES_REQUESTED — I15-F01.A** |
| Exact candidate compile argv | **CHANGES_REQUESTED — I15-F01.B** |

# ITERATION 15 — CHANGES_REQUESTED

I14-F01's source-text problem is removed, and the remediation now exercises the
correct real boundaries. I15-F01 is the sole remaining blocker: make the
existing body prove that each `CCoreProbe` fact independently drives the plan
and that the logged compiler argv contains exact `-target bpf` tokens. No
broader test or design work is needed.

---

# Iteration 16 — I15-F01 terminal re-review

## Iteration 16 metadata

| Field | Value |
|---|---|
| Review ID | `NETNS-DENSITY-295-STEP11-06-DISTILL-REVIEW-16` |
| Review date | 2026-10-23 |
| Reviewer | Same independent DISTILL reviewer |
| Scope | Only the completed I15-F01 remediation in the existing dev-setup/build body and private fake Clang fixture |
| Verdict | **APPROVED** |
| Open findings | 0 |

## I15-F01 disposition

| Finding part | Remediation evidence | Disposition |
|---|---|---|
| I15-F01.A — C prerequisite handling could depend on missing bpf-linker | The original bpf-linker row now carries a complete C state (`dev_setup_bpf_linker.rs:334-368`). A compact five-row table then keeps bpf-linker, nightly, rust-src and every unrelated C prerequisite satisfied while varying only: Clang absent, Clang major 20, bpftool absent, libbpf headers absent, or target BTF unreadable (`:370-398`). Every row forbids reinstalling present bpf-linker and requires its own effective C handling. | **CLOSED.** `CCoreProbe` must independently drive the plan, and exact Clang-major mismatch is covered. |
| I15-F01.B — substring matching could accept a wrong target | The command log is split into exact tab-delimited argv tokens. `-g`, `-O2` and the exact architecture token must each exist, and the token immediately after exact `-target` must equal `bpf`; the candidate source is an exact suffix-bearing argv token (`:292-315`). The private fake compiler separately refuses missing/wrong debug, optimization, target, architecture, source, generated-header, libbpf-header or output inputs (`fixtures/fake_clang_21.sh`). | **CLOSED.** A source path can no longer supply the target value, and `-target wasm` cannot create the fixture artifact. |

The correction remains in the same existing test function and private fixtures.
It adds no test function, acceptance scenario, lane, model, dependency, public
runtime API, production test branch or native execution claim.

## Evidence assessment

The supplied Lima RED runs the same five selected tests: three pass and two
fail. The dev-setup failure occurs on the first isolated state with
`bpf_linker_on_path = true` and only Clang unavailable:

```text
Clang is not independently handled by the plan: []
```

The other failure remains the real build script accepting a missing candidate
object. These are the intended current implementation omissions. Test
collection, private tool fixtures, exact-argv decoding, config decoding and
entry-point launch all succeed before those assertions. The RED therefore does
not disguise a fixture or compilation failure as product semantics.

Author-provided mechanical evidence also passes:

- `cargo fmt`;
- scoped diff check; and
- Lima xtask all-target Clippy with warnings denied.

The real compiler/verifier claim remains supplied only by the frozen disposable
Cloud Hypervisor native receipt. The private fake compiler proves command,
input and artifact plumbing and makes no stronger claim.

## Final correction disposition

| Reviewed group | Final verdict |
|---|---|
| Seven-program order, labels, exact one preparation and fourteen load/attach calls | **APPROVED** |
| Preparation/load/attach typed identities and stop behavior | **APPROVED** |
| Real build-script two-object require/track/export behavior | **APPROVED** |
| Real bpf-build stable artifacts, BTF generation and exact C argv | **APPROVED** |
| Real guest-kit-before-later-setup composition | **APPROVED** |
| Independent dev-setup C prerequisite planning, including Clang 20 | **APPROVED** |
| Structured Lima/CI executable provisioning association | **APPROVED** |
| Existing TCP-CORRELATION, UDP, formal/model and native bodies | **UNCHANGED; PRIOR APPROVAL PRESERVED** |

# ITERATION 16 — APPROVED

All iteration-14 and iteration-15 findings are closed. The authored DISTILL
correction matches the approved handshake-only seventh-program, C/CO-RE build,
two-object preparation and typed setup contracts with honest behavioral RED
evidence and no coverage expansion.

**DISTILL GATE SATISFIED.** The original STEP 11-06 crafter may resume against
these reviewed bodies. Expected REDs for the absent approved production
variant, preparation method, C prerequisite plan, candidate artifact/build
contract and related implementation are legitimate implementation work; no
collection or fixture blocker remains.

---

# Iteration 17 — native production-init fixture correction review

## Iteration 17 metadata

| Field | Value |
|---|---|
| Review ID | `NETNS-DENSITY-295-STEP11-06-DISTILL-REVIEW-17` |
| Review date | 2026-10-23 |
| Reviewer | Same independent DISTILL reviewer |
| Scope | Fixed pre-cut beacon compatibility, production-init fixture PID 1 prerequisites, V24 raw poll observation and behavior-neutral failure-matrix helper split |
| Verdict | **APPROVED** |
| Open DISTILL findings | 0 |
| Production result | **HONEST RED — `bind_inbound_vsock` returns `EINVAL` before READY; original crafter owns it** |

This iteration reviews only the source-applied native test-support correction.
It does not approve the failing production implementation, pull STEP 12-01
wiring forward, modify a public product interface, or add a scenario, cell,
lane or model.

## Fixture corrections

### Pre-cut beacon compatibility

The current production init still dials the fixed pre-12-01 beacon port. The
lane now binds and retains that exact listener through private
`bind_pre_cut_beacon` in both `run_in_guest_cell` and `boot_to_ready`
(`overdrive-testing/src/guest_lane.rs:250-252,291-294,405-408`). The dynamic
`HostListeners` beacon remains separately bound, is still written into the
seven-field token, and is retained as `_advertised_beacon`. Stream and control
also remain kernel-assigned.

This is the smallest compatibility fixture for the current production entry
point. It does not rewrite the token to the fixed port or claim the STEP 12-01
attachment cut has landed. The advertised GUEST-PORTS contract remains exact,
while the fixed listener exists solely so the pre-cut binary can reach its
current bootstrap boundary. The host-kernel-shared lane already serializes the
node-global vsock resource.

`native-precut-beacon-missing-module.log` runs the existing missing-forwarder
cell 1/1 PASS. That proves the lane reaches the production missing-module
refusal rather than dying on an absent fixed beacon.

### Production init remains PID 1 while the fixture supplies kernel prerequisites

For a `GuestInit::OverdriveInit` image, the test-only peer is installed as the
initial PID 1 and the real production binary is staged separately. A private
kernel parameter selects that path. PID 1 mounts the guest filesystems, brings
loopback up, loads the ordinary vsock transport modules, raises
`RLIMIT_NOFILE` to 65,536, conditionally loads only the staged stock dummy and
vsock-loopback fixture modules, and calls `exec` on production init
(`overdrive-testing/bin/guest_peer.rs:660-705`).

`exec` preserves PID 1 and the raised file-descriptor limit. Production init
therefore remains the actual process under test and still owns:

- transport-token parsing and guest transport setup;
- `ovd_fwd` release validation and loading;
- BPF object/map/program preparation;
- program attachment; and
- READY or typed pre-READY refusal.

The wrapper does not call a production initialization function, inject a
success result, pre-bind an intake, load the forwarder module, or branch inside
production. Its module loads are conditional on files explicitly staged in the
disposable image. VM destruction remains the cleanup complement.

The file-descriptor limit is necessary for the approved native density
boundary: 1,024 UDP slots plus the 4,096 combined TCP-place witness and their
application/captured socket pairs exceed the ordinary fixture ceiling. The
sequence of native results proves causality rather than hiding production
failures:

| Evidence | Furthest boundary reached | Assessment |
|---|---|---|
| `native-beacon-fixture-diagnostic.log` | Production reaches UDP-slot setup and fails around slot 1,020 with `EMFILE`. | Fixed beacon and dummy prerequisite work; the inherited fd ceiling is a fixture defect. |
| `native-beacon-nofile-v24-final.log` | Production passes the slot/fd boundary, loads `ovd_fwd`, then its forwarder `StreamRoundTrip` probe fails. | Raising `RLIMIT_NOFILE` is effective; missing loopback transport is the next fixture prerequisite. |
| `native-production-init-fixture-prereqs.log` | Production loads `ovd_fwd`, passes its prerequisites, and fails at `bind_inbound_vsock` with `EINVAL` before READY. | The wrapper now exposes a real production transport defect. The fixture no longer supplies the failure. |

The last log is intentionally RED. Requiring READY here would conflate DISTILL
fixture validity with the original crafter's implementation responsibility.

### V24 observes the real Linux readiness bits

The existing V24 cell keeps its exact two accept orders and its original
sequence:

1. the fixed request read succeeds;
2. the following read returns EOF;
3. the accepted V_g reports `POLLRDHUP`; and
4. the first `MSG_NOSIGNAL` write fails `EPIPE`.

Nix 0.30's typed `PollFd::revents` path drops Linux's undeclared
`POLLRDHUP` bit. The source-local witness now calls libc `poll` over one live
borrowed fd and reconstructs `PollFlags` with `from_bits_retain`
(`overdrive-init/src/guest_flow/real_kernel.rs:1245-1273`). The change exposes
the kernel result; it does not replace `POLLRDHUP` with `POLLHUP`, weaken the
EOF/EPIPE assertions, add a timing retry, or synthesize readiness.

`native-beacon-v24-fixture-fix-2.log` executes the V24 and production-init
wrappers together. V24 is the one passing cell; the other cell reaches its
separate production pre-READY failure. This is valid independent native proof
of the unchanged V24 outcome.

### Failure-matrix helper split

The source-only failure-matrix refactor moves repeated fixture work into
private helpers while retaining the same case set, inputs and assertions. It
adds no case, removes no outcome and creates no runtime seam. Formatting and
scoped diff checks pass.

## Scope and honesty audit

| Check | Result |
|---|---|
| Production composition | Pass — the wrapper `exec`s the real binary as PID 1; it does not reproduce production setup. |
| Pre-12-01 behavior | Pass — fixed beacon is retained only for current bootstrap; dynamic GUEST-PORTS remain advertised and held. |
| Module ownership | Pass — the wrapper loads stock dummy/loopback fixture prerequisites; production still validates/loads `ovd_fwd`. |
| Resource realism | Pass — the raised fd limit permits the already-approved density case; it does not seed sockets, maps or state. |
| V24 oracle | Pass — exact EOF + `POLLRDHUP` + first-write `EPIPE` is preserved from raw kernel revents. |
| Scenario/lane count | Pass — existing cells and the existing disposable guest lane only. |
| Public surface | Pass — changes are private or overdrive-testing fixture support; no product API, wire, persistence, model or owner event changes. |
| Native effects | Pass — all module/socket/resource effects occur inside disposable Cloud Hypervisor guests on Metal. |
| Production GREEN claim | None — final evidence is explicitly a production `EINVAL` RED. |

# ITERATION 17 — APPROVED

The fixed beacon, fd-limit inheritance, staged dummy/loopback loading, PID 1
`exec` wrapper, raw V24 revents and private helper split are necessary,
bounded and behaviorally honest fixture corrections. They preserve every
accepted outcome and expose the next failure through the real production entry
point.

The native DISTILL fixture is approved. The production-init
`bind_inbound_vsock` `EINVAL` remains a separate STEP 11-06 implementation RED
for the original crafter; it is not a DISTILL blocker and must not be repaired
by weakening READY, V24, GUEST-PORTS or the existing native scenarios.

---

# Iteration 18 — GUEST-TRANSPARENT-ROUTE DISTILL alignment review

## Iteration 18 metadata

| Field | Value |
|---|---|
| Review ID | `NETNS-DENSITY-295-STEP11-06-DISTILL-REVIEW-18` |
| Review date | 2026-10-24 |
| Reviewer | Independent DISTILL acceptance-test reviewer |
| Design basis | Commit `34588a68d64fa25c31b9f8c2905791cb9739fdca`; approved feature delta, ADR-0150, ADR-0174 and STEP 11-06 roadmap |
| Authored scope | Existing `transport/tests.rs` and `transport/real_kernel.rs` bodies only |
| Verdict | **APPROVED** |
| Open findings | 0 |
| Execution state | Compiler prerequisite only; semantic RED/GREEN remains for the original crafter |

The independently approved design has no open finding and requires no further
user decision. This iteration reviews source-contract alignment only. Current
production intentionally lacks the approved trait methods and typed variants,
so the authored bodies cannot yet compile. That is the correct Outside-In
prerequisite and is not misreported as a semantic RED.

## Source-local setup contract

The recording adapter adds exactly the two approved operations:

- `add_fwmark_route()` records `FwmarkRoute`; and
- `enable_tcp_fwmark_accept()` records `TcpFwmarkAccept`
  (`transport/tests.rs:198-208`).

`expected_order` is exact:

```text
DummyDevice
AddressAdd
FwmarkRule
FwmarkRoute
TcpFwmarkAccept
FwmarkReflect
ReservedPorts
TCP intake
UDP slots ascending
program preparation/load/attach
forwarder module/context
inbound listener
```

The clean-run body compares the complete vector and exact count
`8 + 1_024 + 1 + 14 + 3` (`transport/tests.rs:727-747`). Thus the route and
accepted-child mark inheritance each occur once, after the rule, before mark
reflection, before every intake/program/READY prerequisite. Existing
TCP-CORRELATION preparation and seven-program order remain unchanged.

The finite first-failure matrix contains each route stage independently and
compares the full call prefix through the failing operation
(`transport/tests.rs:752-780,886-901`). A failure at Rule cannot execute Route;
a failure at Route cannot execute either sysctl; a failure at
`tcp_fwmark_accept` cannot execute `fwmark_reflect`; and every failure prevents
all later setup. The private helper split preserves the same cases and oracle
while keeping the one existing scenario.

## Typed failure mapping and display

The authored plain-data cases pin all four exact representations and strings:

| Setup stage | Typed representation | Display |
|---|---|---|
| Priority/full-mask mark rule | `FwmarkRule { source }` | `adding the fwmark lookup rule failed: {source}` |
| Table-295 local route | `FwmarkRoute { source }` | `adding the fwmark local route failed: {source}` |
| Accepted-child mark inheritance | `TcpFwmarkAccept { source }` | `setting net.ipv4.tcp_fwmark_accept failed: {source}` |
| Kernel reply reflection | `FwmarkReflect { source }` | `setting net.ipv4.fwmark_reflect failed: {source}` |

Evidence is `transport/tests.rs:328-341` plus the first-failure matchers at
`:756-759`. No generic `io::Error` string substitutes for a semantic variant,
and no new public error carrier is introduced.

## Real-kernel observable contract

The existing Tier-3 transport cell now uses a private route-netlink dump parser
rather than `ip` command text (`transport/real_kernel.rs:63-133`). It reads the
kernel's structured IPv4 rule and route objects and requires:

- exactly one rule matching priority 32,765 or mark `0x295`;
- IPv4, zero source/destination prefix selectors and table-lookup action;
- priority `32765`;
- mark `0x295`;
- mask `0xffffffff`;
- lookup table `295`;
- exactly one IPv4 route in table 295;
- destination prefix `0/0`;
- protocol `boot`;
- scope `host`;
- type `local`; and
- output interface equal to the running guest's `lo` ifindex
  (`transport/real_kernel.rs:135-176`).

The same existing test then reads and requires both live sysctls in contract
order:

```text
net.ipv4.tcp_fwmark_accept = 1
net.ipv4.fwmark_reflect = 1
```

It retains every prior postcondition: workload address local, reserved-port
range exact, TCP and all 1,024 UDP intakes held, forwarder module loaded and
referenced, device present, and inbound vsock listener held
(`transport/real_kernel.rs:179-235`). The owner positive-control flow and
in-guest wrapper are unchanged, so the eventual native run still proves the
route by the real production path rather than a source-local substitute.

The netlink helper is test-local and read-only. It opens `NETLINK_ROUTE`, sends
`RTM_GETRULE`/`RTM_GETROUTE` dump requests, parses aligned message/attribute
lengths, and fails on a netlink error. It neither installs a route nor seeds
state.

## Honesty, scope and execution disposition

| Check | Result |
|---|---|
| Exact design shape | Pass — mark, mask, priority, table, local-default route, protocol, scope, device and both sysctls match the approved contract. |
| Order/cardinality | Pass — full-vector equality pins Rule → Route → TcpFwmarkAccept → Reflect and the new exact call count. |
| Closed typed mapping | Pass — four distinct setup failures and displays; first failure stops later work. |
| Observable boundary | Pass — source-local setup uses the production composition function; Tier-3 reads real kernel rtnetlink/sysctl state. |
| Assertion strength | Pass — no command-output substring, stubbed route, relaxed mask, alternate table or weakened route type is accepted. |
| Existing positive control | Preserved — owner flow/native wrapper unchanged. |
| Scenario density | Pass — no new test, scenario, lane or model. |
| Public surface | Pass — tests name only the already-approved crate-private trait/error additions; private netlink helpers add no API. |
| Cleanup/lifecycle | Preserved — setup remains VM-netns owned and any pre-READY failure ends the VM/netns. |
| Current compiler failure | Expected prerequisite — production declarations are absent; no semantic RED or GREEN is claimed. |

Author-provided formatting and scoped diff checks pass. No native rerun is
required at DISTILL source-approval time: the final crafter must first add the
approved production methods/variants, then compile and execute the complete
real test set before claiming GREEN or committing. The earlier independent
native contrast establishes necessity and the exact successful route; it does
not replace that final production-composed execution.

# ITERATION 18 — APPROVED

The GUEST-TRANSPARENT-ROUTE DISTILL alignment exactly captures the approved
setup order, cardinality, typed failure set, display strings and real-kernel
route/sysctl postconditions. It adds no scenario, lane, model, public API or
assertion weakening.

**DISTILL SOURCE GATE SATISFIED.** The original STEP 11-06 crafter may implement
the approved production declarations and behavior. Actual GREEN remains gated
on compilation plus the complete source-local and native production test run;
this approval does not convert the current compiler prerequisite into semantic
evidence.

---

# Iteration 19 — R5-28 application-observer ordering correction

## Iteration 19 metadata

| Field | Value |
|---|---|
| Review ID | `NETNS-DENSITY-295-STEP11-06-DISTILL-REVIEW-19` |
| Review date | 2026-10-24 |
| Reviewer | Same independent DISTILL reviewer |
| Scope | Existing R5-28 real-kernel cell and its existing in-guest-lane wrapper only |
| Verdict | **APPROVED** |
| Open findings | 0 |

## Reproduced fixture race

`guest-app-observer-race-diagnostic-native.log` proves the failed reset oracle
read the wrong application connection:

- the production owner reset the aborted flow's application socket whose peer
  port was `42751`;
- while the guest cell was still settling and collecting accepted application
  sockets, the host opened the positive-control flow;
- `App::take_all()` then returned the live positive-control socket whose peer
  port was `37291`, although both connections shared the application listener
  at port `8081`;
- that fd was blocking (`F_GETFL == 2`, `O_NONBLOCK` false), had no pending
  `SO_ERROR`, and returned `EAGAIN` only after the 1.054 s read timeout.

The failure was therefore an exact fixture-observer race. It did not show that
the aborted application connection remained open or that production failed to
reset it.

## Ordering correction

The guest cell retains its original order through every R5-28 outcome:

1. wait until the target flow is accepted;
2. tell the host it may close V_h;
3. observe the host close;
4. run the owner until the aborted flow leaves both accepted and live-flow
   projections;
5. settle for the original bound;
6. take all application connections then present and require each to be
   `(0, Reset)`;
7. require the target absent from both owner projections;
8. preserve the exact attached-pair/consumed-end-record arithmetic and require
   `ForwardOutcome::Aborted` whenever a pair existed.

Only after those assertions complete does the guest emit the already-existing
private `owner_lane::PAUSED` line
(`guest_flow/real_kernel.rs:1386-1435`). The host wrapper now waits for that
line before calling its unchanged `serve_control_flow`
(`tests/integration/in_guest_lane.rs:351-383`). Consequently the positive
control cannot enter the application's accepted-C set while the abort oracle
is selecting its sockets.

The barrier does not dispatch the owner, change the close, extend the settle
bound, filter sockets by expected outcome, discard a live connection, or move
an assertion after the positive control. It separates two existing narrative
phases at their natural boundary.

## Diagnostic behavior

`app_end` retains its successful branches exactly:

- `read == 0` returns `Eof`;
- received bytes remain counted; and
- `ConnectionReset` returns `Reset`.

Only the already-failing `Other` branch now records fd, local/peer tuples,
`F_GETFL` status flags, nonblocking state, `SO_ERROR` and elapsed read time
(`guest_flow/real_kernel.rs:1047-1108`). `SO_ERROR` is read only after the
required Reset/EOF oracle has failed, so it cannot consume the error that a
successful reset assertion must observe. The raw `F_GETFL` call is read-only
on a live fd and has a source-local safety justification.

## Evidence and scope

| Check | Result |
|---|---|
| Native causality diagnostic | Pass — distinct peer ports prove positive-control substitution, not an aborted-socket defect. |
| Focused disposable-guest rerun | Pass — `guest-app-observer-order-native.log`, 1/1. |
| Reset oracle | Unchanged — zero bytes and `AppEnd::Reset`. |
| Owner state oracle | Unchanged — target absent from accepted and flows after the original bound. |
| End-record oracle | Unchanged — one consumed abort per attached pair, none if unattached. |
| Positive control | Unchanged — same `serve_control_flow` behavior, opened only after PAUSED. |
| Scenarios/lanes | Unchanged — existing source-local cell and existing wrapper. |
| API/model | Unchanged — existing private PAUSED fixture token; no production or public surface. |
| Formatting/diff | Pass. |
| Guest-kit preparation | Pass. |
| Clippy | Not claimed — the attempted run was blocked by concurrent unrelated production lints. |

No unrelated production correction is required for this fixture review, and no
mutation or broader native suite is part of this bounded disposition.

# ITERATION 19 — APPROVED

The R5-28 correction is necessary, causally grounded and assertion-preserving.
It prevents the later live positive control from being mistaken for the
aborted application connection while retaining every reset, state, end-record
and positive-control outcome. The focused disposable Cloud Hypervisor run
passes 1/1. No new scenario, lane, model, API or weakened assertion is added.

---

# Iteration 20 — fwd-pair observational stimulus-handler review

## Iteration 20 metadata

| Field | Value |
|---|---|
| Review ID | `NETNS-DENSITY-295-STEP11-06-DISTILL-REVIEW-20` |
| Review date | 2026-10-24 |
| Reviewer | Same independent DISTILL reviewer |
| Scope | `ReadEof`, `HolderDrop`, `TrailingRst` and `HolderEnd` handlers in the existing `quint_fwd_pair` conformance driver |
| Verdict | **APPROVED** |
| Open findings | 0 |

## Handler assessment

The four formerly empty actions are observations or foreign-side stimuli at the
guest conformance boundary. They do not authorize a second guest owner action,
a synthetic socket transition or consumption of private deferred work. The
correct driver behavior is consequently a validated observational stutter.

| Action | Exact validation | Guest-side effect | Assessment |
|---|---|---|---|
| `ReadEof(d)` | Decodes the real `d` field, rejects datagram mode, and decodes `SPEC_PRE.dir[d].q2` as the approved integer-unit queue (`quint_fwd_pair.rs:843-854`). Unknown directions or malformed/missing queue state fail decoding. | None. The read observation's EOF/reset fact is already present in retained socket arbiters; no owner dispatch is released. | Pass. |
| `HolderDrop` | Requires watch mode and decodes the approved watch direction's pre-state `gh.q2` (`:867-875`). Stream/datagram traces are rejected as not enabled. | None. The holder is outside this guest-side driver; no guest socket close or owner action is invented. | Pass. |
| `TrailingRst(s)` | Decodes exact side `g` or `h`, maps it to the corresponding inbound direction (`hg` or `gh`), rejects any other value, then decodes that direction's pre-state `q2` (`:877-884`). | None. This is the post-completion far-end reset; the ended guest pair is not resurrected or rerun. | Pass. |
| `HolderEnd(k)` | Requires watch mode, accepts only exact `eof` or `err`, and decodes `SPEC_PRE.gh.q2` (`:909-920`). | None. It records no driver flag and performs no read that could become a projection source. | Pass. |

`pre_dir_list` is not a constant ceremonial expression. It requires the
stashed previous spec state to exist, resolves the action's direction in the
qualified `dir` map, resolves the exact queue field, requires a list and
decodes every unit as an integer (`quint_fwd_pair.rs:208-234`). The direction
or value carried by the trace therefore participates in the handler and an
impossible mode, side, end kind, missing direction or malformed queue fails
closed.

These handlers appropriately do not re-evaluate every Quint guard. The trace
producer owns model enablement; the driver's responsibility is to map the
trace's concrete stimulus at the production boundary. For these observational
steps, changing guest state would be less honest than a validated stutter.

## Preservation of the approved conformance boundary

The handlers:

- do not call `settle`;
- do not invoke `GuestFlowOwner`;
- do not mutate a guest `SimSocket`;
- do not call the forwarder;
- do not consume a queued end record;
- do not dispatch private deferred `OwnerClose` work;
- do not add driver bookkeeping used by `from_driver`; and
- do not read `SPEC_PRE` from `from_driver` or otherwise turn model state into
  projection state.

Thus iterations 7–13 remain intact: QPROJ-8 still comes only from retained
SimSocket/forwarder arbiter facts, and `TakeEnd` followed by a distinct
`OwnerClose` remains two action boundaries. No action flag, expected-state
cache or projection source was added.

## Evidence

| Check | Result |
|---|---|
| `cargo xtask conformance-lint` on Metal | Pass — `conformance-lint-stimulus-handlers-metal.log`. |
| Focused fwd-pair named/random suite | Pass — 38/38 in `quint-fwd-pair-stimulus-handlers-with-toolchain-metal.log`. |
| Targeted Clippy with warnings denied | Pass — `clippy-quint-fwd-pair-stimulus-handlers-metal.log`. |
| Temporary Quint tooling | Removed — `/tmp/netns-density-295-quint` is absent after the run. |
| Formal model inputs | Unchanged — no rerecord is required; the prior 279/279 evidence remains the model layer. |

The earlier 0/38 attempt had no Quint executable and did not execute semantic
cases. It is correctly excluded from result evidence; the checksum-pinned
provisioned-tool run is the authoritative 38/38 result.

## Scope audit

| Check | Result |
|---|---|
| Existing actions only | Pass — no new run, action or scenario. |
| Model/spec changes | None. |
| Production behavior | None — conformance driver source only. |
| Projection weakening | None — all prior exact comparisons remain. |
| Driver-created state | None. |
| New lane/API | None. |
| Mutation/formal rerecord | Not required for this bounded source correction. |

# ITERATION 20 — APPROVED

The four handlers now perform meaningful, action-specific stimulus validation
without inventing guest-side effects. They close the hollow-handler lint gap
while preserving QPROJ-8 retained-arbiter projections and the distinct
`TakeEnd -> OwnerClose` ordering. Conformance lint, all 38 focused cases and
targeted Clippy pass.

---

# Iteration 21 — DELIVER-review F2/F4 DISTILL remediation review

## Iteration 21 metadata

| Field | Value |
|---|---|
| Review ID | `NETNS-DENSITY-295-STEP11-06-DISTILL-REVIEW-21` |
| Review date | 2026-10-24 |
| Reviewer | Same independent DISTILL reviewer |
| Scope | DELIVER review F2/F4 test-support and existing-cell remediation, plus neutral existing CI assertions |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 1 |

No new acceptance scenario, lane, model, production accessor, public API or
assertion weakening is reviewed or authorized. Prior iteration approvals remain
incorporated.

## F2 socket-diagnostic fixture disposition

The F2 remediation is **APPROVED**.

`GuestKit::image` now stages the supported stock kernel's exact
`inet_diag.ko`, `tcp_diag.ko` and `udp_diag.ko` modules at private fixture paths
in dependency order (`overdrive-testing/src/guest_kit.rs:91-95,493-496`). The
existing test-only PID 1 loader conditionally loads dummy, loopback and those
three staged diagnostic modules before `exec` of production init
(`overdrive-testing/bin/guest_peer.rs:668-718`). The scripted-peer path uses
the same diagnostic module list.

This supplies the already-accepted guest-image prerequisite; it does not add a
runtime capability API, recovery subsystem, owner branch or production
accessor. Production init remains PID 1 after `exec` and the production owner
still issues the real inet-diag requests.

The retained increment-B native artifact proves the effect through the default
workload kit:

- production reaches READY;
- all 1,024 initial UDP slot round trips echo;
- the next 32 first sends are legitimately refused with `EPERM` while their old
  owners still hold the slots;
- each subsequent retry echoes; and
- `RECYCLE_RESULT lost=0/32`.

That is stronger than module-presence inspection and closes the reproduced
`NLMSG_ERROR(-ENOENT)` prerequisite omission.

## F4 approved subgroups

The existing parameterized cell remains one six-case table and the wrapper
count/lane are unchanged (`guest_flow/real_kernel.rs:947-978,1202-1308`). The
following additions are **APPROVED**:

### Live direct Fast Open rejection

`DirectFastOpenRejected` uses a live ordinary application socket and real
`MSG_FASTOPEN`, verifies that no active metadata exists, identifies the real
child row, waits for terminal `Rejected`, requires zero fabricated identity,
and requires the candidate program's kernel `run_count` to increase
(`:980-1005`). Before owner consumption it sends established data and requires:

- candidate `run_count` unchanged;
- the exact terminal row unchanged;
- no capture event;
- production reset; and
- final row absence/no replay (`:1006-1025`).

Together the real candidate invocation, missing active metadata and terminal
zero-identity rejection observe the approved
`CandidateRejected -> Rejected` path without requiring an unreliable
userspace sample of the lock-serialized transient itself.

### Candidate program identity and invocation count

The source-local map/program observer identifies exactly one tracing program
whose kernel-truncated name is `tcp_correlation` and whose map IDs include the
exact discovered 8,194-entry child map (`:214-261`). `enable_stats(RunTime)` is
called before discovery, and `candidate_run_count` reopens the exact program ID
for each observation (`:341-347,1298-1300`). The count therefore comes from the
running production BPF program, not a fixture counter.

### Terminal/delete observations after classification

For a valid Fast Open connection the cell observes `Pending`, then exact
terminal `Ready` metadata and a candidate invocation. It sends 32 established
bytes and requires zero additional candidate invocations plus byte-identical
terminal state. Owner capture must delete the row once; a later send must not
invoke the candidate or recreate the deleted key; active storage must also
become absent (`:1028-1083`). These assertions honestly cover the post-terminal
and post-delete states, subject to I21-F01's missing overlap.

### Preterminal reset/delete finality

The reset case observes the exact real child key in `Pending`, the matching TCP
connection in `SYN_RECV`, no owner event, and exactly one extra owner-held fd
before reset (`:1085-1114`). It then restores the application's default qdisc
class, performs abortive close, drives the production owner while forbidding a
capture, requires deletion, releases the delayed gate and rechecks no row/no
replay (`:1116-1131`).

The test killed a real defect: before the original crafter's private cleanup
fix, the native RED retained `CandidateReady` after reset; the corrected
existing wrapper now passes 1/1 in
`review-f4-correlation-native-green.log` (57.643 s). This is meaningful
mutation-by-history evidence for cleanup finality.

### Preserved earlier contract

The same table still proves ordinary identity, immediate application release,
forced syncookie traversal, real data-in-SYN `Pending -> Ready`, the shared
4,096 boundary, replacement admission, one-time consumption, active storage
cleanup, child-map baseline and staging-pin absence. None of those oracles was
relaxed.

## Finding I21-F01 — accepted parallel established-send overlap is still absent

| Field | Value |
|---|---|
| Severity | **Blocker** |
| Kind | Existing-cell coverage gap |
| File | `crates/overdrive-init/src/guest_flow/real_kernel.rs:1028-1068` |
| Disposition | **OPEN** |

The accepted STEP 11-06 criterion requires:

> A PRIO/NETEM-delayed real final ACK with **parallel established sends** and
> preterminal reset-close proves child-lock-serialized terminal monotonicity
> and BPF_EXIST deletion finality.

The current terminal helper does not create that overlap:

1. it sends Fast Open and waits first for `Pending`;
2. it then waits synchronously until the row is already terminal `Ready`;
3. only after `Ready` does it restore/remove the gate; and
4. only then does `send_probes(32)` issue 32 sequential sends on the same
   thread (`guest_flow/real_kernel.rs:1033-1058`; `send_probes` at `:464-468`).

The resulting assertions prove that ordinary established data after promotion
is a no-op. They cannot kill an implementation that is correct after `Ready`
but regresses/replaces the row when established input overlaps the delayed
final-ACK candidate/promotion boundary. The passing native result executes this
same non-overlapping sequence, so it does not fill the gap.

### Required bounded remediation

Within the same helper and existing cell, overlap an ordinary-socket send
cohort with release/processing of the real delayed final ACK, then join the
sender and retain the existing exact assertions:

- the candidate invocation count changes only for the handshake candidate,
  not the established sends;
- terminal `Ready` metadata remains exact;
- the row cannot regress or be replaced;
- one-time deletion remains final; and
- all sockets/gate state are cleaned.

A scoped thread/duplicated fd using the same real application socket is
sufficient. Do not add a cell, lane, raw-packet path, seeded state, production
seam or timing lottery. Preserve the current sequential post-terminal/delete
checks if they continue to add distinct evidence.

## Neutral CI assertion disposition

The existing xtask body now checks the current intended bpftool provisioning
shape: concrete-package detection, fallback to running-kernel tools when the
package is virtual, final tool resolution, and a target-BTF dump smoke command.
This is **APPROVED as source-contract alignment for the current plan**. The
mocked 4/4 xtask result proves command/config plumbing only. It is not evidence
that Ubuntu 24.04 Noble's real package resolver accepts the final plan.

The original crafter's separate Noble validation remains mandatory. If that
real environment requires a different valid package/fallback path, production
and this same assertion body must be reconciled together before the final
DELIVER review. This DISTILL review neither accepts a known-bad package plan
nor requires the body to preserve one.

## Mechanical evidence

| Check | Result |
|---|---|
| Relevant init/testing/xtask Clippy | Pass. |
| Existing xtask focused suite | Pass — 4/4. |
| Formatting/scoped diff | Pass. |
| F4 focused disposable guest | Pass — 1/1, 57.643 s, but does not close I21-F01. |
| Formal/model inputs | Unchanged; no rerecord required. |
| Mutation | Not run; final wave gate only. |

# ITERATION 21 — CHANGES_REQUESTED

F2 is approved. Direct live rejection, production run-count observation,
post-terminal/delete assertions and preterminal reset/delete cleanup are
approved. The only blocker is I21-F01: the accepted parallel established-send
overlap is not exercised because all 32 sends begin after `Ready`. Close that
gap inside the same helper/cell, then return for bounded re-review.

---

# Iteration 22 — I21-F01 parallel-overlap re-review

## Iteration 22 metadata

| Field | Value |
|---|---|
| Review ID | `NETNS-DENSITY-295-STEP11-06-DISTILL-REVIEW-22` |
| Review date | 2026-10-24 |
| Reviewer | Same independent DISTILL reviewer |
| Scope | I21-F01 remediation in the same terminal/delete helper and existing parameterized cell; aligned existing bpftool fallback assertions |
| Verdict | **APPROVED** |
| Open findings | 0 |

## I21-F01 disposition

I21-F01 is **CLOSED**.

After observing the real child row as exact `Pending`, the helper starts one
scoped sender over the same production application socket
(`guest_flow/real_kernel.rs:1048-1072`). The thread performs real
`send(MSG_NOSIGNAL)` calls and publishes progress only after each syscall
returns one written byte.

The main thread requires the first completed send within 250 ms and then
requires the child row to remain byte-identical `Pending`
(`:1074-1085`). This is direct evidence that an application send completed
while the real Fast Open final ACK remained behind the PRIO/NETEM gate; the
thread's existence alone cannot satisfy the oracle.

The same sender continues while the main thread:

1. removes the gate;
2. waits for the child row to reach terminal `Ready`; and
3. samples accumulated send progress (`:1087-1093`).

Only after `Ready` has been observed does the main thread send the synchronization
message. The sender receives it, performs one additional real send, publishes
that progress and returns. Joining must report a count greater than the count
visible when `Ready` was observed (`:1094-1101`). The cohort therefore spans
both sides of promotion; it is not a nominal background thread or a fixture
flag.

The production candidate program's kernel `run_count` must equal exactly
`candidate_before + 1` across the complete spanning cohort
(`:1102-1109`). That one invocation is the gated Fast Open final-ACK candidate;
none of the established sends may dispatch the handshake hook.

## Preserved distinct milestones

The remediation retains the separate post-promotion and post-deletion checks:

- after the spanning cohort, 32 further sequential established sends require
  zero candidate-count delta and the exact terminal row unchanged;
- owner consumption deletes the row once;
- a later established send requires zero candidate-count delta;
- deleted state cannot reappear;
- active socket storage becomes absent; and
- abort/no-replay cleanup remains exact.

Thus overlap evidence does not replace the simpler terminal/delete complements;
both are exercised.

The approved shared-quota Fast Open target remains a separate connection in
`fast_open_capture_through_owner_pending`. Its producer still sends no
post-handshake application payload before exact `OutboundCaptured`. The new
send cohort belongs only to the dedicated monotonicity case and does not weaken
the quota/identity/SYN_RECV evidence.

## Native and mechanical evidence

| Check | Result |
|---|---|
| Existing native wrapper | Pass — 1/1 in `i21-f01-correlation-overlap-native.log`, 33.099 s. |
| Actual Pending overlap | Pass — first completed real send precedes the exact Pending-row recheck. |
| Same cohort across Ready | Pass — sender starts before gate release and must send again only after Ready synchronization. |
| Candidate invocation count | Pass — exact `+1` across all spanning sends. |
| Retained terminal/delete oracles | Pass — source retains zero-delta, byte-identical row and no-recreation checks. |
| overdrive-init all-target Clippy | Pass — `i21-f01-overdrive-init-clippy-metal.log`. |
| Scenario/lane/API/model | Unchanged. |
| Mutation/formal rerecord | Not required for this bounded correction. |

## F2/F4 final disposition

| Group | Verdict |
|---|---|
| F2 stock socket-diagnostic staging/loading and default-kit native proof | **APPROVED** |
| Live direct Fast Open `CandidateRejected -> Rejected` and reset | **APPROVED** |
| Production candidate program discovery/run-count observation | **APPROVED** |
| Parallel established sends across delayed-ACK promotion | **APPROVED** |
| Post-terminal zero invocation and row monotonicity | **APPROVED** |
| One-time deletion/post-delete zero invocation/no recreation | **APPROVED** |
| Preterminal held-child reset/delete finality | **APPROVED** |
| Existing identity, syncookie, 4,096 quota, pin and storage cleanup | **APPROVED; unchanged** |

## Neutral bpftool fallback assertions

The existing body is aligned with the current crafter plan: common tools plus
generic kernel tools, concrete `bpftool` when available, otherwise resolution
of an executable `/usr/lib/linux-tools-*/bpftool` into
`/usr/local/bin/bpftool`, followed by command/version checks and a dump of the
declared readable target BTF. The focused xtask suite passes 4/4 and targeted
Clippy passes (`i21-bpftool-fallback-tests-metal.log`,
`i21-bpftool-fallback-clippy-metal.log`).

These are command/config plumbing assertions. They do not prove Ubuntu 24.04
Noble's real apt virtual-package behavior. The original crafter's actual Noble
receipt remains mandatory before final GREEN/commit; any necessary final
production-path adjustment must keep this same body aligned. This limitation
does not reopen the approved F2/F4 test contract.

# ITERATION 22 — APPROVED

I21-F01 is closed. The same real application-socket sender demonstrably makes
progress while the child is `Pending`, spans gate release and terminal
promotion, and progresses again after `Ready`; only the final ACK increments
the production candidate count. All separate terminal, deletion, storage,
quota and cleanup assertions remain.

The F2/F4 DISTILL remediation is approved with no new scenario, lane, model,
API or assertion weakening. Actual Noble package validation remains a separate
final implementation/CI evidence obligation and is not inferred from mocked
4/4 tests.
