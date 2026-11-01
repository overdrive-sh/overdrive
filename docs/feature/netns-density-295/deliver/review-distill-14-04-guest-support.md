# DISTILL review — STEP 14-04 existing-test mapping and GuestKit support

## Review metadata

| Field | Value |
| --- | --- |
| Feature / step | `netns-density-295` / `14-04` |
| Review role | Fresh independent `nw-acceptance-designer-reviewer` |
| Review type | Source-only review of existing acceptance mapping and existing GuestKit support |
| Review iterations | 1, 2, 3, 4, 5, 6, 7 and 8 |
| Date | 2026-11-01 |
| Reviewed HEAD | `db7629972071523535ddbb76983ef1f6e309dd9b` |
| Reviewed handoff | `docs/feature/netns-density-295/deliver/distill-14-04-guest-support.md` |
| Iteration-1 reviewed support delta | `crates/overdrive-testing/bin/fwd_guest_suite.rs`; documentation-only update in `xtask/src/guest_kit_prepare.rs` |
| Iteration-2 reviewed acceptance/support delta | `guest_stack_mtls_egress.rs`; `vsock_walking_skeleton.rs`; `vsock_mesh_egress_mtls.rs`; the same runner and xtask staging files; corrected retained ledger and handoff |
| Iteration-3 reviewed fixture delta | Bounded correction in existing `vsock_mesh_egress_mtls.rs` body plus updated handoff/native-RED ledger |
| Iteration-4 reviewed lifecycle delta | Private natural-completion/operator-stop helpers in `guest_stack_mtls_egress.rs`, the existing V-6 call site, and native run `5e5f472e4191780088cc1a96` |
| Iteration-5 reviewed typed-field delta | Corrected settled row/transition reason expectations in the same private Service helper; no other source scope |
| Iteration-6 reviewed failure-cleanup delta | Existing `guest_flow_through_serve` relay-watch body, native run `129e9c3d78f087fc13e8868e`, and corrected evidence attribution |
| Iteration-7 reviewed reporting delta | Independent panic capture for each existing cleanup step and primary-first combined failure report in the same relay-watch body |
| Iteration-8 reviewed precondition delta | Existing public typed readiness snapshot immediately before the same one-shot client in the same relay-watch body |
| Reviewer native execution | **NOT RUN** |
| Retained native evidence | Prior fixture/lifecycle REDs; `129e9c3d78f087fc13e8868e` — third control-plane body started but never completed; exact-46 mtls-relay receipt remains retained |
| Support-source verdict | **APPROVED** — E23 is correctly added to the existing staged worker ELF and disposable `cgroup-default` boundary |
| Iteration-1 verdict | **NEEDS_REVISION** — preserved below with both original findings |
| Iteration-2 frozen-outcome mapping verdict | **APPROVED** — every retained assertion now has an exact existing current identity and ordinary observable oracle |
| Iteration-3 fixture-correction verdict | **APPROVED** — causal diagnosis and bounded two-phase fixture correction are exact |
| Iteration-4 lifecycle-correction verdict | **NEEDS_REVISION** — prepared helper assigns operator attribution to two fields whose accepted representation is reconciler-attributed |
| Iteration-5 typed-field verdict | **APPROVED** — settled reason, terminal condition and exit-code meanings now match the accepted representation exactly |
| Iteration-6 failure-cleanup verdict | **NEEDS_REVISION** — primary failure is preserved only when every cleanup operation succeeds |
| Iteration-7 cleanup-reporting verdict | **APPROVED** — every cleanup is attempted and the original relay failure remains first even when cleanup also fails |
| Iteration-8 readiness-precondition verdict | **APPROVED** — exact current LWW readiness input is required without weakening any relay outcome |
| Current overall verdict | **APPROVED FOR ONE CURRENT CGROUP-DEFAULT119 GUESTKIT VALIDATION; NATIVE GREEN UNPROVEN** |

This review changes no production source, test body, marker, runner, model,
DES log, roadmap, design artifact or commit. It performs no Metal, GuestKit,
kernel, module, BPF, nftables, cgroup, network-namespace, route, kTLS or VM
execution. The only authored file is this review. The user-owned dirty
`AGENTS.md` and all other worktree changes are preserved.

The retained source gate `b27ef0ffbbe5d2bb222b8857` is accepted as the
handoff's source-only evidence: it used `--no-sudo`, the task-owned
`target/distill-14-04-guest-support` target, and exited zero after format,
check and scoped Clippy for the GuestKit runner and xtask. This reviewer also
checked the local diff mechanically with `git diff --check`; no test or native
job was launched.

## Iteration 1

## Authority and review boundary

STEP 14-04 is a revalidation step. It may reuse existing tests and existing
GuestKit groups, but it may not add a scenario, test body, lane, executor,
source-copy launcher, production seam, public API, typed-error representation
or deferral. Frozen 04-03 / S-ND295-36 requires the named existing suites to
remain green with no assertion weakened. ADR-0145 and the accepted current
feature delta replace the bridge/TAP attachment with the NIC-less,
vsock-only attachment, so bridge/TAP-specific assertions are not restored.
That architectural replacement does not by itself erase attachment-neutral
identity, lifecycle or mTLS-owner guarantees that the frozen bodies asserted.

The review therefore separates two questions:

1. whether the existing current identities are staged safely and counted
   honestly; and
2. whether the proposed current identities actually prove every retained
   observable of the frozen bodies.

The first passes. The second has one blocking gap.

## What is correct

### Historical provenance and accepted replacement boundary

The two frozen bodies are real historical tests, not names reconstructed from
prose:

- commit `e98a5382b7cd7d2efd472f83e0a3d6a9ba65c2fe` contains
  `microvm_dials_a_mesh_peer_by_name_and_receives_the_reply` at historical
  `guest_stack_mtls_egress.rs:5135` and
  `the_guests_mesh_traffic_travels_the_peer_wire_as_mtls_never_in_the_clear`
  at `:5290`;
- commit `b8bdf600511e5432cfddf2aa098b203277e107de` removes their shared
  bridge/TAP `run_mesh_guest_scenario` composition on 2026-10-10; and
- ADR-0145 records the user-approved 2026-10-11 END-STATE: the guest remains
  NIC-less and vsock-only. The accepted feature delta retains ADR-0072
  dial-by-name, replaces old 10-01 with V-6, and keeps 10-04 as a revalidation
  against the vsock attachment.

It is therefore correct not to restore the historical bridge, per-VM TAP,
TAP queue-holder, per-TAP intercept-event or virtio-net-header fixtures. Such
a restoration would contradict the current accepted attachment and would be
a new body disguised under an old name.

The existing V-6 identity is also a legitimate part of the mapping. Its
approved production-composition body proves, through public `serve` and
`deploy`:

- the client VM Service uses gateway DNS and resolves
  `v6-peer.svc.overdrive.local`;
- the reply-dependent issued-VIP TCP action completes;
- a loss-accounted loopback capture correlates one bidirectional TLS 1.3
  software-TX/RX kTLS socket to the backend tuple, sees application-data
  records in both directions and sees neither cleartext marker;
- the same-node tuple emits no uplink frame under a live positive-control
  capture; and
- public stop makes the client and peer allocations terminal, removes their
  VMMs, removes the held population and restores the test-owned ephemeral
  range.

Those are exact current outcomes, and the prior approved V-6 review remains
valid for its own contract. The earlier V-6 native receipt is source-specific,
however. It predates STEP 14-04's HTTP-prober response-disposal change, so it
cannot replace the current native run required below.

### Existing suite and GuestKit mapping

The remaining required families map to existing bodies without creating a
duplicate:

| Required outcome | Existing exact support |
| --- | --- |
| Real DNS responder | Seven `integration::dns_responder_bind::*` rows in `CONTROL_PLANE_NORMAL_NATIVE` |
| Exec/process netns and veth | Two `serve_boot_provisions_veth`, seven `veth_provision_idempotent` and two `workload_netns_provision` rows in `CONTROL_PLANE_NORMAL_NATIVE` |
| Constant-prefix leg C | `integration::vsock_constant_guest_prefix_rules::a_leg_c_diverted_remote_connection_reaches_leg_c_both_ways` in the existing STEP 13-04 remote-firewall manifest |
| Real mTLS/kTLS/splice leg-C path | `integration::mtls_inbound_enforce::inbound_enforce_origdst_server_mtls_ktls_rx_splice_to_server` in `DATAPLANE_NATIVE` |
| Service dataplane selection | `integration::service_map_forward::ten_tcp_syns_to_vip_are_rewritten_and_forwarded_via_veth` in `DATAPLANE_NATIVE` |
| Current V-6 production composition | `integration::vsock_walking_skeleton::a_vm_workload_round_trips_tcp_and_udp_through_serve_and_deploy` in the existing STEP 14-01 CLI manifest |
| E23 | The existing `integration::real_cgroup_fs::refused_thread_kill::a_kill_write_under_thread_refusal_returns_an_io_error_and_never_aborts`, now one exact row in `CGROUP_WORKER` |

The retained userspace records independently cover the existing
`service_kind_vm_workloads`, all six `mtls_resolve_rekey` properties,
`cgroup_accounting_equivalence`, S-SVM-14, exec-release Sim composition,
guest-network Sim composition, DNS semantic/codec acceptance and the safe CLI
Service subset. The eight successful nextest selections report exactly
7/7, 1/1, 7/7, 2/2, 3/3, 7/7, 32/32 and 2/2, with exit zero. The four source
gates also exit zero. The two E6 fixture failures remain the exact known
baseline and supply no positive guest-stack credit.

The 60 retained files under `.context/netns-density-295/14-04/raw/` match
`raw-sha256sums.txt`. No native ELF, native runtime-skip or native cleanup
receipt is claimed in that package.

