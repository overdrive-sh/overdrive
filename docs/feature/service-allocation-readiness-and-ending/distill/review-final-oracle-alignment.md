# Independent DISTILL review — final D7/D9/D10/D12 oracle alignment

## Metadata and verdict

| Item | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Scope | Test and fixture alignment in commits `e4635dd3fb536ba71d3034ce180ee7e4f3475f5b` and `5cce053c3eafdca686bf2e8756d459da6e6885d1` |
| Authority | Approved feature-delta D7, D9, D10 and D12; ADR-0259, ADR-0261, ADR-0262, ADR-0264 and ADR-0267 |
| Related evidence | `distill/final-oracle-alignment.md` and `distill/final-backend-projection-remediation.md` |
| Verdict history | Iteration 1: **NEEDS_REVISION**; iteration 2: **APPROVED** after F-01 was closed |

## Contract mapping and test integrity

| Accepted rule | Reviewed mapping |
|---|---|
| D7 / ADR-0259 | Startup probe exhaustion stops the attempt as `Terminated` with `Stopped { by: StartupProbe }`; `WorkloadLifecycle` owns replacement and any later give-up. The backend projection startup-failure trajectory now drives the stop and checks the typed terminal, predecessor identity, fresh successor and ineligible successor backend. The prior `FinalizeFailed` expectation was obsolete. |
| D9 / ADR-0261 | Liveness starts only after the allocation's committed `Stable` row and `Driver::on_alloc_stable`. The hook ownership fixtures dispatch that Stable row through the production shim before expecting live liveness. Their test-local driver mirrors the production `VmDriver` order: retire Startup, then start Liveness. The probe-count fixture forwards the existing Stable hook through its private driver wrapper; it still uses the real `ProbeRunner` and the same ten conformance identities and seeds. The natural-exit expected snapshot now clears the prior Stable terminal, matching the observer's new Failed row; the focused corrected test passed. |
| D10 / ADR-0262 and ADR-0264 | The Service handler test submits the same validated spec it uses to derive `progress_deadline(startup_probes, streaming_cap)`, advances `SimClock` beyond that deadline, and strengthens the existing typed Timeout check to the exact `after_seconds`. The no-startup-probe case has the approved 90-second derived bound even with a one-second construction floor. The test's response-line and absent verdict-allocation checks remain. |
| D12 / ADR-0267 | A startup-probed allocation is backend-ineligible until its **own** Stable row commits, then readiness applies. The revised backend trajectories check pre-Stable unhealthy rows, committed Stable before health, post-Stable readiness withdrawal, first-write rejection and repair, exact row stamps, listener correlations, changed-row/hydrator ordering before a typed Liveness stop, persisted View reload, replacement identity and fail-closed successor membership. The old predecessor veto and veto-before-startup-stop order were removed by the approved contract; the revised assertions do not revive them. All 12 existing module identities remain, including the six controls that already passed before this correction. |

The backend scenarios drive `ServiceLifecycle`, `WorkloadLifecycle`, `ProbeRunner`, the production convergence tick and shim, and inspect their resulting observation rows and handoffs. The hook ownership tests intentionally use the shim's existing action port to check its write-before-hook boundary. The changed files contain no production behavior change, public API, new test seam, model edit, skipped test or removed seed. `probe_count_fixture.rs` is compiled only under `#[cfg(all(test, feature = "integration-tests"))]`. The CLI's function-local `clippy::doc_markdown` allowance is justified by the required exact machine-read `/// CONTRACT_SHAPE: pure-function.` line; it leaves every variant assertion unchanged. The separate D11 randomized stream-snapshot mapping remains outside these commits and this verdict.

## Verification and limits

| Evidence | Result |
|---|---|
| Focused D9/D10 Metal selection, run `b482a5e4f8d02acc81b5a8d6` | I independently collected the durable run: **20 passed, 2 failed, 22 selected**. The failures were the then-stale natural-exit expected Stable claim and the independent D11 `quint_stream_snapshot_randomized` mapping. This run is **not** wholly green. |
| Corrected natural-exit Metal selection, run `ff3b6854be9a8c320977dac1` | Independently collected: **1/1 passed**, exit 0. |
| Backend projection Metal selection, run `ae8883059f3ba280d1ffdac7` | Independently collected: **12/12 passed**, exit 0. The documented pre-correction selection had six failures and six passing controls. |
| Strict CLI/Sim Clippy, run `1709989d88d8874e3c93afab` | Independently collected: exit 0 with warnings denied, as documented by the remediation. |
| Static diff audit | Both `git show --check` commands passed. Each reviewed commit preserves Marcus as author and has exactly `Co-Authored-By: Codex <codex@openai.com>` with no Claude or Anthropic attribution. |

