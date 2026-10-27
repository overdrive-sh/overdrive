# DELIVER review: STEP 13-01

## Review metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `13-01` |
| Review type | Fresh post-commit DELIVER implementation review |
| Reviewed commit | `9c23d8c63b229adf617970fa08ef51d7e6c701b` |
| Parent | `f613be8b3` |
| Commit scope | 23 files, `+3,214/-662` in the commit diff; scoped DESIGN/DISTILL/DES evidence is included in that commit |
| Roadmap contract | Step `[step 8]` mTLS mesh registration and deletion of leg-F's outbound TPROXY accept / cleartext pass-through |
| Reviewer | Fresh independent STEP 13-01 reviewer |
| Iteration | 1 |
| Verdict | **APPROVED** |
| Open findings | 0 |

This review writes only this Markdown artifact. The pre-existing working-tree
`AGENTS.md` edit was preserved and is outside the reviewed commit. No source,
test, DES, roadmap, evidence, or commit file was changed by the reviewer.

## Authority and review method

The review used the current `AGENTS.md`, applicable `.claude/rules` files,
the software-crafter review dimensions, TDD review enforcement and TDD
methodology. The normative implementation contract is the STEP 13-01 entry in
`deliver/roadmap.json`, `feature-delta.md` D9/D14/DG-10, and the accepted
DESIGN/DISTILL handoffs:

- `deliver/design-13-01-postfin-reset-observation.md`, independently approved
  through iteration 4;
- `deliver/distill-13-01-postfin-reset-alignment.md`, independently approved
  through iteration 3;
- `deliver/native-13-01-postfin-reset-observation.md`; and
- the preserved canonical PFRO-1 evidence under
  `spike-scratch/netns-density-295-postfin-reset/increment-a/`.

The accepted step has three outcomes: a registered mesh flow reaches the
owner-selected backend over mTLS without leg-F re-resolution; an unregistered
tuple is refused and never reaches a cleartext path; and leg-F's old outbound
TPROXY accept / cleartext pass-through implementation and its defending tests
are deleted. Internal state layout and task structure were reviewed only where
they affect those public, ownership, lifecycle, security, or wire contracts.

## Contract and implementation assessment

The public surface matches the accepted feature delta exactly:

| Contract | Evidence | Result |
|---|---|---|
| `MtlsInterceptWorker::register_forwarded_outbound(&self, &AllocationId, SocketAddrV4, SocketAddrV4, ResolvedBackend) -> Result<ForwardedOutbound, MtlsInterceptInstallError>` | `crates/overdrive-worker/src/mtls_intercept_worker.rs:3275-3303` | Exact signature; no extra parameter, builder, or alternate method |
| `MtlsInterceptWorker::leg_f_addr() -> Option<SocketAddrV4>` | `mtls_intercept_worker.rs:3307-3311` | Exact existing accessor |
| `ForwardedOutbound` | `mtls_intercept_worker.rs:155-161` | `pub`, `#[must_use]`, private fields, non-`Clone`; no new public accessor |
| Typed registration errors | `mtls_intercept_worker.rs:127-135` | Exact `NotInterceptLive { alloc }` and `DuplicateForwardedRegistration { forwarder_local }` variants |
| Stage projection | `mtls_intercept_worker.rs:855-878` | Exact `not_intercept_live` and `duplicate_forwarded_registration` labels |
| Owner error projection | `guest_network/owner_runtime.rs:1075-1077`, `:863-868` | Registration failure becomes the existing `Refused(IntakeUnavailable)` and existing `guest_flow.refused` surface; no string-carried semantic error replaces the typed cause |

The registry adds a private `forwarded` table keyed by the bound
`forwarder_local`. `register_forwarded` checks the published owner and exact
active allocation before inserting the registration. `claim_forwarded` takes
the registry lock while checking the registration, rejecting a second claim,
claiming the active capability, and marking the registration claimed. This is
the required atomic registration-before-connect / one-claim boundary.

`ForwardedOutbound::Drop` removes only its own registration generation, wakes
retirement waiters, cancels in-flight enforcement, and requests teardown of a
retained enforcement handle. The token remains move-only and is held by the
owner's `OwnedPair`; it is not copied into a second public owner. Allocation
retirement removes that allocation's registrations, marks claimed state
retired, cancels unclaimed state, waits for in-flight claims, drains only that
generation's handles, and preserves unrelated allocations and listeners.

The leg-F accept path now claims only a registered forwarded tuple and passes
the registration's already-selected `ResolvedBackend` to mTLS enforcement.
It does not call `MtlsResolve` again. An absent or already-claimed
registration drops the accepted socket before resolution or enforcement. The
old `OutboundAction` three-arm worker decision, `spawn_cleartext_passthrough`,
relay join bookkeeping, and their tests are removed. The remaining non-mesh
owner branch is the accepted D14 direct kernel-forwarded path; no worker
cleartext fallback is reintroduced.

