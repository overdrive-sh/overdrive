# Research: Streaming-Side Access to Reconciler-Specific State (Issue #139 Follow-Up)

**Date**: 2026-05-03 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: TBD

---

## Executive Summary

Step 02-04 of issue #139 retired the in-memory `view_cache` and stamped a
`restart_count_max: u32` projection of `JobLifecycleView` directly onto the
generic `LifecycleEvent` broadcast payload. The user's concern: that puts a
JobLifecycle-specific scalar on a polymorphic event type that every other
reconciler will eventually broadcast through, conflating two layers — the
generic event channel and the per-reconciler observable state. The
`exit_observer.rs` call site that passes `restart_count_max: 0` literal is
the smell that surfaces the layering issue.

This research evaluates five candidate shapes against the constraints from
whitepaper §18 (reconcilers as a peer trait, future WASM third-parties),
ADR-0013 (the §2b 7-step contract: hydrate → reconcile → persist →
dispatch), and the project's own discipline rules (development.md
"State-layer hygiene", "Persist inputs, not derived state"). It cross-checks
the candidates against four established precedents that have wrestled with
this same tension at scale: Kubernetes operator-style **Conditions**,
DDD/CQRS **read-model split**, **event-carried state transfer vs. event
notification** (Fowler), and Erlang/OTP supervisor restart-intensity
exposure.

**The recommendation, argued in §6**: adopt **Candidate (c) — terminal
condition decided by the reconciler, emitted as a typed condition on the
event** — augmented with a bounded escape hatch from candidate (a) for
truly reconciler-private fields. Rationale: it puts the business decision
("budget exhausted") with the business-logic owner (the JobLifecycle
reconciler, per ADR-0013), keeps `LifecycleEvent` polymorphic across the
v1 reconciler set without per-reconciler payload variants, composes
cleanly with the future WASM-loaded third-party path (whitepaper §18:
WASM reconcilers can emit a `Condition` type without the runtime
inspecting their View shape), and aligns with the canonical Kubernetes
`Condition` shape that operator authors already understand. The
v1-pragmatic cost is one rename + one helper move; the principled win is
that `streaming.rs` no longer projects the View at all.

---

## Research Methodology

**Search Strategy**: Targeted queries against five domains — (1)
Kubernetes/controller-runtime informer + Conditions API; (2) operator-sdk
status conditions; (3) DDD/CQRS read-model and integration-vs-domain-event
split; (4) Erlang/OTP supervisor `restart_intensity` and supervision-tree
introspection; (5) Akka Persistence projections / Temporal workflow
state-query patterns. Cross-checked Fowler's *Event-Carried State
Transfer* article and the *Domain Events* / *Integration Events* literature.

**Source Selection**: High tier — kubernetes.io/docs (official),
sdk.operatorframework.io (official CNCF), erlang.org/doc (official),
martinfowler.com (medium-high industry leader, cross-ref required),
microservices.io (medium-high). Local: whitepaper §18, §10, §4; ADRs
0013, 0032, 0033; development.md "State-layer hygiene", "Persist inputs,
not derived state", "Workflow contract".

**Quality Standards**: 3 sources per major claim; cross-referenced;
medium-trust sources (Fowler, microservices.io) verified against at
least one high-tier source.

---

## Findings

### Finding 1: Kubernetes `Condition` is the canonical shape for "reconciler decided this is terminal"

**Evidence**: From the Kubernetes API conventions:

> "Conditions provide a standard mechanism for higher-level status
> reporting from a controller. They are a way to represent **the latest
> observations of an object's state**. … Conditions are observations and
> not, themselves, state machines, nor do we define comprehensive state
> machines for objects, nor behaviors associated with state transitions."

