# DELIVER review — STEP 13-06 host-internal deny set (D26)

## Review metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `13-06` |
| Review ID | `STEP13-06-DELIVER-POSTCOMMIT-01` |
| Review date | 2026-10-29 |
| Reviewer | Fresh isolated post-commit DELIVER implementation reviewer |
| Iteration | 1 |
| Parent commit | `4dad2b568b3ee75578b9caedcd7e0b2e81b69c63` |
| Implementation commit | `b51d5f3dc1e35483abe6d6809fb29c3fd38b15e5` |
| Final verdict | **NEEDS_REVISION** |
| Open findings | 2 blocking evidence findings |

This review applies the current `AGENTS.md`, all applicable repository rules,
the software-crafter reviewer dimensions, the complete
`.context/netns-density-295/dispatch-13-06.md`, the accepted STEP 13-06
roadmap entry, feature-delta D26/D26-PINS and ADR-0167. The native-Markdown
review convention takes precedence over the installed review skill's legacy
YAML-only output and iteration cap.

The pre-existing modified `AGENTS.md` was preserved. No production source,
acceptance body, DES event, roadmap, design artifact, spike source, host
kernel state or commit was changed by this review. This review artifact is the
only reviewer-owned file.

## Authority and review boundary

The authoritative contract is `docs/feature/netns-density-295/deliver/roadmap.json`
STEP 13-06, `docs/feature/netns-density-295/feature-delta.md` D26 and
D26-PINS, and
`docs/product/architecture/adr-0167-guest-flows-to-host-internal-destinations-are-refused.md`.
The v21 spike at
`docs/feature/netns-density-295/spike/findings-v21.md` is the required
working-version and falsification reference.

The bounded contract requires:

1. the 13 existing `vsock_host_internal_enforce` cells to be activated by
   marker removal only and to run through the real `start_shared_owner` path;
2. owner-marked gateway UDP/TCP port 53 exceptions before the deny rules;
3. the dedicated `OWNER_EGRESS_MARK` bit matched under a mask;
4. loopback and link-local static rejects plus `fib daddr type != unicast`,
   with plain `reject` semantics;
5. D26 rules before every accept in the output chain and one nft transaction
   for replacement;
6. TCP `ECONNREFUSED`, UDP first-transmit `EPERM` and the existing owner
   projection to `DestinationUnreachable`;
7. preservation of ADR-0172 guest-prefix TCP reset and non-TCP plain reject,
   non-host-internal unicast, service-VIP behavior, leg-C diversion and the
   unmarked owner connect to leg-F; and
8. the required normal and native evidence boundaries, with all nft/netns/
   kernel effects in a disposable Cloud Hypervisor GuestKit guest rather than
   on physical Metal, and durable source identity, exact exit, skip and
   cleanup evidence.

## Commit and DES integrity

The implementation commit has the expected parent, keeps the Marcus author,
contains exactly one `Co-Authored-By: Codex <codex@openai.com>` trailer and
the `Step-Id: 13-06` trailer, and contains no Claude, Anthropic or generated
attribution. `git diff --check` is clean.

The persisted DES events are:

| Phase | Result | UTC timestamp |
|---|---|---|
| RED | FAIL | 2026-10-28 23:54:51 |
| RED | PASS | 2026-10-29 11:12:51 |
| GREEN | PASS | 2026-10-29 11:21:50 |
| COMMIT | PASS | 2026-10-29 11:27:18 |

The failed RED event is retained and the later RED pass is recorded. The
repository-wide integrity command reports unrelated pre-existing missing
events for steps 04-03, 04-04 and 14-01 through 14-05, plus an incomplete
04-02; it does not report a missing phase for 13-06. Those unrelated findings
are outside this review and were not altered.

## Contract and implementation assessment

### Exact public and cross-crate API shape