The production mTLS path arms the existing abortive `SO_LINGER{1,0}` behavior
on outbound legs and preserves join-before-close teardown. The added private
linger helper is the implementation of the already accepted D20 mechanism;
the rejected PFRO alternatives (`TCP_LINGER2`, `TCP_REPAIR`, and privileged
netlink socket destruction) are absent from production. `HostMtlsEnforcement`
still owns both legs and both pumps, while the move-only registration token
only requests the existing enforcement teardown.

The semantic error mapping is complete at the reviewed boundary:

| Failure | Approved representation | Current branch |
|---|---|---|
| allocation has no active capability | `MtlsInterceptInstallError::NotInterceptLive { alloc }` | registration refuses before insertion |
| source tuple already registered | `DuplicateForwardedRegistration { forwarder_local }` | registration refuses without changing the first entry |
| worker shutdown / shared owner unavailable | existing `OwnerShutdown` / `SharedOwner { source }` | checked before registry mutation |
| enforcement rejects a claimed connection | existing `MtlsEnforcementError` inside the existing worker lifecycle | claim is released and the registered path closes; no cleartext fallback |
| token drop after claim | existing async `MtlsEnforcement::teardown` boundary | no new owner-facing error or completion promise is invented |

No typed-error ambiguity or new public error variant is present.

## Test and boundary review

The exact activated marker set is present: 12 `vsock_mesh_egress_mtls` cells,
three `owner_acceptance/policy.rs` cells, and the existing worker stage-label
body. The mesh suite preserves the real `HostMtlsIntercept`, real
`HostMtlsEnforcement`, real allocation activation, real TLS peer, and the
owner's registration boundary. The wire assertions require TLS 1.3
`application_data`, the expected client SPIFFE identity, byte-distinct
request/response behavior, and absence of plaintext request bytes on the
inter-agent wire.

The PFRO-1 correction observes the same peer descriptor that consumed FIN. It
does not read after the first EOF. It waits for exact `TCP_CLOSE` and consumes
`SO_ERROR` once. The three contrasting outcomes remain exact:

| Existing cell | Required result | Current assertion |
|---|---|---|
| claimed drop while both directions are open | reset before FIN | `Termination::Reset` |
| claimed drop after guest-to-remote FIN while reverse remains open | same-flow FIN then reset | `TCP_CLOSE` plus pending `EPIPE` |
| both directions cleanly ended | FIN without reset | `TCP_CLOSE` plus zero pending error |

The three migrated shared-owner fixtures register the exact source-bound socket
before connecting to leg-F and retain the existing token. The migration keeps
the lifecycle, shutdown, drain, listener, node-program, dynamic-member,
traffic, and unrelated-handle assertions. The shared peer helper now accepts
only the observed abortive error class for paths that require reset; it still
rejects handshake, echo, timeout, and unrelated I/O failures.

All transitioned source-local bodies retain their required
`CONTRACT_SHAPE` declarations. The mesh and policy changes activate existing
scenarios; no new scenario, property, model action, lane, public test-support
surface, or expectation runner was authored. The removed worker tests defend
the deleted pass-through path and are the required deletion complement. No
assertion was weakened to accommodate implementation.

## Verification evidence

The receipts are native Metal runs through the repository's durable
`cargo xtask metal run --` route:

| Gate | Result | Receipt |
|---|---|---|
| Worker source-local suite | 33/33 passed | Metal `b56f87efdfbb0966394c9f5e`, nextest `435af33e-746f-4612-89e0-292028d70c13`, `.context/.../worker-source-postretire.log` |
| Required PFRO-1 triple | 3/3 passed | Metal `965310ec95cc6ce433407d28`, nextest `305d03fd-81c6-4e4c-886f-b8380b576e3c`, `final-triple-postclippy.log` |
| Complete mesh integration binary | 12/12 passed | Metal `1410bdb09ac40587eb35ce6b`, nextest `c7378efd-0016-4094-9b6a-61347268031c`, `final-mesh-12-postclippy.log` |
| Approved cgroup-filtered package union | 2,136/2,136 passed, 198 skipped | Metal `08e119c80d2888044aa2b40c`, nextest `3c7a40fa-725a-42f6-bbbf-2fa405faf672`, `package-union-exact55-approved-quint-seed.log` |
| Workspace check | exit 0 | Metal `123eec11a84be62c8beb3ce6`, `final-check-postdistill.log` |
| Workspace Clippy | exit 0 | Metal `758b71d16cbb93b3767ab74f`, `final-clippy-postdistill-const.log` |
| Formatting / whitespace | pass in the recorded source and documentation checks | crafter/DISTILL receipts; `git diff --check f613be8b3..9c23d8c63b229adf617970fa08ef51d7e6c701b` is clean |

