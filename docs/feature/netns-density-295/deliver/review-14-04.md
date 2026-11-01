# DELIVER review — STEP 14-04 non-regression composition

## Review metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `14-04` |
| Review ID | `STEP14-04-DELIVER-POSTCOMMIT-01` |
| Review type | Fresh isolated post-commit implementation review |
| Parent | `92f0c60d4e3001dca7a233b7ec25979bafde1055` |
| Implementation range | `8e017a3e4bd9df4451203beab36b5ec733377aab` through `9b0bb15387de112ff2318604acb7384c69f24947` (including remediation `ccb473c7182c754ceba1329cc469002cb62d9585`, DES repair `9b0bb15387de112ff2318604acb7384c69f24947`, and authorized naming-policy commit `db5b78a5f198d053cb52e65169bdee313d26e2b2`) |
| Iteration | 1 historical review preserved; 2 remediation re-review; 3 F-04 repair re-review |
| Final verdict | **APPROVED** |
| Open findings | 0 |

This review applies the current `AGENTS.md`, the software-crafter reviewer
skills, the applicable Rust, development, testing, verification and workflow
rules, the complete `.context/netns-density-295/dispatch-14-04.md`, the
accepted probe and vsock ADRs, the feature-delta contracts, and the frozen
04-03 / S-ND295-36 suite map. The repository's native Markdown review
convention takes precedence over the installed review skill's legacy YAML
output.

Only this review artifact is written. The user-owned dirty `AGENTS.md` was
preserved. No production source, test body, roadmap, design artifact, DES
event, host state or commit was changed by the reviewer.

The complete iteration-1 record is preserved below, including its original
three findings and dispositions. The sections through the first verdict are
the historical review of the original implementation commits. The following
iteration sections preserve each remediation decision; the latest iteration-3
section is the current cumulative assessment through `HEAD`.

## Review iteration 1 — historical record

## Authority and review boundary

STEP 14-04 revalidates existing behavior after the vsock attachment. Its
accepted outcomes are:

1. Exec/process netns and veth provisioning, leg-C inbound TPROXY and the
   DNS responder retain their existing outcomes.
2. The frozen 04-03 / S-ND295-36 bodies remain green without assertion
   weakening: `guest_stack_mtls_egress`, `service_kind_vm_workloads`,
   `cgroup_accounting_equivalence`, `mtls_resolve_rekey` and the Service
   dataplane selection body.
3. Only the approved #325 issue-linked marker, plus the separately owned
   next-step 14-05 markers while that step is pending, remain ignored.

The implementation change is private and bounded to
`crates/overdrive-worker/src/probe_runner/http_prober.rs:253-264`: it records
the status and drops the Hyper response immediately instead of collecting an
unbounded or incomplete body. The follow-up only appends the actual COMMIT
event to `execution-log.json`.

## Commit and DES integrity

Both commits preserve the Marcus author and contain exactly one
`Co-Authored-By: Codex <codex@openai.com>` trailer and one `Step-Id: 14-04`
trailer. No Claude, Anthropic or generated attribution appears. The reviewed
range changes exactly two files:

| File | Change |
|---|---:|
| `crates/overdrive-worker/src/probe_runner/http_prober.rs` | 12-line private implementation correction |
| `docs/feature/netns-density-295/deliver/execution-log.json` | actual RED/GREEN/COMMIT phase events |

The DES history is present and chronological:

| Phase | Result | UTC timestamp |
|---|---|---|
| RED | FAIL | 2026-11-01 09:13:15 |
| GREEN | PASS | 2026-11-01 09:25:58 |
| COMMIT | PASS | 2026-11-01 09:26:26 |

`git diff --check` passes for the reviewed range. The implementation commit
is a necessary scope expansion beyond the roadmap's guidance list: the
existing S-SVM-14 acceptance body cannot satisfy the accepted status-only
bounded-body contract while `probe_guest_stream` awaits a body that never
reaches EOF. No test body or assertion was changed.

## Contract and implementation assessment

### Status-only probe contract

The accepted `HttpProber` port requires the result to depend on the response
status, with the response body discarded regardless of size
(`crates/overdrive-core/src/traits/prober.rs:99-143`; ADR-0054 §3). The
current implementation obtains `response.status().as_u16()` and calls
`drop(response)` before `classify_http_status` at
`crates/overdrive-worker/src/probe_runner/http_prober.rs:253-264`.

The existing S-SVM-14 body supplies the semantic necessity proof shape at
`crates/overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs:406-461`:
it scripts an `AcceptOpen` guest response with a `Content-Length` of 1 MiB but
only 64 KiB available, and checks 204/302/404/503 status outcomes, no
`HttpProber` call, no allocation-state write, and unchanged probe intent. A
body collection would wait for an end that this scripted guest does not send;
immediate response disposal returns the status outcome.

Dropping the response does not leave the guest stream owned by a detached
task. The `GuestStream` is held by `GuestStreamIo` or one of its in-flight
operation futures, and dropping the Hyper response body causes the legacy
client connection to close when the body is incomplete. The pinned dependency
source for the workspace's `hyper-util` 0.1.20 contains the direct regression
test `drop_response_body_closes_in_progress_connection` and Hyper's H1
dispatcher closes or drains the read side when the body receiver is dropped.
The existing outer `tokio::time::timeout` and probe-supervisor cancellation
path remain unchanged.

### API, typed errors, wire and telemetry

