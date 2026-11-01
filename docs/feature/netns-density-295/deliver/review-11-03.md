# DELIVER Review — Step 11-03

## Review metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `11-03` — `[step 2] Core types and wire codec` |
| Reviewer | Fresh isolated DELIVER reviewer; independent current-date closure audit |
| Review date / iteration | 2026-11-01T22:00:53+01:00 / 1 |
| Implementation commit | `c3ed14f8e98f80a146b8e069506528ee7d931800` |
| DES completion commit | `9a69f6d69b801998f49e8244d50b355dcb349da0` |
| Reviewed head | `00ce922c43b9743651cfda535af006f2393abbe3` (`marcus-sa/flow-quota-240s-cap`) |
| Iteration 1 verdict | **NEEDS_REVISION** — one blocking Contract Shape declaration defect; resolved in iteration 2 below |

This is a current closure review. It does not claim that a historical reviewer
ran at the 2026-10-22 completion time. The review covers the implementation
commit, its append-only RED/GREEN/COMMIT records, the current production
consumers after later approved steps, and the current bounded evidence. No
source, test, design, roadmap, DES, or commit file was changed by this review;
the pre-existing dirty `AGENTS.md`, roadmap edit, and whole-slice review file
remain untouched. No mutation, KASAN, VM, native-metal, benchmark, or new test
run was needed.

The installed `nw-sc-review-dimensions`, `nw-tdd-review-enforcement`,
`nw-tdd-methodology`, `nw-review`, and reviewer-role instructions were applied
subject to the repository rules. The repository's exact Contract Shape rule is
also applied: every new or transitioned test needs a declaration, and every
live source-local pure-function property needs the exact
`/// CONTRACT_SHAPE: pure-function.` line.

## Contract sources and bounded scope

The authoritative step is `docs/feature/netns-density-295/deliver/roadmap.json`
step `11-03` (criteria, scenario IDs `R5-21`, `R5-51`, `R5-55a`, `R5-60`,
`DG-7`, and the four Lima verification commands). The interface and typed
error contract is in `docs/feature/netns-density-295/feature-delta.md`:

- Core vocabulary and newtypes at lines 2128–2620, including `GuestCid`,
  `LaunchToken`, `GuestTransportAssignment`, `GuestFlowPorts`, `ListenPort`,
  `InterfaceIndex`, intake constants, and deployment refusal mappings.
- Flow request, control open/message and wire errors at lines 2624–2885.
- Datagram-frame codec at lines 2886–2920 and DG-8 at lines 12345–12400.
- `FlowForwarder` vocabulary and exact driven-port shape at lines 4616–4818.
- `GuestStreamDial`, `GuestStream`, `GuestStreamEnd`, and `GuestDialError` at
  lines 5084–5485.

The accepted ADRs read against those sections were ADR-0145 (vsock transport
replacement), ADR-0149 (OVD1 datagram frame), ADR-0155 (no topology ceiling),
ADR-0169 (counted move-only quiescence hold), and ADR-0177 (CID plus live
launch-token attribution). The later approved attachment-cut commit
`7105b5807` removed the redirect-era `guest_transport::wire` E-list
(`FlowEvent`, `FlowCell`, `CellEpoch`, `FlowEventDirection`, and event wire
fields) that the step-11-03 implementation intentionally left as pending
scaffold. The current source therefore has no such obsolete transport surface;
the unrelated `overdrive_core::traits::dataplane::FlowEvent` is outside this
step's transport contract.

## Result and acceptance mapping

