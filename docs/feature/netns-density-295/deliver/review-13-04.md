# DELIVER review — STEP 13-04 firewall single cut and owner revalidation

## Review metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `13-04` |
| Review ID | `STEP13-04-DELIVER-POSTCOMMIT-01` |
| Review date | 2026-10-28 |
| Reviewer | Fresh isolated post-commit DELIVER implementation reviewer |
| Iteration | 1 |
| Implementation range | `b8bef6fbbdc576e9a8d06892c9dfa1de303a9aa4..ec3bdc484f176638e66dd1568a77a53ddd7d9d39` |
| DES commit | `0bb78e4731f7d39eae5e91aa149c6316837c29ee` |
| Final verdict | **NEEDS_REVISION** |
| Open findings | 1 blocking verification finding |

This review applies the repository `AGENTS.md`, the applicable Rust, testing,
verification, development, workflow, and DELIVER rules, and the installed
software-crafter review dimensions. The repository's native Markdown review
artifact convention takes precedence over the installed review skill's legacy
YAML output and two-iteration cap.

Only this review artifact is written. No production source, executable test
body, DES event, roadmap, design artifact, spike source, host state, or commit
was changed by the reviewer. The pre-existing modified `AGENTS.md` remains
untouched.

## Authority and review boundary

The authoritative contract is STEP 13-04 in
`docs/feature/netns-density-295/feature-delta.md`, ADR-0125 revalidation,
ADR-0172, ADR-0184, ADR-0137, the roadmap entry at
`docs/feature/netns-density-295/deliver/roadmap.json:1352-1400`, and the
approved DISTILL handoff and review:

- `docs/feature/netns-density-295/deliver/distill-13-04-legc-fixture-alignment.md`
- `docs/feature/netns-density-295/deliver/review-distill-13-04-legc-fixture-alignment.md`

The relevant artifact hashes are:

| Artifact | SHA-256 |
|---|---|
| `feature-delta.md` | `66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09` |
| `deliver/roadmap.json` | `2a909f49f0edc671c989228def9daac8c3d7fb41763d55d7359ea7471c03875a` |
| `deliver/distill-13-04-legc-fixture-alignment.md` | `cdba5e1a5785d98554e78090e1cfbdcec67fc2d4b0cdf21c8b0669d79296b39c` |
| `deliver/review-distill-13-04-legc-fixture-alignment.md` | `a1b1b06cf58316f8c367a4b4f0bf83d717b4d58dd463249d82ca4e37953228da` |
| final native receipt | `7c5d26ce375d4a26fef94c839d9983ab0a738ae303a1ea85b1f97d4bfc935fbe` |

The accepted scope requires:

1. six constant IPv4 rules in their fixed order: leg-S exemption, registered
   leg-C prerouting divert, guest-prefix prerouting drop, leg-S exemption,
   registered output mark/divert, and guest-prefix output reject;
2. one `inbound_destinations` set keyed by IPv4 plus TCP port;
3. removal of rules 2–3, `outbound_sources`, `managed_guest_ips`, their
   element lifecycles, and the R18 guard/read-back;
4. gateway exclusion and the exact output-reject/prerouting-drop behavior;
5. awaited, convergent element release and forwarded-registration teardown,
   failure retention and retry, unrelated-member isolation, and boot clearing;
6. typed boot-table-write refusal before forwarding opens, with a supervised
   restart retry; and
7. the existing leg-C bidirectional path and the exact native firewall cells,
   without pulling forward STEP 13-05 gateway-route/leg-B behavior or STEP
   13-06 host-internal deny behavior.

## Commit and DES integrity

The implementation commit has the unchanged Marcus author, exactly one
`Co-Authored-By: Codex <codex@openai.com>` trailer, and `Step-Id: 13-04`.
The DES persistence commit has the same author, the same exact Codex trailer,
and the same step trailer. No Claude, Anthropic, or generated-by attribution
appears in either commit.

The persisted execution log records the actual ordered history at
`docs/feature/netns-density-295/deliver/execution-log.json`:

| Phase | Result |
|---|---|
| RED | PASS |
| GREEN | FAIL, four recorded attempts |
| GREEN | PASS |
| COMMIT | PASS |

The failed GREEN entries are legitimate recorded attempts, followed by the
successful GREEN and COMMIT. No phase was hand-written by this reviewer.