The commit adds no public method, type, enum variant, parameter, wire field,
configuration field or ownership surface. The existing public
`OWNER_EGRESS_MARK` constant is imported into the private nft encoder; the
new `e_bitwise_mask`, `e_fib_daddr_type`, D26 rule builders and output-rule
count are private. The worker's private normalized postcondition is updated
from four to nine output rules in the same existing identity carrier.

The implementation therefore matches the accepted interface boundary. No
API invention or typed-error taxonomy change was found.

### D26 rule construction

The source-level construction matches D26-PINS:

| Contract | Source evidence | Assessment |
|---|---|---|
| Masked dedicated mark | `crates/overdrive-netlink/src/nft.rs:636-640`; bitwise mask `OWNER_EGRESS_MARK` followed by equality | PASS by source |
| Gateway exceptions | `nft.rs:643-652`, output indices 0 and 1 | PASS by source |
| Static loopback/link-local rejects | `nft.rs:655-660`, output indices 2 and 3 | PASS by source |
| Non-unicast backstop | `nft.rs:663-668`, `fib` address-type result compared with `RTN_UNICAST` | PASS by source |
| Plain reject | `nft.rs:867-874`; no TCP-reset or drop expression in D26 builders | PASS by source |
| Rule order | `nft.rs:809-837`; D26 indices 0-4 precede leg-S, registered-destination and ADR-0172 rules | PASS by source |
| ADR-0172 preservation | `nft.rs:831-836`; TCP reset remains index 7 and non-TCP plain reject index 8 | PASS by source |
| One transaction | `nft.rs:3691-3738`; replacement emits all rule mutations through one `send_transaction` batch | PASS by source |
| Exact identity count | `nft.rs:2907-2908`, `2977-2985`, `3206-3238`; nine output rules are required and validated | PASS by source |

The expression encoder uses the pinned Linux nftables ABI values from
`vendors/linux/include/uapi/linux/netfilter/nf_tables.h`; the netlink package's
50 library tests also pass in the independent focused receipt below. The
required real-kernel confirmation is still open because the native D26 gate
has not been run through GuestKit.

### Owner lifecycle and unaffected paths

The existing owner path retains the designed semantic boundaries:

- `owner_runtime.rs:1048-1053` and `1206-1225` refuse static and local/non-
  unicast destinations as `FlowRefusal::HostInternal` before a direct peer is
  opened, except for the gateway port 53 rule and mesh/VIP paths already
  governed by their contracts.
- Non-mesh TCP creates a marked, abortively-lingering peer at
  `owner_runtime.rs:1120-1128`; the owner connect-to-leg-F mesh branch remains
  `owner_egress_mark: false` with a loopback bind at `1061-1068`.
- UDP creates the owner mark only when the destination is neither a VIP nor
  gateway:53 at `owner_runtime.rs:1233-1238`; the existing peer-address
  guest-prefix check remains at `1252-1256`.
- The existing pair-end projection maps a datagram-side socket error to
  `FlowCloseCause::DestinationUnreachable` at
  `owner_runtime.rs:1397-1407`. Existing owner acceptance coverage at
  `owner_acceptance/datagrams.rs:197-228` exercises the approved EPERM to
  `DestinationUnreachable` semantic mapping through the simulation port.
- Existing `MTLS_LEG_S_DIAL_MARK` and `MTLS_TPROXY_MARK` remain separate from
  the owner bit. The D26 test body for the other marks is retained and was
  activated without assertion changes.

No production caller, cancellation path, drain path or retry path was changed
by this commit. The code-level owner mappings are consistent with the approved
contract; the missing native receipt prevents acceptance of the observable
kernel half.

### Typed semantic error mapping

The required mapping is already represented by approved existing types and is
preserved by this commit:

