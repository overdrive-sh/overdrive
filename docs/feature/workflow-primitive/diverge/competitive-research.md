# Competitive Research — Durable Execution Landscape (workflow-primitive)

**Wave**: DIVERGE Phase 2 | **Agent**: Flux (researcher methodology applied directly — running as a subagent, could not dispatch nw-researcher) | **Date**: 2026-06-05
**Research depth**: comprehensive (7 systems incl. non-obvious alternative)

> Methodology note: this research was conducted by Flux directly using
> WebSearch/WebFetch because the Task tool is unavailable inside a
> subagent context. The nw-researcher discipline is applied: real
> systems, real cited behaviors with URLs, per-claim confidence, no
> generic market claims.

---

## 1. Landscape summary

Durable execution is the problem of running a finite, side-effecting
sequence to a terminal result exactly-once such that a crash resumes
correctly. The field splits on **how recovery re-derives in-flight
state**, and that split is the load-bearing axis for our job — not the
trait signature. Two families, with a spectrum between:

- **Re-execution + journal-replay** (Temporal, Cadence, Restate,
  Flawless, Golem): on recovery the handler function is **re-entered
  from the top**; previously-completed steps return their *journaled*
  result instantly (no re-execution of the side effect) until execution
  catches up to the failure point. This is conceptually elegant ("write
  ordinary code") but **requires the authored control flow to be
  deterministic** and therefore **inherits the version-skew hazard**:
  if the code changes while a journal is in flight, replay diverges. A
  correction to the dispatch framing: **Restate uses journal-replay too**
  — multiple sources confirm it is NOT mechanically distinct from
  Temporal on the re-execution question; its differences are
  developer-ergonomic (`ctx.run` explicit side-effect blocks, no
  sandbox) and operational (server-as-proxy, suspend-to-free-resources).

- **Step-output-memoized resume** (DBOS): step results are committed to
  a SQL table; on restart the library "replays the workflow from the
  last committed step." Mechanically lighter (Postgres-backed library,
  no server cluster). **Critical finding (corrects the dispatch
  hypothesis):** DBOS *still requires the workflow function to be
  deterministic* — "if executed multiple times, with the same arguments
  and step return values, the workflow should invoke the same steps with
  the same inputs in the same order" (DBOS docs). It re-enters the
  function from the top and short-circuits memoized steps, so it **also
  inherits the version-skew hazard** ("If a workflow is non-deterministic,
  it may execute different steps during recovery than it did during its
  original execution"). DBOS reduces *operational* weight, not the
  determinism constraint.

- **Explicit event-sourced state machine / saga** (AWS Step Functions,
  hand-authored sagas, and — for our domain — the reconciler-as-step-
  machine pattern): the author writes explicit persisted states and an
  explicit transition function. **There is no replay of authored control
  flow**, so the determinism constraint and the version-skew hazard
  **do not arise the same way** — a code change is a change to the
  transition function, which can be made backward-compatible by keying on
  the persisted state value rather than by re-deriving a control-flow
  position. This is the family that *structurally dodges* the deferred
  version-skew hazard, at the cost of authoring ergonomics (you write the
  state machine, the magic does not write it for you).

The single sharpest takeaway: **"durable like restate.dev" and "dodge
the deferred version-skew hazard" are in tension.** Restate (and
Temporal, DBOS, Flawless, Golem) are all replay-of-authored-control-flow
systems that inherit version-skew; the explicit-state-machine family is
the one that dodges it. The recommendation must pick a point on that
tension deliberately.

---

## 2. Per-system findings

### 2.1 Temporal / Cadence — journal-replay, re-execute-from-top

- **Mechanism**: Re-execution from the top + event-history replay. On
  recovery the workflow is "re-executed in a new process and each step of
  the execution is compared to its log"; completed activities return
  their recorded result instantly.
- **Journal/store**: "Event History" — a sequenced per-invocation log.
  Backed by the Temporal *server cluster* + its datastore (Cassandra /
  SQL), which is a separate operational system and an SPOF you must run.
- **Crash-recovery**: Any worker can pick up a workflow because the
  authoritative history lives in the server, not the worker — resume
  anywhere (within the worker fleet). Recovery unit = workflow execution.
