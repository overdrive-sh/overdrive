# Adversarial review — DESIGN, workload-resources-human-readable (GH #372)

Reviewer: nw-solution-architect-reviewer (Sonnet), 2026-11-20. Persisted by the orchestrator (the reviewer has no write tool).

**Verdict: CHANGES_REQUESTED** — zero correctness BLOCKERs; four MAJOR artifact-contract defects.

Verified correct: CH citations (`memory_manager.rs:474-481, 635-637, 1499-1514`); no-envelope-bump claim (`Resources` the only rkyv carrier, `traits/driver.rs:311-326`; `spec_digest` over `rkyv::to_bytes`, `aggregate/mod.rs:855-858`; wire `ResourcesInput` not rkyv; `NodeConfig` not persisted); admission coverage via `Job::from_submit` / `Service::from_submit` (CLI `deploy.rs:251,324,577,664`, server `handlers.rs:298,306`; `Schedule::from_submit` is a `todo!()` scaffold); grammar tables vs EBNF; Display/FromStr/serde roundtrip; describe `CPU:` placement under the presence guard (`render.rs:582-586`).

1. MAJOR — Validation table row V-1 says "NOT RUN … blocker" (`feature-delta.md:549`) while later sections say HELD. Fix: row → HELD, cite findings + run id.
2. MAJOR — DISCUSS D-5 (`:48`), AC-1.6 (`:79`), AC-2.6 (`:106`), DoD 3 (`:113`) and likely the slice briefs still demand an envelope bump. Fix: reword to "existing golden fixtures decode unchanged through the new types"; same in slices.
3. MAJOR — ADR-0339 joins the whole-MiB workload rule (decision 5a, independently reversible) and embeds examples. Fix: new short ADR-0343 for DD-8; ADR-0339 links to it; examples stay in the feature delta.
4. MAJOR — Decomposition / grep gate miss live surfaces: `tests/conformance/src/gateway.rs:1701`, `tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs:45` (JSON with old fields, runtime-only failure under `deny_unknown_fields`); `verification/expectations/E06-vm-job-deploy-reaches-running/runner.sh:361-362`; `benchmarks/catalogue.md:11`; raw TOML/JSON literals in `node_volume_boundaries.rs:463`, `quint_node_volume.rs:202-203,421`. Exclude unrelated metric label `mtls_path.rs:89,155`.
5. MINOR — Sim tests submit 256 KiB memory (`netns_density_activation_order.rs:659`, `netns_density_retiring_cleanup.rs:711`, `netns_density_reclaim.rs:587`) which DD-8 refuses; not mechanical. Raise to ≥1 MiB, keep paired `Resources` literals consistent.
6. MINOR — `0.5000` classified `TooFine` but equals 500m exactly. State the rule as "at most three fractional digits"; rename variant (e.g. `TooManyDecimals`).
7. MINOR — `Malformed` vs `Unknown*` boundary inconsistent (`1e3MiB`, `5 cores`). Pin one rule (purely alphabetic tail after the digit run → `Unknown*`, else `Malformed`).
8. MINOR — ADR-0310 Status narrates the change (forbidden pre-implementation); `brief.md:2185` still says PROPOSED.
9. MINOR — ADR-0341 Decision lists struct/function names and states the no-bump fact. Cut to the decision; move the list to the feature delta; layout fact to Consequences.
10. MINOR — Slicing implicit: one `Resources`/`ResourcesInput`/`ResourcesBody` carries both fields; state whether DELIVER is one cut or re-slice. Add: `Schedule::from_submit` must route through `Job::from_submit` when it lands.
