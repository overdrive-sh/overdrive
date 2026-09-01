# Adversarial review — step 02-01

- **Feature:** `guest-stack-transparent-mtls-intercept`
- **Step:** `02-01` — D6 `Exec | Vm` intercept gate and guest egress walking skeleton
- **Reviewer:** `nw-software-crafter-reviewer` (fresh isolated adversarial reviewer)
- **Review ID:** `code_rev_20260828_160810_iteration_1`
- **Iteration:** 1
- **Commit:** `3a1ed07fe684d494eb1643a632aaae59fc0cf68b`
- **Parent:** `c035ac38e02417b276aa344763ca5b6b1bc2ae3b`
- **Subject:** `feat(guest-stack-transparent-mtls-intercept): enable transparent mTLS for guest VMs`
- **Trailer:** `Step-Id: 02-01`
- **Final verdict:** **NEEDS_REVISION**

## Executive summary

The two production D6 predicates are correctly and symmetrically changed from `Exec` to `Exec | Vm` at the fresh-start and restart Running arms. The terminal teardown sites remain ungated by driver type. `MtlsInterceptWorker::start_alloc`, `HostMtlsIntercept::install_outbound`, `install_outbound_tproxy`, and the #26 enforcement proxy have a zero-byte parent-to-commit diff, so the implementation really does reuse the established intercept path. The TAP-owner and exact TAP-sysfs Landlock fallout is tightly related to launching Cloud Hypervisor under the configured VMM uid. All four requested metal scenarios execute successfully, and the independent post-run inspection found no guest netns, TAP, Cloud Hypervisor process, or per-allocation nft rule residue.

The step cannot be approved. The metal harness substitutes an identity contract that production can never supply: the test leaf has a `peer.overdrive.local` DNS SAN and is always available for every allocation, while the production `Ca::issue_svid` contract permits exactly one URI SAN and the production `IdentityMgr` begins empty. The production leg-B client nevertheless always performs WebPKI verification for that fixed DNS name. The green mesh round-trip therefore proves a test-only PKI path, not the claimed production-drivable loop. The peer-wire oracle is also circular: it looks for plaintext only in streams it has already classified as TLS-bearing, and its kTLS check can combine facts from different sockets. Finally, the new and transitioned acceptance tests fail the mandatory Outcome Anchor and bounded-change complement rules. D1–D4 require remediation before the next roadmap step.

## Contract Shape Compliance

**Overall: FAIL**

| Check | Status | Evidence |
|---|---|---|
| Exact per-test declarations | **FAIL** | Eight of nine step-touched behavioral tests carry an exact declaration. The transitioned API acceptance test `alloc_status_response_round_trips_with_empty_and_populated_rows` adds the new `workload_addr` fixture but has no `CONTRACT_SHAPE` declaration. |
| Outcome anchor | **FAIL** | Zero of the six step-touched acceptance tests contain the exact `Outcome anchor: DISCUSS Elevator Pitch` line: S-GTI-01, S-GTI-03, S-GTI-04, S-GTI-07, the describe-render acceptance test, and the transitioned API-shape acceptance test. |
| Banned test-name regex | PASS | None of the nine step-touched test names matches `^test_.*(returns_\d+|exit_code|calls_.*_once|status_code|http_\d+)`. |
| Unbounded-preservation mechanism | **FAIL** | S-GTI-03 snapshots raw AF_PACKET frames, but its projection discards cleartext-only streams before evaluating the preservation claim; see D2. |
| Bounded-change delta and complement | **FAIL** | S-GTI-01, S-GTI-04, S-GTI-07, and `render_workload_describe_surfaces_the_persisted_canonical_address` declare bounded change but do not declare a loose observable universe and prove its full complement unchanged. The render test asserts only `contains(...)`. |
| Layer choice and external contract | **FAIL** | The walking skeleton drives production networking but replaces the production identity contract with a stronger always-present DNS-SAN fixture; see D1. |

The mandated checker script `src/des/cli/check_contract_shape_declarations.py` is not present in this checkout or the installed nWave tree. The mechanical results above come from a direct diff-scoped `rg` audit and are independently reproducible.

## Mechanical evidence

### Commit scope — PASS

- Stat: **20 files changed, 1,111 insertions, 461 deletions**.
- The only D6 production changes are the two predicate flips in `action_shim/mod.rs`.
- `git diff --quiet` confirms zero changes to `mtls_intercept.rs` and `mtls_intercept_worker.rs`; `HostMtlsIntercept::install_outbound` still delegates directly to `install_outbound_tproxy(host_veth, agent_leg_f_port)`.
- Both teardown paths still call `worker.stop_alloc` without a `DriverType` predicate before `teardown_and_release_netns`.
- The public `OVERDRIVE_VMM_UID`, TAP ownership observation/repair, and exact TAP sysfs Landlock read grant are necessary runtime fallout exposed by the real guest boot, not unrelated behavior.
- The API/renderer additions are directly traceable to S-GTI-07.
- No mutation exclusions were changed, and no per-step mutation run was performed.
- Pre-existing dirty `roadmap.json`, `AGENTS.md`, and prior review artifacts were preserved and excluded from the reviewed commit.

### D6 and reuse audit — PASS

| Claim | Result | Evidence |
|---|---|---|
| Fresh-start install gate | PASS | `action_shim/mod.rs:1727-1729` uses `matches!(..., DriverType::Exec | DriverType::Vm)`. |
| Restart install gate | PASS | `action_shim/mod.rs:2023-2025` uses the identical predicate. |
| Outbound TPROXY reuse | PASS | No diff in `mtls_intercept.rs`; `install_outbound_tproxy` remains the one-rule `iifname <host_veth>` TPROXY install. |
| Worker reuse | PASS | No diff in `MtlsInterceptWorker::start_alloc`; it still reads `spec.host_veth` and dispatches through `HostMtlsIntercept::install_outbound`. |
| #26 enforcement proxy reuse | PASS | No dataplane or enforcement implementation file changed. |
| Teardown remains ungated | PASS | Both terminal paths remain `mtls_worker.is_some()` only; no `DriverType` predicate was added. |

### DES phase order — PASS

| Phase | Status | Timestamp |
|---|---|---|
| RED | EXECUTED / PASS | `2026-08-28T15:35:20Z` |
| GREEN | EXECUTED / PASS | `2026-08-28T15:48:06Z` |
| COMMIT | EXECUTED / PASS | `2026-08-28T15:49:07Z` |

All three canonical phases are present, successful, and chronologically ordered. The commit timestamp is the same second as the logged COMMIT event and carries the required trailer.

### Test budget — PASS

| Behaviors | Budget (`2 × behaviors`) | Step-touched tests |
|---:|---:|---:|
| 6 | 12 | 9 |

The six behaviors are the six roadmap acceptance-criteria rows. The nine tests are four transitioned metal scenarios, one new render acceptance test, and four existing tests behaviorally extended for API/TAP-owner/Landlock fallout. Neutral `workload_addr: None` literal updates are not counted.

### Test-integrity diff — PASS WITH DOCUMENTATION FINDING

The old S-VM-74 assertion that VM allocations must receive no intercept was deleted. That is not a weakening to make this step green: the approved D6 criterion directly reverses that old contract, and S-GTI-01/S-GTI-03 replace it with real positive interception evidence. No surviving assertion was weakened or skipped. The deletion did leave multiple source and nextest comments falsely claiming S-VM-74 still exists and is green; that is D4.

## Blocking findings

### D1 — The green walking skeleton replaces the production identity contract with an impossible stronger fixture

