<!-- markdownlint-disable MD013 MD024 -->
# Test Scenarios — built-in-ca-operator-composition (folds GH #40 + GH #215)

**Wave**: DISTILL · **Density**: lean (Tier-1 `[REF]` only) · **Paradigm**: OOP Rust
**Authority**: `feature-delta.md` (DESIGN) + `design/wave-decisions.md` + `design/review-design.md` (conditional approval) + ADR-0063 amendment + ADR-0067 rev 6 + brief.md § "Built-in CA operator composition"

> **SPECIFICATION ONLY — not executed.** This workspace does NOT use `.feature`
> files (`.claude/rules/testing.md` § Testing). The GIVEN/WHEN/THEN blocks below
> are the human-readable specification companion for the Rust scaffold tests
> under `crates/*/tests/{acceptance,integration}`. DELIVER replaces the
> `#[should_panic(expected = "RED scaffold")]` / `todo!("RED scaffold: …")`
> bodies with real assertions slice by slice. Tiers per `.claude/rules/testing.md`:
> Tier 1 DST (pure reconciler, sim adapters) · Tier 3 real-kernel / Lima
> integration (real `RcgenCa`/KEK/redb + `overdrive serve` subprocess + `openssl`).

## Reading Checklist

| Artifact | Status | Notes |
|---|---|---|
| `docs/feature/built-in-ca-operator-composition/feature-delta.md` | + read | DESIGN sections, the 3 DELIVER slices, the E03 evidence path + O05≠E03 split. Authoritative. |
| `docs/feature/built-in-ca-operator-composition/design/wave-decisions.md` | + read | D-OC-1..8 settled; E03 3-sub-claim runner obligation. |
| `docs/feature/built-in-ca-operator-composition/design/review-design.md` | + read | CONDITIONALLY APPROVED; honored: E03 sub-claim-3 (pathLen=0) mandatory + #40-boundary clarity. |
| `docs/feature/built-in-ca-operator-composition/design/c4-diagrams.md` | + read | Component decomposition + boot/rotation flow; corroborates the driving-port table and the Slice ①/②/③ seam boundaries used below. |
| `docs/product/architecture/brief.md` § "Built-in CA operator composition" | + read | Driving ports (`overdrive serve`, `overdrive alloc status`), reuse posture, reframe. No `## For Acceptance Designer` subsection — driving ports sourced from feature-delta § Driving ports + brief CA subsection. |
| `docs/product/architecture/adr-0063-…root-key-protection.md` | + read (referenced) | `Ca` port, root-key envelope (D2/D4), Earned-Trust (D8), `issued_certificates` (D6). Contracts only; not relitigated. |
| `docs/product/architecture/adr-0067-…svid-lifecycle.md` (rev 6) | + read (referenced) | `SvidLifecycle` D1/D2/D8, rotate-as-action reframe, restart re-mint D10. |
| `verification/expectations/D01-…/README.md` + `runner.sh` | + read | Anchored S-02-02; D01 = on-disk byte-scan, no plaintext. |
| `verification/expectations/O04-…/README.md` + `runner.sh` | + read | Anchored S-02-06/07; 4 sub-claims, cause-distinct stderr, no re-mint. |
| `verification/expectations/E03-…/README.md` + `runner.sh` | + read | Anchored S-04-07 + S-03-05; runner is the **2-check shape** Slice ③ MUST extend to 3 (pathLen=0 negative anchor). |
| `verification/expectations/O05-…/README.md` + `runner.sh` | + read | Anchored S-05-03/04; operator-legible audit row, no cert bytes. |
| `verification/README.md` | + read (referenced) | Black-box discipline: runner is bash + `openssl` + file-observation, no `overdrive-*` crate link. |
| `.claude/rules/testing.md` | + read | No `.feature` files; integration-tests gating; RED scaffold convention; 4 tiers; Lima discipline. |
| `.claude/rules/verification.md` | + read | EDD discipline: author at DISTILL, capture at DELIVER, different-fox audit, no self-stamped `satisfied`. |
| `docs/product/journeys/issue-workload-identity.yaml` | + read | J-SEC-001; error_paths step 1 (decrypt-fail refuse-to-start), step 4 (auditable). |
| `docs/feature/built-in-ca/distill/test-scenarios.md` | (precedent) | S-NN scenario-ID scheme + RED scaffold shape. |
| `docs/feature/workload-identity-manager/distill/test-scenarios.md` + `red-classification.md` | + read (precedent) | S-WIM-NN shape; the existing GREEN `svid_lifecycle_reconcile.rs` tests this feature mutates. |
| `crates/overdrive-core/src/reconcilers/svid_lifecycle.rs` | + read | The MODIFY target (Slice ①): gate consts, `near_expiry` helper, `NEAR_EXPIRY_THRESHOLD_SECS`. |
| `crates/overdrive-core/tests/acceptance/svid_lifecycle_reconcile.rs` | + read | EXISTING GREEN tests; the gated-seam test (`near_expiry_rotation_seam_is_emit_gated_until_cert_rotation_registered`) is DELETED/rewritten by Slice ① (single-cut). |
| `crates/overdrive-host/tests/integration/rcgen_ca_chain_verify.rs` | + read | EXISTING GREEN tests; the E03 export-hook MODIFY target (`rcgen_full_svid_chain_verifies_…` + `rcgen_intermediate_cannot_sign_a_further_ca_path_len_enforced`). |
| `crates/overdrive-control-plane/tests/integration/ca_boot_and_audit.rs` | + read | EXISTING GREEN boot/audit tests (S-02-06/07, S-05-03/04); Slice ②/③ EDD captures wrap these. |
| `crates/overdrive-control-plane/src/api.rs` | + read | `AllocStatusResponse` EXTEND target (Slice ③). |
| `docs/feature/built-in-ca-operator-composition/discuss/` | - not found | DISCUSS consolidated into feature-delta; traceability derived from it. |
| `docs/feature/built-in-ca-operator-composition/devops/` | - not found | Warning only; single-node default + existing integration-test/Lima policy applies. |
| `docs/product/kpi-contracts.yaml` | + read (referenced) | Docs-platform-only; no CA-composition KPI entries. K1/K3 are CA outcome KPIs tracked in built-in-ca feature-delta, not the contracts file. Soft-gate: warn, proceed. |