The implementation commit's 29-file scope contains the expected production,
test-support, acceptance, native-runner, and documentation changes. The only
worktree change before this review was the user-owned `AGENTS.md` edit.

## Contract and implementation assessment

### Constant nft program

The private encoder in `crates/overdrive-netlink/src/nft.rs` now builds the
constant program with three prerouting rules and three output rules. The rule
selection is at `nft.rs:713-779`:

| Position | Chain | Program |
|---:|---|---|
| 0 | prerouting | exact leg-S mark exemption |
| 1 | prerouting | exact `(destination, TCP port)` lookup, TPROXY to leg C, mark, accept |
| 2 | prerouting | guest-prefix match excluding `GUEST_NODE_GATEWAY`, drop |
| 0 | output | exact leg-S mark exemption |
| 1 | output | exact registered destination lookup, leg-S guard, mark, accept |
| 2 | output | guest-prefix match excluding `GUEST_NODE_GATEWAY`, plain reject |

`e_guest_prefix_match` compares the fixed `/16` prefix and then excludes the
gateway address at `nft.rs:792-798`. The output reject uses the plain
ICMPX-port-unreachable form at `nft.rs:800-808`, which the native cells
observe as TCP `ECONNREFUSED` and UDP `EPERM` on the first send. The native
path does not use `reject with tcp reset` or a silent drop for output traffic.

The create path appends only the six rules in this order and the one dynamic
set at `nft.rs:3533-3566`. The delete path removes the six rules, the one
dynamic set, both chains, and the table in one `send_transaction` at
`nft.rs:3569-3588`. All batch operations use one netlink send and one ACK
collection loop at `nft.rs:3415-3531`; replacement of listener targets is
also one transaction at `nft.rs:3591-3641`.

Read-back requires the exact table/chains, one set, six owned rules, exact
userdata order, normalized expressions, and the generation bracket at
`nft.rs:3202-3366`. The old set names do not appear in the production nft
encoder or the active worker/sim production adapters. The R18 guard adapter,
guard field, and guard read-back are absent from the current production path.

The existing Tier-3 D15 cells in
`crates/overdrive-worker/tests/integration/mtls_intercept_install.rs:381-399`
also read the real table and assert one empty `inbound_destinations` set with
no `managed_guest_ips` or `outbound_sources` set.

### Allocation ownership and release

`MtlsIntercept` now installs only per-Service-port inbound destination
elements. The trait and state surfaces at
`crates/overdrive-worker/src/mtls_intercept_port.rs:206-371` and
`:374-391` expose one dynamic set. No public method was added; the removed
outbound install method is the accepted firewall-cut deletion.

The host adapter records process-local reference counts and acquires one
destination token per declared port at `mtls_intercept_port.rs:667-750`.
`SharedElementGuard::drop` removes only final-holder destinations and restores
the process-local count when the atomic delete/read-back operation fails
(`mtls_intercept_port.rs:111-162`).

The production worker owns the ordering. It acquires all declared destination
elements before publishing the capability at
`mtls_intercept_worker.rs:2707-2789`. Stop drains enforcement handles first,
then awaits `remove_allocation_elements_blocking`, and only after success drops
the transferred guards and completes the retirement at
`mtls_intercept_worker.rs:3335-3390`. A failed removal stores the drain for a
retry and returns `MtlsInterceptStopError::ElementRemoval`; it does not release
the address or capability. The changed acceptance cells preserve the exact
unrelated-member and same-address successor assertions.

Boot member convergence is owned by the worker. `start_shared_owner_inner`
calls `clear_boot_members` before observing the program, binding listeners, or
starting listener tasks (`mtls_intercept_worker.rs:2375-2455`), and the clear
returns the existing typed `BootMemberClear`/`MembersRemain` forms at
`:2458-2471`. Runtime repair recomputes registry members and converges them
before the final audit (`:2582-2641`). No managed TAP or bridge membership is
used by the current implementation.

### Boot ordering and typed error mapping

The composition root now performs VM reclamation, stale sweep, and then
`mtls_worker.start_shared_owner()` before `shared_guest_network.converge_shared()`
at `crates/overdrive-control-plane/src/lib.rs:4044-4127`. A table-write or
shared-owner failure returns the existing typed chain:

