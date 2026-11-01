# DELIVER Review — Step 11-05 Forwarder Module and Adapter

## Review metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `11-05` |
| Review type | Fresh isolated current-date post-completion closure review |
| Reviewer role | Fresh isolated DELIVER software-crafter reviewer |
| Review date | 2026-11-01 |
| Implementation commit | [`b8226c05ea0b4e00a01c0a398eb41f887e1e664a`](../../../../commit/b8226c05ea0b4e00a01c0a398eb41f887e1e664a) — `feat(netns-density-295): implement event-driven forwarder module and adapter` |
| Completion record | [`ae86884e11630ae50d14eadab76833763bbbd1fa`](../../../../commit/ae86884e11630ae50d14eadab76833763bbbd1fa) — step 11-05 COMMIT event |
| Current review base | `00ce922c43b9743651cfda535af006f2393abbe3` |
| Iteration | 1 |
| Verdict | **APPROVED** |

This artifact is the review performed now. It does not fabricate a reviewer
decision at the 2026-10-23 implementation or completion time. The implementation
and completion commits were read from their actual objects, and current
consumers and retained evidence were checked at the current review boundary.

The review used `AGENTS.md`, the repository development, Rust, testing and
verification rules, the installed `nw-software-crafter-reviewer` methodology
and its `nw-sc-review-dimensions`, `nw-tdd-review-enforcement` and
`nw-tdd-methodology` skills. The controlling design sources were roadmap step
11-05, the EVMOD sections of `docs/feature/netns-density-295/feature-delta.md`,
and accepted ADRs 0180, 0183, 0197–0206, 0209–0212 and 0216.

No production source, test source, DES log, roadmap, design artifact, host
state or commit was changed by this review. The reviewer-owned change is this
Markdown artifact only. Pre-existing dirty work, including `AGENTS.md` and
parallel reviewer artifacts, was preserved.

## Review boundary and authority

Step 11-05 is the accepted EVMOD-ADOPT slice. Its roadmap contract requires:

1. the event-driven `ovd_fwd` Linux 7.0 module, its control UAPI and end
   records;
2. the core `FlowForwarder` port, `KernelFlowForwarder` adapter and
   `SimFlowForwarder` arbiter;
3. stream, datagram, attach-order, end-classification, watch, context-release,
   inventory, module-reference, per-CPU workqueue and kTLS-refusal behavior;
4. the real-kernel and disposable GuestKit integration cells, including the
   100,000-plus V-EV-SAFE stress; and
5. guest-kit staging at `GUEST_FORWARDER_MODULE_PATH` for the in-guest cells.

The accepted contract pins the UAPI magic `0xB9`, request numbers `0x00` to
`0x06` in the EVMOD-CTL table order, ABI version `1`, native 32-byte end
records, and the frame constants `OVD1`, 8-byte header, 59,000-byte payload
maximum and 59,008-byte frame maximum. The parser contract requires the exact
shipped C function to be compiled through test support, differentially tested
against the Rust codec, sanitizer-tested on exact-size allocations and fuzzed
through the retained libFuzzer target. The user's explicit ruling retained
that libFuzzer support and requested its run; KASAN remains outside this step.

The review accepts only evidence whose kernel or module effects run inside the
disposable production Cloud Hypervisor GuestKit boundary. Metal transport is
not treated as isolation. No host-native kernel mutation, cleanup by process
kill, mutation testing, KASAN run or GH #326 benchmark is used as a claimed
step gate.

## Implementation and exact interface review

### Core port and public API

`crates/overdrive-core/src/guest_transport/forwarder.rs:34-406` provides the
approved public vocabulary and exactly the approved `FlowForwarder<S>` methods:
`probe`, `attach`, `watch`, `abort`, `abort_all`, `take_end`, `pair_state` and
`inventory`. The implementation has the pinned `ForwardId`,
`ForwarderGeneration`, `PairSide`, `PairMode`, `AbortCause`, `ForwardOutcome`,
`ForwardEnd`, `SourceEnd`, `DirectionState`, `PairState`,
`ForwarderInventory` and `AbortResult` shapes. The newtypes retain validating
constructors, decimal `Display`/`FromStr` and matching serde forms.

The public semantic error set at `forwarder.rs:412-590` is complete and
bounded. No new method, public type, enum variant, parameter, wire field or
persisted field was invented to make a test pass. Private helper structure and
the Sim's test-support scripting surface remain implementation/test-support
choices.

