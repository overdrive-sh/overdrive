# DISTILL review — STEP 13-04 leg-C fixture alignment

## Review metadata

| Field | Value |
| --- | --- |
| Review ID | `STEP13-04-DISTILL-LEGC-FIXTURE-REVIEW-01` |
| Feature / roadmap step | `netns-density-295` / `13-04` |
| Review date | 2026-10-28 |
| Reviewer | Fresh independent DISTILL source reviewer |
| Iteration | 1 |
| Scope | Existing `vsock_legb_dial::node` fixture alignment for the registered leg-C positive tuple |
| Design authority | Feature-delta D8a, R5-3, R5-25, R5-43 and V-19; ADR-0125, ADR-0172 and ADR-0184; roadmap STEP 13-04 |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This review applied the repository's current `AGENTS.md`, testing,
development and verification rules, and the nWave DISTILL,
acceptance-review, completeness, test-design and BDD criteria. The
repository's direct Rust scenario and native-Markdown conventions take
precedence over the installed review definition's generic `.feature` and YAML
examples.

## Review boundary

The reviewed correction is the test-local delta in
`crates/overdrive-worker/tests/integration/vsock_legb_dial.rs` that retains the
existing concrete `SimGuestStreamDial` in `Node` and scripts the exact tuple
passed to `Node::serve_vm` with the existing
`SimDialScript::AcceptOpen { reply: Vec::new() }` outcome. The source handoff is
`deliver/distill-13-04-legc-fixture-alignment.md`.

The worktree also contains the original STEP 13-04 crafter's production,
marker, DES and execution-log work. That work is preserved and is outside this
bounded DISTILL review. In particular, the four removed
`pending DELIVER step 12` markers in
`vsock_constant_guest_prefix_rules.rs` remain the original crafter's
activation work. This review does not author, alter or claim them.

Only this review artifact was added. No production source, test source,
acceptance body, assertion, Contract Shape declaration, pending marker, DES
event, execution log, roadmap, model, lane or commit was changed by the
reviewer.

## Reviewed artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| `deliver/distill-13-04-legc-fixture-alignment.md` | `9e1fea05d6e6532037d078a30da0881d10c61dee79396f4f0e07bfda3fd89ae3` |
| `vsock_legb_dial.rs` | `ddd7445837a7cc5d71fe44f0b81d771b90c2aab5fa2f6f1b29e95114c5958fa5` |
| `vsock_constant_guest_prefix_rules.rs` | `853f30dede0a4ad29c8579c585a002e16c158ed52cf228b3c8f6d4c9725dd400` |
| `feature-delta.md` | `66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09` |
| ADR-0125 | `9b3adb9662f7a70c80c8479a6694ec2bd9f46212a66ffd5d5622dc5a69e80be6` |
| ADR-0172 | `bee320c2650ae975442ec3d763dad68e391bc91bf4dadda61c0227e0cc60f48f` |
| ADR-0184 | `70382a5b424823a10615ca8e8dbfdacf4a7e33806065a9ecd0d6a1d03942d096` |
| `deliver/roadmap.json` | `2a909f49f0edc671c989228def9daac8c3d7fb41763d55d7359ea7471c03875a` |
| Diagnostic summary | `71bca8236820eafff3b61525e719425da8e5040e1ce1e4bb36569eb5be087c73` |
| Raw GuestKit receipt | `6a41e3dc0a180411aef1d7f444a2df3e258127420a55a118b3f0612108655e2c` |

## Strengths

- The correction uses the existing accepted `GuestStreamDial` port and the
  existing `SimGuestStreamDial` test adapter. It adds no production or
  cross-crate interface surface.
- The fixture scripts only `SocketAddrV4::new(addr, port)`, where `addr` and
  `port` are the same values used to build the `AllocationSpec` passed to
  production `start_alloc`. It does not set the sim's default and does not
  accept an address range, port range or wildcard.
- `AcceptOpen` is the honest lifecycle shape for this cell: the guest-stream
  direction remains open while the remote client holds leg C open for the
  socket-table observation. It does not fabricate the leg-C handshake,
  accepted socket, nft diversion or remote path.
- The default for every unscripted target remains the typed
  `GuestDialError::NotServing`. The undeclared-tuple, unassigned-address and
  other negative firewall paths therefore remain fail-closed.
- All positive and negative test bodies in the two reviewed files, exact errno
  and routing assertions, security complements, Contract Shape declarations
  and body counts are unchanged.

## Contract assessment

ADR-0172 requires the constant output reject and prerouting drop to follow the
leg-C divert. Traffic to an unregistered guest-prefix tuple must be refused or
dropped, reach no host socket and leave by no other route; every packet of a
registered leg-C-diverted connection must reach leg C before either constant
rule sees it. Feature-delta R5-43 and V-19 require the remote and same-node
positive leg-C cases as well as the fail-closed host-local, remote and
undeclared-tuple cases.

The reviewed fixture represents only the driven VM application beyond the
already-diverted leg-C connection. It does not implement or emulate the
firewall contract. The real `HostMtlsIntercept` still owns the production nft
table and listeners; the remote peer still enters from a route-disjoint real
network namespace; `HostMtlsEnforcement` still performs the real mutual-TLS
server handshake on the captured leg-C descriptor; and the kernel socket
table remains the oracle for the registered leg-C tuple.

ADR-0125's program ownership and ADR-0184's boot-write refusal remain
production concerns of STEP 13-04. The correction changes neither. It also
does not touch service-VIP D24/D24a, a gateway exclusion, the node default
route, serve-up/down ownership, table read-back or cleanup.

No typed-error ambiguity exists in this fixture correction. The existing
mapping is exact:

| Condition | Existing typed representation and behavior |
| --- | --- |
| Exact tuple scripted by `serve_vm` | `GuestStreamDial::dial` returns `Ok(GuestStream)`; production retains leg C and starts the existing bidirectional relay. |
| Any unscripted tuple | `GuestDialError::NotServing`; `establish_guest` resets leg C and returns `MtlsEnforcementError::GuestDial { source }`. |
| Tuple absent from `inbound_destinations` | The constant rule rejects or drops before a guest dial can turn it into a positive case. |

The accepted semantic set and its interface representations already exist;
the patch neither adds nor encodes an error through a diagnostic string.

## Causal path and fixture honesty

### Registered tuple identity

`Node::serve_vm` builds `vm_spec(&self.server, addr, port)` and awaits the
production `MtlsInterceptWorker::start_alloc` before adding the test-side
script (`vsock_legb_dial.rs:405-419`). `start_alloc` derives the capability's
workload address and allowed service ports from that exact spec, installs an
inbound element for each exact `(workload_addr, port)` and activates the
capability (`mtls_intercept_worker.rs:2723-2753, 2768-2789`). The script key is
the same `SocketAddrV4::new(addr, port)`; there is no derived, normalized or
broadened identity.

The positive constant-rule cell connects the remote peer to the exact
`VM_ADDR:PORT` supplied to `serve_vm`
(`vsock_constant_guest_prefix_rules.rs:152-182`). The production leg-C listener
passes the accepted socket's recovered local address as
`Routed::Inbound { orig_dst }`; `handle_shared_inbound` claims the exact active
destination and port before spawning enforcement
(`mtls_intercept_worker.rs:1519-1529, 3111-3127`).

`HostMtlsEnforcement::enforce_inbound` carries that same `orig_dst` into
`inbound::establish_guest` for an address inside `GUEST_NODE_PREFIX`
(`overdrive-dataplane/src/mtls/mod.rs:920-954`). After the mutual-TLS server
handshake, `establish_guest` invokes
`guest_dial.dial(orig_dst, DialPurpose::Relay)` and resets leg C on every typed
dial error (`overdrive-dataplane/src/mtls/inbound.rs:43-89`). The fixture script
therefore answers the exact call made by production; it does not bypass that
call or create a second path.

### Open stream stays live for the existing oracle

`SimGuestStreamDial::script` inserts only a target-specific entry. Lookup
checks an exact `(target, purpose)`, then the exact target, then the unchanged
default (`guest_stream_dial.rs:296-299, 342-347`). For `AcceptOpen`,
`open_stream` creates a `GuestStream` whose `guest_end` is `None`
(`:443-460`). Its read drains any scripted reply and then parks while the
guest direction is open (`:481-506`).

Production moves that stream into the tracked leg-C/guest-stream relay
(`inbound.rs:76-89, 98-112`). The remote test thread separately holds its real
TCP and rustls objects until the main thread has observed the socket table
(`vsock_constant_guest_prefix_rules.rs:177-214`). Thus neither endpoint is
dropped merely because the mutual-TLS handshake completed. The cell still has
to complete the real bidirectional mutual-TLS handshake and find a real
established socket with local `VM_ADDR:PORT` and the remote namespace address;
the sim cannot manufacture either observable.

### Negative paths remain closed

`SimGuestStreamDial::new` still initializes its default to
`Refuse(GuestDialError::NotServing)` (`guest_stream_dial.rs:283-293`). The
patch does not call `script_default`. The same-node undeclared-port test calls
`serve_vm` only for `VM_ADDR:SERVICE_PORT`, then dials
`VM_ADDR:UNDECLARED_PORT`; that target has no accepting script and, more
fundamentally, no `inbound_destinations` member. Its exact
`Io(ConnectionRefused)`, sub-second verdict and silent wildcard-listener
assertions are unchanged (`vsock_legb_dial.rs:180-217`).

The remote unassigned-address test never calls `serve_vm` and keeps its exact
timeout/drop plus silent wildcard-listener oracle. The host-local TCP and UDP
tests also create no script and retain exact `ECONNREFUSED` and `EPERM`
assertions. No default accept, production fallback or post-`NotServing` socket
retention was introduced.

## Existing-cell preservation matrix

