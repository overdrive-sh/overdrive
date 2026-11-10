# DELIVER Review — Step 05-01: Declared listeners and TCP service-port view

## Metadata

| Item | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Step | `05-01` — Declared listeners and TCP service-port view |
| Iterations recorded | 1 and 2 |
| Reviewed implementation | `e3646f93229b0ed8417571d107329892ab8a5ed3` |
| Reviewed audit commit | `bafb45c3c0b56b9f5880278c7cb845d09a24a238` |
| Reviewer | Codex, DELIVER review |
| Final verdict | **APPROVED** — F-01 resolved in iteration 2 |

## Scope and authoritative contract

Reviewed the exact PR-4 / D14-A1 contract in `docs/feature/service-allocation-readiness-and-ending/feature-delta.md` (§§ D14 item 2, D14-A1, S-55), ADR-0269, approved roadmap step 05-01, the approved DISTILL native-fixture correction and its independent review, the two commits above, their DES events, and affected production and test paths.

The public contract is `AllocationSpec.declared_listeners: Vec<Listener>` replacing `service_ports` in both `AllocationSpec` and `WorkloadLifecycleState`, plus exactly `pub fn tcp_service_ports(&self) -> Vec<NonZeroU16>`. It returns every TCP listener port in declaration order, retaining duplicates; it omits UDP, does not sort, and does not mutate the spec. Both existing TPROXY consumers read this view. The startup gate is unchanged. The feature delta also explicitly says the hydration characterization and reconcile-trajectory fixtures are regenerated for this field replacement, not hand-edited.

## Iteration 1 summary

The production change matches the approved API and behavior. The full listener declaration is projected from Service intent, carried through the desired lifecycle state and all three allocation-spec production paths, and read through the TCP view by both TPROXY consumers. The worker’s real nft installer and its allowed-port capability set receive TCP ports only. The startup-gate code is untouched.

The code and native-fixture evidence are otherwise satisfactory. The two checked-in generated hydration/trajectory snapshots still contain `service_ports`, though the approved contract removes that field. A focused run reproduced the trajectory snapshot mismatch at the changed field boundary. The missing snapshot regeneration is explicitly part of PR-4’s fixture fallout, so this step is not approved yet.

## Iteration 1 findings

### F-01 — Blocking: generated snapshots still describe the removed `service_ports` field

**Severity:** Blocker — required step artifact incomplete.

**Locations:**

- `crates/overdrive-sim/tests/acceptance/fixtures/hydration_trajectory/workload_lifecycle_trajectory.txt:44`
- `crates/overdrive-control-plane/tests/acceptance/fixtures/hydration_golden/workload_lifecycle.txt:57,133`

**Evidence:**

- The approved feature delta says PR-4 replaces `service_ports` in both types and specifically names the hydration golden and sim trajectory fixtures for regeneration (`feature-delta.md:2539-2555`). S-55 also requires that `WorkloadLifecycleState` no longer carries `service_ports` and that emitted specs carry `declared_listeners` (`feature-delta.md:1872`).
- The implementation removes `service_ports` from `AllocationSpec` and `WorkloadLifecycleState` (`crates/overdrive-core/src/traits/driver.rs:329-345`, `crates/overdrive-reconcilers/src/workload_lifecycle.rs:1985-1989`). The current production trajectory now emits `declared_listeners` without `service_ports`; the committed sim fixture still records both fields, and the control-plane hydration fixture still records `service_ports` on both hydration sides.
- I ran the focused checks through the required Metal runner:
  `cargo xtask metal run -- cargo nextest run -p overdrive-control-plane -p overdrive-sim --features integration-tests --test acceptance -E 'test(pre_move_hydrated_anystate_golden_covers_all_seven_reconcilers) | test(pre_move_reconcile_trajectory_is_reproducible_and_pinned)'`
  Result: **0 passed, 2 failed**. The sim trajectory’s actual/expected output differs at the `service_ports`/`declared_listeners` field shape. The control-plane golden also shows the new `declared_listeners` shape, but its whole test has other snapshot drift; this review does not attribute those unrelated differences to step 05-01.
