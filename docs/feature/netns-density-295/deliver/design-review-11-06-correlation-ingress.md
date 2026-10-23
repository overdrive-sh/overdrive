# TCP-CORRELATION-INGRESS / STEP 11-06 independent DESIGN review

## Review metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CORRELATION-INGRESS-STEP11-06-ARCH-REVIEW-01` |
| Date | 2026-10-23 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | Native falsification of the Fast Open SockOps consumer and the proposed seventh root-cgroup ingress program, candidate lifecycle, typed failures, boundary impact, bounded fallout and reconciled STEP 11-06 roadmap |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 2 high |
| Non-blocking findings | 1 medium |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** |

The review read the replacement handoff, feature/ADR/roadmap diff, the old
production RED, the rewritten findings, the complete scratch sources, the
required disposable Cloud Hypervisor guest log, the corroborating Lima log,
Linux 7.0 receive/state/cgroup paths, and the accepted no-weakening and
ownership contracts. It changed no production source, test, model, DES,
roadmap, ADR, feature delta, spike or kernel state.

## Native falsification disposition

The native-falsification gate is correctly applied. The approved claim that a
Fast Open child parses its final ACK through
`BPF_SOCK_OPS_PARSE_HDR_OPT_CB` is invalid:

- `bpf_skops_parse_hdr` returns for `TCP_SYN_RECV`
  (`net/ipv4/tcp_input.c:147-179`);
- Fast Open validates the final ACK while still `TCP_SYN_RECV` and transitions
  only afterward (`tcp_input.c:7228-7289`); and
- the production guest-kit run retained a live established child and owner fd
  with one `Pending` row and no event.

The old standalone Fast Open result was a false proof because it dropped the
producer before waiting for `Ready`; FIN/later established traffic could drive
the parser. The proposal correctly keeps only that spike's independently valid
active producer, ordinary/syncookie consumer and storage-loader results.

The replacement path itself is natively established. In the required
disposable x86_64 Cloud Hypervisor guest on Metal, Linux
`7.0.0-38-generic`, a live-open data-in-SYN producer with no post-handshake
payload reached `Ready`, and a live-open direct producer reached `Rejected`.
The run used the exact child cookie at root-cgroup ingress and ended
`VERDICT=WORKS`; aarch64 Lima corroborated it. No physical-host BPF/cgroup
state, raw packet producer or seeded child row was used.

## Technical strengths

The proposed replacement is otherwise tightly scoped:

- SockOps remains the active option producer and ordinary/syncookie consumer;
- ingress observes the Fast Open ACK before validation, then the existing
  SockOps state callback alone promotes a candidate after validation;
- every child state occupies the same child-cookie row, so the 8,194-entry
  universe and combined 4,096-place count do not grow;
- the owner holds all nonterminal states and requires both terminal `Ready`
  and fd `TCP_ESTABLISHED` before emission;
- `Rejected` resets/closes before any owner flow or control record;
- the active socket-storage capacity, staging-pin lifecycle, option ABI and
  cleanup complements remain unchanged; and
- the seventh program reuses existing maps and typed setup envelopes, with no
  public/cross-crate type, host wire, persistence, owner event or abstract model
  state.

The Linux ordering is the right one: `tcp_filter(child, skb)` invokes cgroup
ingress before `bh_lock_sock_nested` and before the Fast Open child state
machine, while `tcp_set_state` invokes the state callback after ACK validation
and before storing `TCP_ESTABLISHED` (`tcp_ipv4.c:2292-2324`;
`tcp.c:2997-3065`). The owner's explicit fd-state check correctly handles a
wake that races the state store.

## Findings

### F-01 — candidate updates are not proven monotonic against terminal promotion or release

- **Severity:** High.
- **Claim:** The contract requires packets after a terminal row to be no-ops
  and requires `sock_release` deletion to be final. Cgroup ingress runs before
  the child socket lock, so two packets—or ingress and owner close—can overlap.
  The executed scratch implementation performs a classifiable lookup and then
  an unconditional `BPF_ANY` replacement. A stale ingress invocation can
  therefore overwrite `Ready`/`Rejected` with a candidate, or recreate a row
  after classification/`sock_release` deleted it.
- **Source path:** Linux invokes `tcp_filter` at `tcp_ipv4.c:2299` and takes the
  child socket lock only at `:2315`. The BPF source reads a classifiable state
  at `bpf/src/main.rs:423-430`, then later calls `CHILD.insert(..., 0)` at
  `:434-440`. The state callback independently inserts terminal state at
  `:391-404`. `inet_release` invokes the cgroup `sock_release` hook before TCP
  close (`net/ipv4/af_inet.c:419-444`), so an owner close can also interleave
  with the pre-lock ingress path.
- **Reachable ordering:** A valid final ACK and immediate subsequent ACK-bearing
  application data may enter cgroup ingress on different CPUs before either
  acquires the child lock. One invocation can read `CandidateReady`, stall, and
  replace a `Ready` written by the first packet's validated state transition.
  The second packet then processes with the socket already established, so no
  later `SYN_RECV -> ESTABLISHED` promotion repairs the regression. Similarly,
  an accepted Pending child closed for quota/end can have its deleted row
  recreated by an ingress invocation that passed the earlier lookup.
- **Evidence limit:** The successful native case intentionally sent no
  post-handshake payload and did not close before `Ready`. It proves the hook
  and single-packet ordering but does not prove the proposal's terminal-no-op
  and exact-release claims under this reachable overlap.
- **Required revision:** Pin and prove a monotonic candidate/terminal/release
  transition in which an ingress observation cannot overwrite terminal state
  or recreate a deleted row. The solution architect owns the synchronization
  choice; this review does not prescribe a lock, generation or map primitive.
  Use bounded real-producer/source evidence without raw packets or seeded child
  state. Do not add a map, capacity, timer, retry, lane, public seam or abstract
  model state unless separately authorized and shown necessary.
- **Disposition:** Open. The current single-packet native receipt is
  insufficient for the exact cleanup/lifecycle claim.

### F-02 — the seventh program is a new per-packet mechanism outside the approved six-program contract