| Existing cell / contract | Source result |
| --- | --- |
| Host-local TCP to a non-gateway guest-prefix address | **UNCHANGED.** Exact `ConnectionRefused` within `VERDICT_BOUND`; wildcard listener accepts nothing. |
| Host-local UDP to a non-gateway guest-prefix address | **UNCHANGED.** Exact `PermissionDenied`/`EPERM` at `send`; wildcard socket receives nothing. |
| Remote TCP to an unassigned guest-prefix address | **UNCHANGED.** Exact timeout/drop; wildcard listener accepts nothing. |
| Remote TCP to the registered VM tuple | **UNCHANGED BODY.** Same route-disjoint peer namespace, trust roots, client SVID chain, mutual-TLS handshake, presented-certificate assertion, exact local/peer tuple assertion, release ordering and owner cleanup. The fixture now represents the already-declared serving application beyond leg C. |
| Same-node leg-B to the registered tuple | **UNCHANGED BODY.** Same production `enforce` entry, completed mTLS handshake, exact leg-C and leg-B socket identities, `127.0.0.1` bind and `fwmark 0x1`; the exact tuple receives the same open guest-stream fixture. |
| Same-node leg-B to an undeclared tuple | **UNCHANGED.** Exact `Io(ConnectionRefused)`, latency bound and no host accept. The accepting script exists only on the declared port. |
| Gateway exclusion, with/without default route and serve-up/down obligations elsewhere in V-19 | **UNAFFECTED.** No route, gateway, table-owner, listener-owner, marker, lane or assertion source was edited by this correction. Final STEP 13-04 replay remains responsible for the selected native package. |

Current source has four test bodies and four Contract Shape declarations in
`vsock_constant_guest_prefix_rules.rs`, and three test bodies and three
Contract Shape declarations in `vsock_legb_dial.rs`, exactly matching `HEAD`.
The scoped zero-context diff confirms that the constant-rule file differs from
`HEAD` only by the original crafter's four marker removals. The three
`vsock_legb_dial` bodies and their pending markers are unchanged; its executable
delta is confined to the shared test fixture.

## Acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | The registered remote and same-node positives remain paired with host-local TCP, host-local UDP, remote-unassigned and same-node-undeclared negative cases. |
| Given/When/Then form | 10/10 | The affected positive retains one serving-VM precondition, one remote connection action and exact handshake/socket outcomes. |
| Domain language | 10/10 | The source retains the precise leg C, registered tuple, guest prefix, remote peer and constant-rule vocabulary. |
| Coverage completeness | 10/10 | The correction restores the existing R5-43/V-19 positive fixture without adding or deleting a scenario or weakening a negative partition. |
| Walking-skeleton user-centricity | 10/10 N/A | No walking skeleton changes; this is an existing focused Tier-3 host-adapter cell. |
| Priority validation | 10/10 | The bounded fixture correction addresses the identified `NotServing` configuration and adds no adjacent mechanism. |
| Observable behavior | 10/10 | The cell observes a real mutual-TLS exchange and kernel socket tuple; it does not assert a sim call or private production field. |
| Traceability | 10/10 | The cell remains traced through V-19 and R5-43 to D8a and ADR-0172, with STEP 13-04 as its activation step. |
| Walking-skeleton boundary | 10/10 N/A | The real nft/netns/leg-C production composition remains intact. The correction changes only the already-simulated driven guest-application port's target script; the fixture's existing simulated resolver and clock are unchanged. |

All applicable dimensions meet the approval threshold. Every live Rust cell in
the two reviewed files retains its existing exact `CONTRACT_SHAPE` declaration.

## Test-design mandates and completeness

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production/hexagonal boundary | PASS | The remote cell drives the real production intercept owner and enforcement path. The sim is supplied through the accepted driven `GuestStreamDial` port. |
| CM-B — domain-language abstraction | PASS | Test names, comments and assertions state guest-prefix, leg-C, tuple, remote-peer and errno outcomes; sim details remain in the fixture. |
| CM-C — complete focused journey | PASS | Production registration, nft diversion, mutual TLS, exact leg-C socket observation, peer release and owner cleanup remain in one cell. |
| CM-D — pure logic / fixture isolation | PASS | The correction is test-adapter configuration. It duplicates no production routing, firewall, TLS, relay or guest-dial decision logic. |
| CM-E — observable universe | PASS | This layer-4/Tier-3 integration cell may use traditional assertions; the declared handshake and socket-table universe is unchanged. |
| CM-F — layer-dependent PBT | PASS | The real-kernel cells remain explicit single examples and add no generator. |
| CM-G — two-tier rich journey | PASS / N/A | No new journey or state-space tier is introduced. |
| CM-H — explicit integration sad paths | PASS | TCP refusal, UDP refusal, remote drop and undeclared tuple remain explicit named examples. |

For the canonical C1-C7 completeness checklist, this fixture-only delta adds no
input parameter, collection, state transition, operation, mode flag, semantic
error, resource claim or concurrency claim. Those delta-inapplicable items
count as passing under the checklist's N/A rule. The applicable partitions
remain represented: registered versus unregistered destination, host-local
TCP, host-local UDP and remote TCP, exact typed/errno failures, and a real
route-disjoint network namespace. Delta completeness is therefore **15/15**
without inventing another test body, lane or test-only API.

## Evidence limitations and required replay

The pre-correction leg-C output is not an independently retained runtime
receipt. `.context/netns-density-295/13-04-legc-diagnostic-summary.md` is a
crafter-authored diagnostic summary that reports a completed two-certificate
handshake followed by `ss []`. It explicitly states that the durable runner
retained no raw stdout. This review uses that summary only as a hypothesis
whose source path is consistent with the inspected code; it does not claim the
old runtime failure was independently re-proved.

The raw GuestKit receipt is narrower. It proves that GuestKit prepared the
lane and attempted to launch the disposable Cloud Hypervisor guest, then Cloud
Hypervisor failed to create its API server socket with `EACCES` under the
confined execution identity. It proves no post-correction leg-C outcome.

Accordingly, source approval is not final native acceptance. After this review,
the original STEP 13-04 crafter must use the existing durable Metal/GuestKit
route, keep all kernel, nft, cgroup and netns effects inside the disposable
Cloud Hypervisor guest, and retain the raw receipt for the unchanged existing
cell selection. The replay must demonstrate the mutual-TLS handshake and the
required established `VM_ADDR:PORT`/remote leg-C tuple while the negative
errno, routing, security and cleanup cells remain green. No physical-host
kernel run, new body, scenario, lane, API, seam or architecture is authorized.

## Mechanical verification

| Check | Result |
| --- | --- |
| Scoped source diff | PASS — fixture-only executable delta in `vsock_legb_dial.rs`; constant-rule bodies differ only by the original crafter's marker removals. |
| Registered script identity | PASS — exact `SocketAddrV4::new(addr, port)` after successful production `start_alloc`. |
| Default refusal | PASS — `SimGuestStreamDial::new` remains `Refuse(GuestDialError::NotServing)`; no `script_default` call. |
| Open-stream lifecycle | PASS — `AcceptOpen` has no guest end; its read parks and the production relay owns the returned stream. |
| Positive body honesty | PASS — real mutual TLS plus exact kernel socket tuple remain required. |
| Negative bodies and assertions | PASS — unchanged exact errno, timeout, no-accept and routing outcomes. |
| Body / Contract Shape counts | PASS — constant rules 4/4; leg-B fixture file 3/3; same as `HEAD`. |
| Public API / typed error / production fallback | PASS — none added or altered. |
| Scenario, lane, model and DES changes | PASS — none in the correction. |
| `cargo fmt --all -- --check` | PASS — independently rerun by this reviewer. |
| Scoped `git diff --check` | PASS — independently rerun by this reviewer. |
| `jq empty deliver/roadmap.json` | PASS — independently rerun; roadmap validation status is unchanged by this review. |
| Metal compile run `f014f7fe86ec4da1081c6f48` | PASS as reported by the source handoff; not independently rerun or collected by this reviewer. |
| Metal clippy run `32114f92ed04dc30ea9668f7` | PASS as reported by the source handoff; not independently rerun or collected by this reviewer. |
| Post-correction native replay | NOT RUN — required next through the existing disposable-guest route. |
| Mutation testing | NOT RUN — correctly reserved for the single final DELIVER-wave gate. |
| Commit | NOT CREATED. |

## Iteration 1 verdict

**APPROVED.** The correction makes the existing registered-VM fixture honest
by answering only the exact declared tuple with an open guest stream after
production registration. The real nft diversion, mutual-TLS server handshake,
leg-C descriptor and kernel socket-table oracle remain the system under test.
Every unscripted tuple still returns typed `NotServing`; the constant-rule
negative cases and all exact errno, timeout, no-host-accept, routing, security
and cleanup assertions remain unchanged.

The correction adds no production fallback, public API, typed error, owner,
lifecycle promise, lane, scenario, model, DES event or architecture. There are
no open DISTILL findings. The original STEP 13-04 crafter may proceed to the
existing disposable-guest native replay. STEP 13-04 is not runtime-complete
until that retained replay proves the positive leg-C tuple and the unchanged
negative package together.

# APPROVED

---

# Review iteration 2 — existing GuestKit support and confinement traversal

## Iteration metadata

| Field | Value |
| --- | --- |
| Iteration | 2 |
| Review date | 2026-10-28 |
| Scope | Existing `cgroup-default` manifest extension, worker-guest dependency staging, and GuestKit parent traversal fix before the 73-body replay |
| Prior disposition | Iteration 1's exact-tuple `AcceptOpen` / default-`NotServing` source approval remains in force |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This iteration is additive. It reviews only the bounded execution support added
after iteration 1: `overdrive-fwd-guest-suite` selecting the already-authored
STEP 13-04 native bodies in its existing `cgroup-default` group, the exact
worker-guest tools and kernel modules those bodies need, the matching
`guest-kit-prepare` inventory documentation, and the GuestKit directory
traversal correction required by the existing uid/gid-4200 confinement.

