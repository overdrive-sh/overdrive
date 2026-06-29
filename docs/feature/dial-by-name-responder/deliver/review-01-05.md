# Adversarial review — step 01-05 (FrontendAddrAllocator WRITER)

- **Artifact:** implementation of roadmap step `01-05` — *FrontendAddrAllocator WRITER: assign-on-declare + converge-on-boot rebuild*
- **Reviewer:** `nw-software-crafter-reviewer` (opus, adversarial posture) + orchestrator verification of #211, feature-delta gate refs, and execution-log integrity
- **Latest verdict (REV-2):** **NEEDS_REVISION** — 1 blocking (D3), 1 nitpick (N1). REV-1's D1/D2/D4/D5 all RESOLVED; all four S-DBN-ASSIGN-01..04 now MET.

---

## REV-2 — re-review of correctives (2026-06-26)

Corrective commits verified at HEAD: `8d2036ca` (re-gate rebuild, non-vacuous ASSIGN-04, VIP-leak guard) + `0d9c4eb4` (kill sub-key-filter mutant `||`→`&&`). Code read directly, not trusted from commit messages.

| REV-1 finding | REV-2 status | Evidence |
|---|---|---|
| **D1** un-gated rebuild vs pinned criterion | **RESOLVED** | Rebuild now inside `if state.mtls_worker.is_some()` (`lib.rs:2011-2094`), pinned boot order adopt→GC→sweep→rebuild→serve, before the convergence loop. Conforms to the pin (no ratification needed); rationale now self-consistent (reader is gated behind the same block per DDN-6). |
| **D2** ASSIGN-04 testing theater | **RESOLVED** | `writer_feeds_the_same_allocator_instance_the_name_index_reads` now drives `answer_for → frontend_for → shared allocator` over a seeded running-AND-healthy `ServiceBackendRow`; asserts answered F byte-identical to assigned F. Key-derivation parity (writer `MeshServiceName::new("api.svc.overdrive.local")` vs reader `job_of` of `/job/api/…`) verified. Litmus holds. |
| **D3** no mutation-gate evidence | **PARTIALLY-RESOLVED (BLOCKING)** | One named survivor killed (`boot_rebuild_ignores_service_payload_under_a_sub_key_path`, non-vacuous). But NO MUTATION phase logged in `execution-log.json` (all 3 cycles), no `mutants-summary.json`, no recorded ≥80% kill-rate over `handlers.rs` + `boot_rebuild.rs`. Other surfaces (4a assign-guard, resubmit path, line-138 `is_empty()` disjunct) unverified. |
| **D4** GREEN logged after COMMIT | **RESOLVED** | Both corrective cycles log GREEN before COMMIT (18:07:23<18:09:25; 18:22:17<18:24:50). Cycle-1 anomaly correctly remains append-only. |
| **D5** allocate-then-fail VIP leak | **RESOLVED** | Frontend assign reordered before VIP allocate (`handlers.rs` 4a now frontend, 4b VIP); exhaustion early-return precedes any fsync'd VIP. Symmetric case (4a ok, 4b fails) leaves only a benign in-memory `<job>→F` binding — strict severity reduction, not a swap. Conflict-rollback intact. |
| **D6** #211 deferral citation | **RESOLVED** (unchanged) | #211 scopes `release(<job>)` + records "MUST NOT wire into stop." |

**New (REV-2):**

- **N1 — `nitpick (non-blocking)`:** stale step-label back-reference at `handlers.rs:508` — the VIP-release comment still says "step 4a runs before `put_if_absent`," but the D5 reorder moved the VIP allocate to step 4b (4a is now the frontend assign). Behaviorally correct; comment is stale. Fix: "step 4a" → "step 4b".

**Per-criterion at HEAD:** S-DBN-ASSIGN-01 **MET**, -02 **MET**, -03 **MET**, -04 **MET** (was vacuous, now non-vacuous). No regressions from the correctives; no new public API; test changes are strengthening + additive; all test doubles are sim adapters.

**REV-2 remaining blocking issue:**