- **Severity:** High (authority gate).
- **Claim:** `GuestProgram::TcpCorrelationIngress` is not a private helper. It
  changes the exact READY attachment set from six to seven and attaches
  `CgroupSkb` at the guest root cgroup. The kernel invokes that program for
  every ingress packet to an eligible socket in the cgroup, even though the
  program returns quickly for non-candidate cookies. The earlier user approval
  named the sixth SockOps program and did not approve this per-packet hook.
- **Boundary assessment:** The proposal preserves no-payload, no-packet-
  mutation/drop, no-sockmap and no-parking: only bounded IPv4/TCP
  headers/options are read for a Pending/candidate row, the program always
  allows, and payload is neither read nor moved. It does **not** preserve an
  absence of BPF from the steady packet path. Every ingress invocation performs
  at least BPF dispatch, socket-cookie acquisition and a child-map lookup before
  the common no-op return.
- **Practical limit:** The Metal/Lima receipts prove correctness of the
  handshake path, not throughput, CPU, tail latency or packets-per-second cost.
  No hot-path performance claim can be derived from those logs. The heavier
  bounded header scan/map replacement occurs only for Pending/candidate rows,
  but the base invocation/lookup cost applies to all ingress packets covered by
  the root-cgroup attachment.
- **Authority:** `TCP-CORRELATION-NO-WEAKEN` authorizes designing the smallest
  invariant-preserving replacement after falsification; it does not
  self-approve a seventh enum variant, new attach type or the per-packet
  boundary change. The proposal correctly leaves the decision index and
  roadmap pending, but its no-weakening audit must disclose this mechanism/cost
  rather than claim every accepted boundary remains unchanged.
- **Required revision:** After F-01 is resolved, present the exact seventh
  program and per-packet practical limit for explicit user approval. Do not
  implement, update DISTILL or mark the roadmap approved before that decision.
- **Disposition:** Open pending user authority; this reviewer cannot waive it.

### F-03 — ADR-0150 contains a duplicated, incomplete Fast Open sentence

- **Severity:** Medium.
- **Claim:** ADR-0150's decision says “a second guest-only” and immediately
  starts “Every passive row…”, then repeats “A second guest-only CgroupSkb…”.
  The accepted tuple gate and ingress-program sentence are syntactically
  interleaved.
- **Evidence:** ADR-0150 lines 117-122.
- **Required revision:** Remove the incomplete duplicate and retain one exact
  sequence: passive establishment creates `Pending` for the gated intake child;
  direct ordinary intake becomes `Rejected`; the single
  `TcpCorrelationIngress` program records the Fast Open candidate.
- **Disposition:** Open, non-blocking alone.

## Candidate, identity and cleanup assessment

| Concern | Assessment |
|---|---|
| Exact child identity | Pass. Linux rebinds `skb->sk` to the selected full child and the native run observed its exact socket cookie. |
| Tuple isolation | Pass at contract level. Passive row creation is gated to full IPv4 children at the configured workload address/intake port; other listeners create no row. |
| Metadata identity | Pass. The unchanged 18-byte option carries cookie and destination together; `CandidateReady` keeps them in the same child row. |
| Validation ordering | Pass for one packet. Ingress observes before validation; only `SYN_RECV -> ESTABLISHED` promotes. Invalid ACKs have no state promotion. |
| Multiple observations | Blocked by F-01. Candidate replacement is necessary for a later valid ACK but must be monotonic against terminal/delete transitions. |
| Owner early wake | Pass. `Ready` plus fd `TCP_ESTABLISHED` is required; a pre-store wake leaves the fd held for the next poll/audit scan. |
| Runtime rejection | Pass after a terminal `Rejected`: linger-reset close, release the combined place, no event/flow/control. |
| Socket cleanup | Contract is correct, evidence incomplete under the ingress/release race in F-01. |

## Typed-error mapping

| Designed failure | Exact representation | Assessment |
|---|---|---|
| Ingress object/program load | Existing `ProgramLoad { program: GuestProgram::TcpCorrelationIngress, source }` | Complete. |
| Root-cgroup ingress attach | Existing `ProgramAttach { program: GuestProgram::TcpCorrelationIngress, source }` | Complete. |
| Existing SockOps storage/staging | Existing `TcpCorrelation` `ProgramLoad` / `ProgramAttach`, preserving `EbpfError::MapError` | Unchanged. |
| Intake listen/Fast Open setup | Existing `IntakeBind { addr, source }` | Unchanged. |
| Missing/malformed runtime option | Private `CandidateRejected -> Rejected`, reset/close, no event/flow/control | Complete for a validated ACK, subject to F-01 monotonicity. |

No new public error envelope or diagnostic-string semantic is needed.

## Capacity, quota and model boundary

Candidate states replace the existing Pending value, so the arithmetic remains
4,097 accept-queued + 4,096 accepted/held + one classification boundary =
8,194. Pending and candidates occupy one combined pending place; terminal
classification or exact socket cleanup releases it. Active metadata remains
capacity-free socket storage. No fixed correlation capacity or below-quota
admission bound is introduced if F-01's no-recreation invariant is made true.

No abstract owner/model variable changes. Pending/candidate/terminal is a
private kernel-adapter handoff implementing the already-modeled capture/reject
outcomes. The already-passing 279 global plus 8 new model checks need no rerun
for this proposal because their hashed contract input is unchanged. The
separately approved model task remains separate.

## DISTILL and roadmap assessment

The intended DISTILL delta is minimal: neutral five/six-to-seven setup
assertions and correction of the already-authorized one parameterized
production-path cell. Its Fast Open producer stays open and sends no
post-handshake payload before `OutboundCaptured`; direct live Fast Open proves
`CandidateRejected -> Rejected`. Source-local/wrapper counts remain 8/12. No
new cell, lane, fixture, seeded child row, raw-packet path, public test support
or model is justified.

