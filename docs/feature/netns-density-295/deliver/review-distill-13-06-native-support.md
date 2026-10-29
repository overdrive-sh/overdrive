# DISTILL review — STEP 13-06 existing GuestKit native support

## Review metadata

| Field | Value |
| --- | --- |
| Review ID | `STEP13-06-DISTILL-NATIVE-SUPPORT-REVIEW-01` |
| Feature / roadmap step | `netns-density-295` / `13-06` |
| Review date | 2026-10-29 |
| Reviewer | Fresh independent DISTILL source and evidence reviewer |
| Iteration | 1 |
| Implementation base | `b51d5f3dc1e35483abe6d6809fb29c3fd38b15e5` |
| Scope | Existing `cgroup-default` support for the 13 already-authored D26 native cells, retained group isolation, disposable-guest confinement, exact accounting and the supplied durable native receipts |
| Design authority | Feature-delta D26 and D26-PINS; roadmap and dispatch STEP 13-06; DELIVER review `F-01` |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This review applies the repository's current `AGENTS.md`, applicable project
rules, and the nWave DISTILL, acceptance-review, completeness, test-design and
BDD criteria. The repository's direct Rust scenarios and native-Markdown
review convention take precedence over the installed review definition's
generic `.feature` and YAML examples.

Only this review artifact was added by the reviewer. No production source,
test source, runner source, acceptance body, assertion, Contract Shape
declaration, pending marker, roadmap, DES event, execution log, model, lane,
group, public API or commit was changed.

## Review boundary

The bounded DISTILL delta is the uncommitted change to
`crates/overdrive-testing/bin/fwd_guest_suite.rs` against implementation base
`b51d5f3dc`. It closes the execution-support half of DELIVER review finding
`F-01` by:

- adding the 13 existing `vsock_host_internal_enforce` identities to the
  existing `cgroup-default` worker guest;
- staging the exact `nft_fib` and `nft_fib_ipv4` module closure only for the
  worker run that selects those identities;
- providing a bounded dummy default route only around the three authored UDP
  bodies whose first transmit otherwise cannot reach the output hook in the
  no-NIC guest;
- retaining the existing exact-summary and runtime-skip checks for every D26
  invocation; and
- extending the existing fail-closed `cgroup-default` accounting from 98 to
  exactly 111 passing bodies.

The delta adds no scenario, test body, assertion, fixture inside the test
module, selectable group, lane, guest executor, production mechanism, public
surface or error representation. The original STEP 13-06 crafter still owns
DELIVER review finding `F-02`: the separate normal package receipt and the
final implementation-review remediation. That remaining implementation gate
is outside this DISTILL support approval.

## Reviewed artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| `deliver/distill-13-06-native-support.md` | `ae8377d406c0a10553846e94459b00f422dd6796e89afd2bfad43f11066c50d1` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `e5a3d72b42a504def620d26b95b7474038ac13760af1cce0d3e421328bafec80` |
| `overdrive-worker/tests/integration/vsock_host_internal_enforce.rs` | `eb65379af9ff172eda3af666111cdf908ed4dabe7d56d41f4490d25852ff35b8` |
| `overdrive-netlink/src/nft.rs` | `00a1b57ff41a2ebe4b5ede4acb2fb82b6bd3f98dff3a65b759dd888b1ba0f4e0` |
| `overdrive-worker/src/mtls_intercept_port.rs` | `f4a5be044f3ad4fa759d74aba546db02c23d281cb3afc1dd2ab45ad41c2b2444` |
| `feature-delta.md` | `66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09` |
| STEP 13-06 dispatch | `85140fa3900f2a7a848730331f8b00d42acddabde27a232afd396ea18339ca73` |
| DELIVER review iteration 1 | `2b859af38b8ebcee90aecbaadc457c4dca13ee29e321d3ac034dcf75d43322e6` |
| `overdrive-testing/src/guest_kit.rs` | `9061cd378171fb864d4bc5fcfa51305604cf15dcd35af759f8305dc78f1dc54e` — unchanged confinement source |
| `overdrive-testing/bin/guest_peer.rs` | `ea3de0694b0164c7b8c84b3a08db2d6f275b72b252e29b10ef8eed31fb6cf8ea` — unchanged in-guest executor |
| `xtask/src/guest_kit_prepare.rs` | `086f1214d6244e801d492e2211a1c99cadf771265d94f2268ab502bebbf951e1` — unchanged preparation source |