- **Severity:** Blocker
- **Dimension:** External validity, fixture theater, and no-test-only-wiring criterion
- **Locations:**
  - `crates/overdrive-cli/src/commands/serve.rs:178`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:74`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:135`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:145`
  - `crates/overdrive-dataplane/src/mtls/outbound.rs:167`
  - `crates/overdrive-host/src/ca/rcgen_ca.rs:432`
  - `crates/overdrive-core/src/traits/ca.rs:685`

**Evidence:** The new CLI helper explicitly explains why it overrides `IdentityRead`: production issues URI-only SVIDs, while the production leg-B client always constructs `ServerName("peer.overdrive.local")`. `TestIdentity::mint` then creates a materially different leaf with both a SPIFFE URI SAN and the otherwise-unavailable DNS SAN, gives it both client and server EKUs, and returns that same already-minted identity for every allocation at all times. Production `RcgenCa::issue_svid` is not merely currently URI-only; the public `Ca` contract guarantees exactly one URI SAN and no second SAN. Standard rustls WebPKI DNS-name verification cannot accept that production leaf for `peer.overdrive.local`.

The fixture also bypasses production identity availability and allocation mapping: `IdentityMgr` starts with no held SVID and is populated by the SVID lifecycle after allocations become Running, while the double returns `Some` before either service or VM exists. Consequently, all four green metal scenarios can pass even though the same `serve` path without `mtls_identity_override` cannot complete the leg-B handshake. This is precisely the deletion-test failure: replacing the harness helper with `run_with_kek` changes no network production code but destroys the claimed end-to-end outcome.

**Required remediation:** Prove the walking skeleton through the production `IdentityMgr` and production CA/SVID lifecycle, including real per-allocation availability. Reconcile the production authn-only-v1 verifier with the exact single-URI-SAN SVID contract; do not silently add a DNS SAN in violation of the CA contract. If that requires intended-peer semantics from #242 or a design amendment, surface that dependency rather than certifying the loop with a stronger fixture. A test identity port may remain for focused fault tests, but it cannot be the sole acceptance proof for the production-drivable mesh round-trip.

### D2 — S-GTI-03's confidentiality and kTLS oracles can be satisfied by the wrong streams

- **Severity:** Blocker
- **Dimension:** Circular verification and security-test honesty
- **Locations:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:377`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:390`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:455`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:474`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:575`

**Evidence:** `scan_frames` first selects every loopback TCP stream touching the peer port, but it increments `plaintext_marker_hits` only inside `if records > 0`. A cleartext-only peer-wire stream is therefore excluded from the zero-cleartext oracle by the very fact the test is meant to reject. If a regression creates a clear path alongside any TLS-bearing connection, S-GTI-03 can observe TLS records, ignore the clear stream's request/response markers, and pass. The assertion text accurately exposes the circular scope: it promises zero markers only on a “TLS-bearing peer-wire stream,” while the criterion requires zero cleartext on the peer wire.

`ktls_records_for_port` has a second correlation gap. It concatenates all `ss -tie` records whose header contains `:18951`; `poll_until_ktls` then searches the combined text for ULP, TLS 1.3, RX, and TX tokens. Those facts need not belong to the same socket or to the AF_PACKET flow carrying the litmus. The fixed port and serialization reduce accidental collisions in today's run but do not make the security oracle structurally honest.

**Required remediation:** Identify the exact leg-B/leg-C connection tuple(s) independently of whether their payload parses as TLS, scan every captured payload byte on those peer-wire tuples for both plaintext markers, and require TLS application-data records in both correlated directions. Read kTLS state per exact socket/tuple and require ULP, TLS version, RX, and TX on the intended live connection rather than across concatenated port matches.

### D3 — New and transitioned acceptance tests violate mandatory Contract Shape declarations and preservation proofs

- **Severity:** Blocker
- **Dimension:** Contract Shape Compliance
- **Locations:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:558`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:572`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:606`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:689`
  - `crates/overdrive-cli/tests/acceptance/render_workload_describe.rs:452`
  - `crates/overdrive-control-plane/tests/acceptance/api_type_shapes.rs:184`

**Evidence:** None of the six step-touched acceptance tests has the exact mandatory Outcome Anchor. Four tests declare `bounded-change` but have no declared state universe, exact allowed delta, and full complement equality. S-GTI-07's two describe snapshots and guest-vs-transit negative assertion are useful behavioral evidence, but they do not prove the complement of the response or rendered surface. The pure render acceptance test checks only one `contains` substring. The transitioned API acceptance test has no Contract Shape declaration at all and adds `workload_addr: Some(...)` while asserting only `round_tripped.rows.len() == 1`; dropping or corrupting the new field leaves it green.

**Required remediation:** Add the exact Outcome Anchor to every affected acceptance-test docstring and classify the API test. For each bounded-change test, define the loose observable projection, declare the exact permitted delta, and assert equality of the whole complement. Strengthen the API round-trip to compare the full populated response or at minimum the exact `workload_addr` plus full adjacent-field complement. Preserve S-GTI-03's full frame snapshot while correcting D2's filtering/correlation defect.

### D4 — The gate flip and S-VM-74 removal leave security-critical documentation asserting the opposite behavior