| Mandatory roadmap check | Result | Evidence |
|---|---|---|
| External validity | Pass | The corrected existing Tier-3 cell composes production `Connect4`, both correlation programs and `KernelGuestOwnerIo` in the guest-kit lane. |
| Acceptance-criterion coupling | Pass | The seven-program order, terminal gating, live-open producer and rejection outcome are observable contract/evidence obligations. |
| Step decomposition | Pass | The replacement remains inside STEP 11-06 and the existing cell/lane. |
| Implementation code | Pass | The roadmap fixes state meanings and lifecycle; private parser/synchronization structure remains DESIGN/DELIVER-owned subject to F-01. |
| Concision and precision | Changes required | Candidate monotonicity and the per-packet boundary/authority must be explicit. |
| Test boundary | Pass | DISTILL corrects the existing cell; the crafter remains forbidden to author/edit it. |
| Scope audit | Pass | No public API, map capacity, lane, model, wire, persistence, payload relay, parking or sockmap is added. |

`roadmap.json` parses and validation correctly remains `pending`. STEP 11-06 is
not executable. Roadmap approval must wait for F-01/F-03 remediation, explicit
user approval of the seventh/per-packet mechanism, the canonical authority
fold, and DISTILL's correction plus independent review.

## Exact remaining authority

After the candidate lifecycle is technically closed, the minimum user decision
must disclose the new hot-path boundary. A sufficient ask is:

> Approve `GuestProgram::TcpCorrelationIngress` as the seventh guest-root-
> cgroup attachment, ordered after SockOps `TcpCorrelation` and before the
> existing five programs/READY, to observe Fast Open final ACK headers under
> the exact child cookie and let the post-validation state callback promote
> private candidates? This `CgroupSkb` program always allows packets, never
> reads or mutates application payload, adds no map/capacity/sockmap/parking/
> timer/model/lane, and reuses the existing one Tier-3 cell and typed setup
> errors. It is nevertheless invoked for every ingress packet covered by the
> guest root cgroup and adds at least a BPF dispatch, socket-cookie helper and
> child-map lookup on the non-candidate path; its throughput/CPU/tail-latency
> cost was not measured by the correctness spikes.

An affirmative answer authorizes the exact mechanism and bounded practical
limit. It does not authorize a performance claim, another test/lane/model, or
any relaxation of exact identity, below-quota admission, cleanup,
reset-before-accept, no-payload, no-parking or no-sockmap.

## Verdict

**CHANGES_REQUESTED — EXPLICIT USER DECISION REQUIRED AFTER REMEDIATION.** The
native falsification is valid and the seventh-program candidate/promotion split
is the smallest evidenced direction currently presented. The proposal is not
technically complete until candidate updates are proven monotonic against
terminal promotion and release; the canonical ADR also needs its bounded text
repair. Even after those close, the exact seven-program/per-packet mechanism
requires explicit user approval because the prior decision approved six and no
steady per-packet correlation hook. Roadmap validation remains pending, and
DISTILL/DELIVER remain stopped. No production source, test, model, DES,
roadmap approval or commit is authorized by this review.

---

# Review iteration 2 — handshake-only fentry replacement

## Iteration 2 metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CORRELATION-CANDIDATE-STEP11-06-ARCH-REVIEW-02` |
| Date | 2026-10-23 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | The single `fentry/tcp_check_req` candidate, existing SockOps promotion, C/CO-RE build/object boundary, shared-map staging, typed setup failures, bounded test correction and program authority |
| Technical verdict | **APPROVED FOR EXPLICIT USER DECISION** |
| Workflow verdict | **USER_DECISION_REQUIRED** |
| Blocking technical findings | 0 |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** |

The user authorized validation of whether a narrower handshake hook exists.
That authority produced evidence and a reviewable proposal; it did not approve
the seventh program, C/CO-RE production build or extended setup pin. This
iteration reviews those facts without treating validation authority as product
authority.

## Iteration 1 dispositions

| Prior finding | Replacement disposition | Assessment |
|---|---|---|
| F-01 — pre-lock ingress candidate could overwrite terminal/delete state | The root-cgroup ingress program is removed. One `fentry/tcp_check_req` candidate and the existing state callback run under the same exact child socket lock; both use complete-row `BPF_EXIST` replacement. Owner/`sock_release` deletion wins before update by `ENOENT`, or deletes the replacement afterward. Native delayed-ACK, post-terminal-data and preterminal-release cases held. | **Closed.** Monotonic candidate/promotion/delete semantics now have source and native evidence. |
| F-02 — seventh per-packet root-cgroup mechanism lacked authority | The broad CgroupSkb hook is removed. The replacement dispatches only on system `tcp_check_req`; ordinary calls return at the Fast Open gate and established data never invokes it. It remains a seventh design-owned program and new global handshake/build mechanism, so explicit approval is still required under the narrower practical limit below. | **Technically closed; authority remains.** |
| F-03 — malformed ADR-0150 ingress sentence | The canonical ingress draft intentionally remains unfixed/unfolded because it is superseded, not accepted. Any approved fold must replace that draft with the reviewed fentry contract and receive final folded-contract review. | **Superseded; fold check remains mandatory.** |

No iteration-1 mechanism, cgroup lifecycle or in-value lock survives this
proposal.

## Native source and receipt validation

The independent validator's evidence is internally correlated:

- disposable Cloud Hypervisor GuestKit, x86_64 Linux
  `7.0.0-38-generic`, Aya `0.13.1`;
- native log SHA-256
  `679f659450ae2a0d4a2c112dbe64dc07c28a3e47cded4475209cc93df00fb293`;
- C/CO-RE source SHA-256
  `ce0f92d0c7f6d55c5747aeec961414192ef66e2675c673c3b3d7148eccf6d920`;
- Rust SockOps base, probe and guest-runner hashes match the handoff; and
- `VERDICT=WORKS` records ordinary, forced-syncookie, live-open data-in-SYN
  Fast Open, direct rejection, terminal/delete monotonicity and preterminal
  release, with the candidate counter zero across established data.

The physical host supplied compilation, KVM and vhost-vsock only. All runtime
BPF/maps/cgroups/qdiscs/sockets/sysctls were inside the disposable production
VMM/guest-kit boundary. The validation used ordinary socket producers, not raw
packets or seeded child state.

The failed Rust dynptr prototype proves only that its hand-written extern was
not a valid Aya object. The successful standard-helper C/CO-RE object loaded
through the unchanged Aya `0.13.1`; no dependency upgrade is claimed or
required.