| Designed failure | Implementation | Caller/observable projection |
|---|---|---|
| Shared table write/replace fails at boot | `MtlsSharedOwnerError::Intercept { source }`, with the approved `InterceptError`/`NetlinkError` source | `MtlsBootError::SharedOwner` → `ControlPlaneError::MtlsBoot`; `health.startup.refused`, reason `mtls.shared_owner`; no forwarding owner is published |
| Stale inbound member clear fails or read-back remains non-empty | `MtlsSharedOwnerError::BootMemberClear { source }` with `InterceptError::NftElementUpdateFailed` or `MembersRemain` | Same typed `MtlsBootError::SharedOwner` chain; listener/table publication does not continue |
| Inbound element delete fails | `InterceptError::NftElementUpdateFailed { set: InboundDestinations, operation: Delete, key: Destination, source: NetlinkError }` | `MtlsInterceptStopError::ElementRemoval { alloc_id, source }`; retirement and address stay held until retry |
| Member audit differs from capability registry | `MtlsSharedOwnerError::MemberMismatch` / `MemberRepair` | Supervisor attributes the failure to the existing `IpSets` component and retries under the existing recovery hold |
| Policy route is absent during audit | Existing `InterceptError::PolicyRouteAbsent` | Existing `IpRules`/worker audit projection; no new error variant |

The direct caller mapping is source-preserving. No semantic failure is
flattened into a generic string, and no new public typed error variant was
introduced by this commit. The active ADR-0184 cells assert the typed
`MtlsBoot(SharedOwner)` result, the exact health reason, absent/complete table
state, and successful restart convergence.

### Scope boundaries

The production diff contains no gateway route writer, leg-B mark/source-bind
implementation, or D26 host-internal deny rule. The guest-only dummy default
route used for the two host-local firewall cells is existing test support
reviewed and approved through five DISTILL iterations; it is created and
removed inside the disposable worker guest, with a read-back proving the
prefix is unroutable before the remote cells. The `vsock_legb_dial` fixture
scripts exactly the registered `(VM_ADDR, PORT)` tuple with
`AcceptOpen`; it does not implement firewall behavior or add a production
fallback. The approved DISTILL review explicitly keeps these support changes
inside the existing `cgroup-default` group and existing 63-invocation/73-body
manifest.

## Native and execution evidence

The canonical final receipt is
`.context/netns-density-295/13-04-metal-guest-kit-cgroup-default-final-b55aa9bfea9009e3e09b3523.log`.
It ends with:

```text
OVERDRIVE_METAL_RUN_COMPLETE run_id=b55aa9bfea9009e3e09b3523 exit_code=0 cause=command_returned
```

The receipt records `guest-kit-prepare` for Linux `7.0.0-38-generic`, boots
three disposable Cloud Hypervisor guests, and executes the existing exact
`cgroup-default` selections. Independent parsing of the retained log finds:

| Result line | Count |
|---|---:|
| `test result: ok. 1 passed; 0 failed; 0 ignored` | 59 |
| `test result: ok. 2 passed; 0 failed; 0 ignored` | 2 |
| `test result: ok. 5 passed; 0 failed; 0 ignored` | 2 |
| Total passed bodies | **73** |
| Failed/ignored bodies | **0 / 0** |

The source manifest independently pins 55 baseline bodies, 18 STEP 13-04
bodies, and 73 combined bodies at
`crates/overdrive-testing/bin/fwd_guest_suite.rs:466-468`. Its exact routing
is at `:923-978`: control-plane acceptance/integration guests run the 55
baseline plus the six STEP 13-04 control-plane bodies; the worker guest runs
the seven baseline worker bodies, eight route-independent STEP 13-04 bodies,
two host-local firewall bodies under the guest-only default route, and two
remote firewall bodies after route cleanup. Each invocation requires the
exact libtest summary before its counter advances.

The raw receipt proves the native boundary and the required contrasting
outcomes:

- GuestKit prepares the six staged binaries and the guest forwarder module
  inside the existing lane. No physical-host nft, netns, route, cgroup, or
  module mutation is used by the worker suite.
- The two host-local cells observe TCP `ECONNREFUSED` and UDP `EPERM`.
- The runner reads back the test-owned route before the local cells and reads
  `STEP13-04 route cleanup readback: unroutable` before the remote cells.
- The unassigned remote destination times out, and the registered leg-C cell
  completes the two-certificate mutual-TLS exchange and records the exact
  local VM tuple, remote peer, and `fwmark: Some(2)`.