- **Severity:** Low
- **Dimension:** RPP L1 documentation accuracy
- **Locations:**
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:479`
  - `crates/overdrive-cli/src/commands/serve.rs:153`
  - `crates/overdrive-cli/tests/integration/vm_walking_skeleton.rs:137`
  - `crates/overdrive-cli/tests/integration/vm_walking_skeleton.rs:181`
  - `crates/overdrive-cli/tests/integration/vm_walking_skeleton.rs:221`
  - `crates/overdrive-cli/tests/integration/vm_walking_skeleton.rs:511`
  - `crates/overdrive-cli/tests/integration/vm_walking_skeleton.rs:1119`
  - `.config/nextest.toml:399`

**Evidence:** `MtlsInterceptWorker::start_alloc` still says it is fired for every exec allocation, gated on exact equality with `DriverType::Exec`, and that VM traffic never traverses the veth. This step makes each statement false. The walking-skeleton module and nextest configuration still say S-VM-74 exists, passes, and proves no listener/TPROXY install, even though this commit deletes that test because VM interception is now required. The CLI `run_with_kek` rustdoc likewise says S-VM-74 exists.

**Required remediation:** Update the worker contract to `Exec | Vm` and explain the tap-fed veth path. Mark the historical S-VM-74 contract as superseded by S-GTI-01/S-GTI-03, remove present-tense claims that the deleted test exists or is green, and correct the nextest rationale/counts without disturbing the module-wide serialization rule.

## External validity

**Status: FAIL**

The networking part of the slice is externally real: the tests use real `serve`, deploy real Exec and VM workloads, boot Cloud Hypervisor, resolve the mesh name through the guest resolver, and exercise the production nft/intercept/enforcement code. However, the accepting identity and its availability are supplied only by a test-only port implementation whose certificate violates the production SVID shape. The exact production entry path therefore is not demonstrated and, from the explicit CA and fixed-SNI contracts, cannot perform the asserted handshake as committed.

## Verification

| Verification | Result |
|---|---|
| `git diff --check c035ac38e02417b276aa344763ca5b6b1bc2ae3b 3a1ed07fe684d494eb1643a632aaae59fc0cf68b` | PASS |
| `cargo fmt --all -- --check` | PASS |
| Lima affected-package `cargo check --all-targets --features integration-tests,kvm-tests` | PASS |
| Lima affected-package `cargo clippy --all-targets --features integration-tests,kvm-tests -- -D warnings` | PASS |
| Focused Lima `nextest` for API/render/TAP-owner/Landlock/injection fallout | PASS — 8 passed, 0 failed, 1,084 skipped |
| Metal S-GTI-01/S-GTI-03/S-GTI-04/S-GTI-07 run | PASS — 4 passed, 0 failed, 239 skipped in 59.712 s |
| Metal post-run residue inspection | PASS — no guest netns or TUN/TAP links; no separate Cloud Hypervisor process; nft table retained shared exemption rules only, with no per-allocation TPROXY rule |
| D6 source audit | PASS — both install gates are `Exec | Vm`; teardown remains ungated |
| Worker/proxy parent-to-commit diff | PASS — zero diff |
| Contract Shape direct grep | FAIL — missing declarations/anchors and bounded complement proofs described in D3 |
| Production identity-contract audit | FAIL — exact URI-only SVID contract cannot satisfy fixed DNS-name verification; D1 |
| Mutation testing | NOT RUN — prohibited during individual roadmap steps |

The metal command was:

```text
cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests -E 'test(microvm_dials_a_mesh_peer_by_name_and_receives_the_reply) | test(the_guests_mesh_traffic_travels_the_peer_wire_as_mtls_never_in_the_clear) | test(the_same_guest_reaches_a_non_mesh_destination_in_the_clear) | test(the_operator_sees_the_microvm_workloads_own_mesh_address_not_its_transit_hop)'
```

## Quality gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — Exactly one acceptance active | PASS | The selected roadmap step explicitly activates the walking skeleton plus three companion metal criteria and the describe closeout. |
| G2 — Valid RED failure | PASS | Ordered DES RED event is `EXECUTED/PASS`. |
| G3 — Assertion failure | PASS | Ordered DES RED event is `EXECUTED/PASS`. |
| G4 — No domain mocks | PASS | The identity double sits at a declared port boundary; its contract divergence is an external-validity/theater defect under D1, not an inside-hexagon mock. |
| G5 — Business language | PASS | Scenario names and assertions use mesh peer, plaintext, peer wire, non-mesh destination, and workload-address vocabulary. |
| G6 — All green | PASS | Relevant format, compile, lint, focused Lima, and four-scenario metal lanes passed. |
| G7 — 100% passing before commit | PASS | DES COMMIT event is `EXECUTED/PASS`. |
| G8 — Test budget | PASS | 9 step-touched tests ≤ budget 12. |
| G9 — No test weakening | PASS | The removed S-VM-74 contract was directly superseded by the approved requirement; surviving assertions were not weakened or skipped. |

These mechanical gates do not cure D1's production-path failure, D2's circular confidentiality oracle, or D3's mandatory Contract Shape failures.

## Test integrity and RPP scan

- **Test modification detected:** No prohibited weakening. The S-VM-74 removal is an explicit requirement reversal, not an assertion relaxation.
- **Testing theater detected:** Yes — D1's stronger always-present identity double and D2's TLS-conditioned plaintext scan create green evidence that does not prove the production/security claims.
- **Escalation verification:** Not applicable.
- **RPP levels scanned:** L1.
- **Cascade stopped at:** L1, after the stale and contradictory security-path documentation in D4.
- **RPP findings:** D4.

## Defect counts

| Severity | Count |
|---|---:|
| Blocker | 3 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 1 |
| **Total** | **4** |

## Final verdict

**NEEDS_REVISION**

The implementation is not eligible to advance to step 02-02. D1–D4 must be remediated by the original step 02-01 crafter, after which this same reviewer should perform iteration 2. The repository's no-iteration-cap rule applies until the verdict is `APPROVED`.

---

## Iteration 2

- **Review ID:** `code_rev_20260828_iteration_2`
- **Reviewer:** `nw-software-crafter-reviewer` (same step-specific isolated reviewer)
- **Remediation commit:** `fbda447ec747558db0ad8ed1e651e646f997db6e`
- **Parent:** `3a1ed07fe684d494eb1643a632aaae59fc0cf68b`
- **Subject:** `fix(guest-stack-transparent-mtls-intercept): prove production guest mTLS path`
- **Trailer:** `Step-Id: 02-01`
- **Final verdict:** **NEEDS_REVISION**

### Executive summary

The production-path and wire-oracle defects from iteration 1 are materially corrected. The metal harness now calls `run_with_kek`, uses the real `IdentityMgr` and workload CA, waits for the exact per-allocation issuance audit rows, and completes four out of four metal scenarios with URI-only production SVIDs. The test-only DNS-SAN identity helper and CLI `rcgen` dependency are gone. The outbound verifier delegates chain, validity, server-purpose, and handshake-signature verification to rustls/WebPKI, then requires exactly one project-valid SPIFFE URI; the transport `ServerName` is derived from the dialed IP and is not treated as identity. Existing host-path handshake, guardrail, and inbound enforcement tests remain green. The wire oracle now scans every peer-port byte stream for the plaintext litmus independently of TLS classification and correlates both directional TLS records and the complete kTLS facts to one exact socket tuple. D4's stale S-VM-74 and Exec-only documentation is corrected.

The step is still not approvable. D3 is only partially remediated: the two lifecycle bounded-change tests declare an intentionally narrow five-field projection while excluding multiple observable row fields, and S-GTI-07 compares two post-change responses rather than proving the response complement around the permitted `workload_addr` delta. The newly added certificate-verifier test also declares `bounded-change` for a return-only verifier and has no delta/complement proof. Separately, the new `dangerous()` custom verifier is a security boundary, but its test exercises only the one-URI success and two-URI rejection. It does not exercise zero URI SANs or an untrusted outbound chain, so neither the exact cardinality underflow nor the delegated trust-anchor check is regression-locked. D3 and D5 must be remediated before step 02-02.

### Iteration-1 finding dispositions

| Finding | Disposition | Evidence |
|---|---|---|
| D1 — test-only impossible identity contract | **RESOLVED**, with verifier-test gap tracked as D5 | `spawn_mtls_server` calls `run_with_kek`; `TestIdentity`, `run_with_kek_and_mtls_identity`, the DNS SAN, and the CLI `rcgen` dev-dependency are removed. Both allocations are observed with their exact production `spiffe://overdrive.local/workload/<workload>/alloc/<alloc>` issuance summaries. The 4/4 metal pass proves the URI-only production leaf is accepted through the real `IdentityMgr` lifecycle. |
| D2 — circular confidentiality/kTLS oracle | **RESOLVED** | `scan_frames` counts request/response markers across every payload stream touching port 18951 before any TLS classification. `KtlsSocketEvidence` holds one parsed source/destination tuple and one `ss` record; that same tuple indexes AF_PACKET data in both directions, and one record must contain ULP, TLS 1.3, RX, and TX. |
| D3 — Contract Shape | **PARTIALLY RESOLVED; BLOCKER REMAINS** | Exact declarations and Outcome Anchors are present, the API and pure-render tests now use whole-value/complement equality, and S-GTI-03 uses the full captured packet-event stream. The lifecycle/S-GTI-07 universes and the new verifier test still fail semantic shape review; see D3 below. |
| D4 — stale Exec/S-VM-74 docs | **RESOLVED** | Worker rustdoc now states `Exec | Vm` and the tap-fed host-veth path. CLI, walking-skeleton, and nextest text mark S-VM-74 historical/superseded and preserve the module-wide serialization rule. |

### Contract Shape Compliance

**Overall: FAIL**

| Check | Status | Evidence |
|---|---|---|
| Exact per-test declarations | PASS | All ten step-touched behavioral tests have an exact `CONTRACT_SHAPE` line. |
| Outcome anchor | PASS | All six step-touched acceptance tests have the exact `Outcome anchor: DISCUSS Elevator Pitch` line. |
| Banned test-name regex | PASS | No step-touched test matches `^test_.*(returns_\d+|exit_code|calls_.*_once|status_code|http_\d+)`. |
| Unbounded-preservation mechanism | PASS | S-GTI-03 audits the complete raw AF_PACKET event stream and evaluates plaintext over every peer-port stream; the transitioned API test compares the complete populated JSON value. |
| Bounded-change delta and complement | **FAIL** | `StableAllocProjection` covers only five row fields and explicitly excludes observable fields; S-GTI-07's response equality is after-vs-after, not permitted-delta complement equality; the new verifier test is incorrectly classified as bounded change and has no mutation universe. |
| Layer choice and external contract | PASS | The acceptance path enters through `serve`/`deploy`/`describe`, uses real production networking and identity composition, and hand-installs no intercept, address, route, or resolver fact. |

