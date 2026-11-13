# Final integration verification

Date: 2026-11-13

Feature: `service-allocation-readiness-and-ending`

Status: bounded evidence collection complete; final feature rechecks and
independent remediation reviews remain pending.

## Source and evidence boundary

The full workspace diagnostic run uses a detached, isolated checkout at
`dca2350aad3a9dc5994e1788605fb4903fa8c127`, the final approved roadmap
step review. Later checks use a second isolated checkout; each result below
names its source commit. The shared working tree contained unrelated dirty
`AGENTS.md`, `netns-density-295`, `fwd_pair.qnt` and spike work, none of which
was staged, reset or included in either checkout. The Metal runner reports its
own absent `vendors/linux` submodule and `.overdrive-metal-checkout.v1`
marker in the standalone remote Git materialization; the local checkout's
tracked feature source is the named commit. Native runs used the canonical
foreground `cargo xtask metal run --` controller and completed through its
durable run ID.

All 17 step reviews exist as native Markdown and end in `APPROVED`.
`PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity
docs/feature/service-allocation-readiness-and-ending/deliver/` exited 0:
**all 17 steps have complete DES traces**. No final-gate agent wrote a DES
phase event. The user explicitly waived mutation testing for this final wave;
the unchanged behavioral invariants are still exercised by the accepted
step evidence and the checks below.

## Native product journey already reviewed

The [step 05-07 verification](step-05-07-verification.md) and its
[independent review](review-05-07.md) retain the D15 black-box proof through
the built default-feature `overdrive` binary and the checked-in
`examples/service-kind-vm-workloads/zero-probes*.toml` Service specs. The
positive Service became `Stable` in 1,363 ms and `workload describe` showed
`startup probe[0] guest listeners tcp:18081 last=pass`. The negative Service
reported `tcp:18998` missing after each 60-second startup window, exhausted
the restart budget across six allocation rows, and a byte-identical submit
returned `StartupProbeFailed` with that missing listener. The positive and
negative owned cleanup delta records report zero remaining VM, network,
cgroup, run-directory, mount and loop resources. The retained raw captures
are under `.context/step-05-07-native/rebased-20261112/{positive,negative-retry}/`;
the step review independently approved the proof. Commits after the product
capture through `dca2350aa` changed only the step record, review and TOML
comments, not those operator inputs or production behavior.

The historical
`verification/expectations/E13-vm-service-inferred-tcp-startup/` remains a
SHA-pinned record of the former inferred-TCP claim. D16 explicitly preserves
that point-in-time record, and `.claude/rules/verification.md` prohibits
overwriting its evidence as a claim about the later D15 product. E13 is
therefore unchanged and is not cited as proof of current default startup
behavior.

## Final checks