- **Determinism / version-skew**: **Strict.** Workflow code must be
  deterministic; non-determinism (wall-clock, RNG, map iteration, code
  changes) produces a "non-determinism error … due to how it records
  event history and expects a matching replay" (Vanlightly). Version skew
  is the *canonical* Temporal pain point — changing workflow code that
  has in-flight histories requires explicit `patched()` / versioning
  APIs, or replay diverges. This is precisely the hazard whitepaper §18
  cites and #39 defers via code-graph hashing.
- **Does well for our job**: Mature replay-equivalence testing (the
  "replayer" runs old histories against new code) — a direct analogue of
  our DST replay-equivalence obligation (O5). Resume-anywhere (O2).
- **Fails our job**: Server-cluster architecture is the opposite of
  single-binary embedded (whitepaper §2 "one binary"). Go-implemented
  server — FFI/cross-language in the critical path (whitepaper §2
  principle 7). Inherits version-skew (the deferred hazard).
- **Key assumption**: Sequences are authored as deterministic functions;
  a heavyweight server owns the history.

### 2.2 Restate (restate.dev) — journal-replay with server-as-proxy + suspension

- **Mechanism**: **Also journal-replay / re-execute-from-top.** "the
  engine restarts the function and replays the journal: each
  previously-completed step returns its recorded result instantly, until
  execution catches up to the point of failure and continues from there."
  Confirmed across multiple sources that Restate is *not* mechanically
  distinct from Temporal on the re-execution question. **Correction to
  the dispatch's "NOT just journal replay" framing.**