## Reconciliation (Wave-Decision HARD GATE)

**Result**: PASSED — 0 contradictions.

This feature has no separate DISCUSS / DEVOPS `wave-decisions.md` (consolidated
into `feature-delta.md`). The single DESIGN `wave-decisions.md` (D-OC-1..8) is
internally consistent and consistent with the SSOT brief, ADR-0063 amendment,
ADR-0067 rev 6, and the `review-design.md` conditional approval. The one
upstream tension the review flagged (older ADR-0063 / brief prose overloading
`#40` across internal-SVID and external-ACME rotation) is a **prose-cleanup**
item, NOT a behavioural contradiction — the active handoff path (feature-delta,
wave-decisions, brief § CA composition, ADR-0067 rev 6) consistently frames
internal SVID near-expiry reissue as `Action::IssueSvid`. No scenario depends on
the ambiguous historical prose. Proceeded.

## Settled design (pinned, NOT relitigated)

1. **#40 near-expiry rotation = reconciler action.** `SvidLifecycle::reconcile`,
   `running ∧ held(near-expiry) → Action::IssueSvid` (`"rotate-svid"` correlation)
   UNCONDITIONALLY; threshold = ½ × `WORKLOAD_SVID_TTL` (1800s). `ROTATION_ENABLED`
   / `CERT_ROTATION_WORKFLOW` / `StartWorkflow` / `WorkflowName` imports DELETED.
   The executor mints + `IdentityMgr::hold()`-replaces. NOT a workflow.
2. **#215 boot-side.** `run_server` wires `ca_boot::boot_ca` + `bootstrap_node_intermediate`
   (persistent KEK-backed envelope-sealed root; adopt-on-restart) + Earned-Trust
   refuse-to-start; `ControlPlaneError::CaBoot(#[from] CaBootError)`.
3. **restart = re-mint.** #35's `running ∧ ¬held ∧ ever_issued → IssueSvid` branch
   is correct as-is (NOT the rotation path).

## Driving ports (port-to-port — named per scenario)

| Port | Mechanism | Scenarios |
|---|---|---|
| `SvidLifecycle::reconcile` | Direct pure call in Rust acceptance tests (the pure fn IS the domain driving port) | S-OC-01, S-OC-02, S-OC-03, S-OC-04, S-OC-05 |
| Operator CLI — `overdrive serve` | Real subprocess in Lima (integration-tests gated); boot composition root | S-OC-06, S-OC-07, S-OC-08, S-OC-09 (+ EDD D01/O04) |
| `IssueSvid` action-shim executor (mint + `hold`-replace + audit) | Direct action-shim dispatch with real CA + real ObservationStore (integration) | S-OC-10 (rotate-correlation dispatch) |
| Operator CLI — `overdrive alloc status --job <id>` | Real subprocess in Lima (integration-tests gated); read/render path | S-OC-11, S-OC-12 (+ EDD O05) |
| The gated `rcgen_ca_chain_verify` integration test + `openssl verify` | Real `RcgenCa` mint → PEM export → `openssl` subprocess (integration) | S-OC-13, S-OC-14, S-OC-15 (+ EDD E03) |

No new driving ports. `overdrive serve` and `overdrive alloc status` are existing
CLI verbs gaining additional observable surface; the reconciler and executor are
existing in-process ports. Per CLAUDE.md "Implement to the design — never invent
API surface": the rotate path reuses `Action::IssueSvid` UNCHANGED.

## Scenario Index