| Check | Source and result | Raw receipt |
|---|---|---|
| BPF object build | `dca2350aa`: PASS; dataplane and guest objects built. | `/private/tmp/service-readiness-final-bpf-build.log`, Metal `dc79462f73d0c8566d506d34` |
| Forwarder module build and load | `dca2350aa`: PASS; module version `e2b939e45cb106448fa4d8f8ca11e52f990bf1127c8af80ed09b2e3c6299f3bc` already loaded. | `/private/tmp/service-readiness-final-kmod-{build,load}.log`, Metal `fbb9458291b5933f888b2bdc` and `d9d360ba1ad64d58d87b4b81` |
| Guest kit preparation | `dca2350aa`: PASS; 4,437 runtime inputs staged for kernel `7.0.0-38-generic`. | `/private/tmp/service-readiness-final-guest-kit.log`, Metal `dc0288c390169b4006e11a3a` |
| Other required guest-flow and beacon controls, serial native lane | `dca2350aa`: **209 passed, 1 failed, 3 skipped** of 210 after excluding only the exact user-authorized T-5 and D-22 identities. The sole additional failure was the direct Fast Open correlation cell's one-second row bound. An exact isolated replay then passed 1/1; the serial suite is still recorded as failed. | `/private/tmp/service-readiness-final-guest-controls.log`, Metal `623b9cf022d67db8c20cbe99`, Nextest `a012615c-d52f-45ea-a8d1-30940afee717`; focused `/private/tmp/service-readiness-final-tcp-correlation-focused.log`, Metal `011530b271c2abac7245b6f8` |
| Quint typecheck | `f13990f43`: PASS, 53 files. | `/private/tmp/service-readiness-final-quint-typecheck.log`, Metal `35a3c8a4a46cedcaec431a38` |
| Feature Quint model checks | `f13990f43`: PASS, 84/84 checks; wall clock 167.7 s. | `/private/tmp/service-readiness-final-quint-check.log`, Metal `478b1de09c9a5bb185d9021a` |
| Recorded Quint evidence | `f13990f43`: PASS; the feature evidence matches its spec and every recorded outcome met `expect`. | `/private/tmp/service-readiness-final-quint-evidence.log`, Metal `0e9064f5fc6375d7e6c393bb` |
| Quint conformance lint | `f13990f43`: PASS. | `/private/tmp/service-readiness-final-conformance-lint.log`, Metal `b9e6c99a1860cd0adaa08526` |
| Schema evolution | `f13990f43`: PASS, 48/48; 13 skipped by the existing target selection. | `/private/tmp/service-readiness-final-schema.log`, Metal `92686da911d2fc0f6b936acf` |
| Feature crate formatting | `f13990f43`: PASS for all 11 crates changed by this feature. | `/private/tmp/service-readiness-final-fmt-feature.log`, Metal `621a7f2719da0bce789cd01b` |
| Whole workspace formatting | `f13990f43`: FAIL solely at `xtask/src/metal_run_isolation_conformance.rs:519`, unchanged from `origin/main`. No feature crate appears in the diff. | `/private/tmp/service-readiness-final-fmt.log`, Metal `cb37ff3f44b5c6ce91cd63ef` |
| DST lint | Initial `f13990f43`: FAIL with five async fixture calls and a stale Earned-Trust helper-name scanner; after bounded test and scanner commits, `9b387cb26`: PASS. | `/private/tmp/service-readiness-final-dst-lint-{,after}.log`, Metal `ae3dac63ba6e4596d52154e9` and `a0862783ef26ae7015a8d12d` |
| OpenAPI live-schema check | Initial `f13990f43`: FAIL on stale TCP default text; after D15 documentation/schema correction, `d73862622`: PASS. | `/private/tmp/service-readiness-final-openapi-{check,after}.log`, Metal `92e992ed88053e8f155b815b` and `9a5fe8ce700745a9ba20fd0b` |
| Strict workspace Clippy | `f13990f43`: FAIL on the private D17 `EvaluationSubmission::Standalone` dead-code warning. Its bounded fix landed at `ff29358a8`; the 05-02 scanner follow-up landed at `45086aece`. A rerun at `b0881ff68` still FAILS solely on Clippy's `doc_markdown` warning for the mandatory exact `/// CONTRACT_SHAPE: pure-function.` line in the D10 CLI acceptance case. Bounded DISTILL correction and review remain pending. | `/private/tmp/service-readiness-final-clippy.log`, Metal `ec372bc2d31fcd5529884f21`; `/private/tmp/service-readiness-final-clippy-after.log`, Metal `a4d01646a15ccf21e9b248ca` |
| Full native workspace Nextest, including `integration-tests,kvm-tests` | `dca2350aa`: **FAIL**, 4,643 run: 4,512 passed, 129 failed, 2 timed out, 27 skipped; 5,906.628 s. This is a diagnostic union of all workspace and guest lanes on the pre-remediation source, never a global-green claim. | `/private/tmp/service-readiness-final-nextest-durable.log`, Metal `f960aca3828470b7c7249467`, Nextest `e995fb1f-20c5-445c-bb3b-3ebeb8872357` |

The broad native workspace run is a diagnostic union of all crate and guest
lanes. Its initial build attempt failed before tests because this new isolated
Metal checkout lacked `target/bpf/overdrive_bpf.o`; the successful BPF,
forwarder and guest-kit preparation above preceded the actual 4,643-test
invocation. The initial pre-test failure is not counted as a test result. The
local `tee` briefly ran out of disk space during the long run. The completed
durable Metal run was collected again by its run ID; the complete receipt
named above has the final Nextest summary and all failure output.

## Broad-suite findings and limits

The broad run uses `--no-fail-fast` and is retained truthfully as a workspace
diagnostic. Its 131 failed or timed-out cases span the current feature, older
features and unrelated native fixtures. The 65 nonpassing `overdrive-cli`
integration cases include VM guest fixtures that abort at
`SystemdDetectVirtUnavailable` in the guest before their product assertions;
the two `overdrive-cli` and control-plane integration timeouts are recorded
separately in the durable summary. This result cannot be used as a passing
workspace gate or as evidence that every failure is caused by this feature.

The final Nextest summary retains every failed identity. The nonpassing cases
are distributed as follows; the table is an inventory, not a blanket waiver
or a claim that every case in a group has the same cause:

| Test binary or package | Failed | Timed out |
|---|---:|---:|
| `overdrive-cli::integration` | 64 | 1 |
| `overdrive-control-plane` library | 23 | 0 |
| `overdrive-control-plane::acceptance` | 4 | 0 |
| `overdrive-control-plane::integration` | 2 | 1 |
| `overdrive-bpf::integration` | 6 | 0 |
| `overdrive-sim::integration` | 6 | 0 |
| Other `overdrive-sim` spike binaries | 8 | 0 |
| `overdrive-init` library and integration binaries | 4 | 0 |
| `xtask` binaries | 6 | 0 |
| `overdrive-fwd`, `overdrive-host`, `overdrive-gateway` integration binaries | 5 | 0 |
| `overdrive-cli::acceptance` | 1 | 0 |
| **Total** | **129** | **2** |

The timed-out identities were
`overdrive-cli::integration::walking_skeleton::walking_skeleton_ws1_ws2_ws3_post_adr_0020_wire_shape`
and
`overdrive-control-plane::integration::guest_flow_through_serve::the_forwarder_inventory_counts_every_held_guest_flow_through_serve`.
The durable receipt is the exact identity and failure-text record for all 131
nonpasses. Most have not been individually adjudicated against a clean
`origin/main` run; they remain failed or timed out, never waived by grouping.

The current-feature failures were isolated from that diagnostic run through
bounded focused Metal selections:

| Affected evidence | Focused reproduction and contract relationship |
|---|---|
| D9 hook ownership | Three `terminal_hook_ownership` cases failed 3/3 at pre-Stable Liveness-live assertions, Metal `0e88b6e07d6ba9b1f44ab817`. D9 starts Liveness after committed Stable; the fixture never commits Stable. The D1/D4 terminal hook and sibling-row assertions remain required. |
| D9 probe-count composition | A named Quint case failed at `liveness=None`, `readiness=(1, Fail, 1)`, seed `0x685a885f`, Metal `15bc6abc4f499e88a29f8ddb`. All 11 source-local probe-count cases and 10 Quint probe-count cases failed in the broad run because the fixture's first Stable look did not drive the D9 `on_alloc_stable` hook into the ProbeRunner. |
| D10 Service progress deadline | The older cap-timer acceptance case failed 1/1, Metal `4b2b6db7c720bf96ec2c3c6b`: it advances a 1-second floor by 2 seconds and awaits Timeout, while D10's Service deadline is at least startup deadline plus 30 seconds. The separately reviewed focused correction at commit `530b64ae6` also fixed the stale CLI Timeout wording assertion that failed in the broad run. |
| D11 stream snapshot | The randomized conformance case failed reproducibly with seed `0x43d4164d`, Metal `89146bf03270fe366989b44b`. After event 0's ignored startup stop and event 1's give-up are both queued, one production poll drains both and yields the correct give-up, while the DISTILL model-driver mapping treats `Deliver(0)` as one externally visible consumption step and remains open. No accepted public contract pins one poll to one queued event; this is a test-support mapping issue for DISTILL to adjudicate. |
| D7/D9 backend projection | The accepted feature delta explicitly assigns revision of `service_backend_projection.rs`'s old startup/finalization/eligibility oracles. The full module's focused native Sim selection reproduced 6 passed, 6 failed, Metal `d787256257caf5df86d334f4`: four expected a healthy backend before committed Stable, one expected the old deciding write, and one expected a pre-Stable liveness restart trajectory. |

The unrelated older-feature `fwd_pair` T-5 conformance identity and D-22 owner
burst identities also failed in the diagnostic union; their exact accepted
current-feature nonblocking disposition remains in
[step 05-06 verification](step-05-06-verification.md). Other failed
experimental spike or native fixture cases are retained by name and output in
the durable log; none is silently called green. The six `overdrive-sim`
Service-backend projection failures above are a current-feature DISTILL
obligation because the accepted design named that file. No production or
test-support change was made by this final integration verifier.

The serial guest-flow/beacon rerun above establishes 209 required controls in
one run, while the remaining Fast Open control failed there and passed in an
exact focused replay. Its intermittent one-second correlation-row result is
not one of the two authorized external exceptions; a separate focused pass
does not turn the failed serial suite green.

## Pending bounded handoffs

The accepted D9/D10 test-oracle corrections were committed at `e4635dd3f`;
the D11 stream-snapshot model-driver mapping and the six Sim backend-projection
cases are with their DISTILL owners. The private 03-02 Clippy fallout was
committed at `ff29358a8`, the 05-02 Earned-Trust lint scanner and follow-up
at `b23de5ca7` and `45086aece`, and the 05-07 D15 OpenAPI descriptions at
`d727375c6` and `d73862622`. Their independent follow-up reviews and the
post-remediation feature selections remain to be recorded. This report does
not mark any pending reviewer verdict or final focused test as passed.

No expectation runner was invoked for E13, no Rust integration test launched
the built production binary, and no verification expectation imported an
Overdrive crate or duplicated the internal acceptance assertions.