The mandated checker `src/des/cli/check_contract_shape_declarations.py` remains absent from this checkout and the installed nWave tree. Mechanical declaration results therefore come from a direct diff-scoped `rg` audit; semantic shape review remains the reviewer judgment required by the agent contract.

### Mechanical and scope evidence

- Remediation stat: **14 files changed, 695 insertions, 328 deletions**.
- Full step stat from `c035ac38e02417b276aa344763ca5b6b1bc2ae3b` through remediation: **25 files changed, 1,597 insertions, 580 deletions**.
- Commit parent is exactly the iteration-1 commit, and the required `Step-Id: 02-01` trailer is present.
- Remediation DES events are canonical and ordered: RED `2026-08-28T16:19:22Z`, GREEN `16:42:38Z`, COMMIT `16:43:15Z`, all `EXECUTED/PASS`. The commit is timestamped `16:43:25Z`.
- The two D6 predicates remain exactly `matches!(..., DriverType::Exec | DriverType::Vm)` at the fresh and restart Running arms.
- Both teardown calls remain gated only by `mtls_worker.is_some()`; no driver-kind condition was added.
- `mtls_intercept.rs`, `HostMtlsIntercept::install_outbound`, and `install_outbound_tproxy` remain byte-identical to the pre-step base. `MtlsInterceptWorker::start_alloc` behavior is unchanged; its remediation diff is documentation only.
- The sensitive dataplane expansion is necessary to make the production issuer's URI-only SVID usable without inventing a DNS SAN. It is confined to the outbound handshake/config boundary plus the production `x509-parser` dependency; kTLS arm, pumps, inbound handshake, intercept rules, and enforcement supervision are unchanged.
- Harness expansion is related: it deletes the impossible identity fixture, adds exact tuple evidence and production identity observations, and strengthens Contract Shape assertions. The CLI Cargo change removes the now-unused `rcgen` test dependency.
- The nextest change is comments/count/rationale only; the `host-kernel-shared` module override is preserved.
- No assertion was weakened, no live step test was skipped or ignored, and no new fail-open/test-only identity branch remains. The five `#[should_panic]` RED scaffolds in the guest module belong to later roadmap steps and were not transitioned here.
- No mutation run or mutation-exclusion edit occurred, as required by this repository's single final-wave mutation-gate rule.

### D1 production identity and TLS audit

**Status: RESOLVED in implementation and acceptance path**

- `run_with_kek` leaves the production dataplane and mTLS composition enabled while substituting only the orthogonal cold-test KEK. The identity override is unset, so `HostMtlsEnforcement` receives the production `IdentityMgr`.
- The service SVID is observed before the VM deploy. The VM starts with the normal Running-to-issuance lifecycle; its retrying ordinary `TcpStream` succeeds only after the production held identity becomes available. Both audit summaries are matched to the exact allocation-shaped SPIFFE ID.
- `HostMtlsEnforcement::enforce_outbound` reads `svid_for(alloc)` and `current_bundle()` for every new connection and moves owned clones into that handshake. Re-issuance/bundle rotation is therefore visible to subsequent connections without a cached TLS config; existing connections retain their established session keys, as expected.
- `SpiffeServerVerifier` parses the exact peer leaf, delegates chain/time/server-purpose validation to `verify_server_cert_signed_by_trust_anchor`, delegates TLS 1.2/1.3 handshake-signature checks to the ring provider, and then requires a singleton URI list accepted by `SpiffeId::new`.
- The dialed peer IP supplies rustls' required `ServerName` transport value, while the verifier deliberately ignores DNS/IP name matching and does not claim intended-peer identity. This preserves the documented authn-only-v1 boundary; #242 still owns expected-peer equality.
- The production metal round trip and the existing host mTLS tests pass with this shape, so there is no observed SNI or host-path regression.

The test-protection gap around this new verifier is D5; it does not change the conclusion that the committed implementation is presently chain-validating and fail-closed by source inspection.

### D2 wire-oracle audit

**Status: RESOLVED**

- `FlowTuple` contains both IPv4 addresses and ports, so same-port unrelated streams do not collapse into one oracle bucket.
- Plaintext markers are counted over every captured payload stream touching the peer port, including cleartext-only and malformed streams. This eliminates the iteration-1 TLS-conditioned blind spot.
- The selected `ss` evidence is one record whose destination is the peer port and whose own text contains `tcp-ulp-tls`, TLS 1.3, `rxconf`, and `txconf`.
- AF_PACKET TLS application records must occur on that record's exact tuple in the request direction and its exact reverse tuple in the response direction.
- Guest success remains coupled to byte-exact request/reply semantics, and the distinct litmus markers are scanned independently of the positive TLS classifier.

### Remaining blocking findings

#### D3 — Bounded-change tests still use incomplete or non-causal complements

- **Severity:** Blocker
- **Dimension:** Contract Shape Compliance / universe-too-narrow prevention
- **Locations:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:479`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:647`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:813`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:879`
  - `crates/overdrive-dataplane/src/mtls/tls_config.rs:332`

`StableAllocProjection` includes only `{alloc_id, workload_id, node_id, resources, restart_count}`. S-GTI-01 and S-GTI-04 call that the full complement while omitting the observable `reason`, `workload_addr`, `started_at`, `exit_code`, `last_transition`, `error`, and `last_terminated` fields. The comments explicitly place several observable fields outside the universe rather than proving them unchanged or declaring them as allowed deltas. A regression that corrupts an omitted field while moving Running to Terminated therefore remains green.

S-GTI-07 does prove the renderer's complement by removing its exact one-section delta, but its response-level proof compares `described.snapshot` to a second post-change read. Stable repetition cannot show that adding `workload_addr` left every adjacent response field unchanged; the same collateral change would be present in both reads. The response needs a loose whole-response projection with the exact address slot removed, or an exact expected snapshot whose only admitted difference is that slot.

Finally, `spiffe_server_verifier_accepts_uri_only_svid_and_rejects_ambiguous_identity` is a deterministic return-only verifier test yet declares `CONTRACT_SHAPE: bounded-change`. It mutates no observable state and supplies neither a before/after universe nor complement equality. It should use the correct pure-function shape and assert its output matrix accordingly.

**Required remediation:** For S-GTI-01/S-GTI-04, compare the complete observable row complement after removing only explicitly permitted lifecycle/ephemeral deltas; do not redefine the universe to five convenient fields. For S-GTI-07, prove the whole response complement around `workload_addr`, not two identical post-change reads. Reclassify the verifier test to its actual pure-function shape.

#### D5 — The new dangerous certificate verifier lacks regression proof for two fail-closed duties

- **Severity:** Blocker
- **Dimension:** Security-test completeness and test honesty
- **Locations:**
  - `crates/overdrive-dataplane/src/mtls/tls_config.rs:142`
  - `crates/overdrive-dataplane/src/mtls/tls_config.rs:196`
  - `crates/overdrive-dataplane/src/mtls/tls_config.rs:291`
  - `crates/overdrive-dataplane/src/mtls/tls_config.rs:334`