`KernelFlowForwarder` exposes only the approved inherent `open` and
`readiness_fd` operations in `crates/overdrive-fwd/src/lib.rs:72-109` and
implements the core port over `dyn AsFd`. The type is not `Clone`; it owns one
context and closes it on drop. The current HEAD later adds two debug tracing
events around inventory acquisition for approved downstream observation
consumers (the bounded change in `bc832a3e0e6b031502165b338ab8c4604efbdf10`);
that later telemetry does not change the step 11-05 port or its ownership
contract.

### UAPI, wire and typed error mapping

The crate-private mirror in `crates/overdrive-fwd/src/uapi.rs:18-242` uses the
approved ioctl encodings and compile-time size/offset assertions for every
UAPI structure and end-record field. `decode_end_record` at `uapi.rs:253-322`
rejects invalid id, kind, cause, side, errno and reserved fields before
constructing the approved core outcome.

| Designed failure or outcome | Approved representation | Implementation evidence |
|---|---|---|
| `/dev/ovd_fwd` cannot open | `ForwarderError::DeviceAbsent { source }` | `overdrive-fwd/src/lib.rs:84-101` |
| ABI version differs | `ForwarderError::AbiMismatch { expected, found }` | `uapi.rs:325-340`, version probe in `lib.rs:390-409` |
| Unsupported pair/watch socket kind | `ForwarderError::UnsupportedSocket { side }` | adapter refusal mapping `lib.rs:127-142`; module validation `ovd_fwd.c:786-833` |
| TCP ULP is present | `ForwarderError::UpperLayerProtocol { side }` | `lib.rs:133-139`; module flag/state validation `ovd_fwd.c:640-651,835-869` |
| Never-connected, listening or self-send-shut half | `ForwarderError::NotConnected { side }` | `lib.rs:135`; `ovd_fwd.c:654-681` |
| Connected half already ended | `ForwarderError::HalfEnded { side, errno }` with the half's `end_errno` | `lib.rs:136-137`; `ovd_fwd.c:654-680,990-998` |
| `sk_user_data` already occupied | `ForwarderError::AlreadyHooked { side }` | `lib.rs:138`; `ovd_fwd.c:896-913,721-758` |
| Datagram vsock buffer below 59,008 bytes | `ForwarderError::BufferTooSmall { side }` | `lib.rs:139`; `ovd_fwd.c:737-742,1056-1063` |
| Control/read/ioctl failure | `ForwarderError::Io { op, source }` | `lib.rs:111-117,212-318` |
| Malformed native end record | `ForwarderError::EndDecode { field }` | `uapi.rs:244-322` |
| Probe failure | `ForwarderError::Probe { stage, source }` or `ProbeMismatch { stage, observed }` | `lib.rs:322-505,1016-1054` |
| Pair/watch completion or abort | `ForwardOutcome::{Completed, GuestHalfClose, Aborted { cause }}` | `forwarder.rs:254-266`; C record encoding `ovd_fwd.c:228-267,1770-1797` |
| Per-flow abort cause | `AbortCause::{PeerAbort, SocketError, FrameInvalid, Detached, Unclassified}` | `forwarder.rs:206-252`; C cause counters and records `ovd_fwd.c:240-267,1770-1797` |

The mappings preserve semantic branching at the owner. No semantic distinction
is carried only in a diagnostic string, and no generic outer carrier replaces
an approved typed cause.

### Module lifecycle and safety invariants

The C module implements the accepted lifecycle in
`kmod/ovd_fwd/ovd_fwd.c`:

- module refs are held by open contexts and each hooked socket
  (`:721-758`, `:1882-1913`);
- callbacks read `sk_user_data` with acquire semantics under RCU and invoke the
  current original callback after unhook (`:490-567`, `:761-784`);
- `sk_write_space` is not overridden; a per-socket wait-queue entry is added
  and removed under the socket lock (`:380-394`, `:915-935`);
- pair work uses the module-owned `WQ_PERCPU` queue and a bounded peek/send/
  consume loop (`:1390-1418`, `:1433-1605`);
- end cleanup restores callbacks, performs abort actions only after unhook,
  queues one record and then queues RCU/free work (`:1770-1913`); and
- context release aborts live pairs, shuts watched sockets only on release,
  discards queued records and waits for the live, outstanding and cleanup
  complements before returning (`:1167-1228`, `:1362-1369`).

