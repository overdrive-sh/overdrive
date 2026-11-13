# DELIVER review — step 05-07

## Metadata

| Item | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Step | `05-07` — Default startup gate is guest-reported listeners |
| Iteration | 2 — final; iteration 1 is recorded below |
| Implementation commit | `229edec40ad544fce501823888b49cfe2c1b92db` |
| COMMIT audit | `bbf21a50e3a1f039f0660b74c60bf42204c45bd4` |
| Evidence-format commit | `b11c8cdbbbc6888c51e21a0d902cf35d8fb29091` |
| F1 remediation commit | `62917d18d2d09da9ca2ed81a83b7b6c5bf0da97b` |
| Review scope | D15 parser default, the dial-bearing probe cap, the composed G-10 (8) scenario, typed input error contract retained from D14-E2, E08 native example support, DES evidence, and the step's production/API/test boundaries |
| Working tree | Preserved unrelated dirty `AGENTS.md`, `netns-density-295` roadmap and feature delta, `specs/quint/guest-flow-owner/fwd_pair.qnt`, and `spike-scratch/step-05-07-sigkill/`; none were changed by this review. |

## Verdict

**APPROVED — iteration 2 (final).** The D15 behavior, approved API shape, typed error mappings, test evidence, and native product proof match the accepted design. Iteration 1's blocking F1 finding is closed by comment-only updates to the two checked-in zero-probe Service specs. Their TOML inputs and behavior remain unchanged.

## Contract review

### D15 behavior and public API

ADR-0270 and feature-delta D15 require an omitted Service startup section to materialize exactly one fieldless `ProbeMechanic::GuestListeners` descriptor. `[[health_check.startup]] = []` remains the opt-out; an operator-declared startup probe remains unchanged. The descriptor retains the 5-second timeout, 2-second interval, and 30 attempts, yielding the accepted 60-second startup deadline. The default is still applied in the TOML parser only; raw API omission remains the existing opt-out.

`parse_startup_probes` implements that contract at `crates/overdrive-core/src/aggregate/workload_spec.rs:1200-1214`. The default property at `crates/overdrive-core/tests/acceptance/health_check_toml_parse.rs:412-457` checks the sole startup descriptor, role, `GuestListeners` mechanic, and the exact 5/2/30 values. The separate explicit-empty property at `:459-498` checks that the opt-out remains empty.

The probe-cap change is bounded to the accepted dial-bearing rule. The private `dial_bearing_probe_count` at `crates/overdrive-core/src/aggregate/mod.rs:557-569` counts only `Tcp` and `Http`; both TOML parsing and raw Service admission call it. `GuestListeners` remains one descriptor for the startup gate but consumes no dial-bearing slot. The changed acceptance cases cover eight dials accepted with a default gate, nine dials rejected, and the matching error text (`crates/overdrive-core/tests/acceptance/vsock_deploy_validation.rs:242-359`). No public method, type, enum variant, parameter, wire field, or persisted format was added by this step. `ProbeMechanic::GuestListeners` is the already approved D14 public variant.

The pre-authored G-10 (8) composed scenario is activated by removing its `#[ignore]` marker; its assertions remain intact (`crates/overdrive-control-plane/src/reconciler_runtime/guest_listener_gate_composed.rs:55-208`). It drives the production `VmDriver::start`, `ProbeRunner`, shared listener board, lifecycle reconcilers, action shim, and exit observer through the existing Sim composition. It checks no row before the deadline, the first deadline `Fail` names only the missing listener, startup stop, restart attempts, `StartupProbeFailed` give-up, and no `Stable` event.

### Typed error contract

D15 introduces no new semantic errors. The approved D14-E2 semantic set remains represented and mapped as follows:

| Designed failure | Approved and implemented representation | Caller mapping reviewed |
|---|---|---|
| `GuestListeners` is in Readiness/Liveness, or its descriptor role disagrees with its Startup vector | `AggregateError::GuestListenersInvalidRole { vector_role, descriptor_role, index }` | `Service::from_submit` validates roles before sole-startup and dial-count checks (`crates/overdrive-core/src/aggregate/mod.rs:461-464`, `:650-674`). `ControlPlaneError::Aggregate` is retained; HTTP returns 400 `validation` and the pinned role-specific `field` (`crates/overdrive-control-plane/src/error.rs:962-988`). |
| Correctly tagged `GuestListeners` is not the sole startup descriptor | `AggregateError::GuestListenersNotSoleStartup { count }`, with `count` equal to the startup vector length | Same typed `ControlPlaneError::Aggregate` chain and HTTP 400 `validation`, field `startup_probes`; the acceptance test checks the exact nested variant and body field (`crates/overdrive-control-plane/tests/acceptance/guest_listeners_http_rejection.rs:81-114`). |
| Canonical `guest_listeners` TOML token is supplied by the operator | `ParseError::GuestListenersNotOperatorConfigurable { role, probe_idx }` | Both deploy modes preserve it as `CliError::ParseError` and exit 1 (`crates/overdrive-cli/src/commands/deploy.rs:234-239`, `:523-534`; `crates/overdrive-cli/tests/acceptance/probes_kind_rejection_cli.rs:210-237`). |

The exact variants and ingress conditions are pinned in `feature-delta.md` § D14-E2. The tests check the semantic variants/payloads and caller result instead of relying on display strings as substitutes. I found no generic-error or new-surface shortcut and no unresolved typed-error contract gap in this step.

### Private native-proof support and ownership

The additional private support is tied to the roadmap's required `serve` + `deploy` native proof. `prepare_clone_staging_root` now applies the already approved root:4200/0710 traverse posture to the platform-owned VM parent and clone-staging root (`crates/overdrive-control-plane/src/lib.rs:3690-3725`), matching ADR-0082's requirement that the staging root and its ancestors grant traverse without list access (`docs/product/architecture/adr-0082-vmm-port-trait-and-vmconfig-anti-corruption-value.md:304-326`). It does not widen operator-owned image directories or alter the Landlock sibling-disk boundary.

The `overdrive-init` support creates and mounts sysfs before guest BTF-backed transport setup, and establishes the file-descriptor limit required by the existing guest transport (`crates/overdrive-init/src/main.rs:123-179`, `:355-375`, `:602-607`). These are private implementation details: the new `SysfsMount` is an internal `InitError` used for the existing pre-READY failure path, not a new cross-crate error contract. E08 preparation stages and verifies the same-release module closure and current static init (`examples/service-kind-vm-workloads/prepare.sh:262-354`, `:397-427`). The retained captures show these changes enabled the required default-feature native journey; they add no operator API, configuration, or wire shape.

## Implementation test integrity and evidence — iteration 1