### E23 body, staging and cleanup

E23 is the existing body and keeps its exact declaration:

```text
/// E23 — A refused OS thread is a typed failure, never a panic.
/// CONTRACT_SHAPE: unbounded-preservation.
```

The pending marker at `refused_thread_kill.rs:42` remains in place for the
original STEP 14-04 crafter to remove only after this support/mapping review
cycle is approved. With the marker present, the exact manifest invocation
cannot receive false credit: libtest reports zero passed and one ignored,
while the runner requires the exact one-pass summary.

The body creates its allocation scope before installing the thread cap, moves
its own process into a scratch cgroup, caps `pids.max` at the current task
count, and invokes the retained allocation-scoped
`CgroupManager::cgroup_kill`. A missing cgroup/pids capability is an `expect`
failure, not a runtime skip. `ThreadCreationRefused` restores the process to
its original cgroup and removes the scratch leaf before `AllocCleanup` kills,
reaps and removes the allocation scope; reverse declaration-order drop makes
that cleanup order deterministic. If the pre-fix abort recurs, the exact body
fails and disposal of the entire outer guest remains the fail-closed cleanup
boundary.

`guest-kit-prepare` already stages the current
`overdrive-worker --test integration` ELF. It obtains Cargo's actual test
executable from JSON compiler-artifact output and copies that ELF to the
existing `target/guest-kit/in-guest/overdrive-worker--integration` contract
path. `InGuestTests::prepared` reads that staged path. The change adds no lane,
source copy or generated launcher.

The existing cgroup-default worker guest mounts a private cgroup-v2 hierarchy
and explicitly requires and enables `cpu`, `memory`, `io` and `pids` before
the worker integration ELF runs. All E23 cgroup effects therefore stay in
that disposable outer guest.

### Manifest accounting and vacuity checks

The source accounting is internally consistent:

| Segment | Exact expected bodies |
| --- | ---: |
| `CGROUP_ACCEPTANCE` | 3 |
| `CGROUP_CONTROL_PLANE` | 45 |
| `CGROUP_WORKER` after E23 | 8 |
| **Retained cgroup baseline** | **56** |
| STEP 13-04 extension | 18 |
| STEP 13-05 extension | 5 |
| Control-plane normal-native suffix | 20 |
| STEP 13-06 extension | 13 |
| **Complete pre-STEP-14 boundary** | **112** |
| Existing STEP 14-01 CLI + control-plane bodies | 6 |
| **Canonical cgroup-default result** | **118** |

The support diff changes only the worker baseline from 7 to 8, the baseline
constant from 55 to 56, and the focused diagnostic label from canonical 117
to 118. The STEP 13-04, STEP 13-05, normal-native, STEP 13-06 and STEP 14-01
manifests are unchanged. `mtls-relay` remains exactly 46 bodies.

A static comparison of every relevant cgroup-default and mtls-relay filter
found zero duplicate exact filters and zero prefix-overlap pairs. E23's exact
filter occurs once. Every exact invocation requires exit zero and its exact
pass count, so zero-match and ignored-body runs fail closed. Required native
suffix, nested and mtls-relay cells also reject recognized runtime-SKIP
diagnostics. E23 uses the legacy baseline execution helper, but the current
body contains no skip branch: each missing prerequisite panics through
`expect`, and its pending marker produces a zero-pass summary. The future
native review must additionally confirm that the complete raw group outputs
contain zero runtime-SKIP diagnostics; no new runner mechanism is needed for
this existing body.

### Outer production VMM boundary

Both native groups use the established disposable GuestKit boundary.
`GuestKit::boot` creates the outer guest through production
`CloudHypervisorVmm::create` over a production `VmConfig` with
`VmConfinement::confined`. The confined identity is uid/gid 4200 plus the
actual supplementary groups owning `/dev/kvm` and `/dev/vhost-vsock`; the
production launch path applies its Landlock and seccomp confinement. The
runner does not render a parallel VMM command line.

The V-6 body is the one nested case: its existing `step14-01-cli` outer guest
mounts private cgroup/bpffs/securityfs state and launches workload VMs through
the production `CloudHypervisorVmm` composition from inside that outer guest.
E23 launches no nested VM. `GuestRun::finish` powers off and waits for every
outer guest; explicit route and normal-native cleanup runs before that final
disposal. Metal supplies transport and KVM only. No physical-host native
fallback or cleanup is part of this mapping.

## Findings

### F-01 — Blocker — the proposed V-6 substitution omits retained frozen-body guarantees

**Kind:** `ACCEPTANCE_MAPPING`  
**Severity:** blocker  
**Disposition:** OPEN

The handoff maps both deleted frozen identities to the one current V-6 body,
but its table accounts only for dial-by-name/reply, TLS/no-cleartext, general
round-trip success and a reduced cleanup statement. Historical source proves
that the frozen bodies asserted additional guarantees that do not depend on a
bridge or TAP:

- the first body requires the exact per-allocation SPIFFE identity for both
  the peer Service and caller allocation (`e98a5382b` historical
  `guest_stack_mtls_egress.rs:5150-5163`);
- it applies `assert_exact_lifecycle_delta` to the complete public
  `AllocStatusRowBody` universe (`:5147-5149`; helper `:2688-2734`), allowing
  only the pinned lifecycle fields to change; and
- the wire body requires the exact live kTLS socket to be the socket used by
  both production splice directions (`:5297-5323`, especially
  `require_same_socket_splice`).

Those are not the obsolete shared-bridge, per-TAP intercept, TAP queue-holder
or virtio-net-header assertions that ADR-0145 superseded. They remain
attachment-neutral identity, observable lifecycle and mTLS-owner/mechanism
guarantees. No accepted contract quoted by the handoff removes them.

The current V-6 body does not prove them:

- it never reads the peer/client allocations' `issued_certificates` or
  compares their `spiffe_id` with `SpiffeId::for_allocation`;
- its Job phase checks terminal state, console outcomes and VMM absence, while
  its two Service stops accept either `Terminated` or `Failed`; it does not
  compare the complete running/terminal row universe; and
- `PacketEvidence::assert_tls13_no_cleartext` proves one kTLS inode/fd owner,
  TLS 1.3 software TX/RX, bidirectional application records and no cleartext,
  but observes no `splice(2)` ownership at all.

Other current tests do not close this exact mapping as prepared. The retained
non-mesh guest-stack body still calls `assert_exact_lifecycle_delta`, but it is
not this mesh Service/caller journey and is not in the proposed manifest.
`mtls_inbound_enforce` proves the inbound leg-C decrypt-splice boundary, not
that the current same-node mesh leg-F kTLS socket owns both splice directions.
Lower-level identity-manager tests prove identity derivation in isolation,
not the two allocations in the V-6 production composition.

This is an acceptance-evidence gap, not a production defect and not an API or
typed-error ambiguity. DISTILL must revise the exact existing-body mapping so
each retained guarantee has a current exact executable identity and safe
receipt, or surface the specific guarantee for an explicit contract decision.
This finding authorizes no new test, scenario, lane, seam, architecture or
restoration of the bridge/TAP bodies.

### F-02 — Evidence correction — two retained-ledger statements are not exact

**Kind:** `EVIDENCE_PRECISION`  
**Severity:** medium  
**Disposition:** OPEN; does not invalidate the hashed raw records

The raw files are intact, but `selection-exclusion-ledger.md` has two summary
errors:

1. the `fadba7c7f11b3c5f841961cc` row records source digest
   `a428484e88188b87386a7da61816088f75694dd5f77e110282a09af50ea8`;
   its hashed `metadata.v1` records the actual digest
   `a428484e88188b87386a7da61816088f75694dd5f77e110282a09af50fa35ea8`;
2. the E23 row says the "Current `CGROUP_WORKER` manifest does not select it."
   That was true for the retained original run's source, but the reviewed
   prepared source now selects it. The row must distinguish original-run
   source from current prepared, unexecuted support.

These are bounded documentation corrections. They do not require recollecting
or rerunning any userspace evidence.

## Acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 9/10 | The mapped inventory includes positive journeys, refusal paths, cleanup, E23 resource refusal and known negative baselines. |
| GWT / one behavior | 9/10 | Existing Rust bodies retain focused outcomes; support adds no body. |
| Domain language | 9/10 | VM Service, allocation identity, leg C, leg F, GuestStreamDial and DnsResponder remain precise. |
| Coverage completeness | 3/10 | F-01 leaves three retained frozen-body guarantees without a current exact mapped identity. |
| User/operator value | 8/10 | Current V-6 proves the principal user journey, but the frozen identity/lifecycle guarantees remain unproved. |
| Priority validation | 10/10 | The work addresses the three actual full-review blockers without adding adjacent machinery. |
| Observable behavior | 4/10 | Current TLS and cleanup observations are strong; the complete row and allocation-identity observables are missing from the substitution. |
| Traceability | 6/10 | Historical provenance is real and most current identities are exact; the one-body substitution overstates its assertion coverage. |
| Walking-skeleton boundary | 10/10 | V-6 runs through production `serve`/`deploy` inside nested production Cloud Hypervisor guests. |

The blocker and scores below 5 require `NEEDS_REVISION` even though the
GuestKit support source itself passes.