| Step criterion | Current implementation and evidence | Assessment |
|---|---|---|
| Flow request kinds 1/2/4, control open/messages, datagram frame, READY launch field, canonical bytes and typed refusals | `crates/overdrive-core/src/guest_transport/wire.rs:42-257`, `:264-302`, `:458-573`, `:651-785`; `crates/overdrive-core/src/vm/beacon.rs:322-350`. The focused receipt `.context/netns-density-295/11-03/core-acceptance-final.txt` records 113/113 focused `vsock`/`vm_beacon` tests passed. | PASS |
| `GuestCid`, `LaunchToken`, `FlowId`, `KernelErrno`, forwarder vocabulary, stream vocabulary and `ListenPort`/`InterfaceIndex` text/serde/refusal contracts | `guest_transport/mod.rs:43-116`, `:375-529`; `wire.rs:28-68`, `:312-385`; `forwarder.rs:34-168`; `dial.rs:20-302`; `net.rs:21-145`. The same 113-test receipt includes the PBT and acceptance cells. | PASS |
| Reserved listener ports and the total probe cap refuse at TOML and API ingress before intent write | `aggregate/workload_spec.rs:801-821`, `:1046-1071`; `aggregate/mod.rs:441-554`; the deploy CLI validates before its HTTP submit at `crates/overdrive-cli/src/commands/deploy.rs:198-256` and `:317-326`. `vsock_deploy_validation` is part of the 113 passed tests. | PASS |
| Every named `pending DELIVER step 2` consumer is activated without weakening its body | The implementation removes the named markers; current source-local and consumer tests compile. The later package-union receipts retain a passing `service_kind_vm_workloads` consumer, for example `.context/netns-density-295/13-01/package-union-exact55-approved-quint-seed.log:6370-6377`. The `overdrive-sim`/`overdrive-worker` step receipt records 490/490 passed with 140 future tests still skipped. | PASS |
| No `FlowEvent`, `FlowCell`, `CellEpoch`, or request kind 3 in the transport surface | `wire.rs` currently contains request kinds 1/2/4 only and no redirect-era E-list types; kind 3 maps to `FlowWireError::UnknownKind` at `wire.rs:210-214`. The E-list deletion is recorded by approved later commit `7105b5807`; it is not treated as a fabricated historical step-11-03 change. | PASS |

The current implementation matches the accepted public shape. No new method,
type, enum variant, trait, parameter, wire field, or persisted field was
introduced beyond the feature delta. The step's later consumers use the exact
core types and no source path retains the redirect-era flow-event transport
surface.

## Exact API and typed-error mapping

| Designed semantic boundary | Approved and implemented representation | Evidence |
|---|---|---|
| CID outside the leased range or malformed decimal | `GuestCidError::{OutOfRange { raw }, Unparseable}` | `guest_transport/mod.rs:56-116` |
| Zero or malformed launch token | `LaunchTokenError::{Zero, Unparseable}`; wire decoders project zero to `FlowWireError::InvalidField { field: LaunchToken }`; READY projects parse failure to `BeaconParseError::InvalidLaunchToken` | `mod.rs:384-469`; `wire.rs:227-231`, `beacon.rs:338-350` |
| Invalid flow origin, request magic/version/kind/field | `FlowWireError::{BadMagic, UnsupportedVersion, UnknownKind, OriginMismatch, InvalidField, ...}` with `WireField` payloads | `wire.rs:201-257`, `:576-649` |
| Control open/message malformed bytes | `ControlOpen::decode` checks magic/version/reserved/token; `ControlMessage::decode` maps unknown type/refusal, zero errno, non-zero reserved detail and canonical fields to `FlowWireError` | `wire.rs:284-302`, `:518-573` |
| Datagram frame malformed or over bound | `DatagramFrameError::{Truncated, BadMagic, Oversize, LengthMismatch}`; max is 59,000-byte payload and 59,008-byte frame | `wire.rs:656-785` |
| Forwarder owner defect, ended half, device/ABI/probe failure | Exact `ForwarderError` set, including `HalfEnded`, `BufferTooSmall`, nested `Probe`, and `ProbeMismatch`; no flattening into a string | `forwarder.rs:503-596` |
| Guest stream/dial outcomes | Exact `GuestDialError` set (`Refused`, `TimedOut`, `PairingDeadline`, `NotServing`, `NoSession`, `Quiesced`, `Unreachable`, `Capacity`) | `dial.rs:226-302` |
| Reserved listener and probe-cap ingress refusal | `ParseError::{ListenerPortReserved, ProbeCountExceeded}` at TOML ingress; existing `AggregateError::Validation` with fields `listeners[].port` and `probes` at API ingress | `workload_spec.rs:125-132`, `:285-292`, `aggregate/mod.rs:500-553` |

No unresolved typed-error contract ambiguity was found. The outer
`AggregateError::Validation` is the approved API-ingress carrier; it does not
replace the semantic `ParseError` at the TOML ingress. No failure is encoded
only in a diagnostic string where the design requires a typed branch.

## Contract Shape compliance

All newly authored or transitioned step-11-03 tests except the transitioned
READY tests in `vm_beacon_roundtrip.rs` carry the required declarations. The
PBT and pure codec properties in `vsock_wire_codec.rs`,
`vsock_datagram_frame.rs`, `vsock_guest_transport_vocabulary.rs`,
`vsock_forwarder_vocabulary.rs`, and `vsock_deploy_validation.rs` use the
exact `/// CONTRACT_SHAPE: pure-function.` line. The stream tests use the
declared `pure-function` or `bounded-change` shape. The activated cross-crate
consumer cells carry their bounded-change declarations.