| Contract surface | Assessment |
|---|---|
| Public / cross-crate API | PASS — no method, type, trait, enum, parameter or constructor changed. |
| Typed semantic errors | PASS — transport errors still use the existing `ProbeFailure` / `ProbeOutcome` mapping; body disposal adds no new failure condition. |
| HTTP status and operator reason | PASS — `classify_http_status` and all reason strings are unchanged. |
| Wire / persisted shape | PASS — the same `ProbeOutcome` reaches the existing `ProbeResultRow`; no body bytes were part of the contract. |
| Ownership / cancellation | PASS by source and dependency behavior — response disposal closes the in-progress `GuestStream`; no future is spawned or detached. |
| Lifecycle gates / telemetry | PASS — `probe_tick` still writes one status observation and does not author `Running`; cancellation and terminal ownership remain in the existing supervisor. |

No production defect finding is raised. The implementation is the smallest
change that restores the approved status-only outcome and removes the
reachable incomplete-body wait.

## Test integrity and Contract Shape compliance

The step diff contains no test-file change, so there is no RED-to-GREEN test
modification to approve or reject. The pre-existing S-SVM-14 body retains
both required declarations at lines 410-411:

```text
/// CONTRACT_SHAPE: bounded-change.
/// Outcome anchor: DISCUSS Elevator Pitch
```

Its assertions remain unchanged. The test body is a driving-port acceptance
test through `ProbeRunner::start_alloc` and the simulated `GuestStreamDial`,
not a private helper test. No new test, scenario, lane, seam, model or
executor was authored in this step. Test-budget expansion is not applicable;
zero test bodies were added.

## Required-suite evidence map

The crafter's reported userspace selections were CP 7/7, worker Service 7/7,
S-SVM-14 1/1, cgroup equivalence 2/2, exec-release 3/3, Sim guest-network
7/7, DNS 32/32 and CLI safe subset 2/2, with format/check/scoped Clippy
passing. Those results are not independently accepted as receipts here:
the required `.context/netns-density-295/14-04/` directory does not exist,
and the two implementation commits retain no raw command output,
source-identity manifest, skip ledger, terminal record or cleanup audit.

The current source and retained approved GuestKit support do establish the
following existing coverage without duplicating native work:

| Required family | Current identity / existing support | Current evidence disposition |
|---|---|---|
| Exec/process netns and veth | `CONTROL_PLANE_NORMAL_NATIVE` in `crates/overdrive-testing/bin/fwd_guest_suite.rs:1061-1141` includes the exact DNS, `serve_boot_provisions_veth`, `veth_provision_idempotent` and `workload_netns_provision` identities. | **Covered by existing support.** The approved cgroup-default normal-native suffix and canonical 117 accounting are unchanged by the HTTP-prober diff; no new lane is justified. |
| DNS responder | The seven `dns_responder_bind` identities at `fwd_guest_suite.rs:1073-1096`. | **Covered by existing cgroup-default GuestKit evidence** retained by the 13-05/14-01 reviews. The reported userspace DNS selection alone is not the real bind/wire evidence, but it is not needed as a duplicate. |
| Leg-C / guest-prefix TPROXY | `a_leg_c_diverted_remote_connection_reaches_leg_c_both_ways` at `fwd_guest_suite.rs:928-938`, plus the real `mtls_inbound_enforce::inbound_enforce_origdst_server_mtls_ktls_rx_splice_to_server` body in the existing `mtls-relay` manifest at `fwd_guest_suite.rs:509-512`. | **Covered by existing cgroup-default / mtls-relay GuestKit evidence.** The old direct host-native receipts are not used. |
| Service dataplane selection | `integration::service_map_forward::ten_tcp_syns_to_vip_are_rewritten_and_forwarded_via_veth` at `fwd_guest_suite.rs:580-583`. | **Covered by the approved 46-body mtls-relay GuestKit gate** (`8066a14e0a5afc0f0c4919fd`, exact 46, exit 0). Do not run or add a second host-root veth lane solely for this body. |
| `service_kind_vm_workloads` | Existing control-plane and worker acceptance bodies; they use the approved Sim/port composition. | **Userspace evidence claimed, but step-specific raw receipt absent.** Re-run through the approved unprivileged/Sim path or retain the original crafter receipt before completion. |
| `cgroup_accounting_equivalence` | `cgroup_accounting_equivalence_sim` and `cgroup_accounting_equivalence_real` at `crates/overdrive-host/tests/integration/cgroup_accounting_equivalence.rs:51-151`; the real case uses `TempDir` and never opens VMM or host cgroup state. | **Userspace evidence claimed 2/2; no GuestKit lane is required.** Raw receipt is absent from the step package. |
| `mtls_resolve_rekey` | Existing control-plane acceptance bodies at `crates/overdrive-control-plane/tests/acceptance/mtls_resolve_rekey.rs:97-353,568-600`. | **Userspace/Sim evidence claimed; raw receipt absent from the step package.** |
| `guest_stack_mtls_egress` / S-ND295-36 | Frozen 04-03 names `microvm_dials_a_mesh_peer_by_name_and_receives_the_reply` and `the_guests_mesh_traffic_travels_the_peer_wire_as_mtls_never_in_the_clear`. Neither identity exists in the current `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs`; the current S-ND295-01 map says those old cells were retargeted to 10-01, while the current 14-01 GuestKit body is the different `vsock_walking_skeleton` identity. | **BLOCKED — missing current body/evidence mapping.** The old names cannot be credited by a generic suite selection, and the 14-01 vsock walking skeleton is not evidence for the absent frozen identities. DISTILL must pin the exact current body mapping and reuse the existing `run_nested_vm_test_binary` / cgroup-default staging only after that mapping exists. |