## Test-design mandates and Contract Shape

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | PASS for support | V-6 uses public `serve`/`deploy`; native adapter cells use existing production ports inside GuestKit. |
| CM-B — domain abstraction | PASS | The existing tests keep domain outcomes separate from runner mechanics. |
| CM-C — complete journey | **FAIL for the frozen mapping** | The V-6 journey is complete for V-6, but it does not carry every retained assertion of the two frozen bodies. |
| CM-D — fixture isolation | PASS | All real kernel and VM effects stay in disposable outer GuestKit guests. |
| CM-E — universe-bound assertion | **FAIL for the frozen mapping** | The frozen first body asserted the complete public allocation-row universe; the mapped V-6 body does not. Layer-4+ direct assertions remain otherwise permitted. |
| CM-F — layer-dependent PBT | PASS | All native cells are explicit examples; no PBT was added at a real-I/O layer. |
| CM-G — two-tier rich journey | N/A | No new rich journey or identity is introduced. |
| CM-H — explicit integration paths | **FAIL for the frozen mapping** | The same-socket production splice ownership asserted by the frozen wire body has no current mapped oracle. |
| Contract Shape — E23 | PASS | Exact `unbounded-preservation` declaration is present and unchanged. |
| Contract Shape — V-6 | PASS for its own contract | Exact `bounded-change` declaration is present; the failure is over-crediting it for additional frozen assertions. |

## Native-execution readiness

The E23 staging change itself is mechanically ready: exact identity, current
worker integration ELF, private cgroup-v2/pids substrate, production outer
VMM confinement, one-body count, marker-vacuity failure and cleanup ownership
all pass source review.

Overall STEP 14-04 native acceptance is **not yet authorized** because F-01
must be reconciled and independently re-reviewed first. Once that mapping is
approved, the original STEP 14-04 crafter may remove only the E23 marker and
perform exactly these current-source native gates:

1. `cargo xtask metal run -- cargo xtask guest-kit-prepare` to stage the
   current Cargo-reported ELFs and current kernel/module support;
2. the existing `cgroup-default` GuestKit group, requiring exactly 118 passes,
   zero failures, zero ignored required bodies, zero runtime-SKIP diagnostics,
   current source/ELF identity, terminal exit zero and complete outer-guest
   cleanup. Its required STEP 14-04 identities are E23, V-6, all seven real
   DNS bodies, all eleven exec/process netns-veth bodies and the existing
   constant-prefix leg-C body; and
3. the existing `mtls-relay` GuestKit group, requiring exactly 46 passes,
   zero failures, zero ignored required bodies, zero runtime-SKIP diagnostics,
   current source/ELF identity, terminal exit zero and complete outer-guest
   cleanup. Its required STEP 14-04 identities include the existing real
   mTLS/kTLS/splice leg-C body and Service dataplane-selection body.

The fifteen retained userspace/source records need no duplicate collection.
A busy Metal lease is ordinary serialization, and a lost client route requires
collection of the same run ID. No physical-host native fallback, direct native
nextest, host cleanup, host process kill, force unload or reboot is authorized.

After E23 activation, the two STEP 14-05 `vsock_density` markers remain for
that step's fresh crafter. The final feature-wide zero-pending check occurs
after STEP 14-05. `quint_cid_lease` `pending #325` remains the one approved
enduring issue-linked exception.

## Verdict

**NEEDS_REVISION.** The bounded E23 and GuestKit support is correct and adds
only one exact existing worker identity, producing coherent
56 → 118 cgroup-default accounting while leaving mtls-relay at 46. ELF
staging, private cgroup/pids setup, production Cloud Hypervisor confinement,
zero-match behavior and cleanup are suitable for later native execution.

Approval is blocked because the proposed mapping of both frozen guest-stack
bodies to the single V-6 identity proves the current dial-by-name and
TLS/no-cleartext journey but does not prove the frozen bodies' retained exact
per-allocation identities, complete allocation-row lifecycle delta or
same-socket splice ownership. Correct that existing-body/evidence mapping and
the two ledger statements, then return this package for independent re-review
before E23 marker activation or native execution.

## Iteration 2 re-review — bounded remediation

### Scope and evidence identity

Iteration 2 preserves every iteration-1 finding and disposition above. It
reviews only the bounded remediation that closes F-01 and F-02:

- the existing V-6 body now observes both Running allocations' exact public
  per-allocation identity and compares the caller's complete Running-to-
  Terminated allocation-row universe;
- the existing forwarded-outbound worker happy-path body now correlates its
  exact live kTLS socket with positive production splice calls in both
  directions;
- that existing worker identity is added once to the existing
  `CGROUP_WORKER` manifest, moving the cgroup-default baseline from 56 to 57
  and the canonical group from 118 to 119;
- the retained evidence-ledger digest and E23/source tense are corrected; and
- `strace` is staged through the existing GuestKit tool copier for the
  existing worker integration ELF.

No production source, public or cross-crate API, typed error, scenario
identity, Contract Shape declaration, marker, lane, executor, model, seam,
roadmap, DES event or deferral changed. The only visibility change is
`pub(super)` on an existing test-local lifecycle assertion helper so its
sibling V-6 test module can reuse the exact frozen oracle. It is not product
surface.

The final iteration-2 source gate `5a7dfdadd33367183c0f0a9e` ran
unprivileged on Metal with the task-owned
`target/distill-14-04-guest-support-r2` target and exited zero after
workspace format plus the scoped checks and `-D warnings` Clippy commands for
the CLI integration target, worker integration target, GuestKit runner and
xtask. This reviewer ran no test or native job. Local `git diff --check`
passes, and all 60 retained original-run files still match
`.context/netns-density-295/14-04/raw-sha256sums.txt`.

### F-01 remediation — closed

**Prior disposition:** blocker, OPEN  
**Iteration-2 disposition:** **CLOSED**

#### Exact peer and caller allocation identities

The existing V-6 identity remains
`integration::vsock_walking_skeleton::a_vm_workload_round_trips_tcp_and_udp_through_serve_and_deploy`.
It still drives the production `serve` and public `deploy` composition.

After each peer/client Service reaches its existing Running and console
outcome, `snapshot_with_exact_identity` reads the ordinary public describe
snapshot, finds the row for the already-observed allocation, parses its
public workload id, derives `SpiffeId::for_allocation(workload, alloc)` and
requires the current `issued_certificates` projection to contain exactly that
identity. The projection contract itself is one current certificate per
Running allocation, selected by maximum issuance ordinal; requiring one
summary in each one-allocation Service snapshot is therefore the existing
public contract, not a new no-rotation or audit-history invariant.

The peer assertion occurs before its VIP, workload address and listener are
credited. The client assertion occurs only after the gateway DNS, unique mesh
name and reply-dependent `vip-tcp ok` outcomes. Both identities are therefore
bound to the exact allocations that participate in the mapped journey. No
identity fixture, accessor or production hook is added.

#### Complete caller lifecycle-row universe

V-6 captures the client Service's public Running `AllocStatusRowBody`, stops
the client through the existing public operator verb, requires that exact
allocation to reach `Terminated`, and passes the Running and terminal rows to
the historical `assert_exact_lifecycle_delta` oracle. The helper still starts
from the complete row value and normalizes only the same seven permitted
lifecycle/ephemeral fields: state, reason, workload address, started-at, exit
code, last transition and terminal payload. Every other public row field must
remain equal.

The helper's implementation is unchanged except for sibling-module
visibility and correcting its diagnostics from “Job”/“six” to
“VM allocation”/“seven.” Its semantic assertions are not weakened. Requiring
both stopped Services to be `Terminated` also removes the iteration-1
`Terminated | Failed` allowance. The frozen first body applied the complete
delta to the caller allocation, not the peer Service; the current mapping now
does the same at the replacement journey boundary.

#### Exact live kTLS fd owns both splice directions

The same-socket mechanism remains in the existing worker identity
`integration::vsock_mesh_egress_mtls::registered_mesh_flow_reaches_its_registered_backend_over_mtls_without_leg_f_resolving_again`.
The body retains its `CONTRACT_SHAPE: unbounded-preservation.` declaration,
registered-backend assertion, allocation SVID handshake, TLS 1.3 wire record,
no-cleartext assertion and no-second-resolution assertion.

The new observation closes the exact historical guarantee without a product
hook:

1. a bounded external `strace -f -e trace=splice` observer attaches before
   the one registered flow connects to leg F;
2. after the byte-exact request/reply, `ss -e -i` must expose exactly one
   established socket to the registered backend with `tcp-ulp-tls`, TLS 1.3,
   and software TX/RX;
3. that socket's inode must resolve through `/proc/self/fd` to exactly one fd
   in the process that owns the production pumps; and
4. the trace must contain positive-return `splice(2)` calls with that exact fd
   as a source and as a destination.

The forwarder stream and kTLS connection remain live until after the socket
observation and trace detachment, so the fd cannot be closed and reused during
the correlation. The rig creates one registered happy-path flow, and the
backend tuple plus sole inode owner rejects another socket or flow. Empty,
failed-to-attach, non-positive or one-direction-only traces fail the body.
The observer reads public kernel/process surfaces only and is private test
support. It adds no owner, adapter, port, production accessor or scenario.

Together, the strengthened V-6 and existing worker identity now prove all
three retained attachment-neutral guarantees from iteration 1. The obsolete
bridge, TAP, TAP queue-holder, per-TAP intercept and virtio-net-header
assertions remain correctly superseded by ADR-0145 and are not restored.

### F-02 remediation — closed

**Prior disposition:** medium, OPEN  
**Iteration-2 disposition:** **CLOSED**

The retained ledger now records the full actual source digest
`a428484e88188b87386a7da61816088f75694dd5f77e110282a09af50fa35ea8`
for run `fadba7c7f11b3c5f841961cc`. Its E23 row now distinguishes the
original retained run source, which did not select E23, from the current
prepared source, which does. The frozen guest-stack row likewise says the
current mapping is prepared but natively unexecuted. No raw receipt was
rewritten and no userspace rerun is needed.

### GuestKit staging, accounting and vacuity