The accepted attach precedence and state classification are implemented before
hook installation and are serialized under the required socket/table/callback
locks (`:871-935`, `:1001-1089`). `ovd_fwd_classify_vsock` records each
transition once and keeps the clean-full-end ordering (`:396-472`). The
vsock end recheck orders orphan-sweep facts through `vsock_table_lock`
(`:1503-1534`), leaving `Unclassified` as an alarm for a broken invariant.

### Datagram and FRAME-PARSE behavior

The UDP-to-vsock path consumes an oversized datagram and increments
`dgram_oversize`; otherwise it builds one `OVD1` frame, preserves partial-send
offsets and consumes the UDP datagram only after the whole frame is accepted
(`ovd_fwd.c:1607-1681`). The vsock-to-UDP path uses only
`ovd_fwd_frame_parse`, waits on `vsock_stream_has_data`, rejects bad magic and
oversize lengths as `FrameInvalid`, and sends one payload datagram after the
whole frame is queued (`:1683-1767`). Empty datagrams are represented by an
8-byte frame and remain ordered.

The exact C parser is in `ovd_fwd_frame.c/.h`; the parser test crate compiles
that source itself through `cc` and exposes only test-support `FrameParse` and
`parse`. The retained user-approved evidence in
`.context/netns-density-295/11-05-parser-distill/audit.md` records:

- two default-lane differential proptest bodies, including arbitrary buffers,
  split points, encoded sequences and chunk boundaries;
- the exact shipped source called through the same FFI wrapper;
- userspace C ASan plus trap-mode UBSan fuzz support, with exact-size boxed
  input allocations;
- the requested bounded libFuzzer run: 332,642,216 runs in 301 seconds,
  exit 0, no sanitizer failure, timeout, crash artifact or unknown outcome;
  and
- no KASAN, syzkaller or mutation claim.

This is sufficient evidence for the accepted parser obligation and preserves
the user's explicit fuzzing decision.

## Current consumers and lifecycle ownership

The current host consumer is `HostGuestFlowIo`: it opens/probes a context at
`flow_io/host.rs:343-375`, watches dialled host-vsock streams, attaches
host-opened TCP/datagram pairs before `Paired`, waits on `readiness_fd`, reads
end records, and maps records to owner flow outcomes at `host.rs:587-869`.
The guest production owner opens and ABI-checks its context before READY in
`overdrive-init/src/transport/kernel.rs:388-401`. The GuestKit scripted peer
uses the same `KernelFlowForwarder` adapter (`overdrive-testing/bin/guest_peer.rs`)
and the Sim owners use the named `SimFlowForwarder` arbiter. No consumer
reintroduces the removed host sockmap forwarder, a second public forwarding
port, or a detached payload pump.

Context generation remains owner observation state, not kernel ABI. The host
recovery path closes the old context before opening/probing a new one, and the
owner holds no cloned or inherited descriptor. Guest-kit staging is recorded
by the 11-05 native evidence and the completion contract; guest module load,
release checking and typed setup failures remain step 11-06's bounded owner.

## TDD, test integrity and external validity

### DES and commit evidence

The append-only execution log records the actual step history:

| Phase | Result | Recorded time |
|---|---|---|
| RED | PASS | 2026-10-22 17:04:25Z |
| GREEN | FAIL (intermediate attempt) | 2026-10-22 21:14:53Z |
| GREEN | PASS | 2026-10-22 23:49:27Z |
| COMMIT | PASS | 2026-10-22 23:50:18Z |

The failed intermediate GREEN event is preserved and the later passing GREEN
event precedes COMMIT. Both implementation and completion commits preserve the
Marcus author, exactly one `Co-Authored-By: Codex <codex@openai.com>` trailer
and `Step-Id: 11-05`.

### Test integrity

The implementation activates the DISTILL-authored pending cells. The ordinary
Tier-3 test diffs remove pending markers; they do not weaken assertions. The
large `unload_in_guest` addition is the user-approved V-EV-SAFE evidence
strengthening: it adds singular `ABORT`, racing `PAIR_STATE`, exact id/record
accounting, per-CPU received-byte evidence, the 100,000-end threshold and the
existing no-oops/unload checks. The FRAME-PARSE crate and fuzz target are the
separately recorded user-approved test-support additions. No test-only seam
becomes a production API, and no fixture directly writes the expected module
or end state in place of the production module.