- The earlier `.context/distill/default-lane.log` records the sim trajectory test failing before this commit because the RED scaffold had already added `declared_listeners` while its snapshot still had only `service_ports`. That history means the failure was not newly introduced by this commit. It does not waive the approved PR-4 requirement to regenerate the final fixture for the one-field representation now implemented.

**Required remediation:** Use the existing ignored golden-regeneration entry points for the two fixtures named by D14. Inspect the generated diff and keep it limited to the approved representation change; do not absorb unrelated pre-existing snapshot drift into this step. No production API or behavior change is requested by this finding.

**Iteration 1 disposition:** Pending at that review; returned to the original 05-01 crafter for bounded fixture completion and re-review.

## Contract and production-path review

### Exact API shape

| Contract | Implementation | Assessment |
|---|---|---|
| `AllocationSpec.declared_listeners: Vec<Listener>` replaces `service_ports` | `driver.rs:329-345` | Match |
| `tcp_service_ports(&self) -> Vec<NonZeroU16>` | `driver.rs:336-345` | Exact user-approved signature; no extra API |
| TCP filter preserves declaration order and duplicates, omits UDP, performs no mutation | `driver.rs:340-345` | Match; iterator filtering and collection preserve order/multiplicity |
| `WorkloadLifecycleState` carries the full listener declaration | `workload_lifecycle.rs:1985-1989` | Match |
| Service hydration projects listener protocol, port, order and multiplicity | `workload_lifecycle.rs:588, 2090-2096` | Match; Job/Schedule/absent intent remain empty |
| Start, restart, and live-intent reconstruction carry declared listeners | `workload_lifecycle.rs:1384-1389, 1680-1682, 2125-2132` | Match |

The changed types are in-memory state; this step adds no serialized or wire field. I found no extra public method, enum variant, trait, parameter, configuration surface, or altered ownership/lifecycle meaning.

### Both TPROXY consumers

- The action-shim consumer now obtains `spec.tcp_service_ports()` and maps the owned ports to `ListenPort` (`crates/overdrive-control-plane/src/action_shim/mod.rs:1558-1567`). `NonZeroU16` makes the existing `ListenPortError::Zero` branch unreachable by construction; no error taxonomy changed.
- `MtlsInterceptWorker::start_shared_allocation` now uses the same vector for the allocation capability set and for each inbound install (`crates/overdrive-worker/src/mtls_intercept_worker.rs:2723-2754`). UDP is filtered before either operation. Existing install failures still return `MtlsInterceptInstallError`; this change adds no semantic error path.
- No startup-gate implementation was changed. The typed D14-E2 listener-input rejection contract is outside step 05-01 and is neither changed nor carried by a new generic/string error here. **Typed-error mapping for this step:** UDP declaration → omitted from the TCP view and no inbound rule; valid TCP declaration → included and installed; host install failure → the existing `MtlsInterceptInstallError` path. No unresolved typed error contract was found in the changed path.

## Acceptance tests and evidence

### Acceptance coverage

The two pure-function tests are live and carry the exact `/// CONTRACT_SHAPE: pure-function.` declaration. The property compares arbitrary TCP/UDP listener lists against the declaration-order filter, including generated duplicates; the example covers UDP-only, empty, mixed-protocol and ordering cases (`declared_listeners_tcp_view.rs:48-77`). The native integration test carries `/// CONTRACT_SHAPE: bounded-change.` and enters the real `MtlsInterceptWorker::start_alloc`; it checks exact TCP-only nft destinations and absence of the shared program before setup and after cleanup (`inbound_rules_only_for_tcp_declared_listeners.rs:37-88`). Its additional scrub and cleanup assertions are the independently approved DISTILL fixture correction, not a substitute capture path.

Test-budget review: two observable behaviors (ordered TCP projection; TCP-only inbound installation), four unit-test budget, two unit acceptance tests; the native infrastructure integration is separate. **PASS.** No assertion was weakened, removed, or skipped to make GREEN. The two `#[ignore]` markers were removed to activate the accepted S-55 bodies. The native fixture assertions were approved through `distill/review-declared-listener-native-fixture-remediation.md`.