The existing `overdrive-worker--integration` ELF already contains E23 and the
forwarded-outbound body. `guest-kit-prepare` continues to select Cargo's
actual test executable from compiler-artifact JSON and copy that ELF to the
existing in-guest path. `strace` is added through the established `tool`
staging helper, including its ordinary shared libraries. The worker's
existing `stage_step13_04_nft` path already supplies `ss`, nft/netns support
and the real leg-F substrate; no new group or lane exists.

Static manifest reconciliation gives:

| Segment | Exact bodies |
| --- | ---: |
| `CGROUP_ACCEPTANCE` | 3 |
| `CGROUP_CONTROL_PLANE` | 45 |
| `CGROUP_WORKER` | 9 |
| **Retained cgroup baseline** | **57** |
| STEP 13-04 | 18 |
| STEP 13-05 | 5 |
| Control-plane normal-native suffix | 20 |
| STEP 13-06 | 13 |
| **Complete pre-STEP-14 boundary** | **113** |
| Existing STEP 14-01 CLI + control-plane bodies | 6 |
| **Canonical cgroup-default result** | **119** |

Every unchanged segment retains its prior count. The E23 and forwarded-
outbound exact filters occur once. Across every relevant cgroup-default and
mtls-relay array there are zero duplicate filters and zero prefix-overlap
pairs. `mtls-relay` remains 7 + 36 + 3 = 46.

Exact libtest invocation, exit zero and the exact pass summary reject zero
matches and the still-ignored E23 body. The two new baseline bodies have no
runtime-skip branch: E23 uses `expect` for every required substrate operation,
and the forwarded-outbound body asserts root/capability and panics on every
fixture or observer failure. Required suffix, nested and mtls-relay cells also
use the existing runtime-SKIP diagnostic rejection. The later native evidence
must show zero runtime-SKIP diagnostics over the complete group output.

The outer boundary remains production `CloudHypervisorVmm::create` with
`VmConfinement::confined`, uid/gid 4200, the actual supplementary groups of
`/dev/kvm` and `/dev/vhost-vsock`, and the production Landlock/seccomp launch
path. E23 and the forwarded-outbound body execute in the existing
`cgroup-worker-integration` outer guest; the V-6 body alone launches nested
production workload VMs from its existing `step14-01-cli` outer guest.
Private cgroup, pids, BPF, nft, netns, route, kTLS, ptrace and cleanup effects
remain inside those disposable guests. `GuestRun::finish` retains terminal
outer-VMM cleanup. No physical-host native fallback exists.

### Iteration-2 review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 9/10 | Existing refusal, cleanup and negative suites remain; this remediation closes the exact positive frozen outcomes. |
| GWT / one behavior | 9/10 | Both existing identities keep their established journey/mechanism behavior; no body is added. |
| Domain language | 10/10 | Peer Service, caller allocation, leg F, registered backend, kTLS socket and splice directions remain distinct. |
| Coverage completeness | 10/10 | Every retained attachment-neutral frozen guarantee now has an exact executable current identity. |
| User/operator value | 9/10 | The public journey carries exact allocation identity and lifecycle outcomes; the mechanism stays at its adapter boundary. |
| Priority validation | 10/10 | The changes close only F-01/F-02 and the required runner fallout. |
| Observable behavior | 10/10 | Public describe rows plus `ss`, `/proc/self/fd` and positive syscall records are closed ordinary observations. |
| Traceability | 10/10 | Each historical assertion maps to one named current identity and the exact 119/46 native gates. |
| Walking-skeleton boundary | 10/10 | V-6 remains production `serve`/`deploy`; the splice mechanism remains in the real worker adapter test. |

### Iteration-2 test-design mandates and Contract Shape

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | PASS | V-6 uses public `serve`/`deploy`; the worker body uses the existing real registration/intercept/enforcement composition. |
| CM-B — domain abstraction | PASS | Runner and syscall mechanics remain private support beneath existing domain-named bodies. |
| CM-C — complete journey | PASS | Name/reply, both identities, caller lifecycle, TLS/no-cleartext and cleanup are all retained. |
| CM-D — fixture isolation | PASS | Every real kernel/VM effect stays in disposable GuestKit guests. |
| CM-E — universe-bound assertion | PASS | The caller uses the complete historical public-row universe; layer-4+ mechanism observations remain direct. |
| CM-F — layer-dependent PBT | PASS | Existing real-I/O examples remain example-based. |
| CM-G — two-tier rich journey | N/A | No new journey or generative tier is introduced. |
| CM-H — explicit integration paths | PASS | The exact live kTLS fd is positively observed in both splice directions. |
| Contract Shape — V-6 | PASS | Existing `bounded-change` declaration retained. |
| Contract Shape — forwarded outbound | PASS | Existing `unbounded-preservation` declaration retained. |
| Contract Shape — E23 | PASS | Existing `unbounded-preservation` declaration and marker retained. |

### Native-execution authorization and required receipts

The source and acceptance mapping are now **APPROVED FOR NATIVE EXECUTION BY
THE ORIGINAL STEP 14-04 CRAFTER**. This is not native pass credit. The original
crafter may remove only the E23 marker, preserve the two STEP 14-05 markers
and the `pending #325` exception, then collect current-source evidence through
the existing boundaries:

1. `cargo xtask metal run -- cargo xtask guest-kit-prepare`, retaining the
   exact source digest and every staged ELF identity;
2. `OVERDRIVE_FWD_GUEST_GROUP=cgroup-default`, requiring exactly 119 passed,
   zero failed, zero ignored required bodies, zero runtime-SKIP diagnostics,
   terminal exit zero and complete outer-guest cleanup. The STEP 14-04
   identities requiring current-source credit are:
   - V-6
     `integration::vsock_walking_skeleton::a_vm_workload_round_trips_tcp_and_udp_through_serve_and_deploy`;
   - forwarded outbound
     `integration::vsock_mesh_egress_mtls::registered_mesh_flow_reaches_its_registered_backend_over_mtls_without_leg_f_resolving_again`;
   - E23
     `integration::real_cgroup_fs::refused_thread_kill::a_kill_write_under_thread_refusal_returns_an_io_error_and_never_aborts`;
   - all seven `dns_responder_bind` bodies;
   - both `serve_boot_provisions_veth`, all seven
     `veth_provision_idempotent` and both `workload_netns_provision` bodies;
     and
   - the existing constant-prefix leg-C body
     `integration::vsock_constant_guest_prefix_rules::a_leg_c_diverted_remote_connection_reaches_leg_c_both_ways`;
3. `OVERDRIVE_FWD_GUEST_GROUP=mtls-relay`, requiring exactly 46 passed, zero
   failed, zero ignored required bodies, zero runtime-SKIP diagnostics,
   terminal exit zero and complete outer-guest cleanup. The mapped required
   identities include
   `integration::mtls_inbound_enforce::inbound_enforce_origdst_server_mtls_ktls_rx_splice_to_server`
   and
   `integration::service_map_forward::ten_tcp_syns_to_vip_are_rewritten_and_forwarded_via_veth`.

The fifteen retained userspace/source records need no duplicate collection.
The old V-6 receipt is not current-source credit because STEP 14-04 changed
HTTP response disposal and iteration 2 strengthened V-6. A busy Metal lease
is ordinary serialization; after a lost client route, collect the same run
ID. No direct native nextest, physical-host kernel effect, cleanup fallback,
host process kill, force unload or reboot is authorized.

After E23 activation, the two STEP 14-05 markers remain for that step's fresh
crafter. The feature-wide final pending-marker check remains sequenced after
14-05; `pending #325` is the one approved enduring issue-linked exception.

### Iteration-2 verdict

**APPROVED FOR ORIGINAL-CRAFTER NATIVE EXECUTION.** F-01 is closed by exact
existing identities at their proper current boundaries: V-6 now proves both
allocation identities and the caller's complete lifecycle-row delta, while
the existing forwarded-outbound worker body proves the exact live kTLS fd is
both a positive splice source and destination. F-02's evidence statements are
corrected. No frozen guarantee is weakened, no obsolete bridge/TAP body is
restored, and no public surface, scenario, lane or production hook is added.

GuestKit staging and count evidence are coherent at cgroup-default 119 and
mtls-relay 46. Native pass credit remains outstanding until the original
crafter activates E23 and produces the exact current-source receipts above.

## Iteration 3 re-review — native-RED fixture correction

### Scope and retained evidence

Iteration 3 preserves both earlier reviews, findings and dispositions. It
reviews only the private fixture correction prepared after native run
`2572dafb29ff625bdddc3bbb`. The original crafter's separate E23 production
write correction, active E23 body and DES events are outside this DISTILL
fixture review; none is accepted or modified here.

The native record is complete and internally consistent:

- cgroup-default ran inside the existing disposable GuestKit boundary at
  source digest
  `5adb0b0df0ef6d36337300820945c97262644fd579dccd556c0453945939509f`;
- it advanced past the corrected active E23 body and reached the exact mapped
  forwarded-outbound worker identity;
- that identity exited 101, so the run receives no 119-body pass credit; and
- the raw output, terminal record and byte-preserving output hash
  `19f1c393798e200d0a1138526b7f164d8638f0dd914f23ec6128357fd655f0e8`
  are retained. The complete receipt set still passes the recorded SHA-256
  manifest.

The final iteration-3 source gate `d045bb4c47f30618da6d9085` ran
unprivileged with the task-owned iteration-3 target and passed format, the
worker integration check and scoped Clippy with `-D warnings`. It ran no
test, kernel or VM workload. This reviewer likewise ran no test or native
job; local `git diff --check` passes.

### Native failure causality

The native output contains two independent fixture findings.