The two known E6 failures are retained as baseline evidence only:
`integration::guest_stack_mtls_egress::fault_fixture::setup_fault_matrix_restores_the_exact_production_object_graph`
and
`integration::guest_stack_mtls_egress::fault_fixture::trapped_fixture_paths_restore_programs_counters_handles_userdata_and_fib`.
Their exact prior failures are recorded in
`docs/feature/netns-density-295/distill/upstream-issues.md:110`: the first
reports `the injected setup-original-rename-before-intent interruption is
observable` because `try_install_fixture_only` returned `Ok` where `Err` was
expected, and the second reports `the injected construction interruption is
observable` for `setup-original-rename-after-mutation`. They do not waive the
absent positive S-ND295-36 outcomes or permit a whole-suite skip.

## Findings

### F-01 — Blocker — S-ND295-36 has no current exact guest-stack body or safe native receipt

**Kind:** `EVIDENCE/ACCEPTANCE`

The frozen 04-03 criterion requires the two named guest-stack positive bodies,
but neither function is present at the reviewed HEAD. The current source
contains other `guest_stack_mtls_egress` bodies, while the current DISTILL
partition records S-ND295-01 as retargeted to 10-01. The approved 14-01
`vsock_walking_skeleton` GuestKit receipt proves a different production
composition and cannot silently substitute for the frozen S-ND295-36 body
identities. The E6 fixture failures are pre-existing negative evidence and do
not prove either positive outcome.

This is an acceptance evidence gap, not a production defect: no caller,
shutdown schedule or ordering defect is asserted. The smallest resolution is
an upstream DISTILL reconciliation that names the exact current existing body
for each frozen outcome and then reuses the existing cgroup-default GuestKit
staging/`run_nested_vm_test_binary` path. It must not create a new scenario,
lane, executor, source-copy launcher or host-root fallback. Until that exact
mapping and a retained safe receipt exist, S-ND295-36 is not closed.

**Disposition: OPEN — blocks approval.**

### F-02 — Blocker — E23 remains an unowned #295 pending marker without GuestKit evidence

**Kind:** `EVIDENCE/ACCEPTANCE`

The marker at
`crates/overdrive-worker/tests/integration/real_cgroup_fs/refused_thread_kill.rs:42`
still names `pending DELIVER step 12-01 (E23)`. The body is a real cgroup-v2
thread-refusal scenario at line 43: it creates an allocation scope, delegates
`pids`, caps `pids.max` at the current task count and calls
`CgroupManager::cgroup_kill`, requiring a typed `io::Error` or successful
write without panic. It is therefore not a documentation-only marker and is
not covered by the approved #325 exception.

The existing `CGROUP_WORKER` manifest at
`crates/overdrive-testing/bin/fwd_guest_suite.rs:790-830` does not select this
identity. The approved cgroup-default setup already enables the `pids`
controller at lines 126-158, so the smallest existing support need is one
additional exact row in that existing worker manifest, its exact count/raw
receipt and cleanup/skip ledger, followed by removal of this marker by the
owning workflow. That is test-support/DISTILL work; no new executor, group,
lane, production API or host-root execution is authorized. Until it is either
safe GuestKit-green with the marker removed or explicitly reconciled by the
owning upstream workflow, criterion 3 cannot be accepted.

**Disposition: OPEN — blocks approval.**

### F-03 — Blocker — the step-specific raw evidence and exclusion ledger are absent

**Kind:** `EVIDENCE/ACCEPTANCE`

The dispatch requires the full raw command, source-identity, result,
exclusion, terminal and cleanup ledger under
`.context/netns-density-295/14-04/`. That directory is absent at the reviewed
HEAD. The parent-to-HEAD implementation range contains only
`http_prober.rs` and `execution-log.json`; the DES GREEN and COMMIT events do
not identify which required package/test bodies ran, which native identities
were excluded, or whether any runtime skip was rejected. The crafter's
reported counts therefore cannot independently establish the step's required
coverage, even where an existing approved GuestKit group can supply the
underlying identity.

The smallest remediation is to retain the original crafter's already-run
safe userspace results and the existing cgroup-default117 / mtls-relay46
GuestKit receipts, with exact source identity, selected identities, exit
status, runtime-skip count and cleanup records, under the step-specific
evidence directory. Any missing required body must be routed through those
existing GuestKit manifests or returned to DISTILL; this finding authorizes
no new lane, executor, host-root fallback or duplicate test.

**Disposition: OPEN — blocks approval.**

## Review dimensions

| Dimension | Result | Evidence |
|---|---|---|
| Exact public API shape | PASS | Only private response disposal changed; no public surface was added. |
| Typed semantic errors | PASS | Existing status, transport and `ProbeFailure` representations remain unchanged. |
| HTTP status/body contract | PASS | Status is classified before body disposal; body bytes cannot alter the result. |
| Guest-stream lifecycle | PASS by source/dependency review | Hyper body drop closes incomplete H1 connections; no detached future or leaked owner is introduced. |
| Test integrity | PASS | No test file or assertion changed in the reviewed range. |
| Contract Shape compliance | PASS | Existing S-SVM-14 retains `bounded-change` and the required outcome anchor; no new test was authored. |
| Existing GuestKit reuse | PASS | cgroup-default117 and mtls-relay46 cover the mapped netns/veth, DNS, leg-C and dataplane identities; no duplicate lane is requested. |
| S-ND295-36 acceptance completeness | **BLOCKER** | Frozen positive guest-stack identities are absent at current HEAD and have no current safe receipt. |
| E23 marker completeness | **BLOCKER** | The real E23 body remains ignored and is omitted from the existing approved GuestKit manifest. |
| Step evidence retention | **BLOCKER** | No `.context/netns-density-295/14-04/` raw command/result/exclusion/cleanup ledger exists. |
| Scope / architecture | PASS | No bridge-era TCX/guard double-loss case, new detach hook, benchmark, mutation run or public mechanism was introduced. |

