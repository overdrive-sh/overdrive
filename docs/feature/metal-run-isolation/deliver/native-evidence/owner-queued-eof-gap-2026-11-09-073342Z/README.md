# Queued controller EOF under real EX ownership — reproduced conformance gap

## Current failure, not the earlier seed

`current/` preserves the completed 073342Z native capture unchanged. Its
source receipt names `e26f0448aaa09f692b7bb2be2d601a87ea79c43d` with the actual dirty source
status. Outer run `90f2673b92e51f2dd5f2ba16` returned **100**: eleven identities
passed, the randomized identity failed and three were cancelled. The current
seed is **0x8e8e19a4**, not the prior 0x8c542458.

```sh
cargo xtask metal run -- cargo nextest run -p xtask --lib --features integration-tests -E 'test(metal_run_isolation_conformance::protocol_)' --no-capture
```

The current telemetry already shows LoseController(actor 1), maintenance 1,
no terminal and both fixture operation gates present. This differs from the
previous closed-checkout-gate failure: the operation is now blocked by the
actual exclusive kernel lock, not an unreleased operation input.

## Exact targeted reproduction, no behavioral correction

`reproduction/` preserves one targeted native replay and its diagnostic driver
diff. Source HEAD receipt: `e26f0448aaa09f692b7bb2be2d601a87ea79c43d` plus the exact recorded
dirty paths. The controller-loss handler was verified byte-identical to e26f0448
before replay. Only the existing macro's temporary seed input and failure-only
actual-maintainer diagnostics changed. No public env/config or model surface
was added; the temporary seed is absent from the final source.

```sh
cargo xtask metal run -- cargo nextest run -p xtask --lib --features integration-tests -E 'test(=metal_run_isolation_conformance::protocol_randomized_owner_traces)' --no-capture
```

Run `38ca82b4f0c75177bc49fb4a` returned **100**, selecting only the randomized
identity at the same **0x8e8e19a4**. It reproduced the terminal-poll failure and
retained the other real owner before cleanup:

| Actual owner | Direct observation |
|---|---|
| Ordinary actor 1, unit `aa7d6c91d1c231b4ac7668f1`, MainPID 2468380 | Active/loaded; retained phase `waiting_for_lease`; terminal NotFound; barrier FD open with no FLOCK; no checkout FD/selected body |
| Actor 1 fixture inputs | Both `barrier` and `checkout` files present; `short-wait`, `stop` and `body.pid` absent |
| Maintainer, unit `5e098265a58b0f6613f3837f`, MainPID 2468424 | Active/loaded; actual phase `preparing`; actual kernel barrier mode **exclusive**; no terminal or body.pid |
| Decoded failing stimulus | **LoseController(actor 1, value 0)**; model expects actor 1 phase 5/term 75 while maintenance remains **1** |

The large actual argv and full descriptor tables remain raw in the log; this
README does not replace them with invented owner records. `lock_mode` reads the
actual maintainer's `/proc/<MainPID>/fd` and fdinfo for the same fixture barrier.
The maintainer tuple is an observation, not the model's maintenance value or
an inference from the driver's handles.

## Concrete entry/ordering mismatch

The real ordinary `lease-holder.sh --durable` opens its barrier FD and invokes
`flock -s` at `infra/metal/lease-holder.sh:869-880`. With both test operation
gates open, `fixtures/model-flock` delegates to the real `/usr/bin/flock`.
The shared acquisition is correctly excluded by the measured EX owner.
Production does not read the control pipe until **after both locks**, owner
publication and phase preparing (`lease-holder.sh:919-1007`). Closing controller
stdin therefore leaves the ordinary main queued on the real kernel barrier;
it cannot reach the EOF terminal while that EX ownership remains.

The maintainer's accepted model state 1 deliberately retains EX before the
controlled maintenance command: `World::maintain(1)` acquires it; the later
Maintain(2)/(3) inputs start/end the controlled command and release its owner.
No such later stimulus runs while the synchronous LoseController handler is
awaiting terminal.

The approved model nevertheless permits LoseController from phase 1/2 and
atomically moves the run to **phase 5, terminal 75**, with maintenance unchanged
(`runner_protocol.qnt:76-80`). The pinned actual-state mapping defines phase 5
as main-ended/unit-live, not a live queued service with a closed controller
handle. The required poststate `(maintenance=1, run1=5/75)` cannot be obtained
by the current helper's EOF-read-after-acquisition ordering while the other
owner continues holding EX.

This is a **production/model queued-EOF conformance gap**, not the old
closed-operation-gate starvation. The capture proves the real queue and held
EX; it does not by itself choose whether the intended contract requires early
queued cancellation in the current production owner, or the approved model
must represent asynchronous pending EOF until a legal admission/deadline
transition. That lifecycle/error timing decision must be resolved at the
existing owner/model boundary before a correction is accepted. No new
architecture or production interface is proposed by this report.

## Why no fixture-only GREEN was manufactured

Releasing or advancing the independent maintainer outside its modeled actions
would change the required maintenance state. Projecting a queued main as phase
5/term 75 from the closed controller handle would invent observed state.
Injecting a signal changes the termination/control cause, and shortening a
real flock wait to force `maintenance_timeout` substitutes a different typed
cause for the required EOF behavior. Waiting longer does not preserve the
required state vector: EX must release, a deadline wins, or fixture runtime
termination occurs first. The model, actual-state oracle, exact EOF cause,
source/body assertions and all seven/60/120-second bounds remain unchanged.

There is no private file gate left to open, and no honest scheduling-only
correction within the current pinned poststate was identified. The affected
path is pre-admission controller loss while actual exclusive ownership is
held. Other P1/P3/P4 behavior and previously passed owner identities are not
reclassified by this result.

## Scope and checks

Only failure-context support was extended to read the actual maintainer's
unit, phase, kernel lock mode and terminal. The temporary existing seed
attribute was removed, restoring normal randomized selection. The submitted
`final-diagnostic.diff` contains no controller-loss behavior/projection/model
change. Canonical Lima strict xtask test Clippy and scoped formatting passed.
No all-fifteen rerun, model generation/check campaign, production change,
mutation test, signal/timeout workaround or fabricated DES phase event was
performed. Both original captures are byte-identical; the randomized identity
remains failed and the queued-EOF path is a genuine blocker.