| Designed failure | Approved and implemented representation | Evidence / status |
|---|---|---|
| Owner's static/local-delivery check identifies a host-internal destination | `FlowRefusal::HostInternal` | `owner_runtime.rs:1048-1053,1206-1225`; PASS by source |
| Plain-reject TCP connect is refused by the kernel | `FlowRefusal::DestinationUnreachable` after `connect_peer` failure | `owner_runtime.rs:1144-1152`; PASS by source |
| Plain-reject UDP first transmit returns `EPERM` on the forwarded owner socket | Datagram pair closes with `FlowCloseCause::DestinationUnreachable` | `owner_runtime.rs:1397-1407`; existing simulation acceptance at `datagrams.rs:197-228`; native proof OPEN |
| Gateway:53 exception | No refusal; marked TCP/UDP output rule accepts before the deny rules | `nft.rs:643-652`; native proof OPEN |
| Mesh-resolved TCP / leg-F connect | Unmarked `PeerSocketSpec`, existing mesh path | `owner_runtime.rs:1061-1068`; PASS by source |
| Service-VIP datagram | Unmarked peer and existing connect4/kernel-peer policy | `owner_runtime.rs:1213-1238`; PASS by source |

No generic string or unapproved public error variant substitutes for a typed
semantic distinction.

## Test integrity and Contract Shape

The test diff is exactly 13 deletions of
`#[ignore = "pending DELIVER step 14"]`. No test body, assertion, fixture,
helper, name or expected value changed. The 13 live Tokio tests each retain
one exact `/// CONTRACT_SHAPE:` declaration (13 declarations, 0 ignores).
The tests are real-kernel Tier-3 integration tests, and the dispatch explicitly
exempts their exact errno and nft chain-order cells from the general property-
based paradigm. They enter through `boot_shared_table`, which calls the
production `MtlsInterceptWorker::start_shared_owner` and real
`HostMtlsIntercept` path; the marked sockets are the documented kernel-half
driving boundary.

No zero-assertion, tautological, mock-dominated, circular, assertion-free or
fixture-end-state test theater was found in the changed diff. Mutation testing
was correctly not run under the explicit repository/user override.

## Verification receipts

The following independent, read-only or ordinary userspace receipts were run
through the mandated Metal command path during this review:

| Check | Receipt | Result |
|---|---|---|
| Package/all-target compilation | Metal run `c499eac2b26860dc1d2c22ba`, exit 0 | `cargo check -p overdrive-netlink -p overdrive-worker --all-targets --features integration-tests` PASS |
| Package/all-target Clippy | Metal run `d687cd90ac8196595e130d01`, exit 0 | `cargo clippy -p overdrive-netlink -p overdrive-worker --all-targets --features integration-tests -- -D warnings` PASS |
| Formatting | Metal run `3475b018d1b098964104589e`, exit 0 | `cargo fmt --all -- --check` PASS |
| Focused netlink library tests | Metal run `235d466538eeb592d3db4ada`, exit 0; nextest `53e6e57b-82c1-4805-8763-43eb306a9351` | 50 passed, 0 skipped |

These receipts do not prove the required D26 Tier-3 behavior. The repository
has no `.context/netns-density-295/13-06/` directory or step-specific raw
receipt in the handed-off workspace. The commit contains only execution-log
phase events, not command output, source identity, native run identity, exact
exit, cleanup evidence or skip accounting.

The required normal package gate from the dispatch is also not represented by
a current step receipt:

```text
cargo xtask metal run -- cargo nextest run -p overdrive-netlink -p overdrive-worker --no-fail-fast
```

I did not run that package-wide command on physical Metal because this package
includes the root/kernel/netns D26 integration binary and the dispatch requires
all such effects to stay inside disposable Cloud Hypervisor guests. A direct
physical-Metal invocation would violate the accepted safety boundary. The
focused netlink library run above is not a substitute for the normal package
gate.

## Findings

### F-01 — Blocking: the required native D26 GuestKit boundary and manifest are absent