The user explicitly waived mutation testing. This review adds no mutation or D17 witness obligation. Unrelated dirty work and the other agent's D11 files were left untouched.

## Finding F-01 — transitioned D10 handler test lacks its per-test Contract Shape

**Severity:** blocking process and test-contract declaration defect. **Location:** `crates/overdrive-control-plane/tests/acceptance/service_submit_dispatch_wiring.rs:445–450`.

`s_shcp_wire_13_cap_timer_synthesises_service_failed_timeout` was transitioned from a fixed one-second cap oracle to the approved derived Service progress deadline: the fixture now computes the deadline, advances by that duration and asserts its exact value. `AGENTS.md` requires every new or transitioned test to carry a per-test Contract Shape declaration. This test has no `CONTRACT_SHAPE` line; a search of its entire file found none. Its sibling hook-ownership and backend tests retain their per-test declarations, and the CLI test retains its exact pure-function declaration. The changed handler test therefore remains the one declaration gap even though its runtime assertion passed.

**Disposition:** add the appropriate function-local Contract Shape declaration to this existing test and verify the bounded edit. This finding requests no extra test, production change, or D11 remediation. No code or commit was made by this reviewer.

**Iteration 1 verdict: NEEDS_REVISION.** The D7/D9/D10/D12 oracle mappings and focused outcomes are sound within the reviewed scope; approval waits for F-01's required declaration.

## Iteration 2 — F-01 remediation review

### Revision and contract disposition

Commit `6b7d268063ed4686855b1201386e89bab100389a` adds `/// CONTRACT_SHAPE: bounded-change.` immediately above the D10 handler test's attributes. This is the correct shape for a test that drives the production Service submit handler, advances `SimClock`, and observes a bounded Timeout wire result. A function-local `clippy::doc_markdown` allowance names the exact machine-read marker; it does not suppress any other Clippy lint or apply to sibling tests. The commit's only other file is `distill/final-oracle-contract-shape-remediation.md`, which records the bounded correction and verification.

The diff adds no test-body statement and changes no fixture input, deadline calculation, assertion, test identity, seed, production code, public interface, model or mutation setting. The original D10 result evidence therefore still applies to the unchanged body. The accepted D7/D9/D10/D12 mappings and the separate D11 scope boundary from iteration 1 are unchanged.

### Verification and findings

| Evidence | Result and limit |
|---|---|
| `git show --check 6b7d268063ed4686855b1201386e89bab100389a` | Passed independently. The commit keeps Marcus as author and exactly the required Codex co-author trailer, with no Claude or Anthropic attribution. |
| Scoped strict Metal Clippy, run `fff4f70185f4ae8881e6966b` | I independently collected the durable record: exit code 0. The remediation identifies the command as `cargo clippy -p overdrive-control-plane --test acceptance --features integration-tests -- -D warnings`. |
| Metal BPF prerequisite, run `b9c341304216ded2de41bac8` | Independently collected: exit code 0. Its generated `vmlinux.h` warnings did not fail the prerequisite. |
| Changed-file formatting and `git diff --check` | Passed as recorded by the remediation author; the independently checked committed diff has no whitespace defect. |
| Runtime | The D10 handler test body was unchanged by this commit and passed in the earlier focused Metal selection. No native test was rerun for this declaration-only correction. |

| Finding | Iteration 2 disposition |
|---|---|
| F-01 — missing per-test Contract Shape on the transitioned D10 handler test | **Closed.** The exact required declaration is present and the local lint allowance passes scoped strict Clippy. |

No finding remains in this bounded DISTILL review. The user waived mutation testing, so no mutation or D17 witness is required. This reviewer changed only this Markdown review artifact and made no code edit or commit.

**Final verdict: APPROVED — iteration 2.** F-01 is closed without changing the D10 test's behavior or the previously reviewed D7/D9/D12 oracles.
