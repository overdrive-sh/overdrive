# DELIVER review — step 05-06

## Metadata

| Item | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Step | `05-06` — Guest listen hooks, ordered forwarding and workload cgroup |
| Iteration | 2 — final; iteration 1 is recorded below |
| Implementation commit | `d4a5eb3a5252319d16eaf7b393ac5fa4c8313358` |
| COMMIT audit | `5e5562591b5a65a5d9637d952e23dc8d31093559` |
| Lint-remediation commit | `a6f428d292dd904cf4027c33e4c49e34eb85de0f` |
| Review scope | D13 guest hook programs, workload-cgroup attachment and placement, beacon ordering, real-boot acceptance, guest-lane fixture, and the existing verifier-regress gate extension |
| Working tree | Iteration 1 preserved three unrelated dirty `netns-density-295` files. During iteration 2, in-progress 05-07 changes were also present; all remained untouched and outside this review. |

## Verdict

**APPROVED — iteration 2 (final).** The implementation matches D13's accepted guest-side contract and step 05-06's bounded evidence obligations. The two required native Tier-3 tests pass, the three named guest programs have their own measured verifier counts and 500,000-instruction limit, and the existing dataplane verifier corpus still passes. The two externally owned failure modes remain explicitly failed or skipped and are not represented as green; the user's scoped direction permits this step to proceed without waiting for those fixes. A later workspace Clippy diagnostic on the two exact test-contract rustdoc lines was closed by a per-test lint allowance, as recorded in iteration 2 below.

`praise:` The implementation keeps listener observation in the existing guest eBPF object, makes workload placement atomic with the hooks through `CLONE_INTO_CGROUP`, and keeps all beacon writes on the existing single `&mut File` path. The native tests exercise the real `run_server` composition and production `overdrive-init` guest rather than a test-installed hook or board.

Iteration 1 had no accepted findings. Iteration 2 reviews only the two per-test lint allowances in `verifier_budget.rs`; no D13 behavior, tests, models, DES events, or in-progress 05-07 work were changed by this reviewer.

## Contract review

### Hook shape, scope, and workload placement

The accepted D13 contract requires a separate TCP `TCP_LISTEN_CB` program and two UDP `post_bind4` / `post_bind6` programs in the already loaded guest object. The TCP arm reports only on the listen callback and excludes IPv4, IPv6, and v4-mapped loopback (`crates/overdrive-bpf/src/programs/listen.rs:59`, `:79`, `:88`). Both UDP programs filter protocol 17, exclude loopback, and return 1; the TCP program also returns 1. The IPv4 byte-order mask and IPv6/v4-mapped comparisons match the accepted D13 item 8 representation (`listen.rs:45`, `:53`). The maps use the accepted ring-buffer/drop-counter mechanism (`listen.rs:24`, `:27`, `:38`).

`KernelGuestTransportOps::prepare_guest_programs` loads the existing embedded guest object, initializes its maps, and creates one workload cgroup before program attachment (`crates/overdrive-init/src/transport/kernel.rs:316`). `program_cgroup_file` directs only the three listener programs to that child cgroup; the existing programs continue to use the root cgroup (`kernel.rs:247`, `:254`). The private `GuestProgram::ALL` list adds exactly `ListenTcp`, `ListenUdp4`, and `ListenUdp6` (`crates/overdrive-init/src/transport/mod.rs:445`, `:469`). The production attachment uses the existing Aya link path. Linux 7.0's cgroup link creation forces `BPF_F_ALLOW_MULTI` (`vendors/linux/kernel/bpf/cgroup.c:1506`), so the root SockOps link remains effective alongside the child SockOps listener hook as required by P-b.

The workload is created directly in the child cgroup with `clone3` and `CLONE_INTO_CGROUP`; argv preparation happens before the syscall and the child then creates the process group and calls `execvp` (`crates/overdrive-init/src/workload.rs:14`, `:31`, `:34`, `:62`). The production owner path transfers the listener runtime before `READY`, accepts `EXEC`, invokes this spawn path, supervises the child, drains reports, sends `EXIT`, then powers off (`crates/overdrive-init/src/main.rs:122`, `:129`, `:141`, `:143`, `:144`, `:192`).

### Beacon order, deduplication, and failure mapping

PID 1 owns the report set as a `BTreeSet<(Proto, NonZeroU16)>`. It drains the ring during supervision and once after the process group has completed; `drain_listener_reports` sends only when insertion into that set succeeds, then the existing lifecycle sends `EXIT` (`main.rs:805`, `:812`, `:880`, `:900`). There is no forwarding thread or second writer. A ring-drop count remains guest-readable and is logged after the final drain (`main.rs:913`; `kernel.rs:131`, `:146`).

The DELIVER note's process-creation mapping is implemented: a `clone3` failure returns through the existing `InitError::Spawn` representation, while `execvp` failure exits with status 127 (`workload.rs:31`, `:65`, `:77`; `main.rs:784`). The new `GuestProgram`, `GuestListenerRuntime`, and `spawn_in_cgroup` surfaces are crate-private or live under private modules; no new cross-crate API, configuration, public error, or wire variant is introduced by this step. The additional `GuestTransportSetupError` cases describe internal setup failures and reach the existing pre-READY `InitError::GuestTransportSetup` path; they do not substitute strings for a designed caller-branching error contract.