| ID | Title | Scaffold | Tags | Slice | Tier | Trace |
|---|---|---|---|---|---|---|
| S-OC-01 | Near-expiry held SVID emits exactly one rotate `IssueSvid` | `overdrive-core/tests/acceptance/svid_lifecycle_rotation.rs::near_expiry_held_alloc_emits_one_rotate_issue_svid` | `@dst @property @driving_port @slice-1` | ① | L1 | D-OC-1/2, ADR-0067 rev 6 D8 |
| S-OC-02 | Not-near-expiry held SVID emits no `IssueSvid` (no-op) | `…svid_lifecycle_rotation.rs::held_alloc_outside_near_expiry_window_emits_no_issue_svid` | `@dst @property @error @driving_port @slice-1` | ① | L1 | D-OC-1, ADR-0067 rev 6 D8 |
| S-OC-03 | Near-expiry `<=` boundary: `not_after == now + 1800` rotates; `+1801` does not | `…svid_lifecycle_rotation.rs::near_expiry_boundary_is_inclusive_at_half_ttl` | `@dst @error @driving_port @slice-1` | ① | L1 | D-OC-3/8 (live mutation target) |
| S-OC-04 | Rotation threshold TRACKS ½ × `WORKLOAD_SVID_TTL` via the emitted action | `…svid_lifecycle_rotation.rs::rotation_threshold_tracks_half_of_workload_svid_ttl_via_emitted_action` | `@dst @driving_port @slice-1` | ① | L1 | D-OC-3 |
| S-OC-05 | Rotate is distinct from restart-recovery re-issue (not the gated path) | `…svid_lifecycle_rotation.rs::rotate_is_distinct_from_restart_recovery_reissue` | `@dst @property @error @driving_port @slice-1` | ① | L1 | D-OC-1/6, ADR-0067 rev 6 D10 |
| S-OC-06 | `overdrive serve` boots a persistent root on first boot (generate + seal + persist) | `overdrive-control-plane/tests/integration/built_in_ca_operator_composition/serve_persistent_ca.rs::serve_first_boot_generates_seals_and_persists_root` | `@integration @real-io @adapter-integration @driving_port @slice-2 @edd:D01` | ② | L3 | D-OC-4, ADR-0063 D2/D4 |
| S-OC-07 | `overdrive serve` adopts the SAME root on restart (same serial, no re-mint) | `…serve_persistent_ca.rs::serve_restart_adopts_same_root_no_remint` | `@integration @real-io @adapter-integration @driving_port @slice-2 @edd:D01` | ② | L3 | D-OC-4/6, ADR-0063 D2 |
| S-OC-08a | `overdrive serve` refuses to start on the WRONG KEK | `…serve_persistent_ca.rs::serve_refuses_on_wrong_kek` | `@integration @real-io @error @driving_port @slice-2 @edd:O04` | ② | L3 | D-OC-5, ADR-0063 D8, journey err-path 1 |
| S-OC-08b | `overdrive serve` refuses to start on a TAMPERED envelope | `…serve_persistent_ca.rs::serve_refuses_on_tampered_envelope` | `@integration @real-io @error @driving_port @slice-2 @edd:O04` | ② | L3 | D-OC-5, ADR-0063 D8, journey err-path 1 |
| S-OC-08c | `overdrive serve` refuses to start when the KEK is ABSENT | `…serve_persistent_ca.rs::serve_refuses_on_absent_kek` | `@integration @real-io @error @driving_port @slice-2 @edd:O04` | ② | L3 | D-OC-5, ADR-0063 D8, journey err-path 1 |
| S-OC-08d | The three refusal causes render pairwise-distinct stderr | `…serve_persistent_ca.rs::serve_refusal_causes_are_pairwise_distinct` | `@integration @real-io @error @driving_port @slice-2 @edd:O04` | ② | L3 | D-OC-5, ADR-0063 D8 |
| S-OC-09 | Refuse-to-start leaves the persisted root unchanged (no silent re-mint) | `…serve_persistent_ca.rs::refuse_to_start_does_not_remint_the_root` | `@integration @real-io @error @driving_port @slice-2 @edd:O04` | ② | L3 | D-OC-5, ADR-0063 D8 |
| S-OC-10 | Rotate-correlation `IssueSvid` dispatches through the existing executor (mint + hold-replace + audit) | `overdrive-control-plane/tests/integration/built_in_ca_operator_composition/rotate_issue_svid_dispatch.rs::rotate_correlation_issue_svid_mints_replaces_hold_and_audits` | `@integration @real-io @adapter-integration @driving_port @slice-1` | ① | L3 | D-OC-1, ADR-0067 D3 |
| S-OC-11 | `overdrive alloc status` surfaces the current `issued_certificates` summary | `…alloc_status_issued_certificates.rs::alloc_status_surfaces_current_issued_certificate_summary` | `@integration @real-io @adapter-integration @driving_port @slice-3 @edd:O05` | ③ | L3 | D-OC-7, ADR-0063 D6, journey step 4 |
| S-OC-12 | `issued_certificates` summary carries NO cert bytes / NO key; latest-by-`issued_at` | `…alloc_status_issued_certificates.rs::issued_certificate_summary_omits_cert_bytes_and_key_latest_by_issued_at` | `@integration @real-io @error @driving_port @slice-3 @edd:O05` | ③ | L3 | D-OC-7, ADR-0067 #215-boundary |
| S-OC-13 | Exported chain verifies under `openssl verify` (E03 sub-claim 1) | `overdrive-host/tests/integration/rcgen_ca_chain_verify.rs::rcgen_full_svid_chain_verifies_root_intermediate_svid` (MODIFY: `OD_E03_CA_DIR` export) | `@integration @real-io @adapter-integration @driving_port @slice-3 @edd:E03` | ③ | L3 | S-04-07, ADR-0063 D1, K1 |
| S-OC-14 | Exported leaf profile: one spiffe URI SAN / CA:FALSE / critical digitalSignature (E03 sub-claim 2) | `…rcgen_ca_chain_verify.rs::rcgen_svid_leaf_carries_exactly_one_uri_san_and_leaf_profile` (EXISTING) + runner.sh | `@integration @real-io @adapter-integration @driving_port @slice-3 @edd:E03` | ③ | L3 | S-04, K2 |
| S-OC-15 | pathLen=0 negative anchor: further-CA chain FAILS `openssl verify` (E03 sub-claim 3) | `…rcgen_ca_chain_verify.rs::rcgen_intermediate_cannot_sign_a_further_ca_path_len_enforced` (MODIFY: `OD_E03_CA_DIR` export) + runner.sh | `@integration @real-io @error @driving_port @slice-3 @edd:E03` | ③ | L3 | S-03-05 (negative anchor) |

**Error/edge ratio**: 10 of 18 scenarios are `@error` (S-OC-02, S-OC-03, S-OC-05,
S-OC-08a, S-OC-08b, S-OC-08c, S-OC-08d, S-OC-09, S-OC-12, S-OC-15) = **56%**
(≥ 40% mandate met). Failure modes covered: not-near-expiry no-op, `<=` boundary,
rotate-vs-restart distinction, wrong-KEK / tampered-envelope / absent-KEK
refuse-to-start (3 distinct, now one scenario each) + their pairwise-distinct
stderr contract, no silent re-mint, no cert-bytes leak, pathLen=0 negative anchor.

