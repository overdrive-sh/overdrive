# BE02 acceptance review — bounded singleton correction

## Metadata

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| Acceptance item | BE02 — `complete_listener_projection_is_idempotent` |
| Reviewer | Fresh isolated `nw-acceptance-designer-reviewer` |
| Model | GPT5.6 Luna, maximum thinking |
| Reviewed | 2026-09-08 |
| Baseline | Checkpoint `cbcf9a6d205b187735c5a080be9711ed32a2116d` |
| Verdict | **APPROVED** |

## Bounded scope

This review covers only the explicitly approved BE02 correction: change seed
`257211` from a requested replica count of two to one production-scheduled
allocation while retaining its three listeners and every existing identity,
health, port, weight, listener-universe, and repeated-reconciliation equality
assertion. It does not review BE10, the other ADR-0101 scenarios, production
implementation changes elsewhere in the dirty worktree, or the broader
multi-replica scheduling design.

## Evidence reviewed

### Exact source correction

The checkpoint version declares two replicas for the three-listener case at
`service_backend_projection.rs:788-791`. The current diff changes only that
literal from `2` to `1` and updates the adjacent scenario prose to state the
singleton boundary; the listener tuples remain exactly
`18082/UDP`, `18081/TCP`, and `18081/UDP`:

```text
git diff --no-ext-diff --unified=3 cbcf9a6d205b187735c5a080be9711ed32a2116d -- \
  crates/overdrive-sim/tests/integration/service_backend_projection.rs
```

The diff has no assertion changes. The retained assertions are visible at
`service_backend_projection.rs:432-457` and `:796-820`: exact ServiceId
listener universe, one backend per listener, listener-specific address/port,
health, weight, allocation identity derived from observed allocation rows, and
full-row equality after three additional ServiceLifecycle reconciliations
(including the original logical timestamp).

The required Rust Contract Shape declaration remains exactly
`/// CONTRACT_SHAPE: bounded-change.` at
`service_backend_projection.rs:782`. The scenario narrative at `:783-786`
states the Given/When/Then behavior and explicitly limits the ordering claim.

### Production owner path and anti-fabrication check

The singleton is supplied by the composed owner path, not by a fabricated
allocation row:

1. `World::new` validates the submitted Service with `ServiceV2::from_submit`,
   archives the resulting `WorkloadIntent::Service` into the real
   `LocalIntentStore`, and rebuilds listener facts from that intent
   (`service_backend_projection.rs:229-269`). The direct allocator call in
   `:238-257` assigns the Service VIP input; it does not create an allocation
   observation.
2. `world.run("workload-lifecycle")` invokes the existing
   `run_convergence_tick` entry point (`service_backend_projection.rs:293-309`)
   over the registered production `WorkloadLifecycle` reconciler
   (`:469-479`).
3. Current WorkloadLifecycle preserves the declared Service replica field while
   hydrating (`workload_lifecycle.rs:347-360`), then performs one fresh
   placement when no allocation is Running (`:977-1005`) and emits one
   `StartAllocation` (`:1048-1105`). A subsequent normal evaluation stops at
   the existing any-Running guard (`:690-703`).
4. The serial action shim awaits the supplied `SimDriver` only as the external
   process adapter and publishes the resulting `AllocState::Running` row from
   the action path (`action_shim/mod.rs:1930-1995`). The test therefore observes
   a production-authored singleton allocation; it does not insert an
   `AllocStatusRow` or supply a second allocation through a test-only placement
   path.

The test's `identities` vector is derived from the observed allocation rows
(`service_backend_projection.rs:798-805`), and `assert_shape` independently
requires one backend per listener and distinct observed allocation identities
(`:440-455`). These assertions cannot pass merely because a favorable identity
was fabricated in the fixture.

### Historical RED and deferred boundary

The checkpoint still provides the historical two-replica RED witness: its
`257211` tuple requested `2` at the same source location. The later failure is
preserved in `red-classification.md:268-293` and
`deliver/adr-0101-test-transfer.md:291-335` as “expected two, observed one,”
explicitly classified as the current normal-convergence limitation rather than
as evidence that ServiceLifecycle dropped an existing allocation.