One mechanical defect remains:

- The five tests transitioned from `#[ignore = "pending DELIVER step 2"]` in
  `crates/overdrive-core/tests/acceptance/vm_beacon_roundtrip.rs` have no
  Contract Shape declaration: `ready_roundtrips_through_display_and_from_str`
  (`:99-102`), `ready_message_renders_the_measured_wire_shape` (`:111-116`),
  `from_str_tolerates_an_unstripped_trailing_newline_after_ready` (`:145-147`),
  `from_str_rejects_ready_with_missing_key_prefix` (`:175-178`), and
  `from_str_rejects_ready_with_non_integer_pid` (`:186-187`). The target
  commit removed only their pending markers, so these are live transitioned
  tests, not future ignored tests. The first is a source-local property and
  must carry the exact pure-function line; the four example cells need their
  declared shape as required for transitioned tests.

This is a mechanical reviewer blocker. It requires only the declarations on
the five already-authored bodies; it does not authorize test-body changes,
new scenarios, a new lane, a production API, or a design change.

## DES, attribution, test integrity, and verification

The append-only execution log records the actual step events in order:

| Phase | Recorded event |
|---|---|
| RED | `11-03 / RED / EXECUTED / PASS` at `2026-10-22T10:21:59Z` |
| GREEN | `11-03 / GREEN / EXECUTED / PASS` at `2026-10-22T10:49:33Z` |
| COMMIT | `11-03 / COMMIT / EXECUTED / PASS` at `2026-10-22T10:50:57Z` |

No historical event is added or reinterpreted. Both implementation and
completion commits retain the Marcus author, exactly one
`Co-Authored-By: Codex <codex@openai.com>` trailer, and `Step-Id: 11-03`.
`git diff --check` passes for both boundaries. The completion commit changes
only `execution-log.json`.

The target tests are acceptance-boundary tests over pure codecs and typed
vocabulary. They are not expectation runners and do not spawn the production
binary. The focused current receipt records 113 tests passed; the companion
Sim/worker receipt records 490 passed and 140 skipped future tests. The
workspace check and workspace Clippy receipt both pass with
`--features integration-tests`; formatting and diff checks pass. The RED
receipt shows the authored bodies failed on the scaffold before GREEN, and the
GREEN receipt shows the production implementation made them pass. No assertion
was weakened, no test was deleted, and no test body was changed to make the
implementation pass. The two inbound integration bodies were adapted to the
already-existing shared-owner production representation and strengthened to
assert exact destination-set and constant-program complements; their live
tests carry bounded-change declarations.

The active focused test count is exactly 113 (26 wire, 17 datagram, 22 core
transport, 13 forwarder, 8 stream-dial, 10 deployment, 17 beacon). These are
pre-authored acceptance/PBT cells over the step's multiple independent wire,
newtype, error and ingress behaviors; the count is not a duplicated unit-test
set or an implementation-mirroring suite. No domain-layer test, mock-dominated
SUT, zero-assertion test, circular oracle, or expectation-runner boundary
violation was found. Property tests use bounded strategies and assert the
canonical byte/typed outcome; integration cells exercise their existing
production composition.

The step's model/ordering rules do not introduce a new control-plane defect
hypothesis. No seeded simulation regression, native spike, or test seam was
invented or needed for this pure vocabulary/codec step. Kernel and VM claims
remain in the accepted ADR/spike evidence and are not reclassified by this
review.

## Iteration 1 review dimensions and gates

| Gate / dimension | Disposition |
|---|---|
| Exact public API and wire shape | **PASS** — matches feature delta and accepted ADRs; later E-list deletion is present in current source |
| Typed semantic error set and mappings | **PASS** — exact variants/payloads and ingress projections are preserved |
| Reachable production composition | **PASS** — current bounded consumers compile and later package-union receipts exercise the named consumer |
| Observable behavior and external validity | **PASS** — codecs/newtypes are entered through their public constructors/codecs; no internal production class is tested directly |
| TDD RED → GREEN → COMMIT order | **PASS** — three recorded events are ordered and PASS; evidence is retained under `.context/netns-density-295/11-03/` |
| Test integrity / no weakening | **PASS** — marker removals and the strengthened shared-owner assertions were inspected; no weakened oracle found |
| Test budget | **PASS with scope note** — 113 active cells cover the five scenario IDs' independent protocol/newtype/error branches and PBT domains; the acceptance/PBT vocabulary suite is not a duplicate unit-test expansion |
| Contract Shape declarations | **BLOCKED** — five transitioned READY tests lack required declarations (CS-01) |
| RPP / implementation bias | **PASS** at the step boundary — no unapproved production mechanism or API expansion found |
| Mutation testing | **Not run** — prohibited per the active DELIVER instructions |

