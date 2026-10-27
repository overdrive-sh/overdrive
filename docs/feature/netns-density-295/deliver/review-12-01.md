# DELIVER review: STEP 12-01

## Review metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `12-01` |
| Review type | Fresh post-commit DELIVER implementation review |
| Reviewed commit | `7105b5807b1dfef5f93a7e3fb338aee45a44de61` |
| Commit event | `9f7befe04c719d098491fdeaa445133bc8cfae24` (DES event only) |
| Parent | `c4961dafe` |
| Reviewer | Fresh independent STEP 12-01 reviewer |
| Review date | 2026-10-27 |
| Iteration | 1 |
| Verdict | **NEEDS_REVISION** |

The review covers the one attachment-cut commit and its bounded compiler,
fixture, infrastructure, formal-evidence, and documentation fallout. The
working-tree `AGENTS.md` edit was preserved and is outside the reviewed
commit. No production, test, DES, or commit file was changed by this review;
this Markdown artifact is the only review output written here.

## Authority and review method

The governing contract was read from the current `AGENTS.md`, all current
`.claude/rules` files, the installed DELIVER/reviewer guidance, the full
`feature-delta.md`, the accepted STEP 12-01 roadmap entry, the accepted
ADR-0187–ADR-0211 contracts referenced by that entry, the eight
`design-12-01-*` artifacts and their reviews, the full
`review-12-01-replacement-proof.md`, and the raw receipts under
`.context/netns-density-295/12-01/`.

The review compared the implementation and public surface against the
accepted design, traced production composition and owner paths, checked the
typed error taxonomy and caller mappings, inspected the transitioned tests
for boundary and oracle honesty, and checked the mechanical commit scope and
attribution. Internal structure was treated as the crafter's choice unless it
violated an observable contract or an ownership/lifecycle invariant.

## Strengths

`praise:` The commit preserves the accepted single-cut shape. The pool,
transport handoff, READY launch attribution, owner admission and flow
forwarding, activation, reclaim, dial, DNS association, quiescence, and
supervisor wiring are composed through the production paths named by DG-2;
the implementation does not introduce a parallel TAP path or an unapproved
public recovery API.

`praise:` The typed error boundary is substantially faithful. The current
source keeps the designed mappings from pool admission (`PoolExhausted`,
`AdmissionCapReached`, `LeaseRetiring`), startup (`FdBudget`, prefix
overlaps, owner placement, `Forwarder`), netlink (`Netlink` with
`FibLookupMissing` at the netlink layer), owner wire/I/O/forwarder failures,
beacon binding (`BeaconBind` → `ControlPlaneError::VmmBoot`), guest transport
setup, dial outcomes, and the exact eight fail-stop causes. No string-only
semantic substitute or extra public error variant was found in the reviewed
paths.

`praise:` The evidence set is current and independently checked. The
replacement-proof review records the durable three-job formal run at
279/279, 28/28 input hashes, 140/140 trace artifacts, and six exact-seed
replays. The recorded gates include the 2,124-test default union, exact
55-guest cgroup run, 7 mTLS relay cells, 40 FwdPair cells, 46 owner-flow
cells, 13 UDP-slot cells, the approved 240-second eight-sample/full-capacity
flow-quota run, quiescence and recovery suites, workspace check/Clippy/format
and conformance lint. No mutation testing was run, as required for a single
step review.

## Contract mapping

| Contract area | Implementation evidence | Assessment |
|---|---|---|
| Pool/CID/launch token | `crates/overdrive-control-plane/src/guest_network.rs:448-664`; `GuestAddressPool::assign` derives CID from the leased offset, draws a non-zero `LaunchToken`, preserves next-fit and release-last state | Conforms to the accepted pool and lease contract |
| Transport handoff and VM caller half | `crates/overdrive-control-plane/src/action_shim/mod.rs:1460-1474`; `crates/overdrive-worker/src/vm_driver.rs:1220-1435` | `AllocationSpec.transport`, leased CID, launch token and guest ports are carried through the production VM start path |
| READY/beacon contract | `crates/overdrive-worker/src/beacon_intake.rs:51-242`; `crates/overdrive-worker/src/vm_driver.rs:1591-1658,2121-2139`; `crates/overdrive-init/src/main.rs:267-270` | Launch-token attribution, live-claim recheck, FIFO accept routing and typed bind refusal are present |
| Shared owner and flow boundary | `crates/overdrive-control-plane/src/guest_network/owner_runtime.rs`; `guest_network/flow_io.rs`; `guest_network/flow_io/host.rs` | Socket handles, forwarder context, route effects and owner ordering stay behind the approved crate-private seam |
| Admission/session/lease generation | `owner_runtime.rs:530-648`, `guest_network.rs:882-1127`, and `flow_owner.rs` | Active/Provisioned/Retiring, stale launch/session refusal, quota and release-last paths are wired |
| Dial/UDP/DNS/forwarder | `guest_network/dial.rs`; `owner_runtime.rs:1027-1291`; `owner_acceptance/{dial,datagrams}.rs`; `flow_io/host.rs:723-863` | `GuestDialError`, gateway:53 datagrams, attach refusal handling and forwarder generation observations match the accepted surface |
| Quiescence/recovery | `guest_network.rs:1136-1497`; `lib.rs:1287-1974`; `core/src/guest_network.rs:240-420` | Counted holds, exact fail-stop set, component recovery and EXEC gate wiring are present; no unapproved kill subsystem remains |
| Typed semantic errors | `crates/overdrive-control-plane/src/error.rs:741-838`; `guest_network.rs:270-374`; `crates/overdrive-netlink/src/error.rs:78-90` | Designed failure conditions map to the designed typed carriers and caller branches |