The dispatch requires the 13 D26 cells to run through the existing disposable
Cloud Hypervisor GuestKit, with no physical-Metal nft/netns/kernel mutation,
and requires an actual durable result with source identity, exact exit and
cleanup. The existing GuestKit runner does not select these cells. Its worker
manifest is `STEP13_05_WORKER` at
`crates/overdrive-testing/bin/fwd_guest_suite.rs:888-908`, containing only
the three leg-B cells. `run_cgroup_default` passes only the baseline,
STEP 13-04 and STEP 13-05 manifests at `:1892-1910`; the count constants at
`:996-1003` likewise stop at the five STEP 13-05 bodies. No
`vsock_host_internal_enforce` identity or STEP 13-06 count appears in the
runner.

The implementation commit changes only `nft.rs`, the worker postcondition
carrier, the D26 test marker lines and `execution-log.json`; it contains no
bounded existing-runner/DISTILL selection or staging update. Therefore a
claimed `cargo nextest ... test(vsock_host_internal_enforce)` on physical
Metal cannot satisfy this dispatch: it would exercise the host kernel outside
GuestKit. The absent `.context/netns-density-295/13-06/` receipt directory
confirms that no accepted native13 run, source identity, exact exit or cleanup
receipt was handed off.

This is an evidence-boundary failure, not a request for a new lane, API,
scenario or mechanism. DISTILL must extend the existing cgroup worker
manifest/count and its already-existing GuestKit setup for these 13 authored
cells, then the step must be rerun with full raw and durable receipts. Runtime
skips must remain fail-closed and every selected body must execute normally.

**Disposition: OPEN — blocks approval.**

### F-02 — Blocking: the required normal gate and complete step receipts are missing

The crafter's DES log records `GREEN=PASS` and `COMMIT=PASS`, but no actual
command receipt is present for the required normal package gate, the native
D26 gate, or the asserted `332 passed / 332 accounted skips` result. The
step-specific receipt directory is absent, so there is no durable command
identity, raw output hash, exact terminal exit, selected-body identity ledger,
runtime-skip scan or cleanup proof to review. The repository-wide
`des-verify-integrity` output cannot supply any of these command receipts.

The independent compilation, Clippy, formatting and 50-test netlink-library
receipts above are valid evidence for those narrower claims only. They cannot
establish that all required worker/netlink normal tests passed, that no
runtime skip made the normal count vacuous, or that the 13 D26 real-kernel
cells reached the production composition root inside a disposable guest.

The crafter/orchestrator must retain the complete raw durable receipts under
`.context/netns-density-295/13-06/`, including the normal package command, the
GuestKit native command and its source/runner identity, exact terminal exit,
per-body counts, no-SKIP evidence and cleanup read-back. No mutation run is
needed or authorized for this step.

**Disposition: OPEN — blocks approval.**

## Review dimensions

| Dimension | Result | Evidence |
|---|---|---|
| Exact API shape | PASS | No new public/cross-crate interface; private D26 builders only. |
| Typed semantic errors | PASS by source; native projection OPEN | Existing `HostInternal`, `DestinationUnreachable` and datagram close-cause mappings are preserved exactly. |
| D26 mask/order/reject/fib semantics | PASS by source | `nft.rs:636-668,809-837,867-874`; no native receipt yet. |
| Atomic replacement | PASS by source | `nft.rs:3691-3738` sends all rule replacements in one transaction. |
| ADR-0172 preservation | PASS | Guest-prefix TCP reset and non-TCP plain reject remain distinct at output indices 7 and 8. |
| Leg-F / non-host-internal / VIP preservation | PASS by source | Owner mark and mesh/VIP paths remain unchanged; direct D26 rules only match the owner bit. |
| Test integrity | PASS | Marker-only activation; no assertion or body changes. |
| Contract Shape | PASS | 13 live tests, 13 declarations, 0 pending ignores. |
| External validity | BLOCKED by F-01 | Production worker path is used by the test, but no valid disposable-guest native receipt exists. |
| Native boundary and cleanup | BLOCKED by F-01/F-02 | Existing runner omits the cells and no step receipts are present. |
| Verification completeness | BLOCKED by F-02 | Narrow compile/lint/fmt/unit receipts pass; required normal/native gates are unproven. |
| RPP / readability | PASS for bounded source change | No relevant L1/L2 smell or speculative production mechanism found. |
| Scope | PASS | Commit is bounded to the D26 encoder, identity projection, marker activation and DES event persistence. |

