# DESIGN review — E11 current Stable observation, ADR-0101 revision 6

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| Amendment under review | `docs/feature/service-kind-vm-workloads/design/amendment-e11-stable-observation.md` |
| ADR under review | `docs/product/architecture/adr-0101-service-backend-health-observed-convergence.md`, proposed revision 6 D9 |
| Related feature contract | `docs/feature/service-kind-vm-workloads/feature-delta.md`, E11 / DDD-16 |
| Related decision record | `docs/feature/service-kind-vm-workloads/design/wave-decisions.md`, ACD-10 |
| Roadmap boundary | DELIVER step `03-01`, E11 readiness recovery |
| Review type | Fresh isolated DESIGN review |
| Reviewer | `/root/e11_stable_observation_design_reviewer` |
| Model | User-selected GPT 5.6 Luna, maximum thinking |
| Review date | 2026-09-09 |
| Iteration | 1 |
| Verdict | **APPROVED** |

## Scope and review boundary

This is a focused review of the proposed ADR-0101 revision-6 observation
amendment. It verifies the demonstrated public-surface omission, the exact
additive API and renderer contract, current-versus-prior terminal semantics,
ownership and ordering boundaries, compatibility and privacy consequences,
alternatives, and the required implementation/native-evidence handoff.

The review does not approve implementation, the E11 expectation, the native
capture, the evidence-audit verdict, ADR-0101 revision 5, or the 03-01
roadmap step. Revision 5's readiness-wake decision remains a separate design
gate, as the amendment and ADR explicitly state. No production, test, example,
expectation, evidence, DES, roadmap, or unrelated dirty file was edited. No
test, native run, mutation run, staging operation, or commit was performed.
The required review artifact is the only file written by this review.

I read the mandatory project files (`CLAUDE.md` and all seven
`.claude/rules/{bpf,debugging,design,development,rust,testing,verification}.md`)
and the relevant architecture-review guidance (`nw-review-workflow` and
`nw-sar-critique-dimensions`). I independently read the complete amendment,
ADR-0101, feature-delta, wave decisions, roadmap step 03-01, the fresh E11
recapture blocker and retained evidence, the prior independent E11 evidence
audit, and the production source paths named below.

## Reviewed artifact pins

These hashes identify the design and evidence inputs reviewed in this
iteration.

| Artifact | SHA-256 |
|---|---|
| `design/amendment-e11-stable-observation.md` | `cc1ae2dd157b8cfa5e9ad745080126adaefce5bcc96e62b65150409ea9898505` |
| `adr-0101-service-backend-health-observed-convergence.md` | `9d288f50a541ba2b5cb5f9b9fb14c7b4885b09638c10e1bc6b6d17a5fbd6d0b0` |
| `feature-delta.md` | `f16a4a268d8eadea964a156180947efbb3a67c3301ccf1f2b8c84edaf87b7f65` |
| `design/wave-decisions.md` | `53b8e76d80c16bee4839e2cf1cfcb7d40753861aa1bdd593d04a58d5e4196fd2` |
| `deliver/roadmap.json` | `56e279e133c15309de4248e727040e69481ac40af730c2f2852a57c20067e3d1` |
| `deliver/review-e11-evidence.md` | `0e46ddd851e83bef1c7ce7edc077bd72f22b88ddf22bb225899db7fc9bd14b71` |
| `verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/recapture-blocker.md` | `350bbf07556e78d45ec6b964f0acde858b3628f6d6b9b1a2f0528ff071e08400` |
| `verification/expectations/E11-vm-service-readiness-traffic-recovery/runner.sh` | `7b0bd12efe5324bc751e66498665c7aee79357187ff17841e0e9e8895d97d5a6` |
| `examples/service-kind-vm-workloads/run-example.sh` | `22b6c44147238d8b1ee3f0d129fa98d7dcb7c4210ee10e4ad52f1f9c861f57ad` |