The custom `dangerous()` verifier now owns two security-critical decisions that ordinary DNS/IP WebPKI previously bundled: trust-chain/time/server-purpose validation and the X.509-SVID URI profile. Its only new test proves a trusted one-URI leaf succeeds and a trusted two-URI leaf fails. It never constructs the cardinality-underflow case (`mint_chain(&[])` or a DNS-only/no-SAN leaf), so the exact singleton contract is not fully exercised. It also never verifies that a one-URI leaf under an untrusted root is rejected, so deleting or bypassing `verify_server_cert_signed_by_trust_anchor` would leave the new test and the production-positive metal test green. The existing wrong-CA guardrail is inbound and exercises `WebPkiClientVerifier`, not this new outbound custom verifier.

**Required remediation:** Add a table-driven/focused proof through the verifier or outbound enforcement boundary covering at least: trusted + exactly one URI succeeds; trusted + zero URI fails; trusted + two URIs fails; untrusted + exactly one URI fails. Keep the cases within the existing test budget and preserve the no-DNS-fixture production acceptance path.

### Test budget

| Behaviors | Budget (`2 × behaviors`) | Step-touched tests | Status |
|---:|---:|---:|---|
| 6 | 12 | 10 | PASS |

The remediation adds one focused verifier test to the prior nine. Expanding that test as a table, or adding at most two focused cases, remains within budget.

### Verification

| Verification | Result |
|---|---|
| `git diff --check 3a1ed07f..fbda447e` and full-step diff check | PASS |
| `cargo fmt --all -- --check` | PASS |
| Lima affected-package `cargo check --all-targets --features integration-tests,kvm-tests` | PASS |
| Lima affected-package `cargo clippy --all-targets --features integration-tests,kvm-tests -- -D warnings` | PASS |
| Focused Lima verifier/API/render/TAP-owner/Landlock lane | PASS — 6 passed, 0 failed, 1,256 skipped |
| Existing Lima host mTLS handshake/guardrail/inbound lane | PASS — 3 passed, 0 failed, 167 skipped in 33.779 s |
| Metal S-GTI-01/S-GTI-03/S-GTI-04/S-GTI-07 | PASS — 4 passed, 0 failed, 239 skipped in 60.212 s |
| Post-metal residue | PASS — no guest netns, TUN/TAP link, or Cloud Hypervisor process; `table ip overdrive-mtls` contains only the two shared mark-exemption rules and no per-allocation TPROXY rule |
| D6 fresh/restart source audit | PASS — both install gates are `Exec | Vm`; teardown remains ungated |
| Contract Shape semantic audit | FAIL — D3 |
| New outbound verifier security matrix | FAIL — D5 |
| Mutation testing | NOT RUN — prohibited during an individual roadmap step |

### Quality gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — selected acceptance slice | PASS | The roadmap explicitly activates the walking skeleton and its three companion metal outcomes. |
| G2 — valid RED | PASS | Remediation RED is ordered and `EXECUTED/PASS`. |
| G3 — assertion failure | PASS | Remediation RED is recorded `EXECUTED/PASS`; no scaffold/compile-only substitution is present. |
| G4 — no domain mocks | PASS | The live acceptance path uses production identity/networking; the cold-test KEK remains an orthogonal port substitution. |
| G5 — business language | PASS | Live scenario names retain the DISTILL outcome vocabulary. |
| G6 — all executed tests green | PASS | Format, compile, lint, focused Lima, host mTLS, and metal lanes are green. |
| G7 — green before commit | PASS | Ordered remediation COMMIT event is `EXECUTED/PASS`. |
| G8 — test budget | PASS | 10 ≤ 12. |
| G9 — no prohibited test modification | PASS | Remediation only strengthens/corrects the live tests and removes the invalid identity fixture; no surviving assertion is weakened. |

### Test integrity, external validity, and RPP

- **External validity:** PASS. Real CLI composition, deploy, guest boot, DNS resolution, production identity issuance, intercept, enforcement, and describe path are exercised.
- **Testing theater:** None remains in the live metal path. D5 is a missing negative security lock, not evidence that the currently green production path is fixture-authored.
- **Test modification violation:** None. All remediation changes are additive/strengthening or delete the previously rejected impossible fixture.
- **Escalation verification:** Not applicable.
- **RPP levels scanned:** L1–L3 across the expanded sensitive surface.
- **Cascade stopped at:** L3 after Contract Shape/test-security defects; no additional production-code smell was found.
- **RPP findings:** D3 and D5.

### Iteration-2 defect counts

| Severity | Count |
|---|---:|
| Blocker | 2 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |
| **Total** | **2** |

### Iteration-2 final verdict

**NEEDS_REVISION**

The production walking skeleton and wire-security implementation are now externally valid and all requested runtime lanes are green, but zero-defect approval is blocked by D3 and D5. Return those findings to the original step-02-01 crafter, then re-run this same step-specific reviewer. The repository's no-iteration-cap rule remains in force; do not advance to 02-02 until an on-disk iteration records `APPROVED`.

---

## Iteration 3

- **Review ID:** `code_rev_20260828_iteration_3`
- **Reviewer:** `nw-software-crafter-reviewer` (same step-specific isolated reviewer)
- **Remediation commit:** `91aa6a50c68c1cdf4c3da5ad3a57f39841694396`
- **Parent:** `fbda447ec747558db0ad8ed1e651e646f997db6e`
- **Subject:** `test(guest-stack-transparent-mtls-intercept): close verifier and contract-shape gaps`
- **Trailer:** `Step-Id: 02-01`
- **Final verdict:** **NEEDS_REVISION**

### Executive summary

Iteration 3 resolves most of the remaining test-contract work. The lifecycle helper now starts from the complete `AllocStatusRowBody`, explicitly admits the five lifecycle fields that legitimately change from Running to Terminated, and compares the exact complement. The outbound verifier test is correctly classified as a pure function and now exercises a typed four-case matrix: a trusted singleton URI succeeds, trusted zero and two URI identities fail with the URI-cardinality category, and an untrusted singleton URI fails with the distinct trust-anchor category. The focused Lima lane, the three existing host-mTLS regressions, and all four metal scenarios pass. Production identity, verifier, wire, D6, and teardown code did not change in this remediation.

One blocker remains. S-GTI-07 still has no real pre-change response. It clones the already post-change snapshot, deletes `workload_addr`, reinserts that same value, and compares the reconstruction to its source. That equality is true by construction and copies every possible collateral change into both sides; it cannot prove that production changed only `workload_addr`. The renderer comparison repeats the same synthesized-baseline pattern. This is the exact causal-baseline defect called out in iteration 2, so zero-defect approval and advancement to step 02-02 remain blocked.

### Iteration-2 finding dispositions

| Finding | Disposition | Evidence |
|---|---|---|
| D3 — incomplete/non-causal bounded-change complements | **PARTIALLY RESOLVED; BLOCKER REMAINS** | S-GTI-01 and S-GTI-04 now compare a cloned complete `AllocStatusRowBody` after normalizing exactly `{state, reason, workload_addr, started_at, last_transition}` and assert the explicit field deltas. The verifier declaration is now `pure-function`. S-GTI-07 still derives its alleged pre-change baseline from the post-change observation, so its complement equality is non-causal and tautological. |
| D5 — incomplete outbound verifier security matrix | **RESOLVED** | The focused test distinguishes `Accepted`, `UriCardinalityRejected`, and `TrustAnchorRejected` and covers trusted 1 URI, trusted 0 URI, trusted 2 URI, and untrusted 1 URI. The lane passes in Lima. |

### Contract Shape Compliance

**Overall: FAIL**