## Iteration history and verdict

### Iteration 1 — 2026-10-29

The fresh post-commit review read the complete dispatch, accepted D26/ADR-0167
contract, v21 spike, implementation diff, production owner path, typed error
mapping, test source and DES events. Source-level D26 semantics, exact API
shape, owner mark/leg-F/VIP preservation, test integrity and commit
attribution pass. The required existing GuestKit manifest does not include
the 13 D26 cells, and the required normal/native durable receipts are absent.
Those are blocking evidence-boundary failures under the explicit dispatch and
cannot be closed by focused compilation or the 50 netlink library tests.

No remediation was performed. The two findings remain open and the step must
return to the bounded DISTILL/DELIVER evidence path before re-review.

**Verdict: NEEDS_REVISION.**

## Iteration 2 — post-remediation re-review

### Re-review metadata

| Field | Value |
|---|---|
| Review iteration | 2, same STEP 13-06 reviewer |
| Re-review date | 2026-10-29 |
| Remediation commit | `f683f09099d8c008be567c78bc6159a1402b6712` |
| Implementation base | `b51d5f3dc1e35483abe6d6809fb29c3fd38b15e5` |
| Remediation scope | Existing GuestKit worker manifest/module/route support, native receipts, normal-gate receipts and DES persistence |
| Native receipt | Metal run `1bb49a61e3a35201fdc93597`, exact exit 0, `command_returned` |
| Normal inventory | Metal run `e79f87b6289f966ce5c35221`, exit 0; 341 identities: 266 selected, 73 exact exclusions, 2 ignored |
| Normal execution | Metal run `8598d106dfb716885f917f8f`, exit 0; 266 passed, 75 skipped, 0 failed |
| Final verdict | **APPROVED** |

The remediation commit retains the Marcus author, exactly one Codex
co-author trailer and `Step-Id: 13-06`. It changes only the existing
`fwd_guest_suite` support executable and the native/DES review artifacts; the
production D26 encoder, worker postcondition carrier and the activated D26
test body remain the implementation from `b51d5f3dc`. The user-owned dirty
`AGENTS.md` remains untouched. No reviewer production/test/DES mutation or
commit was made.

### F-01 disposition — CLOSED: existing GuestKit now executes all 13 D26 cells

The remediation extends the existing `cgroup-worker-integration` tuple in
`crates/overdrive-testing/bin/fwd_guest_suite.rs`:

- `STEP13_06_WORKER` selects ten existing D26 identities and
  `STEP13_06_ROUTED_WORKER` selects the remaining three. An independent
  source comparison finds exactly 13 manifest identities, exactly 13 live
  `vsock_host_internal_enforce` async tests, no missing/extra identity, 13
  `CONTRACT_SHAPE` declarations and zero live ignores.
- `STEP13_06_NFT_MODULES` stages only `nft_fib` and `nft_fib_ipv4` for the
  worker tuple that has a non-empty D26 manifest. The module closure probe
  `02f81d15e5eedec1514da1c3` passed on Linux `7.0.0-38-generic`; the
  unrelated `mtls-relay` path remains unchanged.
- The three routed UDP cells run under the bounded `ovd-gk-d26` dummy/default
  route setup. The setup proves `192.0.2.1` is initially unroutable and not
  local, reads back the exact interface/source route, and the cleanup removes
  the exact route/interface and proves the destination is unroutable and not
  local again. The route is scoped to those three cells only.
- Each selected body runs through `exec_required_cgroup_cells`, which requires
  process exit zero, the exact `1 passed; 0 failed` summary and no recognized
  runtime-skip diagnostic. The retained 55 + 18 + 5 + 20 boundary is checked
  as 98 before the D26 13-body boundary and complete 111-body boundary are
  accepted.