- Earlier run `603782e98f2f5fd2ee6b8821` failed at the missing disposable
  guest `/var/run/netns` compatibility path; it is diagnostic evidence only.
  The accepted DISTILL iteration-5 correction supplies the standard guest-only
  `/var/run -> /run` alias. The earlier route failure and failed guest launch
  receipts are not relabelled as final proof.

This native receipt is strong evidence for the selected Tier-3 cells. It does
not itself prove the normal package-union verification command required by the
roadmap; that evidence gap is the blocking finding below.

## Test integrity and acceptance completeness

The active STEP 13-04 native package contains 18 exact existing bodies. The
final receipt proves all 18 selected STEP 13-04 bodies and the unchanged 55
baseline bodies. The source search finds no pending STEP 12 marker in the
named STEP 13-04 bodies. The remaining `pending DELIVER step 13` markers are
the explicitly deferred STEP 13-05 gateway-route/leg-B cells and remain
untouched.

The test-support changes are bounded migrations required by the accepted
surface deletion:

- destination-only member models replace the removed two address sets;
- expected guard-drop counts change from `2 + P` to `P`;
- the registration/release assertions, retry behavior, unrelated-member
  preservation, and exact error projections remain active;
- obsolete outbound-install-only bodies were removed;
- `vsock_legb_dial` adds only the exact registered-tuple GuestStream script,
  as approved by the DISTILL review; its real mTLS, socket-table, timeout,
  errno, route, and cleanup oracles remain intact.

No assertion was weakened, no active test was skipped, no test scenario or
lane was added, and no production binary is spawned by the Rust tests. The
reviewed source retains the required Contract Shape declarations. The final
run is an external black-box guest-kit runner and does not invoke `cargo test`,
`nextest`, or an `overdrive-*` test binary inside the expectation harness.

The known quiescence counterexample seed `0x651125aa` from STEP 13-01, and the
additional preserved baseline seed recorded by the prior review, are not used
as STEP 13-04 evidence and were not silently erased. Mutation testing was not
run, as required by the repository's single final DELIVER-wave gate.

Independent checks performed for this review:

| Check | Result |
|---|---|
| `git diff --check b8bef6fbb..ec3bdc484` | PASS |
| `cargo fmt --all -- --check` | PASS |
| Roadmap JSON parse | PASS |
| DES phase order/status read-back | PASS for RED/PASS, GREEN attempts, COMMIT/PASS |
| Native final receipt | PASS, exit 0, 73/73 bodies |
| Normal package-union receipt | **MISSING** |
| Mutation testing | Not run, correctly deferred |

## Review dimensions

| Dimension | Result | Evidence |
|---|---|---|
| Exact API shape | PASS for the reviewed changed surface | No public additions; worker and netlink changes remove the accepted outbound/set/guard mechanisms and retain the four-port worker constructor. |
| Typed semantic errors | PASS | Boot write, boot clear, element delete, member mismatch, and policy-route mappings preserve approved typed carriers and caller projections. |
| Constant rule semantics | PASS | Six-rule source identity, gateway exclusion, order, one set, and native TCP/UDP/drop/leg-C outcomes are all evidenced. |
| Atomicity | PASS by source and native read-back | One netlink batch/ACK loop for create/delete/replace and one guest-only native table boundary. |
| Awaited lifecycle | PASS | Stop awaits enforcement teardown and blocking element removal before releasing guards/retirement; retry retains ownership on failure. |
| Boot convergence | PASS | Clear-before-observe/listener/task publication and typed restart retry are source-traced and exercised by sim/native cells. |
| Test integrity | PASS | Production changes are present; test adaptations remove obsolete mechanisms and retain behavior oracles. |
| Native/Tier-3 evidence | PASS for the 73 selected bodies | Final durable Metal receipt proves 59+2+2 result lines, 73 passed, 0 failed/ignored, and required native routing. |
| Verification completeness | **BLOCKED** | The roadmap's normal package-union gate has no retained command/exit receipt in this checkout. |
| Scope | PASS | No STEP 13-05 gateway/leg-B production mechanism or STEP 13-06 host-internal deny behavior was pulled forward. |
| RPP / readability | PASS at the applicable review level | No new L1/L2 implementation smell was found that affects this bounded cut; native runner support is already separately DISTILL-reviewed. |