The original STEP 13-04 crafter's production changes, acceptance-body
transitions, marker activation, DES events and execution log remain outside
this DISTILL-owned support review. The selected bodies are reviewed for
identity, activation and placement; this iteration does not claim authorship
of their STEP 13-04 compiler fallout or assertions.

Only this review artifact was changed by the reviewer. No production source,
test source, GuestKit source, runner source, marker, scenario, lane, model,
roadmap, DES event, execution log or commit was changed.

## Iteration 2 artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| `deliver/distill-13-04-legc-fixture-alignment.md` | `4d51ba2899eb7d48353bda31b46412829e830a54bd0688de3c1e030e841ab3c0` |
| Iteration 1 review before this append | `001145f98673377ddf1bcd535b564a61798aec1fbb2cf98030077a0d82dab8fe` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `7e9c09f5d0621bd84f5642039a32bf9e6e4eb5d43a21c25f7691f0f3204b0082` |
| `overdrive-testing/src/guest_kit.rs` | `9061cd378171fb864d4bc5fcfa51305604cf15dcd35af759f8305dc78f1dc54e` |
| `xtask/src/guest_kit_prepare.rs` | `8fb67dfa3e504338a2db986e77f871afab30136492fbd340c5ad15d827c505ca` |
| `vsock_legb_dial.rs` | `ddd7445837a7cc5d71fe44f0b81d771b90c2aab5fa2f6f1b29e95114c5958fa5` |
| `vsock_constant_guest_prefix_rules.rs` | `853f30dede0a4ad29c8579c585a002e16c158ed52cf228b3c8f6d4c9725dd400` |
| `deliver/roadmap.json` | `2a909f49f0edc671c989228def9daac8c3d7fb41763d55d7359ea7471c03875a` |
| Post-fix 55-body raw receipt | `6090e5241628df6615300f85c56832f675684fe55b3c56dd170970a2a94c013f` |
| Rootfs-parent failure receipt | `96a6fd3ba7dc9ffb5b29fd8d81702adfe1fe4eb6c3f5d2eaa397d5f1dcff8e8d` |
| Runtime-parent diagnostic receipt | `b3ef8eefb1ccfde43925253f2c1f93849bf93171dacc1a8eb347e966714c0899` |

## Strengths

- The extension reuses one existing selectable group, three existing staged
  integration binaries, three existing disposable-guest boots and the
  existing `GuestRun::exec_cgroup` result oracle. It adds no executor or lane.
- Every new manifest row is an exact libtest name with expected count one.
  Missing, ignored, renamed, broadened or multiply matching cells therefore
  fail the existing per-invocation summary check.
- Baseline, STEP 13-04 and combined body counts are checked separately. The
  previous 55-body identity cannot be traded against the new 18-body identity.
- Only the worker integration guest receives the extra nft/socket diagnostic
  tools and tproxy/reject module closure. The control-plane guests retain their
  prior payloads.
- The permission correction preserves root ownership of parent directories
  and grants group 4200 execute-only traversal through exact mode `0710`.
  Production still chowns only per-allocation artifacts to uid/gid 4200 and
  still launches Cloud Hypervisor through the unchanged uid drop, device
  supplementary groups, launch seccomp filter and Landlock rules.
- A retained raw Metal receipt proves the same post-fix boundary booted all
  three disposable guests and ran the prior 45 invocations / 55 bodies with
  zero failures before the STEP 13-04 selection was appended.

## Existing group, guests and executor

The selectable group registry remains exactly the pre-existing eight names:
`probe`, `core`, `unload`, `ulp`, `mtls-relay`, `cgroup-default`, `stress` and
`nested-boundary` (`fwd_guest_suite.rs:1009-1033`). No new environment value,
CLI option, hook, binary target or dispatch branch exists.

`run_cgroup_default` still calls `run_cgroup_binary` exactly three times:

| Existing disposable guest | CID-band slot | Staged binary | Added STEP 13-04 rows |
| --- | ---: | --- | ---: |
| `cgroup-control-plane-acceptance` | 14 | `overdrive-control-plane--acceptance` | 0 |
| `cgroup-control-plane-integration` | 15 | `overdrive-control-plane--integration` | 6 |
| `cgroup-worker-integration` | 6 | `overdrive-worker--integration` | 12 |

The names, slots and binary identities are unchanged
(`fwd_guest_suite.rs:852-879`). `xtask::guest_kit_prepare::IN_GUEST_LANES`
also retains the same six entries and feature sets; its only change is the
inventory comment recording that the three already-staged binaries now serve
the additional selection (`guest_kit_prepare.rs:78-130`).

`run_cgroup_binary` still boots one `GuestRun`, mounts cgroup v2 and bpffs
inside that guest, enables the same cgroup controllers, and invokes the staged
libtest through `GuestRun::exec_cgroup` (`fwd_guest_suite.rs:758-849`). That
executor runs the existing `run-with-modules` guest payload, then the existing
`CGROUP_NOFILE_EXEC`, and accepts a result only when the command exits zero and
the new console slice contains exactly the required
`test result: ok. <N> passed; 0 failed;` prefix (`:521-559`). A missing exact
test, a still-ignored test or a zero-test libtest success cannot pass as one
body.

## Exact manifest and count accounting

Independent source extraction gives:

| Manifest | Harness invocations | Expected bodies |
| --- | ---: | ---: |
| Existing control-plane acceptance | 3 | 3 |
| Existing control-plane integration | 35 | 45 |
| Existing worker integration | 7 | 7 |
| **Existing baseline total** | **45** | **55** |
| STEP 13-04 control-plane integration | 6 | 6 |
| STEP 13-04 worker integration | 12 | 12 |
| **STEP 13-04 total** | **18** | **18** |
| **Combined `cgroup-default` total** | **63** | **73** |

The runtime accounting retains three distinct gates:

1. baseline bodies must equal `CGROUP_DEFAULT_BASELINE_BODIES = 55`;
2. STEP 13-04 bodies must equal `STEP13_04_NATIVE_BODIES = 18`; and
3. their sum must equal `CGROUP_DEFAULT_TOTAL_BODIES = 73`
   (`fwd_guest_suite.rs:424-426, 880-906`).

Each invocation separately checks its own expected libtest summary before its
manifest counter advances. The counters add the per-row expected count only
after that check returns `Ok`, so a row cannot contribute to the aggregate
when the selected binary ran zero tests, ignored a test, selected too many or
reported any failure.

All 18 added filter strings are unique, use `--exact`, set expected count one,
and resolve in the current source to active existing bodies:

| Existing binary / scenario family | Exact active bodies | Count |
| --- | --- | ---: |
| Control-plane integration — `boot_member_clear_refusal` | Rejected clear; non-empty postcondition; late commit after refusal | 3 |
| Control-plane integration — `shared_element_cleanup_failure` | Deletion rejection; read-back failure | 2 |
| Control-plane integration — `shared_guest_network_startup` | Failed step-8 table write, refusal, supervised restart convergence | 1 |
| Worker integration — `mtls_intercept_install` | Absent/create/read-back/idempotence; target replacement/foreign complement; valid wrong-target non-mutation | 3 |
| Worker integration — `netns_density_shared_owner` | Independent allocation stop; member repair; policy-route repair; wrong-target refusal | 4 |
| Worker integration — `shared_intercept_members` | Exact repair of every deleted intercept object with live allocations | 1 |
| Worker integration — `vsock_constant_guest_prefix_rules` | Host TCP refusal; host UDP refusal; remote unassigned drop; registered leg-C positive | 4 |

The source search found 18/18 function identities, zero duplicates and no
remaining pending marker on any selected body. Every selected body retains its
Contract Shape declaration. The name-resolution run
`0674ac52277fa1b0196ef774` additionally reports that the six staged libtest
binaries expose every exact name. That run is source-handoff evidence; this
reviewer did not recollect its durable output.

The roadmap's remaining STEP 13-04 executable criteria are source-local sim,
Quint or ordinary userspace cells: `netns_density_boot_order.rs`,
`shared_network_recovery_acceptance.rs`, `quint_quiescence.rs` and the worker's
module-local cells. They do not become stronger by moving into a real-kernel
guest. The 18-row manifest therefore covers the complete existing Tier-3
native subset named by STEP 13-04 without absorbing a non-native body or
omitting a native one.

## Worker-guest dependency closure

The worker guest alone sets `stage_step13_04_nft = true`. Its tool extension is
exactly `nft`, `ss` and `uname`; the existing payload already stages `cat`,
`chmod`, `ip`, `mkdir`, `mount`, `sh`, `stat`, `sleep`, the forwarder module,
the staged libtest and their dynamic-library closures
(`fwd_guest_suite.rs:600-637, 871-879`). Source inspection of the selected
worker families finds those tools sufficient for their external command
surface. No selected body calls an unstaged helper such as `python3` or
`sysctl`.

The worker guest's existing load order remains:

1. nested KVM/vhost device dependencies;
2. `nfnetlink`, `nf_tables`, `veth` and `tls`; then
3. the STEP 13-04 expression closure:
   `nf_defrag_ipv4`, `nf_defrag_ipv6`, `nf_reject_ipv4`, `nf_reject_ipv6`,
   `nf_tproxy_ipv4`, `nf_tproxy_ipv6`, `nft_reject`, `nft_reject_ipv4`, and
   `nft_tproxy`.