## Hook scope, validation ordering and monotonicity

The exact hook is technically sound:

1. Linux has already made the complete `doff`-sized TCP header linear.
2. The selected Fast Open child reaches `tcp_rcv_state_process` with its socket
   lock held.
3. `tcp_check_req(..., fastopen=true, ...)` runs under that lock before
   `tcp_validate_incoming`/`tcp_ack`; fentry observes the current ACK there.
4. Only a validated ACK reaches `tcp_set_state(TCP_ESTABLISHED)`, whose existing
   SockOps state callback runs under the same lock before the state store.
5. Established packets use `tcp_rcv_established`, so the candidate hook does
   not run on established payload/data traffic.

The candidate and promoter replace one whole 24-byte value with `BPF_EXIST`.
The child lock serializes packet-side candidate and terminal writers; Linux's
hash bucket lock and RCU publication make each full value coherent to
userspace. `BPF_EXIST` makes deletion final across the only unsynchronized
actors:

- delete first causes later candidate/promotion update to return `ENOENT` and
  cannot recreate the row;
- replacement first is removed by the later owner/`sock_release` deletion; and
- no post-passive program inserts a child row.

An invalid ACK may leave a charged candidate. A later ACK can replace it under
the same socket lock; only the validated state transition makes the row
terminal. Native delayed final ACK, established-data sends and reset-close
cases match this source proof. The ingress-era terminal regression and row
resurrection are removed without a new map, lock field, generation or timer.

## Packet-read and identity boundary

The C program's packet access is bounded to the already-linear TCP header:

- `BPF_CORE_READ(skb, data)` relocates the kernel field through BTF rather than
  cloning a fixed `sk_buff` offset;
- `bpf_probe_read_kernel` copies the 20-byte TCP base header and at most one
  18-byte window at each of 23 bounded positions within the maximum 60-byte TCP
  header;
- header length checks prevent reads past that TCP header; and
- only exact kind `254`, length `18` and magic `0x4f56` decode metadata.

The program reads no application payload and cannot change the kernel function
return, packet bytes or verdict. It obtains the cookie from the real full child
`struct sock *` and updates only an already-existing nonterminal row under that
cookie. Ordinary calls return before cookie/map/header work; unrelated Fast
Open sockets return after the cookie/map lookup because no child row exists.

The full application cookie and original destination remain one option/row;
ordinary and syncookie consumption remains in SockOps. Direct Fast Open without
the option becomes `CandidateRejected -> Rejected`; the owner reset-closes it
before any event/flow/control record. Exact identity, one-time consumption and
reset-before-accept are preserved.

## Capacity, quota and owner lifetime

The private child ABI remains `u32 state + u32 pad + CorrelationMeta` with
`Pending | CandidateReady | CandidateRejected | Ready | Rejected`. Candidate
and terminal transitions replace one existing entry. The arithmetic remains
4,097 accept-queued + 4,096 accepted/held + one classification boundary =
8,194, and the same combined 4,096 count covers internally held children plus
emitted `CapturedConn` handles.

`KernelGuestOwnerIo` holds Pending/candidates; emits `Ready` only after the fd
also reports `TCP_ESTABLISHED`; consumes `Rejected` once with linger reset; and
deletes terminal state once. Nonterminal quota/end cleanup reset-closes before
synchronous `sock_release` deletion. The native preterminal-release case and
`BPF_EXIST` proof establish that no dead row can recreate an earlier admission
bound. Active metadata remains capacity-free socket storage with its already
approved lifetime.

## Program identity, kernel compatibility and process lifetime

The program contract is exact and implementable:

- crate-private variant `GuestProgram::TcpCorrelationCandidate`;
- label and ELF symbol `tcp_correlation_candidate`;
- section/target `fentry/tcp_check_req`;
- Aya `FEntry` load against the running guest's BTF and global process-owned
  BPF link;
- order: SockOps `TcpCorrelation`, `TcpCorrelationCandidate`, then the existing
  five programs before READY; and
- exact Linux 7.0 typed prototype with `sock`, `skb`, `req`, `fastopen`,
  `req_stolen` and `drop_reason` arguments.

The object is tied to an internal kernel function name and compatible BTF
prototype. CO-RE relocates kernel fields but does not promise attachment across
an incompatible future prototype. Missing/incompatible BTF, symbol,
relocation, prototype or attach fails before READY through the pinned typed
setup errors; no fallback or silent loss of Fast Open correlation is allowed.

Both embedded Aya objects and all seven links stay process-owned. No runtime
pin, lookup or adoption is introduced.

## C/CO-RE build and embedding review

The build amendment is complete enough for implementation after authority:

- one C file in the existing `overdrive-bpf` source tree;
- generated `vmlinux.h`, standard libbpf CO-RE/endian/helper/tracing headers;
- Clang/LLVM major 21, `bpftool`, target-kernel BTF and libbpf headers as exact
  dev/CI/release build prerequisites;
- x86_64/arm64 target-arch macros, `-g -O2 -target bpf`;
- deterministic object path
  `target/bpf/overdrive_guest_tcp_correlation_candidate_bpf.o`;
- `overdrive-init/build.rs` compile-time embedding and no guest runtime compiler
  or object lookup; and
- no Rust object, Aya version or runtime dependency upgrade.

An absent tool, unsupported build architecture, unreadable/wrong target BTF,
dump/compile failure or missing artifact fails the build with no fallback. A
guest-kit/release build must explicitly supply the BTF of the kernel it ships;
the native Linux default is only valid when the running build kernel is that
target.

The receipt proves x86_64 Linux 7.0.0-38 with Clang 21. The proposed build path
supports aarch64 structurally but has no native aarch64 receipt. No CPU,
throughput, tail-latency or packets-per-second result is proven. Those are
honest practical limits, not hidden performance claims.

## Shared-map staging and D19 boundary

The two-object loader needs the exact shared map instance. The reviewed setup
sequence creates/pins active storage, loads the Rust object so it creates and
temporarily pins `CHILD_CORRELATION`, loads the C object through the same pin
path so it reuses that map, then unlinks both pins and the directory before any
program load, attach or READY. Each object retains only its fd thereafter.