## Iteration 1 finding

### CS-01 — Blocker: five transitioned READY tests are live without Contract Shape declarations

The target commit activates five pre-authored `pending DELIVER step 2` READY
tests in `vm_beacon_roundtrip.rs` by deleting their `#[ignore]` attributes,
but it does not add the repository-required per-test Contract Shape
declarations. The exact live locations are listed in the Contract Shape
section above. This violates the explicit repository rule for transitioned
tests and the reviewer mechanical gate. The first test is a source-local
pure-function property and must use the exact rustdoc declaration
`/// CONTRACT_SHAPE: pure-function.`; the four deterministic tests need their
appropriate shape declarations while retaining their existing bodies and
assertions.

Reachability is direct: `crates/overdrive-core/tests/acceptance.rs:348`
registers `vm_beacon_roundtrip`, and the target commit's marker removal makes
the five bodies live in the focused acceptance binary. The current receipt
proves they execute, so this is not a hypothetical or unreachable test
state. No production failure or design gap is alleged.

**Required disposition:** Return to the original step-11-03 crafter for a
mechanical declaration-only remediation, then perform the same review
iteration again. Preserve every existing body, assertion, scenario identity,
and API; do not add tests or a new lane.

**Status:** OPEN; no remediation was applied by this reviewer.

## Verdict

**NEEDS_REVISION.** The core types, codecs, deploy refusals, exact typed
errors, current bounded consumers, DES evidence and commit attribution satisfy
the accepted step contract. Approval is blocked solely by CS-01, the five
missing Contract Shape declarations on live transitioned READY tests. The
original crafter must add those declarations without changing any test body;
the same isolated reviewer must record the remediation iteration and final
verdict in this artifact.

## Iteration 2 — CS-01 remediation review

| Field | Value |
|---|---|
| Review date / iteration | 2026-11-01T22:06:23+01:00 / 2 |
| Remediation scope | Five existing rustdoc declarations in `crates/overdrive-core/tests/acceptance/vm_beacon_roundtrip.rs` |
| Remediation classification | Mechanical test metadata only |
| Production/API/design/DES changes | None |
| Reviewer rerun scope | No tests or native checks rerun; the user-supplied remediation receipt records formatting and `git diff --check` pass |
| Iteration 2 verdict | **APPROVED** |

### CS-01 disposition

**CLOSED.** The remediation diff adds exactly one declaration to each of the
five previously transitioned tests:

- `ready_roundtrips_through_display_and_from_str` now carries the exact
  `/// CONTRACT_SHAPE: pure-function.` declaration.
- `ready_message_renders_the_measured_wire_shape`,
  `from_str_tolerates_an_unstripped_trailing_newline_after_ready`,
  `from_str_rejects_ready_with_missing_key_prefix`, and
  `from_str_rejects_ready_with_non_integer_pid` each carry the same exact
  pure-function declaration.

The bounded diff changes no test identity, body, assertion, fixture, scenario,
production source, API, design artifact, roadmap entry, or DES event. The
five declarations now sit directly on the existing live test bodies, and the
remediation preserves the original exact pure-function classification. The
supplied formatting and `git diff --check` receipts pass. No executable test
or native lane was rerun because the remediation changed only rustdoc metadata
and the user explicitly bounded this re-review to that change.

No other iteration-1 finding was open. The implementation, typed-error
mapping, evidence, attribution, test integrity, and current bounded-consumer
assessments remain unchanged from iteration 1.

## Final verdict

**APPROVED.** CS-01 is closed by the declaration-only remediation. Step 11-03
now has a complete on-disk review record with both iterations, and the accepted
core vocabulary, exact wire codecs, deploy refusal mappings, current consumer
closure, and test-contract metadata have no unresolved findings. No source,
test body, API, design, roadmap, DES, or commit change beyond the five
approved declarations was required.