The durable native command was independently recollected with
`cargo xtask metal collect 1bb49a61e3a35201fdc93597`. Its output visibly boots
Cloud Hypervisor guests (`DMI: Cloud Hypervisor cloud-hypervisor`) with the
GuestKit runner's Linux `7.0.0-38-generic` kernel. The retained native
receipt, as independently reviewed by DISTILL, records:

| Native oracle | Result |
|---|---:|
| Complete cgroup-default bodies | 111 passed, 0 failed, 0 ignored |
| D26 identity starts | 13 |
| D26 exact one-pass summaries | 13 |
| Recognized runtime-skip diagnostics | 0 |
| D26 cleanup-scrub observations | 117 (`13 × 9`) |
| D26 route setup read-backs | 1 |
| D26 route cleanup read-backs | 1 |
| Disposable guest completions | 3 |
| Durable terminal | exit 0, `command_returned` |

The native output hash is
`197e7578db123a2297ebc180f2b4c35776ebd66debef5d733861ce207556941c`; the
terminal record hash is
`d37197a3749a4e78c73eae6997577018bae95818b75ba2c0329a058822a6df4f`.
Current source hashes match the native handoff exactly:

| Source | SHA-256 |
|---|---|
| `fwd_guest_suite.rs` | `e5a3d72b42a504def620d26b95b7474038ac13760af1cce0d3e421328bafec80` |
| `vsock_host_internal_enforce.rs` | `eb65379af9ff172eda3af666111cdf908ed4dabe7d56d41f4490d25852ff35b8` |
| `nft.rs` | `00a1b57ff41a2ebe4b5ede4acb2fb82b6bd3f98dff3a65b759dd888b1ba0f4e0` |
| `mtls_intercept_port.rs` | `f4a5be044f3ad4fa759d74aba546db02c23d281cb3afc1dd2ab45ad41c2b2444` |

The 90-file local SHA manifest is complete: all 90 listed files exist and
hash uniquely. The prior direct physical-Metal RED/GREEN/normal receipts and
the read-only post-run audit remain classified honestly as physical-host
history. That audit's later absence snapshot cannot prove the host was
unchanged and is not used as native proof. The current GuestKit receipt is
the accepted native boundary.

F-01 is therefore closed without adding a new test, lane, group, executor,
scenario, API or mechanism. The independently approved DISTILL review at
`docs/feature/netns-density-295/deliver/review-distill-13-06-native-support.md`
confirms the same bounded selection and receipt interpretation.

### F-02 disposition — CLOSED: normal package gate and exclusion ledger are complete

The normal gate is independently recorded in
`.context/netns-density-295/13-06/normal/normal-gate-manifest.md` and its
durable raw records. It runs as unprivileged `cargo xtask metal run --no-sudo`
against the implementation commit `b51d5f3dc` and source digest
`95d246cc50a641cdc2edeb6c171141bea44cbd1a7ffb9a21a919d68bd5b93f1b`:

| Normal evidence | Result |
|---|---|
| Inventory | 341 identities: 266 selected, 73 exact expression exclusions, 2 pre-existing ignored |
| Execution | 266 passed, 0 failed, 75 skipped, exit 0 |
| Runtime skip diagnostics | None in selected output |
| Exact package set | `overdrive-netlink` + `overdrive-worker`, `integration-tests` |
| Root/capability boundary | No selected identity ran as root or touched the physical kernel |

The 73 exclusions are explicit in the retained global filter, whose exact
identity count is 73. The manifest classifies them as 65 native identities
plus eight exact identities whose unchanged bodies emit the root/CAP
diagnostic. The eight are accounted as exclusions, never counted as passes.
Thirty-one of the 73 are covered by the current 111-body GuestKit receipt,
including all 13 D26 identities; the remaining 42 are pre-existing native
families with separately retained approved receipts. The two ignored tests
are named in the manifest and remain pre-existing. Thus the normal pass does
not silently treat capability failures, unrun native identities or ignored
tests as ordinary passing coverage.