This preserves no runtime pin/adoption and process-owned teardown, but it
**extends the previously approved ADR-0159 staging exception**: that approval
named active storage; the replacement also temporarily pins the ordinary child
map so a second object can reuse it. That extension is a design-owned lifecycle
change and must be stated explicitly in the user decision and later folded into
ADR-0159. It cannot be implied by “reuse the existing map.”

## Typed setup failures

| Failure | Exact representation | Assessment |
|---|---|---|
| Active storage create/BTF/pin, Rust-map reuse/load, shared unlink | `ProgramLoad { program: TcpCorrelation, source: EbpfError }`, preserving map cause where applicable | Complete. |
| Candidate embedded-object parse/load, child-map reuse, CO-RE relocation, running BTF read, symbol/prototype/FEntry load | `ProgramLoad { program: TcpCorrelationCandidate, source: EbpfError }` | Complete. |
| Candidate FEntry attach | `ProgramAttach { program: TcpCorrelationCandidate, source: ProgramError }` | Complete. |
| Existing programs and intake | Existing program-specific `ProgramLoad`/`ProgramAttach`; `IntakeBind` | Unchanged. |
| Deleted runtime row | `BPF_EXIST -> ENOENT`; no owner-visible error and no recreation | Complete. |
| Missing/malformed validated metadata | Private `Rejected`, reset-close, no event/flow/control | Complete. |

The crate-private `prepare_guest_programs() -> Result<(),
GuestTransportSetupError>` pins the pre-loop two-object preparation boundary
without changing existing load/attach signatures. No generic string carrier or
new public/cross-crate error type is used.

## Fallout, DISTILL and model boundary

Required fallout is bounded but material: the one C source and build artifact,
tool provisioning/CI, build-script embedding, crate-private variant and
`prepare_guest_programs`, two-object loader/staging, exhaustive matches and
private owner candidate handling. Public API, host wire, persistence, owner
event, flow lifecycle, quota and model variables remain unchanged.

DISTILL corrects neutral five/six-to-seven setup assertions and the already
authorized one parameterized Tier-3 cell/wrapper. Counts remain 8/12 and the
lane is unchanged. The same cell keeps the real Fast Open producer open before
capture, proves direct rejection, delayed final ACK, parallel established
sends, preterminal reset-close, ordinary/immediate-release/syncookie behavior,
one-time consumption, combined quota and pin absence. No new cell, lane,
fixture, seeded state, raw-packet input, public test API or model is authorized.

The candidate states are a private kernel-adapter handoff for already-modeled
capture/reject outcomes. The reported fresh 279 global plus 8 new formal checks
remain valid because no hashed abstract contract input changes; another formal
record run is not required for this hook replacement. The separately approved
Quint task remains separate.

## Roadmap checks

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | Pass | The corrected existing Tier-3 cell composes production `Connect4`, SockOps candidate promotion, the embedded FEntry candidate and `KernelGuestOwnerIo`. |
| Acceptance-criterion coupling | Pass | Program identity/order, exact child states, validation boundary, live-open producer, deletion finality and typed failures are observable contract obligations. |
| Step decomposition | Pass | One hook/build amendment remains inside STEP 11-06 and the existing cell/lane. |
| Implementation code | Pass for a necessary binary boundary | C source location, object identity and build/loader contracts are architecture-owned compatibility inputs; helper structure remains with the crafter. |
| Concision and precision | Pass | Hook scope, locks, map flags, kernel compatibility and failure mappings each have one interpretation. |
| Test boundary | Pass | DISTILL corrects the existing cell; the crafter remains forbidden to author/edit it. |
| Scope audit | Pass | No dependency upgrade, map, cgroup, capacity, public API, lane, model, kernel patch, payload path, parking or sockmap is added. |

The roadmap and canonical ingress draft correctly remain pending. They must not
be approved or executed until the user approves the exact seventh-program,
build and staging-extension decision; DESIGN then folds that authority and this
fentry contract, and DISTILL corrects/reviews the existing cell.

## Exact remaining user decision

The handoff's decision is technically ready, but the approval request must
explicitly include the expanded child-map staging exception. A complete minimal
ask is:

> Approve `GuestProgram::TcpCorrelationCandidate` as the seventh guest program:
> one global `fentry/tcp_check_req` candidate ordered after SockOps
> `TcpCorrelation` and before the existing five programs/READY, plus the pinned
> C/CO-RE build and two-object loader contract? It runs once for every system
> `tcp_check_req` (ordinary calls return at the Fast Open gate), performs
> cookie/map/header work only for Fast Open with an existing child row, and
> never runs on established data. It uses the existing child socket lock,
> whole-row `BPF_EXIST` replacement and existing SockOps state callback; adds no
> map, cgroup, Aya upgrade, runtime pin, capacity, payload path, sockmap,
> parking, model or test lane; and reuses the already-authorized one Tier-3
> cell. The C object is built with Clang 21, bpftool, libbpf headers and the
> shipped guest kernel's BTF, embedded at build time and loaded by Aya 0.13.1.
> Extend ADR-0159's private pre-program-load staging exception to temporarily
> pin `CHILD_CORRELATION` as well as active storage solely so both embedded
> objects reuse the exact maps; unlink both pins and the directory before any
> program load/attach/READY, with no runtime pin or adoption. The x86_64 Linux
> 7.0.0-38 correctness run passed; native aarch64 behavior and
> CPU/throughput/tail latency were not measured.

Approval authorizes exactly that mechanism, crate-private preparation method,
build/tooling/compiler fallout, staging extension, existing-test corrections
and stated practical limits. It does not authorize a performance claim,
dependency upgrade, another test/lane/model, or any relaxation of exact
identity, Fast Open support, admission, cleanup, reset-before-accept,
no-payload, no-parking or no-sockmap.

## Iteration 2 verdict