### Real-boot acceptance and test integrity

The two pre-authored D13 native tests are activated by removing their pending markers. The positive scenario submits a Service with explicit `GuestListeners`, runs the actual server and production guest, observes `Pass` while `Running`, and verifies the workload's exact TCP and UDP bind lines. It holds the production console file open across terminal cleanup, so the assertion reads the same inode after the VM run directory is removed (`crates/overdrive-control-plane/tests/integration/guest_listener_real_boot.rs:323`, `:330`, `:342`, `:362`, `:382`). The negative scenario covers `127.0.0.1`, `::1`, and `::ffff:127.0.0.1` with a wildcard control listener (`guest_listener_real_boot.rs:392`, `:399`, `:405`). The reviewed DISTILL fixture correction changes the console observation lifetime and the test lane's beacon parser; it keeps the existing outcome assertions and adds no production seam (`distill/guest-listener-real-boot-fixture-remediation.md`).

The existing in-guest test lane now accepts only a valid `LISTEN` message before `EXIT`, using one total deadline and preserving failures for malformed or unexpected frames (`crates/overdrive-testing/src/guest_lane.rs:583`). The transport setup-order assertion changes from 1,050 to 1,056 operations: the three new programs add exactly three load and three attach calls. `GuestProgram::ALL`, the independent test list, labels, and ordered-call oracle all include those same three programs (`crates/overdrive-init/src/transport/tests.rs:59`, `:737`, `:744`). No assertion was removed, skipped, broadened, or relaxed to make the implementation pass.

Two new verifier decision/parser tests carry `/// CONTRACT_SHAPE: pure-function.`; the transitioned real-boot tests retain `CONTRACT_SHAPE: bounded-change.` (`crates/overdrive-dataplane/bin/verifier_budget.rs`, tests added by the implementation commit; `guest_listener_real_boot.rs:331`, `:400`). The two new unit methods cover two distinct pure behaviors, within the `2 × 2 = 4` method budget. The two real-boot scenarios are Tier-3 integration evidence, not additional unit methods.

## Verifier-gate and baseline review

The existing `verifier-regress` command now loads both named objects, measures the existing dataplane baseline set and separately loads the guest object to measure exactly `listen_tcp`, `post_bind4`, and `post_bind6` (`crates/overdrive-dataplane/bin/verifier_regress.rs:53`, `:69`, `:90`, `:127`, `:368`). Each guest program must produce a measurement and remain at or below 500,000 verified instructions; the gate also requires an object-matched initial baseline (`verifier_regress.rs:156`, `:180`). The pure evaluator joins on both object and program (`crates/overdrive-dataplane/bin/verifier_budget.rs:211`), preventing a dataplane program with the same name from satisfying a guest baseline. `GatePolicy::default` retains the existing 5% plus 50-instruction growth policy and 1,000,000-instruction ceiling with 10% proximity threshold (`verifier_budget.rs:132`); no existing dataplane budget was weakened.

The initial guest baseline records the pinned Linux 7.0 kernel, exact guest object, three program names, and measured values `91`, `31`, and `70` (`perf-baseline/main/verifier-budget/veristat-guest-listeners.txt`). The first gate run failed because those initial entries were absent; the final canonical exclusive run then passed all nine baseline entries. This correctly separates measurement from the initial-baseline failure and from the final gate result (`deliver/step-05-06-verification.md:36`, `:37`).

## TDD, quality gates, and verification

| Gate | Result | Evidence and limit |
|---|---|---|
| RED → GREEN → COMMIT | **PASS, scoped** | `execution-log.json` records RED/FAIL, eight intermediate GREEN/FAIL outcomes, final GREEN/PASS, then COMMIT/PASS in order. The verification record identifies the failed external identities and distinguishes them from the final required gate. |
| G1 — required acceptance activation | **PASS** | Both required pre-authored real-boot outcomes for this step are active; no unrelated pending scenario was activated. |
| G2 — semantic RED | **PASS as recorded** | The 05-06 RED event is recorded as FAIL before implementation. This reviewer did not rerun the native tests. |
| G3–G4 — unit RED and boundary doubles | **N/A / PASS** | No new production unit-test method or in-hexagon mock was added. The production behavior is exercised through native real-boot composition. |
| G5 — business-language outcomes | **PASS** | The Tier-3 scenarios assert listener readiness, loopback rejection, and production allocation rows. |
| G6–G7 — final green and pre-commit | **PASS with explicit external failures** | Both D13 native tests and the final verifier gate passed. The serial init run had 210 passes, one known D22 owner-burst failure, and two approved external identities skipped; none is reported as green. |
| G8 — unit-test budget | **PASS** | Two new unit methods for two pure gate/parser behaviors; maximum four. |
| G9 — test integrity | **PASS** | Test changes add expected outcomes and the exact six operation-count increments. No unrelated assertion was weakened or removed. |
| Contract Shape Compliance | **PASS** | New pure-function verifier tests and transitioned bounded-change Tier-3 tests carry their required declarations. No new source-local Rust property was added without the exact required declaration. |
| External validity | **PASS** | The real-boot scenarios call production `run_server`, boot the guest kit, and exercise production `overdrive-init`; the verifier gate loads the built ELF directly. Rust tests do not spawn the built Overdrive production binary or act as an expectation runner. |
| Typed error and interface audit | **PASS** | No designed public typed-error set is extended. The approved private `clone3`/exec mapping is present and internal setup errors remain in the existing pre-READY failure path. |
| RPP | **PASS** | L1–L2 review found no actionable step-specific defect; no broader refactor is requested. |
| Mutation gate | **Not run** | Correctly reserved for the single final DELIVER-wave gate. |