## Findings

### `issue (blocking):` The required normal package-union verification receipt is absent

The roadmap's required verification list includes the package union
`cargo nextest run -p overdrive-netlink -p overdrive-worker -p overdrive-control-plane -p overdrive-sim --no-fail-fast` and workspace check/Clippy at
`docs/feature/netns-density-295/deliver/roadmap.json:1394-1398`. The retained
DES dispatch repeats those requirements at `.context/netns-density-295/dispatch-13-04.md:63-67`.

The final Metal receipt proves the exact native `cgroup-default` manifest and
all 73 selected bodies. It does not run the normal package union. The only
persisted STEP 13-04 execution artifacts in this checkout are the focused
native receipts and the DISTILL support receipts; no durable package-union
command log, exact command result, or exit path is retained. The crafter's
reported focused Sim/shared-owner/boot-write checks and workspace check/Clippy
claims are not a substitute for the broader required union gate.

This is a verification-completeness finding, not a claim that the production
path has a reachable runtime defect. The missing evidence cannot be cured by
the 73-body native count: the package union covers ordinary tests and compile
paths outside the selected guest-kit manifest, including retained callers and
test-support surfaces affected by the removed element sets.

Required disposition: the original STEP 13-04 crafter/root must provide the
actual full normal package-union command, durable run ID, raw receipt, and
exact exit path, or execute that gate through the already-authorized durable
Metal route with the safe existing routing/skip accounting. The verification
must preserve the existing 55+18 native selection and must not run known
host-mutating tests directly on the physical host, create a new lane, or alter
the production implementation to fit a focused subset. If the full union
fails, remediate only the reachable failure through the original crafter and
return this same step to this reviewer; if it passes, append the receipt and
return for iteration 2. Do not start STEP 13-05.

**Disposition: OPEN — blocks approval.**

### `praise:` The final native receipt is honest and complete for its declared boundary

The final durable run ends with exit 0 and independently parses to 73 passed
bodies with zero failures or ignored tests. The runner's route setup is scoped
to the two host-local cells, is removed and checked unroutable before the
remote cells, and the leg-C positive reports the exact VM tuple and `fwmark`
observation. The failed `603782...` run is retained as a diagnostic failure,
not misreported as final proof. This is the correct evidence boundary for the
native package.

### `praise:` The lifecycle cut preserves ownership and retry semantics

The changed worker path keeps the allocation's capability and address held
through a failed element removal, awaits the actual removal/read-back before
releasing guards, and retains the drain for a retry. The native and Sim cells
keep the unrelated allocation's handle/member state unchanged, while the boot
clear path remains before listener/task publication.

## Iteration history and verdict

### Iteration 1 — 2026-10-28

The fresh post-commit review covered the complete implementation diff, the
accepted feature-delta and ADR contracts, the DISTILL handoff and all five
DISTILL review iterations, typed error/caller mappings, source-local deletion
and lifecycle paths, test changes, DES history, and the final durable Metal
receipt. The firewall implementation, ownership lifecycle, scope boundaries,
test integrity, and native 73-body evidence pass. The normal package-union
verification receipt is not present, so the broad verification gate remains
unproven.

**Verdict: NEEDS_REVISION.** Obtain the missing full package-union receipt from
the original STEP 13-04 crafter/root, then return this same step for iteration
2. No production change is authorized by this finding, and no later roadmap
step may start.

# NEEDS_REVISION

## Iteration 2 — full-union remediation re-review

### Re-review metadata

| Field | Value |
|---|---|
| Review iteration | 2 (same fresh STEP 13-04 reviewer) |
| Re-review date | 2026-10-28 |
| Original blocking finding | Missing retained normal four-package union receipt |
| Remediation commits | `254cb4a98aa77da156199c59643c05c21899703d`, `70c096415d7805d5e8f4e4fc3adee665e6b185eb` |
| Current implementation | `70c096415d7805d5e8f4e4fc3adee665e6b185eb` |
| Current source digest | `700f082bf680ecd9bd233a9572e5871c26af6c7519d1b0d653a7f55a2c41b124` |
| DISTILL source handoff | `9850c6b791fc8cb1d57ee6d8dbd7e3fb43c7c18e6b19a27164c5dd5d1da5abd9` |
| DISTILL review, iteration 6 | `9d33422f1adf08d4ed451dd976d7493df78116bed5b7b6509718b8b1bed6d974` |
| Current DES log | `c95c81e857bcc4d42b3139e8aa4e79fa0dbb1f7d08d404d570dccf5e05e462c5` |
| Final verdict | **APPROVED** |