**APPROVED FOR EXPLICIT USER DECISION.** The handshake-only fentry replacement
closes the ingress concurrency finding with exact child-lock serialization,
whole-row publication and deletion-final `BPF_EXIST`, while removing all steady
established-data BPF dispatch. Its source, native receipt, identity,
pre/post-validation order, capacity, owner lifetime, C/CO-RE build, program
identity, compatibility failures and bounded fallout are technically complete.
The seventh program, global handshake hook, C build pipeline and expanded
active-plus-child staging-pin exception remain new design-owned mechanism and
require the exact user approval above. Canonical documents, roadmap validation,
DISTILL and DELIVER remain pending. No production source, test, model, DES,
roadmap approval or commit is authorized by this review.

## Iteration 3 — explicit-authority fold and roadmap revalidation

### Iteration metadata

| Field | Value |
|---|---|
| Date | 2026-10-23 |
| Trigger | The user answered **“Approve and continue”** to the reviewed handshake-only replacement question |
| Scope | Exact fold into the feature delta, ADR-0038, ADR-0150, ADR-0159, STEP 11-06 roadmap and generated dispatch |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 3 |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** |

This iteration re-read the folded normative contract, the complete STEP 11-06
roadmap cell, its generated dispatch, the candidate/ingress/option findings and
the executed fentry spike source and receipt. The receipt hashes still match
the accepted evidence: log
`679f659450ae2a0d4a2c112dbe64dc07c28a3e47cded4475209cc93df00fb293`,
C source
`ce0f92d0c7f6d55c5747aeec961414192ef66e2675c673c3b3d7148eccf6d920`,
Rust SockOps source
`dfa0be81606ae3aa59bac5f8dfcc6bc24619328f5ab61b03c4ee4dba0691cd22`,
probe
`2f48c006afc231de9d54c869a8ddb231e70d29ea00877c9d63cb034aa94486f7`
and runner
`3daa31d162935919eb95fb4f5ad0da4de18f260f93876445f23da72e6010a87d`.

The accepted correlation section, ADR-0038, ADR-0150, ADR-0159 and STEP 11-06
otherwise agree on the technical contract: exact seven-program identity/order,
global handshake-only `fentry/tcp_check_req`, lock-free whole-row `BPF_EXIST`
replacement under the child socket lock, one C/CO-RE artifact, Aya `0.13.1`,
two-object preparation, temporary ACTIVE and CHILD pins removed before program
load/attach/READY, process-owned runtime fds/links, existing 8,194 child-map and
4,096 combined-owner bounds, exact typed setup failures, x86-only native
receipt, no performance claim, one existing parameterized Tier-3 cell and no
new lane/model/benchmark. The approved no-weakening rules remain intact in that
section. The following fold and dispatch contradictions still block final
approval.

### I3-F01 — the feature's controlling wave state still selects the superseded ingress mechanism

- **Severity:** High.
- **Evidence:** `feature-delta.md:17-20` says the “native-proven
  cgroup-ingress replacement” and roadmap are pending review.
  `feature-delta.md:39-44` likewise says the production failure proves a
  “proposed seventh cgroup-ingress program” that the reviewer must still
  approve. The same document's accepted TCP-CORRELATION section at lines
  878-935 records the user's approval and selects the global handshake-only
  C/CO-RE `fentry/tcp_check_req` candidate. The root/private cgroup ingress
  mechanisms are superseded alternatives at lines 1148-1151.
- **Reachable consequence:** Wave-state and status text are the entry point an
  orchestrator uses to decide whether DESIGN is accepted and what DISTILL must
  correct. The current text names the wrong program type and says the already
  approved replacement still awaits approval, while the roadmap and dispatch
  name `TcpCorrelationCandidate`. The canonical document therefore gives two
  incompatible instructions for the current STEP 11-06 handoff.
- **Required bounded correction:** Replace only the stale wave-state/status
  sentences with the accepted fentry decision and current gate: exact
  mechanism user-approved, final independent folded-contract/roadmap review
  pending, then DISTILL's authorized neutral corrections and one existing-lane
  parameterized cell before DELIVER. Historical ingress evidence and the
  explicitly superseded alternative may remain.
- **Disposition:** **OPEN / BLOCKING.** This is a canonical fold defect, not a
  request for another mechanism or test.

### I3-F02 — the feature charter still forbids the newly approved C eBPF exception

- **Severity:** High.
- **Evidence:** `feature-delta.md:741-756` remains binding user-selection
  text. It says “C eBPF” is forbidden, says Aya Rust eBPF stays the guest
  capture mechanism and concludes “the C-eBPF prohibition is unchanged.” The
  accepted TCP-CORRELATION contract at lines 920-1082, ADR-0038's amendment
  and ADR-0150 instead require exactly one C/CO-RE eBPF source,
  `tcp_correlation_candidate.bpf.c`, under the user's explicit approval.
- **Reachable consequence:** The STEP 11-06 crafter is required to obey the
  feature delta's exact interface and mechanism contract. It cannot both add
  the approved C fentry artifact and obey an unchanged absolute C-eBPF
  prohibition. This is the same implementation path named by the roadmap, not
  an adjacent hypothetical.
- **Required bounded correction:** Amend the charter at its two contradictory
  clauses to name exactly the approved single
  `tcp_correlation_candidate.bpf.c` C/CO-RE exception. Keep the prohibition for
  every other C eBPF program and keep the separate C kernel module distinction.
  Do not broaden the exception.
- **Disposition:** **OPEN / BLOCKING.** The user already supplied the exact
  authority; this needs reconciliation, not another user decision.

### I3-F03 — the generated crafter dispatch selects a model forbidden by the workspace rule

- **Severity:** Medium, blocking dispatch executability.
- **Evidence:** `.context/netns-density-295/dispatch-11-06.md:11` identifies
  the assigned crafter as `nw-software-crafter (GPT-6 Luna, maximum
  reasoning)`. The binding workspace instruction requires **GPT 5.6 Luna with
  maximum thinking** for every DELIVER crafter and reviewer and says never to
  fall back to another role model. The dispatch otherwise correctly carries
  the fentry mechanism and the DISTILL-before-crafter gate.
- **Reachable consequence:** Executing this concrete dispatch as written would
  start STEP 11-06 with a model selection outside the user's mandatory routing
  rule. This is a mechanical dispatch mismatch, not a proposed architecture or
  product change.
- **Required bounded correction:** Regenerate or correct the dispatch identity
  to GPT 5.6 Luna with maximum thinking when the post-DISTILL dispatch is made.
  Preserve every accepted correlation criterion and the pending DISTILL gate.