## Verification record

- Read the complete STEP 14-04 dispatch and the accepted probe/vsock ADRs.
- Compared the complete parent-to-HEAD diff. It contains only the private
  `http_prober.rs` correction and the three actual DES events.
- Verified the commit author, exactly one Codex trailer and Step-Id trailer on
  both commits.
- Audited all current `#[ignore = "pending DELIVER ..."]` markers. The only
  current markers are the two step-14-05 density bodies and E23 step-12-01;
  the two 14-05 markers are correctly future-step ownership, while E23 is an
  actual unresolved #295 body. The only approved enduring issue-linked marker
  is `pending #325` on `lease_conf_325`.
- Verified current source manifests for the existing cgroup-default and
  mtls-relay groups and matched the required netns/veth, DNS, leg-C and
  dataplane identities to those groups. No physical-host native test or
  cleanup was run.
- Confirmed the required step-specific evidence directory
  `.context/netns-density-295/14-04/` is absent. The reported userspace
  counts therefore remain claims without retained raw command, source,
  terminal, skip or cleanup evidence in this workspace.
- No mutation testing, KASAN, density benchmark, Lima run, host-root native
  run, host process kill, network cleanup or module operation was performed
  by this reviewer.

## Remediation disposition

No source remediation was performed. The HTTP change is contract-correct and
needs no implementation revision. The open findings are upstream evidence and
acceptance reconciliation: F-01 must return to DISTILL for the current exact
guest-stack body map and safe GuestKit evidence; F-02 must return to the E23
owner/DISTILL for existing cgroup-default staging and exact safe evidence; F-03
must retain the original safe command and exclusion ledger under the
step-specific evidence directory.
The original step crafter owns any implementation remediation after those
bounded support decisions, and the step reviewer must re-review the retained
receipts and marker audit.

## Verdict

**NEEDS_REVISION.** The 12-line HTTP correction itself matches the accepted
status-only bounded-body contract, preserves API/error/wire/telemetry shape,
and has no demonstrated lifecycle leak. Existing approved GuestKit groups
cover the exec/process, DNS, leg-C and Service dataplane identities, while the
userspace acceptance families have no step-specific raw receipt in the current
workspace. Approval is blocked by the absent current S-ND295-36 positive
guest-stack identities/evidence, the unhandled E23 pending marker and the
missing step-specific evidence ledger.

## Review iteration 2 — remediation re-review

### Cumulative boundary and review result

This re-review covers the complete cumulative step from approved parent
`92f0c60d4e3001dca7a233b7ec25979bafde1055` through `HEAD`
`58b877e4e0ac85eec323e8c80bb9eafde651fe19`, including the original
implementation commits `8e017a3e4bd9df4451203beab36b5ec733377aab` and
`db7629972071523535ddbb76983ef1f6e309dd9b`, and remediation commit
`ccb473c7182c754ceba1329cc469002cb62d9585`. The current cumulative source
diff is 13 files, 2,772 insertions and 73 deletions. The implementation and
follow-up commits preserve the Marcus author, exactly one
`Co-Authored-By: Codex <codex@openai.com>` trailer and `Step-Id: 14-04`;
their stats and owned paths were mechanically checked.

The remediation closes the three iteration-1 acceptance gaps with current
exact identities, retained raw receipts and a real E23 production correction.
The current native and userspace evidence is sufficient for the required
behavioral review. One mechanical DES-integrity finding remains open: three
mid-remediation `EXECUTED` events contain explanatory prose in `d`, although
the dispatch requires each such value to be exactly `PASS` or `FAIL`. This is
why the current verdict remains **NEEDS_REVISION**.

### Remediation evidence and safe execution boundary

The recovered evidence package is under
`.context/netns-density-295/14-04/`. It contains `commands.txt`, the
selection/exclusion ledger, source and terminal metadata, phase records,
complete output, byte-preserving raw output and the SHA-256 manifests for all
15 prior durable runs. The commands ledger records safe userspace commands
with `--no-sudo` and explicit native exclusions. No reviewer test or native
command was launched for this re-review.

The current preparation run `fcfe14d369a99ea0f2f887be` exited 0 and staged the
current control-plane, CLI and worker integration ELFs, kernel
`7.0.0-38-generic`, the x86_64 musl peer and `ovd_fwd.ko`. Its source digest
is
`ed245580352f2ca94174d6c21b3bdf209ffa444bbe5b4b7b0c7aea31ee788199`.
The current `cgroup-default` run `9c8fbc97ef29018ca5426e5c` used that digest,
ran inside the existing disposable production Cloud Hypervisor / VMM
GuestKit boundary and exited 0. Its raw output SHA is
`a86ff954fe6168af8b1d31d688e1070a2bcddec1cdeab90503c58f4781511d69`.

The fail-closed manifest accounted for exactly 119 bodies: 57 retained
baseline, 18 STEP 13-04, 5 STEP 13-05, 20 normal-native, 13 STEP 13-06,
one STEP 14-01 CLI and five STEP 14-01 control-plane bodies. All 119 started
and passed; the 109 emitted libtest summaries each report `ok`, zero failed
and zero ignored. No runner-recognized runtime-skip diagnostic occurred.
The E23 row passes at output lines 1935–1938, the existing mesh row passes at
1942–1945, the leg-C row passes at 2116–2121, the current V-6 row starts at
3106 and passes, and all five `guest_flow_through_serve` bodies pass. The
outer command, VMM cleanup, guest-network lease retirement and release all
completed with terminal exit 0. Idempotent absent-target cleanup statuses are
retained in the raw output and are covered by the cleanup ledger; they do not
represent a runner failure or host cleanup.