| Gate | Result | Evidence and limit |
|---|---|---|
| RED → GREEN → COMMIT | **PASS, final replacement sequence** | `execution-log.json` records the replacement's independent RED/FAIL at `2026-11-12T22:15:12Z`, GREEN/PASS at `23:33:45Z`, then COMMIT/PASS at `23:43:17Z`. Earlier failed attempts remain recorded; the replacement did not claim them as its RED. |
| G1 — required acceptance activation | **PASS** | The required composed G-10 (8) scenario is active; the two D15 parser properties and the dial-cap cases are live. |
| G2 — semantic RED | **PASS as recorded** | The replacement's isolated RED overlaid the active parser property while production remained at the rebased preimplementation HEAD. The recorded Metal run failed for the intended assertion: it produced `Tcp { host: "0.0.0.0", port: 1 }` where D15 requires `GuestListeners` (`step-05-07-verification.md`, § DES). |
| G3–G4 — unit RED / doubles | **N/A / PASS** | No production unit-test method was added; the composed case uses the existing Sim driver, observation store, clock, beacon, and production owners at their ports. |
| G5 — domain language | **PASS except F1** | The acceptance assertions use `GuestListeners`, startup rows, declared listeners, `StartupProbeFailed`, and `Stable`. F1 is the stale operator-facing wording in the checked-in specs. |
| G6–G7 — green and pre-commit | **PASS as recorded** | Core acceptance 646/646; G-10 composed 1/1; S-22 and progress-deadline 2/2; both guest-listener real-boot cases 2/2; E08 positive and negative native captures; scoped Clippy/formatting pass. This reviewer did not rerun these Metal gates. |
| G8 — test budget | **PASS** | Eight distinct step outcomes map to the criteria: omitted startup materializes the 60-second gate; explicit empty remains opt-out; the gate consumes no dial slot; a missing report stops/restarts/gives up without `Stable`; S-22's startup window remains unchanged; the progress deadline remains 90 seconds; a native listening Service reaches `Stable`; and a native never-listening Service reaches `StartupProbeFailed`. Budget: 8 × 2 = 16 unit-test methods; actual new unit methods: 0. Seven existing core acceptance methods were changed for the two parser outcomes and dial-cap behavior (two parser properties and five cap methods); their bounded input variations remain one method each. The separately mandated composed scenario is integration evidence. |
| G9 — test integrity | **PASS** | The composed test changed only from ignored to active. The parser expectations and count/error-message assertions changed only where D15 changes the contract; no existing assertion was deleted or weakened. |
| Contract Shape Compliance | **PASS** | The two transitioned source-local pure properties have exact `/// CONTRACT_SHAPE: pure-function.` declarations (`health_check_toml_parse.rs:415`, `:462`; `vsock_deploy_validation.rs:339`). The composed test retains its per-test `bounded-change` declaration (`guest_listener_gate_composed.rs:61-63`). |
| External validity | **PASS** | The E08 evidence directly drives the built default-feature `overdrive` binary using checked-in zero-probe specs and public `deploy`, `workload describe`, and stop commands. It does not import an Overdrive crate, launch a Rust test binary, or act as an acceptance-test runner. |
| Mutation gate | **Not run** | Correctly reserved for the single final DELIVER-wave gate after all 17 step reviews. |

### Native production-path evidence

The checked-in positive Service has no `health_check` table (`examples/service-kind-vm-workloads/zero-probes.toml`). The retained native capture uses the built default-feature CLI and reports `Stable` with `startup probe[0] guest listeners tcp:18081 last=pass` (`.context/step-05-07-native/rebased-20261112/positive/service-describe.log`); deploy settles in 1,363 ms (`positive/service-deploy-stdout.log`). Its stop and E08 cleanup report zero VM, probe, network, cgroup, run-directory, mount, loop, and preparation deltas (`positive/cleanup-resource-delta.log`).

The checked-in negative Service also has no `health_check` table. The default-feature CLI's first `deploy` returned the expected 90-second progress timeout while the product continued the existing restart lifecycle; the subsequent public `workload describe` shows six allocation rows, attempts 0–4 `Terminated`, attempt 5 `Failed`, and each startup failure names `tcp:18998` after 60 seconds. Re-submitting the byte-identical spec returns `StartupProbeFailed` with the missing listener (`negative-retry/deploy-terminal.stdout`, `negative-retry/workload-describe-latest.log`). The owner path is the CLI parser (`crates/overdrive-cli/src/commands/deploy.rs:189-225`, `:523-526`) → `parse_startup_probes` (`workload_spec.rs:1200-1214`) → `ProbeRunner::start_alloc` dispatch (`crates/overdrive-worker/src/probe_runner/mod.rs:315-357`) → `supervised_guest_listener_loop` over the declared set (`:761-852`) → `ServiceLifecycle`'s Running + Pass `Stable` branch and deadline stop branch (`crates/overdrive-reconcilers/src/service_lifecycle.rs:607-627`, `:1190-1214`). The production task stops cooperatively on its supervisor cancellation token (`probe_runner/mod.rs:363-375`, `:779-783`, `:844-850`); the native outcome confirms the existing retry/give-up path, not a changed shutdown mechanism. Cleanup deltas are zero for VM, network, cgroup, run-directory, mount, loop, preparation, and serve (`negative-retry/cleanup-resource-delta.log`).

The full run IDs, CLI test outcomes before and after, guest-kit preparation, and retained verification are in `step-05-07-verification.md`. The before/after CLI streaming test passed in both runs; no fix is attributed to this step. E13 and website finalization remain outside this step per the roadmap and are not reported as passing here.

## Findings and remediation dispositions