- **Disposition:** **OPEN / BLOCKING FOR THIS DISPATCH.** It does not change
  the roadmap's architecture.

### Iteration 3 roadmap checks

| Check | Result | Evidence |
|---|---|---|
| User authority | Pass | Exact reviewed question and **“Approve and continue”** appear in the accepted correlation section and approval record. |
| Program identity/order | Pass | `TcpCorrelation`, `TcpCorrelationCandidate`, then the existing five; exact label, symbol and `fentry/tcp_check_req`. |
| Pre/post-validation lifecycle | Pass | Candidate and SockOps promoter share the exact child lock; whole-row `BPF_EXIST` plus delete finality is pinned. |
| Capacity and admission | Pass | No global handoff collection; active SK_STORAGE is capacity-free, child hash remains 8,194 and owner count remains 4,096. |
| Build and loader | Pass | One Clang-21 C/CO-RE object, running target BTF, second embedded Aya object and no Aya upgrade/runtime lookup. |
| Staging lifetime | Pass | ACTIVE and CHILD pins plus directory are absent before `SockOps::load`, `FEntry::load`, attach or READY; no runtime pin/adoption. |
| Typed failures | Pass | Candidate load/compatibility failures map to `ProgramLoad(TcpCorrelationCandidate, EbpfError)` and attach to `ProgramAttach(TcpCorrelationCandidate, ProgramError)`; active/Rust/shared-unlink failures remain attributed to `TcpCorrelation`. |
| DISTILL boundary | Pass | Neutral existing setup/build corrections plus one existing parameterized guest-kit cell, counts 8/12; no added lane/model/public test API. |
| Practical limits | Pass | Native receipt is x86_64 only; aarch64 build shape and absent performance receipt are stated without a performance claim. |
| Canonical fold | Fail | I3-F01 and I3-F02 leave two live contradictory contracts in the feature delta. |
| Dispatch compliance | Fail | I3-F03 selects the wrong mandatory crafter model. |

### Iteration 3 verdict

**CHANGES_REQUESTED.** The user-approved handshake-only correction remains
technically sound and the detailed accepted contract, ADRs and roadmap preserve
identity, admission, cleanup, reset-before-accept, payload, ownership and
runtime-pin invariants. Final folded-contract and roadmap approval is withheld
only for the two exact stale feature-delta clauses and the generated dispatch's
model identity. Roadmap validation must remain `pending`; DISTILL and DELIVER
remain gated. No production source, test, model, DES or commit is authorized by
this review.

## Iteration 4 — remediation re-review and final roadmap validation

### Iteration metadata

| Field | Value |
|---|---|
| Reviewed at | `2026-10-23T19:29:00Z` |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | I3-F01 through I3-F03 remediation, complete accepted fold, STEP 11-06 roadmap and generated dispatch |
| Verdict | **APPROVED** |
| Blocking findings | 0 |
| Roadmap disposition | **APPROVE after recording this review; execution remains DISTILL-gated** |

### Finding dispositions

| Finding | Disposition | Evidence |
|---|---|---|
| I3-F01 — stale ingress wave state | **CLOSED** | `feature-delta.md:17-23` and `:41-47` now name the approved handshake-only `fentry/tcp_check_req` replacement, explicit authority, final folded-roadmap review and bounded DISTILL gate. Cgroup ingress appears only as superseded history/alternative. |
| I3-F02 — absolute C-eBPF prohibition | **CLOSED** | `feature-delta.md:744-764` keeps C eBPF forbidden except for exactly the user-approved `tcp_correlation_candidate.bpf.c` C/CO-RE fentry; it says the other six guest capture programs remain Aya Rust and keeps the C forwarder module distinction. |
| I3-F03 — dispatch model | **WITHDRAWN / SESSION AUTHORITY SUPPLIED** | Iteration 3 lacked the standalone user instruction for this `/nw-deliver` run: **“only use gpt 6 luna for crafters”**. That persistent session-specific ruling overrides the repository default, and the available selector contains `gpt-6-luna` but no `gpt-5.6-luna`. The dispatch now records the exact override at lines 11-14. GPT-6 Luna maximum is therefore the authorized selection, not an unauthorized fallback. |

I3-F03 was a reviewer context error, not a product or DESIGN defect. It is
closed by the pre-existing user authority and the corrected dispatch rationale;
no model change or new user decision is required.

### Folded-contract equality

The canonical fold now equals the reviewed mechanism and exact user approval:

- The accepted identity/order is `TcpCorrelation`,
  `TcpCorrelationCandidate`, `Connect4`, `Sendmsg4`, `Getpeername4`,
  `Recvmsg4`, `SockRelease`; the candidate label/symbol is
  `tcp_correlation_candidate` and its target is `fentry/tcp_check_req`.
- The global hook returns at `fastopen == false`, does only exact-child
  cookie/map lookup for unrelated Fast Open, reads bounded TCP headers only for
  an existing nonterminal row and never runs on established data. The fold
  claims no measured CPU, throughput or tail-latency result.
- The candidate and existing SockOps state callback run under the exact child
  socket lock and publish complete lock-free 24-byte rows with `BPF_EXIST`.
  Bucket-lock replacement plus deletion finality prevents terminal regression
  or recreation; no post-passive path inserts.
- `ACTIVE_CORRELATION` remains capacity-free socket storage and
  `CHILD_CORRELATION` remains 8,194 entries. Candidate/terminal transitions
  replace one charged row, and the same 4,096 combined count covers held
  children and emitted handles. Valid below-quota connections gain no new
  correlation-capacity refusal.
- One C/CO-RE source is built with Clang/LLVM major 21, `bpftool`, libbpf
  headers and the target guest kernel BTF. The deterministic second object is
  embedded at build time and loaded through Aya `0.13.1`; no dependency
  upgrade, runtime compiler or filesystem object lookup is added.
- Crate-private `prepare_guest_programs` stages the capacity-free ACTIVE map,
  lets the Rust object create/stage the ordinary CHILD map, lets the C object
  reuse CHILD, and removes both pins and the directory before `SockOps::load`,
  `FEntry::load`, any attachment or READY. Runtime and teardown retain only
  process-owned object/link fds and perform no pin discovery or adoption.
