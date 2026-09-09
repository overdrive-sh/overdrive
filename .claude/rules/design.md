# Design Discipline

Repository-specific rules for DESIGN work. These bind architects, reviewers,
and orchestrators relaying architecture choices to the user.

## Preserve the meaning and ownership of existing states

A locally useful readiness check does not automatically own an upstream
lifecycle transition. Before proposing that a handshake, connection,
preflight, acknowledgement, capability, or dependency gate any existing
state, trace the accepted state model first.

Do not infer ownership from names. `READY`, `Running`, `Stable`, healthy,
available, connected, and converged may describe different promises owned by
different components. A mechanism being "ready" is not evidence that it may
gate a state named `Running` or `Ready`.

The order of authority is:

1. accepted ADRs and the architecture brief define the intended contract;
2. current production entry points establish what is implemented now;
3. spike evidence establishes what is technically possible;
4. a proposed design describes what may change.

Never collapse these evidence classes. If accepted design and production
ordering differ, surface the mismatch before recommending a new gate.

## Lifecycle Gate Ownership is a blocking DESIGN artifact

Every DESIGN that adds, removes, or moves a gate must contain a
`Lifecycle Gate Ownership` section. If the feature changes no gate, record
`Not applicable` with one sentence of evidence. A reviewer rejects the design
when this section is absent, incomplete, or contradicted by an ADR or a real
production caller path.

Start with the existing state-ownership matrix:

| Signal or state | Owning component | Promise it makes | Inputs that may gate it | States it must not gate |
|---|---|---|---|---|
| `{existing signal/state}` | `{owner}` | `{meaning from accepted design}` | `{current gates}` | `{unrelated states}` |

Then declare every proposed gate separately:

### Gate G-N — `{gate name}`

- **Existing evidence:** accepted ADR/brief references and the complete current
  production entry point → caller/owner → state-write path.
- **Owner:** the one component allowed to decide the gate.
- **Promise:** the exact condition established when the gate passes.
- **Affected state:** the one state or result this gate may change.
- **Failure projection:** the typed result/state produced by unavailable,
  timeout, malformed, duplicate, cancelled, and disconnected outcomes as
  applicable.
- **Explicitly unaffected:** lifecycle and health states this gate may not
  delay, advance, revoke, or reinterpret.
- **Ordering:** what must happen before and after the gate, including the one
  timeout budget it consumes. Do not multiply an existing deadline silently.
- **Counterexample:** at least one plausible case that demonstrates why the
  gate must not be moved to a broader or earlier state.
- **Evidence lane:** pure/unit, integration, seeded simulation, and/or
  real-kernel/native-metal proof required for the claimed effect.

## Gate placement rules

- Place a gate at the narrowest owner of the promise it establishes. Adapter
  availability normally becomes an adapter result; it does not become driver
  startup failure merely because failing earlier is convenient.
- Preserve existing state meanings. Moving a gate across a state boundary is
  a semantic change even when no enum or wire field changes.
- A proposal that changes state meaning or ordering requires an explicit ADR
  amendment, a `Changed Assumptions` entry quoting the superseded contract,
  and user approval before the new meaning is used.
- Keep failure in its owning domain. Driver lifecycle failures, startup-probe
  failures, readiness ineligibility, and liveness policy are not
  interchangeable error channels.
- Do not solve a first-use race by gating an unrelated earlier state. Specify a
  bounded wait or unavailable outcome at the consuming port unless the
  accepted state contract explicitly makes dependency readiness a prerequisite.
- Reconnect must not silently strengthen a state promise. State whether loss
  revokes the owned state, yields a bounded result, or is ignored, and why.

## Required boundary scenarios

The DESIGN handoff must contain executable obligations for both the affected
state and the states declared unaffected. At minimum, cover:

1. gate available — the owned state/result advances;
2. gate unavailable or timed out — the typed failure lands in the owning
   domain;
3. unrelated state — it still advances or remains unchanged as declared;
4. late success — it cannot resurrect or overwrite a newer terminal state;
5. disconnect/reconnect — ordering, cancellation, and replay behavior are
   explicit;
6. feature disabled — the pre-feature path is unchanged and the new mechanism
   is not activated.

For correctness that depends on ordering, timing, cancellation, retry, or
convergence, the handoff names a seeded `overdrive-sim` invariant. For effects
simulation cannot observe, it names the real production entry point and the
Tier-3 real-kernel or native-metal evidence. A test-only state or hand-wired
adapter is not evidence that the production gate is correct.

## Orchestrator and reviewer enforcement

Architect recommendations are hypotheses until checked against the lifecycle
matrix. Before relaying a recommendation, the orchestrator must independently
verify its cited accepted contract and production state-write path. Do not
forward a recommendation merely because its local race or failure argument is
sound.

The DESIGN reviewer blocks approval when any of the following is true:

- a gate has no named owner, affected state, or failure projection;
- the proposal uses two readiness-like words as if they were the same state;
- an adapter or probe capability gates driver/allocation lifecycle without an
  accepted contract saying it owns that boundary;
- the design changes state meaning without an ADR amendment and explicit user
  approval;
- unaffected-state counterexamples or executable obligations are missing;
- the cited failure cannot be traced through the real production entry point;
- spike feasibility is presented as proof that a particular architecture or
  state transition is required.

This is enforced first as a DESIGN artifact and blocking review contract, then
as executable state-boundary scenarios in DISTILL/DELIVER. Do not claim that
prose alone makes the invariant structurally enforced in production.
