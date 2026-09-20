# Design Discipline

Repository-specific rules for DESIGN work. These bind architects, reviewers,
and orchestrators relaying architecture choices to the user.

## Surface material design decisions to the user before they become accepted

**A design document is a record of a proposal, not evidence that the user has
accepted it.** Before a material design decision is marked accepted, carried
into DISTILL, or used to authorize DELIVER, the architect or orchestrator MUST
surface it to the user in the conversation and obtain an explicit approval.
Buried ADR prose, a reviewer verdict, a roadmap, a prior issue, or silence
after an artifact was written is not approval.

This applies to every new or amended decision that changes any of these:

- the owner or meaning of a domain identity, state, lifecycle transition, or
  cleanup authority;
- a public or internal action/port contract, including the meaning of an
  existing field or parameter;
- driver-neutral versus driver-specific policy placement;
- persistence, retry, ordering, recovery, compatibility, or migration
  behavior;
- an explicitly rejected alternative whose exclusion constrains the
  implementation.

The user-facing decision request names the exact proposed contract, affected
owners and boundaries, material alternatives, and the consequence for the
next wave. Use numbered decisions so the user's approval or rejection is
unambiguous. An architect may write those decisions as **proposed** in an ADR
or feature artifact, but must not label them accepted, amend an accepted SSOT
as if ratified, or dispatch implementation from them until the user explicitly
approves them.

After approval, record the approval in the canonical design artifact with the
decision identifier and date, then proceed to independent design review and
downstream waves. If the user rejects or changes a proposal, revise the
proposal and surface the replacement; do not quietly preserve the rejected
branch as a compatibility exception.

**Reviewers enforce this boundary.** A technically coherent design whose
material decisions were not surfaced to the user is `CHANGES_REQUESTED`, not
accepted. The reviewer must name the missing user decision rather than
substituting its own judgment.

## One ADR records one decision — never use an ADR as a design bucket

An Architecture Decision Record captures **one independently decidable and
reversible architectural choice**. “System Architecture,” “Domain Model,”
“Application Architecture,” a whole subsystem, or an entire feature is an
organizational category — not one decision. If two parts could be accepted,
rejected, superseded, or reversed independently, they require separate ADRs.

An ADR contains only its decision-specific context and forces; one decision
and its scope; viable alternatives and why they lost; consequences and
trade-offs; status; and links to the authoritative architecture, interface,
diagram, and verification artifacts.

An ADR does **not** contain Rust signatures, enum variants, constructors,
accessors, trait methods, wire structs, error catalogs, file-by-file
implementation instructions, executable test obligations, scenario matrices,
fixture mechanics, mutation targets, runner commands, C4 source or
walkthroughs, roadmap steps, delivery sequencing, review history, validation
output, or a comprehensive feature specification.

Route design content to its owner:

| Content | Authoritative home |
|---|---|
| One architectural choice, alternatives, consequences | One ADR |
| Exact implementation-facing API/port contract pinned by DESIGN | `docs/feature/{feature-id}/feature-delta.md` under the relevant DESIGN `[REF]` component, driving-port, or driven-port section |
| Current high-level architecture and decision links | `docs/product/architecture/brief.md` |
| C4 diagrams and detailed relationships | `docs/product/architecture/c4-diagrams.md` |
| Executable acceptance scenarios and test obligations | DISTILL artifacts |
| Implementation order, files and quality gates | DELIVER roadmap/artifacts |

Moving Rust signatures out of an ADR does not permit leaving them unspecified:
DESIGN pins exact implementation-facing contracts in `feature-delta.md`, and
the ADR links to that contract. Never duplicate the same normative signature
in both places.

Before creating or extending an ADR, state its decision in one sentence and
ask whether any clause could be adopted or superseded independently. If so,
split those clauses into separate ADRs and move API references, C4 detail, and
tests to their owning artifacts. Reviewers block an ADR that joins independent
decisions, embeds interface or test specifications, uses an architecture layer
as its decision statement, or becomes large by acting as a feature bucket.

## Do not manufacture amendment history before implementation

An accepted ADR whose decision has not yet been implemented has no operative
historical contract to amend or supersede. When DESIGN review, DISTILL, or
another pre-implementation activity corrects or completes that decision,
incorporate the approved correction directly into the ADR's current Context,
Decision, Alternatives, and Consequences in present tense. Do not add
"amended," revision-history, remediation-history, or supersession prose that
makes an unimplemented proposal read like a sequence of shipped behaviours.

Material corrections still require explicit user approval and independent
design review. Preserve the ADR's acceptance and review provenance, but record
the resulting contract as the one current decision rather than narrating every
pre-implementation draft that led to it. Git history already retains those
drafts.

Amendment or supersession history becomes appropriate only after the prior
decision is operative: its implementation has landed, an external consumer can
depend on it, persisted or wire state exists under it, or production behaviour
otherwise makes the old contract historically relevant. At that point, retain
the old contract and state the amendment or supersession explicitly so readers
can understand compatibility, migration, and operational consequences.

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