This iteration rechecks the exact remediation that closed Iteration 1's sole
blocking finding. The implementation author remains Marcus with exactly the
Codex co-author and `Step-Id: 13-04` trailers. The two remediation commits do
not add a public API, production mechanism, DES event, lane, scenario, model
action, exclusion, or contract relaxation. The only moved test support is a
private `#[cfg(all(test, feature = "integration-tests"))] mod
replacement_acceptance` beside the existing action-shim core
(`crates/overdrive-control-plane/src/action_shim/mod.rs:3876-3880`). The
pre-existing user-owned `AGENTS.md` remains the only unrelated dirty file;
this review artifact is the only reviewer-owned file.

### Disposition of Iteration 1

The Iteration 1 finding required the actual normal package-union command and
its durable exit path. The crafter supplied and the reviewer independently
inspected the retained union receipt
`.context/netns-density-295/13-04-metal-package-union-iteration6-final.log`
(SHA-256
`b3be1dd3fd8042cf1bc81064627013fc3a19d803624078e07db798d399613461`). It
records the exact required command:

```text
cargo nextest run -p overdrive-netlink -p overdrive-worker -p overdrive-control-plane -p overdrive-sim --no-fail-fast
```

It records the exact four-package normal union, Nextest run ID
`97539847-3738-4aca-9cd2-73ad9e7d9dc9`, and the durable Metal run ID
`5337675ce16d6c914cce9cf1`. The receipt ends with:

```text
Summary [2208.946s] 1715 tests run: 1715 passed (3 slow), 148 skipped
OVERDRIVE_METAL_RUN_COMPLETE run_id=5337675ce16d6c914cce9cf1 exit_code=0 cause=command_returned
```

The completion marker is interleaved with one test line in the raw log, but
the marker, the Nextest summary, and the command return are all present in
the same durable receipt. Independent parsing of the result lines finds
1,715 passed, zero failed, and zero ignored results. The 148 skips are the
existing package selection's accounted skips; the remediation diff adds no
new `#[ignore]`, skip, filter, or exclusion. The package union therefore
closes the original verification-completeness finding.

The separately required current guest-kit cgroup-default receipt was also
retained and independently inspected:

| Evidence | Result |
|---|---|
| Raw receipt | `.context/netns-density-295/13-04-metal-guest-kit-cgroup-default-final-iteration6.log` |
| Receipt SHA-256 | `7193fc46562497ab3f4d01bd143cd53194641c184f71142453be7cce48d51ab1` |
| Durable Metal run | `ce0d4d7c9d72212c08c5f0d7` |
| Disposable guests | 3 |
| Runner accounting | 55 existing baseline bodies + 18 STEP 13-04 bodies; 63 invocations |
| Body results | 73 passed, zero failed, zero ignored |
| Exit | 0, `cause=command_returned` |

The raw receipt retains the expected boundary evidence: the guest-only route
precondition reads `100.95.255.201 dev ovd-gk-route src 192.0.2.1`, the route
cleanup reads `unroutable`, the remote-drop case observes a connection
timeout, and the leg-C case observes two certificates and the accepted
socket tuple with `fwmark: Some(2)` (`...iteration6.log:1930-1971`). The
runner performs its cleanup before the remote-drop and leg-C cells. The
earlier failed `603782...` netns-path run remains diagnostic evidence and is
not substituted for this final receipt.

### Remediation source audit

DISTILL iteration 6 is approved at
`review-distill-13-04-legc-fixture-alignment.md:1396-1728`. It records the
actual 23-failure partition from the prior union, the source corrections, the
old-to-new test identity map, the exact journal disposition, and the
no-exclusion audit. I rechecked those claims against the current source,
both remediation commits, and the new full union rather than treating the
DISTILL approval or green count as a substitute for review.