The reviewed production source identities were also pinned during inspection:
`api.rs` `361ab745a5cc7951a769c9890be22ebeb0f90bc9df4383469994cb0aebe27f24`,
`handlers.rs` `b2b18a51433417a1a7e1a6c847cd0f771ff3d0ab70970b51a57f030b5adec842`,
`render.rs` `e9c11190955fcd55a34974d6f19b8ab876bc880065235bed337c005d25a73844`,
`service_lifecycle.rs` `5b1d857858d67a329680c777dbbc1b5f7c6238b772d4dbb178adc286eddfeea1`,
`action_shim/mod.rs` `7b0aeb789b8d2b9654dfbeab0d3cd19b0132ade45b0e1b73fc96c9d397f6c729`,
`observation_store.rs` `fca016ee58519435a00bcd46c73de2d724b714ad55055c2d2783a6ed28d63c45`,
`transition_reason.rs` `cf54a66defa51a8d5460b97b2e287c0fdf3e5a374c1d2b9558dc8755bf9bea81`,
and `service_kind_vm_terminal_invariant.rs`
`a272301cce988b84c2dbe6d6f1970c4f10cc636f8a7f38f73569162ceea9f62d`.

## Necessity and revalidated production path

**Result: PASS.** The amendment addresses a concrete, reachable omission in
the existing operator observation boundary. It does not treat the missing
line as evidence that the lifecycle fact is absent.

The fresh native-metal attempt at `2026-09-09T16:45:19Z` directly proves the
non-observation predicates: readiness `Pass -> Fail -> Pass` at `338 ms`,
`195 ms`, and `160 ms` (`verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/attempt-20260909T164519Z/readiness-recovery.tsv:2-4`);
the same allocation is `Running` with `0` restarts in the three public
descriptions (`verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/attempt-20260909T164519Z/product-run.out:36-38,72-74,108-110`); the peer results are
exact reply, unreachable/no exact reply, and exact reply; and teardown has a
zero delta (`verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/attempt-20260909T164519Z/product-run.out:148-151`). The receipt identifies the built
default-feature binary and native-metal substrate
(`verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/attempt-20260909T164519Z/verification.yaml:1-14`).

The same transcript proves the gap rather than closing it: `Stable` occurs in
the initial submit stream only (`verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/attempt-20260909T164519Z/product-run.out:22-32`), while the during
and after public descriptions (`verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/attempt-20260909T164519Z/product-run.out:69-88,105-124`) show
readiness and `Running` without a current Stable line. The extracted ledger
has no terminal column (`verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/attempt-20260909T164519Z/readiness-recovery.tsv:1-4`). The independent
audit therefore correctly returned **NEEDS_RECAPTURE** for the full E11
contract (`docs/feature/service-kind-vm-workloads/deliver/review-e11-evidence.md:145-156,271-295`),
and the retained blocker correctly identifies the absent public API projection
(`verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/recapture-blocker.md:31-68`).

The complete current owner and caller path is grounded in production source:

1. `ServiceLifecycle` emits the existing
   `Action::FinalizeFailed { terminal: Some(TerminalCondition::Stable { .. }) }`
   for a Running allocation after startup Pass
   (`crates/overdrive-reconcilers/src/service_lifecycle.rs:576-598`). The
   existing `TerminalCondition::Stable` is documented as a fully-probed
   Running Service claim, with its settled duration and `ProbeWitness`, not as
   a lifecycle-terminal state (`crates/overdrive-core/src/transition_reason.rs:611-688`).
2. The FinalizeFailed action-shim arm threads the exact terminal value onto
   the durable allocation row and lifecycle event
   (`crates/overdrive-control-plane/src/action_shim/mod.rs:1520-1527`). Its
   Stable path keeps the prior row state Running, writes the row, retires only
   startup supervision, and does not perform genuine-terminal cleanup
   (`crates/overdrive-control-plane/src/action_shim/mod.rs:1637-1672,1680-1715,1720-1750`). The durable
   `AllocStatusRowV3` already owns `terminal: Option<TerminalCondition>`
   (`crates/overdrive-core/src/traits/observation_store.rs:1412-1420`).