The revised material is honest about the boundary. The BE02 row and correction
notes in `distill/adr-0101-acceptance.md:64-110` and the test comment at
`service_backend_projection.rs:783-786` do not claim multi-allocation ordering
coverage. GitHub issue [#282](https://github.com/overdrive-sh/overdrive/issues/282)
and its follow-up comment
([specific BE02 follow-up](https://github.com/overdrive-sh/overdrive/issues/282#issuecomment-5584665927))
explicitly track basic multi-replica scheduling and restoration of two
genuinely scheduled allocations. The issue body and comment were checked with
`gh issue view 282 --comments`; their scope matches the documentation.

## Acceptance-design assessment

| Dimension / mandate | Result | Evidence |
|---|---|---|
| GWT and single behavior | PASS | One projection behavior is described at `:783-786`; repeated reconciliation is the single When-side action. |
| Business/domain language | PASS | The scenario uses Service, listener, allocation, VIP, backend identity, health, and weight; the required owner terms are precise for this integration boundary. |
| Driving-port / production composition | PASS | The test drives `run_convergence_tick` and registered production reconcilers; Sim substitutes the external process/prober ports only. |
| Observable assertions | PASS | Assertions read observed backend and allocation rows and compare complete domain values, not a private projection cache or mock call count. |
| Given fixture causality | PASS | Setup supplies validated Service intent and VIP input; WorkloadLifecycle and the action shim author the allocation and Running observation. |
| Idempotence | PASS | Three subsequent ServiceLifecycle runs are followed by full-row equality at `:813-820`, so timestamp churn is covered. |
| Contract Shape | PASS | Exact `bounded-change` rustdoc declaration remains present. |
| Scope/priority | PASS | The correction removes the proven unsupported two-replica precondition only; it does not invent replica scheduling or weaken any retained assertion. |

The broader feature's error-path ratio, walking-skeleton boundary, and other
scenario families are outside this one-test correction. They are not grounds
for expanding this review or requiring unrelated replica architecture.

### Scored review gate

The role rubric uses a 0–10 scale; approval requires every applicable
dimension to be at least 7, all mandates to pass, and no blocker.

| Dimension | Score | Rationale |
|---|---:|---|
| Happy-path bias | 8 | This bounded item is a success/idempotence scenario, while its scope explicitly excludes the feature-wide error-path ratio. |
| GWT format and single When | 9 | The rustdoc contract states the Given/When/Then behavior and the test has one bounded reconciliation flow per seed. |
| Business/domain language | 9 | Service, listener, allocation, VIP, backend identity, health, and weight are used as the domain outcomes. |
| Coverage completeness | 8 | Both seed shapes and all three listener protocols/ports remain covered; multi-allocation coverage is an owned, documented #282 follow-up. |
| Walking-skeleton/user value | 8 | The composed integration slice reaches the user-visible listener backend projection through production reconcilers. |
| Priority validation | 10 | The correction removes only the reproduced unsupported two-replica precondition and preserves the requested listener projection behavior. |
| Observable assertions | 9 | Assertions inspect persisted listener and allocation observations, complete values, identities, and equality after retries. |
| Traceability | 9 | Checkpoint diff, test, acceptance documents, RED classification, transfer handoff, and #282 use the same bounded correction vocabulary. |
| Boundary/fixture causality | 9 | The fixture seeds intent and VIP input; the production WorkloadLifecycle/action path authors the singleton Running allocation. |

All scored dimensions are at least 7. CM-A (hexagonal boundary), CM-B
(business-language assertions), and CM-C (user-journey outcome) pass.

## Findings and dispositions

No blocking or non-blocking finding was identified within the approved scope.

The following is an accepted limitation, not a defect in the correction:

| Boundary | Disposition |
|---|---|
| Multi-allocation membership and meaningful allocation-ID ordering | **Explicitly deferred.** With one observed allocation, the retained identity vector and sort remain valid singleton identity checks but cannot prove an ordering relation between multiple allocations. The test and docs say so, and #282 owns the later two-allocation production path. |

It would violate the approved scope to demand a second fabricated row, add a
replica scheduler, replace the composed owner path, or remove the three-listener
case. None is requested here.

## Verification

### Author handoff evidence

The handoff records this exact command:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test integration -E 'test(complete_listener_projection_is_idempotent)' --no-capture
```

Run `c46116bd-0a5d-402d-ae26-e2941f6e60ec` reported **1 passed, 35 skipped,
exit 0**, printed both seeds `257210` and `257211`, and is recorded in all
three directly matching correction documents. The handoff correctly labels
this as execution evidence, not independent approval.

### Independent focused execution

The same command was run independently during this review. Nextest run
`80d4d68c-9eb4-4982-ba11-f1b03382661a` reported:

```text
1 passed; 0 failed; 35 filtered out; exit 0
service-backend-projection seed=257210
service-backend-projection seed=257211
```

The current source contains no skip, ignore, early return, or conditional
around the BE02 loop; each seed executes the retained listener/identity
assertions and the three-repeat full-row equality suffix.

The scoped tracked-file check also passed:

```text
git diff --check cbcf9a6d205b187735c5a080be9711ed32a2116d -- \
  crates/overdrive-sim/tests/integration/service_backend_projection.rs \
  docs/feature/service-kind-vm-workloads/distill/adr-0101-acceptance.md \
  docs/feature/service-kind-vm-workloads/distill/red-classification.md
```

No native-metal suite, full workspace suite, mutation run, production binary,
or unrelated dirty file was run or changed. Those omissions are appropriate
to this bounded acceptance review and do not weaken its focused result.

## Verdict

**APPROVED.** The correction exactly matches the approved bounded change: seed
`257211` now reaches one genuinely WorkloadLifecycle-scheduled allocation while
retaining all three listener tuples and all existing identity, health, port,
weight, and no-stamp-churn assertions. The historical two-replica failure is
preserved and honestly classified, and the revised singleton is not presented
as multi-allocation ordering evidence.