The runner's Git-blob identity is
`80ed8434f171f55b4be7e6afa97516ddf6ea4e57`; the unchanged D26 test file's
Git-blob identity is `965146f85b938d630656552c0e1ea0ac23ef89eb`.

## Strengths

- The source reuses the existing `cgroup-default` group, the same three
  disposable Cloud Hypervisor guests, the same CID slots, the same scripted
  peer and the same prepared integration binaries. It adds no selector or
  executor.
- All 13 manifest entries are exact libtest identities with expected count
  one. Static comparison against the live async test functions found no
  missing or extra D26 body.
- Every D26 invocation uses `exec_cgroup_required`, which requires process
  exit zero, the exact `1 passed; 0 failed` summary and absence of a recognized
  runtime-skip diagnostic before its count advances.
- The existing 55-body baseline, 18-body STEP 13-04 extension, five-body STEP
  13-05 extension and 20-body control-plane normal-native extension retain
  their independent checks. D26 then has its own exact 13-body check before
  the complete 111-body check.
- The temporary route is narrower than the earlier STEP 13-04 fixture. It
  uses a distinct interface, source address and metric, runs around only three
  bodies, and proves the off-host destination is neither pre-existing nor
  local before and after the route lifetime.
- Module staging is derived from non-empty D26 manifests per staged binary.
  Only the worker tuple is non-empty, so the control-plane guests and the
  unrelated `mtls-relay` guest do not receive the two fib modules.
- The successful durable native receipt is non-vacuous: all 111 libtest
  passes reconcile, all 13 D26 identities have their own one-pass summary,
  no ignored body or runtime skip is present, cleanup observations are
  complete, and all three disposable guests finish before the exact exit-zero
  terminal record.

## Exact scenario selection and Contract Shape

The manifest selects the complete existing D26 module exactly once:

| Route lifetime | Exact selected bodies | Count | Result |
| --- | --- | ---: | --- |
| Ordinary no-NIC guest state | loopback TCP refusal; host-interface TCP refusal; local UDP first-transmit refusal; established-tuple reuse refusal; subnet-directed broadcast absence; link-local refusal; unicast-neighbour positive control; masked owner-bit refusal; leg-S/TPROXY positive controls; atomic-replacement storm | 10 | Each exact identity followed by `1 passed; 0 failed` |
| Bounded D26 dummy default | limited-broadcast absence; all-hosts multicast `EPERM`; off-host unicast positive control | 3 | Each exact identity followed by `1 passed; 0 failed` |

Independent static checks found:

| Check | Result |
| --- | --- |
| Exact D26 manifest identities | 13 |
| Live authored D26 async tests | 13 |
| Missing or extra manifest identity | 0 |
| `/// CONTRACT_SHAPE:` declarations | 13 |
| Live `#[ignore]` markers | 0 |
| Test-file diff from `b51d5f3dc` | 0 lines |

The support change therefore preserves every body and oracle already accepted
by the implementation review: exact `ECONNREFUSED` and first-transmit
`EPERM`, no host-socket delivery, established-tuple non-bypass, non-unicast
coverage, unicast controls, masked-mark selectivity and zero leakage during
repeated production table replacement. Every body still boots the production
shared table through `MtlsInterceptWorker::start_shared_owner` over the real
`HostMtlsIntercept` path.

## Route precondition, read-back and cleanup

The no-NIC GuestKit image needs a main-table route for the limited-broadcast,
multicast and off-host UDP sends to reach the output hook. Reusing the STEP
13-04 fixture would be invalid because it assigns `192.0.2.1` locally, while
the unchanged off-host control uses `192.0.2.1` as its remote destination.

`STEP13_06_ROUTE_SETUP` instead fails closed unless `192.0.2.1` is initially
unroutable and absent from the local table. It creates only:

```text
ovd-gk-d26
198.51.100.1/32
default dev ovd-gk-d26 src 198.51.100.1 metric 42761
```

It then requires the route lookup to name both `ovd-gk-d26` and source
`198.51.100.1`, and again requires no local-table entry for
`192.0.2.1`. The successful raw receipt records exactly one setup read-back:

```text
192.0.2.1 dev ovd-gk-d26 src 198.51.100.1 uid 0
    cache
```

The string annotation `destination_type=unicast` is not the only proof of
that type. The local-table absence is checked before and after setup, and the
unchanged marked off-host body succeeds. If the kernel classified
`192.0.2.1` as local or non-unicast, D26 would return `EPERM` and that exact
body would fail.