## Verification evidence

The reviewed commit's recorded evidence was accepted only as evidence for the
current source and current formal input hashes. Historical failed or partial
runs were not used as proof. The following receipts were inspected or
reconciled with the replacement-proof record:

- `.context/netns-density-295/12-01/quint-guest-flow-owner-record-jobs3-durable98-20261027.{log,exit,collect.log,collect.exit}` and `quint-verify-evidence-durable98-20261027.{log,exit}`.
- `.context/netns-density-295/12-01/default-union2124-after-recround1-lint96-20261027.{log,exit}`, the exact FwdPair/owner/UDP/quiescence/recovery/quota receipts, and the cgroup55 and mTLS relay receipts.
- `.context/netns-density-295/12-01/replacement-proof-*-current-20261027.{log,exit}` for the six stale exact-seed replays.
- `.context/netns-density-295/12-01/workspace-check-final-after-recround1-20261027.{log,exit}`, `workspace-clippy-final-after-recround1-20261027.{log,exit}`, `fmt-check-after-recround1-20261027.{log,exit}`, `dst-lint-after-recround1-20261027.{log,exit}`, and `conformance-lint-after-fwd-hazard-packaging98-20261027.{log,exit}`.

The step commit retains Marcus as author, exactly one
`Co-Authored-By: Codex <codex@openai.com>` trailer, and `Step-Id: 12-01`.
The follow-on commit contains only the DES commit event. The commit scope and
the approved durable Metal execution support were checked against the
roadmap's bounded fallout allowance.

## Findings

### issue (blocking): the accepted public `VsockBeaconIo::new` API lost its required `const` qualifier

**Location:** `crates/overdrive-host/src/beacon_io.rs:56-69`.

The accepted DG-4-CTOR contract requires the public constructor to be exactly
`pub const fn new() -> Self`. The authoritative contract states this in the
`feature-delta.md` beacon adapter code fence at lines 11689-11699 and repeats
the DG-CONST decision at lines 11701-11705. The current implementation is:

```rust
pub fn new() -> Self {
    Self {
        instance: NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed),
        state: Mutex::new(State::default()),
    }
}
```

The public API therefore does not match the accepted design even though the
ordinary runtime callers and the current test suite compile. The production
composition entry point reaches this constructor through
`NodeAdapters::host`/the beacon adapter, so this is part of the shipped
cross-crate API rather than an unused declaration. The implementation's
`AtomicU32::fetch_add` and runtime mutex initialization explain the current
shape but do not authorize relaxing the contract or adding a second
constructor.

The required remediation is to restore the exact existing public shape,
`pub const fn new() -> Self`, while preserving the design's no-I/O/no-bind
constructor semantics and the current private handle isolation. The crafter
must choose the internal representation needed to make that exact signature
const-compatible; no new public method, flag, constructor, or adapter seam is
authorized.

**Impact:** API-contract divergence; callers that rely on the approved
const-construction guarantee cannot use the shipped type as designed. This is
a blocking design-compliance finding independent of the green runtime gates.

**Remediation disposition:** OPEN. Return to the original STEP 12-01 crafter
for the exact API correction, then re-run this review iteration against the
updated source and the required bounded verification. Do not weaken the
contract or modify the accepted design.

## Hypotheses explicitly not promoted to findings

The review also inspected two ordering-sensitive areas that would require a
reproducible seeded invariant or a real production-entry regression before any
remediation could be required:

- `HostGuestStream::Drop` at `crates/overdrive-control-plane/src/guest_network/flow_io/host.rs:1021-1030` resolves the current forwarder context while the stream stores a generation. A cross-generation drop would be a concern only if the real recovery owner path allowed an old stream to survive its watch abort and then collide with a new-context ID. The current checked formal and owner receipts do not reproduce that path, and no test-only abort or theoretical schedule is accepted as a defect.
- `HostSharedGuestNetworkOwner::quiesce` at `guest_network.rs:1340-1470` closes the owner table's pair handles after `abort_all`; the accepted recovery contract also discusses draining queued records before context release. The current evidence does not contain a failing seeded production-composition invariant for a clean completed record queued at recovery entry, so this remains an unproven hypothesis and does not prescribe a fix in this review.