| Check | Status | Evidence |
|---|---|---|
| Exact per-test declarations | PASS | The remediated verifier uses the exact `/// CONTRACT_SHAPE: pure-function.` declaration; the step's acceptance declarations remain exact. |
| Outcome anchors | PASS | All six step-touched acceptance tests retain the exact `Outcome anchor: DISCUSS Elevator Pitch` line. |
| Banned test-name regex | PASS | No step-touched test matches the banned output-shaped naming pattern. |
| Lifecycle observable universe | PASS | `without_permitted_lifecycle_delta` clones the full thirteen-field `AllocStatusRowBody`, normalizes only five named lifecycle fields, and equality checks all eight remaining fields. The helper separately asserts the Running/Terminated state, address retirement, and changed reason/timestamps/transition record. Both consuming metal scenarios pass. |
| Return-only verifier shape | PASS | The verifier is a deterministic return-only matrix and now declares `pure-function`; no before/after state fiction remains. |
| S-GTI-07 bounded-change causality | **FAIL** | Lines 879–896 manufacture `pre_change` from `post_change` and then reconstruct `expected_post_change` from that clone. Lines 908–916 manufacture the renderer baseline from the same observed output. Neither side is an independently obtained pre-change observation or fixture. |
| Layer choice and external contract | PASS | The metal path still enters through real `serve`, `deploy`, `describe`, and `stop`, with production identity and networking composition and no hand-installed intercept/address/route/resolver fact. |

The mandated checker `src/des/cli/check_contract_shape_declarations.py` remains absent from this checkout and the installed nWave tree. Mechanical declaration results therefore come from a direct diff-scoped audit; semantic classification is this reviewer's required judgment.

### Mechanical and scope evidence

- Iteration-3 remediation stat: **3 files changed, 175 insertions, 92 deletions**.
- Full step stat from `c035ac38e02417b276aa344763ca5b6b1bc2ae3b` through iteration 3: **25 files changed, 1,681 insertions, 581 deletions**.
- The commit parent is exactly the iteration-2 remediation commit, and `Step-Id: 02-01` is present.
- Iteration-3 DES phases are canonical and ordered: RED `2026-08-28T17:02:59Z`, GREEN `17:10:05Z`, COMMIT `17:10:21Z`, all `EXECUTED/PASS`; the commit follows at `17:10:33Z`.
- The remediation changes only the guest acceptance test, the dataplane verifier's `#[cfg(test)]` module, and the execution log. No production authentication, handshake, intercept, kTLS, splice, D6, or teardown implementation changed.
- The D6 install predicates remain exactly the two `matches!(spec.driver.driver_type(), DriverType::Exec | DriverType::Vm)` gates, at fresh and restart Running paths.
- Both teardown sites remain gated solely by `mtls_worker` presence; neither has a driver-kind condition.
- `MtlsInterceptWorker` and `install_outbound_tproxy` remain unchanged. No test-only identity or relaxed-authentication branch was introduced.
- No test was skipped or ignored. The five `#[should_panic]` functions remain roadmap-owned RED scaffolds for later steps, not transitioned 02-01 tests.
- No mutation test or mutation-exclusion edit occurred, as required by the repository's single final DELIVER-wave mutation gate.

### D3 lifecycle remediation audit

**Status: RESOLVED for S-GTI-01 and S-GTI-04**

`without_permitted_lifecycle_delta` takes an owned clone of the complete public row rather than reconstructing a narrow projection. It normalizes only state, reason, workload address, started-at timestamp, and last-transition record. The caller pins Running to Terminated, proves the live address is present then retired, proves both timestamp-bearing observations exist and change, and proves the reason and transition record change. Equality of the normalized clones therefore covers allocation identity, workload identity, node identity, resources, exit code, error, restart count, and last-terminated without an omitted-field blind spot. The two consuming metal tests pass against real Running and terminal rows.

This closes iteration 2's lifecycle-universe defect. The helper's permitted field set is explicit, its complement is the full remaining row, and compiler-added future fields automatically join that complement because the comparison operates on the cloned struct rather than a handwritten projection.

### D5 verifier matrix audit

**Status: RESOLVED**

- The verifier test is correctly declared `pure-function`.
- Each trusted case uses its own root as the verifier's trust anchor, so the singleton success and zero/two URI failures reach the URI-profile decision honestly.
- The untrusted singleton leaf chains under one root while the verifier receives an unrelated root. It produces `InvalidCertificate(UnknownIssuer)`, distinct from the cardinality cases' `ApplicationVerificationFailure`.
- The helper rejects any unexpected rustls category instead of collapsing all failures to `is_err()`.
- The four cases remain one focused behavioral test, so the step stays within budget.
- Source inspection confirms the production verifier still calls `verify_server_cert_signed_by_trust_anchor` before `validate_single_spiffe_uri`; the remediation adds no test-only escape and no weaker authentication path.

### Remaining blocking finding

#### D3 — S-GTI-07's alleged pre-change baseline is synthesized from the post-change response