3. Readiness is owned separately. The existing readiness projection changes
   `ServiceBackendRow.backends[*].healthy` and its consecutive-pass input; it
   does not rewrite the allocation row's state, terminal claim, restart count,
   or lifecycle history (`crates/overdrive-reconcilers/src/service_lifecycle.rs:1084-1217`).
4. The seeded production-owner-path composition observes the baseline Stable
   claim, then asserts the same claim after readiness Fail and Pass, with the
   same allocation ID, Running state, zero restarts, unchanged history, and
   continuing supervision (`crates/overdrive-sim/tests/acceptance/service_kind_vm_terminal_invariant.rs:553-603,604-659,668-703`).
   This is an owner-path assertion, not a fabricated replacement row.
5. The existing `GET /v1/allocs?job=<id>` handler already reads the current
   allocation snapshot and the CLI client already consumes it
   (`crates/overdrive-control-plane/src/handlers.rs:1219-1257`; `crates/overdrive-cli/src/http_client.rs:322-339`).
   The current `From<AllocStatusRow>` projection omits `row.terminal` from
   `AllocStatusRowBody`, while retaining the semantically different nested
   `last_terminated.terminal`
   (`crates/overdrive-control-plane/src/api.rs:376-456`; `crates/overdrive-control-plane/src/handlers.rs:95-173`).
6. The existing Service renderer prints the allocation table, cause details,
   and prior-terminal details but has no current Stable branch
   (`crates/overdrive-cli/src/render.rs:1095-1145`). The existing command is a
   single snapshot request with no replay or output-mode argument
   (`crates/overdrive-cli/src/cli.rs:100-109`; `crates/overdrive-cli/src/commands/workload.rs:114-224`).
   The submit stream closes after its submit result and is not a post-submit
   snapshot surface (`crates/overdrive-control-plane/src/streaming.rs:886-985`).

This is a complete reachable production path in the default composition. The
amendment does not rely on a test-only state, a forced cancellation, a second
allocation, a scheduler hypothesis, or an internal row read substituted for
operator evidence.

## Exact API and rendering contract

**Result: PASS.** The proposed shape is closed, mechanically implementable,
and matches the design's exact public API. No implementation latitude requires
inventing a method, type, enum variant, trait, parameter, endpoint, or second
field.