Dependencies precede their consumers. The IP-family owned table does not need
`nft_reject_inet`; IPv6 defragmentation/reject/tproxy modules are present
because the pinned kernel's `nft_tproxy` module links both protocol helpers.
The read-only Metal `modinfo` run `489cf650ec149c7dfc6b227b` reports that
dependency graph, and staging preflight `16f853ecdecaa7674eb0054b` reports every
listed module path present for the running 7.0 kernel. Those two results are
reported source-handoff evidence and were not rerun by this reviewer; the
source list and load order match them.

`host_module_file` only reads or decompresses the physical host's module files
into the image staging tree. `GuestRun` passes their in-image paths to the
existing scripted peer, whose `run-with-modules` implementation performs
`finit_module` inside the disposable guest and treats only `EEXIST` as an
already-loaded success. The runner issues no physical-host `nft`, `ip`,
`mount`, cgroup or module-load command for this group. The physical host
remains the Cloud Hypervisor launch substrate.

## Confinement traversal correction

The retained failure evidence establishes two consecutive DAC traversal
failures under the Metal runner's `umask 0027`:

- the original Cloud Hypervisor process could not create its API socket below
  a `root:root 0750` runtime parent; and
- after that parent was addressed, Cloud Hypervisor could not open the
  production-created rootfs clone below the `root:root 0750` image-work parent
  (`13-04-metal-guest-kit-rootfs-permission-41591c6f534f97e5f8d2643f.log`,
  exact `EACCES` on the per-launch clone).

The source correction applies one representation at every required
fixture-owned parent:

| Parent | Owner after correction | Mode | Lifetime / purpose |
| --- | --- | ---: | --- |
| Shared GuestKit staging root | Existing root uid, gid 4200 | `0710` | Persistent platform-owned fixture root; traverse to staged per-run work only |
| Each `WorkDir` image/work root | Existing root uid, gid 4200 | `0710` | Unique transient test directory; removed by `WorkDir::drop` |
| `GuestBoot::run_dir` rootfs-clone parent | Existing root uid, gid 4200 | `0710` | Transient child of the work root; lets uid/gid 4200 reach the production-chowned clone |
| Private `/run/ovd-gk-<pid>-<n>` runtime parent | Existing root uid, gid 4200 | `0710` | Unique transient `WorkDir`; lets uid/gid 4200 reach its production-chowned per-allocation run directory |

Every `chown` passes `None` for uid, so it cannot transfer parent ownership
away from root. Exact mode `0710` gives root `rwx`, group 4200 execute-only
traversal, and no access to others; it grants neither group listing/read nor
group write. The affected `WorkDir` values retain their existing RAII cleanup,
and `GuestKitVm` retains the private runtime `WorkDir` until the VMM is
terminated.

The per-allocation boundary remains production-owned. `RootfsPlan` places the
clone under `GuestBoot::run_dir`; `CloudHypervisorVmm::create` still performs
the real `FICLONE`, then `prepare_confined_paths` chowns the clone and the
exclusive per-allocation run directory plus entries to the configured
uid/gid. The correction adds no alternative open, copy, socket path or
unconfined fallback.

`confined_identity()` is unchanged: uid 4200, primary gid 4200, and the actual
gid owners of `/dev/kvm` and `/dev/vhost-vsock` as supplementary groups
(`guest_kit.rs:925-940`). `VmConfig` still uses
`VmConfinement::confined(confined_identity()?, 1024)`. Production rendering
still prepends `prlimit` and `setpriv --reuid=4200 --regid=4200
--groups=<device groups> --no-new-privs`, renders Cloud Hypervisor
`--seccomp true`, registers the launch seccomp filter, enables `--landlock`,
and supplies only the existing per-allocation run-directory Landlock grant.
No UID, primary/supplementary group, seccomp mode/filter, Landlock rule, device
mode or device holder contract changed.

The retained post-fix receipt
`13-04-metal-guest-kit-cgroup-default-final-873eb46d2df0c195e547c3ec.log`
independently records three Cloud Hypervisor guests on Linux 7.0, the existing
device modes, 45 successful libtest invocations, 55 passed bodies, zero failed
or ignored bodies, and terminal `exit_code=0`. An independent mechanical parse
of its 45 summary lines totals exactly 55 passes. It proves the confinement
boundary and prior manifest after the permission correction. It predates the
18-row selection and therefore does not prove the combined 73-body result.

## Prior fixture approval remains valid

Iteration 2 does not touch `vsock_legb_dial.rs` after the source identity
reviewed in iteration 1. `Node::serve_vm` still scripts only the exact
registered tuple with `SimDialScript::AcceptOpen`; `SimGuestStreamDial::new`
still defaults every unscripted target to typed `NotServing`. No runner or
permission change can turn the remote leg-C positive into a wildcard accept,
weaken the undeclared-tuple refusal or bypass the real nft/netns/mTLS/socket
oracle.

## Acceptance-review dimensions and mandates

| Gate | Result | Assessment |
| --- | --- | --- |
| Coverage and traceability | PASS | The 18 rows map exactly to the native cells named by STEP 13-04; no unrelated or duplicate scenario was selected. |
| Observable behavior | PASS | The runner delegates to the existing libtest bodies and requires their real pass summaries; it adds no substitute assertion. |
| Hexagonal / production boundary | PASS | Tests still drive their production owners inside the disposable guest; support code only stages dependencies and executes the existing binaries. |
| Business/domain language | PASS | Manifest names preserve the exact owner, object, fault and outcome vocabulary of the authored bodies. |
| Complete focused journeys | PASS | No body is split, copied or shortened; positive, refusal, recovery, restart and non-mutation controls remain in their existing cells. |
| Fixture isolation | PASS | Physical-host activity is limited to staging and VMM launch; guest kernel effects execute through the existing guest payload. |
| Contract Shape | PASS | All 18 selected bodies resolve to live existing tests with their declarations intact. |
| Layer discipline | PASS | Real-kernel examples remain example-based; sim and Quint properties stay in their ordinary lower-cost lanes. |
| API and typed-error contract | PASS | The support delta adds no public method/type/variant/parameter and changes no semantic error representation. |
| Security boundary | PASS | Exact parent traversal closes reproduced DAC failures without changing the uid drop, device groups, Landlock, seccomp or per-allocation ownership. |

The C1-C7 completeness taxonomy is unchanged because iteration 2 adds no
scenario, input class, state, operation, flag, error or concurrency claim. Its
only completeness question is execution selection: every existing native
partition in the STEP 13-04 roadmap is present exactly once, and every
non-native partition remains in its proper lane. Delta completeness remains
**15/15** under the checklist's N/A rule.

## Mechanical verification

| Check | Result |
| --- | --- |
| Selectable group names | PASS — unchanged eight-name registry; no new group. |
| Guest boots / CIDs / staged binaries | PASS — same three names, slots 14/15/6 and existing binaries. |
| Baseline manifest | PASS — 45 invocations / 55 expected bodies. |
| STEP 13-04 manifest | PASS — 18 unique exact invocations / 18 expected bodies. |
| Combined accounting | PASS — separately gated 55 + 18 = 73 bodies across 63 invocations. |
| Exact name resolution | PASS — 18/18 source identities found, all active; Metal run `0674ac52277fa1b0196ef774` reported staged-libtest resolution. |
| Native-criterion inventory | PASS — all roadmap-named Tier-3 bodies selected; sim/Quint/source-local bodies remain outside the guest. |
| Worker tools | PASS — added only `nft`, `ss`, `uname`; selected external-command inventory has no missing tool. |
| Worker kernel modules | PASS — dependency-before-consumer closure matches reported `modinfo` and staging preflight runs. |
| Physical-host kernel effects | PASS — modules/tools copied on host, loaded/executed only by the in-guest peer. |
| Parent ownership and mode | PASS — uid preserved as root, gid 4200, exact `0710`; no group read/write or other bits. |
| Parent lifecycle | PASS — persistent fixture root only; work/rootfs/runtime parents stay within existing RAII and VMM lifetimes. |
| VMM identity and device groups | PASS — unchanged uid/gid 4200 plus actual device-owner supplementary gids. |
| Landlock and seccomp | PASS — unchanged `--landlock`, fixed run-dir grant, `--seccomp true`, launch filter and `no_new_privs`. |
| Post-fix baseline native receipt | PASS — independently inspected raw run `873eb46d2df0c195e547c3ec`: 45 summaries, 55 passed, zero failures, exit 0. |
| `cargo fmt --all -- --check` | PASS — independently rerun by this reviewer. |
| Scoped `git diff --check` | PASS — independently rerun by this reviewer. |
| `jq empty deliver/roadmap.json` | PASS — independently rerun. |
| Runner/xtask Cargo check and Clippy | PASS as reported by Metal run `c7b2a147bb442f59c0a5a604`; not recollected by this reviewer. |
| Module dependency / presence reads | PASS as reported by Metal runs `489cf650ec149c7dfc6b227b` and `16f853ecdecaa7674eb0054b`; not rerun by this reviewer. |
| Combined 73-body native replay | **NOT RUN** — required next through `OVERDRIVE_FWD_GUEST_GROUP=cgroup-default`. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER-wave gate. |
| Production / DES / commit changes by reviewer | NONE. |

## Iteration 2 verdict

**APPROVED.** The execution-support delta selects exactly the 18 existing
STEP 13-04 native bodies in the existing `cgroup-default` route, retains the
prior 45-invocation / 55-body identity, and fails closed on each invocation and
on the baseline, extension and combined body totals. It reuses the same three
disposable guests, CID slots, staged binaries and guest executor. Added worker
tools and kernel modules are the bounded dependency closure of the selected
nft/tproxy/reject and socket-table cells and are loaded only inside the
disposable worker guest.

The GuestKit fix gives the unchanged uid/gid-4200 VMM execute-only traversal
through fixture-owned parents while preserving root ownership, per-allocation
chown, supplementary device groups, Landlock, seccomp and cleanup. The retained
raw 55-body run proves that corrected boundary without a physical-host kernel
replay.