1. **D3** — Run the mandated diff-scoped mutation gate and log it:
   `cargo xtask lima run -- cargo xtask mutants --diff origin/main --features integration-tests --package overdrive-control-plane --file crates/overdrive-control-plane/src/handlers.rs --file crates/overdrive-control-plane/src/dns_responder/boot_rebuild.rs`
   — achieve ≥80% over the WRITER call sites (Service-arm assign guard, idempotent-resubmit path, rebuild enumeration incl. the line-138 `is_empty()` disjunct), add tests for any survivor, and log a MUTATION phase in `execution-log.json` with the recorded kill-rate.

Non-blocking: **N1** (stale comment).

---

## REV-1 — original review (2026-06-26)

> Historical. D1/D2/D5 superseded by the REV-2 correctives above; D3 carried forward (still blocking); D4 resolved; D6 resolved.

- **Commit reviewed:** `71ac0add`
- **Verdict:** **NEEDS_REVISION** (4 blocking, 2 non-blocking)

---

## Per-criterion verdicts (S-DBN-ASSIGN-01..04)

| Criterion | Verdict | Defending test | Litmus |
|---|---|---|---|
| **ASSIGN-01** (assign-on-declare, Service-only guard) | **MET** | `service_submit_assigns_frontend_addr_in_shared_allocator` + `job_submit_assigns_no_frontend_addr` | Gutting the Service-arm `assign` (`handlers.rs:372`) flips RED; Job-kind test pins the Service-only guard. |
| **ASSIGN-02** (idempotent resubmit + no eviction on conflict) | **MET** | `byte_identical_resubmit_does_not_change_binding` + `conflicting_resubmit_does_not_evict_existing_binding` | Asserts `snapshot().len()==1` and byte-stable binding across both paths; conflict returns `Err(Conflict)` without eviction. |
| **ASSIGN-03** (converge-on-boot rebuild, idempotent) | **MET** | `boot_rebuild_repopulates_empty_allocator_from_declared_services` | Seeds 3 declared Services, asserts all 3 bound in `10.98.0.0/16`, re-runs and asserts `snapshot()==after`. Skipping a `<job>` flips RED. |
| **ASSIGN-04** (single-owner: writer feeds the readers' instance, DDN-2) | **NOT-MET (vacuous)** | `writer_feeds_the_same_allocator_instance_the_name_index_reads` | See **D2**. Constructs `let _name_index` and drops it unused; asserts `writer_f == reader_f` over two clones of the same `Arc<Mutex<…>>` — a tautology. The criterion's "answers F … once J has a running-AND-healthy backend" is never exercised. |

---

## Blocking issues

### D1 — `issue (blocking)`: converge-on-boot rebuild deviates from a PINNED criterion (mtls_worker gating) without surfacing a DECISION

`lib.rs:2086-2091` places `rebuild_frontend_addrs_from_intent` **outside** the `if state.mtls_worker.is_some()` block (`lib.rs:2011-2062`). The 01-05 "Pinned surface ONLY" criterion states the rebuild runs "from `run_server` … **gated by the SAME `mtls_worker.is_some()` block**." The crafter flipped this pinned decision and documented the reversal *after the fact* (`boot_rebuild.rs:107-112`, `lib.rs:2077-2085`, the commit message) — there is no `wave-decisions.md` entry and no DECISION/BLOCKER surfaced for approval.

CLAUDE.md "Implement to the design — never invent API surface" is explicit: when the design pins a shape, **STOP and surface the gap** — "never reach for the nearest mechanism that compiles." This is the ADR-0065 precedent (a technically-defensible change that compiled + passed tests but diverged from the pinned contract → a rework cycle caught only in adversarial review).

**The crafter's stated rationale is self-defeating.** The justification is: "gating would leave the allocator empty on a non-mTLS boot and the `name_index` reader would withhold every declared name." But the feature-delta gates the **reader itself** on `mtls_worker.is_some()` (DDN-6, `feature-delta.md:741`, `:865`; composition root `:618`, `:755`, `:998`). On a non-mTLS boot there is therefore **no responder and no reader** — populating the allocator is wasted work, not a fix. The un-gated rebuild simultaneously (a) overrides the roadmap's explicit pin and (b) contradicts the feature-delta's reader gating, creating an asymmetry where 02-01's mTLS-gated responder will not serve the allocator the rebuild populated on a non-mTLS boot.

**Required:** surface this as a DECISION to the user/orchestrator. Either ratify un-gated and amend the roadmap criterion[6] + feature-delta accordingly, or re-gate to match. A pinned-criterion deviation may not ship silently, even if it were technically superior.

### D2 — `issue (blocking)`: S-DBN-ASSIGN-04 is testing theater (the criterion's observable is never exercised)

`dns_frontend_assigner.rs:372-411`. The criterion demands a name_index that **answers F for J once J has a running-AND-healthy backend**, byte-identical to the assigned F. The test does none of it:

- `let _name_index = NameIndex::new(obs, reader_allocator.clone());` (line ~386) is **immediately dropped** — never used.
- `answer_for` / `frontend_for` are never called; no running-and-healthy `ServiceBackendRow` is seeded (so the index's resolvable set is empty and could never answer).
- The only assertion compares `writer_allocator.snapshot()` to `reader_allocator.snapshot()` — both clones of the **same** `Arc<Mutex<BTreeMap>>` (`FrontendAddrAllocator` is `#[derive(Clone)]` over `held: Arc<Mutex<…>>`). Two snapshots of the same mutex are byte-identical *by construction*: `assert_eq!(x, x)`.

**Litmus:** mutate `answer_for`/`frontend_for` to fabricate an F or assign-on-read — this test stays GREEN. The DDN-2 single-owner invariant (an answered F always HITs `by_frontend` → never fail-closed) has no non-vacuous defense at this slice. The reader path is fully testable today (`frontend_for` is a pure reader, `name_index.rs:325-341`).

**Required:** seed a running-and-healthy backend row for `<job>=api`, `probe()` the index, and assert `answer_for(job_name("api"), …) == Records(vec![writer_f])` — byte-identical to the writer's assigned F.

### D3 — `issue (blocking)`: missing mutation gate; no MUTATION-phase evidence

The 01-05 mutation-gate criterion mandates a per-step diff-scoped `cargo xtask mutants … --file handlers.rs --file <rebuild file>` at kill-rate ≥80%, targeting the Service-arm assign guard, the idempotent-resubmit path, and the converge-on-boot enumeration. `execution-log.json` for `sid: 01-05` records **PREPARE, RED_ACCEPTANCE, RED_UNIT(SKIPPED), COMMIT, GREEN** — **no MUTATION event, no kill-rate**. Given D2, a mutation run is exactly the gate that would have flagged surviving mutants on the writer call sites.

**Required:** run the mandated diff-scoped mutation gate at ≥80% and log a MUTATION phase.

### D4 — `issue (blocking)`: RED→GREEN→COMMIT ordering inverted in the execution log

`execution-log.json` `sid: 01-05`: `COMMIT` PASS at `2026-06-25T17:37:14Z`; `GREEN` PASS at `2026-06-25T17:37:22Z` — GREEN logged **8s after** COMMIT. Under ADR-025 GREEN must precede COMMIT. Either a genuine ordering violation (committed before green) or out-of-order logging (the DES log is a contract; an inverted record cannot evidence a green-before-commit run). A Lima nextest run for 4 `#[tokio::test]`s plus compile cannot complete in 8s, which argues the GREEN entry was appended retroactively. Combined with D3, there is no trustworthy machine record that the suite was green-then-committed.

**Required:** re-establish/re-log RED→GREEN→COMMIT with timestamps reflecting actual phase execution; confirm green under `cargo xtask lima run -- cargo nextest run` for this test file.

---

## Non-blocking findings

### D5 — `suggestion (non-blocking)`: allocate-then-fail VIP leak edge introduced by inserting step 4b between the VIP commit and the admission write

`handlers.rs`: step 4a allocates **and fsyncs** the ServiceVipAllocator VIP (`:324-332`). Step 4b then `?`-propagates `FrontendRebuildError::Exhausted` (`:372-377`) — early-returning **before** `put_if_absent` (`:406`) and before the only VIP-release path (`:500`, reachable only inside the Conflict branch). On frontend-block exhaustion after the VIP is durably committed, **the VIP leaks** (no `WorkloadIntent` persisted ⇒ no `ReleaseServiceVip` ever fires). The crafter's comment (`:361-364`, "follows the SAME release-on-conflict-ONLY discipline as the VIP") does not cover this exit.

Severity is low-to-medium: the VIP is content-addressed/memoised per `spec_digest` (same-spec retry reuses it), the trigger is exhausting `10.98.0.0/16` (~65k addrs), and the allocator is empty-on-boot. But it is a genuine new asymmetry. **Fix options:** move step 4b *before* the VIP allocate, or release the VIP on the 4b `Err` path; at minimum document the bounded leak and name the exit in the comment.

### D6 — `question` → **RESOLVED (citation valid)**: #211 scopes the `release(<job>)` deferral

Verified `gh issue view 211 --comments`: the issue explicitly adds `FrontendAddrAllocator::release(<job>)` to the deletion-verb teardown producer scope, and records the hard constraint "`release` MUST NOT be wired into the **stop** path" (stop is withhold-not-release). The release-BLOCKER handling is **correct** — the crafter surfaced it, did not improvise a deletion verb, and cited a real, properly-scoped issue (CLAUDE.md "Deferrals require GitHub issues … cited by issue number" — satisfied). Grep confirms no `release` call on the frontend allocator anywhere in `handlers.rs`.

---

## Praise

- **`praise:` typed-error discipline on the boot path is exemplary.** `error.rs:564-577` + `boot_rebuild.rs:52-82` get never-flatten-to-`Internal` exactly right: `ControlPlaneError::FrontendRebuild(#[from] FrontendRebuildError)` with `#[error(transparent)]`, and `FrontendRebuildError` splits the two genuinely-distinct failure modes (`IntentScan` → HTTP 500; `Exhausted { job, source }` → HTTP 503 `frontend_exhausted`, operator-actionable). `to_response` (`error.rs:915-938`) maps each cause and documents boot-path reachability honestly. Matches the `ViewStoreBoot`/`NetnsRecovery`/`ListenerFactRebuild` pattern.
- **`praise:` reader/writer key-derivation parity.** `MeshServiceName::new(format!("{id}.{SUFFIX}"))` is byte-identical across `job_of` (`name_index.rs:137`), the rebuild (`boot_rebuild.rs:155-159`), and the handler (`handlers.rs:366-369`), with matching skip semantics on both sides — no reader/writer skip-divergence.
- **`thought:` pinned-surface compliance is otherwise clean.** No new allocator method; new public surface limited to the `FrontendRebuildError` typed enum (acceptable per never-flatten), the `rebuild_frontend_addrs_from_intent` call site, the `ControlPlaneError::FrontendRebuild` variant, and exactly one new `AppState` field default-constructed with no new ctor parameter.

---

## Quality-gate summary

- **G2 (AT fails for valid reason):** PASS.
- **G7 (100% green before commit):** UNVERIFIABLE — GREEN logged after COMMIT (D4).
- **G8 (mutation gate ≥80%):** FAIL — no mutation run (D3).
- **G9 (no test weakening to fit impl):** PASS — production call sites do the work; no fixture theater.
- **Test integrity:** ASSIGN-04 is testing theater (D2).

---

## Blocking issues to resolve before approval

1. **D1** — Surface the `mtls_worker` gating deviation as a DECISION; ratify-and-amend or re-gate.
2. **D2** — Rewrite ASSIGN-04 to drive `answer_for`/`frontend_for` over a seeded healthy backend; assert answered F == assigned F.
3. **D3** — Run + log the mandated ≥80% diff-scoped mutation gate over `handlers.rs` + `boot_rebuild.rs`.
4. **D4** — Re-establish/re-log RED→GREEN→COMMIT in correct order.

Non-blocking: **D5** (VIP leak edge — fix or document), **D6** (resolved — #211 citation valid).