After the three routed bodies, cleanup deletes the exact metric-42761 default
and `ovd-gk-d26`, requires the destination to be unroutable again, and again
requires no local-table entry. The raw receipt contains exactly one cleanup
read-back:

```text
STEP13-06 route cleanup readback: unroutable destination_not_local=true
```

Cleanup is attempted whether a routed body passes or fails. A successful body
count is retained only if cleanup also succeeds. On a body failure the
original error is returned after the cleanup attempt, and the disposable
guest remains the final containment boundary.

Raw ordering proves the setup occurs after the ten route-independent D26
identities and before only the final three. It does not leak into the existing
STEP 13-04, STEP 13-05 or control-plane normal-native families.

## Module and tool closure

Durable Metal probe `02f81d15e5eedec1514da1c3` ran on
`7.0.0-38-generic` and exited zero. It resolved:

| Module | Qualified source path | Declared dependency |
| --- | --- | --- |
| `nft_fib` | `net/netfilter/nft_fib` | `nf_tables` |
| `nft_fib_ipv4` | `net/ipv4/netfilter/nft_fib_ipv4` | `nf_tables,nft_fib` |

The existing worker guest already stages `nfnetlink`, `nf_tables`, `veth`,
`tls`, the STEP 13-04 reject/TProxy closure, the dummy module, `ip`, `nft`,
`uname`, cgroup v2, bpffs, the `/var/run -> /run` compatibility alias and the
prepared worker integration ELF with shared libraries. Source inspection of
the 13 unchanged bodies and their transitive fixture helpers found no new
tool or workload command outside that existing closure.

`STEP13_06_NFT_MODULES` is staged only when one of the two D26 slices for a
particular `run_cgroup_binary` call is non-empty. Both control-plane calls pass
empty D26 slices. The worker call alone stages and loads `nft_fib` followed by
`nft_fib_ipv4` inside the guest through the existing `RUN_WITH_MODULES`
protocol.

## Group isolation, confinement and security boundary

The runner delta does not alter `GuestKit`, `guest_peer`, Cloud Hypervisor or
the group registry. The physical Metal process stages files, builds the guest
image and launches the existing VMM. Module loading, route changes, nft
programming, netns creation, cgroup and bpffs mounts, marked sockets and test
cleanup all occur through the scripted peer inside disposable guests.

The existing VMM uid/gid 4200 confinement, actual device-owner supplementary
groups, fixture-parent traversal, per-allocation ownership, Landlock grant,
launch seccomp filter, `no_new_privs`, device modes and VM lifetime remain
unchanged. The successful receipt contains the three expected completion
headers and no fourth guest:

```text
cgroup-control-plane-acceptance
cgroup-control-plane-integration
cgroup-worker-integration
```

`GuestRun::finish` powers each guest off and requires its VM exit within the
existing bound. The durable command publishes its terminal result only after
all three calls return. No physical-host nft, route, netns, cgroup or module
mutation was introduced.

The `mtls-relay` path is source-unchanged. Its constants remain exactly
seven original TLS bodies, 36 dataplane integration bodies and three
dataplane library bodies, for 46 total. No D26 module, route, manifest or
count is reachable from `run_mtls_relay`; its current retained 46-body receipt
therefore did not need repetition for this worker-only delta.

## Fail-closed count and skip accounting

The source retains the prior boundary before adding D26:

| Boundary | Exact passing bodies |
| --- | ---: |
| Existing baseline | 55 |
| STEP 13-04 extension | 18 |
| STEP 13-05 extension | 5 |
| Control-plane normal-native extension | 20 |
| **Retained subtotal** | **98** |
| STEP 13-06 D26 extension | 13 |
| **Complete `cgroup-default` boundary** | **111** |

Each subtotal is checked independently before the next total is accepted.
The D26 count is computed from the three binary results at the dedicated tuple
index; only the worker contributes 13.

Independent read-only collection and full-stream audit of durable run
`1bb49a61e3a35201fdc93597` produced:

| Audit | Result |
| --- | ---: |
| Libtest summaries | 101, all `ok` |
| Sum of passed bodies | 111 |
| Sum of failed bodies | 0 |
| Sum of ignored bodies | 0 |
| D26 identity starts | 13 |
| D26 exact one-pass summaries | 13 |
| Recognized runtime-skip diagnostics | 0 |
| D26 cleanup-scrub observations | 117 = 13 × 9 |
| D26 route setup read-backs | 1 |
| D26 route cleanup read-backs | 1 |
| Panic or `FAILED` diagnostics | 0 |
| Disposable guest completions | 3 |