> **Scenario count**: 18 (was 15). The S-OC-08 split (HIGH 2) replaced one
> multi-`When` scenario with S-OC-08a/b/c (one cause each) + S-OC-08d (the
> pairwise-distinctness contract). S-OC-04 (HIGH 1) was reframed observable but
> NOT merged into S-OC-03 (the two pin different contracts — literal boundary vs
> TTL-tracking — see the merge decision note under S-OC-04).

## Scenarios

### Slice ① — Rotation (core action flip)

> Tier-1 DST, pure reconciler. The existing GREEN `svid_lifecycle_reconcile.rs`
> test `near_expiry_rotation_seam_is_emit_gated_until_cert_rotation_registered`
> (asserts the OLD gated-NO-emit behaviour) is DELETED/rewritten by Slice ①
> (single-cut, per CLAUDE.md "single-cut greenfield migrations"). The NEW
> behaviour lives in a new file `svid_lifecycle_rotation.rs` so the scaffold and
> the production flip land together; the existing pure tests for the other
> branches (issue / drop / backoff / restart-recovery) stay GREEN, unchanged.

#### S-OC-01 — Near-expiry held SVID emits exactly one rotate `IssueSvid`

```gherkin
Given a SvidLifecycle desired set containing a Running allocation
And the allocation is currently held with a real not_after within ½ × WORKLOAD_SVID_TTL of now
When SvidLifecycle::reconcile is driven through the pure reconciler port
Then it emits exactly one Action::IssueSvid for the allocation
And the action carries the held spiffe_id, the running node_id, and a "rotate-svid" correlation
And it emits no StartWorkflow (the cert_rotation workflow no longer exists)
```

Universe: the emitted action list (`IssueSvid` count + fields) + the next View.
No private runtime fields. `@property`: holds for arbitrary `now` and arbitrary
slack strictly inside the window.

#### S-OC-02 — Not-near-expiry held SVID emits no `IssueSvid` (no-op)

```gherkin
Given a Running allocation held with a far-future not_after (outside the near-expiry window)
When SvidLifecycle::reconcile is driven
Then it emits no Action::IssueSvid for the held-running allocation
And the converged action vector for a single held-running alloc is exactly [Noop]
```

Universe: the emitted action list. `@property`: holds for arbitrary `now` and
arbitrary `not_after` strictly beyond `now + threshold`.

#### S-OC-03 — Near-expiry `<=` boundary is inclusive at half-TTL

```gherkin
Given a Running held allocation whose not_after equals exactly now + 1800s
When SvidLifecycle::reconcile is driven
Then it emits one rotate Action::IssueSvid (the <= boundary is inclusive)
Given instead a Running held allocation whose not_after equals now + 1801s
When SvidLifecycle::reconcile is driven
Then it emits no Action::IssueSvid (just outside the window)
```

Universe: the emitted `IssueSvid` count across the two boundary fixtures. This
is the **live mutation target** (D-OC-8): the `<=`→`<` and `<=`→`==` mutants must
be killed by this scenario. Two pinned examples (boundary cases), not PBT.

#### S-OC-04 — Rotation threshold TRACKS ½ × `WORKLOAD_SVID_TTL` via the emitted action

```gherkin
Given the workload SVID TTL is WORKLOAD_SVID_TTL (3600s, sourced from validity.rs)
And a Running held allocation whose not_after equals exactly now + WORKLOAD_SVID_TTL/2
When SvidLifecycle::reconcile is driven through the pure reconciler port
Then it emits exactly one rotate Action::IssueSvid for the allocation
Given instead a Running held allocation whose not_after equals now + WORKLOAD_SVID_TTL/2 + 1s
When SvidLifecycle::reconcile is driven
Then it emits no rotate Action::IssueSvid (just outside the TTL-derived window)
```

Universe: the emitted `IssueSvid` count across the two TTL-derived boundary
fixtures — **port-observable only** (the action list returned by
`SvidLifecycle::reconcile`); NO inspection of a private threshold constant.
Where S-OC-03 pins the **literal** `<=` boundary (`now + 1800s` / `+1801s`) as
the mutation kill-test, S-OC-04 proves the boundary is **derived from**
`WORKLOAD_SVID_TTL` — the fixtures are computed as `now + WORKLOAD_SVID_TTL/2`
(not a bare `1800`), so a regression that hardcodes the threshold to a literal
(silently ignoring a TTL policy change) flips the emit decision and reds this
scenario. DELIVER MAY compute the fixtures FROM the TTL const but MUST NOT assert
or expose a private near-expiry threshold value. Two pinned examples (the
TTL-derived boundary and just past it), not PBT.

**Merge decision (review HIGH 1)**: S-OC-04 is NOT merged into S-OC-03. The two
assert different contracts — S-OC-03 pins the literal `1800s` boundary (kills
`<=`→`<` / `<=`→`==`), S-OC-04 proves the boundary value *tracks the TTL const*
(kills a "hardcode 1800, ignore TTL policy" regression). Both now assert only the
emitted action list (no threshold-const inspection), satisfying the
observable-behavior fix without becoming duplicates.

#### S-OC-05 — Rotate is distinct from restart-recovery re-issue (not the gated path)