| Prior union failure partition | Current disposition | Verification |
|---|---|---|
| 19 direct guest-owner callers: E09 (2), E10 (1), node admission (1), reclaim (2), service-backend projection (11), VM-finalize (2) | Each existing fixture starts its existing shared mTLS owner, converges the exact guest-network owner injected into `AppState`, captures that owner's bound ports, and opens the paired EXEC gate only after convergence. Every action tick keeps that same owner and wiring instance. | DISTILL review `:1441-1510`; source paths and the package union all pass. |
| 3 D-UP-15 replacement scenarios using the legacy `guest_network: None` branch | Each body moved once into private `action_shim::replacement_acceptance`, where the existing private guest-aware dispatch core receives the existing owner, pool, bound ports, gate, and custom `MtlsInterceptLifecycle`. | `replacement_acceptance.rs:413-479,501-950`; union log tests 22–24 pass. |
| Seed `25717` probe-event timeout | The existing supervised `ProbeRunner` producer, existing `GuestStreamDial`, typed refusal/HTTP stimuli, readiness threshold 3, and one late pending dial are used. No seeded observation row, restarted supervisor, lowered threshold, or changed invariant was introduced. | DISTILL review `:1512-1543`; current union passes the seed case. |

The direct fixture correction preserves the accepted order

```text
existing mTLS owner start
  -> same guest-network owner startup/convergence
  -> bound GuestFlowPorts capture
  -> AppState composition with that owner
  -> paired EXEC gate open
  -> production action/shim ticks using the same owner
```

This closes the concrete prior failure at the existing owner composition
boundary. It does not manufacture a second owner, manually write flow-port
state, bypass the gate, or alter the production `AppState` contract. The
existing guest-only confinement, threshold, lifecycle, typed error
precedence, and no-op stop assertions remain in the bodies. The remediation
also preserves the standard `/var/run` to `/run` alias preparation and does
not introduce a physical host path.

The moved replacement module is private and source-local. Its `dispatch_one`
helper (`replacement_acceptance.rs:413-479`) calls the already-existing
`dispatch_with_network_provisioner_and_guest` path and supplies
`Some((&guest_network, &guest_pool, Some(flow_ports)))`; it does not add a
public method, trait, enum variant, cfg-test API, test seam, or production
fallback. The three test bodies are at lines 503, 612, and 907, each retains
its required `CONTRACT_SHAPE: bounded-change` declaration, and the current
source still contains the guest-netns prohibition oracle at lines 244-251.
The old three IDs are absent from the external location, the new three IDs
are unique, and the old-to-new count is exactly 3 to 3. The retained
assertion counts are 17, 25, and 12 for the three scenarios. The full union
receipt runs all three new identities and reports each PASS.

Commit `254cb4a98` is independently bounded to one integration file, with
two insertions and five deletions. It removes only the four stale
`Step::InterceptInstalled` expectations from destination-only install
journals. The recorder, `steps_for` projection, and unexpected-entry
detection remain. The full package union exercises these journal paths after
the correction and passes them; this is an oracle alignment with the accepted
destination-only contract, not a weakening of an error or lifecycle outcome.

The raw diff and union receipt show no new skip or exclusion. The existing
55+18 native accounting remains unchanged, and no new native lane, guest
scenario, model action, API, or mechanism was introduced. Mutation testing is
correctly absent from this step review because it is a single final
DELIVER-wave gate, not a per-step gate.

The previously approved DISTILL support evidence remains within its exact
boundary after the fixture move. The registered leg-C `GuestStreamDial`
accepts only the registered `(VM_ADDR, PORT)` tuple and reports the default
path as `NotServing`; the same recording owner is converged before
`AppState` port capture and before EXEC, and predecessor/fault stimuli remain
after boot. Guest confinement still traverses the root with gid `4200` and
mode `0710`, uses UID `4200` and the existing device-group boundary, and
retains the existing Landlock and seccomp restrictions. The current native
`cgroup-default` group remains the three-disposable-guest selection with the
exact 55 baseline and 18 STEP 13-04 bodies (73 body results over 63
invocations); no new lane or body group was authored.

### Contract and behavior recheck

The Iteration 1 contract checks remain valid after remediation. The source
still implements the accepted STEP 13-04 firewall cut: the exact six
constant rules and order, the destination-only `inbound_destinations` set,
removal of rules 2–3 and `outbound_sources`/`managed_guest_ips` lifecycles,
R18 guard/read-back retirement, gateway exclusion, output reject and
prerouting drop behavior, and atomic delete/add in one batch. No STEP 13-05
gateway-route/leg-B mark behavior or STEP 13-06 host-internal deny behavior
was pulled forward.