The sole public DTO addition is appended after `last_terminated`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub terminal: Option<overdrive_core::transition_reason::TerminalCondition>,
```

The existing handler assignment is exactly `terminal: row.terminal`. The
existing route, response envelope, `WorkloadCommand::Describe { id }`,
`DescribeArgs`, `WorkloadDescribeOutput`, and
`ApiClient::alloc_status_for_workload(&str)` signature remain unchanged. The
existing `TerminalCondition` is already a serde- and `ToSchema`-derived,
tagged type (`crates/overdrive-core/src/transition_reason.rs:600-688`), and
the current `LastTerminatedBody` already establishes the same optional typed
terminal projection (`crates/overdrive-control-plane/src/api.rs:426-456`).

The renderer contract is also closed: retain the exact Service table columns
and order; immediately after each Service allocation row and before the
existing cause/`last terminated` blocks, emit exactly one
`    terminal: Stable` line if and only if that row's current claim is
`Some(TerminalCondition::Stable { .. })`. Emit no line for `None` or another
variant; do not print the settled duration, witness, opaque detail, Debug
format, or a table column. Job and Schedule render branches are not changed.

The current/prior distinction is precise and testable:

| Field | Meaning | Review disposition |
|---|---|---|
| `rows[i].terminal` | Typed claim carried by the current LWW allocation row | Correct source for the new field; mechanically moved into the DTO |
| `rows[i].last_terminated.terminal` | Prior terminal observation the current allocation survived | Must remain nested and untouched; it is not a substitute for current Stable |

The design correctly forbids deriving Stable from `AllocStateWire::Running`,
readiness Pass, `ProbeResultRow`, initial-stream history, or the ledger. A
genuine later terminal/replacement row may replace the current claim; the
snapshot must report that current row rather than carry Stable forward.

Adding the field requires compiler-required updates to existing
`AllocStatusRowBody` struct literals and the generated OpenAPI artifact. Those
are bounded mechanical fallout from the named field and existing `ToSchema`
derive, not permission to alter the public shape or add a second abstraction.

## Serialization, compatibility, privacy, and non-Service consumers

**Result: PASS.** The compatibility claims are scoped accurately.

- Appending the field preserves the existing serialized field order. With
  `skip_serializing_if = "Option::is_none"`, a row whose current claim is
  `None` retains its prior JSON bytes; a row with a claim intentionally gains
  the new key. `serde(default)` lets a new client deserialize old payloads,
  and the existing non-deny deserializers let old clients ignore the additive
  key.
- When present, the value is the unchanged tagged serde representation of
  `TerminalCondition`; it includes the existing Stable `settled_in_ms` and
  every `ProbeWitness` field, rather than a hand-written or lossy Stable
  object. No rkyv row, redb table, LWW stamp, lifecycle occurrence, or
  migration changes.
- The shared DTO means a Job or Schedule response can carry the additive
  optional field when its current row has a typed terminal claim. This is an
  explicitly disclosed wire addition, not a claim that all Job/Schedule JSON
  bytes remain unchanged. Their render branches, verdict logic, tables,
  lifecycle behavior, and public request shapes remain unchanged; only the
  Service renderer emits the new plain-text Stable line.
- The field reuses a terminal payload already exposed by the submit stream and
  by `LastTerminatedBody` in the existing authenticated snapshot response. It
  introduces no credential, endpoint, log, redaction, or privacy category.
  Implementation review must verify the existing endpoint privacy policy and
  must not add full-payload logging or a new redaction convention.

## Lifecycle, ordering, retry, cancellation, and effect boundaries

**Result: PASS.** The amendment is read-only at the HTTP/CLI boundary and does
not alter a lifecycle owner or effect protocol.

| Boundary | Verified design behavior |
|---|---|
| Startup Stable publication | Existing ServiceLifecycle and action-shim ordering remains authoritative; the new field is visible only after the existing row write accepts the claim. |
| Readiness Fail/Pass | Existing readiness writes affect only `ServiceBackendRow.healthy`; they do not rewrite the allocation's current terminal, state, restart count, or history. |
| Row to HTTP | Existing handler reads the same snapshot and copies `row.terminal` with no second store read, join, retry, cache, or ordering barrier. |
| HTTP to CLI | Existing `workload describe` performs one async request with existing transport/body-decode errors; no detached task, polling loop, replay, or new retry policy is added. |
| Concurrent LWW update | The response is a point-in-time current-row observation. A genuine terminal or replacement row winning before the read is reported as that row; no stronger monotonicity or cross-allocation consistency promise is made. |
| Cancellation/shutdown | No future, task, cleanup, broker, probe, or consumer acknowledgement is added. Existing request cancellation and server shutdown outcomes remain in force. |
| Unreadable row / `None` | Existing typed read and serialization failures remain failures. `None` means a readable current row has no claim and is never an unreadable-row substitute. |
| Downstream consumers | Mesh, DNS, and dataplane consumers continue to consume `ServiceBackendRow` asynchronously; the operator projection neither waits for nor infers their application. |

The lifecycle gate matrix is explicit: Running remains owned by the existing
action shim/driver, Stable remains a ServiceLifecycle startup claim,
`Backend.healthy` remains ServiceLifecycle readiness output, liveness remains
ServiceLifecycle-owned, and restart/finalization remains WorkloadLifecycle's
sole authority. The observation field cannot gate, redefine, or clear any of
those states.

## Alternatives and priority validation

**Result: PASS.** The selected option is the smallest source-direct projection
that closes the proven public-surface gap. The amendment rejects each tempting
source of semantic drift:

| Alternative | Disposition |
|---|---|
| `stable: bool` / `is_stable` | Rejected: duplicates a typed source of truth, loses settled/witness data, and invites readiness-to-Stable inference. |
| `current_terminal` with the same type | Rejected: duplicates the established current-row field name; the row/prior distinction is already represented by nesting. |
| New `StableObservation`, `TerminalKind`, wrapper, or enum variant | Rejected: invents public surface and can drift from existing serde/ToSchema and core semantics. |
| `last_terminated.terminal` | Rejected: it is the prior terminal observation the allocation survived and has the opposite temporal meaning. |
| Readiness Pass as Stable | Rejected: readiness is a post-Stable eligibility input; Fail must not erase Stable and Pass cannot prove startup Stable. |
| New describe route/flag/JSON mode or submit-stream replay | Rejected: the existing authenticated snapshot already reads the authoritative row; replay adds protocol surface and remains an indirect source. |
| Ledger or initial transcript label | Rejected: it would fabricate an operator observation outside the production boundary and cannot prove post-transition state. |
| Top-level replica Stable aggregate | Rejected: Stable is allocation-scoped and mixed-replica aggregation would require an unapproved policy. |

The priority is supported by both evidence layers: native E11 proves the
stakeholder-facing traffic/readiness journey while omitting the direct Stable
observation, and the seeded owner-path composition proves the durable current
claim survives the readiness flap. No broader persistence, broker, cadence,
recovery, consumer, or lifecycle architecture is needed for this omission.

## Verification and evidence handoff

No implementation or verification result is claimed by this design review.
After this amendment and the separate revision-5 gate are approved, the
original 03-01 crafter must implement only the named shape and the bounded
fallout. Its implementation review must provide independent in-process
evidence for:

1. Stable payload conversion is exact, including `settled_in_ms` and every
   `ProbeWitness` field; `None` remains `None`.
2. JSON omits `terminal` for `None`, emits the unchanged tagged enum for Stable,
   and accepts both new payloads and old payloads without the field.
3. Service rendering preserves the table and emits exactly one line per
   current Stable row, with no line for `None` or another variant; Job and
   Schedule rendering remains unchanged.
4. The real production-owner composition proves readiness Fail and Pass leave
   the same current terminal claim, Running state, allocation identity, zero
   restart count, and lifecycle history; a replacement Stable row must not be
   seeded merely to exercise conversion.
5. Tests remain in-process and carry the repository's required contract-shape
   declarations, including the exact
   `/// CONTRACT_SHAPE: pure-function.` rustdoc line on every live source-local
   pure-function property. The black-box boundary must not be replaced by a
   test binary or a crate import.