The retained qualified `mtls-relay` run `1e8aba12ce7f53f7be0560ff` remains
valid at exact 46/46, exit 0, with raw SHA
`2adfa9f4bb8488a42b6f00b3be222062718bbbe3caa7b4d0be9bcd05c36911ab`.
It supplies the existing mTLS inbound and Service dataplane selections whose
source bodies were unchanged by this step. The current source gate
`313bcce3eca0bf1407ffb343` used `--no-sudo`, exited 0, and passed format,
both integration-target checks and matching `-D warnings` Clippy checks; its
raw SHA is
`68a7410368cab10c340498246d09ebfacc9260f13946d8e925d3cb70fc8f64d7`.

### Required outcome and suite map

The current evidence now maps every required family to an exact existing
identity or safe userspace receipt. The mapping does not restore the removed
bridge/TAP attachment or the D19-LINKS bridge-era TCX-and-guard case.

| Required outcome or suite | Exact current identity / receipt | Current result |
|---|---|---|
| Frozen `guest_stack_mtls_egress` mesh reply and lifecycle outcome | Existing `integration::vsock_walking_skeleton::a_vm_workload_round_trips_tcp_and_udp_through_serve_and_deploy` at `crates/overdrive-cli/tests/integration/vsock_walking_skeleton.rs:52-247`, with exact public allocation identity and full lifecycle complement helpers at `:249-285`; selected as the existing STEP 14-01 CLI body | Current cgroup-default 119 pass. The body proves the current production `serve`/`deploy` composition, exact `SpiffeId::for_allocation` certificate identity, byte-exact TCP/UDP outcomes, probe descriptors and results, row field complement, terminal state and cleanup. |
| Frozen `guest_stack_mtls_egress` protected peer-wire outcome | Existing `integration::vsock_mesh_egress_mtls::registered_mesh_flow_reaches_its_registered_backend_over_mtls_without_leg_f_resolving_again` at `crates/overdrive-worker/tests/integration/vsock_mesh_egress_mtls.rs:1291-1349`, with live kTLS identity and positive splice source/destination checks at `:1099-1230` | Current cgroup-default 119 pass. The existing body performs two byte-distinct exchanges after establishment, requires one live TLS 1.3 software-TX/RX kTLS fd, requires that exact fd in both positive splice directions, observes application-data records without plaintext and proves no second resolve. |
| `service_kind_vm_workloads` | Existing CP acceptance selection `123d1d305a59c62621167c6a` and worker acceptance selection `fadba7c7f11b3c5f841961cc`; safe CLI subset `9caa18ff5d95626ec9416474` | CP 7/7, worker 7/7, CLI safe subset 2/2, all exit 0. The CLI cgroup-dependent exclusion remains explicit in the ledger; it is covered by the approved current GuestKit boundary where selected. |
| `cgroup_accounting_equivalence` | Existing host integration target, selection `47d5b8e4c15f2130fcfc0efe` | 2/2, exit 0, TempDir fixture only; no host cgroup state or VMM was opened by this userspace receipt. |
| `mtls_resolve_rekey` | Existing CP acceptance selection `123d1d305a59c62621167c6a` | Included in the exact 7/7 CP result, with all six rekey properties retained. |
| S-SVM-14 bounded HTTP probe | Existing worker acceptance selection `6bb340a111340465b09285c5` | 1/1, exit 0. The 1 MiB advertised/incomplete body still proves status-only bounded disposal; no body oracle or status mapping changed. |
| Exec/process netns and veth | Existing `CONTROL_PLANE_NORMAL_NATIVE` identities in `crates/overdrive-testing/bin/fwd_guest_suite.rs:1072-1153`, including the seven DNS rows and veth/netns provisioning rows | Current cgroup-default 119 pass includes all 20 normal-native bodies inside GuestKit. |
| Real DNS responder | Existing `integration::dns_responder_bind::*` rows at `fwd_guest_suite.rs:1084-1108` | All seven real bind/readback/wire rows pass in current cgroup-default; DNS semantic/codec userspace selection `49c18c3dacf978236f7a8056` is independently 32/32. |
| Leg-C inbound TPROXY / guest-prefix path | Existing `integration::vsock_constant_guest_prefix_rules::a_leg_c_diverted_remote_connection_reaches_leg_c_both_ways` at `fwd_guest_suite.rs:939-949`; existing mTLS/kTLS leg-C body in the qualified `mtls-relay` group | Constant-prefix leg-C passes in current cgroup-default output lines 2116–2121. The existing mTLS/kTLS body and Service dataplane body pass in mtls-relay exact 46/46. |
| Service dataplane selection | Existing `integration::service_map_forward::ten_tcp_syns_to_vip_are_rewritten_and_forwarded_via_veth` at `fwd_guest_suite.rs:581-584` | Pass in the qualified exact-46 mtls-relay GuestKit receipt; no host-root veth/XDP fallback or duplicate lane was used. |
| E23 refused-thread outcome | Existing `integration::real_cgroup_fs::refused_thread_kill::a_kill_write_under_thread_refusal_returns_an_io_error_and_never_aborts` at `fwd_guest_suite.rs:818-821` | Marker removed, exact row passes in current cgroup-default 119. The raw output records kernel pids refusal and reaches `ok`, rather than the pre-fix Tokio panic. |