The D26 test writes evidence after libtest has emitted its `...` marker, so
the final `ok` text is not on the same physical line as the identity. The
sequence audit therefore tracked each identity until its following exact
summary instead of relying on a same-line `... ok` grep. All 13 sequences
closed normally.

The existing line-oriented skip detector remains unchanged. It checks leading
diagnostics and text after a libtest `" ... "` prefix for `[skip]`, `SKIP `,
`SKIP:`, `skip:` and `(skip)`. D26 uses the required form of the cgroup
executor, so a root or capability bailout cannot increment the 13-body count.

## Durable receipts and source identity

### Complete existing cgroup group plus D26

Durable run `1bb49a61e3a35201fdc93597` executed the handed-off command:

```text
cargo xtask metal run -- bash -c 'set -eu; OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build; cargo xtask guest-kit-prepare; OVERDRIVE_FWD_GUEST_GROUP=cgroup-default cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests'
```

Repeated read-only `cargo xtask metal collect` calls returned:

```text
OVERDRIVE_METAL_RUN_COMPLETE run_id=1bb49a61e3a35201fdc93597 exit_code=0 cause=command_returned
```

The collector writes a 153-byte SSH/bootstrap preamble before the retained
command log. Removing that exact collector-only preamble independently
reproduced the handed-off raw artifact identity:

| Receipt property | Verified value |
| --- | --- |
| Retained `output.log` length | 397,357 bytes |
| Retained `output.log` SHA-256 | `197e7578db123a2297ebc180f2b4c35776ebd66debef5d733861ce207556941c` |
| Exact terminal exit | 0 |
| Terminal cause | `command_returned` |

The native run occurred after the runner's final source modification, and the
current source hash and Git-blob identity match the handoff. The receipt's
exact selected identities, route read-backs and 111-body accounting also match
that source shape.

### Source checks

Durable Metal run `fa13f31bb138b5c40cc2a202` exited zero for:

```text
cargo fmt --all -- --check
cargo check -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests
cargo clippy -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests -- -D warnings
```

The reviewer also ran scoped `git diff --check` successfully. No new broad or
kernel-mutating run was needed; the existing full native receipt is current
and was recollected rather than duplicated.

## Acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | Refusal cells retain positive delivery controls, and neighbour, off-host, leg-S and TPROXY complements remain selected. |
| GWT / one behavior | 9/10 | Each Rust cell retains one concrete production trigger and observable outcome; the support delta changes no body. |
| Domain language | 10/10 | Host-internal destination, owner bit, route type, service, neighbour, replacement and refusal terms remain exact. |
| Coverage completeness | 10/10 | All 13 existing D26 cells are selected exactly once and pass non-vacuously. |
| User/operator value | 9/10 | The focused Tier-3 cells prove guest flow refusal and unaffected paths at the real kernel boundary. |
| Priority validation | 10/10 | The delta closes the reproduced missing GuestKit selection without broadening the scenario or execution surface. |
| Observable behavior | 10/10 | Exact errno, payload delivery, no-host-service, table identity and atomic-replacement outcomes remain asserted. |
| Traceability | 10/10 | Exact bodies map to D26, D26-PINS, R5-6, R5-7, R5-48 and DELIVER finding F-01. |
| Walking-skeleton boundary | 10/10 N/A | No new walking skeleton is authored; existing focused real-kernel cells retain their production adapter boundary. |

All scored dimensions meet the approval threshold and no blocker remains in
the bounded DISTILL package.

## Test-design mandates and completeness

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | PASS | Every D26 body still enters `MtlsInterceptWorker::start_shared_owner` over real host adapters. |
| CM-B — domain abstraction | PASS | No acceptance vocabulary or assertion changed; fixture commands remain private runner support. |
| CM-C — complete focused journey | PASS | Each selected body retains stimulus, exact kernel-visible outcome, complement and cleanup. |
| CM-D — fixture isolation | PASS | Support configures only disposable guests and delegates behavior to production owners. |
| CM-E — universe-bound assertion | PASS / layer N/A | These are layer-4+ real-kernel integration cells, where direct observable assertions are permitted. |
| CM-F — layer-dependent PBT | PASS | The Tier-3 cells remain explicit examples; no PBT machinery was added. |
| CM-G — two-tier rich journey | N/A | No new rich chained journey or Tier-B surface is introduced. |
| CM-H — explicit integration sad paths | PASS | Every refusal remains a named, explicit real-I/O example with its exact outcome. |