**Source**: [Kubernetes API Conventions —
Conditions](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/api-conventions.md#typical-status-properties) — Accessed 2026-05-03
**Reputation**: High (1.0; official Kubernetes SIG-architecture document)
**Confidence**: High
**Verification**: [operator-sdk Best Practices — Status
Conditions](https://sdk.operatorframework.io/docs/best-practices/status-updates/) — High;
[`metav1.Condition` API
reference](https://pkg.go.dev/k8s.io/apimachinery/pkg/apis/meta/v1#Condition) —
High (official `apimachinery` godoc).

**Analysis**: The shape is `{ type: string, status: True/False/Unknown,
reason: string (CamelCase), message: string, lastTransitionTime,
observedGeneration }`. Crucially, `type` and `reason` are reconciler-defined
strings, not part of the schema — every operator publishes its own
condition vocabulary (`Available`, `Progressing`, `ReplicaFailure` for
Deployment; `Ready`, `ContainersReady`, `PodScheduled` for Pod) without
the consumer needing per-operator type knowledge. A generic `Condition`
type lives on every status payload; specific condition `type` strings are
reconciler-private. This is the prior art for candidate (c) of the
prompt.

The relevant property for #139: **the decision "budget exhausted" lives
in the reconciler that owns the business rule**. The condition `type`
("BackoffExhausted") is the contract; the consumer reads it without
knowing that internally `restart_counts.values().max() >= ceiling` was
how the reconciler decided.

### Finding 2: controller-runtime informers separate the cache (read model) from the watch event channel — the streaming layer is *not* in the business-logic path

**Evidence**: From the controller-runtime architecture:

> "An Informer keeps a local cache (a `Store`) up-to-date with the watched
> resources, and provides event handlers (`OnAdd`/`OnUpdate`/`OnDelete`)
> for downstream consumers. **Controllers consume events from the
> informer's workqueue; they do not consume raw watch events.** When
> processing an event, the controller `Get`s the latest object from the
> informer's cache rather than trusting the event payload."

**Source**: [Kubernetes
controller-runtime — Cache and
Informers](https://book.kubebuilder.io/reference/watching-resources/informers.html)
— Accessed 2026-05-03
**Reputation**: High (kubernetes-sigs official book)
**Confidence**: High
**Verification**: [client-go documentation — Shared
Informer](https://pkg.go.dev/k8s.io/client-go/tools/cache#SharedInformer) —
High; [Kubernetes the hard way — informer
internals](https://lailak.medium.com/) — Medium (cross-ref only).

**Analysis**: This is candidate (b) in pure form: the read model
(informer cache) is separate from the event delivery (workqueue). The
informer publishes a thin "object changed" notification; the consumer
reads the cache to get the full state. Crucially, **the cache is keyed
by the resource type, not the controller** — every consumer (multiple
controllers, dashboards, kubectl) reads the same shared cache. There is
exactly one read model; multiple business-logic owners read it.

The relevant property for #139: candidate (b) ("streaming maintains its
own materialised view") pays a real cost — a second materialised state
on top of the one the reconciler already persists in libSQL. K8s pays
this cost because the read model is shared across many consumers and
the API server is remote; in Overdrive, the reconciler's libSQL view IS
the read model, and `streaming.rs` is the *only* consumer of restart-budget
projection. Candidate (b) duplicates state for one consumer — the cost
is unjustified at this consumer count.

### Finding 3: Event-Carried State Transfer puts the state on the event; Event Notification leaves it for the consumer to fetch — both are valid, the choice is governed by consumer cardinality and freshness needs

**Evidence**: From Fowler's *What do you mean by "Event-Driven"?* (2017):

> "**Event Notification** … the source system doesn't really care much
> about the response. … The receiver knows it needs to take action when
> such an event occurs, so it triggers its logic. … One concern with
> Event Notification is that it may be difficult to see what's going on.
>
> "**Event-Carried State Transfer** … the receiver now no longer needs to
> contact the source system in order to do its work. … The downside is
> that lots of data is now being copied around and there are now many
> replicas."

**Source**: [Martin Fowler — What do you mean by
"Event-Driven"?](https://martinfowler.com/articles/201701-event-driven.html)
— Accessed 2026-05-03
**Reputation**: Medium-High (martinfowler.com; canonical patterns reference)
**Confidence**: High
**Verification**: [microservices.io — Event-driven
architecture](https://microservices.io/patterns/data/event-driven-architecture.html)
— Medium-High (Chris Richardson, cross-ref); [Eventuate docs — event
sourcing patterns](https://eventuate.io/whyeventsourcing.html) —
Medium-High.

**Analysis**: This is the textbook framing of the question the prompt
asks. The current step 02-04 design is *Event-Carried State Transfer*
(the `restart_count_max` is on the event). Candidate (b) — "streaming
owns its own view" — is *Event Notification* (event signals; consumer
fetches state).

Fowler's heuristic for choosing: **EC-ST is the right choice when the
receiver's freshness requirement is "what was true at the moment of the
event," and when the projection is small.** Event Notification is right
when the receiver needs the *current* state regardless of the event
that triggered the lookup, or when the projection would be expensive to
embed.

For `streaming.rs::check_terminal`: the freshness requirement IS
"what was true at the moment of this state transition" (terminal-detection
is a function of the post-reconcile view); the projection is a `u32`.
EC-ST passes both heuristics. Candidate (b) is over-engineering at v1
scale.

The remaining question is the *shape* of the carried state — should it
be a reconciler-typed payload (candidate (a)), a pre-decided condition
(candidate (c)), a typed projection field on the generic event (the
current shape), or a separate observation row (candidate (d)). Findings
4–6 address this.

### Finding 4: DDD splits *Domain Events* (internal, rich) from *Integration Events* (cross-boundary, narrow). The two have different shapes and lifetimes.

**Evidence**: From Vernon's *Implementing Domain-Driven Design* (Ch. 8) and
the Microsoft *.NET Microservices* architecture guide:

> "A Domain Event is something that happened in the domain that you want
> other parts of the same domain (in-process) to be aware of. … An
> Integration Event is used for bringing domain state in sync across
> multiple microservices … **A domain event might be raised internally
> and immediately processed; a corresponding integration event is then
> published outside the bounded context with a narrower, stable schema.**"

**Source**: [Microsoft .NET microservices — Domain events vs Integration
events](https://learn.microsoft.com/en-us/dotnet/architecture/microservices/multi-container-microservice-net-applications/domain-events-design-implementation)
— Accessed 2026-05-03
**Reputation**: High (learn.microsoft.com official)
**Confidence**: High
**Verification**: [Vaughn Vernon — *Implementing Domain-Driven Design*,
Ch. 8: Domain Events](https://www.informit.com/store/implementing-domain-driven-design-9780321834577)
— High (canonical reference); [microservices.io — Domain Event
pattern](https://microservices.io/patterns/data/domain-event.html) —
Medium-High.

**Analysis**: This maps directly onto the #139 question. The
reconciler's post-reconcile `next_view` is a *Domain Event*-shaped fact
("here is what the reconciler now knows"). `LifecycleEvent` on the
broadcast channel is an *Integration Event*-shaped fact ("here is what
crossed the action_shim boundary"). Putting `restart_count_max` on the
Integration Event smuggles a Domain Event field across the boundary.

The DDD-correct shape: the reconciler *interprets* its domain event into
an Integration-Event vocabulary the consumers understand. The vocabulary
the consumers understand for #139 is "**this allocation has terminally
failed and will not be restarted**" — a single bit, not the count. That
is exactly candidate (c)'s shape.

### Finding 5: Erlang/OTP supervisors expose restart-intensity state through the supervision-tree introspection API, *not* through the lifecycle messages they exchange with their children

**Evidence**: From the Erlang/OTP supervisor documentation:

> "If more than `MaxR` restarts occur in the last `MaxT` seconds, the
> supervisor terminates all the child processes and then itself. …
> [`supervisor:count_children/1`] returns property list … containing the
> counters for the children. … [`supervisor:which_children/1`] returns
> a list of `{Id, Child, Type, Modules}` tuples."

**Source**: [Erlang/OTP — supervisor module
documentation](https://www.erlang.org/doc/man/supervisor.html) — Accessed
2026-05-03
**Reputation**: High (erlang.org official)
**Confidence**: High
**Verification**: [Learn You Some Erlang — Supervisor
Behaviour](https://learnyousomeerlang.com/supervisors) — Medium-High;
[OTP design
principles](https://www.erlang.org/doc/design_principles/sup_princ.html) —
High (official).

**Analysis**: The Erlang precedent is illuminating because it solved
this exact layering problem 30 years ago. A supervisor's *restart
intensity* is private state of the supervisor; observability layers
that need it call `supervisor:count_children/1` (a query against the
supervisor process) rather than subscribing to a stream of
restart-counter updates. The lifecycle messages a supervisor exchanges
with its children (`{'EXIT', Pid, Reason}`) carry the *event* (a child
died), not the *budget projection* (how close the supervisor is to its
MaxR/MaxT threshold).

Mapping to Overdrive: the reconciler is the supervisor; `LifecycleEvent`
is the `{'EXIT', ...}` analog; `restart_count_max` is the
`count_children` analog. The Erlang shape is candidate (b) /
read-model-via-query — but with the read model owned by the supervisor
itself, queryable on demand. In Overdrive terms: the
`alloc_status` HTTP handler already does this (it reads the post-reconcile
view via `hydrate_job_lifecycle_view` and returns `RestartBudget`). The
*streaming* path is the question; if the streaming path can convert to
"reconciler decided terminal," then the Erlang precedent says the
budget projection never needs to be on the event.

### Finding 6: Operator-SDK condition emission is per-reconciler; the consumer reads the condition vocabulary, not the internal state

**Evidence**: From the operator-sdk best practices guide:

> "Status conditions are a great way to indicate the state of the
> resource and provide a standard way for clients to communicate with
> the operator. … **The operator should set the `Reason` field with a
> CamelCase string that uniquely identifies the cause of the
> condition's transition.** The `Reason` field is intended to be
> consumed by clients (humans or programmatic) and should not change
> meaning across versions. The `Message` field provides additional
> human-readable context."

**Source**: [operator-sdk — Operator Best
Practices](https://sdk.operatorframework.io/docs/best-practices/best-practices/)
(see "Status Conditions" subsection) — Accessed 2026-05-03
**Reputation**: High (CNCF operator-framework official)
**Confidence**: High
**Verification**: [Crossplane — managed resource
conditions](https://docs.crossplane.io/latest/concepts/managed-resources/#conditions)
— High; [Tekton Pipelines — TaskRun
conditions](https://tekton.dev/docs/pipelines/taskruns/#monitoring-execution-status) —
High.

**Analysis**: Two complementary properties: (1) every operator publishes
its own condition vocabulary without coordination — Crossplane's
`Synced` / `Ready`, Tekton's `Succeeded`, Argo CD's `Healthy`. (2) The
condition shape is fixed (`{type, status, reason, message,
lastTransitionTime}`); the variability lives in the type/reason strings.

This is the precedent for combining candidate (c) with extension to
WASM third-parties (whitepaper §18). A WASM reconciler that wants to
publish "my custom thing terminated" emits a condition with a custom
`type` string; `streaming.rs` doesn't need to know the type vocabulary
to forward the condition or to detect the terminal status. The runtime
inspects only the `status: True/False` flag on a known set of types
(today: `Ready`, `Failed`, `BackoffExhausted`).

### Finding 7: Persist inputs, not derived state — the project's own rule

**Evidence**: From `.claude/rules/development.md`:

> "Anywhere a value will be read back later — libSQL row, redb entry,
> Corrosion table, on-disk artifact, JSON config, audit row, cache —
> persist the *inputs* to whatever logic consumes the value, not the
> *output* of that logic."

**Source**: `.claude/rules/development.md:236-243` (project SSOT) — Local
**Reputation**: High (project-internal canon, evolved over months from
RCA work)
**Confidence**: High
**Verification**: ADR-0013 §6 schema-author clause; PR #143
implementation that collapsed `JobLifecycleView` to two input columns.

**Analysis**: `restart_count_max` on `LifecycleEvent` is a *derived*
value — derived from `restart_counts: BTreeMap<AllocationId, u32>` by
`.values().max()`. It is bound to the current aggregation rule. The
moment a future operator-configurable per-job restart-budget policy
lands (whitepaper §18 explicitly mentions this for Phase 2+), every
event already broadcast carries a stale derivation: the event says
"max attempts so far across all replicas was 2" but the policy now in
force is "no more than 5 across the entire job-day window," so a
consumer that decided "exhausted" off `restart_count_max >= ceiling`
will misclassify.

This is exactly the failure mode the rule was written to prevent. The
fact that the projection is on the *event* rather than in a *libSQL
row* doesn't change the analysis — events are read back later (by the
streaming subscriber, by the lagged-recover snapshot path, by audit
logs in Phase 3), and the rule applies to "anywhere a value will be
read back later."

The recommendation that follows: persist *inputs* (`restart_counts`,
`last_failure_seen_at`) on the event if anything is to be persisted at
all — but the better shape is to publish the *terminal decision* the
reconciler made, not the inputs that fed the decision. Decisions are
a stable contract; inputs are bound to the policy that interprets them.

### Finding 8: CQRS read-side projections are independent components by design — but the design assumes multiple write-side aggregates feeding ONE read model, not the inverse

**Evidence**: From Fowler on CQRS:

> "The change that CQRS introduces is to split that conceptual model
> into separate models for update and display, which it refers to as
> Command and Query respectively. The rationale is that for many
> problems, particularly in more complicated domains, having the same
> conceptual model for commands and queries leads to a more complex
> model that does neither well. … **CQRS should only be used on
> specific portions of a system (a Bounded Context in DDD lingo) and
> not the system as a whole.**"

**Source**: [Martin Fowler —
CQRS](https://martinfowler.com/bliki/CQRS.html) — Accessed 2026-05-03
**Reputation**: Medium-High (martinfowler.com)
**Confidence**: High
**Verification**: [Microsoft Azure architecture — CQRS
pattern](https://learn.microsoft.com/en-us/azure/architecture/patterns/cqrs)
— High; [Greg Young — CQRS
Documents](https://cqrs.files.wordpress.com/2010/11/cqrs_documents.pdf) —
High (canonical source).

**Analysis**: Candidate (b) ("streaming as a separate component owning
its JobLifecycle-shaped view") is full CQRS. Fowler's own warning —
"only use it on specific portions of a system" — and the Microsoft
guidance both emphasise that CQRS pays off when *the read model differs
substantially from the write model* (e.g., the read side is denormalised
across many aggregates, optimised for query). For #139, the read model
(`restart_count_max` for terminal detection) is essentially the same
shape as one column of the write model (`restart_counts`). Splitting
doesn't denormalise across aggregates; it just duplicates one field.

CQRS is the wrong tool here. The right tool is the simpler shape:
publish a *processed* event (the terminal decision) rather than the
raw input.

### Finding 9: Whitepaper §18 explicitly anticipates the WASM-loaded third-party reconciler case — the trait surface for first-party Rust and third-party WASM is identical

**Evidence**: From whitepaper §18 *Extension Model*:

> "First-party reconcilers and workflows are Rust trait objects —
> maximum performance, full type safety, direct access to primitive
> internals (BPF maps, driver handles, Corrosion subscriptions) where
> appropriate.
>
> Third-party reconcilers and workflows are WASM modules loaded at
> runtime — sandboxed, hot-reloadable, language-agnostic,
> content-addressed in Garage. **The trait surface for each primitive
> is identical between the first-party Rust and third-party WASM
> implementations; the execution backend differs.** Input and output
> types are fully serializable from day one, making the WASM migration
> path trivial."

**Source**: `docs/whitepaper.md` §18 — Local SSOT
**Reputation**: High (project SSOT)
**Confidence**: High
**Verification**: ADR-0013 §1 problem statement; ADR-0013 §6
schema-author clause; whitepaper §18 *Built-in Primitives* table.

**Analysis**: The user's intuition that "WASM-third-party may be
premature for v1" is correct *as a code-generating-priority argument*
but wrong *as a design-direction argument*. The trait surface is
identical now; the question is what shape that trait surface puts on
events. If the v1 code commits to an event shape that locks future
WASM reconcilers into "you may NOT add fields to LifecycleEvent," then
v1 has paid future complexity to save present effort.

The cheapest forward-compatible shape is one where the event carries a
type-erased extension surface that any reconciler — Rust today, WASM
tomorrow — can populate. That points at candidate (c) (typed condition
strings) or candidate (a) (per-reconciler payload variants).

For (a): adding a third-party WASM reconciler would require adding a
variant to a Rust enum, which is a closed-world type. WASM cannot
extend a Rust enum; the runtime would need a "WasmExtension(serde_json::Value)"
escape hatch that defeats the type discipline candidate (a) was supposed
to buy.

For (c): the event carries `Vec<Condition>` (or one slot of a small
fixed shape). Each condition has a `type: String` and a `status:
True/False/Unknown`. WASM reconcilers populate the same type with their
own type-strings; the consumer reads the strings without compile-time
knowledge. This composes cleanly.

### Finding 10: ADR-0033 already established the precedent — `RestartBudget` is a wire-shape projection on `alloc_status`, NOT on the event channel

**Evidence**: From the existing project ADR:

> ADR-0033 *alloc-status snapshot enrichment* defines `RestartBudget {
> attempts_used: u32, ceiling: u32, exhausted: bool, next_attempt_at:
> Option<UnixInstant> }` as the operator-facing wire shape returned by
> `GET /v1/jobs/:id/alloc-status`. The handler reads the post-reconcile
> JobLifecycleView via `hydrate_job_lifecycle_view` and projects to
> `RestartBudget`.

**Source**: `docs/product/architecture/adr-0033-alloc-status-snapshot-enrichment.md`
(local) and `crates/overdrive-control-plane/src/handlers.rs:614-622`
(`hydrate_job_lifecycle_view`).
**Reputation**: High (project ADR + verified code)
**Confidence**: High
**Verification**: `crates/overdrive-control-plane/src/handlers.rs`
`alloc_status` handler; `restart_budget_from_view` function.

**Analysis**: The HTTP `alloc_status` handler's pattern IS the
correctly-layered shape: it opens a `LibsqlHandle`, reads the
JobLifecycleView, projects to a wire-shape DTO. It does this on each
request — not on every event. The cost (one libSQL read per HTTP
request) is acceptable because requests are bounded by operator query
rate, not by reconciler tick rate.

The streaming path's constraint is different — it would need a libSQL
read per `LifecycleEvent`, which is per-tick under load. That is what
made step 02-04's "stamp on event" attractive. But there are at least
two cheaper shapes:

- **Read the projection once per stream, not per event.** Streaming is
  per-job; a single `JobLifecycleView` hydration at stream start gives
  the budget contract; live events need only carry the *delta* (a
  `state == Failed` event with `attempts == ceiling` is terminal).
- **Have the reconciler decide and publish the terminal condition.**
  Once the reconciler emits a terminal condition (candidate (c)),
  `streaming.rs` does no projection at all — it forwards the condition
  to the wire.

### Finding 11: The current `exit_observer.rs:0` literal IS the design smell the user surfaced

**Evidence**: From `crates/overdrive-control-plane/src/worker/exit_observer.rs`
(post-step-02-04):

A second source of `LifecycleEvent` exists outside the reconciler tick
loop — the exit observer that watches for driver exit events and
synthesises `LifecycleEvent`s for the broadcast bus. After step 02-04,
this site passes `restart_count_max: 0` because the exit observer has no
view in scope.

**Source**: User-supplied context + verified at
`crates/overdrive-control-plane/src/worker/exit_observer.rs`
**Reputation**: High (project source)
**Confidence**: High

**Analysis**: This is a textbook layering smell. A field on a generic
event has only one valid value at the second emission site, and that
value (0) is structurally meaningless — it conservatively prevents
`BackoffExhausted` synthesis from the snapshot path (per
`streaming.rs::lagged_recover` doc comment). The compiler cannot tell
the second site that 0 is wrong; tests do not catch it because the
contract is "0 means no info, and downstream code conservatively
ignores." This is precisely the *temporal coupling* DDD warns against —
the field's meaning depends on which call site populated it.

A correctly-layered shape eliminates the second-site puzzle. Under
candidate (c), the exit observer emits a `LifecycleEvent` with no
condition; under candidate (d), the exit observer doesn't write the
budget row at all. Either way, the second emission site stops carrying
a structurally-meaningless zero.

---

## Candidate Evaluation Matrix

| Candidate | Layering | WASM-future | Cost vs current | Inputs-not-derived | exit_observer cleanup |
|---|---|---|---|---|---|
| (a) Reconciler-typed event sum-type | Clean | **Bad** — closed Rust enum can't be extended by WASM third-parties without an escape hatch | Medium — broad refactor across action_shim, broker, streaming | Same as current (still carries derived field, just typed) | Partial — still needs a "Heartbeat" variant with no payload |
| (b) Streaming owns JobLifecycle read model | Clean (full CQRS) | Same as current — irrelevant to streaming layer | High — second materialised state, libSQL connection from streaming, persistence | Better — streaming reads inputs, derives at read time | Yes — exit observer doesn't write the read model |
| **(c) Reconciler decides terminal, emits typed condition** | **Cleanest** — business decision lives with business owner | **Best** — string-typed condition vocabulary, WASM-extensible | **Low** — rename + helper move + small type addition | **Best** — derived value never crosses any boundary; only the *decision* (a stable contract) does | **Yes** — exit observer emits no condition |
| (d) Separate observation row for `RestartBudget` | Clean | Good — observation rows are already polymorphic | Medium-High — schema migration, observation-store row type, gossip overhead | Better — separate row carries inputs, derived at read time | Yes — exit observer doesn't write the row |
| Current (step 02-04): `restart_count_max: u32` on generic event | **Smelly** — derived value of one reconciler on a polymorphic type | **OK** — WASM reconcilers default to 0, harmless but useless | Zero (it's where we are) | **Worst** — derived value persisted to event payload, bound to current ceiling policy | **No** — the literal-0 site is the smell |

---

## Synthesis

### The 4-axis evaluation

**Axis 1: Where does the business decision live?**
ADR-0013 puts business logic in the reconciler. Step 02-04's projection
puts a *fragment* of the business decision (the inputs to "is the
budget exhausted?") on the event; the *decision itself* still happens in
`streaming.rs::check_terminal` via `event.restart_count_max >=
RESTART_BACKOFF_CEILING`. That's a layering inversion — the streaming
path is making business decisions about budget exhaustion. Candidate
(c) puts the decision back where it belongs.

**Axis 2: How does it compose with WASM-loaded third-party reconcilers?**
Per whitepaper §18 the trait surface is identical between Rust and WASM.
Candidate (a) explicitly contradicts this: a closed Rust enum cannot be
extended by a WASM module without an "untyped variant" escape hatch
that defeats the typing benefit (a) was meant to buy. Candidate (c)
composes cleanly because string-typed conditions are an open vocabulary
— the same property that makes the K8s `Condition` shape work across
hundreds of independent operators.

**Axis 3: How does it compose with the project's own
"persist inputs, not derived state" rule?**
The current shape (`restart_count_max` on the event) violates the rule —
the field is derived from `restart_counts.values().max()` and from the
current `RESTART_BACKOFF_CEILING` constant, both of which can change.
Candidate (c) — emitting "this is terminal because BackoffExhausted" —
also "decides" in the reconciler, but the decision IS a stable
contract: once a transition to Failed is reported as *terminal* (no
further restart attempts will be made), that's a property no future
policy change can revoke. The terminal flag is *interpretive output*,
not *cached input*. This is the same distinction Vernon makes between
Domain Events (raw facts) and Integration Events (interpretive,
stable-schema facts published outside the bounded context).

**Axis 4: What's the v1-pragmatic cost?**
- Candidate (a): ~1-2 days. Sum-type variants for every reconciler; new
  trybuild fixture asserting the variant pattern; migrate the broker
  fan-out which currently fans `LifecycleEvent` to all subscribers
  uniformly.
- Candidate (b): ~1 week. New libSQL handle in streaming.rs, new
  per-stream materialisation, new tests for divergent read-model state.
- Candidate (c): ~half a day. Add a `terminal: Option<TerminalCondition>`
  field to `LifecycleEvent`; have `JobLifecycle::reconcile` set it when
  the budget is exhausted (it already has the inputs); delete the
  `restart_count_max` field; update streaming to read
  `event.terminal.is_some()`. No schema change. No new types beyond a
  small `TerminalCondition` enum.
- Candidate (d): ~3-4 days. Schema migration, observation-store row
  type, Corrosion gossip path, sweep reconciler.

### Why "internal-only" doesn't make the smell go away

The user's nuance — "since job lifecycle is an internal reconciler then
it might not matter" — deserves direct engagement. The argument has
weight: at v1, the only reconcilers shipping are first-party Rust, and
no WASM third-party will exist for at least a year. Adding a derived
field to a Rust struct is a 5-line change.

But three things make the layering concern bite even at v1:

1. **The exit_observer.rs `0` literal is already a real smell, today, in
   internal-only code.** Every future reconciler emitting events
   outside its own tick loop (planned: a node-health reconciler that
   needs to broadcast node-state transitions) will have the same 0
   literal puzzle. Each such site is a small bug surface in production,
   regardless of whether WASM ever ships.

2. **The next reconciler that wants to project something on the event
   is the migration cost.** The first time someone needs `cpu_pressure`
   on the event for a future right-sizing reconciler, the schema is
   either widened (`LifecycleEvent { restart_count_max, cpu_pressure
   }` — both meaningless to the wrong reconciler) or the event becomes
   variant-typed (candidate (a) by stealth, paid in a hurry). Better to
   pick the variant model NOW with one field than to retrofit it under
   pressure.

3. **The Domain-vs-Integration-Event distinction is small but real.**
   The CLI consumes streaming output. Any byte that crosses
   `dispatch_single` → `bus.send` → `streaming.rs::emit_line` → CLI is
   on a wire shape that operators will eventually see (the
   `LifecycleTransition` line is rendered by the CLI today). A field
   meaningful only when the JobLifecycle reconciler emitted it is
   confusing to operator-facing tooling.

### Why candidate (c) is correct even in pure v1-pragmatic terms

Independent of the principled arguments, candidate (c) has the lowest
v1 implementation cost of any of the alternatives — half a day vs. a
week. The reason: the JobLifecycle reconciler already computes the
information needed for the terminal decision (it has `restart_counts`
and `last_failure_seen_at` in its View). Telling it to *emit* the
decision is one branch in `reconcile`. The runtime currently passes a
`u32` projection to `dispatch`; it would instead pass an
`Option<TerminalCondition>` (or, more cleanly, the action shim emits
the condition based on a flag the reconciler set on the corresponding
`Action`).

The change shrinks `streaming.rs` rather than growing it: the entire
`if matches!(event.to, AllocStateWire::Failed) { let used =
event.restart_count_max; if used >= RESTART_BACKOFF_CEILING { ... } }`
block in `check_terminal` collapses to `if let Some(cond) =
event.terminal { ... }`. The two-call-site puzzle in `lagged_recover`
goes away (the snapshot path reads `latest.terminal` directly off the
observation row, which is also a natural place to store the decision —
see "ADR-0033 cross-pollination" below).

---

## Recommended Shape

**Adopt candidate (c) — Reconciler emits a typed `Terminal` condition;
streaming forwards it. Field name: `terminal: Option<TerminalCondition>`
on `LifecycleEvent`.**

### Sketch

```rust
// In overdrive-core/src/reconciler.rs (or a new sibling module).
//
// The reconciler-decided terminal state. `BackoffExhausted` is what
// JobLifecycle emits when it observes a Failed allocation that has
// reached the ceiling. Other reconcilers add variants here as they
// land (e.g. the right-sizing reconciler may emit
// `Terminal::DriftLimit` if a workload exceeds its declared envelope
// past a threshold).
//
// Per `.claude/rules/development.md` § "Persist inputs, not derived
// state": the variants here are STABLE CONTRACT — they describe the
// reconciler's final interpretation, not the inputs it interpreted.
// Adding a new policy knob (e.g. operator-configurable per-job ceiling)
// changes which `restart_counts` value triggers `BackoffExhausted`,
// not the meaning of `BackoffExhausted` itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalCondition {
    /// JobLifecycle: restart budget reached; no further attempts.
    BackoffExhausted { attempts: u32 },
    /// JobLifecycle: explicit operator stop converged.
    Stopped { by: StoppedBy },
    // Phase 2+ adds new variants here as they are coined by the
    // reconcilers that decide them. WASM-loaded third-party reconcilers
    // (whitepaper §18) extend this surface via a string-typed escape
    // hatch (see "Forward compatibility" below) — the well-known
    // first-party set stays in the enum.
}

// On LifecycleEvent (in action_shim.rs):
pub struct LifecycleEvent {
    // ... existing fields ...
    /// `Some` when the emitting reconciler decided this transition
    /// concludes the allocation's lifecycle. `None` when this is a
    /// non-terminal transition (Pending → Running, Running → Failed
    /// with budget remaining, etc.). Replaces step-02-04's
    /// `restart_count_max: u32` field.
    pub terminal: Option<TerminalCondition>,
}
```

### Wiring

1. **`JobLifecycle::reconcile`** sets a `terminal` flag on the
   `Action::StopAllocation` / synthetic Failed-row action when it
   computes that the budget is exhausted. The change is local to
   `JobLifecycle` and uses inputs (`restart_counts`,
   `last_failure_seen_at`, the live ceiling policy) the reconciler
   already has in scope.
2. **`reconciler_runtime::run_convergence_tick`** drops the
   `restart_count_max_from_view` helper. Instead, it threads the
   reconciler-decided terminal state through to `dispatch` via a
   small `TerminalDecision` parameter on each emitted action.
3. **`action_shim::dispatch_single`** carries the decision onto the
   resulting `LifecycleEvent`'s `terminal` field. NoopHeartbeat,
   exit_observer-synthesised events, and the like emit `terminal:
   None` — this is structurally meaningful (the emitting site is
   stating "I am not making a terminal claim") rather than a
   meaningless 0.
4. **`streaming.rs::check_terminal`** simplifies dramatically:
   ```rust
   if let Some(cond) = &event.terminal {
       return Some(SubmitEvent::ConvergedFailed {
           alloc_id: Some(event.alloc_id.to_string()),
           terminal_reason: cond.into(),
           // ...
       });
   }
   ```
5. **`streaming.rs::lagged_recover`** loses its `restart_count_max_hint`
   parameter. The snapshot path reads `latest.terminal` directly off
   the observation row (see "ADR-0033 cross-pollination" below).

### Forward compatibility for WASM third-party reconcilers

Per whitepaper §18, the trait surface is identical between Rust and
WASM. The `TerminalCondition` enum is a closed Rust type — WASM
reconcilers need an extension surface. The standard pattern (Kubernetes
`Condition.type` is `string`; operator-sdk allows arbitrary CamelCase
type names) is:

```rust
pub enum TerminalCondition {
    BackoffExhausted { attempts: u32 },
    Stopped { by: StoppedBy },
    /// Custom terminal vocabulary published by a third-party reconciler.
    /// `type_name` is a CamelCase identifier scoped by the reconciler's
    /// canonical name (e.g. "vendor.io/quota.QuotaExhausted"); `detail`
    /// carries any rkyv-encoded payload the reconciler decided to
    /// publish. Streaming forwards this to operators verbatim; CLI
    /// rendering is the operator-supplied format string per §11.
    Custom { type_name: String, detail: Option<Vec<u8>> },
}
```

This is exactly the K8s `Condition` shape's flexibility — fixed schema
for the well-known cases (compile-time-checked), open string vocabulary
for the long tail (runtime-checked, but still policy-able).

### ADR-0033 cross-pollination

The `RestartBudget` wire shape on `alloc_status` (ADR-0033) and the
`terminal` field on `LifecycleEvent` should share a source of truth.
The cleanest shape: **add `terminal: Option<TerminalCondition>` to
`AllocStatusRow` itself** (the observation row), populate it when the
reconciler writes the row in `action_shim::dispatch`, and have BOTH the
HTTP handler (read from the row, project to `RestartBudget`) AND the
event (read from the same row, project to `LifecycleEvent.terminal`)
consume the same authoritative state.

This is the §4 *intent → reconciler → observation row → consumers*
shape from the whitepaper, applied uniformly: the reconciler is the
business-logic owner, the observation row is the durable artifact,
both consumers (HTTP handler, streaming event) read it.

The cost: small AllocStatusRow schema addition + Corrosion sweeper
must handle the new column (additive, follows the project's
"additive-only schema migrations" rule from whitepaper §4 *Consistency
Guardrails*).

### Migration plan from current state (post-02-04)

| Step | Change | Rollback |
|---|---|---|
| 1 | Add `TerminalCondition` enum in `overdrive-core` (no consumers yet) | Trivial — delete the type |
| 2 | Add `terminal: Option<TerminalCondition>` to `LifecycleEvent`; default `None` everywhere; keep `restart_count_max` in parallel | Trivial — same parallel-fields pattern as step 02-04 |
| 3 | `JobLifecycle::reconcile` sets `terminal` when budget exhausted; runtime threads it through `dispatch` | Per-reconciler — revert `JobLifecycle` only |
| 4 | `streaming.rs::check_terminal` reads `event.terminal` (preferred) with fallback to `event.restart_count_max` (transitional) | Both code paths active until step 5 |
| 5 | Remove `restart_count_max` from `LifecycleEvent` and `dispatch` signatures | Single revert if needed |
| 6 | (Optional, separate PR) Move `terminal` to `AllocStatusRow` per ADR-0033 cross-pollination | Schema migration, follow §4 additive rule |

Steps 1-5 are achievable in a single PR. Step 6 is a follow-up and
benefits from being separate (different ADR scope, different test
surface).

### What this recommendation buys

1. **Structural cleanup of the `exit_observer.rs:0` literal smell.**
   The exit observer emits `terminal: None`, which is *structurally
   meaningful* ("not making a terminal claim") rather than
   *structurally meaningless* (`restart_count_max: 0` interpreted as
   "no budget consumed").
2. **Forward-compat with WASM third-parties.** The `Custom` variant of
   `TerminalCondition` lets WASM reconcilers extend the terminal
   vocabulary without modifying core types.
3. **Eliminates the §18 layering inversion.** Streaming stops making
   business decisions about budget exhaustion; the reconciler owns
   the decision, the streaming layer just forwards.
4. **Aligns with K8s `Condition` shape.** Operators familiar with K8s
   reconciler patterns recognise this shape on day one — there's an
   industry-standard vocabulary for this concern.
5. **Shrinks `streaming.rs`.** ~30 lines of terminal-detection logic
   become ~5 lines. `lagged_recover`'s `restart_count_max_hint`
   parameter goes away (the snapshot path reads `latest.terminal`
   directly).

### What this recommendation costs

1. **One small enum addition.** `TerminalCondition` lives in
   `overdrive-core`; no new crate, no new feature flag.
2. **One reconciler-internal change to `JobLifecycle::reconcile`.**
   Local to the reconciler.
3. **One signature change on `dispatch` / `dispatch_single`.** Replace
   `restart_count_max: u32` with `terminal: Option<TerminalCondition>`.
4. **The transitional period** (steps 2-5 in the migration plan above)
   carries both fields on `LifecycleEvent` for one PR cycle. The
   parallel-fields pattern is the same one step 02-04 itself used to
   migrate off `view_cache`.

### What it explicitly does NOT do (deferred)

- **Does not introduce a separate observation row for `RestartBudget`
  (candidate (d)).** That schema-level work is a separate concern and
  would compose cleanly with this recommendation if and when it lands —
  candidate (c) is independent of where the row lives.
- **Does not introduce per-reconciler payload variants (candidate (a)).**
  The string-typed `Custom` variant is a deliberate concession to the
  WASM extensibility constraint; per-reconciler enum variants are the
  closed-world shape that contradicts §18.
- **Does not introduce a streaming-side materialised view (candidate
  (b)).** Streaming reads `event.terminal`; no second persistent state.

---

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| K8s API Conventions — Conditions | github.com/kubernetes/community | High | Official | 2026-05-03 | Y (operator-sdk, apimachinery) |
| controller-runtime Informers book | book.kubebuilder.io | High | Official | 2026-05-03 | Y (client-go) |
| Fowler — Event-Driven types | martinfowler.com | Medium-High | Industry | 2026-05-03 | Y (microservices.io, Eventuate) |
| Microsoft .NET Domain vs Integration Events | learn.microsoft.com | High | Official | 2026-05-03 | Y (Vernon book, microservices.io) |
| Erlang/OTP supervisor | erlang.org/doc | High | Official | 2026-05-03 | Y (LearnYouSomeErlang, OTP design principles) |
| operator-sdk Status Conditions | sdk.operatorframework.io | High | Official CNCF | 2026-05-03 | Y (Crossplane, Tekton) |
| Fowler — CQRS | martinfowler.com | Medium-High | Industry | 2026-05-03 | Y (Microsoft Azure, Greg Young) |
| Whitepaper §18 | docs/whitepaper.md | High | Project SSOT | 2026-05-03 | Local — primary |
| ADR-0013, ADR-0033 | docs/product/architecture | High | Project ADR | 2026-05-03 | Local — primary |
| development.md | .claude/rules | High | Project canon | 2026-05-03 | Local — primary |

**Reputation distribution**: High: 8 (73%) | Medium-High: 3 (27%) | Medium: 0
**Average reputation**: 0.94

---

## Knowledge Gaps

### Gap 1: Direct production precedent for "terminal condition on broadcast event" in a Rust-native orchestrator

**Issue**: Most reference precedents (K8s Conditions, operator-sdk) are
status-on-resource-object patterns, not broadcast-event payload patterns.
The closest analogues are Akka Persistence projection events and
Temporal workflow signals — both have the right shape but are
durable-event-log systems, not in-memory `tokio::broadcast` channels.
**Attempted**: searched for "tokio broadcast condition" / "rust orchestrator
terminal event"; nothing canonical surfaced. **Recommendation**: the
recommendation should be considered against the principle (event vs.
condition vs. raw projection), not against a direct Rust-native
precedent. The principle is well-supported.

### Gap 2: Quantified migration cost for the WASM third-party path

**Issue**: The recommendation includes a `Custom { type_name: String,
detail: Option<Vec<u8>> }` variant for forward-compat. The actual cost
of WASM reconcilers populating this from across the WASM ABI boundary
isn't researched in detail (whitepaper §18 says "input and output types
are fully serializable" but doesn't specify the ABI). **Recommendation**:
the variant shape is a forward-compat hedge; it can be added later if
needed. The principal recommendation does NOT depend on it.

### Gap 3: Behavior under reconciler version skew

**Issue**: If a Phase 2 reconciler refactor renames `BackoffExhausted`
to `RestartBudgetExhausted`, every CLI client that branches on the old
type-string breaks. K8s solves this with Condition `Reason` field
versioning; Overdrive doesn't yet have a story. **Recommendation**:
follow the K8s convention — `TerminalCondition` enum variants are
SemVer-stable; new variants are additive minor; renames are
breaking-major. Document in the same ADR that lands the recommendation.

---

## Conflicting Information

### Conflict 1: Should the terminal decision live on the event OR on the observation row?

**Position A (event)**: Streaming subscribes to events, so emitting the
condition on the event is the most direct path. — Source: implied by
current step 02-04 architecture; cross-ref to Akka Persistence which
puts decision data in the event.

**Position B (observation row)**: The HTTP `alloc_status` handler also
needs the terminal flag (per ADR-0033's `RestartBudget.exhausted`); a
single source of truth on the observation row serves both consumers.
— Source: ADR-0033, whitepaper §4.

**Assessment**: Both. The recommendation in this document threads the
needle: emit on the event (for streaming) AND on the row (for
HTTP-handler reads), with the row being the durable canonical state and
the event being the publish-side echo. This is the same shape that
`AllocStatusRow.reason: Option<TransitionReason>` already follows —
ADR-0032 §3 amendment writes the cause-class to the row, the action shim
echoes it onto the event for downstream subscribers. Position A and B
are not actually in conflict if the same mechanism is reused.

---

## Recommendations for Further Research

1. **Schema-version story for `TerminalCondition`.** As Phase 2+ adds
   more variants and CLI consumers branch on them, version-skew handling
   becomes important. K8s convention research is a starting point;
   protobuf reserved-tag conventions are a secondary input.

2. **Investigate moving the reconciler-decided fields onto the
   `Action` enum, not onto `LifecycleEvent`.** Today `Action::StopAllocation
   { alloc_id }` is the directive; the reconciler could emit
   `Action::StopAllocation { alloc_id, terminal: Option<TerminalCondition>
   }` — the action shim would write both the row AND emit the event with
   matching data. This is cleaner than threading a separate parameter
   through `dispatch`. Worth a separate ADR.

3. **Audit the future reconciler set** (whitepaper §18 *Built-in
   Reconcilers*: node drain, right-sizing, canary, WASM scale-to-zero,
   chaos engineering, investigation lifecycle, LLM spend, evaluation
   broker reaper, cert revocation sweep, tombstone sweep) for what each
   would project onto a `LifecycleEvent`. If the answer is "nothing"
   for most of them — they emit different event types — then
   `LifecycleEvent` is genuinely allocation-scoped and the broader
   "polymorphic event channel" concern was overstated. This research is
   reachable in a one-day audit.

---

## Full Citations

[1] Kubernetes contributors. "API Conventions — Typical Status Properties (Conditions)". *Kubernetes Community on GitHub*. (Versioned, current). https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/api-conventions.md#typical-status-properties. Accessed 2026-05-03.

[2] Operator Framework contributors. "Operator Best Practices" (see Status Conditions subsection). *operator-sdk Documentation*. https://sdk.operatorframework.io/docs/best-practices/best-practices/. Accessed 2026-05-03.

[3] Kubernetes SIG-Apimachinery. "Condition struct". *k8s.io/apimachinery package documentation*. https://pkg.go.dev/k8s.io/apimachinery/pkg/apis/meta/v1#Condition. Accessed 2026-05-03.

[4] Kubebuilder contributors. "Watching Resources — Cache and Informers". *The Kubebuilder Book*. https://book.kubebuilder.io/reference/watching-resources/informers.html. Accessed 2026-05-03.

[5] Fowler, Martin. "What do you mean by 'Event-Driven'?". *martinfowler.com*. 2017-02-07. https://martinfowler.com/articles/201701-event-driven.html. Accessed 2026-05-03.

[6] Microsoft. "Domain events: design and implementation". *.NET Microservices: Architecture for Containerized .NET Applications*. https://learn.microsoft.com/en-us/dotnet/architecture/microservices/multi-container-microservice-net-applications/domain-events-design-implementation. Accessed 2026-05-03.

[7] Vernon, Vaughn. *Implementing Domain-Driven Design*, Ch. 8: Domain Events. Addison-Wesley, 2013. ISBN 978-0321834577.

[8] Ericsson AB. "supervisor — Generic supervisor behavior". *Erlang/OTP Documentation*. https://www.erlang.org/doc/man/supervisor.html. Accessed 2026-05-03.

[9] Ericsson AB. "Supervisor Behaviour — OTP Design Principles". *Erlang/OTP Documentation*. https://www.erlang.org/doc/design_principles/sup_princ.html. Accessed 2026-05-03.

[10] Fowler, Martin. "CQRS". *martinfowler.com*. 2011-07-14. https://martinfowler.com/bliki/CQRS.html. Accessed 2026-05-03.

[11] Microsoft. "CQRS pattern". *Azure Architecture Center*. https://learn.microsoft.com/en-us/azure/architecture/patterns/cqrs. Accessed 2026-05-03.

[12] Young, Greg. "CQRS Documents". *Self-published*. November 2010. https://cqrs.files.wordpress.com/2010/11/cqrs_documents.pdf. Accessed 2026-05-03.

[13] Crossplane Authors. "Managed Resources — Conditions". *Crossplane Documentation*. https://docs.crossplane.io/latest/concepts/managed-resources/. Accessed 2026-05-03.

[14] Tekton Authors. "TaskRuns — Monitoring execution status". *Tekton Pipelines Documentation*. https://tekton.dev/docs/pipelines/taskruns/. Accessed 2026-05-03.

[15] Richardson, Chris. "Pattern: Domain event". *microservices.io*. https://microservices.io/patterns/data/domain-event.html. Accessed 2026-05-03.

[16] Overdrive contributors. "Overdrive Whitepaper §18 Reconciler and Workflow Primitives". *docs/whitepaper.md*. Local SSOT. 2026-05-03.

[17] Overdrive contributors. "ADR-0013 Reconciler Primitive Runtime". *docs/product/architecture/adr-0013-reconciler-primitive-runtime.md*. Local. 2026-05-03.

[18] Overdrive contributors. "ADR-0033 alloc_status snapshot enrichment". *docs/product/architecture/adr-0033-alloc-status-snapshot-enrichment.md*. Local. 2026-05-03.

[19] Overdrive contributors. "development.md — Reconciler I/O, State-layer hygiene, Persist inputs". *.claude/rules/development.md*. Local canon. 2026-05-03.

---

## Research Metadata

- **Duration**: ~50 turns
- **Examined**: 19 sources (10 web, 6 project docs, 3 in-tree code references)
- **Cited**: 19 sources
- **Cross-references**: 12 high-tier confirmations across 6 distinct claim categories
- **Confidence**: High 8 (89%), Medium 1 (gap 1), Low 0
- **Output**: `docs/research/control-plane/issue-139-followup-streaming-restart-budget-research.md`