- **Severity:** Blocker
- **Dimension:** Contract Shape Compliance / causal bounded-change proof / test integrity
- **Locations:**
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:853`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:879`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:894`
  - `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:908`

The test discards the output returned by `poll_until_running` and makes one post-change `describe` call. It serializes that post-change snapshot, clones it into a variable named `pre_change`, removes `workload_addr`, clones the result, reinserts the same address, and compares the reconstruction with the source snapshot. For any collateral mutation `X` in any other response field, both `post_change.X` and `expected_post_change.X` originate from the same value, so the assertion stays green. It proves only that the test can delete and reinsert a JSON member.

The rendered-output assertion has the same causality problem: `address_free` is a clone of `described`, and `baseline` is rendered from that clone after clearing the address. It verifies local renderer additivity for this already-changed object, not that the observed production response changed only in the address slot from a genuine prior contract.

Iteration 2 explicitly required a pre-change baseline rather than two post-change reads. Iteration 3 replaces those two reads with an even less independent self-derived equality. The positive address and anti-transit assertions are valuable and pass on metal, but they do not close the whole-response complement.

**Required remediation:** Compare the observed post-change response and render against an independently sourced, genuine pre-change contract or observation, with `workload_addr`/the one Addresses section declared as the sole allowed delta and exact equality over the complete complement. Do not construct either baseline by cloning the post-change value and deleting the field under test. Preserve the positive guest-address and negative transit-address assertions.

### Regression audit

- **Production identity:** PASS. `run_with_kek` still composes the real `IdentityMgr`; exact service and VM allocation SPIFFE issuance summaries remain asserted. No DNS-SAN identity fixture or `run_with_kek_and_mtls_identity` seam is present.
- **Outbound authentication:** PASS. Chain/time/server-purpose verification remains delegated to rustls/WebPKI, exact one project-valid SPIFFE URI remains required, and the four-case matrix regression-locks the trust and cardinality duties.
- **Wire oracle:** PASS. Plaintext scanning remains independent of TLS classification across every peer-port stream; the exact `ss` tuple still keys both AF_PACKET directions and one record still must contain TLS 1.3 plus RX/TX kTLS state.
- **D6 and teardown:** PASS. Exactly two Exec-or-VM install gates remain; both stop paths remain ungated by driver kind.
- **Metal behavior:** PASS. Dial-by-name round trip, encrypted peer wire, non-mesh plaintext pass-through, and canonical guest address all pass through the real composition root.
- **Regression caveat:** S-GTI-07's complement assertion is not an effective regression lock because its expected value is derived from the actual value under test.

### Test budget

| Behaviors | Budget (`2 × behaviors`) | Step-touched tests | Status |
|---:|---:|---:|---|
| 6 | 12 | 10 | PASS |

The verifier's four input cases remain one focused test. No new behavioral test was added in iteration 3.

### Verification

| Verification | Result |
|---|---|
| `git diff --check fbda447e..91aa6a50` and full-step diff check | PASS |
| `cargo fmt --all -- --check` | PASS |
| Lima affected-package `cargo check --all-targets --features integration-tests,kvm-tests` | PASS |
| Lima affected-package `cargo clippy --all-targets --features integration-tests,kvm-tests -- -D warnings` | PASS |
| Focused Lima verifier/API/render/TAP-owner lane | PASS — 6 passed, 0 failed, 2,899 skipped |
| Existing Lima host mTLS handshake/guardrail/inbound lane | PASS — 3 passed, 0 failed, 2,902 skipped in 33.788 s |
| Metal S-GTI-01/S-GTI-03/S-GTI-04/S-GTI-07 | PASS — 4 passed, 0 failed, 2,901 skipped in 59.227 s |
| Post-metal residue | PASS — no guest netns, TUN/TAP link, or Cloud Hypervisor process; `table ip overdrive-mtls` contains only the two shared mark-exemption rules and no per-allocation TPROXY rule |
| DES phase order and commit trailer | PASS |
| Contract Shape semantic audit | **FAIL — D3 causal-baseline defect** |
| Outbound verifier security matrix | PASS |
| Mutation testing | NOT RUN — prohibited during an individual roadmap step |

### Quality gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — selected acceptance slice | PASS | The roadmap explicitly activates S-GTI-01, S-GTI-03, S-GTI-04, and S-GTI-07. |
| G2 — valid RED | PASS | Iteration-3 RED is ordered and `EXECUTED/PASS`. |
| G3 — assertion failure | PASS | The remediation RED is recorded as a real executed failure-to-green cycle; no compile-only/scaffold substitute is present. |
| G4 — no domain mocks | PASS | Runtime acceptance uses production identity/networking; only the orthogonal cold-test KEK adapter is substituted. |
| G5 — business language | PASS | Scenario names remain the DISTILL outcomes. |
| G6 — all executed tests green | PASS | Format, check, clippy, focused Lima, host mTLS, and metal lanes are green. |
| G7 — green before commit | PASS | GREEN and COMMIT precede the commit timestamp. |
| G8 — test budget | PASS | 10 ≤ 12. |
| G9 — no prohibited test weakening | **FAIL under D3** | The independent second production read was removed and replaced by a reconstruction from the sole post-change value. The replacement equality cannot detect collateral response changes. |

### Test integrity, external validity, and RPP

- **External validity:** PASS for the production behavior. The real CLI composition, deploy, VM boot, guest DNS, production identity, intercept, enforcement, and describe paths remain exercised.
- **Testing theater:** Present only in S-GTI-07's response-complement assertion: its expected value is reconstructed from the actual post-change value and therefore cannot falsify the claimed sole delta.
- **Test modification violation:** D3 removes an independent-read equality and replaces it with a weaker self-derived equality. This is recorded as the same blocker rather than double-counted.
- **Escalation verification:** Not applicable.
- **RPP levels scanned:** L1–L3 over the iteration-3 test-only remediation and the unchanged sensitive production seams.
- **Cascade stopped at:** L3 after the causal Contract Shape defect; no new production-code smell was found.
- **RPP findings:** D3 only.

### Iteration-3 defect counts

| Severity | Count |
|---|---:|
| Blocker | 1 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |
| **Total** | **1** |

### Iteration-3 final verdict

**NEEDS_REVISION**

D5 and the lifecycle portion of D3 are closed, production/runtime verification is green, and no authentication or intercept regression was found. S-GTI-07 still lacks the independently sourced pre-change baseline required to prove that `workload_addr` is the sole response delta. Return this one blocker to the original step-02-01 crafter and re-run this same reviewer; do not advance to step 02-02 until a later on-disk iteration records `APPROVED`.

---

## Iteration 4

- **Review ID:** `code_rev_20260828_iteration_4`
- **Reviewer:** `nw-software-crafter-reviewer` (same step-specific isolated reviewer)
- **Remediation commit:** `3a7e92f1e8f8fbd7369287fc54161f96a21e102c`
- **Parent:** `91aa6a50c68c1cdf4c3da5ad3a57f39841694396`
- **Subject:** `test(guest-stack-transparent-mtls-intercept): use independent describe baseline`
- **Trailer:** `Step-Id: 02-01`
- **Final verdict:** **APPROVED**

### Executive summary

Iteration 4 closes the last D3-S-GTI-07 blocker with an independently sourced oracle. The test now takes a copy-on-write snapshot of the production observation database, opens that copy through `LocalObservationStore`, reads the real allocation and certificate-audit facts, and projects them into an explicit frozen pre-change response contract without reading or cloning the post-change `describe` result. It then adds exactly `workload_addr` to that independent contract and requires whole-snapshot equality with the live HTTP/CLI response. The four non-serializable `WorkloadDescribeOutput` wrapper fields are compared separately, so the complete command response is covered.

The rendered baseline is independently frozen as the exact pre-step string contract. The live renderer must first reproduce that contract from the address-free independent response; the actual post-change output must contain the canonical Addresses section exactly once, and removing only that exact section must reproduce the frozen contract byte-for-byte. The real guest `/30` positive assertion and transit-veth negative assertion remain. A change to any unrelated API field or any unrelated rendered byte now fails.

S-GTI-07 passes on metal and leaves no guest namespace, TUN/TAP, Cloud Hypervisor, or per-allocation nft residue. The iteration touches no production implementation and does not change the other acceptance scenarios, verifier matrix, identity path, wire oracle, D6 gates, or teardown. Those previously resolved findings remain sound. No new defect was found.

### Iteration-3 finding disposition

| Finding | Disposition | Evidence |
|---|---|---|
| D3-S-GTI-07 — post-derived baseline cannot prove the sole response delta | **RESOLVED** | `genuine_pre_change_contract` reads a CoW snapshot of production durable observation/audit inputs, never the post-change response. It constructs the frozen pre-step API contract with `workload_addr: None`; the test adds only the expected guest address and compares the entire serialized snapshot. `frozen_pre_change_render_contract` fixes the historical renderer output independently, and the actual render with exactly one Addresses section removed must equal it byte-for-byte. |

### Contract Shape Compliance

**Overall: PASS**

| Check | Status | Evidence |
|---|---|---|
| Exact declarations | PASS | S-GTI-07 retains `CONTRACT_SHAPE: bounded-change`; the verifier retains exact `pure-function`; all other step declarations remain exact. |
| Outcome anchors | PASS | All six step-touched acceptance tests retain the exact `Outcome anchor: DISCUSS Elevator Pitch` line. |
| Banned test-name regex | PASS | No step-touched test matches the banned output-shaped naming pattern. |
| Independent baseline | PASS | The pre-change contract is built from a separate redb snapshot plus deploy-input facts and an explicit frozen historical schema. It never reads or clones `described` or its snapshot. |
| Complete response universe | PASS | All four `WorkloadDescribeOutput` wrapper fields are compared, and the complete `AllocStatusResponse` is serialized and compared as a whole after adding only `rows[0].workload_addr`. |
| Exact response delta | PASS | The independent baseline holds `workload_addr: None`; `expected_post_change` changes only that slot to the derived guest `/30` address before whole-value equality. |
| Complete rendered universe | PASS | The address-free live renderer must equal a separately formatted frozen contract, then the post-change rendered value with one exact Addresses section removed must equal that same contract byte-for-byte. |
| Positive and negative address facts | PASS | The live row equals `VmTapPlan.guest_addr` and differs from `WorkloadNetnsPlan.workload_addr`; the durable row is independently asserted to contain the guest address. |
| Unrelated-change sensitivity | PASS | Any other API-field mutation differs from the independently built snapshot; any other renderer change differs from the frozen string before or after the one permitted section is removed. |
| Lifecycle and verifier shapes | PASS | The complete-row lifecycle complements from iteration 3 remain unchanged, and the outbound verifier remains a return-only pure-function matrix. |

The repository still contains no `src/des/cli/check_contract_shape_declarations.py`, so mechanical declaration checks use a direct diff-scoped audit. The semantic bounded-change review above satisfies the reviewer-owned part of the Contract Shape gate.

### Independent-baseline audit

The remediation does not rename a post-change clone or clear a field on the actual value. Its oracle has three independent layers:

1. `cp --reflink=always` takes a CoW snapshot of the production `observation.redb` while the long-running VM is stable. `LocalObservationStore` opens that separate file and supplies the real allocation row and issuance-audit rows.
2. The pre-change API contract is explicitly reconstructed from those durable facts, the independent deploy result, and the submitted workload's fixed inputs. It covers every current `AllocStatusRowBody` and `AllocStatusResponse` field while intentionally omitting the one historical absence, `workload_addr`.
3. The pre-change renderer contract is independently formatted in the acceptance test. The only production renderer change from the pre-step base is the Addresses section, and the metal assertion proves both that the no-address live renderer matches the frozen historical contract and that the post-change output differs by exactly one canonical section.

The API comparison includes workload id, spec digest, desired/running replica counts, the complete allocation row, restart budget, kind, VIP, listeners, issued certificates, probes, and probe results. Wrapper identity, digest, allocation count, and empty-state message are compared separately because `WorkloadDescribeOutput` is not serializable. Future struct-field additions force the explicit contract constructor to change at compile time; unrelated current-field changes fail whole-value equality.

The renderer contract includes the Job header, digest, verdict, complete attempt table, declared memory, and all four issued-certificate lines. It deliberately contains no Addresses section. The post-change render must contain the exact allocation/address section once; `replacen` of that asserted unique section must leave the entire frozen contract with no other byte difference.

### Mechanical and scope evidence

- Iteration-4 remediation stat: **3 files changed, 216 insertions, 36 deletions**.
- Full step stat from `c035ac38e02417b276aa344763ca5b6b1bc2ae3b` through iteration 4: **25 files changed, 1,866 insertions, 586 deletions**.
- The commit parent is exactly the iteration-3 remediation commit, and the required `Step-Id: 02-01` trailer is present.
- Iteration-4 DES events are canonical and ordered: RED `2026-08-28T17:24:08Z`, GREEN `17:43:47Z`, COMMIT `17:44:04Z`, all `EXECUTED/PASS`; the commit follows at `17:44:12Z`.
- The source delta is confined to the S-GTI-07 integration oracle. The `Cargo.toml` change only clarifies the existing `overdrive-store-local` dev-dependency comment; it adds no dependency edge.
- Production CLI rendering, handler projection, identity, TLS verification, outbound enforcement, intercept, kTLS, splice, D6, and teardown sources are byte-identical to iteration 3.
- Within the guest acceptance module, the existing identity, wire, lifecycle, and non-mesh scenario bodies are unchanged; only imports, independent-oracle helpers, and S-GTI-07 changed.
- No live test is ignored or skipped. The five `#[should_panic]` tests remain explicit RED scaffolds assigned to later roadmap steps.
- No mutation command was run and no mutation exclusion changed, as required for an individual roadmap step.

