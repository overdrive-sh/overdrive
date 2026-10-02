# DELIVER Review — Step 05-01

## Review metadata

- Feature: `netns-density-295`
- Step: `05-01` — Required serve-boundary ports and the shared intercept listener
- Iterations: 1 and 2
- Reviewer: `nw-software-crafter-reviewer`
- Reviewer model: GPT-6 Luna, maximum reasoning
- Reviewed commits: `bb48b90283d55c42b974639eee2504745dda4a77`, `0b1d20d2fa56f74ceef3ab8da2e0ccbd351339d4`, `e7b5fee53031ab688010647fc7973bb2ff6dc5ad`, and `b0e000b1e2f179ebcc2ed3e7848518017bfa6bfa`
- Iteration 1 verdict: **NEEDS_REVISION**
- Final verdict after iteration 2: **APPROVED**
- Authority: approved `deliver/roadmap.json` step 05-01; accepted R16, B-1, B-7, and B-8 contracts in `feature-delta.md`; ADR-0138; DISTILL S-ND295-65, S-ND295-34, the DNS leg of S-ND295-00, S-ND295-70, S-ND295-71, and the S-ND295-20 worker twins; `red-classification.md`.

## Scope and evidence

The review covered the exact public and cross-crate port shapes, the serve and worker owner paths affected by the step, the named acceptance bodies, and the bounded production/test files in the two commits. The only pre-existing dirty path is `AGENTS.md`; it was preserved. This review wrote only this artifact.

The implementation commits retain Marcus as author and each has exactly one `Co-Authored-By: Codex <codex@openai.com>` trailer and `Step-Id: 05-01`. The completion commit changes only the DELIVER execution log.

## Strengths

The required API shapes match the accepted contract. `ServerConfig::new(kek, mtls_intercept, guest_dns)` has the pinned three-argument signature; `GuestDns`, `GuestDnsDeps`, `GuestDnsFactory`, and `HostGuestDnsFactory` have the required doc-hidden visibility and methods ([`dns_responder/mod.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-control-plane/src/dns_responder/mod.rs:56), [`lib.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-control-plane/src/lib.rs:1114)). `DnsServeTaskOwner` holds `Arc<dyn GuestDns>`, `DnsResponder::audit` reads back the recorded socket identities, and the worker, DNS owner, and supervisor are composed unconditionally. Both `AppState` constructors take the worker, shared owner, gate, and pool; the `ServerHandle` owner fields are non-optional. The optional composition hooks and test replacement/injection methods named by R16/B-1 are absent.

B-7 is implemented at the intended port boundary. `bind_transparent` returns `Arc<dyn InterceptListener>`; the host adapter retains the transparent socket and implements async accept through Tokio; the sim listener is socket-free. The worker shares listener Arcs with two cancelable accept tasks, awaits `MtlsResolve::resolve`, and its production allocation path no longer contains the per-allocation listener branch ([`mtls_intercept_port.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-worker/src/mtls_intercept_port.rs:650), [`mtls_intercept_worker.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-worker/src/mtls_intercept_worker.rs:1239)). The S-ND295-70 host bodies exercise the host listener for both outbound original-destination recovery and inbound virtual-address recovery through the shared program ([`mtls_intercept_install.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-worker/tests/integration/mtls_intercept_install.rs:2212), [`mtls_intercept_install.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-worker/tests/integration/mtls_intercept_install.rs:2306)).

B-8's install partition is in the accepted order on both adapters: a missing record yields `SharedProgramNotConverged`, a mismatched target yields `SharedListenerPortMismatch`, and only then can element work fail. `HostMtlsIntercept::install_inbound` calls the shared-element path without the retired per-rule fallback; the sim records targets and applies the same install refusals ([`mtls_intercept_port.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-worker/src/mtls_intercept_port.rs:774), [`mtls_intercept_port.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-worker/src/mtls_intercept_port.rs:1176), [`mtls_intercept.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-sim/src/adapters/mtls_intercept.rs:750)). The two member-aware S-ND295-71 equivalence bodies remain pending for 08-02.