The generated OpenAPI schema/check is bounded compiler-required fallout from
the existing `ToSchema` derive and should be regenerated/checked with the
repository's normal gate. This does not add a schema type or a new endpoint.

After implementation and its review, the checked-in
`readiness-recovery` example must be rerun through the built default-feature
binary on native metal. The fresh transcript and extracted ledger must carry
the direct current Stable observation from each of the three existing public
`workload describe` responses, not from the initial submit stream. The ledger
header is pinned exactly as:

```text
phase	readiness	terminal	observed_at_ms	detected_at_ms	transition_latency_ms	client_started_at_ms	client_elapsed_ms	lifecycle	restarts	peer_result
```

For `before`, `during`, and `after`, `terminal` must be exactly `Stable`,
extracted from that phase's `terminal: Stable` line in the same response as
the expected readiness result. A constant, initial-stream carry-forward,
internal row read, separate later request, or readiness-derived boolean does
not satisfy the observation. The existing proof must still show `Running`,
restart `0`, exact-reply / unreachable-no-exact-reply / exact-reply, both
transition latencies at most `2,000 ms`, and the complete zero teardown delta.

The evidence audit must be independent of implementation tests and must inspect
the fresh transcript, ledger, runner/README contract, metadata, provenance,
cleanup, and retained attempt inventory. The unrecoverable earlier failed
attempt is now explicitly retained as
`evidence/attempt-20260909T091209Z/EXCLUDED.md`, while the earlier successful
partial capture and the fresh attempt retain their executable artifacts. The
future auditor must preserve that exclusion rather than promote DES prose to
executable evidence. It must record its new verdict in the existing convention
at `docs/feature/service-kind-vm-workloads/deliver/review-e11-evidence.md`.
The prior **NEEDS_RECAPTURE** disposition remains historical until that fresh
audit returns **SATISFIED**; it is not an approval of this design or of a
future run.

