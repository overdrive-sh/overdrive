# DESIGN review round 1 — workload-spiffe-identity (2026-11-21)

Adversarial (nw-solution-architect-reviewer, Sonnet): conditionally_approved — 0 blocker, 1 high, 4 medium, 4 low.
Subtractive (Sonnet): 4 SHOULD-REMOVE, 6 CONSIDER. Persisted by the orchestrator (reviewers have no write tool).

## Consolidated fix list (orchestrator)
1. (A-H1, A-M2, S-2) Old `IssuedCertificateRow` V1 rows: greenfield single-cut — old data is discarded with the data directory. Drop every "V1 stays readable / translates `/alloc/<a>` to `Allocation(a)`" claim; V1 `into_latest` returns `EnvelopeError` (logged and skipped as observation rows are); no `/alloc/` parser survives. Keep the envelope bump procedure and the frozen-V1 golden fixture per development.md (a V1 fixture that no longer decodes after appending the larger V2 is regenerated as an explicit `Envelope::V1(...)` per the established precedent and asserts the EnvelopeError projection). One policy across ADR-0344, ADR-0345 and DD-5.
2. (A-M3) Add a `Changed Assumptions` table quoting ADR-0241's immutable association / rejection and ADR-0234's exact-allocation pin, each pointing at its superseder.
3. (A-M4) Same-call duplicate endpoint: specify "a duplicate endpoint in one `update_service` call is `LoadFailed` regardless of identity" in the contract and rustdoc, host and Sim alike; add it to the approved equivalence case.
4. (A-M5, S-1) Remove `rejectTakeover`, `UpdateNeverRefusedForIdentity`, the `update_refused` action and the `g.refused` ghost (a rejected alternative is recorded in the ADR, not as a check). `AssociationFollowsProgrammedWorkload` + `staleAssoc` carry the rule.
5. (S-6) Merge ADR-0347 and ADR-0348 into one decision ("the dataplane association carries the workload identity and follows the workload programmed at the endpoint"); ADR-0241 superseded by that one ADR; trim alternatives (c)/(d) into one; move model-check names out of the Status line; drop internal API detail.
6. (S-7) ADR-0345 stays (it owns the audit-row/restart-recovery decision) but trimmed; ADR-0346 drop the duplicated consequence and the test obligation.
7. (S-3) Remove ADR-0347's Lifecycle Gate Ownership section (lives in the delta).
8. (S-5) Keep `serve_restart_replacement` (S-PIG-71 critical scenario) and state why; drop its duplicated middle; keep `replacement_reuses_backend_id` focused.
9. (A-L6) Pin the driver's BackendId domain: ids read from receipts or observed through `update_apply`, never a driver counter.
10. (A-L7, S-4) Remove stale/process text (rename "pending", rebind traces "must be removed", no-shell note, wall time/memory, deletion history, empty decision rows); keep a one-line declined-proposals note.
11. (A-L8) Fix the glued README bullets.
12. (A-L9) List website/content/docs/concepts/identity.mdx as a FINALIZE doc item.
13. (S-9, S-10) One home for the projection tables (the delta; README points to it); trim driver-internal prescriptions and file-count lists to behavioural obligations; drop V-4/V-5.
Declined: S-8 (keep the G-1/G-2 gate blocks — the identity they compare changes meaning).