First, the peer-session thread panics at the test fixture's
`watch_termination` raw `std::net::TcpStream::read` with
`Interrupted system call (os error 4)`. The stack and source path stay inside
`vsock_mesh_egress_mtls.rs`: peer acceptor thread → `run_peer_session` →
`watch_termination` → `RecordingTcp::read`. The production owner and
production splice pumps do not appear in that failure path.

The observer attached to every test-process thread before the flow and was
detached after the first reply and kTLS observation. At that point the peer
fixture had completed its response and was blocked in the raw termination
read. Terminating the owned strace child detached ptrace from the fixture
thread and interrupted that blocking syscall. This accounts for the exact
EINTR without inventing a production schedule. The production
`blocking_splice` path is separate and already retries only `EINTR` before
reissuing the same call.

Second, the native assertion reports the exact live TLS 1.3 software-TX/RX
kTLS socket as fd 15, with positive splice sets
`source_fds={15,19}` and `destination_fds={14,20}`. The old one-phase
stimulus supplied no post-establishment forward request. Source inspection of
`mtls/outbound.rs:121-147` shows why: bytes captured during establishment are
placed in the in-memory forward `prelude`, which the production pump writes
with `write_all` because those bytes have no source fd. The first response did
exercise the steady-state return splice, so fd 15 honestly appeared only as a
positive source. Repeating the one-phase stimulus could not prove the missing
destination direction.

The observed native trace and production source therefore support the
prepared fixture correction. They do not establish a production mTLS defect
or authorize a production change.

### Byte-distinct steady-state exchange

The existing body and identity remain
`integration::vsock_mesh_egress_mtls::registered_mesh_flow_reaches_its_registered_backend_over_mtls_without_leg_f_resolving_again`, with its existing
`CONTRACT_SHAPE: unbounded-preservation.` declaration.

Its original byte-distinct request/reply still establishes the connection and
retains the registered-backend, client-SVID, TLS 1.3, no-cleartext and
no-second-resolution assertions. Only that happy-path fixture selects the new
private `PeerMode::TwoPhase` mode. After the first response is read, the body
sends a different `REQUEST2` and requires the exact different `RESPONSE2`.
The peer independently records and requires the full second request before it
writes the second response, and the body does not inspect syscall evidence
until it has read that response byte-exact.

The second request therefore begins after establishment and after the
pre-arm buffer has already been consumed. It must traverse the production
forward steady-state splice into the live kTLS fd. The second response must
traverse the production return steady-state splice out of that same fd. The
existing external oracle remains unchanged: `ss` selects one live TLS 1.3
software-TX/RX socket to the exact backend, `/proc/self/fd` requires one
process fd for its inode, and positive-return strace records must contain that
exact fd as both source and destination. No empty, prelude-only,
one-direction-only or different-fd trace can pass.

The other existing worker scenarios retain their prior single-exchange
behavior through the mechanically renamed `Finish` and `HoldOpen` modes. No
new body, scenario id, selector, port, owner or observation hook is added.

### EINTR handling and deadline

The correction is confined to the fixture's post-exchange raw termination
read. `watch_termination` creates its existing absolute 60-second `give_up`
once, before the loop. `ErrorKind::Interrupted` now continues that same loop;
it does not reset or extend the deadline. A `Read` result of EINTR carries no
completed byte count, so retrying it cannot duplicate or discard fixture
payload.

Every other outcome retains its prior meaning:

- raw EOF enters the existing FIN path;
- timeout/WouldBlock remains the existing bounded polling result;
- reset/abort/broken-pipe remains `Termination::Reset`;
- every other error still panics immediately; and
- the stop flag and absolute deadline still terminate the loop.

The constants remain unchanged: round trip 15 seconds, connection/closure 5
seconds and peer observation 10 seconds; the raw termination deadline remains
60 seconds. Reusing the existing 15-second round-trip bound for a necessary
second phase adds a stimulus, not a looser timeout or retry policy.

`read_tls_record` retains the prior fail-closed result shape: an incomplete
record after EOF or error cannot equal its byte-distinct expected request, so
the peer records `Termination::Failed` and the test cannot satisfy its exact
session predicate. The EINTR exception is not applied to handshake, TLS
record, owner or production I/O.

### Lifecycle, cleanup and scope

The original body still drops the live forwarder stream and registration,
then invokes the existing owner shutdown and joins the peer fixture. The
strace guard still terminates and reaps its owned child and removes its output
file on every unwind path. `KernelStateLock`, `SharedInfraCleanup`, CID lease,
worker and peer ownership are unchanged. The new EINTR retry lets the fixture
continue to its pre-existing bounded termination/cleanup path; it does not
hide cleanup failure.

Iteration 3 changes only the existing worker test fixture. The dirty
`CgroupFs` production adapter, E23 body and execution log belong to the
original crafter's separate E23 correction and are neither changed nor
reviewed here. There is no iteration-3 production behavior, public API,
typed error, timeout, test identity, GuestKit lane, manifest count or cleanup
change.

### Iteration-3 disposition

| Check | Result |
| --- | --- |
| Native stack/source causality | PASS — failure is the peer fixture raw read during observer detach |
| Production premise | PASS — production splice already handles EINTR separately; no production mTLS defect inferred |
| Second stimulus | PASS — byte-distinct request/reply occurs only after establishment |
| Exact-fd forward evidence | PASS in source — second request forces positive splice into the selected live kTLS fd |
| Exact-fd return evidence | PASS in source — second response forces positive splice out of the same fd |
| Oracle strength | PASS — same sole-inode fd must remain in both positive-return sets |
| EINTR handling | PASS — only `ErrorKind::Interrupted` retries, with no progress and no payload credit |
| Deadline | PASS — the absolute 60-second termination deadline and all constants are unchanged |
| Other errors | PASS — EOF/reset/timeout semantics retained; unexpected errors still panic |
| Contract Shape | PASS — existing `unbounded-preservation` declaration retained |
| Scenario/API/lane | PASS — none added or changed |
| Lifecycle/cleanup | PASS — existing stream, registration, owner, peer, observer and GuestKit cleanup retained |
| Source gate | PASS — `d045bb4c47f30618da6d9085`, unprivileged, no tests/native effects |

The iteration-2 frozen-outcome mapping, V-6 identity/lifecycle assertions,
cgroup-default 119 accounting, mtls-relay 46 accounting, staged-ELF boundary,
runtime-skip policy and production Cloud Hypervisor confinement remain
unchanged and approved.

### Current native readiness

**APPROVED FOR ONE CURRENT CGROUP-DEFAULT119 GUESTKIT VALIDATION BY THE
ORIGINAL STEP 14-04 CRAFTER.** The crafter must first run the existing
`guest-kit-prepare` boundary so the current corrected
`overdrive-worker--integration` ELF and its source identity are staged, then
run exactly the existing `OVERDRIVE_FWD_GUEST_GROUP=cgroup-default` group.

Acceptance requires exactly 119 passed, zero failed, zero ignored required
bodies, zero runtime-SKIP diagnostics, terminal exit zero, exact current
source/ELF identity and complete outer-guest cleanup. The corrected
forwarded-outbound identity must print positive source and destination sets
containing the same selected live kTLS fd, and the V-6 and active E23 bodies
must pass in the same current-source group.

The retained `mtls-relay` run `1e8aba12ce7f53f7be0560ff` already establishes
the unchanged exact-46 group and is not rerun for this worker-fixture-only
correction. The failed cgroup-default receipt remains native RED and receives
no partial canonical credit. No direct native nextest, physical-host kernel
effect, fallback cleanup, host process kill, force unload or reboot is
authorized.

### Iteration-3 verdict

**APPROVED.** The captured EINTR is causally confined to the private peer
fixture during strace detach. The one-phase stimulus was insufficient because
the forward request could ride the production prelude; the second
byte-distinct exchange now forces both steady-state directions while the
same-fd positive splice oracle remains exact. EINTR retry is no-progress,
fixture-only and bounded by the unchanged deadline. All other errors,
lifecycle and cleanup remain fail-closed.

The package is ready for one current cgroup-default119 GuestKit validation by
the original crafter after current-ELF preparation. Native pass credit remains
outstanding until that exact receipt is collected.

## Iteration 4 re-review — V-6 lifecycle-helper correction

### Scope and native evidence

Iteration 4 preserves every earlier finding and disposition. It reviews the
private lifecycle-helper correction prepared after current GuestKit run
`5e5f472e4191780088cc1a96`; it does not review or change the original
crafter's E23 production correction, DES entries or other source.

The native receipt is honest and useful. Cgroup-default completed all 113
pre-STEP-14 bodies, including active E23 and the corrected same-fd splice
identity, then entered the existing STEP 14-01 CLI V-6 body. The two Services
were intentionally stopped through `lane.stop`; both VMM processes were
reaped with status zero, but no guest application EXIT record existed. The
body failed at the reused natural-completion helper because the public row
correctly carried `exit_code=None` while the helper required `Some(0)`.
That body was 114 of the canonical 119, so the run remains native RED with no
partial canonical credit. The later teardown timeout followed the assertion
unwind and is secondary to the first lifecycle-oracle failure.

The prepared correction correctly preserves `exit_code=None` for an
operator-stopped Service and correctly keeps the natural Job helper strict at
`Some(0)`. The final unprivileged source gate
`0d2406e019828b0c208d2c0d` passed format, the CLI integration check and
scoped Clippy with `integration-tests,kvm-tests` and `-D warnings`. It ran no
test or native workload. This reviewer likewise ran no test or native job;
local `git diff --check` and the retained raw-receipt hash manifest pass.

### F-03 — blocker — prepared operator-stop helper contradicts the accepted typed representation

**Kind:** `ACCEPTANCE_ORACLE`  
**Severity:** blocker  
**Disposition:** **OPEN**

The new private helper requires all three stop-attribution surfaces to say
`StoppedBy::Operator`:

- `terminal.reason`;
- `terminal.last_transition.reason`; and
- `terminal.terminal`.

That is not the accepted current representation. The production source pins
the distinction explicitly:

1. `Action::StopAllocation` treats the typed `terminal` claim as the source
   of truth for who initiated the stop (`Operator` versus `Reconciler`). Its
   compatibility comment states that the row `reason` and wire-side
   `last_transition.reason` remain legacy
   `Stopped { by: Reconciler }`, while operator attribution lands exclusively
   on `terminal` (`action_shim/mod.rs:3290-3297`).
2. The row write implements that contract: `reason` is
   `TransitionReason::Stopped { by: StoppedBy::Reconciler }`, and the passed
   terminal claim is preserved separately (`action_shim/mod.rs:3317-3341`).
3. `AllocStatusRowBody::from` copies the row's reason directly into
   `last_transition.reason` (`handlers.rs:100-112`), so that transition also
   carries `Reconciler`, not `Operator`.
4. The terminal-fence characterization seeds the same exact approved pair:
   row reason `StoppedBy::Reconciler` and terminal condition
   `StoppedBy::Operator` (`worker/exit_observer.rs:760-769`).
5. The public handler projects `exit_code` only for typed `Completed` and
   `Failed` terminals; `Stopped` therefore correctly projects `None`
   (`handlers.rs:122-136`).

The exit observer can transiently classify an intentional process exit with
an operator reason, but it emits no terminal claim. `StopAllocation` then
converges the authoritative terminal row and deliberately preserves the
compatibility reason described above. The prepared helper must validate the
settled public row produced by that owner path, not a transient competing
writer's shape.

As written, the next native run can pass the corrected `exit_code=None`
assertion only to fail deterministically at `terminal.reason`: actual
`StoppedBy::Reconciler`, expected `StoppedBy::Operator`. The matching
last-transition assertion is wrong for the same source-level reason. A green
compile/Clippy gate cannot establish this runtime semantic equality.

The bounded correction is private and requires no DESIGN or production
change: retain the complete-row complement, `Terminated`,
`exit_code=None`, and
`TerminalCondition::Stopped { by: StoppedBy::Operator }`; require both the row
reason and `last_transition.reason` to equal
`TransitionReason::Stopped { by: StoppedBy::Reconciler }`. Keep the natural
Job `Some(0)` oracle unchanged. Then return the same helper and V-6 call site
for source re-review before another native run.

### What remains approved

The helper refactor itself is otherwise sound:

- `assert_exact_lifecycle_delta` still calls the complete shared complement
  and separately requires natural Job `Some(0)`;
- the shared complement still permits changes only to state, reason,
  workload address, started-at, exit code, last transition and terminal
  payload, comparing every other public row field exactly;
- V-6 still invokes the Service-specific helper only for its existing caller
  Service allocation and still requires both Services to reach
  `Terminated`;
- the exact peer/client allocation identities are unchanged;
- the iteration-3 two-phase, same-live-fd positive splice oracle is unchanged;
- all timeouts, retry behavior, scenario identities, GuestKit manifests and
  119/46 accounting are unchanged; and
- no public API, wire type, state meaning, production hook, lane or seam is
  added.

The native record also strengthens the retained evidence chain: active E23
and the forwarded-outbound splice body advanced, and V-6 reached its stop
oracle after the existing journey. That progress does not waive the incorrect
typed-reason expectation or establish 119/119.

### Iteration-4 disposition

| Check | Result |
| --- | --- |
| Native first failure | PASS — natural Job `Some(0)` oracle was incorrectly reused for operator-stopped Service `None` |
| Natural Job oracle | PASS — strict `Terminated` / `Some(0)` remains separate and unchanged |
| Complete row universe | PASS — unchanged seven-field permitted delta with exact complement |
| Service state | PASS — `Terminated` required |
| Service exit code | PASS — `None` matches the approved stopped-terminal projection |
| Service terminal condition | PASS — typed `StoppedBy::Operator` is correct |
| Service row reason | **FAIL** — prepared helper expects `Operator`; accepted settled row is `Reconciler` |
| Last-transition reason | **FAIL** — it copies the settled row reason and is also `Reconciler` |
| Allocation identities | PASS — unchanged exact `SpiffeId::for_allocation` assertions |
| Same-fd splice | PASS — unchanged approved two-phase exact-fd oracle |
| Timeout/retry/cleanup | PASS — unchanged |
| Public API/scenario/lane | PASS — none added |
| Source gate | PASS for compilation — `0d2406e019828b0c208d2c0d`; semantic blocker remains |

### Current native readiness

**NOT READY FOR NATIVE RERUN.** The prepared private helper is guaranteed by
current production source to reject the accepted settled operator-stop row.
Correct the two reason expectations to `StoppedBy::Reconciler`, retain the
operator terminal condition and `exit_code=None`, run the bounded source gate,
and return for independent source re-review.

After that correction is approved, the intended execution remains one current
`guest-kit-prepare` followed by one existing
`OVERDRIVE_FWD_GUEST_GROUP=cgroup-default` validation requiring exact 119/119,
zero failures, zero ignored required bodies, zero runtime-SKIP diagnostics,
terminal exit zero, current source/ELF identity and complete outer-guest
cleanup. The retained exact-46 mtls-relay receipt remains source-qualified
and valid; no mtls-relay rerun is required for this CLI-test-helper-only
correction.

### Iteration-4 verdict

**NEEDS_REVISION.** Splitting natural Job completion from operator-stopped
Service lifecycle is the correct remedy, and `exit_code=None` is exact. The
prepared helper nevertheless assigns operator attribution to the row reason
and last-transition reason, contradicting the action-shim's explicit
compatibility contract and terminal-fence characterization. Only the terminal
condition is operator-attributed. Correct those two private expectations
before authorizing the next cgroup-default119 GuestKit validation.

## Iteration 5 re-review — settled typed-field correction

### F-03 remediation — closed

**Prior disposition:** blocker, OPEN  
**Iteration-5 disposition:** **CLOSED**

The existing private
`assert_exact_operator_stopped_service_lifecycle_delta` helper now matches the
settled owner and public-projection paths exactly:

| Public field | Required value | Contract source |
| --- | --- | --- |
| `state` | `Terminated` | Shared complete lifecycle complement |
| `reason` | `TransitionReason::Stopped { by: StoppedBy::Reconciler }` | `StopAllocation` compatibility reason |
| `last_transition.reason` | The same `StoppedBy::Reconciler` reason | Mechanical copy of the settled row reason |
| `terminal` | `TerminalCondition::Stopped { by: StoppedBy::Operator }` | Authoritative typed stop initiator |
| `exit_code` | `None` | Public projection reserves an exit code for `Completed` / `Failed` terminals |

This is the representation pinned by the action-shim write, handler
projection and terminal-fence characterization cited in iteration 4. The
helper uses one `settled_reason` value for both the row and last-transition
checks, so those two surfaces cannot drift inside the test. Operator
attribution remains exclusively on the terminal condition.

The complete row oracle remains closed. Both lifecycle helpers call the same
private `assert_exact_lifecycle_complement`, which normalizes only state,
reason, workload address, started-at, exit code, last transition and terminal
payload before comparing the entire remaining `AllocStatusRowBody`. The
operator-stop helper then pins every normalized typed field listed above.

The historical natural Job oracle is not weakened: its existing
`assert_exact_lifecycle_delta` still requires the same complete complement
and `exit_code == Some(0)`. It is not redirected through the Service helper.
V-6 imports the Service-specific helper and invokes it only for the existing
caller Service allocation after public operator stop; both Service
allocations must still reach `Terminated`, and both VMM-absence checks remain.

### Retained guarantees and scope

Iteration 5 changes no production code, public API, wire type, state meaning,
scenario identity, Contract Shape declaration, timeout, retry, cleanup path,
GuestKit manifest, lane, runner or count. The exact peer/client
`SpiffeId::for_allocation` assertions and the approved two-phase same-live-fd
splice oracle are unchanged from their prior reviewed scope. The native
failure and teardown evidence remain recorded without waiver or partial
canonical credit.

The final unprivileged source gate `4f8f454c5e786fa142bfd0b5` reused the
task-owned iteration-4 target and passed format, the CLI integration check and
scoped Clippy with `integration-tests,kvm-tests` and `-D warnings`. It ran no
test or native workload. This reviewer likewise ran no test or native job;
local `git diff --check` and the retained raw-receipt hash manifest pass.

### Iteration-5 disposition

| Check | Result |
| --- | --- |
| Complete public-row complement | PASS — unchanged seven-field permitted delta, exact remainder |
| Natural Job completion | PASS — strict `Some(0)` retained |
| Service state | PASS — `Terminated` |
| Settled row reason | PASS — `StoppedBy::Reconciler` |
| Last-transition reason | PASS — same settled `StoppedBy::Reconciler` reason |
| Terminal condition | PASS — `StoppedBy::Operator` |
| Service exit code | PASS — `None` |
| V-6 call-site scope | PASS — existing caller Service only |
| Allocation identities | PASS — unchanged exact public identity checks |
| Same-fd splice | PASS — unchanged approved two-phase exact-fd oracle |
| Timeout/retry/cleanup | PASS — unchanged |
| API/scenario/lane/count | PASS — none added or changed; cgroup-default remains 119, mtls-relay 46 |
| Source gate | PASS — `4f8f454c5e786fa142bfd0b5`, unprivileged, no tests/native effects |

### Current native readiness

**APPROVED FOR ONE CURRENT CGROUP-DEFAULT119 GUESTKIT VALIDATION BY THE
ORIGINAL STEP 14-04 CRAFTER.** The crafter must first use the existing
`guest-kit-prepare` boundary to stage the current CLI and worker integration
ELFs with their exact source identity, then run exactly the existing
`OVERDRIVE_FWD_GUEST_GROUP=cgroup-default` group.