### Previously resolved finding regression audit

- **Production identity:** PASS. The harness still enters through `run_with_kek`, composes the real `IdentityMgr`, and asserts exact service and VM allocation SPIFFE issuance. No DNS-SAN identity fixture or identity override reappears.
- **Outbound verifier:** PASS. Production still calls rustls/WebPKI chain, validity, and server-purpose verification before enforcing a singleton project-valid SPIFFE URI. The unchanged four-case typed matrix passes again in Lima.
- **Wire confidentiality:** PASS by unchanged source and prior direct metal evidence. Plaintext scanning remains independent of TLS classification over all peer-port streams; exact tuple correlation, bidirectional TLS application records, and one-record TLS 1.3 RX/TX kTLS requirements remain intact.
- **Lifecycle complements:** PASS. S-GTI-01 and S-GTI-04 retain the complete `AllocStatusRowBody` complement and five explicit lifecycle deltas from iteration 3.
- **D6 gates:** PASS. Exactly two fresh/restart predicates remain `DriverType::Exec | DriverType::Vm`.
- **Teardown:** PASS. Both `worker.stop_alloc` sites remain conditioned only on `mtls_worker` presence, with no driver-kind guard.
- **Authentication/test seams:** PASS. No fail-open verifier path, relaxed-auth branch, test-only identity composition, or new production seam exists.

### Test budget

| Behaviors | Budget (`2 × behaviors`) | Step-touched tests | Status |
|---:|---:|---:|---|
| 6 | 12 | 10 | PASS |

Iteration 4 adds oracle helpers but no behavioral test. The step remains at ten tests for six behaviors.

### Verification

| Verification | Result |
|---|---|
| `git diff --check 91aa6a50..3a7e92f1` and full-step diff check | PASS |
| `cargo fmt --all -- --check` | PASS |
| Lima affected-package `cargo check --all-targets --features integration-tests,kvm-tests` | PASS |
| Lima affected-package `cargo clippy --all-targets --features integration-tests,kvm-tests -- -D warnings` | PASS |
| Focused Lima outbound-verifier matrix | PASS — 1 passed, 0 failed, 2,904 skipped |
| Metal S-GTI-07 independent response/render contract | PASS — 1 passed, 0 failed, 2,904 skipped in 15.149 s |
| Post-metal residue | PASS — no guest netns, TUN/TAP link, or Cloud Hypervisor process; `table ip overdrive-mtls` contains only the two shared mark-exemption rules and no per-allocation TPROXY rule |
| D6 fresh/restart and teardown source audit | PASS |
| Contract Shape semantic audit | PASS |
| Mutation testing | NOT RUN — prohibited during an individual roadmap step |

### Quality gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — selected acceptance slice | PASS | The roadmap explicitly activates S-GTI-01, S-GTI-03, S-GTI-04, and S-GTI-07. |
| G2 — valid RED | PASS | Iteration-4 RED is ordered and `EXECUTED/PASS`. |
| G3 — assertion failure | PASS | The remediation records a genuine RED-to-GREEN assertion cycle, not a compile-only or scaffold substitute. |
| G4 — no domain mocks | PASS | The actual behavior still uses real CLI/network/identity composition; the baseline reads production durable facts through the real local-store adapter. |
| G5 — business language | PASS | The scenario keeps the DISTILL outcome name and exact anchor. |
| G6 — all executed tests green | PASS | Format, Lima compile/lint, verifier regression, and metal S-GTI-07 are green. |
| G7 — green before commit | PASS | GREEN and COMMIT events precede the commit timestamp. |
| G8 — test budget | PASS | 10 ≤ 12. |
| G9 — no prohibited test weakening | PASS | The self-derived equality is replaced with stronger independent API and frozen-render equality; the guest and anti-transit assertions remain. |

### Test integrity, external validity, and RPP

- **External validity:** PASS. S-GTI-07 reaches the real `serve`, deploy, VM boot, production identity issuance, HTTP describe, live renderer, stop, and cleanup paths.
- **Testing theater:** None. The expected API and render values no longer inherit unrelated fields or bytes from the actual response.
- **Test modification violation:** None. The remediation strengthens the previously ineffective assertions and preserves the positive and negative outcome facts.
- **Escalation verification:** Not applicable.
- **RPP levels scanned:** L1–L3 over the test-oracle expansion and the unchanged sensitive production seams.
- **Cascade stopped at:** L3 with no defect found.
- **RPP findings:** None.

### Iteration-4 defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |
| **Total** | **0** |

### Iteration-4 final verdict

**APPROVED**

The final D3-S-GTI-07 causal-baseline defect is resolved. The API contract admits only `workload_addr`, the rendered contract admits only the exact canonical Addresses section, all independently executed verification is green, and no regression or new finding remains. Step 02-01 is approved and may advance to step 02-02 under the repository's strict sequence.