The added cleartext relay ownership is required by accepted C-295-L. The current shared leg-F production path claims an allocation, awaits resolve, and can classify `NonMesh`; the accepted contract requires stop to end relays of that exact generation, including one classified during stop's claim wait, and requires owner shutdown to end every relay. The implementation registers each relay under the capability before releasing the claim, then aborts and joins drained relays during allocation stop and owner shutdown ([`mtls_intercept_worker.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-worker/src/mtls_intercept_worker.rs:1071), [`mtls_intercept_worker.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-worker/src/mtls_intercept_worker.rs:2745), [`mtls_intercept_worker.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-worker/src/mtls_intercept_worker.rs:2483), [`mtls_intercept_worker.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-worker/src/mtls_intercept_worker.rs:2892)). S-ND295-20 explicitly assigns the pre-existing detached-relay condition and the two additional stop/shutdown clauses to 05-01. This work preserves the approved retirement outcome; it is not adjacent lifecycle hardening.

## Contract Shape Compliance

| Check | Result | Evidence |
|---|---|---|
| Exact public/cross-crate API shape | PASS except D2 | Required constructor, port, and owner shapes match the approved contract. D2 records the old public accept helpers that the B-7 design says to remove once they have no production caller. |
| Contract Shape declarations | PASS for the inspected activated bodies | S-ND295-65 uses the exact `/// CONTRACT_SHAPE: pure-function.` line; the activated S-ND295-34, S-ND295-00, S-ND295-70, S-ND295-71, and S-ND295-20 bodies carry bounded-change declarations and outcome anchors. No source-local pure-function property was added without its required declaration. |
| Banned test names | PASS | No activated test name matches the repository's banned technical-name pattern. |
| Assertion preservation / skip integrity | **FAIL — blocker D1** | The step adds an ignore marker to one previously active S-ND295-20 twin. The prior body is classified active and passing, and DISTILL says a twin that passes today stays active. |
| External validity | PASS for the required composition and port paths | The serve composition uses the required ports, and the S-ND295-70 host original-destination bodies drive the `HostMtlsIntercept` port. |

## Roadmap criteria assessment

| Criterion | Result | Evidence |
|---|---|---|
| R16 required serve ports and unconditional owners | PASS | `ServerConfig`, `AppState`, `ServerHandle`, action-shim lifecycle, and `run_server*` match the required shapes and composition. |
| B-1 required AppState and ServerHandle ownership | PASS | Both constructors and the handle carry the required worker, owner, gate, and pool; removed optional fallback/test hooks are absent. |
| B-7 port-owned shared listener | PASS except D2 | Listener port, host/sim implementations, cancellation, awaited resolve, and per-allocation branch deletion match the pin. D2 identifies retained obsolete helpers and helper-only tests. |
| B-8 recorded program and ordered install refusal | PASS for this step's clauses | Both adapters enforce the recorded-target precondition and the host inbound per-rule fallback is gone. Removal and failed-converge clauses remain with their assigned later steps. |
| Named scenario activation and preservation | **FAIL — blocker D1** | The intended markers were removed from the listed bodies, but the previously active shared owner-shutdown twin was newly skipped. |

## TDD and phase evidence

`execution-log.json` records RED, GREEN, and COMMIT as `EXECUTED` / `PASS`, in order, at lines 657–674. The RED classification contains an assertion-level S-ND295-65 failure identifying optional composition fields and hooks, the S-ND295-34 missing `DnsResponder::audit`/required-port refusal, and the S-ND295-00 DNS leg's missing responder composition. The orchestrator reports the RED source scan had 59 semantic findings and no compile or collection error.

The code diff activates the step-owned S-ND295-65, S-ND295-34, S-ND295-00 DNS-leg, S-ND295-70, S-ND295-71, and S-ND295-20 bodies without weakening their assertions. D1 is the exception: the new ignore marker is a test removal from the active suite. The RED classification lists `shared_allocation_start_after_owner_shutdown_is_rejected_before_install` among active worker tests, and the approved scenario mapping says that its shared twin stays active if it passes ([`red-classification.md`](/Users/marcus/conductor/workspaces/helios/wellington-v2/docs/feature/netns-density-295/distill/red-classification.md:505), [`test-scenarios.md`](/Users/marcus/conductor/workspaces/helios/wellington-v2/docs/feature/netns-density-295/distill/test-scenarios.md:2415)).

## Findings and required disposition

### D1 — A previously active S-ND295-20 body is skipped

- **Severity:** Blocker
- **Dimension:** Test integrity / G9
- **Location:** `crates/overdrive-worker/src/mtls_intercept_worker.rs:5696-5698`