```gherkin
Given a Running allocation that is HELD with a near-expiry not_after (rotate case)
And a separate Running allocation that is NOT held but ever_issued (restart-recovery case)
When SvidLifecycle::reconcile is driven over both
Then the held near-expiry alloc emits a "rotate-svid"-correlation IssueSvid
And the unheld ever_issued alloc emits an "issue-svid"-correlation IssueSvid
And neither emits any StartWorkflow, and the two correlations are distinct
```

Universe: the emitted action list partitioned by correlation purpose. Proves the
rotate branch (`running ∧ held(near-expiry)`) and the restart-recovery branch
(`running ∧ ¬held ∧ ever_issued`) are independent and both live — the rotate
path is NOT routed through the deleted gated seam. `@property` over arbitrary
near-expiry slack + arbitrary ever_issued membership.

#### S-OC-10 — Rotate-correlation `IssueSvid` dispatches through the existing executor

```gherkin
Given an Action::IssueSvid carrying a "rotate-svid" correlation for a held running allocation
And a real CA adapter whose issue_and_audit mints a fresh leaf and writes an audit row
When the action shim dispatches the action
Then a fresh issued_certificates row (new serial, new window) is observable
And IdentityMgr holds the freshly-minted SvidMaterial for the allocation (hold-replace)
And the held cert serial matches the new audit-row serial
```

Universe: action-shim result, `IdentityMgr` held snapshot (post-replace), the
ObservationStore audit row. Layer 3 (real CA + real ObservationStore): example-
based, one example (Mandate 11). Proves the rotate `IssueSvid` reuses the SAME
executor as first-issue/restart — no new executor surface.

### Slice ② — Boot-side #215 (persistent CA wired into `serve`)

> Tier-3 integration, Lima. `overdrive serve` subprocess is the driving adapter.
> EDD captures D01 (data-at-rest) + O04 (operator CLI). The in-tree boot tests
> (`ca_boot_and_audit.rs` S-02-06/07) already prove the refuse-to-start at the
> `boot_ca` seam; these scenarios prove the SAME behaviour through the wired
> `run_server` composition root (the prior ephemeral path probed nothing).

#### S-OC-06 — `overdrive serve` boots a persistent root on first boot

```gherkin
Given a clean IntentStore and a resolvable KEK in the keyring
When overdrive serve starts for the first time
Then it generates a self-signed P-256 root and a node intermediate
And it persists the root as a KEK-sealed AES-256-GCM envelope in the IntentStore file
And the control plane reaches a serving state
```

Universe: the `overdrive serve` startup outcome (reaches serving), the on-disk
IntentStore file contents (sealed envelope present). EDD D01 sub-claim 1+2: the
on-disk file contains the AEAD envelope fields and NO plaintext root-key DER
(byte-scan). Example-based (one first-boot example).

#### S-OC-07 — `overdrive serve` adopts the SAME root on restart (no re-mint)

```gherkin
Given a control plane that booted once and persisted a KEK-sealed root
When overdrive serve restarts with the same KEK available
Then it decrypts and adopts the SAME root (identical root serial across the restart)
And it does not generate a new root
And the on-disk IntentStore file still contains no plaintext key bytes
```

Universe: the root serial observed before vs after restart (must be identical),
the on-disk file byte-scan (no plaintext, EDD D01 sub-claim 3 — guardrail holds
across the lifecycle). Example-based.

#### S-OC-08a — `overdrive serve` refuses to start on the WRONG KEK

```gherkin
Given a persisted root whose envelope cannot be opened with the supplied KEK
When overdrive serve starts with the WRONG KEK
Then it refuses to start and the stderr names a wrong-KEK cause (CaError::WrongKek)
And the control plane does NOT begin serving
And health.startup.refused is emitted
```

Universe: the `overdrive serve` exit outcome (refuse) + the stderr cause string
(names the wrong-KEK cause + the IntentStore path, not a bare panic/backtrace).
EDD O04 sub-claim 1. NAMED example-based sad path (Mandate 11): `Sad_wrong_kek`.

#### S-OC-08b — `overdrive serve` refuses to start on a TAMPERED envelope

```gherkin
Given a persisted root whose envelope has been TAMPERED (AEAD tag mismatch)
When overdrive serve starts against the tampered envelope
Then it refuses to start and the stderr names a corrupt/tampered-envelope cause (CaError::TamperedEnvelope)
And the control plane does NOT begin serving
And health.startup.refused is emitted
```

Universe: the `overdrive serve` exit outcome (refuse) + the stderr cause string
(names the tampered-envelope cause + the IntentStore path). EDD O04 sub-claim 2.
NAMED example-based sad path (Mandate 11): `Sad_tampered_envelope`.

#### S-OC-08c — `overdrive serve` refuses to start when the KEK is ABSENT

```gherkin
Given NO KEK is resolvable from the keyring
When overdrive serve starts
Then it refuses to start BEFORE any issuance with an absent-KEK cause (CaBootError::KekUnavailable)
And no throwaway KEK is generated
And the control plane does NOT begin serving
And health.startup.refused is emitted
```

Universe: the `overdrive serve` exit outcome (refuse) + the stderr cause string
(names the absent-KEK cause + the IntentStore path) + the absence of any
generated throwaway KEK. EDD O04 sub-claim 3. NAMED example-based sad path
(Mandate 11): `Sad_absent_kek`.

#### S-OC-08d — The three refusal causes render pairwise-distinct stderr

```gherkin
Given the three refused-boot stderr messages from S-OC-08a/b/c
When the rendered cause strings are compared
Then the wrong-KEK, tampered-envelope, and absent-KEK messages are pairwise distinct
```