The selected normal output has no runtime `SKIP` diagnostics and its 75 skip
total reconciles exactly to 73 filter exclusions plus the two ignored tests.
This closes F-02 while preserving the physical-Metal receipt limitation from
iteration 1.

### Revalidated implementation contract

The original source review remains valid against the remediation:

- `nft.rs:636-668` still masks the dedicated owner bit, rejects loopback,
  link-local and every non-unicast route type, and uses plain reject.
- `nft.rs:809-837` still places D26 indices 0-4 before every later accept,
  while preserving the separate ADR-0172 TCP reset and non-TCP plain reject.
- `nft.rs:3691-3738` still sends rule replacements in one nft transaction.
- The owner path still maps direct static/local checks to `HostInternal`, TCP
  kernel connect failure to `DestinationUnreachable`, UDP datagram-side socket
  error to `FlowCloseCause::DestinationUnreachable`, keeps service-VIP paths
  unmarked, and leaves the mesh leg-F connect unmarked.
- The implementation commit adds no public API, error variant, wire field,
  lifecycle owner or configuration surface. The remediation is test-support
  infrastructure only.
- The activated test file remains byte-identical to the implementation commit
  except for the 13 marker deletions already reviewed; the remediation does
  not alter any test body or assertion.

The typed semantic mapping remains:

| Designed failure | Approved representation | Current evidence |
|---|---|---|
| Owner's static/local check | `FlowRefusal::HostInternal` | Source path `owner_runtime.rs:1048-1053,1206-1225`; retained simulation acceptance |
| Plain-reject TCP connect | `FlowRefusal::DestinationUnreachable` | `owner_runtime.rs:1144-1152`; native D26 TCP errno cells pass |
| UDP first transmit `EPERM` | `FlowCloseCause::DestinationUnreachable` | `owner_runtime.rs:1397-1407`; existing datagram acceptance and native EPERM cells |
| Gateway:53 | Accepted before D26 reject | Native GuestKit and source rule order |
| Mesh/VIP/unmarked paths | Existing unmarked peer specifications | Source and retained unaffected-path cells |

### Remediation verification

| Check | Result |
|---|---|
| DISTILL support review | **APPROVED**, zero open findings |
| Native GuestKit run | **PASS**, durable run `1bb49a61e3a35201fdc93597`, exit 0 |
| Normal inventory | **PASS**, durable run `e79f87b6289f966ce5c35221`, exit 0 |
| Normal execution | **PASS**, durable run `8598d106dfb716885f917f8f`, 266/266 passed |
| Source identity | **PASS**, current hashes match native handoff |
| Source fmt/check/Clippy | **PASS**, durable run `fa13f31bb138b5c40cc2a202`, exit 0 |
| Receipt SHA coverage | **PASS**, 90/90 local receipt files present and hash-unique |
| Physical-Metal audit honesty | **PASS**, prior direct-host limitation retained explicitly |
| Mutation testing | Correctly deferred; no mutation run performed |

### Iteration 2 verdict

Both blocking findings from iteration 1 are closed. The 13 D26 bodies now run
through the existing disposable GuestKit boundary with exact identity,
module, route, cleanup, skip and count controls. The normal package gate has
an independent unprivileged inventory/execution ledger with explicit exact
exclusions, named ignored tests and no silently accepted root/capability
coverage gaps. Prior direct physical-Metal receipts remain historical and are
not misrepresented as native proof.

No unresolved API, typed-error, D26 semantic, lifecycle, test-integrity,
scope, source-identity, normal-gate or native-boundary finding remains.

**Verdict: APPROVED.** STEP 13-06 may advance under the strict DELIVER
sequence. No mutation testing was run, consistent with the explicit user
override.