### F1 — Blocking — checked-in zero-probe specs describe the removed TCP default

**Locations:** `examples/service-kind-vm-workloads/zero-probes.toml:19-21`; `examples/service-kind-vm-workloads/zero-probes-failure.toml:19-20`.

**Evidence and reachability:** These are the exact checked-in specs used by the step's default-feature `overdrive deploy` native proof. The positive product `workload describe` reports `guest listeners tcp:18081 last=pass`; the negative product path reports `guest listeners not open after 60s: tcp:18998` and reaches `StartupProbeFailed`. The parser's live D15 property also asserts the exact `GuestListeners` mechanic. Thus the production behavior is reachable through the CLI and its Service owner path, while the adjacent comments still state that the parser infers a TCP probe against the first listener. The mismatch is reproduced by the retained black-box output, not inferred from an imagined failure schedule.

**Contract violated:** ADR-0270 / D15 removes the default TCP probe in favor of one guest-listener gate over every declared TCP and UDP listener. The comments tell an operator that only a first-listener TCP probe is used and that the negative case is an inferred TCP connect. This contradicts the behavior demonstrated by this step's native proof and makes the product example describe the superseded contract.

**Required remediation at iteration 1:** Update only these two TOML comments to state that an omitted startup section materializes the guest-listener gate, and describe the negative case as the declared listener remaining unreported through the 60-second startup deadline. Preserve the TOML behavior and assertions. Keep the E13 expectation and website finalization work at their recorded finalization scope.

**Iteration 1 disposition:** Open at the end of iteration 1; returned to the original 05-07 crafter for bounded comment-only remediation. No production fix or test expansion was indicated.

## Iteration 1 disposition

**NEEDS_REVISION.** All findings except F1 were closed by the inspected implementation and recorded evidence. F1 blocked approval until the two checked-in E08 specs described the accepted D15 default accurately. At that point, no remediation iteration had yet been reviewed.

## Iteration 2 — F1 remediation

**Remediation commit:** `62917d18d2d09da9ca2ed81a83b7b6c5bf0da97b` (`docs(service-readiness): align zero-probe specs with D15`).

The commit changes only comments in `examples/service-kind-vm-workloads/zero-probes.toml` and `zero-probes-failure.toml`. The positive spec now describes the D15 `GuestListeners` gate across declared TCP and UDP listeners, its 60-second deadline, and the guest report needed for `Stable`. The negative spec now describes the unreported TCP/18998 listener, deadline, stop/restart/give-up outcome, and `StartupProbeFailed`. The diff contains comment lines only; all TOML values and parsed semantics are unchanged. `git diff --check` passes. The commit preserves Marcus as author and has exactly the required Codex co-author trailer.

No tests or native proof were rerun because this remediation changes comments only; the iteration-1 parser, composition, and native evidence remains applicable. The E13 expectation and website finalization scope were not changed. F1 is closed, with no new findings.

### Iteration 2 verification and disposition

| Check | Result | Evidence and limit |
|---|---|---|
| Remediation scope | **PASS** | The commit changes only the two requested E08 spec comment blocks; no production or test file changed. |
| D15 wording | **PASS** | The positive comment names the all-declared-listener guest gate and 60-second deadline; the negative comment names the missing report and `StartupProbeFailed` restart/give-up outcome. |
| TOML behavior | **PASS** | The complete commit diff contains comments only, so parsed input and product behavior are unchanged. No parser tests were rerun. |
| Diff hygiene | **PASS** | `git diff --check 62917d18d2d09da9ca2ed81a83b7b6c5bf0da97b^ 62917d18d2d09da9ca2ed81a83b7b6c5bf0da97b` returns no diagnostics. |
| Commit attribution | **PASS** | Marcus remains author; the commit has exactly `Co-Authored-By: Codex <codex@openai.com>` and no Claude/Anthropic attribution. |
| Tests/native/mutation | **Not rerun** | Not needed for comment-only remediation; native and test evidence remains as recorded in iteration 1. Mutation testing remains the final wave gate. |

**Iteration 2 verdict: APPROVED.** The finding is resolved and no open findings remain. Step 05-07 is finally approved.