Universe: the three captured stderr cause strings, compared for pairwise
distinctness. EDD O04 sub-claims 1–3 (the cross-cause contract). This pins the
operator-triage value the original single-When scenario carried — that an
operator can tell the three failure modes apart from stderr alone — without
muddying the per-cause fail-for-right-reason triage. Example-based; reuses the
three captured outputs from S-OC-08a/b/c.

#### S-OC-09 — Refuse-to-start leaves the persisted root unchanged (no silent re-mint)

```gherkin
Given a persisted root and a boot that refused (wrong KEK or tampered envelope)
When the correct KEK is re-supplied and overdrive serve starts again
Then it adopts the SAME original root (identical serial) — the refused boot did not re-mint
And no new root envelope was written during the refused boot
```

Universe: the root serial after a refused-then-recovered boot (must equal the
original) + the IntentStore envelope (unchanged across the refused boot). EDD O04
sub-claim 4. Example-based. This is the load-bearing guardrail — a silent re-mint
would orphan every issued identity.

### Slice ③ — Consumer-side #215 (issued-cert summary surfaced) + E03 chain proof

> **O05 ≠ E03.** S-OC-11/12 (O05) render operator-legible metadata
> (`serial / spiffe_id / issuer_serial / not_after`, NO cert bytes, NO key) — they
> do NOT and CANNOT prove the chain verifies. E03 (S-OC-13/14/15) is the SEPARATE
> exported-PEM `openssl verify` proof. The O05 render MUST NOT be treated as
> satisfying E03.

#### S-OC-11 — `overdrive alloc status` surfaces the current issued-certificate summary

```gherkin
Given the platform has issued an SVID for a deployed running workload
When the operator runs overdrive alloc status --job <id>
Then the output surfaces an issued-certificate summary for the running allocation
And the summary shows serial, spiffe_id, issuer_serial, and not_after
And the surfaced serial matches the minted certificate's serial
```

Universe: the `overdrive alloc status` rendered output (the issued-certificate
section + its fields) and the cross-checked minted serial. EDD O05 sub-claim 1+2.
Example-based, one deployed-workload example.

#### S-OC-12 — Summary omits cert bytes / key; renders latest-by-`issued_at`

```gherkin
Given a running allocation with MULTIPLE issued_certificates audit rows over time (first issue + a re-mint)
When the operator runs overdrive alloc status --job <id>
Then the issued-certificate summary renders exactly the latest-by-issued_at row per running alloc (NOT history)
And the summary contains NO certificate PEM/DER bytes and NO private key
And a post-restart serial change reads as the current cert, not an anomaly
```