The C1-C7 taxonomy is unchanged because this support package adds no input
class, state, operation, mode, error or concurrency claim. Its
delta-completeness question is exact execution selection and dependency
closure. All 13 accepted partitions are selected once, all route and module
preconditions are fail-closed, and all cleanup and skip complements are
observed. The canonical 15-item checklist remains **15/15** under its N/A
rule.

## Finding dispositions

### F-01 from DELIVER review — missing native D26 GuestKit boundary and manifest

**Disposition: CLOSED for DISTILL support.**

The existing `cgroup-default` worker manifest now selects all 13 exact D26
identities, stages only the required fib modules, supplies the bounded route
precondition only to the three affected UDP bodies, retains exact-summary and
no-skip checks, and reconciles 98 retained plus 13 D26 bodies to 111. Durable
run `1bb49a61e3a35201fdc93597` proves the complete group inside three
disposable Cloud Hypervisor guests with exact exit zero, full raw identity,
zero ignored bodies, zero recognized runtime skips and complete cleanup.

### New review findings

None.

### DELIVER review F-02

**Disposition: OUTSIDE THIS REVIEW; remains assigned to the original STEP
13-06 crafter.**

This approval supplies the native half of the missing evidence. It does not
claim the separate normal `overdrive-netlink` plus `overdrive-worker` package
gate, does not amend the original implementation review and does not approve
the implementation step itself. The original crafter must retain that normal
receipt and persist the bounded remediation for the same implementation
reviewer's next iteration.

## Mechanical verification

| Check | Result |
| --- | --- |
| Scoped source diff | PASS — one private test-support binary, 195 insertions / 9 deletions; no product or test-source change. |
| Exact D26 identity set | PASS — 13 manifest entries equal 13 live authored bodies. |
| Contract Shape | PASS — 13 declarations, 0 ignores. |
| Retained count | PASS — 55 + 18 + 5 + 20 = 98. |
| D26 count | PASS — exactly 13. |
| Complete count | PASS — exactly 111. |
| Module closure | PASS — `nft_fib` then `nft_fib_ipv4`, probed on the qualified kernel. |
| Module isolation | PASS — staged only for the D26 worker guest. |
| Route precondition | PASS — initially unroutable/non-local, exact interface and source read-back. |
| Route scope | PASS — only the final three UDP bodies. |
| Route cleanup | PASS — exact default/link removed; unroutable and non-local again. |
| Full native raw | PASS — 397,357 bytes, exact SHA-256 reproduced. |
| Full native exit | PASS — durable exact exit 0, `command_returned`. |
| Full native libtest accounting | PASS — 111 passed, 0 failed, 0 ignored. |
| D26 sequence accounting | PASS — 13 starts, 13 exact one-pass summaries. |
| Runtime-skip audit | PASS — 0 recognized diagnostics. |
| D26 cleanup audit | PASS — 117 authored scrub observations plus one route cleanup. |
| Disposable guest lifecycle | PASS — exactly three completion headers and terminal exit after all three. |
| Source format/check/Clippy | PASS — durable Metal `fa13f31bb138b5c40cc2a202`, exit 0. |
| `git diff --check` | PASS. |
| `mtls-relay` | PASS by unchanged source and retained receipt — exact 46-body boundary untouched. |
| New API / scenario / lane / group / executor / model / DES event | NONE. |
| Mutation testing | NOT RUN — explicitly unauthorized for this step. |

## Iteration 1 verdict

**APPROVED.** The bounded runner support closes DELIVER finding F-01 without
changing any accepted body or product contract. It selects exactly the 13 D26
cells through the existing disposable GuestKit worker guest, stages only the
two required fib modules, scopes an honest remote-unicast route around only
the three affected UDP bodies, and fails closed on identity, summary, skip,
count and cleanup evidence.

The full durable run proves 111 passed, zero failed, zero ignored, all 13 D26
one-pass sequences, zero recognized runtime skips, complete authored cleanup,
three disposable guest completions and exact exit zero. The raw artifact hash
and current source identity are independently reconciled.

There are no open DISTILL findings. The original STEP 13-06 crafter may now
persist this approved support and produce the separate normal package receipt
for DELIVER review F-02. This verdict does not pre-approve that later
implementation-review iteration.

# APPROVED
