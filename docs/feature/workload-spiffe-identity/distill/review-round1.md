# DISTILL review round 1 — workload-spiffe-identity (2026-11-21)

Sentinel (nw-acceptance-designer-reviewer, Sonnet): conditionally_approved, 0 blocker / 5 high. Subtractive (Sonnet): 7 SHOULD-REMOVE / 7 CONSIDER. Persisted by the orchestrator.

## Apply
- SEN-1: `identities_that_are_not_exactly_a_workload_path_name_no_workload` — construct every input first, then assert; drop inputs `SpiffeId::new` itself rejects.
- SEN-2: S-WSI-09 `expected` → the refusal text ("the dataplane refused the update" or the exact refusal Display); assert the programmed fingerprint is unchanged on refusal; drop the "ServiceLifecycle/WorkloadLifecycle order" claim (only idle ticks vary).
- SEN-3: add one pending-RED acceptance test discharging V-3: a Running allocation driven through BackendDiscoveryBridge / ServiceLifecycle reconcile yields `Backend.workload_identity == SpiffeId::for_workload(w)`.
- SEN-5 / SUB-10: held files stay gated (no compilable form exists for 3 of 4 tests) but (a) compile-prove their bodies once against a throwaway local patch (re-key `ever_issued`, V2 alias) in a /tmp worktree, never committed, and record the result in red-classification; (b) move `every_allocation_of_a_workload_is_issued_the_workload_identity` out to an active pending-RED test.
- SEN-6: S-WSI-03 — for `FIXTURE_V1_UNDER_V2_ROOT` assert the specific envelope error kind; keep `is_err()` for the pre-V2 literal; fix the delta text to "`FIXTURE_V1` stays untouched"; record how `FIXTURE_V2` was produced and that it round-trips; check `print_fixture_v1_bytes` / `canonical_v1_payload` usage.
- SEN-7: randomized Quint test — make it deterministic (pin `QUINT_SEED` / sample count) or remove its should_panic and keep it unmarked only if it can't flake; the named run already proves the RED.
- SEN-8: driver doc vs `PROBED_BACKEND_IDS` probing; `read_identity` must not `drop` a Result (propagate non-`BackendIdentityMissing` errors); note `held` cookie translation; fix stale header text.
- SEN-9: record in § Fault obligations that a same-workload replacement passing G-2 is covered only by Metal S-PIG-71 (or add an S-PIG-64-style equality case if cheap and within approved scope — prefer recording).
- SEN-10: attach the `BackendIdentityConflict` / update-failed log line from the S-PIG-71 Metal run to the RED evidence; state why S-PIG-71 is not should_panic and that the Metal lane stays red until DELIVER.
- SUB-1: remove S-WSI-08 (a) and (b) (duplicates of S-WSI-09/10 and Quint runs); keep S-WSI-08 (c) as the only default-lane check of duplicate-endpoint `LoadFailed`.
- SUB-4, SUB-5: remove the dead `_workload` param and the `issue_for_both` wrapper.
- SUB-6: drop the `held_snapshot().is_empty()` fresh-constructor assertion.
- SUB-7: keep one of the two `/alloc/`-shape rejection tests.
- SUB-11: drop the H-1 restatement test once moved (SEN-5b keeps one instance only).
- SUB-13: fix test counts in the delta. SUB-14: rustfmt the new files.
## Declined
- SUB-3 (enumerate 12 schedules) — the approved contract is a seeded invariant; keep. SUB-8 pre-V2 half — Sentinel keeps it. SUB-9 — S-WSI-10(a) is in the approved equivalence case.
## For the user
- SEN-4: the Quint conformance driver `quint_selection_receipt.rs` is placed `host_pure` but mutates node-global kernel state (veth, bpffs, XDP, cgroup/connect4) — pre-existing.
- The held-test gate marker (`distill-pending-owner-api` on ordinary acceptance tests) is not sanctioned by testing.md.