- Candidate object/map/CO-RE/BTF/symbol/prototype/FEntry load failures use
  `ProgramLoad { program: TcpCorrelationCandidate, source: EbpfError }`;
  candidate attach uses `ProgramAttach { program: TcpCorrelationCandidate,
  source: ProgramError }`. Active/Rust/shared-unlink failure remains
  `ProgramLoad(TcpCorrelation, EbpfError)`. Missing or malformed runtime
  metadata becomes private rejection/reset before owner-flow creation.
- The user authority is quoted exactly, the no-weakening audit preserves exact
  identity, one-time capture, ordinary/syncookie/live-open Fast Open,
  reset-before-accept, cleanup and admission below the 4,096 bound, and the
  charter limits the C exception to the approved candidate.

ADR-0038 records the one C/CO-RE build exception; ADR-0150 records the exact
guest program, pre/post-validation protocol, owner behavior and validation;
ADR-0159 records temporary ACTIVE plus CHILD staging and pin absence before
load/attach/READY. No obsolete ingress program, locked child ABI or private
cgroup remains normative. The ingress findings remain clearly bannered as
superseded fallback evidence, and the option findings remain source evidence
only for the independently valid producer, ordinary/syncookie consumer and
active-storage lifecycle.

### Roadmap and dispatch revalidation

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | Pass | Native x86_64 Linux `7.0.0-38-generic`, Aya `0.13.1` receipt with matching source/log hashes covers live-open Fast Open, ordinary, syncookie, direct reject, terminal/delete and preterminal release. |
| User authority | Pass | The feature approval record and accepted section quote the exact seventh-program/C-build/ACTIVE+CHILD staging question and **“Approve and continue”** answer. |
| Acceptance-criterion coupling | Pass | Exact program order, candidate states, BPF_EXIST finality, compatibility failures, quota/lifetime and pin absence are observable obligations in the existing production-path cell. |
| Step decomposition | Pass | The correction stays within STEP 11-06 and its existing guest-kit lane; no extra step, lane, model or benchmark enters the roadmap. |
| Test boundary | Pass | DISTILL owns neutral six-to-seven/build assertion corrections and the one existing parameterized TCP-CORRELATION cell; the crafter remains forbidden to edit/author bodies. |
| Existing-lane counts | Pass | `guest_flow::real_kernel` remains 8 and `in_guest_lane` remains 12; the correlation matrix remains one parameterized cell. |
| Compiler/caller fallout | Pass | One C source/object, exact build tools, build-script embedding, crate-private program/preparation arms, exhaustive matches and private candidate handling are all listed. |
| Typed errors | Pass | Every designed setup, compatibility, attach and runtime rejection condition has one exact representation. |
| No weakening | Pass | No accepted zero/never/must/exact/fail-closed outcome is removed or relaxed; active state adds no capacity bound and child state remains charged within the accepted quota. |
| Practical limits | Pass | Native aarch64 behavior and numeric performance remain unclaimed; x86 receipt and structural arm64 build target are distinguished. |
| Dispatch | Pass | It carries the folded criteria, exact spike versions, no-invention/test-ownership rules and the persistent GPT-6 Luna user override; it says DISTILL must complete before the crafter resumes. |
| Mechanical validity | Pass | `roadmap.json` parses; all reviewed diffs pass `git diff --check`; validation remained pending throughout review. |

The separately approved `SLOT-ATTACH-REFUSAL-SHAPE` fold remains incorporated:
its thirteenth source-local UDP-slot cell is DISTILL-owned, and its separate
Quint-model task remains outside this fold. This review neither changes nor
reopens those decisions.

### Exact roadmap validation record

Record exactly:

```json
{
  "status": "approved",
  "reviewer": "Independent TCP-CORRELATION-CANDIDATE folded-contract and STEP 11-06 roadmap review iteration 4 approved on 2026-10-23; the prior SLOT-ATTACH-REFUSAL-SHAPE roadmap approval remains incorporated. DELIVER remains gated until DISTILL authors and independently reviews the authorized neutral six-to-seven/build corrections, the one parameterized TCP-CORRELATION cell in the existing guest-kit lane, and the user-approved thirteenth source-local UDP-slot cell. The separately approved Quint-model work remains outside this fold.",
  "approved_at": "2026-10-23T19:29:00Z"
}
```

The DESIGN owner owns that scoped roadmap metadata update and commit. This
reviewer does not self-record or commit it and will mechanically verify the
record after the owner writes it.

### Iteration 4 verdict

**APPROVED.** I3-F01 and I3-F02 are reconciled without broadening the approved
mechanism; I3-F03 is withdrawn because the previously missing explicit session
authority requires GPT-6 Luna for this crafter run. The feature delta, ADRs,
roadmap and dispatch now express one invariant-preserving handshake-only
contract with exact authority, identity, lifecycle, capacity, typed failures,
build/loader boundary and DISTILL scope. Record the validation block above,
then keep DELIVER stopped until the required DISTILL work and independent
review complete. No production source, test, model, DES state or commit was
changed by this review.

### Post-record mechanical verification

At `2026-10-23T19:30:07Z`, the DESIGN owner had recorded the iteration-4
validation block. The reviewer verified:

- `roadmap.json` parses as JSON;
- `validation.status`, `validation.reviewer` and `validation.approved_at` are
  byte-for-byte equal to the exact block above;
- STEP 11-06 still carries the exact seven-program order, candidate hook,
  single C artifact, real-kernel count 8, existing-lane count 12, one
  parameterized correlation cell and DISTILL-before-crafter gate;
- no `TcpCorrelationIngress` or `CgroupSkb` mechanism remains in STEP 11-06;
- the generated dispatch retains the exact candidate contract and recorded
  GPT-6 Luna session override;
- the five native evidence files still match their reviewed SHA-256 values;
  and
- every reviewed artifact passes `git diff --check`.

**Mechanical result: PASS.** The roadmap validation is exactly recorded as
approved. Its execution gate remains the DISTILL work and independent review
named in the validation record.