Iteration 1's exact-tuple `AcceptOpen` and default typed `NotServing` approval
remains valid. There are no open DISTILL findings. The original STEP 13-04
crafter may now run the existing `cgroup-default` group and retain the raw
disposable-guest receipt. STEP 13-04 is not native-complete until that new
receipt proves exactly 55 baseline passes plus 18 STEP 13-04 passes, 73 total,
with zero failures.

# APPROVED

---

# Review iteration 3 — `shared_element_cleanup_failure` Harness boot order

## Iteration metadata

| Field | Value |
| --- | --- |
| Iteration | 3 |
| Review date | 2026-10-28 |
| Scope | Existing `shared_element_cleanup_failure::Harness` boot correction after the first CH73 replay reached the action-shim flow-port invariant |
| Trigger | Durable GuestKit run `c13d2413f408e84f38d6453c`, first STEP 13-04 control-plane body |
| Prior disposition | Iterations 1 and 2 remain approved: exact-tuple fixture, confinement traversal, dependency staging and 55+18 manifest identity are unchanged |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This iteration reviews one DISTILL-owned fixture correction. The existing
Harness now awaits the accepted `SharedGuestNetworkOwner::converge_shared`
operation on its exact `RecordingGuestNetwork` instance before the fixture
convenience constructor captures flow ports and before the paired EXEC
supervisor opens the gate.

Only this review artifact was modified by the reviewer. No production source,
test source, assertion, marker, scenario, runner group, manifest, lane, model,
roadmap, DES event, execution log or commit was changed.

## Iteration 3 artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| `deliver/distill-13-04-legc-fixture-alignment.md` | `79c4b408e95dda13d576b899e9a73b1bda29b26468e11529dc79ee6210652d99` |
| Iteration 2 review before this append | `81c55fff0fbeb973cf60d1a0b2a66b2eaa206966fbbd90834504f337e76dbd29` |
| `shared_element_cleanup_failure.rs` | `dc954eb23f948f7802f2661a421cd240f8b4d0bf98868f86115938db0186894b` |
| `13-04-ch73-failure.md` | `7c38fe388f473aa3c36d64e20fa7bd551857854fdfd08456b29bf256f0ae84f8` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `7e9c09f5d0621bd84f5642039a32bf9e6e4eb5d43a21c25f7691f0f3204b0082` |
| `overdrive-testing/src/guest_kit.rs` | `9061cd378171fb864d4bc5fcfa51305604cf15dcd35af759f8305dc78f1dc54e` |
| `xtask/src/guest_kit_prepare.rs` | `8fb67dfa3e504338a2db986e77f871afab30136492fbd340c5ad15d827c505ca` |
| `vsock_legb_dial.rs` | `ddd7445837a7cc5d71fe44f0b81d771b90c2aab5fa2f6f1b29e95114c5958fa5` |
| `deliver/roadmap.json` | `2a909f49f0edc671c989228def9daac8c3d7fb41763d55d7359ea7471c03875a` |
| `feature-delta.md` | `66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09` |

The runner, GuestKit, staging manifest and exact-tuple fixture hashes are
identical to iteration 2. Their approved group, permission, module/tool,
physical-host and leg-C findings remain in force.

## Failure classification

The retained handoff reports that the first combined 73-body run reached the
disposable Cloud Hypervisor boundary and failed in
`shared_element_cleanup_failure_deletion_rejected_retains_retirement_and_address`
before the authored cleanup-fault stimulus:

```text
internal error: entered unreachable code:
a successfully booted shared owner has reported its bound flow ports
at crates/overdrive-control-plane/src/action_shim/mod.rs:1468:13
```

The durable output was not recollected by this reviewer;
`.context/netns-density-295/13-04-ch73-failure.md` is the retained local
handoff. The source independently supports its classification:

- the test dispatch passes `state.guest_flow_ports.get().copied()` with the
  Harness's provisioner (`action_shim/mod.rs:1410-1442`);
- the action path treats absent ports as unreachable after successful boot
  (`:1460-1474`);
- `AppState::new` copies ports only when its injected owner already reports
  them (`lib.rs:700-709`); and
- `SimSharedGuestNetworkOwner::flow_ports` returns `None` until successful
  convergence, then the accepted fixed ports
  (`overdrive-sim/src/adapters/guest_network.rs:352-368`).

The prior Harness started the shared mTLS owner and opened EXEC without
converging the shared guest-network owner. This is `FIXTURE_BROKEN`, not
missing production functionality and not a falsification of the accepted
startup contract. Production correctly rejected the impossible boot claim.

## Startup-order alignment

The production boot's load-bearing relative order is:

1. start the shared mTLS owner after stale-state cleanup;
2. converge the exact shared guest-network owner;
3. capture that owner's bound `GuestFlowPorts` once; and
4. only after retained owner composition, open the paired EXEC gate
   (`lib.rs:4089-4136, 4457-4468`; feature-delta boot steps 8–12).

Production constructs the full `AppState` earlier and fills its `OnceLock`
after convergence. The broad fixture constructor has a documented companion
shape: its caller supplies an already-converged owner, and `AppState::new`
captures `flow_ports()` during construction (`lib.rs:700-704`). The correction
uses that contract rather than writing the slot manually.

The corrected Harness order is exact:

```text
MtlsInterceptWorker::start_shared_owner
  -> RecordingGuestNetwork::converge_shared
  -> AppState::new reads that owner's flow_ports
  -> the same GuestNetworkExecWiring supervisor opens EXEC
  -> allocation actions dispatch
```

The source positions are 804, 821–824, 825–847 and 848–849 in
`shared_element_cleanup_failure.rs`. The reconciler registration between owner
construction and convergence opens no forwarding or EXEC state. This focused
fixture need not replay every earlier `run_server` probe and stale-sweep step;
it must establish the accepted preconditions of the production action path.
The same fixture order already appears in
`mtls_install_fail_closed::SeamFixture` and the action-shim admission fixture.

## Same owner, same gate, no fallback

`Harness::boot` creates one `Arc<RecordingGuestNetwork>` at lines 805–806. The
new call converges that Arc. The same Arc is injected into `AppState` as the
`SharedGuestNetworkOwner` at line 844, retained as `Harness.network` at line
855, and passed as the action-shim `GuestNetworkProvisioner` at lines 864–870.
No second owner or copied state exists.

`RecordingGuestNetwork::converge_shared` delegates directly to
`self.inner.converge_shared().await` (`:641-655`). It does not assign a port,
fabricate a listener, toggle EXEC or choose a fallback. The inner operation
records `ForwarderOpen` in its own sim trace and sets `converged` only on `Ok`;
`flow_ports` derives its answer from that state. Failure remains a fail-closed
Harness setup failure through the existing `expect`.

The Harness retains one `GuestNetworkExecWiring`: `AppState::new` receives its
gate at line 845 and the supervisor comes from that same wiring at line 848.
`open_after_boot` therefore opens the gate the action shim reads.

## Fault and evidence honesty

The convergence call cannot consume or mask the cleanup fault:

- `ElementModel::default` starts with `fault = None`;
- owner convergence never touches `ElementModel`;
- the predecessor still dispatches and reaches `Running` first
  (`shared_element_cleanup_failure.rs:962-979`); and
- `harness.model.arm(fault)` remains immediately before the normal stop
  (`:981-987`).

It also does not alter the allocation/lease journal. The fixture `Log` is
written only by provision/activate/teardown recording and `LeaseLayer`.
`RecordingGuestNetwork::converge_shared` delegates without recording,
clearing or slicing that log. The inner sim's boot `ForwarderOpen` trace is a
separate owner trace.

Existing phase marks remain only around stop and retry. The complete
append-only attempt, install and network/lease histories are still printed
after all checks (`:983-1035, 1183-1206`). No boot-time mark or evidence clear
was added.

The full behavioral oracle remains active. It still proves the exact injected
typed cause, complete failed-removal attempt, preserved pre-state,
not-converged stop, same-address refusal with no acquisition, absence of
teardown/release on failure, different address for the fresh successor,
Retiring lease, Running durable row, successful exact retry removal, converged
stop, one teardown before one lease release, and terminal row only after
cleanup.

There are 17 `report.check` calls, equal to `HEAD`, and both live tests retain
their bounded-change Contract Shape declarations. Iteration 3 changes no
predicate, expected value, typed-error match, fault timing, address check,
journal slice, aggregate assertion or body count.

## Previous approvals remain valid

The unchanged runner still selects 45 baseline invocations / 55 bodies and 18
STEP 13-04 invocations / 18 bodies, 63 invocations / 73 bodies total, through
the same three disposable guests and separate fail-closed count gates.

The unchanged GuestKit and staging support retain the approved root-owned
gid-4200 `0710` parents, uid/gid-4200 VMM, supplementary device groups,
Landlock, seccomp, bounded tools/modules and physical-host boundary.

The unchanged leg-C fixture still scripts only the registered tuple with
`AcceptOpen`; every unscripted tuple retains typed `NotServing`.

## Acceptance-review gates

| Gate | Result | Assessment |
| --- | --- | --- |
| Production boundary | PASS | The Harness satisfies the existing owner operation before driving the production action shim; no side channel. |
| Startup order | PASS | mTLS owner, exact guest-network convergence, port capture and same-gate open preserve the accepted order. |
| Test honesty | PASS | A pre-stimulus setup failure is removed; all cleanup witnesses and outcomes remain. |
| Owner/gate identity | PASS | One owner Arc and one wiring instance serve convergence, AppState, provision and EXEC. |
| Fault placement | PASS | Unarmed through boot/start; armed only at the authored stop boundary. |
| Journal completeness | PASS | No boot clear/slice; phase marks and complete diagnostics unchanged. |
| API / typed error | PASS | No public surface or semantic mapping added or weakened. |
| Scenario / lane / model | PASS | No new body, scenario, runner selection, lane or model. |
| Prior security and leg-C gates | PASS | Iteration 1 and 2 identities are unchanged. |