The two E6 `guest_stack_mtls_egress::fault_fixture` failures remain the exact
pre-existing baseline names and errors recorded in the upstream-issues ledger.
They are not used to waive either positive guest-stack outcome, and no new
failure is present in the current required selections. The previous current
native failures are retained as truthful history: `0d228...` reproduced the
E23 Tokio panic, `2572...` exposed the one-direction splice/EINTR fixture
problem, and `5e5...` exposed the incorrect `Some(0)` expectation for an
operator-stopped Service. Their bounded fixture corrections are now exercised
by the exact-119 receipt. The historical third control-plane readiness failure
remains qualified as its observed one-shot `mesh_unreachable` after the
public readiness precondition; the current body does not claim future health
or map hydration, and the exact current receipt passes it.

### Production correctness, typed error mapping and lifecycle

The original HTTP correction remains correct. `probe_guest_stream` records
the response status and drops the Hyper response before the existing status
classifier at `crates/overdrive-worker/src/probe_runner/http_prober.rs:253-264`.
The accepted `HttpProber` trait and ADR-0054 require a status-only result with
the body discarded; no public API, error variant, wire field, telemetry event,
owner, retry rule or shutdown path changed. Hyper's existing incomplete-body
drop behavior closes the in-progress H1 connection, and no future is spawned
or detached.

The E23 remediation is a reachable production fix, not fixture-only
hardening. The real path is:

1. The existing E23 body creates a real allocation scope, applies the pids
   refusal and calls `CgroupManager::cgroup_kill` through `Arc<dyn CgroupFs>`.
2. `CgroupManager::cgroup_kill` resolves `<scope>/cgroup.kill`, awaits
   `self.fs.write(&path, b"1\n")` and maps only `NotFound` to `Ok`; all other
   `std::io::Error` values propagate at
   `crates/overdrive-worker/src/cgroup_manager.rs:263-280`.
3. The same owner path is used by VM start-failure cleanup at
   `crates/overdrive-worker/src/vm_driver.rs:1149-1155` and live-driver
   artifact cleanup at `:1480-1487`.
4. Before remediation, the disposable GuestKit kernel refused the next
   Tokio blocking-pool thread and `tokio::fs::write` panicked at
   `tokio-1.52.1/src/fs/mod.rs:317:11` in run `0d228...`. This was a bounded,
   reachable failure before the typed port result could return.

`RealCgroupFs::write` now calls the private
`write_on_fallible_thread` helper at
`crates/overdrive-host/src/cgroup_fs.rs:94-107,179-189`. The existing async
`CgroupFs::write` signature remains unchanged. The semantic mappings are:

| Reachable failure | Implemented approved representation | Completion / ownership |
|---|---|---|
| `std::thread::Builder::spawn` is refused by the allocation pids cap | The returned `std::io::Error` propagates through `CgroupFs::write` and `CgroupManager::cgroup_kill` | No task is detached; the async owner receives the error. |
| `std::fs::write` fails after a thread starts | Its existing `std::io::Error` propagates unchanged | The joined thread completes before `write` returns. |
| The write thread panics | Private `std::io::Error::other("cgroupfs write thread panicked")` | `join` completes before return; no panic escapes and no public error variant is invented. |

The current E23 receipt proves the first path reaches the typed result without
aborting. The joined handle preserves effect completion: `write` cannot
return while the cgroup write is still in flight, and cleanup callers already
await `cgroup_kill`. There is no new executor, detached future, public method,
error enum, retry mechanism or lifecycle owner. A speculative concern about
blocking the current async thread is not a finding without a reachable
production failure, and none is present in the current path or receipt.

The fixture changes are within the approved existing GuestKit support: the
E23 marker was removed only after its exact manifest row passed; the current
V-6 and mesh bodies retain their public composition and assertions; the
support review records all eight iterations and their corrections. No oracle
was weakened, no new test/scenario/lane/seam/model was authored, and no
bridge/TAP assumption or removed scenario ID was restored.

### Marker, baseline and formal-gate audit