Universe: the rendered issued-certificate section (exactly one row per running
alloc, the latest), and the absence of cert-bytes/key tokens in the output.
`@error`: guards the no-leak invariant (ADR-0067 #215-boundary). Example-based.

#### S-OC-13 — Exported chain verifies under `openssl verify` (E03 sub-claim 1)

```gherkin
Given the gated rcgen chain-verify test mints a coherent root → intermediate → SVID chain
And OD_E03_CA_DIR is set so the test exports root.pem / intermediate.pem / svid.pem
When openssl verify -CAfile root.pem -untrusted intermediate.pem svid.pem runs over the exported PEMs
Then it exits 0 with "svid.pem: OK"
```

Universe: the `openssl verify` exit code + stdout over the exported `$CA_DIR/*.pem`.
EDD E03 sub-claim 1. The export hook is a TEST FIXTURE change to the existing
`rcgen_full_svid_chain_verifies_root_intermediate_svid` (env-gated PEM write before
the TempDir drops) — NO production API surface, NO operator verb. Example-based.

#### S-OC-14 — Exported leaf profile (E03 sub-claim 2)

```gherkin
Given the exported svid.pem from the gated chain-verify test
When openssl x509 -in svid.pem -noout -text inspects it
Then it carries exactly one URI:spiffe://overdrive.local/job/<name>/alloc/<id> SAN
And basicConstraints is CA:FALSE
And keyUsage is digitalSignature, marked critical
```

Universe: the parsed leaf-profile assertions over the exported `svid.pem`. EDD
E03 sub-claim 2. The in-tree assertion already exists
(`rcgen_svid_leaf_carries_exactly_one_uri_san_and_leaf_profile`); the runner
re-checks the profile over the exported PEM. Example-based.

#### S-OC-15 — pathLen=0 negative anchor FAILS `openssl verify` (E03 sub-claim 3)

```gherkin
Given OD_E03_CA_DIR is set so the further-CA test exports its further-CA chain PEMs
And the pathLen=0 intermediate signs a FURTHER CA cert
When openssl verify -CAfile root.pem -untrusted intermediate.pem furtherca.pem runs
Then it FAILS (non-zero exit) — pathLen=0 is ENFORCED, not merely set
```

Universe: the `openssl verify` exit code over the further-CA chain (must be
non-zero). EDD E03 **sub-claim 3 — MANDATORY before any E03 `satisfied`** (review
finding; the different-fox audit MUST reject E03 evidence missing it). The export
hook is a TEST FIXTURE change to the existing
`rcgen_intermediate_cannot_sign_a_further_ca_path_len_enforced`. Example-based,
sad path (Mandate 11).

## Scenario ↔ EDD ↔ Slice ↔ Tier mapping

| Scenario | EDD | Slice | Tier | Driving port |
|---|---|---|---|---|
| S-OC-01 | — | ① | L1 (DST) | `SvidLifecycle::reconcile` |
| S-OC-02 | — | ① | L1 (DST) | `SvidLifecycle::reconcile` |
| S-OC-03 | — | ① | L1 (DST) | `SvidLifecycle::reconcile` |
| S-OC-04 | — | ① | L1 (DST) | `SvidLifecycle::reconcile` |
| S-OC-05 | — | ① | L1 (DST) | `SvidLifecycle::reconcile` |
| S-OC-10 | — | ① | L3 (integration) | `IssueSvid` action-shim executor |
| S-OC-06 | **D01** | ② | L3 (Lima) | `overdrive serve` |
| S-OC-07 | **D01** | ② | L3 (Lima) | `overdrive serve` |
| S-OC-08a | **O04** | ② | L3 (Lima) | `overdrive serve` |
| S-OC-08b | **O04** | ② | L3 (Lima) | `overdrive serve` |
| S-OC-08c | **O04** | ② | L3 (Lima) | `overdrive serve` |
| S-OC-08d | **O04** | ② | L3 (Lima) | `overdrive serve` |
| S-OC-09 | **O04** | ② | L3 (Lima) | `overdrive serve` |
| S-OC-11 | **O05** | ③ | L3 (Lima) | `overdrive alloc status` |
| S-OC-12 | **O05** | ③ | L3 (Lima) | `overdrive alloc status` |
| S-OC-13 | **E03** | ③ | L3 (Lima) | `rcgen_ca_chain_verify` + `openssl` |
| S-OC-14 | **E03** | ③ | L3 (Lima) | `rcgen_ca_chain_verify` + `openssl` |
| S-OC-15 | **E03** | ③ | L3 (Lima) | `rcgen_ca_chain_verify` + `openssl` |

**EDD graduation check**: D01, O04, O05, E03 each have ≥1 graduating scenario;
all four expectations already exist + are anchored (`verification/expectations/`).
Slice ① rotation scenarios (S-OC-01..05, S-OC-10) stay in the Rust test tiers —
no EDD graduation (pure in-process reconciler logic + action-shim dispatch, no
new operator surface). E03 is satisfiable ONLY when the runner enforces all three
sub-claims (S-OC-13/14/15); the different-fox audit MUST refute E03 evidence
missing S-OC-15.

## Adapter coverage (every driven adapter → ≥1 real-I/O Tier-3 scenario)

| Driven adapter / port | Real-I/O (Tier-3) scenario | Note |
|---|---|---|
| `Ca` / `RcgenCa` (mint root/intermediate/SVID, adopt) | S-OC-06, S-OC-07, S-OC-10, S-OC-13, S-OC-14, S-OC-15 | Real `ring`/rcgen crypto |
| `Kek` / `SystemdCredsKeyring` (resolve, seal/open) | S-OC-06, S-OC-07, S-OC-08a, S-OC-08b, S-OC-08c, S-OC-09 | Real keyring resolve; wrong/tampered/absent KEK exercised |
| `IntentStore` / `LocalIntentStore` (redb) | S-OC-06, S-OC-07, S-OC-09 | Real redb root-key envelope persist/load; byte-scan for D01 |
| `ObservationStore` / `LocalObservationStore` | S-OC-10, S-OC-11, S-OC-12 | Real `issued_certificate_rows()` write + read/render |

Every driven adapter touched by this feature has ≥1 real-I/O Tier-3 scenario.
Slice ①'s pure reconciler scenarios (S-OC-01..05) use NO adapter (pure fn over
typed `State`) — the adapter behaviours the rotate `IssueSvid` triggers are
proven at S-OC-10 (real CA + real ObservationStore).

## Driving-adapter coverage (real protocol, not internal calls)

| Driving adapter | Real-protocol scenario | Mechanism |
|---|---|---|
| `overdrive serve` (CLI) | S-OC-06, S-OC-07, S-OC-08a, S-OC-08b, S-OC-08c, S-OC-08d, S-OC-09 | Real subprocess in Lima — boot, restart, refuse-to-start (3 causes, one scenario each) + pairwise-distinct stderr |
| `overdrive alloc status --job <id>` (CLI) | S-OC-11, S-OC-12 | Real subprocess in Lima — read/render the issued-certificate summary |

Both CLI verbs are exercised via their real protocol (subprocess in Lima), not
just internal `run_server` / handler calls — the EDD captures (D01/O04/O05) run
the BUILT `overdrive` binary per `verification/README.md`.

## Test placement

| Crate | File | Rationale |
|---|---|---|
| `overdrive-core` | `tests/acceptance/svid_lifecycle_rotation.rs` (NEW) | Pure rotate-branch reconciler scenarios (Slice ①). New file so the production gate-flip and the new behaviour land together; existing `svid_lifecycle_reconcile.rs` branches stay GREEN. |
| `overdrive-control-plane` | `tests/integration/built_in_ca_operator_composition/serve_persistent_ca.rs` (NEW) | `overdrive serve` boot/restart/refuse-to-start, gated `integration-tests`, Lima (Slice ②). |
| `overdrive-control-plane` | `tests/integration/built_in_ca_operator_composition/rotate_issue_svid_dispatch.rs` (NEW) | Rotate-correlation `IssueSvid` executor dispatch with real CA + ObservationStore (Slice ①, L3). |
| `overdrive-control-plane` | `tests/integration/built_in_ca_operator_composition/alloc_status_issued_certificates.rs` (NEW) | `alloc status` issued-cert summary render, gated `integration-tests`, Lima (Slice ③). |
| `overdrive-host` | `tests/integration/rcgen_ca_chain_verify.rs` (MODIFY) | E03 `OD_E03_CA_DIR` export hook added to the two EXISTING GREEN tests (Slice ③). No new file — the export is a fixture change. |
| (runner) | `verification/expectations/E03-…/runner.sh` (MODIFY) | Extend the 2-check runner to 3 checks (add S-OC-15 pathLen=0 negative anchor). DELIVER owns the export-hook wiring — DISTILL does NOT edit `runner.sh`. |

