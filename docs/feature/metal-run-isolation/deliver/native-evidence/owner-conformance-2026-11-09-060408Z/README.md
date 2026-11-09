# Native owner conformance — 14 identities pass; randomized trace remains failed

This folder preserves the two completed native captures byte-for-byte in
`initial/` (060408Z) and `remaining/` (060632Z). The separate
`offline-seed-reconstruction/` contains diagnostic model stimuli regenerated
from the logged seed; those files are **not** original native traces or measured
owner state. No source, test, production, design or oracle was changed.

## Native results and source

| Capture | Source HEAD receipt | Actual result |
|---|---|---|
| Initial 060408Z | `eb210bb204e6d93f860faa8de20b1cc30ca35462` | Outer run `ad63893630d7914309b019f5`, exit 100; 11 passed, randomized identity failed, three cancelled |
| Remaining 060632Z | `9fb6840ddb055269ba832cea392a2318815d20e5` | Outer run `b8c8d838d254efa6867b20c9`, exit 0; the three previously cancelled identities passed |

Both receipts retain their dirty source status. The later HEAD differs by the
placement-evidence-only commit; the driver, model, durable helper and flock
fixture sources have no diff across these two HEADs. Taken together, **14/15
identities pass**: the thirteen named protocol cases and the direct illegal-
control case. `protocol_randomized_owner_traces` remains failed. The second
command's exit 0 does not replace the first command's exit 100.

The exact initial command from its invocation receipt is:

```sh
cargo xtask metal run -- cargo nextest run -p xtask --lib --features integration-tests -E 'test(metal_run_isolation_conformance::protocol_)' --no-capture
```

The exact completion command from its own receipt is:

```sh
cargo xtask metal run -- cargo nextest run -p xtask --lib --features integration-tests -E 'test(=metal_run_isolation_conformance::protocol_repeat_collection_agrees) | test(=metal_run_isolation_conformance::protocol_same_checkout_excludes_sync) | test(=metal_run_isolation_conformance::protocol_supervisor_loss_is_not_a_result)' --no-capture
```

The randomized test generated four traces at seed `QUINT_SEED=0x8c542458`,
then failed after 17.96s with `actual owner never reached retained
command/runner terminal`. The macro reports `metal_run_isolation_conformance.rs:1274`;
that is the generated test wrapper, not the production failure site. The exact
message comes from `NativeUnit::terminal` using the driver's seven-second
`poll` (`xtask/src/metal_run_isolation_conformance.rs:37-44,316-320`). Unit-not-
loaded messages also occur in passing cases, so they are not independently
classified as the cause.

## Logged-seed reconstruction and exact incompatible ordering

The native foreground did not print step traces. Pinned quint-connect 0.1.2
creates its ITF files in a managed `TempDir` (`src/trace/generator/mod.rs`,
`src/trace/iter.rs`) and drops that directory after replay, including on error.
It iterates `ReadDir` without sorting. Thus the raw capture alone does not
retain the native failing file ordinal, current step or actor's unit snapshot.

To recover the existing stimuli, one offline generation used the installed
repository-pinned Quint **0.32.0**, unchanged model SHA-256
`f67ed362807560c981a60be33d127106f47329a290f5148add70fadcbd732b5e`, exactly the logged four samples/24 steps and seed.
The exact command, hashes and limits are in
`offline-seed-reconstruction/reconstruction.json`. No owner was replayed and
no new invariant, manifest check, model definition or verification campaign
was introduced. All four generated ITFs are retained unchanged.

`run_0.itf.json` has the following relevant source-derived sequence. Indices
refer to its actual ITF states; phase values here are **model expectations**.