The ownership and failure semantics remain the accepted ones: release awaits
convergent inbound-member and forwarded-registration teardown; failure keeps
the member and supports retry; unrelated member/address/lease state remains
isolated; boot convergence clears state before listener/task publication and
retries under the existing supervised owner; and the typed boot shared-table
write failure occurs before forwarding opens. The existing typed error
mappings remain traceable from design to implementation to callers. No
generic string replacement or new error representation was added by either
remediation commit.

The native result is consistent with those boundaries. The final route and
cleanup read-backs, output reject, prerouting drop, TCP refusal, UDP EPERM,
remote-drop timeout, and leg-C bidirectional mTLS/mark observations are
present in the current receipt. No host nft, host netns, host cgroup, Lima,
reboot, or force-unload mutation was introduced by the re-review.

### Verification gates

| Gate | Result | Evidence / disposition |
|---|---|---|
| Full normal four-package union | **PASS** | Durable run `5337675ce16d6c914cce9cf1`; Nextest `97539847-3738-4aca-9cd2-73ad9e7d9dc9`; 1715/1715 passed, 148 accounted skipped, exit 0. |
| Current native guest-kit selection | **PASS** | Durable run `ce0d4d7c9d72212c08c5f0d7`; 3 guests, 63 invocations, 73 body results passed, exit 0. |
| Formatting | **PASS** | Crafter receipt `7a98d7b7941f2e7b881233a8`; no formatting diff. |
| Workspace check | **PASS** | Crafter receipt `c4f6bce8ed6299953ad28a10`. |
| Clippy | **PASS** | Crafter receipt `1e213f25ee8da7f084e8ab`. |
| DES history / commit scope | **PASS** | Existing RED PASS, recorded GREEN failures, GREEN PASS, COMMIT PASS; remediation commits have exact attribution and step trailers. |
| Native skip accounting | **PASS** | 55+18 body accounting preserved; no new exclusion, filter, or lane. |
| Mutation testing | Correctly deferred | Reserved for the final DELIVER-wave gate by repository rule. |
| Known quiescence counterexamples | Preserved limitation | Baseline `651125aa` and additional `7a576b92` remain recorded; this review does not claim every random seed passes. |

### Findings and dispositions

No new reachable production, test-integrity, design-contract, typed-error,
ownership, lifecycle, ordering, or native-boundary finding was identified.
The only Iteration 1 finding was verification completeness; it is closed by
the retained, independently parsed package-union command and exit receipt.
The fixture remediation is necessary and bounded to the actual 23 recorded
failures. It does not require a design change, new architecture, or source
behavior expansion.

| Finding | Disposition | Evidence |
|---|---|---|
| Missing normal package-union verification receipt | **CLOSED** | Raw union SHA, Nextest ID, durable run ID, exact 1715/1715 summary, 148 accounted skips, and exit 0 above. |
| Direct owner fixture composition failures | **CLOSED** | Existing owner start/convergence and same-owner AppState/EXEC ordering, with unchanged assertions and successful union. |
| Legacy replacement helper selected forbidden non-guest branch | **CLOSED** | Three bodies moved once to private guest-aware test module; exact IDs/assertions retained; all three pass in union. |
| Seed 25717 producer timeout | **CLOSED** | Existing supervised monotonic producer and late pending dial preserve the original threshold and stimuli; union passes. |
| Destination-only journal oracle mismatch | **CLOSED** | `254cb4a98` removes only obsolete install-step expectations; union passes and recorder remains active. |

The prior diagnostic `603782...` run is not used as final proof, and the
prior Iteration 1 artifact is not rewritten or erased. The approved DISTILL
review remains the source record for the fixture remediation, while this
artifact records the post-remediation full verification and implementation
review.

## Iteration 2 verdict

The original blocking verification finding is closed. The exact full package
union, current 73-body native selection, fixture remediation, test identity
and assertion preservation, DES history, scope, and accepted STEP 13-04
contract all pass review. No unresolved finding remains, and no additional
remediation or design decision is required.

**Verdict: APPROVED.** STEP 13-04 may advance under the DELIVER sequence.
This approval does not erase the known quiescence counterexample limitation
and does not claim all random seeds pass. No STEP 13-05 work was started by
this reviewer.

# APPROVED