The E23 `pending DELIVER step 12-01` marker is removed and its exact current
body is green. The two STEP 14-05 64-VM markers remain untouched and belong to
the next fresh crafter; they are not an early activation requirement. The
final feature-wide zero-pending audit still belongs after STEP 14-05. The
only enduring approved issue-linked marker is `cid_lease #325`. No density
benchmark (#326), KASAN, Lima or mutation run was performed. The retained
formal-model record `2228824-1793511799.752` with 279 checks in three jobs
remains valid and was not rerun.

### Iteration-1 finding dispositions

#### F-01 — CLOSED

The support review now gives the frozen two positive outcomes exact existing
current identities: V-6 for the current production composition, public
allocation identity and complete lifecycle row delta; and the existing
registered mesh flow for the live kTLS fd, two byte-distinct exchanges,
positive source and destination splice evidence and no re-resolution. The
current cgroup-default 119 receipt executes and passes both identities. This
closes the evidence gap without restoring bridge/TAP fixtures or creating a
new lane.

#### F-02 — CLOSED

The original E23 marker is removed. The exact existing worker manifest row is
at `fwd_guest_suite.rs:818-821`; the current cgroup-default runner accounts
for the baseline as 57 and passes all 119 bodies. The raw E23 output records
the kernel refusal and a successful typed-outcome assertion, with no runtime
skip or host fallback.

#### F-03 — CLOSED

The step-specific evidence directory is now present. `commands.txt`, raw
run directories, metadata, phase and terminal records, output hashes and the
selection/exclusion/cleanup ledger retain the exact commands, source digest,
native identity, result, exclusions and cleanup status. The final current
119 and retained exact-46 receipts are independently addressable by run ID
and SHA.

### F-04 — Blocker — `EXECUTED` DES events contain noncanonical data values

**Kind:** `DES / mechanical integrity`

The dispatch requires the `--data` value for every `--status EXECUTED` event
to be exactly `PASS` or `FAIL` and says that prose belongs in the report, not
the event (`.context/netns-density-295/dispatch-14-04.md:111-123`). The
append-only execution log contains three actual remediation events whose
`d` values are prose-prefixed failures:

| Log location | Actual event and receipt | Problem |
|---|---|---|
| `docs/feature/netns-density-295/deliver/execution-log.json:1931-1935` | RED, `2026-11-01T10:54:50Z`, run `0d228...` | `d` is `FAIL: current-source GuestKit run ...` rather than exactly `FAIL`. |
| `.../execution-log.json:1937-1942` | GREEN, `2026-11-01T11:24:58Z`, run `2572...` | `d` is `FAIL: current cgroup-default GuestKit run ...` rather than exactly `FAIL`. |
| `.../execution-log.json:1944-1949` | GREEN, `2026-11-01T11:55:00Z`, run `5e5...` | `d` is `FAIL: current cgroup-default GuestKit run ...` rather than exactly `FAIL`. |

The failure descriptions are truthful receipts and must be preserved. The
later GREEN at `:1951-1956` and COMMIT at `:1958-1963` have canonical `PASS`
values, so the current latest phase state is green, but that does not erase
the earlier noncanonical events. The installed `des-verify-integrity` run
does not report this because it validates the latest per-phase state and
instead reports unrelated historical gaps in steps 04-02, 04-03, 04-04 and
14-05. The dispatch's stricter exact-event rule still applies.

No reviewer may manually rewrite the append-only log, change a truthful
failure to `PASS`, invent a timestamp or fake a phase rerun. The smallest
resolution is for the original STEP 14-04 crafter, routed by the orchestrator,
to use an approved DES/tool-supported append-only correction if one exists,
or obtain and record an honest legacy-event audit disposition accepted by the
DES owner/orchestrator. That disposition must preserve the three real run
receipts and explain why the latest exact PASS events are authoritative. Until
that correction or disposition is recorded, the mechanical DES criterion is
not complete.

**Disposition: OPEN — blocks approval.**

### Iteration-2 verification record

- Read the complete current dispatch, feature-delta, accepted ADRs, frozen
  04-03 / S-ND295-36 suite map, current support review and all remediation
  evidence ledgers.
- Rechecked the cumulative parent-to-HEAD diff, exact commit trailers, stats,
  source gate, native preparation metadata, current 119 output and retained
  exact-46 output. No source or test command was rerun.
- Verified the current `RealCgroupFs::write` caller/owner path, typed
  `io::Error` mappings, joined effect completion and shutdown cleanup calls.
  The pre-fix E23 panic and current post-fix pass are both retained as
  bounded production evidence.
- Verified current manifest identities and exact count accounting for E23,
  V-6, the mesh same-fd body, DNS, netns/veth, leg-C, Service dataplane and
  `guest_flow_through_serve`. Existing mtls-relay coverage is retained without
  an unnecessary duplicate run.
- Verified marker ownership: E23 is removed, future 14-05 markers remain for
  the next step, and only `cid_lease #325` remains as an enduring issue-linked
  ignore. E6's two exact fixture failures remain baseline evidence.
- Verified the current raw SHA manifests and terminal records. No physical
  host native test, host process kill, network cleanup, module operation,
  Lima, KASAN, density benchmark or mutation run was performed.
- `des-verify-integrity` was run read-only; it reports unrelated historical
  feature-wide step gaps and does not reject the current latest 14-04 phase
  state. A direct inspection still finds the three noncanonical `d` values
  described in F-04, so the dispatch exactness rule remains unsatisfied.

### Iteration-2 remediation disposition

F-01, F-02 and F-03 are closed by the current exact support map, E23 pass and
durable evidence package. The HTTP and cgroup production changes satisfy the
approved API, typed-error, wire, telemetry, ownership, lifecycle and bounded
outcome contracts. F-04 is routed only to the original step crafter and DES
orchestrator for a tool-supported correction or honest audit disposition; it
does not authorize any source, test, model, design, roadmap or architecture
change. The reviewer will re-review after that bounded mechanical issue is
resolved.

## Iteration-2 verdict

**NEEDS_REVISION.** The substantive implementation and acceptance blockers
from iteration 1 are closed: current existing identities prove every required
guest-stack, exec/process, DNS, leg-C, E23 and Service dataplane outcome;
userspace receipts are retained; the current cgroup-default 119 and qualified
mtls-relay 46 native groups are green inside disposable GuestKit; and the
E23 production panic is now an approved typed `io::Error` path with joined
effect completion. Approval remains blocked solely by F-04: three truthful
mid-remediation `EXECUTED` DES events violate the dispatch's exact `PASS` /
`FAIL` data contract. The original crafter must resolve or explicitly audit
that append-only history before this step can be approved.

## Review iteration 3 — authorized F-04 repair re-review

### Scope and explicit authorization

This re-review is limited to the remaining F-04 DES recording finding. The
original executing crafter's commit is
`9b0bb15387de112ff2318604acb7384c69f24947`. Its execution-log diff changes
only the three malformed `d` values identified in F-04 from prose-prefixed
failure text to the canonical value `FAIL`; it does not add, remove, reorder
or rerun a phase. The user explicitly approved the one-time exception:

> Approve three-field repair

The complete request and approval provenance, including the exact original
strings, is retained in
`docs/feature/netns-density-295/deliver/des-audit-format-repair.md`. That
approval overrides append-only handling for those three fields only. It does
not authorize rewriting any other event, timestamp, phase history, raw
receipt, source, test or native result.

The current range also contains the separately authorized naming-policy
commit `db5b78a5f198d053cb52e65169bdee313d26e2b2`, which adds only
`.claude/rules/domain-naming.md` and tracks the migration in issue #333. Its
rule explicitly grandfathers current required schemas and filenames while
the tooling migration is pending. This is incidental documentation policy,
not an unreviewed production or test change; no historic evidence or
required filename is being rewritten for it.

### Repair integrity verification

The pre-repair execution log at the authorized repair base
`db5b78a5f198d053cb52e65169bdee313d26e2b2` has SHA-256
`2dcdd46c56b6b6213b7e02725dbe0ae512d9d74c28a116d526ff03417e7f5de1`. The
current post-repair log has SHA-256
`6603439ef608d058381366562d638f6141045fbb4ad7e29142d045a72ce4a489`. A
field-by-field comparison of the two parsed logs finds exactly three
differences, all at the approved selectors:

| Event selector | Previous value | Current value | Verification |
|---|---|---|---|
| `sid=14-04`, `p=RED`, `s=EXECUTED`, `t=2026-11-01T10:54:50Z` | Prose beginning `FAIL: current-source GuestKit run 0d228...` | `FAIL` | Only `d` changed. |
| `sid=14-04`, `p=GREEN`, `s=EXECUTED`, `t=2026-11-01T11:24:58Z` | Prose beginning `FAIL: current cgroup-default GuestKit run 2572...` | `FAIL` | Only `d` changed. |
| `sid=14-04`, `p=GREEN`, `s=EXECUTED`, `t=2026-11-01T11:55:00Z` | Prose beginning `FAIL: current cgroup-default GuestKit run 5e5...` | `FAIL` | Only `d` changed. |

The event count is 280 before and after. Every event's order, `sid`, phase,
status, timestamp and all unrelated fields are unchanged. Current
`execution-log.json:1930-1965` shows all eight 14-04 events in their original
order. All eight have `s=EXECUTED`; their `d` values are now exactly the
allowed canonical values `PASS` or `FAIL`, and the
terminal COMMIT event remains the original `PASS` at
`2026-11-01T14:05:23Z`. No phase event was appended or rerun.

The three original explanations remain available verbatim in the audit note,
alongside the raw receipts and Git history. The repair therefore makes the
current machine-readable event values conform to the dispatch without
erasing the reason each historical run failed.

### F-04 disposition — CLOSED

The exact dispatch requirement is now satisfied: every current `EXECUTED`
event has `d` exactly `PASS` or `FAIL`, and the three approved repairs are the
only event-field changes. The repair is within the user's explicit exception
and has independently verified provenance. F-04 is **CLOSED** and no DES
recording blocker remains for STEP 14-04.

### Global DES-integrity and sequencing audit

The read-only `des-verify-integrity` command still reports inherited
feature-wide gaps for frozen or separately owned steps: 04-02 lacks its
COMMIT entry, 04-03 and 04-04 have no current DES entries, and future 14-05
has no entries. Those results are unchanged by the three-field repair and are
not a STEP 14-04 finding. This review does not invent old events, timestamps
or frozen history, and it does not demand early 14-05 activation. The final
feature-wide zero-pending and marker audit remains sequenced after 14-05 as
required. The only enduring approved issue-linked ignore remains
`cid_lease #325`.

### Iteration-3 verification record

- Rechecked the repair commit author, exact Codex trailer, Step-Id trailer,
  owned diff and file stats. The repair commit changes only the three
  execution-log `d` fields and the audit note; no source, test, model, native
  receipt or phase event was added.
- Compared the complete pre-repair log from `db5b78a5f198d053cb52e65169bdee313d26e2b2`
  with the current log. The comparison found exactly three approved `d`
  changes and 280 events before and after.
- Verified the audit note preserves all three original failure strings and
  records the exact user approval. No original failure outcome was changed
  to `PASS`, and no timestamp or event order was altered.
- Verified the existing current-native 119 and qualified mtls-relay 46
  evidence remains unchanged and sufficient. No test, native command, source
  check, mutation run, KASAN run, benchmark or Lima run was performed for
  this metadata-only repair re-review.
- Preserved the user-dirty `AGENTS.md` and all other worktree state. The only
  new workspace artifact written by this reviewer is this review document.

### Iteration-3 remediation disposition

F-01, F-02 and F-03 remain closed from iteration 2. The explicit three-field
repair closes F-04 without changing phase outcomes, execution chronology,
native evidence, implementation behavior or accepted contracts. No further
remediation is required for STEP 14-04.

## Iteration-3 verdict

**APPROVED.** The complete cumulative STEP 14-04 review now has no open
findings. The status-only HTTP probe correction, the typed and joined E23
cgroup write path, the exact current GuestKit 119-body receipt, qualified
mtls-relay 46-body receipt, frozen positive-outcome mapping, marker ownership,
and retained userspace evidence satisfy the accepted step contract. The
three malformed DES failure-data fields were repaired under the user's
explicit one-time exception with exact three-field provenance; all 280 events,
timestamps, order, statuses and outcomes otherwise remain unchanged. Global
DES gaps in frozen 04-02/04-03/04-04 history and future 14-05 sequencing are
separate workflow state and do not reopen this approved step.
