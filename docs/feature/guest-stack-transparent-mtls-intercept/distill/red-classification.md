# RED classification — bounded lifecycle/network correction

**Original classification base:** `465b96c39083984a1d2d470caff918a723b9301f`

**Feature:** `guest-stack-transparent-mtls-intercept`

**Current amendment:** 2026-09-01 BTR-3 lifecycle-port test transition

## Current classification

| Scenario | Current executable state | Classification |
|---|---|---|
| S-GTI-BTR-01 | `TerminalContentionConverges` is registered as `terminal-contention-converges` and drives the real Stop/exit-observer LWW race. The focused control-plane test retains the distinct 0/1/2-proposal and typed-error table. | `IMPLEMENTED_GREEN` |
| S-GTI-BTR-02 | `VmProvisionFailureCleansNetworkAndReusesSlot` is registered as `vm-provision-failure-cleans-network-and-reuses-slot` and drives seeded partial-network cleanup plus smallest-free slot reuse. The focused control-plane test retains teardown-failure/store-precedence complements. | `IMPLEMENTED_GREEN` |
| S-GTI-BTR-03 | ADR-0089 §7 now requires the exact `MtlsInterceptLifecycle` port and socket-free `SimMtlsInterceptLifecycle`. Those production shapes are not yet present, so the Tier-1 same-ID invariant has an intentional Rust RED scaffold. | `RED_SCAFFOLD_PRESENT` / `MISSING_FUNCTIONALITY` |

BTR-3's canonical future invariant name is
`same-id-restart-removes-prior-protection-before-replacement-provision`.
Registration and the real evaluator are DELIVER work because registering an
evaluator that cannot yet compose the approved trait/adapter would either
break the default DST catalogue for the wrong reason or fabricate a GREEN
model. The executable DISTILL scaffold therefore pins the scenario name,
fixed seed, reproduction command, partitions, and fail-for-right-reason while
leaving production API untouched.

## BTR-3 fail-for-right-reason contract

The missing functionality is the approved high-level lifecycle seam and its
socket-free Sim adapter, not the already implemented restart order itself.
The activated invariant must:

- establish prior `Live` ownership through a successful production
  `StartAllocation` dispatch rather than preloading state;
- drive same-ID `RestartAllocation` through the real dispatcher;
- prove lifecycle stop completes before structural teardown/slot release and
  replacement provision → identity → driver start → lifecycle start;
- cover clean, transient lifecycle-stop, and transient structural-teardown
  partitions, with no later event at either failure cut;
- exercise replacement network-provision, identity, and driver-start failures,
  asserting lifecycle absence and no later replacement event while leaving
  their detailed error/cleanup assertions in the existing focused tests;
- distinguish `TeardownPending` from absence and reject stale/partial owner
  snapshots;
- converge within one additional dispatch to exactly the same ID `Live`, with
  exactly one replacement driver start and lifecycle start; and
- print the seed and fail its pure checker under the specified deletion/
  reorder negative control.

The integration sibling is transitioned to independent real-worker evidence:
it checks real loopback-listener closure and guard drop before teardown returns
on success and typed teardown failure. It no longer duplicates the Tier-1
driver/network/identity ordering oracle.

## Targeted execution

```text
cargo xtask lima run -- cargo nextest run -p overdrive-sim \
  --test acceptance \
  -E 'test(same_id_restart_removes_prior_protection_before_replacement_provision)'
```

The expected DISTILL result is one runner pass caused only by the exact
`RED scaffold` panic. A compile/import failure or different panic is `BROKEN`.
Nextest run `13ba4726-9ccf-46c4-bac8-c8e9a44e2bc6` executed exactly that
test and passed by matching its intentional marker; 95 tests were filtered
out.
After DELIVER activates and registers the invariant, exact reproduction is:

```text
cargo dst --seed 424242 --only same-id-restart-removes-prior-protection-before-replacement-provision
```

## Historical immutable-baseline record

At `465b96c39083984a1d2d470caff918a723b9301f`, all three original action-shim
scaffolds were correctly RED: Stop used an unbounded proposal loop,
post-assignment provision failure skipped structural unwind, and same-ID
restart entered replacement provisioning before prior mTLS/network cleanup.
The recorded Lima nextest run
`d3823317-38f3-4b90-b6ea-0ebc55196dd2` passed all three intentional
`#[should_panic(expected = "RED scaffold")]` bodies. That historical evidence
is retained here but is superseded as a statement of current executable state
by the table above.

## Scope audit

- No production API, adapter, error, expectation, example, shell/Python
  harness, roadmap, review artifact, or mutation configuration is changed by
  this DISTILL amendment.
- BTR-1/BTR-2 registered invariants and focused complementary tests are not
  redesigned or generalized.
- BTR-3's Tier-3 evidence is restricted to real socket/worker/guard effects;
  deterministic cross-port ownership/order remains Tier 1.
- Pre-existing dirty architect and repository-instruction files are excluded
  from this amendment and any eventual commit.