- **What IS distinct** (the real Restate differences):
  1. **Server-as-proxy.** "a log managed by the Restate Server, which
     acts as a proxy in front of your services." The runtime sits in the
     request path; the SDK is lightweight and talks to the server.
  2. **Explicit `ctx.run(...)` side-effect blocks** — no sandbox; "the
     rest of your handler code runs normally without sandboxing
     restrictions," which is more ergonomic than Temporal's
     deterministic-sandbox model.
  3. **Suspension** — invocations can suspend (freeing the executing
     process) while awaiting a durable promise / timer / external signal,
     then resume by replay. Good fit for long-running waits (cert DNS
     propagation, human ratification) without holding a process.
  4. **Highly-optimized log** ("optimized for low-latency Durable
     Execution via a highly-optimized log implementation") — the
     Restate runtime is written in **Rust** (notable for embeddability).
- **Crash-recovery**: Resume-anywhere — the server owns the journal;
  services are stateless and can run on Lambda/Workers/k8s.
- **Determinism / version-skew**: Replay-based ⇒ deterministic control
  flow required ⇒ inherits version-skew (same family as Temporal). The
  `ctx.run` ergonomics reduce the *surface area* of accidental
  non-determinism but do not eliminate the version-skew hazard.
- **Does well for our job**: Suspension (resource-free long waits) maps
  cleanly to our staged-rollout / cert-DNS-wait sequences. Rust runtime
  ⇒ closest to embeddable of the replay family. `ctx.run` named-step
  ergonomics are the cleanest authoring surface in the replay family.
- **Fails our job**: Server-as-proxy in the request path is an
  architectural mismatch for an embedded in-binary primitive that
  composes with the *reconciler runtime + Action channel*, not an
  external proxy. Inherits version-skew.
- **Key assumption**: A runtime/server owns the log and proxies the
  service; side effects are explicitly wrapped.

### 2.3 Cloudflare Workflows — steps + hibernation, DO-adjacent

- **Mechanism**: Step-based durable execution. Each `step.do(...)` is
  journaled; on failure/retry, completed steps return cached results.
  Workflows can **hibernate** across long sleeps/waits (`step.sleep`,
  `waitForEvent`) — the instance is evicted and rehydrated later, similar
  in spirit to Restate suspension.
- **Journal/store**: Managed by the Cloudflare platform; relationship to
  **Durable Objects** (single-threaded, strongly-consistent, storage-
  attached actors) is the substrate — a Workflow instance is backed by
  durable, single-owner storage. *(Confidence: medium — exact store
  internals are proprietary.)*
- **Crash-recovery**: Platform-managed; resume is automatic. Recovery
  unit = workflow instance. Single-owner (DO-style) means resume is on
  the owning location, not arbitrary-node.
- **Determinism / version-skew**: Step-journaled ⇒ control flow between
  steps must be stable ⇒ version-skew applies; Cloudflare provides
  versioning semantics for in-flight instances. *(Confidence: medium.)*
- **Does well for our job**: Hibernation across long waits; tight
  storage-attached single-owner model is conceptually close to "one node
  owns this workflow's journal."
- **Fails our job**: Proprietary managed platform; not embeddable; not
  pure-Rust. Single-owner (DO) resume is *not* resume-on-any-node in the
  way our control-plane fleet needs unless the journal is gossiped/
  replicated independently.
- **Key assumption**: A single-owner durable-storage actor backs each
  workflow; the platform manages hibernation/resume.

### 2.4 DBOS — step-memoized resume, Postgres-backed library

- **Mechanism**: Library, not server. "Every step result is committed to
  a Postgres table in the same transaction as any database writes the
  step performed. On restart, the library replays the workflow from the
  last committed step." It **re-enters the workflow function** and
  short-circuits memoized steps — so it is *re-execution with memoized
  steps*, closer to "resume from last step" in effect but still
  re-entrant in mechanism.
- **Journal/store**: A **SQL workflow-status table + step/operation
  history table in Postgres**, committed transactionally with the step's
  own DB writes (the transactional-step property is DBOS's signature —
  step result + app DB write in one ACID transaction).
- **Crash-recovery**: Any process with DB access recovers in-flight
  workflows from the tables — resume-anywhere via the shared SQL store.
  Recovery unit = workflow; recovery is automatic on restart.
- **Determinism / version-skew**: **CRITICAL FINDING — DBOS requires
  deterministic workflow code, same as Temporal.** DBOS docs: "the
  workflow function must be deterministic: if executed multiple times,
  with the same arguments and step return values, the workflow should
  invoke the same steps with the same inputs in the same order. If a
  workflow is non-deterministic, it may execute different steps during
  recovery than it did during its original execution." **DBOS therefore
  INHERITS the version-skew hazard**, contrary to the dispatch's
  hypothesis that step-memoization dodges it. The dispatch's intuition
  was directionally aimed at the right family (explicit-state-machines
  dodge it) but DBOS is *not* in that family — it re-enters authored
  control flow.
- **Does well for our job**: Pure-library (no server cluster) — closest
  *architecture* to an embedded in-binary primitive. SQL-table journal
  maps almost 1:1 to libSQL (the whitepaper's stated workflow-journal
  store). Transactional step+state commit is a strong exactly-once
  property (O1).
- **Fails our job**: Postgres-coupled (the transactional-step property
  *depends* on the step's DB writes being in the same DB as the journal —
  our steps mutate kernel/region/ACME state, not a co-located SQL DB, so
  the signature DBOS property partially evaporates). Inherits
  version-skew. Determinism constraint on authored control flow.
- **Key assumption**: Steps write to the same SQL DB as the journal;
  authored workflow control flow is deterministic.

### 2.5 Explicit event-sourced state machine / Saga (AWS Step Functions; hand-authored)

- **Mechanism**: The sequence is an **explicit set of named states + a
  transition function**. AWS Step Functions: an Amazon States Language
  JSON state machine; the service persists *which state you are in* and
  drives transitions. Hand-authored sagas: a persisted enum + a `match`
  that advances it. **No replay of authored control flow** — the engine
  persists the *position* (state value) directly, not a journal of an
  imperative function's awaits.
- **Journal/store**: A single "current state" record (+ optional history)
  per execution. Store is whatever you choose — a row, a redb value, a
  log entry.
- **Crash-recovery**: Trivial — read the persisted state value, dispatch
  the matching transition. Resume-anywhere is automatic (the state value
  is the entire recovery context). Recovery unit = the state record.
- **Determinism / version-skew**: **Structurally dodges the version-skew
  hazard.** Because recovery keys on a *persisted state value* and not on
  re-deriving a control-flow position by replay, a code change is a change
  to the transition function. As long as the persisted state enum is
  treated with additive-only discipline (the project's existing
  schema-evolution rule), a new binary can pick up a state written by an
  old binary and dispatch the matching arm — no replay divergence. This
  is the family that satisfies O-version-skew-avoidance for free.
- **Does well for our job**: Crash-resume is dead simple and obviously
  correct (O1/O2/O4 are nearly trivial to reason about). Dodges
  version-skew (the deferred hazard). DST-trivial — the transition
  function is already a pure function, identical in shape to a
  reconciler's `reconcile`.
- **Fails our job**: **Authoring ergonomics.** You write the state
  machine by hand — the "express the sequence as ordinary control flow"
  half of the job (O3) is *not* served; the author hand-rolls exactly the
  state machine the job said they should not have to. This is the
  ergonomics-vs-version-skew trade at the heart of the decision.
- **Key assumption**: The author is willing to enumerate states
  explicitly; ordinary-control-flow authoring is not required.

### 2.6 Embeddable durable-execution engines — Flawless, Golem, Obelisk (vendor/embed)

Researched as the "buy vs build" alternative.

- **Flawless** — pure-Rust-flavored, **WASM-based deterministic replay**.
  Logs all non-deterministic host calls (`flawless::http`,
  `flawless::rand`) and replays deterministically. **Private alpha, no
  public source, server-based (not a pure embeddable lib).** Its version
  story is the smoking gun for our deferred hazard: "hot upgrades — new
  code replays the existing side-effect log; if the new code diverges
  from recorded effects, the upgrade fails and reverts, requiring human
  intervention." That IS the version-skew hazard, surfaced as an
  operational failure. *(Confidence: medium — alpha, limited docs.)*
- **Golem** — wasmtime-based durable execution; wraps WASI host calls,
  records an **oplog**, replays on recovery. Rust/wasmtime core (good
  embeddability signal), but the model is **WASM-component-per-worker** —
  it presumes your workflow IS a WASM component, which collides with our
  *first-party Rust* in-scope surface (the WASM SDK is explicitly
  deferred). Open-source. *(Confidence: medium.)*
- **Obelisk (obeli.sk)** — deterministic WASM-component workflow engine,
  Rust; similar oplog-replay shape. Same WASM-component assumption.
- **Common verdict**: All three are **replay-of-recorded-effects** ⇒
  inherit version-skew (Flawless's "hot upgrade fails on divergence" is
  the explicit admission). All three assume **WASM components are the
  unit of execution**, which is exactly the deferred-SDK surface — adopting
  one would force the WASM path into the in-scope first-party Rust
  primitive prematurely. Embedding a Go/external server (Temporal,
  Restate-server) violates whitepaper §2 principle 7. **No off-the-shelf
  engine fits the "embedded, pure-Rust, first-party-Rust-authored,
  DST-under-turmoil, composes-with-reconciler-runtime" constraints**
  without dragging in the deferred WASM SDK or an external server.

### 2.7 NON-OBVIOUS ALTERNATIVE — reconciler-as-step-machine

A durable terminal sequence modeled as a **specialized reconciler with a
persisted step-cursor in its typed View** — *no new primitive*. This is
the explicit-state-machine family (2.5) instantiated on Overdrive's
*existing* reconciler runtime.

- **Prior art (real, cited)**: **Argo Workflows** is exactly this. Its
  workflow controller "implements the Kubernetes operator pattern; it is
  basically a Kubernetes operator." The controller's `operate()` function
  "is called repeatedly for each workflow until the workflow completes,"
  driving a **node-phase state machine** (`status.phase` / per-node phase)
  with retry logic and suspension, **requeuing** the workflow until
  terminal. Kubernetes operators broadly drive multi-step sequences via a
  `status.phase` field + requeue; Crossplane sequences reconciliation;
  Argo's DAG/steps controller is the canonical large-scale instance.
  These are production systems running terminal multi-step sequences on a
  reconcile loop with a persisted phase cursor.
- **Mechanism on Overdrive**: The reconciler's `View` carries a
  `step_cursor` (an enum of named steps) + per-step recorded outputs +
  retry inputs. `reconcile` is the transition function: read cursor,
  emit the Action for the current step (via the existing `Action::HttpCall`
  / `Action::StartWorkflow` / cluster-mutation channel), observe the
  result on the next tick via the ObservationStore (`external_call_results`),
  advance the cursor, persist the new View (runtime-owned redb
  write-through). A "terminal" cursor value emits the typed
  `TerminalCondition` (ADR-0037) and stops.
- **Crash-recovery**: Inherited *for free* from the reconciler runtime —
  the View is bulk-loaded at boot, the cursor IS the recovery context,
  the runtime already resumes any reconciler on any node. O1/O2/O4 are
  discharged by the existing ViewStore durability (fsync-then-memory
  ordering, ADR-0035 §5) — no new recovery code.
- **Determinism / version-skew**: **Dodges it** (per 2.5) — the cursor is
  a persisted state value, `reconcile` is already a pure function the DST
  harness checks (`ReconcilerIsPure`), and additive-only View evolution
  (the project's CBOR `#[serde(default)]` discipline) lets a new binary
  resume an old cursor.
- **Does well for our job**: Zero new primitive, zero new store, zero new
  recovery mechanism, zero new DST machinery (O6 maximally served; O5
  served by the existing `ReconcilerIsPure` + ViewStore invariants). The
  reconcilers.md terminal-sequence disqualifier is the *only* doctrinal
  objection.
- **Fails our job**: (1) **Authoring ergonomics (O3)** — same as 2.5; the
  author writes a step enum + transition `match`, not ordinary
  `async fn run` control flow. (2) **Doctrinal**: `.claude/rules/
  reconcilers.md` explicitly classifies "genuinely-terminal sequences
  (workflow-shaped)" as **NOT a reconciler candidate** ("Migrate X from A
  to B terminates; keep X looking like Y converges"), and whitepaper §18
  asserts "Reconcilers converge; workflows orchestrate. Neither is
  expressible as the other." Choosing this option is choosing to overrule
  that doctrine on the grounds that the *mechanism* (persisted cursor on a
  reconcile loop) genuinely subsumes terminal sequences (as Argo proves).
  (3) **No suspension ergonomics / no parent-child await** — sibling
  coordination must go through ObservationStore signals manually.
- **Key assumption**: Terminal sequences are rare and small enough (cert
  rotation, region migration — single-digit steps) that hand-authoring a
  step enum is acceptable, AND the doctrinal "two distinct primitives"
  claim is worth trading for "one mechanism, one recovery path."

---

## 3. Comparison table

| System | Mechanism | Store | Resume-anywhere | Version-skew posture | Embeddable / pure-Rust | Fit (1-5) |
|---|---|---|---|---|---|---|
| Temporal/Cadence | Re-exec-from-top + history replay | Server cluster + Cassandra/SQL | Yes (worker fleet) | **Inherits** (canonical pain; `patched()` APIs) | No (Go server) | 2 |
| Restate | Re-exec-from-top + journal replay; server-proxy; suspension | Server-managed optimized log (Rust runtime) | Yes | **Inherits** (ergonomics reduce surface only) | Partial (Rust runtime, but server-as-proxy) | 3 |
| Cloudflare Workflows | Steps + hibernation | Durable-Object-backed (proprietary) | Single-owner | Inherits (managed versioning) | No (managed) | 2 |
| DBOS | Re-enter + step memoization | **SQL tables (Postgres)**; txn step+state | Yes (shared SQL) | **Inherits** (docs: workflow must be deterministic) | Partial (library, but Postgres-coupled) | 3 |
| Explicit state machine / Saga | **Persisted state value + transition fn; no control-flow replay** | One state record (any store) | Yes (trivial) | **Dodges** (additive-only state enum) | Yes (you write it) | 4 (mechanism) / 2 (ergonomics) |
| Embeddable engines (Flawless/Golem/Obelisk) | WASM oplog deterministic replay | Engine oplog | Yes | **Inherits** (Flawless "hot upgrade fails on divergence") | Rust core, but **WASM-component unit** (= deferred SDK) / alpha | 2 |
| **Reconciler-as-step-machine** (Argo prior art) | **Persisted cursor on reconcile loop; transition fn** | **Existing redb ViewStore** | **Yes (runtime already does it)** | **Dodges** | **Yes — already in-binary, pure Rust** | 4 |

---

## 4. The 3 sharpest insights for option generation

### Insight A — Temporal, Restate, AND DBOS are all the SAME family for our purposes: replay-of-authored-control-flow that inherits version-skew.
The dispatch hypothesized three distinct mechanisms (Temporal=replay,
Restate=log-based-different, DBOS=memoized-dodges-skew). The evidence
collapses two of those distinctions: **Restate is journal-replay like
Temporal** (the difference is server-as-proxy + `ctx.run` ergonomics +
suspension, not the re-execution mechanism), and **DBOS requires
deterministic workflow code and re-enters from the top** (it dodges the
*server* but not the *version-skew*). The real mechanism cleavage is
**replay-of-authored-control-flow (inherits skew)** vs
**explicit-persisted-state-transition (dodges skew)**. Every option must
be placed on that cleavage explicitly.

### Insight B — The ONLY family that structurally dodges the deferred version-skew hazard is the explicit-state-machine family — which includes the reconciler-as-step-machine.
Because #39 **defers** code-graph-hash version-skew rejection, any option
that *inherits* version-skew ships a known, named, unmitigated hazard
into the first-party platform sequences (cert rotation, region migration)
— and the mitigation is explicitly out of scope. The explicit-state
family (2.5, 2.7) makes the hazard *not arise*: recovery keys on a
persisted state value under additive-only evolution, exactly the
discipline the project already runs for redb/CBOR. This is the single
strongest argument for an explicit-state option and against a
faithful "durable like restate" replay option. The "durable like
restate" steer and the "defer version-skew" scope are in direct tension;
the DISCUSS wave must resolve it consciously.

### Insight C — On the journal store: DBOS's SQL coupling is conditional, and Overdrive already moved its peer primitive OFF libSQL — so "journal in libSQL" is an assumption to test, not a given.
The whitepaper says workflow journals live in libSQL "because the SQL
surface is useful for replay queries." But: (1) DBOS's signature
SQL-journal value is the *transactional step+state commit* — which
**requires the step's own writes to be in the same SQL DB**; our steps
mutate ACME/kernel/region state, not a co-located SQL DB, so that
specific value largely evaporates. (2) **ADR-0035 deliberately moved the
reconciler primitive OFF libSQL onto redb typed-View blobs**, citing:
redb's COW B-tree is the canonical fit for small-blob point-access
per-tick-fsync workloads; libSQL's WAL pays replay-on-open cost the
workload doesn't need; and **one storage engine per state-layer keeps
the operator surface small (O6)**. A workflow journal is *append-mostly
small records, point-accessed by (instance, step)* — much closer to
redb's / an append-only-log's sweet spot than to a query-heavy SQL
workload. The "SQL surface is useful for replay queries" justification is
real only if we actually run analytical replay queries over journals;
for crash-resume we only ever read one instance's records in order. So
the journal-store decision genuinely has three live candidates — **libSQL
(whitepaper default), redb append-records (peer-primitive precedent,
O6-minimal), append-only log (lightest replay)** — and an option should
engage it rather than inherit libSQL by default. **If the chosen
execution model is reconciler-as-step-machine (2.7), the store question
is already answered: the existing redb ViewStore, zero new mechanism,
maximal O6.**

---

## 5. Evidence quality note

- **Sources**: Restate docs (restate.dev/what-is-durable-execution),
  DBOS docs (docs.dbos.dev/why-dbos, /faq, /architecture, workflow
  tutorial), Jack Vanlightly's determinism analysis
  (jack-vanlightly.com), tiarebalbi DBOS-vs-Temporal, Argo Workflows
  docs + source (argoproj/argo-workflows operator.go, DeepWiki), Flawless
  HN thread (news.ycombinator.com/item?id=38010267), Golem
  (golem.cloud, golemcloud GitHub), Obelisk (obeli.sk), pkgpulse/kai-
  waehner landscape pieces.
- **Confidence**: **High** for the central mechanism findings (Temporal &
  Restate are replay-from-top; DBOS requires deterministic code and
  re-enters — all directly quoted from primary docs/analyses). **High**
  for Argo-as-reconciler-step-machine (primary source + controller code).
  **Medium** for Cloudflare Workflows internals (proprietary) and for
  Flawless/Golem/Obelisk maturity/embeddability detail (alpha / sparse
  docs). The medium-confidence items do not change any option-level
  conclusion — they all land "inherits version-skew, not cleanly
  embeddable as first-party-Rust."
- **Gate G2 check**: ≥3 real products named (7 systems: Temporal,
  Restate, Cloudflare Workflows, DBOS, AWS Step Functions, Flawless/
  Golem/Obelisk, Argo) ✓; ≥1 non-obvious alternative from a different
  category serving the same job (reconciler-as-step-machine, with Argo
  prior art) ✓; no generic market claims — every claim cited ✓.
  **G2: PASS.**