No C1-C7 partition changes. The correction makes the existing failure/retry
journey executable through its accepted preconditions, so delta completeness
remains **15/15** without a new scenario or seam.

## Mechanical verification

| Check | Result |
| --- | --- |
| Correction placement | PASS — converge after mTLS start, before AppState capture and EXEC open. |
| Same owner / gate | PASS — one owner Arc and one `GuestNetworkExecWiring`. |
| Port source | PASS — existing `flow_ports()` after successful converge; no manual write/fallback. |
| Fault timing | PASS — predecessor start line 969 precedes `arm(fault)` line 983. |
| Journal mutation | PASS — convergence neither writes nor clears the allocation/lease log. |
| Assertions / Contract Shape | PASS — 17 checks equal to `HEAD`; two bounded-change cells remain. |
| Runner identity | PASS — unchanged 45/55 + 18/18 = 63/73. |
| Prior support hashes | PASS — runner, GuestKit, xtask and leg-C fixture unchanged. |
| `cargo fmt --all -- --check` | PASS — independently rerun. |
| Scoped `git diff --check` | PASS — independently rerun. |
| `jq empty deliver/roadmap.json` | PASS — independently rerun. |
| Scoped Cargo check | PASS as reported by Metal run `f86ac248f0df8706ea0c7c74`; not recollected here. |
| Scoped Clippy `-D warnings` | PASS as reported by Metal run `de69af6d9807559116f847d1`; not recollected here. |
| Post-correction 73-body replay | **NOT RUN** — required next through the approved disposable-guest route. |
| Mutation testing | NOT RUN — reserved for the final DELIVER-wave gate. |
| Production / DES / commit changes by reviewer | NONE. |

## Iteration 3 verdict

**APPROVED.** The correction supplies the missing accepted boot precondition
through the existing port on the exact owner already injected by the Harness.
Successful convergence is the sole source of bound flow ports;
`AppState::new` captures them before the same wiring's EXEC gate opens. No
alternate owner, manual state, fallback port, public API or production
weakening was introduced.

The cleanup fault remains post-start and pre-stop. All typed-failure,
membership, retry, retirement, address, lease-order, durable-row and complete
journal evidence remains active. Iterations 1 and 2 remain valid, and the
manifest remains exactly 55 + 18 = 73 bodies.

There are no open DISTILL findings. The original STEP 13-04 crafter may replay
the existing `OVERDRIVE_FWD_GUEST_GROUP=cgroup-default` route and retain the
raw receipt. STEP 13-04 is not native-complete until that run proves all 73
bodies with zero failures.

# APPROVED

---

# Review iteration 4 — guest-only route precondition for output-reject cells

## Iteration metadata

| Field | Value |
| --- | --- |
| Iteration | 4 |
| Review date | 2026-10-28 |
| Scope | Bounded main-table route lifetime around the two existing host-local constant-prefix output-reject cells |
| Trigger | Durable GuestKit run `ada3ff7709ff6cde475adca0`, exact TCP cell returned `ENETUNREACH` before nft output |
| Prior disposition | Iterations 1–3 remain approved and source-identical except for this runner-only routing support |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This iteration reviews a private change to the existing
`overdrive-fwd-guest-suite` runner. It supplies a real but guest-local routing
precondition only while the existing TCP and UDP host-local output-reject
cells execute, then removes it before the existing remote-drop and leg-C
cells. It changes no production route, test body, errno oracle, scenario,
group, lane, public API, model, count or DES state.

Only this review artifact was modified by the reviewer. All other dirty work
is preserved.

## Iteration 4 artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| `deliver/distill-13-04-legc-fixture-alignment.md` | `9bec56a642a750d0a61e42c365629b8b71a01d12ee9ee53335f048d55956c3bc` |
| Iteration 3 review before this append | `17f45a11a8b4b8254a292a98b26db695947966c874e4bb4f837a91c6721700bb` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `e6a6580e4a8599bbca0e8748b69558636a34263761ea8f970ad87d7e10b8ec8d` |
| `vsock_constant_guest_prefix_rules.rs` | `853f30dede0a4ad29c8579c585a002e16c158ed52cf228b3c8f6d4c9725dd400` |
| `shared_element_cleanup_failure.rs` | `dc954eb23f948f7802f2661a421cd240f8b4d0bf98868f86115938db0186894b` |
| `overdrive-testing/src/guest_kit.rs` | `9061cd378171fb864d4bc5fcfa51305604cf15dcd35af759f8305dc78f1dc54e` |
| `xtask/src/guest_kit_prepare.rs` | `8fb67dfa3e504338a2db986e77f871afab30136492fbd340c5ad15d827c505ca` |
| Route-failure diagnosis | `21bf2b8d3c1b15f6e147a5f199065b645f49b2070a502143befed005365a4419` |
| Raw `ada3ff7709ff6cde475adca0` receipt | `27fb89f8691a76ad64c0855777e447dac5383dee988860f210199e8ce898e774` |
| `deliver/roadmap.json` | `2a909f49f0edc671c989228def9daac8c3d7fb41763d55d7359ea7471c03875a` |
| `feature-delta.md` | `66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09` |

The constant-rule test body, GuestKit, staging manifest and cleanup Harness
hashes are unchanged from their prior approvals. The only iteration 4
executable change is in the existing test-support runner.

## Native failure and correct classification

The retained raw receipt directly shows that iteration 3's Harness correction
worked: both `shared_element_cleanup_failure` cells reached and passed their
unchanged 17-entry universes. The worker guest then reached the first
constant-rule body and observed:

```text
[rules-host-tcp][connect][+0ms] Network is unreachable (os error 101)
assertion failed: left NetworkUnreachable, right ConnectionRefused
```

This is a real route-lookup result from Linux 7.0 inside the disposable guest.
It does not disprove ADR-0172: V-19 permits `ENETUNREACH` when no node route
covers the prefix. It does prove that this particular body never reached its
subject. The body requires a routable main-table lookup so the production nft
output hook can emit exact `ECONNREFUSED`; its UDP sibling requires the same
hook to emit exact `EPERM`. Accepting `ENETUNREACH` would turn both into
vacuous positives and weaken their authored contract.

The correction changes the environment so the existing assertions reach their
intended boundary. It does not change production or the expected error.

## Exact guest-only route ownership

`STEP13_04_ROUTE_SETUP` first requires
`ip -4 route get 100.95.255.201` to fail. It then:

1. creates the uniquely named dummy link `ovd-gk-route`;
2. assigns only `192.0.2.1/32` to that link;
3. brings it up;
4. adds one main-table default route through that dummy with preferred source
   `192.0.2.1` and metric `42760`; and
5. accepts setup only when `ip route get 100.95.255.201` reports both exact
   `dev ovd-gk-route` and exact `src 192.0.2.1`
   (`fwd_guest_suite.rs:128-147`).

No `table` selector is supplied to `ip route add`, so this is the normal main
table the unmarked host-local sockets use. The fixture installs no
`100.95.0.0/16` route, no gateway `/32`, no `local` route and no address in the
guest prefix. `192.0.2.1` is the TEST-NET-1 documentation address and is
disjoint from both `GUEST_NODE_PREFIX` and the `TestCidrLease`
`10.250.0.0/16` pool.

The dummy module is already staged in every GuestKit image and loaded by the
scripted guest's production-fixture prerequisite path. The worker payload also
names that same in-image module in its existing `run-with-modules` list;
re-loading returns the established `EEXIST` success. No physical-host module
is loaded.

The dummy interface is a sink with no external peer. It lets Linux complete
source and route selection while giving the packet no physical or virtual
uplink out of the disposable guest. The real production nft output program is
still installed by each cell's `Node::start` and remains the only mechanism
that can produce the asserted immediate TCP/UDP errors.

## Positive-gate honesty

The two routed manifest rows are exactly:

- `a_host_local_connection_to_a_guest_prefix_address_is_rejected_at_once`;
  and
- `a_host_local_datagram_to_a_guest_prefix_address_fails_eperm`.

Their bodies are byte-identical to iteration 2. The TCP cell still opens a
real unmarked `TcpStream`, demands exact `ConnectionRefused` inside the
existing latency bound and proves a wildcard listener accepted nothing. The
UDP cell still opens a real unmarked `UdpSocket`, demands exact
`PermissionDenied` from `send_to` and proves a wildcard socket received
nothing. The runner neither invokes nft directly for their verdict nor
injects an errno.

If the constant output reject is absent, TCP no longer receives the authored
immediate rejection and UDP send no longer returns the authored `EPERM`; the
dummy precondition cannot satisfy either oracle. The correction therefore
removes a false negative without creating a false positive.

## Route lifetime and no-default controls

The worker manifest is now partitioned without changing membership:

| Phase | Existing exact bodies | Count | Main default state |
| --- | --- | ---: | --- |
| Route-independent STEP 13-04 worker cells | `mtls_intercept_install` (3), `netns_density_shared_owner` (4), `shared_intercept_members` (1) | 8 | absent |
| Host-local constant-rule cells | TCP output reject, UDP output reject | 2 | test-owned dummy default present |
| Remote constant-rule cells | remote unassigned drop, registered leg-C positive | 2 | absent again |

Source order is setup at line 887, the routed cells at 888, cleanup at 889,
and post-route cells only at 904. Cleanup deletes the exact metric/device
default, deletes the uniquely owned link, and then requires the same unassigned
guest-prefix target to be unroutable (`:148-156`). The remote-drop and leg-C
bodies cannot start unless cleanup succeeds.