The implementation commit adds `#[ignore = "pending DELIVER step 05-01 (S-ND295-20)"]` to `shared_allocation_start_after_owner_shutdown_is_rejected_before_install`. The same body had no ignore marker in the parent commit and is listed as an active passing worker test in the RED classification. DISTILL's S-ND295-20 mapping says twins that do not need a scripted connection and pass today remain active (`test-scenarios.md:2415`). This twin does not script a connection. Its production path returns `OwnerShutdown` at the initial worker lifecycle check before it reaches `start_shared_allocation` or any listener bind (`mtls_intercept_worker.rs:2307-2311`).

**Required disposition:** remove the newly added ignore marker and retain this existing body as active. No production change is required. The new skip violates the no-test-weakening gate even though the body’s assertions remain unchanged.

### D2 — Retired accept helpers and their helper-only tests remain

- **Severity:** Blocker
- **Dimension:** Accepted B-7 API/scope contract
- **Locations:** `crates/overdrive-worker/src/mtls_intercept.rs:1130-1142,1156-1167`; `crates/overdrive-worker/tests/integration/mtls_intercept_install.rs:963-999,1304-1348`; `crates/overdrive-worker/src/mtls_intercept.rs:1261-1266`

The B-7 contract allows reusing `accept_outbound_and_recover_orig_dst` and `accept_inbound_leg`; it then requires deleting any helper left without a production caller and its tests, with original-destination evidence carried through the host listener (`feature-delta.md:4265-4268`). The new production path does not call either helper: `HostInterceptListener::accept` uses Tokio accept and the accepted socket's `local_addr` (`mtls_intercept_port.rs:655-688`), and `shared_listener_task` calls the port's `accept` (`mtls_intercept_worker.rs:1245-1250`). The only remaining Rust call sites are test code: the source-local outbound helper test, the old outbound integration body, and the old inbound integration body. The inbound body also installs the retired per-virtual-address rule directly with `install_inbound_tproxy` (`mtls_intercept_install.rs:1317-1326`).

S-ND295-70 already supplies the accepted host-port evidence for both directions through `HostMtlsIntercept::bind_transparent` and `LegListener::accept_leg` (`mtls_intercept_install.rs:2212-2287,2306-2362`). Keeping the unused public helpers and tests preserves a second, non-production path and leaves the explicitly deleted helper API in the crate surface.

**Required disposition:** remove both unused accept helpers and their helper-only source/integration tests. Retain the S-ND295-70 host-listener bodies as the original-destination evidence. This is a bounded B-7 contract completion; it does not require a new API or design choice.

## Quantitative validation and verification

The step's acceptance suite is organized by the approved six scenario IDs, with source-local/adapter clauses mapped to their named bodies. The review found two blockers: one newly skipped previously active test and one retained obsolete helper surface. No test-budget finding is raised against the DISTILL-authored scenario set.

The orchestrator reports these roadmap checks passed:

1. Required serve-port source scan — 2 passed.
2. DNS responder bind and shared-network startup integration selectors — 14 passed.
3. Worker mTLS intercept unit selector — 30 passed.
4. Worker intercept equivalence integration selector — 10 passed.
5. Sim mTLS intercept unit selector — 20 passed.
6. Workspace check and workspace Clippy with `-D warnings` passed.

The DELIVER review did not rerun tests. No mutation test was run; it remains the final DELIVER-wave gate.

## Iteration 1 verdict

**NEEDS_REVISION.** The required serve composition, listener ports, B-8 install partition, and C-295-L relay ownership match their accepted contracts. Approval is blocked by D1's test skip and D2's retained helpers and helper-only tests, both directly covered by the B-7/S-ND295-20 contract.

## Iteration 2

### Remediation commits and scope

The re-review covered `e7b5fee53031ab688010647fc7973bb2ff6dc5ad` and `b0e000b1e2f179ebcc2ed3e7848518017bfa6bfa`. The remediation restores the previously active owner-shutdown twin, removes the two obsolete accept helpers and their private `getsockname_orig` implementation, and deletes the helper-only source and integration tests. The S-ND295-70 host listener tests for outbound and inbound original-destination recovery remain in `mtls_intercept_install.rs`. No production, test, API, or design changes outside the two findings were found in the remediation diff.