Acceptance requires exactly 119 passed, zero failed, zero ignored required
bodies, zero runtime-SKIP diagnostics, terminal exit zero, exact current
source/ELF identity and complete outer-guest cleanup. The active E23 body,
same-fd forwarded-outbound body and strengthened V-6 body must all pass in
that one current-source group. The two STEP 14-05 markers and `pending #325`
remain outside this run's activation scope.

The retained exact-46 mtls-relay receipt remains source-qualified and valid;
this private CLI helper correction changes none of its source, staged ELFs,
manifest or assertions. No mtls-relay rerun, direct native nextest,
physical-host kernel effect, fallback cleanup, host process kill, force unload
or reboot is authorized.

### Iteration-5 verdict

**APPROVED.** F-03 is closed. The private Service helper now distinguishes
the settled compatibility reason (`Reconciler`) from the authoritative stop
initiator (`Operator`) and correctly requires no application exit code. The
complete row complement and strict natural Job `Some(0)` oracle remain intact,
as do the allocation-identity and same-fd splice guarantees.

The package is ready for one current cgroup-default119 GuestKit validation by
the original crafter after current-ELF preparation. Native pass credit remains
outstanding until that exact receipt is collected.

## Iteration 6 re-review — failed relay-fixture cleanup

### Corrected native evidence history

Run `129e9c3d78f087fc13e8868e` does not credit the third STEP 14-01
control-plane identity. The exact `test result: ok. 1 passed` summary visible
at raw output line 5634 belongs to the preceding V-6 CLI body. The runner then
issued the exact command for
`integration::guest_flow_through_serve::the_forwarder_inventory_counts_probe_and_relay_watches_through_serve`.
That body started, but it emitted no libtest result, its direct child never
exited, and the guest peer therefore sent no EXEC reply. The GuestKit runner
timed out that still-live command. The ledger's corrected attribution is
exact: V-6 is green in that run; this control-plane body is neither passed nor
completed; canonical 119/119 is not established.

The first semantic failure occurred inside the started body, before the
600-second support timeout:

- the Service listener and earlier readiness probes were live;
- readiness flow 2147483655 ended `Unreachable { errno: 104 }`;
- the production service lifecycle and service-map hydrator consequently
  exposed no healthy backend;
- the one-shot client followed the ordinary owner/resolve path and received
  the typed `mesh_unreachable` refusal; and
- its guest application reported a terminal reset before `FLOWING_LINE`.

That remains a failed accepted relay outcome. It is not converted into a
pass, retried, ignored or attributed to RPC framing. The later EXEC timeout is
fixture cleanup fallout: the panic dropped `ServerHandle`, whose Drop does not
await server shutdown, leaving blocking owners and the two workload VMMs
without stop/reap receipts.

### What the prepared correction gets right

`wait_flowing_or_terminal` preserves the success predicate: only the existing
`FLOWING_LINE` returns `Ok(())`. A terminal client observation or the unchanged
boot deadline returns `Err` with the client console and all retained
allocation-console diagnostics. The readiness reset and mesh-unreachable
client refusal therefore remain failure evidence; neither can satisfy the
body's relay-pair assertions.

On that error path, the body now attempts the established lifecycle effects
in the correct order:

1. public operator stop of the client workload;
2. public operator stop of the Service workload;
3. observation of an empty CID map;
4. exact forwarder inventory `(0, 0)` on that same observation;
5. awaited graceful `ServerHandle::shutdown`; and
6. re-report of the saved primary failure.

The CID and inventory complement is exact if reached: the wait returns only a
snapshot with `cids.is_empty()`, and the immediate assertion requires both
live-pair and live-watch counts to be zero. The normal success branch and all
its relay/quota/watch assertions are unchanged. No retry, timeout, production
hook, public API, scenario identity, GuestKit lane or production recovery
mechanism is added.

### F-04 — blocker — cleanup failure can still erase the primary semantic failure

**Kind:** `FAILURE_EVIDENCE_PRESERVATION`  
**Severity:** blocker  
**Disposition:** **OPEN**

The correction preserves `primary_failure` only if every cleanup operation
succeeds. Each cleanup step can panic before the final
`panic!("{primary_failure}")`:

- `Node::stop` unwraps request transport and asserts the HTTP status;
- `wait_observation` asserts on its bounded timeout and `observe` expects the
  owner observation;
- the inventory complement is an ordinary `assert_eq!`; and
- `Node::shutdown` expects graceful shutdown to succeed.

Any stop, observation, inventory or shutdown failure therefore unwinds
immediately with only the cleanup panic. The saved readiness/client-flow
failure is displaced, contrary to the requested guarantee that the original
semantic error remain present even if cleanup itself fails. This matters on
the exact path under review: cleanup is being added because a prior failure
left owners and VMMs live, so cleanup failure is a reachable secondary
outcome, not a hypothetical unrelated fault.

The bounded remediation remains private test support. Preserve the same
cleanup order and exact zero-CID/inventory requirements, but collect any
cleanup failure so the final failure report always contains the original
`primary_failure` and, when present, the cleanup failure as additional
context. Do not turn cleanup failure into success, suppress the original
readiness/mesh-unreachable evidence, retry the client, or change production
recovery. No DESIGN or production change is required.

### Iteration-6 disposition

| Check | Result |
| --- | --- |
| Native test attribution | PASS — V-6 owns the visible green summary; third CP body started and never completed |
| Original semantic failure | PASS on cleanup success — terminal client remains an error with full console evidence |
| Readiness/client refusal acceptance | PASS — ECONNRESET-driven unhealthy backend and `mesh_unreachable` never count as flowing |
| Retry/timeout behavior | PASS — no retry and no timeout change |
| Public lifecycle effects | PASS — existing operator-stop verbs and awaited shutdown only |
| CID complement | PASS in source — exact empty map required |
| Forwarder complement | PASS in source — exact `(0, 0)` required on the empty-CID snapshot |
| Graceful shutdown | PASS in source when successful — explicitly awaited before primary report |
| Primary error after successful cleanup | PASS — saved failure is reported after shutdown |
| Primary error when cleanup fails | **FAIL** — cleanup panic displaces the saved semantic failure |
| Production/API/scenario/lane | PASS — unchanged |
| Source gates | PASS for compilation — `877a7df19197f55043fd02f8` and diff check `8c43d5a3ea613d06e1068f2e`; semantic blocker remains |

All iteration-5 lifecycle, allocation-identity and same-fd splice approvals
remain intact. The active E23 correction is original-crafter-owned and is not
reopened or reviewed here. The production readiness/resolve failure remains
for the original crafter to diagnose; this review prescribes no production
recovery change.

### Current native readiness

**NOT READY FOR NATIVE RERUN.** Make the bounded private failure-cleanup path
retain the original terminal-client failure even when stop, observation,
inventory or graceful shutdown cleanup also fails. After a scoped source gate
and independent re-review, the intended next execution remains current
`guest-kit-prepare` followed by one existing cgroup-default validation
requiring exact 119/119, zero failed, zero ignored required bodies, zero
runtime-SKIP diagnostics, terminal exit zero, exact current source/ELF
identity and complete outer-guest cleanup.

The retained exact-46 mtls-relay receipt remains source-qualified and valid;
no mtls-relay rerun is required for this control-plane test-fixture-only
correction.

### Iteration-6 verdict

**NEEDS_REVISION.** The prepared branch correctly detects the already-terminal
one-shot client, preserves the original relay failure when cleanup succeeds,
requires zero CIDs and zero forwarder inventory, and awaits graceful server
shutdown. It does not preserve the original semantic failure if any cleanup
step itself fails. Close that bounded evidence-preservation gap before
authorizing another cgroup-default119 GuestKit validation.

## Iteration 7 re-review — primary-first cleanup reporting

### F-04 remediation — closed

**Prior disposition:** blocker, OPEN  
**Iteration-7 disposition:** **CLOSED**

The existing failed-relay path now saves the original terminal-client failure
before cleanup and delegates to one private `fail_after_relay_cleanup` helper.
That helper independently catches and labels every cleanup operation:

1. client operator stop;
2. Service operator stop;
3. empty-CID observation;
4. exact zero-forwarder-inventory assertion; and
5. awaited graceful server shutdown.

Each asynchronous cleanup future is wrapped with
`AssertUnwindSafe(...).catch_unwind().await`; the synchronous inventory
assertion has its own `catch_unwind`. A panic in an earlier step is appended
to `cleanup_failures` and cannot skip any later cleanup attempt. Graceful
shutdown is therefore attempted even after either stop, observation or
inventory failure.

The final report is constructed once and the helper always panics. With clean
cleanup, it is exactly the original `primary_failure`. With secondary cleanup
failures, the report begins with that same primary text and appends labelled
cleanup evidence under `cleanup failures after the primary relay failure`.
The primary readiness/mesh-unreachable failure is no longer displaced by a
cleanup panic.

The exact complement remains fail-closed. Inventory is checked only when
`wait_observation` actually returns a snapshot whose CID map is empty. If that
observation fails, the report includes both the empty-CID failure and an
explicit statement that zero inventory was unavailable. The body never
credits `(0, 0)` without the required empty-CID snapshot. If the snapshot is
available but its inventory is nonzero, that assertion is independently
captured and reported as cleanup failure.

`panic_evidence` retains string panic payloads verbatim and labels non-string
payloads rather than dropping them. Catching cleanup panics does not turn any
path into success: the single final panic is unconditional.

### Acceptance and scope preservation

