# Slice 01 — Replace action: new instance, sentinel cleared, intent retained

> DISCUSS brief (2026-06-29). Feature: `backend-instance-replacement` (#249). Story: **US-BIR-1**.
> Job: J-OPS-003 (extended). **The walking skeleton of the feature's own work.** Effort: ≈1d.
> **DESIGN CLOSED `[D1]` — implement per ADR-0073.** Verb = **`overdrive workload restart <id>`** (new `workload` namespace); HTTP = **`POST /v1/jobs/:id/restart`** (mirrors `stop`); mechanism = the **desired-run generation precursor** (`workloads/<id>/generation`, 8-byte BE u64) gating the line-520 veto, bumped **atomically** via the NEW `TxnOp::IncrementU64` inside one `IntentStore::txn` (+ `Delete` of `/stop`). `overdrive deploy` stays pure-declare. The six pinned signatures + the R1–R5 reconciler state machine are in ADR-0073.

## Goal (one line)

The replace action ends a declared workload's current instance and clears the
operator-stop sentinel so `WorkloadLifecycle` brings up a NEW instance (NEW
AllocationId, NEW `workload_addr`) while `jobs/<id>` stays declared — driven end-to-end
through `overdrive serve` + the replace action.

## Learning hypothesis

A production action can clear `IntentKey::for_workload_stop(<id>)` atomically against a
converging stop so the reconciler's `is_operator_stopped` short-circuit
(`workload_lifecycle.rs:520`) stops firing and a fresh placement lands — **without**
withdrawing the `jobs/<id>` intent (distinct from #211) and **without** reusing the
alloc_id/slot (distinct from crash-restart).
**Predicted:** after the action, `overdrive alloc status --job <id>` shows a new
AllocationId reaching Running, a new `workload_addr`, and the `jobs/<id>` row still present.

## Thinnest serve+deploy loop

`overdrive serve` (one node) + `overdrive deploy payments.toml` (→ Running as `payments-0`)
+ `overdrive job stop payments` (→ Terminated, sentinel written) + `overdrive workload
restart payments` → generation bumped + sentinel cleared (one `IntentStore::txn`),
`payments-1` reaches Running (new `workload_addr`), `jobs/payments` still declared.

## Behavior (implemented per ADR-0073)

- `overdrive workload restart <id>` → `POST /v1/jobs/:id/restart` → `restart_workload`
  handler: for a declared `workloads/<id>` whose current instance is operator-stopped
  (or running), it bumps `workloads/<id>/generation` and deletes the `/stop` sentinel
  **atomically** in one `IntentStore::txn` (`TxnOp::IncrementU64` + `Delete` —
  read-modify-write inside the write txn, atomic + monotonic per `development.md`
  § "Check-and-act must be atomic"), then enqueues a `job-lifecycle` eval. The
  `WorkloadLifecycle` reconciler places a fresh instance when `observed_generation <
  generation` (running-origin ⇒ `StopAllocation` first, then place — per ADR-0073's
  R1–R5 table); intent stays declared.
- A restart against a workload with no `workloads/<id>` row → 404 (`NotFound { resource:
  "workloads/<id>" }`), same posture as `stop_workload`.
- Response: `200 { workload_id, outcome ∈ { restarted, resumed } }` — `resumed` when the
  `/stop` sentinel was present at the check-exists read, `restarted` otherwise.

## Carpaccio taste tests

- **Closes a real loop through production?** Yes — `serve` + `overdrive workload restart`; the new AllocationId in `alloc status` is the proof. NO test hand-clears the intent key / supplies the replacement production omits (CLAUDE.md vertical-slice rule).
- **Thinnest?** Yes — one workload, one cycle, single replica, the new-instance assertion only (address-stability is slice-02).
- **No `#[test]`-only composition?** Driven through `run_server` + the production handler + `WorkloadLifecycle`, not a hand-rolled harness.

## Acceptance (= US-BIR-1 ACs)

- [ ] Driven through `overdrive serve` + `overdrive workload restart <id>` — no test-only intent-key clear / harness.
- [ ] After the action, a NEW allocation reaches Running with `A1 ≠ A2` and a distinct `workload_addr`.
- [ ] `workloads/<id>` intent row present before AND after (distinct from #211).
- [ ] `IntentKey::for_workload_stop(<id>)` cleared by the action; reconciler stops short-circuiting on `is_operator_stopped` once `observed_generation < generation`.
- [ ] The generation bump is atomic + monotonic: two concurrent restarts advance `generation` by 2 (audited) and never wedge (ADR-0073 § item 4; the `txn_increment_u64` store acceptance test). Cardinality is **level-triggered / coalescing** — two *concurrent* (pre-placement) restarts converge to **one** fresh instance for the latest generation (NOT two); two *sequential* restarts (the second after the first placement) each cycle the workload (`payments-1` then `payments-2`). ADR-0073 § "Idempotency posture: level-triggered coalescing".
- [ ] Restart against a workload with no `workloads/<id>` row → 404 (`NotFound`); no allocation created.

## Dependencies

- **ADR-0073** (DESIGN, `[D1]` CLOSED) — the verb (`overdrive workload restart <id>`), HTTP (`POST /v1/jobs/:id/restart`), the generation precursor, the NEW `TxnOp::IncrementU64` atomic-bump primitive (+ its trait contract + the `txn_increment_u64` concurrency acceptance test), and the R1–R5 reconciler state machine. The six pinned signatures DELIVER builds are in ADR-0073 § "The six pinned signatures".
- SHIPPED: `stop_workload` + `for_workload_stop` sentinel; `WorkloadLifecycle` operator-stop/SystemGc asymmetry; `IntentStore::txn`/`get`/`delete` (the `IncrementU64` variant is the one new addition to this port).
