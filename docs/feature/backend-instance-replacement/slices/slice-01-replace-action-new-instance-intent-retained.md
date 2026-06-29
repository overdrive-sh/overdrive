# Slice 01 — Replace action: new instance, sentinel cleared, intent retained

> DISCUSS brief (2026-06-29). Feature: `backend-instance-replacement` (#249). Story: **US-BIR-1**.
> Job: J-OPS-003 (extended). **The walking skeleton of the feature's own work.** Effort: ≈1d.
> Mechanism DECIDED (`[D1]` in feature-delta.md): an explicit lifecycle verb; `overdrive deploy` stays pure-declare. The verb's name + semantics is the DESIGN-open part.

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
+ `overdrive job stop payments` (→ Terminated, sentinel written) + the replace action on
`payments` → sentinel cleared, `payments-1` reaches Running (new `workload_addr`),
`jobs/payments` still declared.

## Behavior (DESIGN owns API + mechanism `[D1]`)

- A production operator action (verb shape DESIGN-open) that, for a declared `jobs/<id>`
  whose current instance is operator-stopped (or running), ends the current instance,
  clears the `for_workload_stop` sentinel (TOCTOU-safe per `development.md`
  § "Check-and-act must be atomic"), and lets `WorkloadLifecycle` provision a fresh instance.
- A replace against a workload with no `jobs/<id>` row → not-found (404-shape), same posture as `stop_workload`.

## Carpaccio taste tests

- **Closes a real loop through production?** Yes — `serve` + the replace action; the new AllocationId in `alloc status` is the proof. NO test hand-clears the intent key / supplies the replacement production omits (CLAUDE.md vertical-slice rule).
- **Thinnest?** Yes — one workload, one cycle, single replica, the new-instance assertion only (address-stability is slice-02).
- **No `#[test]`-only composition?** Driven through `run_server` + the production handler + `WorkloadLifecycle`, not a hand-rolled harness.

## Acceptance (= US-BIR-1 ACs)

- [ ] Driven through `overdrive serve` + the replace action — no test-only intent-key clear / harness.
- [ ] After the action, a NEW allocation reaches Running with `A1 ≠ A2` and a distinct `workload_addr`.
- [ ] `jobs/<id>` intent row present before AND after (distinct from #211).
- [ ] `IntentKey::for_workload_stop(<id>)` cleared by the action; reconciler stops short-circuiting on `is_operator_stopped`.
- [ ] Replace against a workload with no `jobs/<id>` row → not-found error; no allocation created.

## Dependencies

- **DESIGN `[D1]`** — verb name + semantics (mechanism already decided: explicit lifecycle verb, `deploy` pure-declare) + ADR + TOCTOU-safe clearing mechanics.
- SHIPPED: `stop_workload` + `for_workload_stop` sentinel; `WorkloadLifecycle` operator-stop/SystemGc asymmetry.