New integration files are wired through `crates/overdrive-control-plane/tests/integration.rs`
inside the inline `mod integration { … }` block (per `.claude/rules/testing.md` —
each `tests/*.rs` is a crate root; the inline wrapper shifts the lookup base) with
a `mod built_in_ca_operator_composition { mod serve_persistent_ca; … }` sub-module.

## RED scaffold convention (project, NOT pytest)

- **Test-side** (new behaviours): `#[should_panic(expected = "RED scaffold")]` on
  the test fn with a body `panic!("Not yet implemented -- RED scaffold (<S-OC-NN> / <desc>)")`.
- **Production-side** (where a new arm/field is needed): `todo!("RED scaffold: …")`
  gated `#[expect(clippy::todo, reason = "RED scaffold; lands GREEN in slice NN")]`.
- **Integration scaffolds** (Lima-only behaviour): `#[ignore = "blocked on slice NN — overdrive serve CA boot wiring (Lima)"]` is the correct marker for the `overdrive serve` / `alloc status` subprocess tests whose runtime surface is `#[cfg(feature = "integration-tests")]` and only reachable in Lima — per `.claude/rules/testing.md` § "What about `#[ignore]`?": the blocker is "the production wiring does not exist yet AND the test can only run under Lima." DELIVER removes `#[ignore]` slice by slice. The pure-reconciler L1 scaffolds (S-OC-01..05) use `#[should_panic]` (they run in the default lane, no Lima).

## DELIVER notes

- Do NOT introduce `.feature` files or Python step definitions.
- Replace `#[should_panic(expected = "RED scaffold")]` / `todo!` bodies + remove
  `#[ignore]` slice by slice; confirm fail-for-right-reason before writing prod code.
- **Slice ① single-cut**: delete the gate consts (`ROTATION_ENABLED`,
  `CERT_ROTATION_WORKFLOW`), the `StartWorkflow`/`WorkflowName` imports, the
  `#[mutants::skip]` on `near_expiry`, the `.cargo/mutants.toml` `"near_expiry"`
  `exclude_re` entry, AND the existing gated-seam test
  `near_expiry_rotation_seam_is_emit_gated_until_cert_rotation_registered` — all
  in the same commit (CLAUDE.md "Delete unused code AND its tests").
- **Slice ① mutation kill-test**: the `near_expiry` `<=` boundary is now a live,
  un-skipped mutation target — S-OC-03 is the kill-test (`<=`→`<`, `<=`→`==`).
- **CLAUDE.md "Implement to the design — never invent API surface"**: the rotate
  path reuses `Action::IssueSvid` UNCHANGED (no new field/flag/variant); `node_id`
  comes from `running.node_id` already in scope. If a gap surfaces, STOP and
  surface it — do not improvise.
- Do NOT touch `mint_ephemeral_ca()`, whitepaper §18, ADR-0064/0065/0066.

## Verification catalogue (EDD)

Four expectations already exist + are anchored; this feature unblocks all four:

- `D01-ca-root-key-never-plaintext-at-rest` ← Slice ② (S-OC-06/07).
- `O04-ca-refuse-to-start-actionable-error` ← Slice ② (S-OC-08/09).
- `O05-ca-issued-certificates-audit-row` ← Slice ③ (S-OC-11/12).
- `E03-ca-full-chain-verifies` ← Slice ③ (S-OC-13/14/15) — **runner MUST enforce
  all 3 sub-claims**; the different-fox Haiku reviewer per expectation rejects E03
  evidence missing the pathLen=0 negative anchor (S-OC-15). The authoring agent
  never self-stamps `satisfied`. DELIVER owns the `runner.sh` 3-check extension +
  the `OD_E03_CA_DIR` export-hook wiring; DISTILL does NOT edit `runner.sh`.

EDD capture: `verification/harness/run-expectation.sh <ID>` in Lima, SHA-pinned,
against the built `overdrive` binary (D01/O04/O05) or the gated test's exported
PEMs (E03). The runner stays black-box (bash + `openssl` + file-observation; no
`overdrive-*` crate link).

## DISTILL review

Review captured in
`docs/feature/built-in-ca-operator-composition/distill/review-distill.md`.
Verdict: conditionally approved, pending two scenario-shape revisions (S-OC-04
observable-behavior tightening; S-OC-08 single-When GWT split/table).

**Update (revisions applied)**: both high-severity revisions are now applied.
HIGH 1 — S-OC-04 reframed to assert ONLY the emitted action list at TTL-derived
boundaries (no private-threshold inspection); scaffold renamed
`rotation_threshold_tracks_half_of_workload_svid_ttl_via_emitted_action`; kept
distinct from S-OC-03 (literal `<=` boundary) rather than merged (see the merge
decision under S-OC-04). HIGH 2 — S-OC-08 split into S-OC-08a/b/c (one refusal
cause each: wrong-KEK / tampered-envelope / absent-KEK) + S-OC-08d (pairwise-
distinct stderr contract); the single `serve_refuses_to_start_with_cause_distinct_errors`
scaffold replaced by `serve_refuses_on_wrong_kek` / `serve_refuses_on_tampered_envelope`
/ `serve_refuses_on_absent_kek` / `serve_refusal_causes_are_pairwise_distinct`.
LOW 3 — the `c4-diagrams.md` checklist row corrected from `- not found` to
`+ read`. Scenario count 15 → 18; error-ratio 47% → 56% (still ≥ 40%).