The commit retains the original Marcus author, exactly one
`Co-Authored-By: Codex <codex@openai.com>` trailer, and `Step-Id: 13-01`.
`execution-log.json` records the required RED → GREEN → COMMIT sequence for
13-01. No mutation testing was run; it remains the single final DELIVER-wave
gate.

## Reproduced random-seed concern and disposition

The first normal package-union run must remain part of the evidence record. It
ran 2,136 selected tests and failed one randomized conformance test:

```text
quint_quiescence::quiescence_quiesce_conf_randomized
QUINT_SEED=0x651125aa
action `RepFail` is not enabled in the production state:
ControlListener's repair is not parked after the retry tick
```

This was not accepted as “generically pre-existing.” The raw trace was
replayed through the current production composition on Metal:

- bounded replay Metal run `afc5f6d88ca8ab65e300371f`;
- nextest run `59c22a80-0e9e-49dd-ab78-90e9b5a2033b`;
- the same seed, action, production-state error, and failing trace were
  reproduced; and
- the raw trace remains in the replay output and the original
  `.context/netns-density-295/13-01/package-union-exact55-postfocused3.log`.

The causal and authority comparison is decisive for this step. The
`quint_quiescence.rs` blob is exactly
`eb3d212f7a789889a12df24b7e3a8827f08b021f` at both `f613be8b3` and
`9c23d8c63b229adf617970fa08ef51d7e6c701b`; the 13-01 diff touches no
quiescence source, Quint input, driver, model, or owner path. The accepted
STEP 12-01 replacement-proof route pins the unchanged randomized target to
`QUINT_SEED=0x94db98ed`; its exact selected union run
`08e119c80d2888044aa2b40c` passed all 2,136 selected tests. That route and
seed are the existing approved conformance gate for this step's unchanged
package union, and the step's changed D9/D14 tests pass independently in the
worker and mesh gates above.

Therefore this is a reproducible current baseline conformance counterexample
owned by the unchanged quiescence model/driver path, rather than a
STEP 13-01 regression or a justification to weaken D9/D14. It is recorded as
an evidence disposition, not hidden, erased, or converted into a generic
environment failure. No STEP 13-01 remediation, new seed route, issue-based
deferral, model change, or production hardening is authorized by this review.

## Contract Shape Compliance and review gates

| Gate | Result | Evidence |
|---|---|---|
| Exact public API shape | PASS | Registration/token/accessor signatures and private token fields match D9/DG-10 |
| Typed semantic errors | PASS | Registration variants, stage labels, and owner projection are preserved exactly |
| Ownership/lifecycle | PASS | Atomic claim, move-only token drop, generation retirement, handle drain and teardown ownership are bounded and source-traced |
| mTLS wire confidentiality | PASS | Native mesh suite proves authenticated TLS wire, application data, byte-exact response, and no plaintext request |
| D9 registration / D14 fail-closed | PASS | Registered path reaches enforcement; duplicate, unregistered and inactive paths close without resolution or cleartext |
| Deletion discipline | PASS | Worker pass-through action/relay and defending tests are removed; retained shared source membership is the accepted post-cut owner surface |
| Test declarations and integrity | PASS | Existing Contract Shape declarations remain; no assertion weakening, skip addition, mock-dominated SUT, or fixture theater found |
| Production composition boundary | PASS | Tests enter through the worker port and real adapters; no product binary is spawned by Rust tests |
| TDD phase record | PASS | `execution-log.json` contains 13-01 RED, GREEN, COMMIT in order, each `EXECUTED/PASS` |
| Required verification | PASS | Focused native, full mesh, exact filtered union, check, and Clippy receipts above |

## Findings and disposition

No blocking, high, medium, or low findings remain for STEP 13-01. The
reproduced `0x651125aa` quiescence counterexample is explicitly retained as an
out-of-scope baseline evidence note above; it is not a claim that every
unseeded randomized run is green. No design premise, D9/D14 outcome, typed
error, ownership boundary, cleartext prohibition, or reset invariant is
weakened.

## Iteration 1 verdict

**APPROVED.** The commit implements the exact registered mesh mTLS contract,
refuses unregistered tuples fail-closed, removes the worker cleartext path,
preserves the required error and ownership semantics, and proves the
post-FIN reset with same-peer `TCP_CLOSE` plus `EPIPE`. Required native and
mechanical gates pass through the approved route, and the one reproduced
randomized conformance failure is demonstrably unchanged from the parent and
outside this step's contract.