## Findings and dispositions

No critical, high, medium, or low design finding remains within the amendment's
declared boundary. The following potentially adverse observations were
examined and are accepted as explicit limits or bounded implementation duties,
not hidden defects:

| Review observation | Disposition |
|---|---|
| The shared DTO may add a terminal key to Job/Schedule JSON when populated. | Accepted: the amendment explicitly discloses this additive wire effect; it changes neither their render branches nor lifecycle semantics, and avoids inventing a second DTO. |
| `serde(default)` / skip-if-none does not make a payload with a present terminal byte-identical to the old payload. | Accepted and correctly scoped: rows with `None` remain byte-identical; present current claims are the intentional new observation. |
| A snapshot can race a genuine LWW row update. | Accepted: the amendment promises only the current point-in-time row and explicitly makes no monotonicity, history, or cross-replica guarantee. |
| Existing `TerminalCondition::Custom.detail` is opaque. | Accepted: the same typed payload already exists on current/prior terminal surfaces; no new logging or reinterpretation is authorized, and endpoint privacy remains an implementation-review check. |
| Existing public struct literals and OpenAPI output require updates. | Accepted as tightly bounded compiler/schema fallout from the exact appended field, not an API expansion. |
| The fresh native evidence is partial and the earlier failure is not passing evidence. | Correctly retained as a post-implementation recapture and independent-audit obligation; the design makes no green or satisfied claim. |
| Revision 5 remains pending. | Correctly preserved as a separate design gate; revision 6 does not approve, rewrite, or replace the readiness-wake amendment. |

None of these dispositions authorizes a new persistence system, broker wake,
retry/cadence mechanism, lifecycle owner, consumer acknowledgement, recovery
protocol, route, CLI option, or derived health field.

## Iteration history

| Iteration | Reviewed revision | Verdict | Findings | Remediation disposition |
|---:|---|---|---|---|
| 1 | ADR-0101 revision 6 D9 / E11 current Stable observation amendment | **APPROVED** | None remaining in scope | No remediation. The exact DTO field, mechanical handler copy, Stable-only Service line, current/prior semantics, ownership limits, and evidence obligations are ready for the separately owned implementation and review workflow. |

## Final verdict

**APPROVED.** The amendment is necessary for a reproduced operator-observation
gap: the existing production owner already writes and preserves the typed
`TerminalCondition::Stable` claim on the current Running allocation row through
readiness Fail/Pass, while the existing snapshot DTO and Service renderer drop
it. The selected additive
`AllocStatusRowBody.terminal: Option<TerminalCondition>` projection and exact
`terminal: Stable` Service line close that gap without changing lifecycle
ownership, readiness policy, state, restart/finalization, persistence, route,
retry, cancellation, consumer convergence, or history semantics.

This verdict approves only the proposed revision-6 design. DELIVER step 03-01
remains blocked until the separate revision-5 review is approved, the original
crafter implements and passes its independent review, and a fresh native E11
recapture plus independent evidence audit satisfy the full roadmap contract.