The relay acceptance oracle is unchanged. `FLOWING_LINE` is still the only
success result. A terminal one-shot client and the unchanged boot deadline
remain errors with full client and all-allocation console diagnostics. The
readiness ECONNRESET, unhealthy backend, typed `mesh_unreachable` refusal and
terminal reset from run `129e9c3d78f087fc13e8868e` remain failed production
behavior; this fixture correction neither retries nor accepts them.

The source change is confined to the existing control-plane integration test
file and existing identity. It uses the established public stop verbs,
observation surface and `ServerHandle::shutdown`. No timeout, production API,
hook, lane, scenario identity, health rule, resolver behavior or recovery
mechanism changes. The normal success path and its relay/quota/watch oracles
are untouched.

The corrected evidence attribution remains exact: the visible green summary
in the retained run belongs to V-6, while the third control-plane identity
started but never completed and receives no pass credit. A future recurrence
should now finish cleanup and report the production failure rather than
masquerading as a lost EXEC reply; it still receives no acceptance credit.

### Iteration-7 disposition

| Check | Result |
| --- | --- |
| Primary failure capture | PASS — saved before cleanup and used as the first final-report text |
| Client stop failure | PASS — independently caught and labelled |
| Service stop failure | PASS — independently caught and labelled after client-stop failure |
| Empty-CID observation failure | PASS — independently caught and labelled |
| Inventory credit without empty-CID snapshot | PASS — explicitly unavailable, never credited |
| Nonzero inventory | PASS — exact `(0, 0)` assertion failure retained as secondary context |
| Graceful shutdown failure | PASS — independently caught and labelled; shutdown is always attempted last |
| Clean cleanup | PASS — final failure remains the original relay failure only |
| Failed cleanup | PASS — final failure remains primary-first with all captured cleanup context |
| Relay success oracle | PASS — unchanged `FLOWING_LINE` only |
| Retry/timeout/production behavior | PASS — unchanged |
| API/hook/lane/test identity | PASS — none added or changed |
| Source gates | PASS — `9e9aad5c39b2852b4a9d703d` and scoped diff check `3c4868d644c9a7cd83373499`; no tests/native effects |

All earlier lifecycle, exact-identity, same-fd splice, GuestKit confinement
and 119/46 accounting approvals remain intact. F-04 is closed without
reopening E23 or prescribing a production recovery change.

### Execution readiness and production blocker

**THE FAILURE-CLEANUP SUPPORT IS APPROVED.** It is safe for a bounded current
cgroup-default execution to report and clean up either the original relay
failure or a later result. This approval is not evidence that the third
control-plane body is green and is not approval of the production
readiness/resolve behavior seen in `129e9c3d78f087fc13e8868e`.

The original STEP 14-04 production crafter still owns the reproduced
readiness-reset cause: an already-listening Service became unhealthy after
`ECONNRESET`, and the one-shot client was correctly refused
`mesh_unreachable`. Until that production cause is resolved through the
ordinary DELIVER/review path, canonical 119-GREEN remains blocked. A rerun of
unchanged production may be used only as bounded failure evidence, not as a
retry strategy or waiver.

After the production correction is independently accepted, the acceptance
gate remains current `guest-kit-prepare` followed by one existing
`OVERDRIVE_FWD_GUEST_GROUP=cgroup-default` run requiring exact 119/119, zero
failed, zero ignored required bodies, zero runtime-SKIP diagnostics, terminal
exit zero, exact source/ELF identity and complete outer-guest cleanup. The
retained exact-46 mtls-relay receipt remains source-qualified and valid.

### Iteration-7 verdict

**APPROVED FOR FAILURE-CLEANUP SUPPORT.** F-04 is closed: all cleanup effects
are attempted independently, exact complement credit remains fail-closed, and
the original relay failure is always retained first with any cleanup failures
as secondary context. The third control-plane identity remains failed and
uncredited. Production readiness/reset remediation is a separate original-
crafter responsibility and still blocks canonical cgroup-default119 GREEN.

## Iteration 8 re-review — typed current-readiness precondition

### Exact precondition mapping

The prepared correction reads the existing public
`AllocStatusResponse` through the same `GET /v1/allocs?job=<service>` surface
the fixture already uses. Immediately before the same one-shot client is
created, `wait_current_readiness_pass` requires:

- exactly one row for the already-selected Service allocation;
- that exact allocation still in `Running`;
- exactly one declared readiness descriptor;
- readiness index zero;
- the unchanged TCP mechanic at `0.0.0.0:SERVICE_PORT`;
- the unchanged one-second interval;
- the unchanged `success_threshold == Some(1)`; and
- the allocation's single current LWW readiness result at index zero to be
  `ProbeStatusJson::Pass`.

These are existing typed facts, not recreated health logic. The public probe
result projection contains the LWW winner per allocation/role/index, and the
descriptor's threshold of one makes a current Pass the existing
ServiceLifecycle input for backend eligibility. The earlier console count of
three successful probe connections proves historical application reachability
but cannot substitute for this current typed row.

The placement is correct. The fixture finishes its historical probe-count and
inventory-agreement observations, then reads this current precondition, and
only after success calls the existing `deploy_job` for the one-shot client.
If the snapshot read, descriptor assertions or bounded Pass condition fails,
the approved primary-first cleanup path receives `client=None`, stops only the
existing Service, requires the same cleanup complements, awaits shutdown and
reports the precondition failure. No client is manufactured on a failed
precondition.

### Bounds and evidence limits

The precondition uses the existing `SETTLE_BOUND` and the file's established
200 ms predicate-polling cadence. No timeout constant is extended, no blind
delay is inserted, no production probe is triggered and the client is never
retried. Waiting for a typed precondition to become true within its existing
bound does not count an earlier Fail as Pass; timeout retains the latest typed
result and descriptor diagnostics as a primary failure.

The observation is deliberately point-in-time. It establishes that the
Service allocation is Running and its current LWW readiness input is Pass at
the last public snapshot immediately before client creation. It does not prove
that a later probe cannot fail, that service-map hydration has already
consumed that exact row, or that the backend will remain healthy throughout
client boot. No such future-health or map-convergence credit is claimed. If a
probe fails after the snapshot and the client is again refused
`mesh_unreachable`, the unchanged terminal-client oracle fails and the
primary-first cleanup reports it. The current native run is therefore still
required.

### Acceptance preservation

The normal relay outcome is untouched:

- `FLOWING_LINE` remains the only client success predicate;
- the Paired Relay dial, guest flow, watch, quota and inventory assertions are
  unchanged;
- readiness probe-count and application-terminal assertions remain;
- zero CID and `(0, 0)` forwarder complements remain exact;
- failure cleanup remains primary-first and independently attempted; and
- no readiness Fail, client refusal, runtime skip or timeout receives pass
  credit.

The correction adds no production API, accessor, probe, health rule,
service-map policy, retry, sleep-only stabilization, seam, hook, lane,
scenario identity or changed descriptor. It does not explain or waive the
pre-accept ECONNRESET from run `129e9c3d78f087fc13e8868e`. That failure's
internal stage remains unproven and separately routed to the production
crafter.

### Iteration-8 disposition

| Check | Result |
| --- | --- |
| Exact allocation | PASS — same selected Service allocation, exactly one public row |
| Allocation state | PASS — must remain `Running` |
| Descriptor identity | PASS — readiness idx 0, unchanged TCP target and one-second cadence |
| Threshold | PASS — exact existing `success_threshold=1` |
| Current readiness | PASS — one LWW result, exact `ProbeStatusJson::Pass` |
| Placement | PASS — observation completes immediately before client creation |
| Failed precondition | PASS — no client created; approved primary-first cleanup runs |
| Historical pass substitution | PASS — historical count receives no current-health credit |
| Future health claim | PASS — none; later failure still fails the relay body |
| Map hydration claim | PASS — none; native composition must prove admission |
| Relay/watch/quota/inventory oracles | PASS — unchanged |
| Retry/timeout/probe behavior | PASS — no client retry or new probe; existing bound/polling only |
| API/seam/lane/test identity | PASS — none added or changed |
| Source gates | PASS — `9943406b681ce601b088feff` and scoped diff check `a4ad2a4fa5cbd5117ba3036e`; no tests/native effects |

All prior lifecycle, exact-identity, same-fd splice, failure-cleanup,
confinement and 119/46 accounting approvals remain intact.

### Current native readiness

**APPROVED FOR ONE CURRENT CGROUP-DEFAULT119 GUESTKIT VALIDATION BY THE
ORIGINAL STEP 14-04 CRAFTER.** Current `guest-kit-prepare` must first stage the
exact control-plane, CLI and worker integration ELFs and source identity, then
the existing `OVERDRIVE_FWD_GUEST_GROUP=cgroup-default` group may run once.

Acceptance still requires exact 119/119, zero failed, zero ignored required
bodies, zero runtime-SKIP diagnostics, terminal exit zero, exact current
source/ELF identity and complete outer-guest cleanup. The new observation is
source readiness only; it is not proof that the native body or canonical group
is green. A precondition failure or later `mesh_unreachable` refusal remains a
normal failed receipt and does not authorize an unchanged retry.

The retained exact-46 mtls-relay receipt remains source-qualified and valid.
The unresolved mechanism behind the earlier readiness ECONNRESET remains a
separate production-crafter investigation regardless of the next native
result; this DISTILL approval neither closes nor weakens it.

### Iteration-8 verdict

**APPROVED FOR ONE CURRENT NATIVE VALIDATION.** The existing body now checks
the exact typed current-readiness input that backend eligibility depends on,
at the correct boundary immediately before creating its one-shot client. It
does not claim future health or map hydration, does not retry the client, and
preserves every accepted relay and cleanup oracle. Canonical 119-GREEN remains
unproven until the exact current GuestKit receipt is collected.