`accept_outbound_and_recover_orig_dst`, `accept_inbound_leg`, and `getsockname_orig` now have no Rust declarations or call sites in `overdrive-worker/src` or `overdrive-worker/tests`. The production path remains `shared_listener_task` → `InterceptListener::accept` → `HostInterceptListener::accept`; the host S-ND295-70 bodies exercise that boundary through `HostMtlsIntercept::bind_transparent`, `LegListener::accept_leg`, and the shared program. This closes D2 at the exact B-7 boundary. The deletions remove obsolete helper-only APIs and evidence after the host listener took over the accepted-connection and original-destination obligations.

### Finding dispositions

| Finding | Disposition | Evidence |
|---|---|---|
| D1 — previously active S-ND295-20 body was skipped | **RESOLVED** | The remediation removes the added ignore marker from `shared_allocation_start_after_owner_shutdown_is_rejected_before_install`; the body and assertions remain. Its focused run passed after restoration. The crafter also ran it before and after restoration and recorded no synthetic RED. |
| D2 — retired accept helpers and helper-only tests remained | **RESOLVED** | Both public helper functions and private `getsockname_orig` are deleted. The helper-only source and integration tests are deleted, while S-ND295-70's two shared-program original-destination bodies remain. Call-site search finds no helper names in worker production or test Rust sources. |

### Contract and test review

| Review area | Result | Iteration 2 evidence |
|---|---|---|
| R16 / B-1 required composition and owner inputs | PASS | The remediation does not touch these surfaces. The three-argument `ServerConfig::new`, required `AppState` inputs, non-optional `ServerHandle` owners, and unconditional worker/DNS/supervisor composition remain as reviewed in iteration 1. |
| B-7 listener port and original-destination evidence | PASS | The host and sim listener contract remains unchanged. The two host original-destination bodies still drive the shared program and call `InterceptListener::accept` through `LegListener`. The old helper API and helper-only tests are gone. |
| B-8 install refusal partition | PASS | No B-8 code changed in the remediation; both adapters retain the required-record and port checks, and the host inbound per-rule fallback remains absent. |
| C-295-L cleartext relay retirement | PASS | No relay ownership code changed. Capability-scoped relay retention, stop joining, and owner-shutdown joining remain required by the accepted exact-generation retirement contract. |
| Contract Shape declarations and Outcome anchors | PASS | The restored D1 test already carries its bounded-change declaration and outcome anchor. The two retained S-ND295-70 host bodies keep their bounded-change declarations and anchors. The remediation adds no new test body or property. |
| Test integrity / G9 | PASS | D1 restores the previously active body without changing its setup or assertions. The helper-only test deletions follow B-7's explicit deletion clause; their original-destination outcomes remain covered through the host listener. |
| New public or cross-crate surface | PASS | The remediation removes obsolete helpers and adds no public type, method, parameter, variant, or port. |

All roadmap criteria reviewed in iteration 1 pass after remediation. The two blocking criteria are closed: D1's active body is restored, and D2's obsolete helper API/tests are removed with the S-ND295-70 host evidence retained.

### Re-review and verification

No new public API, error variant, port, lifecycle, or persistence behavior was added in the remediation. The D1 change only restores the previously active test. The D2 deletions match the accepted B-7 clause: the new listener port owns acceptance and original-destination recovery, and the obsolete helpers had no production caller. The C-295-L relay tracking remains necessary for the exact-generation stop and owner-shutdown obligations reviewed in iteration 1.

The root/orchestrator reports these remediation checks passed:

1. Restored `shared_allocation_start_after_owner_shutdown_is_rejected_before_install` — passed before and after restoration; no synthetic RED was claimed.
2. S-ND295-70 host listener original-destination tests — 2 passed.
3. Worker mTLS intercept unit tests — 31 passed.
4. `cargo check --workspace --all-targets --features integration-tests` — passed.
5. `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` — passed.
6. `git diff --check` — passed.

The remediation execution log appends RED, GREEN, and COMMIT as `EXECUTED` / `PASS` in order at `execution-log.json:677-697`. The RED record does not claim a failure for D1 because its existing body already passes. Commit attribution and bounded scope were checked by the orchestrator; `AGENTS.md` remains the only pre-existing dirty file. No mutation test was run; it remains the final DELIVER-wave gate.

## Final verdict

**APPROVED.** D1 and D2 are resolved. The previously active S-ND295-20 owner-shutdown test remains active, the obsolete accept helpers and helper-only tests are removed, and the S-ND295-70 host listener evidence still covers inbound and outbound original-destination behavior. The accepted R16, B-1, B-7, B-8, and C-295-L contracts are satisfied for step 05-01.