The remote bodies are the only selected STEP 13-04 worker cells that create a
`PeerNetns` and acquire a `TestCidrLease`. They run after the default is gone.
Their `10.250.x.0/24` veth route is therefore never exposed to the temporary
default, and their peer namespace's guest-prefix route remains the only route
the remote fixture itself adds. The remote drop still exercises prerouting
drop with no node default, and the leg-C cell still exercises the real
bidirectional remote path with no node default.

The raw `ENETUNREACH` receipt remains valid diagnostic evidence of the initial
no-route state. The runner now verifies that state both before setup and after
cleanup. Existing with/without-default-route, gateway-exclusion and
serve-up/down obligations are not reclassified or collapsed.

## Cleanup and error priority

On normal success, both routed bodies must pass, explicit cleanup must pass,
and the target must be unroutable before the post-route cells run. A cleanup
failure after successful bodies is returned as the runner failure and blocks
the no-default suffix.

On a routed-body failure, the runner stores the authored error, attempts the
same route/link cleanup, and returns the authored error even if cleanup also
fails (`fwd_guest_suite.rs:887-902`). This preserves the first behavioral
failure rather than replacing it with fixture-cleanup noise. If setup itself
fails after partial mutation, or explicit cleanup also fails, normal Rust
unwinding drops the `GuestRun`; `GuestKitVm::drop` terminates the disposable
Cloud Hypervisor process and destroys that guest's entire network namespace.
That teardown is the fail-closed cleanup complement.

All setup and cleanup commands are sent through `GuestRun::command` to
`overdrive-guest-peer` inside `cgroup-worker-integration`. The physical Metal
host performs only staging and VMM lifecycle work. It receives no default
route, dummy link, nft change, network namespace change or module load.

## No STEP 13-05 mechanism pulled forward

The support route is deliberately a generic main default to a TEST-NET dummy
source. STEP 13-05's production gateway mechanism is a tagged
`local <GUEST_NODE_GATEWAY>/32 dev lo` route with production ownership and
lifecycle. This correction creates none of those objects, names or owners.
It is a private fixture precondition with a bounded two-cell lifetime, not a
production route implementation.

No public type, method, enum, parameter, config, CLI, model action or semantic
error representation changed.

## Count and prior-approval preservation

Independent source extraction yields:

| Manifest | Invocations | Bodies |
| --- | ---: | ---: |
| Existing baseline | 45 | 55 |
| STEP 13-04 control-plane | 6 | 6 |
| Route-independent STEP 13-04 worker | 8 | 8 |
| Routed host-local firewall | 2 | 2 |
| Post-cleanup remote firewall | 2 | 2 |
| **STEP 13-04 total** | **18** | **18** |
| **Combined total** | **63** | **73** |

All 18 filters remain unique exact names with expected count one. The
baseline, STEP 13-04 and combined total gates are unchanged. There is still
one `cgroup-default` selectable group, three disposable guests and the same
staged binaries and CID slots.

Iteration 1's exact-tuple `AcceptOpen` and default typed `NotServing`,
iteration 2's confinement/dependency support, and iteration 3's exact-owner
boot convergence all remain source-identical and approved.

## Acceptance-review gates

| Gate | Result | Assessment |
| --- | --- | --- |
| Real boundary | PASS | Real guest routing reaches the real nft output hook; no mocked verdict or injected errno. |
| Test honesty | PASS | Exact TCP/UDP oracles remain; `ENETUNREACH` is not accepted as a substitute. |
| Fixture ownership | PASS | One uniquely named dummy/default inside one disposable worker guest. |
| Scope | PASS | Route exists only around the two host-local cells and no production route is added. |
| No-default controls | PASS | Explicit cleanup and an unroutable postcondition precede both remote cells. |
| Failure cleanup | PASS | Original cell error wins; explicit cleanup is attempted; guest teardown is the complement. |
| Network isolation | PASS | TEST-NET dummy source is disjoint; the 10.250 lease topology is created only after cleanup. |
| Scenario / API / model | PASS | No body, assertion, scenario, lane, public surface or formal model changed. |
| Count identity | PASS | Same 55+18=73 fail-closed manifest. |
| Prior gates | PASS | Iterations 1–3 remain source-identical where bounded by their hashes. |

The route is an environment precondition for existing C7 configuration
partitions, not a new behavior or error partition. C1-C7 delta completeness
remains **15/15** without a new scenario.

## Mechanical verification

| Check | Result |
| --- | --- |
| Raw failure | PASS — independently inspected `ada3ff7709ff6cde475adca0`: exact `ENETUNREACH` before nft output. |
| Route precondition | PASS — initially unroutable; exact dummy device and source after setup. |
| Route contents | PASS — one main default; no guest-prefix, gateway, local-table or production route. |
| Routed selection | PASS — exactly the TCP and UDP host-local output cells. |
| Cleanup order | PASS — route/link deletion and unroutable postcondition before remote cells. |
| Error priority | PASS — routed test error retained after cleanup attempt. |
| Guest teardown complement | PASS — `GuestRun`/`GuestKitVm` ownership destroys the namespace on failure. |
| TestCidrLease isolation | PASS — only post-cleanup cells acquire the disjoint 10.250/16 pool. |
| Body / errno changes | PASS — constant-rule file hash unchanged; only prior marker removals differ from `HEAD`. |
| Manifest identity | PASS — 18 unique filters; split 8+2+2; combined 55+18=73. |
| Shell syntax | PASS — setup and cleanup snippets independently parsed with `sh -n`. |
| `cargo fmt --all -- --check` | PASS — independently rerun. |
| Scoped `git diff --check` | PASS — independently rerun. |
| `jq empty deliver/roadmap.json` | PASS — independently rerun. |
| Runner Cargo check / Clippy | PASS as reported by Metal run `39c1f5e6464fa1d377862ea1`; not recollected here. |
| Post-correction 73-body replay | **NOT RUN** — required next through the approved disposable-guest route. |
| Mutation testing | NOT RUN — reserved for the final DELIVER-wave gate. |
| Production / DES / commit changes by reviewer | NONE. |

## Iteration 4 verdict

**APPROVED.** The runner now supplies exactly the real main-table lookup the
two existing output-reject cells require, using a guest-only dummy sink and an
outside-prefix source. The production nft output hook must still produce the
unchanged exact TCP and UDP errors, so the correction does not bypass or
weaken the cells.

The default route is absent before setup, removed and re-proved absent before
the remote/no-default cells, and destroyed with the guest on every unclosed
failure path. No production gateway route or STEP 13-05 mechanism is pulled
forward. The exact 73-body manifest and all prior approvals remain intact.

There are no open DISTILL findings. The original STEP 13-04 crafter may replay
the existing `OVERDRIVE_FWD_GUEST_GROUP=cgroup-default` route and retain the
raw receipt. STEP 13-04 is not native-complete until that post-correction run
proves all 73 bodies with zero failures.

# APPROVED

---

# Review iteration 5 — disposable-worker `/var/run` compatibility

## Iteration metadata

| Field | Value |
| --- | --- |
| Iteration | 5 |
| Review date | 2026-10-28 |
| Scope | Standard `/var/run -> /run` compatibility path required by the existing `PeerNetns` fixture inside the disposable worker guest |
| Trigger | Durable GuestKit run `603782e98f2f5fd2ee6b8821`, remote-drop cell reached `/var/run/netns` after host-local route cells and cleanup passed |
| Prior disposition | Iterations 1–4 remain approved; their bounded source identities are unchanged except for this runner prerequisite |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

Iteration 5 reviews one test-support filesystem prerequisite in the existing
worker guest. Before any worker libtest body runs, the runner prepares
`/run/netns`, creates the conventional `/var/run -> /run` alias only when the
alias is wholly absent, and verifies the canonical destination and namespace
directory. No production path, mount, host filesystem, test body, assertion,
lane, public API, model, manifest row or count is changed.

Only this review artifact was modified by the reviewer. All other dirty work
is preserved.

## Iteration 5 artifact identity

| Artifact | SHA-256 at review |
| --- | --- |
| `deliver/distill-13-04-legc-fixture-alignment.md` | `cdba5e1a5785d98554e78090e1cfbdcec67fc2d4b0cdf21c8b0669d79296b39c` |
| Iteration 4 review before this append | `f4a7457db334c56294163e435c8d4c4e8fff6780970ea7fc85b37fac09499a7d` |
| `overdrive-testing/bin/fwd_guest_suite.rs` | `37d48818f1cc781bea7870ad44e080616eb8747b41e8d1a15501780c37562f0d` |
| `overdrive-testing/src/guest_kit.rs` | `9061cd378171fb864d4bc5fcfa51305604cf15dcd35af759f8305dc78f1dc54e` |
| `xtask/src/guest_kit_prepare.rs` | `8fb67dfa3e504338a2db986e77f871afab30136492fbd340c5ad15d827c505ca` |
| `shared_element_cleanup_failure.rs` | `dc954eb23f948f7802f2661a421cd240f8b4d0bf98868f86115938db0186894b` |
| `vsock_legb_dial.rs` | `ddd7445837a7cc5d71fe44f0b81d771b90c2aab5fa2f6f1b29e95114c5958fa5` |
| `vsock_constant_guest_prefix_rules.rs` | `853f30dede0a4ad29c8579c585a002e16c158ed52cf228b3c8f6d4c9725dd400` |
| Netns-path diagnosis | `49f63528ff820a7ca011c3d1214b354f5dafb54e64e607e4f1ea57e7163d0626` |
| Raw `603782e98f2f5fd2ee6b8821` receipt | `cc4386269832346c77e10dda5b8e05dc9809a5aa11b8f6c63b3b0e145467461a` |
| `deliver/roadmap.json` | `2a909f49f0edc671c989228def9daac8c3d7fb41763d55d7359ea7471c03875a` |
| `feature-delta.md` | `66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09` |