Additional retained verification: guest-kit preparation and native BPF build passed on Linux `7.0.0-38-generic`; verifier-harness tests passed 13/13; scoped formatter and strict Clippy passed (`deliver/step-05-06-verification.md:29`, `:30`, `:38`, `:39`). These are retained results, not independently rerun by this reviewer. The step verification record has intentional Markdown hard-break spaces on its Date and Crafter metadata lines; no source-code whitespace issue was found in the implementation diff.

## External failures and scope disposition

The watch-mode `quint_fwd_pair_pair_conf_guest` mismatch and the D22 one-second `every burst flow ended` failure through the outer and direct owner-cell identities are recorded as unresolved external dependencies. The current verification record includes the two Quint seeds, the exact D22 assertion and current count, the preimplementation baseline, and the serial-suite outcome (`deliver/step-05-06-verification.md:9`, `:11`, `:12`, `:32`, `:33`, `:34`). They are not D13 listener behavior and are not called green. The approved 05-06 scope records the user's “no, u dont need to wait for anything” direction; this review neither waits for those fixes nor alters their older contracts.

The parallel Fast Open child-correlation timeout passed in isolation and in the full serial run. The G-10 init-bind scenario remains waived under DG-13; the mapped non-loopback pass case, tight drain race, V39, P-12, and GH #335/#336/#340 remain outside this step as recorded in the approved roadmap. No new failure mode was found that meets the repository's production-reachability and reproduction gate.

## Findings and remediation dispositions

Iteration 1 accepted no findings. Iteration 2's lint finding and remediation are recorded below and are closed. The external failure modes above retain their approved nonblocking disposition for this step; they are not approval findings or green test results.

## Iteration 1 disposition

**APPROVED.** Step 05-06 satisfies its accepted D13 contract and verification criteria within the recorded, user-approved external-failure scope. The next roadmap step may proceed.

## Iteration 2 — verifier-test rustdoc lint follow-up

### D1 — `issue (blocking):` exact Contract Shape declarations triggered `doc_markdown`

**Severity:** Blocking for the reported strict Clippy gate; closed by the original crafter

**Locations:** `crates/overdrive-dataplane/bin/verifier_budget.rs:599`, `:639` before remediation; `:599`, `:640` after the first inserted allowance shifts the second test

**Disposition:** Closed by remediation commit `a6f428d292dd904cf4027c33e4c49e34eb85de0f`

`doc_markdown` flagged each exact `/// CONTRACT_SHAPE: pure-function.` rustdoc line. Those declarations are mandatory machine-readable test metadata, so changing their text or formatting to satisfy the lint would violate the test-contract rule. The crafter added `#[allow(clippy::doc_markdown, reason = "exact per-test contract declaration")]` directly to the two affected test functions (`verifier_budget.rs:600`, `:641`). The lint is suppressed only for those function items; there is no module-, file-, crate-, or workspace-level allowance and no suppression for other Clippy lints. The exact rustdoc declarations remain unchanged.

`git show a6f428d29` confirms that the remediation changes only `verifier_budget.rs` and adds exactly those two item-level attributes. Their positions associate each allowance with the test whose mandated rustdoc triggers the warning. No production behavior, test assertion, verifier threshold, API, or budget changed.

### Iteration 2 verification and disposition

| Check | Result | Evidence and limit |
|---|---|---|
| Targeted all-target `overdrive-dataplane` Clippy with warnings denied | **PASS** | Original crafter's retained result after `a6f428d29`; this reviewer did not rerun the command. |
| Package formatting | **PASS** | Original crafter's retained result after `a6f428d29`; no formatter or test-contract text change. |
| Remediation diff whitespace check | **PASS** | Original crafter's retained `git diff --check` result. |
| Commit attribution | **PASS** | `a6f428d29` retains Marcus as author and has exactly `Co-Authored-By: Codex <codex@openai.com>`; no Claude or Anthropic attribution. |
| D13 native tests, guest verifier gate, or other step checks | **Not rerun** | The remediation is limited to lint attributes; the iteration-1 behavioral and native evidence remains applicable. |

No finding remains open. **Iteration 2 verdict: APPROVED.** The iteration-1 verdict remains valid, and step 05-06 is finally approved.