Contract Shape review: all three activated S-55 identities have their required per-test declaration; both pure tests use the mandated exact line. The feature delta explicitly records that no DISCUSS wave exists (`feature-delta.md:42`), and the prior step review marks the DISCUSS outcome anchor not applicable (`review-01-01.md:66`); no placeholder outcome anchor was invented. The test names do not match the reviewer role’s banned-name pattern.

### Reported and independently reproduced verification

**Crafter-reported evidence (not rerun in this review):** TCP-view tests 2/2, assigned nft integration 1/1, lifecycle-projection tests 4/4, mTLS installer unit tests 22/22, shim tests 46/46; affected all-target Clippy, guest-suite Clippy, rustfmt and `git diff --check` passed. The exact native result is retained in `.context/step-05-01-declared-listener-native-final.log`; the existing nft fixture control passed 1/1 in `.context/step-05-01-existing-nft-fixture-control.log`. The independently approved DISTILL record identifies the preflight failure as missing guest `nft`/`uname`, and the correction stages only those tools for this exact test while retaining the real worker, kernel observation, TCP-only oracle and cleanup checks.

**Reviewer-run evidence:** the two focused golden tests above failed as recorded under F-01. I did not run a broad suite or mutation testing. The roadmap assigns mutation testing to the single final DELIVER-wave gate.

### Iteration 1 DES and commit audit

`execution-log.json` records step 05-01 RED fail, GREEN fail, GREEN pass, COMMIT pass in order. The recorded GREEN failure is consistent with the documented native-fixture prerequisite correction; the later native run passed on the existing Metal host. The feature commit is scoped to the public field/accessor cut, bounded consumers, constructor fallout, the accepted exact-test fixture correction and DES state. The two generated snapshot updates required by the approved design are still missing. Both commits preserve the Git author and use exactly `Co-Authored-By: Codex <codex@openai.com>`; both carry the 05-01 step/task trailers. The worktree was clean before this review artifact was written.

## Iteration 1 verdict

**NEEDS_REVISION.** F-01 is a bounded, design-required fixture update. The API, production consumer behavior, typed-error boundary, native test honesty and startup-gate scope otherwise match the approved contract. No adjacent projection cleanup, production hardening, or architecture change is requested.

## Iteration 2 review

| Item | Result |
|---|---|
| Remediation commit | `4e199d785bf1d474fd844e16fd24c75d877d5b2b` |
| Scope | The two generated fixtures named in F-01 |
| Production, acceptance-test, or DES changes | None |
| Commit attribution | Existing Git author preserved; exact Codex co-author and 05-01 step/task trailers present |
| Iteration 2 reviewer verification | Focused hydration-characterization and trajectory tests passed 2/2 |

The remediation commit changes only the two expected snapshots. The control-plane hydration fixture replaces `service_ports: []` with `declared_listeners: []` on desired and actual `WorkloadLifecycleState`; the sim trajectory removes the obsolete `service_ports: []` from the emitted `AllocationSpec`, retaining `declared_listeners: []`. The fixture diff contains no unrelated output changes. This matches the current production output and preserves the Job empty-list outcome.

I reran the exact focused checks through the required Metal runner:

```text
cargo xtask metal run -- cargo nextest run -p overdrive-control-plane -p overdrive-sim --features integration-tests --test acceptance -E 'test(pre_move_hydrated_anystate_golden_covers_all_seven_reconcilers) | test(pre_move_reconcile_trajectory_is_reproducible_and_pinned)'
```

Result: **2 tests run, 2 passed, 596 skipped**. No broad suite or mutation testing was run. The golden regeneration is confined to F-01; no other iteration-1 finding remains open.

**F-01 disposition:** Resolved. Both generated fixtures match the approved `declared_listeners` shape and their focused production-path snapshot tests pass.

## Final verdict

**APPROVED.** Iteration 2 closes the sole blocking finding. The exact API, TCP-only behavior, production consumer paths, native test boundary, error contract, and startup-gate scope remain as assessed in iteration 1.