| State | Recorded stimulus | Relevant model state |
|---:|---|---|
| 0 | init, checkout choice 2 | Both invocations absent |
| 1 | Maintain(0), timeout branch | Maintenance resolved (3) |
| 2 | Sync(checkout 2) | Source revision updated |
| 3 | Register(invocation 2) | Invocation 2 registered, phase 1 |
| 4 | Sync(checkout 1) | Invocation 1 still absent |
| 5 | Barrier(invocation 2) | Invocation 2 owns SH, phase 2 |
| 6–7 | Idle | Phase 2 remains; no Checkout action |
| **8** | **LoseController(invocation 2)** | **Model moves phase 2→5, controller false, terminal representative 75** |
| 13 | StopOwner(invocation 2, complete) | Model phase 6 |
| 14 | Collect(invocation 2) | Collection follows stopped owner |

This is the only LoseController stimulus among these four reconstructed
traces. Before it, maintenance is resolved, invocation 1 is absent and the
second checkout has no competing owner. The concrete fixture nevertheless
keeps its **test-only checkout operation gate closed**:

1. `World::new` installs `fixtures/model-flock` as the fixture's `flock`
   (`metal_run_isolation_conformance.rs:362-368`); `NativeUnit` passes its actor,
   barrier and tree paths in the existing test-only environment.
2. Register starts the actual captured `lease-holder.sh --durable` service.
   Barrier opens only the `barrier` input and independently waits for the real
   SH descriptor (`:442-450`). Checkout is the method that opens `checkout`
   (`:453-463`), and this trace has not called it.
3. The real durable helper calls checkout flock at `lease-holder.sh:919`.
   The external adapter waits for `<actor-2>/checkout` **before** execing the
   actual `/usr/bin/flock` (`fixtures/model-flock:5-11`). This is a fixture
   gate, not observed contention in the physical kernel.
4. `World::lose_controller` closes the real controller stdin and, because the
   retained phase is not running, immediately calls `unit.terminal()`
   (`metal_run_isolation_conformance.rs:515-520`). It does not release that
   checkout gate before waiting.
5. The production helper reaches its existing EOF read/error only **after**
   checkout acquisition, owner publication and phase preparing
   (`lease-holder.sh:919-1007`). Closing stdin does not execute that read while
   the helper remains blocked in the external gate. The driver's seven-second
   terminal poll can therefore expire before the owner can observe EOF.

The source-predicted concrete state at that point is a live ordinary durable
service retaining `waiting_for_lease`, an actual SH barrier descriptor, an
unacquired checkout, no selected body and no terminal yet. Those are a
**prediction of this reconstructed stimulus/source path**, not a retained
native owner snapshot. The native record does not contain that snapshot, so
exact attribution of the logged failure to this reconstructed step is strong
source evidence rather than a measured native per-step receipt.

## Classification, collect hypothesis and falsification

The identified ordering is a **preauthored fixture/driver compatibility
failure**: the driver waits for a terminal while its own external operation
gate prevents the production helper from reaching the control read. The
current native failure does **not** establish a production P2/P5 violation.
The fully admitted named controller-EOF case passed, exercising the real EOF
path after checkout.

The premature-collect hypothesis does not explain this source path.
`World::collect` (`:646-654`) performs the actual collector call and does not
invoke `NativeUnit::terminal`; in the reconstructed trace Collect is state 14,
after StopOwner. The permanent `Collect` guard also requires phase 6 when the
production configuration has no `collectAtTerminal` hazard enabled
(`runner_protocol.qnt:103-110`). No collector oracle is relaxed here.

This diagnosis would be falsified by an actual failing-step receipt showing a
different stimulus, a released checkout gate, or an owner that had already
acquired checkout/reached its control read but still failed the required EOF
terminal behavior. No such evidence is retained in these captures. The report
does not infer that all other possible early-controller-loss schedules are
correct, prescribe a production remedy, change any action mapping, or mark
the randomized identity passed.

Only capture preservation, local source inspection and the one bounded
logged-seed stimulus reconstruction were performed for this diagnosis. There
was **no new native owner/test run, code/test/design change, formal check
campaign, oracle relaxation or DES phase event**. Original native capture
bytes remain unchanged.