Neither hypothesis is a required remediation or a design change. The exact
const API mismatch above is independently proven from the normative contract
and current source.

## Iteration 1 disposition

The implementation has strong evidence coverage and its major ownership,
lifecycle, typed-error, and test-boundary decisions align with the accepted
contracts. It cannot be approved while the public beacon constructor has the
wrong exact signature.

**Verdict: NEEDS_REVISION.**

## Review iteration 2 — remediation re-review

### Remediation under review

The original STEP 12-01 crafter committed `83419d30825f359b5c6892d26f41b7ca5e096753`,
whose only changed path is `crates/overdrive-host/src/beacon_io.rs`. The commit
retains Marcus as author, the exact Codex co-author trailer, and `Step-Id:
12-01`. It does not modify the formal inputs, model, scenarios, public APIs
other than restoring the accepted qualifier, or any unrelated source/test
path.

`VsockBeaconIo::new` is now exactly:

```rust
pub const fn new() -> Self {
    Self { instance: OnceLock::new(), state: Mutex::new(State::new()) }
}
```

This matches the accepted DG-4-CTOR/DG-CONST code fence and the explicit
contract that construction is infallible, socket-free, and does not bind the
listener. The private `State::new` is const-compatible and initializes an
empty listener, bound-port slot, connection table, and sequence counter.

### Private isolation and lifecycle recheck

The remediation preserves the required internal behavior while keeping it
private:

- The per-adapter instance identity is allocated lazily once through the
  private `OnceLock<u32>`, so const construction performs no runtime allocation
  or kernel operation.
- `handle` encodes that private instance identity with the private connection
  sequence; `sequence` rejects a handle from another adapter before looking up
  the connection. The `connections` table remains private and is keyed by the
  local sequence.
- `accept` creates and stores a connection before returning its opaque
  `BeaconSocket`; `read_line` and `write_line` still use the per-connection
  locks and buffered line protocol; `close` still removes the connection and
  shuts down the owned socket.
- `bind` remains idempotent and continues to be the only operation that opens
  and stores the listener. `new` has no `Default` implementation and no public
  identity or state accessor was added.
- The core `BeaconSocket::from_raw` and `raw` methods remain the accepted
  `const` adapter primitives. No new public method, type, field, constructor,
  or seam was introduced.

The private identity is assigned on the first returned/consumed handle rather
than at construction. That is an internal implementation change permitted by
the interface-ownership rule; the observable handle isolation and lifecycle
contract remain unchanged.

### Iteration 1 finding disposition

`issue (blocking):` The missing `const` qualifier is **CLOSED**. Current
source at `crates/overdrive-host/src/beacon_io.rs:61-71` has the exact
`pub const fn new() -> Self` shape required by `feature-delta.md:11689-11705`.
The current private state initializer is const-compatible, and the lazy
identity allocation preserves the no-I/O constructor guarantee. No contract
weakening or public-surface invention was used.

### Iteration 2 verification

The crafter's bounded remediation receipts were checked:

- `review-remediation-vsockbeacon-const-check-20261027.{log,exit}` — Metal
  `cargo check -p overdrive-host`, exit 0, run
  `add755b45cd87c1426eed333`.
- `review-remediation-vsockbeacon-const-clippy-20261027.{log,exit}` — Metal
  `cargo clippy -p overdrive-host --lib -- -D warnings`, exit 0, run
  `de95536a502f6ee89d805215`.
- `review-remediation-vsockbeacon-const-fmt-r2-20261027.{log,exit}` — Metal
  `cargo fmt --check --package overdrive-host`, exit 0, run
  `27fafe2149c849c2da88a02d`.

The first formatting receipt for the remediation recorded a formatting diff
and exit 1; the required bounded rerun applied the formatter result and passed
without changing the contract. No new test, scenario, API, seam, model input,
or formal run was needed for this exact signature correction. The previous
iteration's current-source formal, conformance, native, workspace, and typed
error evidence remains valid because this remediation changes only the private
constructor representation and the previously failing API qualifier; the
reviewed commit does not touch those inputs or execution paths.

The two hypotheses retained from iteration 1 remain unproven and are not
findings or remediation requirements:

- The generation-specific `HostGuestStream::Drop` concern still has no
  failing seeded invariant or real production-entry regression.
- The queued clean-end record during forwarder recovery still has no failing
  seeded production-composition invariant.

Per the repository's falsification and reachability gates, neither is
promoted without that evidence.

## Final verdict

`praise:` The exact API correction is small, contract-directed, and preserves
the private adapter isolation and no-side-effect construction lifecycle. The
remediation receipts are green and the commit scope is limited to the finding.

No blocking findings remain. The accepted public API, typed error mapping,
ownership/lifecycle boundaries, test/evidence boundaries, and current formal
receipts satisfy the STEP 12-01 contract.

**APPROVED.**