The source-local pure properties carry the exact
`/// CONTRACT_SHAPE: pure-function.` declaration. State-changing Sim and stress
tests carry the bounded-change declaration required by the repository. The
real-kernel and GuestKit cells are integration/E2E wiring evidence and remain
single-example where the methodology permits it. No zero-assertion,
tautological, mock-dominated, circular, skipped-at-runtime or fixture-theater
pattern was found in the reviewed step boundary.

The conventional unit-test budget is not used to reject this slice: the
roadmap's real-kernel cells are adapter integration tests, the Sim bodies are
the DISTILL acceptance cells for the named arbiter, and FRAME-PARSE's two
pure-function properties cover the pure parser surface. The required evidence
is the accepted behavior matrix and its real kernel boundary, not a quota of
duplicated example tests.

### Evidence matrix

| Contract surface | Retained current evidence | Result |
|---|---|---|
| Build, sparse/checkpatch, module artifact | `11-05/evidence/lima-kmod-build-final-after-fixes.log`; checkpatch reports 0 errors/warnings and the Linux 7.0 module builds | PASS |
| Rust workspace checks | `lima-cargo-check-final*.log`, `lima-clippy-final*.log`, `lima-fmt-final*.log` | PASS |
| Core Sim forwarder | `lima-sim-flow-forwarder.log`: 19 passed, 102 skipped in the selected acceptance binary | PASS |
| Control-plane Sim flow I/O cells | `lima-flow-io-sim.log`: 10 passed, 1,119 skipped in the selected binary | PASS |
| UAPI/decode/layout | `lima-fwd-uapi-tests.log`: 3 passed, 0 skipped | PASS |
| Host module integration | `metal-disposable-fwd-suite-results.log`: 86 core cells and 4 unload cells passed; the targeted core receipt records 68 additional focused cells passed, with the intentionally separated absent-loopback negative cell covered by the disposable probe group | PASS |
| Guest boundary | `metal-guest-boundary-all.log`: 7 passed, 0 failed | PASS |
| V-EV-SAFE and V-EV-UNLOAD | `metal-disposable-stress-parallel-contexts.log` and the suite receipt: 106,216 unique pair ends, 212,432 `PAIR_STATE` calls, 686 unloads, 16/16 CPU byte counters positive, zero faults/residue/unclassified ends | PASS |
| Guest-kit nested boundary | `metal-disposable-nested-boundary.log`: 7 passed, 0 failed; nested effects remain inside disposable GuestKit guests | PASS |
| FRAME-PARSE differential/fuzz | `11-05-parser-distill/audit.md`: 2 properties, sanitizer smoke/deep-header runs and requested 301-second libFuzzer run | PASS |
| kTLS negative control | Suite receipt's ULP cell and `metal-unload-in-guest-ulp-host-after-path-fix.log` | PASS |

Earlier failed native receipts are retained as evidence of bounded harness or
fixture iterations. They are not used as final pass evidence where the later
isolated receipt proves the same contract; no historical failure was deleted or
rewritten.

## Findings

No blocking or non-blocking implementation finding remains for step 11-05.
The accepted public API, wire/UAPI shapes, semantic error mapping, ownership,
release ordering, module-reference lifecycle, parser support and current
consumer composition agree with the design and the retained bounded evidence.

The current worktree contains unrelated dirty files and parallel review
artifacts. They are outside this step review and were left untouched.

## Verification and disposition

- `cargo xtask kmod-build` evidence is retained for Linux 7.0.
- Lima format, check, clippy and targeted nextest receipts are retained.
- Native disposable GuestKit receipts cover module, unload, stress and guest
  boundary behavior.
- The parser source was differentially proptest-tested and fuzzed with the
  requested userspace sanitizer/libFuzzer support.
- Mutation testing, KASAN, syzkaller and GH #326 benchmark evidence are not
  claimed, consistent with the explicit workflow/user boundary.
- No rerun was necessary for this closure review; the retained current
  bounded receipts are sufficient and no specific evidence gap was found.

## Final verdict

**APPROVED — iteration 1.**

This is a current-date closure approval for step 11-05 based on the actual
implementation/completion commits, current bounded consumers and retained
evidence. It is not a fabricated historical approval, and it does not alter
the frozen 01–04 history, later step contracts, explicit KASAN/mutation/
benchmark exclusions or the user's ownership boundaries.