The GuestKit, staging manifest, cleanup Harness, constant-rule bodies and
leg-C fixture hashes remain identical to their prior approvals. The runner is
the only iteration 5 executable delta.

## Native failure and boundary classification

The retained raw receipt proves iteration 4's route support through its exact
boundary: both host-local output-reject cells passed, explicit route cleanup
and the unroutable postcondition returned successfully, and execution entered
the first post-cleanup remote cell. That cell then failed only while opening
its namespace handle:

```text
precondition: netns /var/run/netns/ovt-rules-remote-drop-00210 opens:
No such file or directory (os error 2)
```

The same receipt also shows both cleanup-failure cells passed their unchanged
17-entry universes and the step-8 boot-write refusal/restart cell passed. It is
partial native evidence, not the final 73-body receipt.

`PeerNetns::create` successfully invokes `ip netns add`, which publishes the
handle under `/run/netns/<name>` in the disposable guest. Its unchanged
`spawn_in` then opens the traditional path `/var/run/netns/<name>` before
`setns`. A normal Linux filesystem supplies `/var/run -> /run`; the minimal
GuestKit rootfs omitted that alias. The failure is therefore a fixture-rootfs
precondition, not a namespace, routing, nft or production defect.

## Exact path and source ownership

Only the worker guest, selected by `stage_step13_04_nft = true`, gains the
already-installed `ln` userspace tool. Before its baseline or STEP 13-04 bodies
run, the runner executes one fail-closed script through the in-guest peer:

```text
mkdir -p /var /run/netns
require /var/run to be neither an existing path nor any symlink
ln -s /run /var/run
require /var/run to be a symlink
require its physical destination to be exactly /run
require /var/run/netns to be a directory
```

The exact command is at `fwd_guest_suite.rs:883-895`, before baseline execution
at line 897, route setup at line 900 and the remote cells at line 917.

The two absence checks are complementary: `test ! -e /var/run` rejects every
existing resolved path, while `test ! -L /var/run` also rejects a dangling
symlink. The runner never deletes, replaces or silently adopts pre-existing
filesystem state. This is deliberately one-shot ownership in a fresh
disposable rootfs rather than an idempotent reconcile: a second setup in the
same guest fails before mutation. The existing runner invokes it once.

The in-guest peer and libtest run as guest root, so `/var`, `/run/netns` and
the alias are guest-root-owned. No permission is widened on a physical-host
directory, no mount is created for the alias, and no host path is referenced.
The alias persists only for the worker guest's lifetime. Normal success powers
off that guest and releases its work image; any setup or test failure drops
`GuestRun`, whose `GuestKitVm` terminates the VMM and discards the entire guest
rootfs. No separate cleanup is required or possible outside that rootfs.

## Existing namespace fixture remains authoritative

The correction leaves `PeerNetns` unchanged:

- `create` still acquires a route-disjoint `TestCidrLease`, creates the netns
  with `ip netns add`, moves the veth peer, and gives the remote namespace its
  guest-prefix route;
- `spawn_in` still opens `/var/run/netns/<name>` and calls the real `setns`;
- `Drop` still deletes the veth and namespace through `ip`; and
- the remote-drop and leg-C cells still assert their original real-kernel
  outcomes.

The compatibility alias fixes the minimal rootfs to the fixture's standard
Linux contract. Changing `spawn_in` to `/run/netns` would instead make the
authored fixture depend on this one stripped rootfs and would duplicate
filesystem variation inside the scenario support.

No test can pass merely because the alias exists. `ip netns add` must still
create the handle, `File::open` must resolve it, `setns` must succeed, the veth
topology must configure, and the existing TCP/mTLS/nft oracles must pass.

## Bounded prerequisite audit

The immediately related audit is complete for the selected 18 bodies:

| Selected-body surface | Existing disposable-worker support |
| --- | --- |
| Namespace handles | `ip`, `mkdir`, `ln`, `/run/netns`, and verified `/var/run -> /run` compatibility |
| nft and policy routing | `nft`, `ip`, nfnetlink/nf_tables, tproxy/reject dependencies and veth, all guest-only |
| Socket/kernel evidence | `ss` and `uname` with their library closures |
| Route precondition | GuestKit's stock dummy module and the reviewed bounded dummy default |
| Locks and CIDR leases | Existing rootfs `/tmp`; unchanged `KernelStateLock` and `TestCidrLease` paths |
| Kernel filesystems | Existing procfs, sysfs, devtmpfs, private cgroup v2 and bpffs mounts |
| Process/device limits | Existing RLIMIT and confined `/dev/kvm`/`/dev/vhost-vsock` permissions and groups |

The selected `mtls_intercept_install` rows require only their existing
`ip`/`nft` surface. Later unselected rows that call `sysctl`, `python3` or
other tools remain outside the exact manifest; no speculative tool or module
was staged for them.

## Route, security and physical-host preservation

Iteration 4's guest-only default route setup and cleanup are unchanged. The
raw `603782...` run reaching the remote cell proves the two routed errno cells
passed and the cleanup/unroutable gate completed. The compatibility script
runs before those phases and changes no route, nft object, address, network
namespace or module state.

GuestKit confinement support is unchanged: root-owned gid-4200 `0710`
parents, uid/gid-4200 VMM, supplementary device groups, Landlock and seccomp.
The new alias is created after guest boot by guest root and grants the VMM
process no host access.

All compatibility and test effects remain inside
`cgroup-worker-integration`. The physical Metal host performs staging and VMM
lifecycle only; it receives no symlink, directory, mount, route, netns, nft,
cgroup or module mutation.

## Manifest and prior approvals

The manifest remains 18 unique exact STEP 13-04 rows:

- control-plane: 6;
- route-independent worker: 8;
- routed host-local firewall: 2; and
- post-cleanup remote firewall: 2.

Together with the unchanged 45-invocation / 55-body baseline, the runner still
requires 63 invocations / 73 bodies through the separate baseline, STEP 13-04
and combined gates. The same group, three guests, CID slots and binaries
remain.

Iteration 1's exact-tuple `AcceptOpen` and typed default `NotServing`,
iteration 2's confinement/dependency support, iteration 3's exact-owner boot
convergence, and iteration 4's bounded route/cleanup remain approved.

## Acceptance-review gates

| Gate | Result | Assessment |
| --- | --- | --- |
| Rootfs boundary | PASS | Standard compatibility is created and verified only inside the disposable worker rootfs. |
| Source ownership | PASS | Pre-existing path or symlink refuses before mutation; runner owns exactly one fresh alias. |
| Namespace honesty | PASS | Real `ip netns`, file open and `setns` remain mandatory; no fake handle. |
| Permissions/lifetime | PASS | Guest-root-owned paths live until guest teardown; no physical-host permission change. |
| Failure semantics | PASS | Any prerequisite failure stops before bodies and disposable teardown removes partial state. |
| Prerequisite completeness | PASS | All surfaces used by the selected 18 bodies are staged/mounted; unselected tools remain unstaged. |
| Route/no-default controls | PASS | Iteration 4 source unchanged and partially native-proved by reaching the remote suffix. |
| Scenario / API / model | PASS | No body, assertion, scenario, lane, public surface or model changed. |
| Count identity | PASS | Same 55+18=73 manifest. |
| Prior approvals | PASS | Iterations 1–4 remain source-identical within their bounded artifacts. |

This is a C7 environment prerequisite for existing scenarios, not a new
behavior, state or error partition. Delta completeness remains **15/15**.

## Mechanical verification

| Check | Result |
| --- | --- |
| Raw failure | PASS — independently inspected `603782e98f2f5fd2ee6b8821`; exact missing `/var/run/netns` path after routed cells passed. |
| Tool staging | PASS — only worker payload adds existing `ln`; `ip`, `mkdir` and `sh` already present. |
| Alias source | PASS — exact target `/run`; canonical `pwd -P` and `/var/run/netns` verified. |
| Existing-state refusal | PASS — resolved paths and dangling symlinks both reject before `ln`. |
| Ordering | PASS — compatibility at line 889 before baseline line 897, route line 900 and remote line 917. |
| Manifest identity | PASS — 18 unique filters; 6+8+2+2; combined 55+18=73. |
| Test source | PASS — constant-rule and `PeerNetns` source hashes unchanged. |
| Related prerequisite audit | PASS — no missing selected-body tool/module/filesystem surface found. |
| Shell syntax | PASS — exact script independently accepted by `sh -n`. |
| `cargo fmt --all -- --check` | PASS — independently rerun. |
| Scoped `git diff --check` | PASS — independently rerun. |
| `jq empty deliver/roadmap.json` | PASS — independently rerun. |
| Post-correction 73-body replay | **NOT RUN** — required next through the approved disposable-guest route. |
| Mutation testing | NOT RUN — reserved for the final DELIVER-wave gate. |
| Production / DES / commit changes by reviewer | NONE. |

## Iteration 5 verdict

**APPROVED.** The runner now supplies the standard Linux `/var/run -> /run`
filesystem contract that the unchanged `PeerNetns` fixture requires. It owns
one verified alias in one fresh disposable worker rootfs, refuses any
pre-existing path rather than overwriting it, and leaves the real namespace
creation, handle open, `setns`, topology and outcome assertions intact.

The prior native replay already proves the two host-local errno cells and
route cleanup before this filesystem precondition. All previous fixture,
confinement, routing, leg-C and count approvals remain valid. No production or
physical-host surface changed.

There are no open DISTILL findings. The original STEP 13-04 crafter may replay
the existing `OVERDRIVE_FWD_GUEST_GROUP=cgroup-default` route and retain the
raw receipt. STEP 13-04 is not native-complete until that run proves all 73
bodies with zero failures.

# APPROVED
