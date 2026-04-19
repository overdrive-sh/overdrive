# Research: Native SRE Investigation Agent for Overdrive

**Date**: 2026-04-19 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 24

## Executive Summary

The prior research [[1]](./holmesgpt-integration-analysis.md) settled the dependency question — HolmesGPT is not embedded. This document answers the follow-on: **what does Overdrive build natively to match or exceed HolmesGPT's SRE capabilities, in Rust, leveraging primitives §12 already describes?**

The conclusion in one sentence: **Overdrive already has the hard parts; what's missing is (a) a declarative catalog around the toolset concept, (b) a first-class `Investigation` lifecycle object, (c) a runbook primitive, and (d) a `SimLLM` trait for DST.** Everything else — the LLM agent (`rig-rs`), the tool execution substrate (WASM reconcilers / sidecars / policies), the telemetry spine (DuckLake + ObservationStore with SPIFFE identity on every event), the approval gate, incident memory with embedding similarity, typed reconciler actions — is already in the whitepaper or is a trivial extension of it.

Six concrete proposals structured around HolmesGPT's conceptual primitives:

1. **Toolsets as WASM** — a declarative Overdrive primitive alongside reconcilers, policies, and sidecars. Same sandbox, same content-addressed storage in Garage, same load-time verification. The hard-coded §12 tool list becomes a *default toolset*; operators add more without rebuilding the binary.
2. **Runbooks as a new primitive** — markdown + frontmatter matching HolmesGPT's format so the community catalog is re-usable. Runbooks guide LLM *reasoning*; they do not replace workflows, which remain the durable-execution primitive (§18).
3. **Investigation as a first-class resource** in the data model alongside `Job`, `Node`, `Allocation`, `Policy`. Lifecycle: `triggered → gathering → reasoning → concluding → persisted`. The persisted form is the §12 incident-memory record.
4. **Correlation via SPIFFE-ID + causal window** — Overdrive's per-event identity makes this a DuckLake SQL query, not a separate subsystem. HolmesGPT correlates via alert-label matching; Overdrive correlates via cryptographic identity. Structural advantage.
5. **Remediation as typed `Action` enums** — HolmesGPT emits YAML patches; Overdrive's LLM emits typed reconciler actions that flow through Raft. This collapses "remediation" back into the reconciler model. Risk-tiered approval gate (already in §12) decides what auto-executes.
6. **`SimLLM` DST trait** — seeded, deterministic completions from stored transcripts. §21 lists six sim traits today; LLM is the missing seventh. This is the largest correctness investment available.

**Structural advantages Overdrive ships out of the box** that HolmesGPT structurally cannot have:

- Kernel-native, zero-instrumentation telemetry with SPIFFE identity on every flow and resource event (§12) [[18]](../whitepaper.md). HolmesGPT scrapes Prometheus and kubectl; Overdrive has the source.
- SQL + time travel on the full telemetry history via DuckLake (§17) [[18]](../whitepaper.md). HolmesGPT correlates via text matching; Overdrive correlates with snapshot reads.
- Owned dataplane — the LLM agent can install temporary BPF counters to verify hypotheses (§7) [[18]](../whitepaper.md). No other orchestrator's SRE agent can do this.
- DST foundation — §21 already takes determinism seriously; adding `SimLLM` is a genuine first. No existing SRE agent has deterministic simulation of its reasoning tier.

**Roadmap alignment**: Proposals (1), (3), (5) land in Phase 5 as natural extensions of the §12 LLM agent already on that roadmap. Proposals (2), (4) land in Phase 6. Proposal (6) — `SimLLM` — is the cheapest and should land in Phase 2 alongside the other DST trait stabilization, before the LLM agent ships, so the agent is born simulation-testable.

The whitepaper edits are additive: a new §12.X *Native SRE Investigation Agent* subsection draftable below, a one-line DST-trait addition to §21, and Roadmap bullets.

---

## Research Methodology

**Search strategy**. The HolmesGPT factual base is already established in the prior research document [[1]](./holmesgpt-integration-analysis.md) — 18 sources cited there, every major claim cross-referenced. This pass re-aimed that material at the native-build question and added targeted verification on (a) rig-rs's provider surface in 2026, (b) the official `rmcp` Rust SDK status, (c) HolmesGPT toolset execution semantics for the native re-implementation discussion.

**Source selection**. Primary: Overdrive whitepaper v0.12 (project SSOT); prior research on HolmesGPT. Secondary: rig-rs docs ([docs.rs/rig-core](https://docs.rs/rig-core/latest/rig/), [rig.rs](https://www.rig.rs/), [GitHub 0xPlaygrounds/rig](https://github.com/0xPlaygrounds/rig)); MCP official Rust SDK ([modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk)); HolmesGPT toolset YAML format ([custom_toolset.yaml example](https://github.com/robusta-dev/holmesgpt/blob/master/examples/custom_toolset.yaml)).

**Quality standards**. Every HolmesGPT claim carries a URL from the prior research (which itself cross-referenced official docs + source code). Every Overdrive claim carries a whitepaper section reference. Every technology claim (rig-rs, rmcp) is verified from ≥1 official source.

**Adaptive depth note**. The hard HolmesGPT facts were already gathered in [[1]](./holmesgpt-integration-analysis.md); this pass spent budget on the design analysis rather than re-gathering the factual base. Where prior research answered a question, the citation points there rather than re-citing the underlying URL.

---

## 1. Toolsets as a First-Class Concept

### 1.1 What HolmesGPT actually has

HolmesGPT's toolset is a declarative YAML document. A toolset names a set of tools; each tool has a name, description, shell command template, and parameter schema. The LLM sees the descriptions and parameter schemas; it selects which tool to call and fills parameters via Jinja-style `{{ variable }}` templating; the runtime executes the resulting shell command. Jinja `${VAR}` interpolation from env vars is invisible to the LLM and holds credentials/endpoints. ([HolmesGPT — Custom Toolsets](https://holmesgpt.dev/latest/data-sources/custom-toolsets/); [[1]](./holmesgpt-integration-analysis.md) §1.5)

The execution surface is arbitrary shell. Mitigations are read-only RBAC, per-tool memory caps, output streaming to disk. The ecosystem value is not the YAML format — it is the 40+ curated toolsets spanning Prometheus, Grafana, Loki, Datadog, PagerDuty, cloud provider APIs, databases, K8s operators, and so on. ([HolmesGPT — Built-in Toolsets](https://holmesgpt.dev/latest/data-sources/builtin-toolsets/); [[1]](./holmesgpt-integration-analysis.md) §1.6)

### 1.2 What §12 has today

The §12 LLM agent tool list is:

```
query_flows(sql)        get_job_status(job_id)       get_policy_decisions()
get_node_metrics()      get_incident_history()       propose_action(action)
```

This is a Rust trait object dispatch — hard-coded at compile time, not extensible without a binary rebuild. It is sufficient for what §12 specifies, but it cannot absorb community-contributed toolsets or operator-specific diagnostics.

### 1.3 Design gap and the natural Overdrive answer

Overdrive already has three WASM extension surfaces, each with identical execution primitives:

| Surface | Purpose | Reference |
|---|---|---|
| WASM reconciler | Convergence loop, writes through Raft | §18 |
| WASM policy | Verdict, writes to `policy_verdicts` in ObservationStore | §13 |
| WASM sidecar | Request-path transformation, runs in-process | §9 |

The natural extension is a **fourth** surface: WASM investigation toolsets. Same WASM sandbox, same content-addressing in Garage, same sha256-pinned load, same SVID-scoped host function boundary. A toolset declares a set of tools via a Rust trait; the LLM consumes their descriptions through the agent's tool surface; invocations run in the sandbox.

```rust
trait Toolset: Send + Sync {
    /// Unique toolset name, scoped globally.
    fn name(&self) -> &str;

    /// Tool definitions visible to the LLM.
    fn tools(&self) -> &[ToolDef];

    /// Execute a tool call. Context provides read-only access to the
    /// subset of host functions this toolset is authorised to use.
    async fn invoke(
        &self,
        tool: &str,
        args: serde_json::Value,
        ctx: &ToolsetContext,
    ) -> Result<ToolResult>;
}

struct ToolDef {
    name:         String,
    description:  String,
    input_schema: serde_json::Value,   // JSON Schema
    output_schema: serde_json::Value,
    /// Cost/risk class used by the agent's tool-selection policy.
    class: ToolClass,
}

enum ToolClass {
    /// Cheap, read-only, unlimited calls per investigation.
    Read,
    /// Expensive read (e.g. large DuckLake scan). Budgeted.
    ExpensiveRead,
    /// Writes via `propose_action` — never auto-executes.
    Propose,
}

struct ToolsetContext<'a> {
    investigation_id: InvestigationId,
    caller_identity:  SpiffeId,            // SVID of the investigation
    ducklake:         DuckLakeReadOnly<'a>,
    observation:      ObservationReadOnly<'a>,
    intent:           IntentReadOnly<'a>,  // read-only projection
    incident_memory:  IncidentStoreReadOnly<'a>,
    propose:          ProposalSink<'a>,    // posts into the approval gate
    budget:           &'a mut TokenBudget,
}
```

Three consequences:

1. **The hard-coded §12 tool list becomes the `builtin:overdrive-core` toolset** — a Rust trait object, zero WASM overhead, loaded by default. This matches the Rust-native + WASM-extension pattern §9 already uses for sidecars (`builtin:credential-proxy` vs user WASM).
2. **Operators add toolsets without rebuilding the binary.** A DuckLake-heavy toolset for SQL-level anomaly detection; a PagerDuty ingestion toolset; a Git-log toolset that correlates deploy commits against flow-event timestamps. The binary doesn't grow; the Image Factory doesn't gain `kubectl`.
3. **The LLM-visible tool surface is versioned and content-addressed.** A toolset loaded as `sha256:abc123` produces bit-for-bit the same tool surface on every node. Investigation transcripts (§3 below) can cite toolset hashes, making past investigations reproducible.

### 1.4 Why not port HolmesGPT's YAML format directly

The prior research evaluated this in Shape C and rejected wholesale adoption [[1]](./holmesgpt-integration-analysis.md) §3 Shape C. The reason is that HolmesGPT tools shell out to binaries (`kubectl`, `curl`, `aws`, `psql`, ...) — the Image Factory discipline (§24) explicitly excludes these from the node OS. A Rust toolset crate that calls DuckLake SQL directly is both faster and safer than a WASM sandbox executing shell; conversely, a user toolset that needs arbitrary tooling should run as a sidecar on an ordinary persistent-microVM workload, not in the investigation sandbox.

**The cherry-pick is the format, not the execution model.** A toolset manifest file declaring tool names, descriptions, JSON Schemas for input/output, and a reference to the backing Rust trait object (or WASM module hash) is a small loader; every tool that HolmesGPT's YAML expresses as a shell command, Overdrive expresses as a DuckLake SQL, an ObservationStore subscription, or a WASM module invocation. The LLM-visible shape is identical.

### 1.5 What the default `builtin:overdrive-core` toolset ships with

A concrete proposal for the tools that should exist on day one, lifting what HolmesGPT's Kubernetes toolset provides and mapping each to a Overdrive-native backend:

| HolmesGPT-equivalent tool | Overdrive-native backend |
|---|---|
| `kubectl get pods` / `describe pod` | `overdrive.query_allocations(sql)` against ObservationStore `alloc_status` (§4) |
| `kubectl get nodes` | `overdrive.query_nodes(sql)` against `node_health` |
| `kubectl top node/pod` | `overdrive.query_resources(sql)` against DuckLake resource events |
| `kubectl logs` | `overdrive.tail_events(alloc_id, duration)` — eBPF ringbuf + structured logs |
| `kubectl describe policy` (NetworkPolicy) | `overdrive.get_policy(id)` + `overdrive.query_policy_verdicts(sql)` |
| Prometheus `query_range` | `overdrive.query_flows(sql)` + `overdrive.query_resources(sql)` (DuckLake time travel) |
| `kubectl get events` | DuckLake SQL against telemetry events, scoped by SPIFFE ID and time window |
| Kubernetes Remediation MCP | `overdrive.propose_action(action)` — typed `Action` enum (§5) |
| Past-incident search | `overdrive.search_incidents(text_or_embedding)` — libSQL with embeddings (§12) |
| Install a diagnostic counter | `overdrive.attach_probe(bpf_sha, alloc_id, duration)` — **unique to Overdrive** (§7 below) |

Every single one of these is SQL-shaped or typed-action-shaped. None requires `kubectl` to be in the image.

### 1.6 Finding

**Overdrive should extend the §12 tool surface to a declarative toolset catalog backed by Rust trait objects (built-in) and WASM modules (third-party), loaded via content-addressed hashes from Garage.** The `rig-rs` agent consumes the union of loaded toolsets; investigations cite the toolset hashes used so transcripts reproduce. This is the structural Overdrive answer to HolmesGPT's toolset ecosystem, and it composes natively with the WASM extension model §9, §13, §18 already specifies. **Confidence: High.**

---

## 2. Runbooks as Structured Investigation Plans

### 2.1 What HolmesGPT actually has

HolmesGPT runbooks are markdown files with YAML frontmatter describing when a runbook applies (alert name regex, labels, namespaces) and a prose body instructing the LLM on investigation procedure. A `catalog.json` index pairs runbook IDs with descriptions for LLM-driven match; on match, the agent calls a `fetch_runbook` tool to retrieve the markdown. The LLM interprets the prose — it is *guidance*, not a program. ([HolmesGPT — Runbooks](https://holmesgpt.dev/latest/reference/runbooks/); [[1]](./holmesgpt-integration-analysis.md) §1.7)

### 2.2 Workflow vs runbook — the distinction that matters

§18 already ships a **workflow reconciler** — a first-class durable-execution primitive for multi-step operations with crash-safe resume, checkpointed at every `await` point in libSQL. Workflows are deterministic: every step has a defined action and a defined expectation. "Roll certificate through DNS propagation → wait validation → swap trust anchor → verify acceptance → retire old cert" is a workflow.

A runbook is not a workflow. A runbook is an *LLM-interpreted investigation guide* — "when PodCrashLooping fires, check CPU throttle first, then OOM events, then recent deploys, then correlate against policy denials in the last hour." The steps are ambiguous by design — the LLM decides which apply, in what order, and when to stop. The *output* of a runbook is a diagnosis, not a state change. If the LLM proposes a state change, it goes through `propose_action` and — once ratified — may instantiate a workflow for durable execution.

Concretely:

| Primitive | What it does | Runtime | Output |
|---|---|---|---|
| **Workflow** (§18) | Execute defined sequence with crash-safe resume | Deterministic | State transitions |
| **Runbook** (new) | Guide LLM investigation of a flagged incident | LLM-interpreted | Diagnosis + optional `propose_action` |

They compose: a runbook may conclude "roll back deployment foo to v2.11.3"; the LLM emits `propose_action(RollBackDeployment{...})`; the approval gate ratifies; a workflow reconciler (`DeploymentRollback`) executes.

### 2.3 Format — adopt HolmesGPT's markdown + frontmatter

The HolmesGPT runbook format is adoptable verbatim. Markdown is ubiquitous; the frontmatter is YAML. The benefit of matching HolmesGPT is that the community-maintained runbook corpus is directly re-usable — Overdrive doesn't re-author every PodCrashLooping runbook the community has already written; it translates the Kubernetes references in the runbook body to Overdrive allocations as the LLM applies it.

A Overdrive runbook adds optional Overdrive-specific frontmatter for scoping on SPIFFE IDs and policy verdicts:

```yaml
---
id: overdrive/alloc-oom-repeated
description: Repeated OOM kills for the same allocation
triggers:
  - event: alloc_state_change
    filter:
      to_state: terminated
      reason: oom
      window_seconds: 3600
      min_count: 3
scope:
  job_identities: ["spiffe://overdrive.local/job/*"]
required_toolsets:
  - "builtin:overdrive-core"
  - "sha256:abc123"   # an optional community toolset
---

When repeated OOM kills occur for the same job, investigate in this order:

1. Has the resource profile changed recently? Check
   `query_resources` for p95 memory over the past 7 days vs the memory limit.
2. Was a deploy rolled out in the last 24 hours that changed the limit or
   the workload binary? Check `query_intent_history` for this job_id.
3. ...
```

### 2.4 Storage and retrieval

Runbooks are:

- **Stored** in Garage, content-addressed by sha256 of the markdown. Immutable; cite-able by hash in investigation transcripts.
- **Indexed** in the incident-memory libSQL (§12) alongside past incidents. The same embedding-similarity index that serves incident retrieval serves runbook match. `INSERT INTO runbooks_embeddings (runbook_sha, description, embedding) VALUES (...)`.
- **Retrieved** via a `overdrive.get_runbook(sha_or_id)` tool exposed from the `builtin:overdrive-core` toolset.
- **Matched** by the agent at investigation-start: for a flagged incident, the agent's first tool call is `overdrive.match_runbook(incident_description)` returning top-k runbooks by embedding similarity against their frontmatter `description`.

### 2.5 The runbook reconciler

Because runbooks are evaluated *when* an investigation is triggered, they need no continuous reconciliation loop — they are static documents. However, the set of runbooks loaded and indexed on a cluster is itself intent, so a `RunbookLoader` reconciler is natural: watches `IntentStore.runbook_attachments` (operator declares which runbooks apply to which scopes); on change, (re)loads the markdown from Garage, regenerates embeddings, updates the index in libSQL. This is strictly analogous to how `policy_verdicts` flow from compiled policy in the IntentStore through the reconciler into the ObservationStore.

### 2.6 Finding

**Runbooks belong as a new primitive alongside workflows**, with format borrowed from HolmesGPT (markdown + frontmatter + `catalog.json`-equivalent index) so the community ecosystem is leverageable. Storage in Garage (content-addressed), indexed via the existing incident-memory libSQL embedding system. A `RunbookLoader` reconciler handles attachment/indexing; runbook retrieval during investigation is a tool call in `builtin:overdrive-core`. **Confidence: High.**

---

## 3. Investigation-as-a-Lifecycle

### 3.1 Why make `Investigation` a first-class resource

§4 lists the core data model: `Job`, `Node`, `Allocation`, `Policy`, `Certificate`. Self-healing is described in §12 as tiers and reasoning steps, but there is no resource representing *"an in-flight reasoning session on a specific anomaly"*. This gap shows up in several places:

- Observability. An operator cannot `overdrive investigation list` or `overdrive investigation describe inv-a1b2c3`.
- Approval gate. `propose_action` needs to carry a reference to *which* reasoning session produced the proposal — otherwise the audit trail is lossy.
- DST. §21 can simulate the dataplane and the control plane, but an LLM reasoning pass is not a first-class object it can name and replay.
- Cost metering (§6 below). Per-investigation token budgets need an investigation identifier to attribute spend.
- Correlation (§4 below). Multiple correlated alerts should group under one investigation; without the object, the grouping has no handle.

### 3.2 Data model

```rust
struct Investigation {
    id:              InvestigationId,      // ULID, time-ordered
    trigger:         Trigger,              // why did this start?
    related_allocs:  Vec<SpiffeId>,        // identities under investigation
    correlation_key: CorrelationKey,       // see §4
    runbook:         Option<RunbookRef>,   // matched runbook, if any
    toolsets:        Vec<ToolsetRef>,      // sha-pinned
    state:           InvestigationState,
    budget:          Budget,               // token + wallclock
    trace:           InvestigationTrace,   // tool calls, LLM turns, findings
    diagnosis:       Option<Diagnosis>,    // filled at conclusion
    proposals:       Vec<ProposalRef>,     // actions proposed to the gate
    started_at:      Timestamp,
    concluded_at:    Option<Timestamp>,
}

enum InvestigationState {
    Triggered,
    Gathering,     // in a tool-call / LLM-turn loop
    Reasoning,     // final-synthesis LLM turn
    Concluding,    // writing diagnosis + persistence
    Concluded,
    Cancelled { reason: String },
    Failed { reason: String },          // budget exceeded, tool hard-error
}

enum Trigger {
    AlertFired { source: AlertSource, alert_id: String },
    ReconcilerEscalation { reconciler: String, alloc: SpiffeId },
    OperatorRequested { operator: String, query: String },
    Scheduled { schedule_id: ScheduledInvestigationId },
}
```

### 3.3 Lifecycle and storage boundary

An `Investigation` transits three consistency boundaries cleanly:

| Stage | State layer | Store |
|---|---|---|
| Triggered → Gathering → Reasoning | Transient / live | ObservationStore (`investigation_state` Corrosion table) |
| Concluding → Concluded | Authoritative persistence | IntentStore? **No** — this is observation data, not intent. Persisted to incident-memory libSQL (§12) |
| Proposed actions | Authoritative | IntentStore (Raft), like any other action proposal |

This matches §17's three-layer state taxonomy: live runtime is observation; accumulated memory is private libSQL; authoritative commitments are intent.

### 3.4 The Investigation reconciler

```rust
impl Reconciler for InvestigationReconciler {
    fn reconcile(
        &self,
        desired: &State,    // investigations that should be running
        actual: &State,     // investigations in ObservationStore
        db: &Db,            // private libSQL — transcripts, prompts, token counts
    ) -> Vec<Action> {
        // For each Triggered investigation: run the agent for one turn,
        // emit Action::UpdateInvestigationState on each step.
        //
        // For each Concluded investigation: emit Action::PersistIncident
        // and Action::SubmitProposals for any proposed actions.
    }
}
```

The investigation itself is *not* a workflow — workflows are deterministic with defined sequences. An investigation is inherently nondeterministic (the LLM chooses what to do next). But it uses the reconciler model uniformly: level-triggered evaluation of desired-vs-actual state, typed actions through Raft.

### 3.5 Incident as the persisted compressed form

The user's proposal in the prompt ("Investigation is the runtime object; Incident is the persisted compressed record") is exactly right. The table shape:

```sql
-- ObservationStore — Corrosion, live only
CREATE TABLE investigation_state (
    investigation_id ULID PRIMARY KEY,
    state            TEXT,
    trigger          BLOB,
    related_allocs   BLOB,   -- JSON array of SPIFFE IDs
    correlation_key  TEXT,
    started_at       INTEGER,
    updated_at       INTEGER
);
SELECT crsql_as_crr('investigation_state');

-- libSQL incident memory — per-control-plane-node, optional Turso sync
CREATE TABLE incidents (
    incident_id      ULID PRIMARY KEY,
    investigation_id ULID,              -- back-link to trace
    correlation_key  TEXT,
    first_seen       INTEGER,
    last_seen        INTEGER,
    affected_allocs  BLOB,
    diagnosis        TEXT,
    diagnosis_embed  BLOB,              -- embedding for similarity search
    proposed_actions BLOB,
    outcome          TEXT,              -- 'auto-applied' | 'operator-applied' | 'rejected' | 'unresolved'
    resolution_notes TEXT,
    toolsets_used    BLOB,              -- sha list
    runbook_sha      TEXT,
    model_id         TEXT,
    tokens_in        INTEGER,
    tokens_out       INTEGER,
    tool_call_count  INTEGER,
    wallclock_ms     INTEGER
);
CREATE INDEX idx_incidents_embedding
    ON incidents USING VECTOR (diagnosis_embed);
```

An investigation is *compressed* into an incident on conclusion: the full tool-call trace lives in the libSQL DB; the incident row carries the fields used for retrieval (embedding, correlation key, affected allocations, outcome).

### 3.6 DST — the SimInvestigation path

With `Investigation` as a typed resource and transcripts persisted, the path from §21 DST to deterministic investigation replay is straightforward:

- `SimLLM` (see §6 below) replays stored transcripts step-for-step.
- A recorded investigation can be re-run with a new `SimLLM` seed; the framework asserts that the final diagnosis is unchanged (or fails with a diff).
- The trace of tool calls is the DST boundary — the agent's prompt, the tools called, the tools' outputs are all captured. Re-running deterministically reproduces the final diagnosis.

This makes investigation-agent correctness regressions catchable in CI. Today, changes to the §12 agent prompt or tool list are untestable except via live runs against a real LLM.

### 3.7 Finding

**Add `Investigation` as a core resource alongside `Job`, `Node`, `Allocation`, `Policy`, `Certificate`.** Live state in the ObservationStore; compressed persisted form in the incident-memory libSQL. Lifecycle managed by an `InvestigationReconciler`. Transcripts are DST-replayable via `SimLLM`. This is the missing state anchor for every other proposal in this document. **Confidence: High.**

---

## 4. Alert Correlation and De-duplication

### 4.1 What HolmesGPT / Robusta have

Robusta (HolmesGPT's sibling project) handles correlation at the alert layer — grouping correlated AlertManager alerts by label similarity into one investigation. The correlation substrate is text/label matching on what AlertManager emits. ([[1]](./holmesgpt-integration-analysis.md) §1.6 — the built-in toolset catalog includes AlertManager ingestion.)

### 4.2 Why Overdrive's substrate is better

Every eBPF event Overdrive emits carries the full SPIFFE ID of the source and destination workloads [[18]](../whitepaper.md §12). Every resource event carries the alloc ID and job name. Every policy verdict carries the rule matched. The correlation substrate is *already cryptographic identity*, not a label set an operator hopes to be consistent.

Mechanically: correlation is a DuckLake SQL query over events joined on `src_identity` / `dst_identity` / `alloc_id`, windowed by causal-time proximity.

```sql
-- Find all allocations whose flow-event error rate spiked at the same
-- time as `payments/alloc/a1b2c3`, within a 5-minute window:
WITH anchor AS (
  SELECT timestamp, src_identity
  FROM flows
  WHERE src_identity = 'spiffe://overdrive.local/job/payments/alloc/a1b2c3'
    AND verdict = 'deny'
    AND timestamp BETWEEN :t0 AND :t1
)
SELECT f.src_identity, count(*) AS denies
FROM flows f
JOIN anchor a ON
  f.timestamp BETWEEN a.timestamp - INTERVAL '2.5 min'
                  AND a.timestamp + INTERVAL '2.5 min'
WHERE f.verdict = 'deny'
GROUP BY f.src_identity
HAVING denies > 3
ORDER BY denies DESC;
```

This kind of correlation is a routine DuckLake query. No special subsystem required.

### 4.3 Data model: `correlation_key`

A `correlation_key` column on each Investigation and each Incident ties together events that should be treated as one phenomenon. The key is a tuple derived from:

```
correlation_key = hash(
    primary_identity,          // the alloc most central to the event
    rule_or_signal_class,      // 'policy-deny', 'oom', 'cert-expiry', ...
    time_bucket                // quantized to 5-minute bucket by default
)
```

When an event arrives that would trigger an investigation, the reconciler computes the correlation key and checks the ObservationStore:

- Live investigation with matching key → append event to existing investigation (bump `last_seen`, add to `related_allocs` if new).
- No matching investigation → new investigation.

This collapses N correlated alerts into one investigation cleanly, without label-based hand-wiring.

### 4.4 Why not a separate `incidents` table in ObservationStore

The data model section of the whitepaper (§4) has `alloc_status`, `service_backends`, `node_health`, `policy_verdicts` as Corrosion tables. The natural extension would be adding `investigations` and `incidents`. But:

- **Live investigation state belongs in ObservationStore** — changes frequently, read by schedulers/gateways/operators, eventually-consistent is fine.
- **Persisted incident memory belongs in private libSQL** — the embedding search is a per-control-plane-node read; cross-node sync via optional Turso (§17) is sufficient.

So the split is: `investigation_state` in Corrosion (live); `incidents` in libSQL (persisted). This matches the three-layer taxonomy in §17 [[18]](../whitepaper.md).

### 4.5 Finding

**Correlation is a `correlation_key` column + a DuckLake SQL pattern, not a separate subsystem.** SPIFFE identity on every event makes this mechanically straightforward. Live investigation state in ObservationStore; persisted incident memory in libSQL. De-duplication becomes "did a live investigation with this correlation_key already exist? if so, append; else, start new." **Confidence: High.**

---

## 5. Remediation Actions

### 5.1 What HolmesGPT has

HolmesGPT's remediation surface is the "Kubernetes Remediation" MCP server — one of ~10 addons, opt-in, ships as a separate Helm sub-chart. It proposes concrete actions as YAML patches: restart pod, scale replicas, roll back deployment, cordon node, patch annotation. Human ratification is "operator reviews the YAML patch and applies it." ([[1]](./holmesgpt-integration-analysis.md) §1.6, §1.10)

### 5.2 Overdrive already has this, and better

The §18 reconciler model ships with a set of built-in reconcilers:

> Job lifecycle (start, stop, migrate, restart) · Certificate rotation · Resource right-sizing · Rolling deployment strategies · Canary promotion/rollback · Node drain and replacement · WASM function scaling · Chaos engineering · Workflow execution

Every common SRE remediation is already a reconciler action. The LLM agent does not need a "Overdrive Remediation MCP server" — it emits typed `Action` enums that the existing reconcilers consume.

The action catalog — drafted as a typed enum the agent produces:

```rust
enum Action {
    // Job lifecycle — backed by Job lifecycle reconciler
    RestartAllocation { alloc: SpiffeId, reason: String },
    StopAllocation    { alloc: SpiffeId, reason: String },
    ScaleJob          { job: JobId, replicas: u32, reason: String },
    RollBackDeployment{ job: JobId, to_revision: RevisionId, reason: String },

    // Node operations — backed by Node drain/replacement reconciler
    CordonNode        { node: NodeId, reason: String },
    DrainNode         { node: NodeId, grace: Duration, reason: String },

    // Resource — backed by Right-sizing reconciler
    ResizeAllocation  { alloc: SpiffeId, resources: Resources, reason: String },

    // Policy — goes via IntentStore write through policy compilation
    ProposePolicyEdit { policy: PolicyId, patch: PolicyPatch, reason: String },

    // Investigation-tier writes — expand the investigation, not the cluster
    AttachDiagnosticProbe { alloc: SpiffeId, bpf_sha: Sha256, duration: Duration },
    // Permission-bounded: only pre-signed, auditable probes from a curated catalog.

    // Workflow trigger — for multi-step operations
    StartWorkflow { workflow: WorkflowId, params: serde_json::Value, reason: String },
}
```

Every variant carries a `reason: String` — this is the LLM's audit trail. The Investigation object (§3) ties the reason back to the tool calls that motivated it.

### 5.3 Risk classification — the graduated approval gate

§12 describes a graduated approval gate. The risk model the user proposed in the prompt is correct and lifts cleanly to the Action taxonomy:

| Tier | Criteria | Examples | Default behavior |
|---|---|---|---|
| **Tier 0: Reversible Reads** | No state changes. Pure reads. | Running a DuckLake query, tailing events, attaching a time-bounded diagnostic probe | **Auto-execute**. No gate. Budget-bounded. |
| **Tier 1: Low-Blast-Radius Writes** | Reversible; affects one workload; stateless service | `RestartAllocation`, `ResizeAllocation` (up to 2× profile), `ScaleJob` up to 120% of current | **Auto-execute with notification**. Written to audit log; operator can rollback. |
| **Tier 2: High-Blast-Radius Writes** | Affects multiple workloads, stateful systems, or cluster membership | `DrainNode`, `RollBackDeployment`, `ProposePolicyEdit`, `ScaleJob` beyond 120%, stateful restarts | **Human approval required**. Proposal queued; operator ratifies via CLI / API / UI. |

Tier classification is encoded on each `Action` variant at the type level (a `const fn risk_class()` on the enum), not configured at runtime. Operators can *tighten* tiers (demote Tier 1 to Tier 2 for a namespace); they cannot loosen them beyond the compiled-in default.

### 5.4 Why typed actions are better than YAML patches

HolmesGPT emits YAML because Kubernetes is the substrate and kubectl-apply is the interface. Overdrive is not Kubernetes; its substrate is typed Rust enums flowing through Raft.

Concrete benefits:

- **Compile-time exhaustiveness.** Adding a new reconciler action type propagates through the approval gate, the audit log, and the simulation harness. A new kubectl resource kind propagates nowhere.
- **Type-level risk classification.** The risk tier is part of the Action type. YAML has no equivalent.
- **Deterministic replay.** A stored Action is a stored enum variant; re-applying it in simulation is a deterministic call. A stored YAML patch is a string whose effect depends on kubectl version, CRD schema evolution, webhook behavior.
- **SPIFFE-bound.** The Action carries SVIDs, not name strings. A `RestartAllocation` cannot accidentally match a different alloc because its SVID has moved to another workload — the SVID is the identity.

### 5.5 The `propose_action` → approval gate → reconciler flow

End to end:

```
LLM agent decides to propose action
    │
    ▼
Agent calls `propose_action(Action, investigation_id, reasoning)`
    │
    ▼
Proposal is written to IntentStore.proposals (Raft)
    │
    ├── Tier 0 → auto-executed inline; proposal shows as Ratified + Applied
    ├── Tier 1 → auto-ratified by policy; operator notified async; applied
    └── Tier 2 → queued Awaiting; operator ratifies manually
    │
    ▼ (once ratified)
Proposal converted to the target reconciler's input
Job-lifecycle / Right-sizing / Node-drain / Workflow reconciler consumes it
    │
    ▼
Reconciler converges; outcome (applied / failed) written back into the
investigation's trace and into the incident memory
```

The proposal → ratification → application cycle is itself a *workflow* under §18. A Tier 2 proposal that the operator ratifies 12 hours later resumes cleanly from the workflow's journal.

### 5.6 Finding

**Overdrive already has HolmesGPT's remediation capability structurally; the missing piece is the typed `Action` enum that the LLM agent emits.** This collapses remediation back into the reconciler model — no new subsystem. Risk-tiered auto-execution is encoded at the type level, not configured at runtime. The approval gate (§12) ratifies Tier 2; Tier 1 is audited but auto-applied; Tier 0 (reads) is unrestricted. **Confidence: High.**

---

## 6. The LLM Layer

### 6.1 rig-rs provider surface — strong enough

Verified directly from `docs.rs/rig-core` (current release, accessed 2026-04-19): rig-rs natively supports 11+ providers — **Anthropic, Azure, Cohere, DeepSeek, Gemini, Groq, Mistral, Ollama, OpenAI, Perplexity, xAI**. Bedrock is available as a separate crate (`rig-bedrock`); Vertex AI as `rig-vertexai`. rig-rs advertises "20+ providers" total via the ecosystem of community crates. ([rig-rs — rig-core docs](https://docs.rs/rig-core/latest/rig/); [rig.rs](https://www.rig.rs/); [GitHub 0xPlaygrounds/rig](https://github.com/0xPlaygrounds/rig))

Comparison to HolmesGPT's LiteLLM-backed surface of 11 documented providers ([[1]](./holmesgpt-integration-analysis.md) §1.4): **parity**. The LiteLLM advantage is breadth of edge-case providers; in practice the 90% of deployments will use OpenAI, Anthropic, Gemini, Bedrock, or Ollama, all of which rig-rs covers natively.

**Finding**: rig-rs's provider surface is sufficient. The Rust-native foundation is worth keeping. No replatforming required.

### 6.2 Local model support — essential for air-gapped

Ollama is already in rig-rs's supported list. `mistral.rs` and `llama.cpp` are ABI-compatible with the OpenAI chat-completions schema when exposed via their HTTP servers, so rig-rs's OpenAI client points at a local endpoint without code changes.

For truly air-gapped deployments, Overdrive's option space is:

| Option | Ships LLM inference in the binary? | Operator burden |
|---|---|---|
| A. Call operator-provided endpoint (Ollama, mistral.rs, OpenAI) | No | Operator runs the endpoint |
| B. Embed `mistral.rs` in the control plane | Yes | None — but 8–30 GB model weights in the Image Factory |
| C. Ship LLM as a Overdrive *workload*, not in the binary | No (not in the CP binary) | Overdrive scheduler places the LLM workload |

**Recommendation: Option C.** Consistent with the single-binary principle (§2 principle 8) — the control plane stays ~30 MiB single / ~80 MiB HA. The LLM runs as a microVM workload (persistent, `snapshot_on_idle_seconds` set, GPU schedulable), scheduled like any other workload, reached by `rig-rs` via a private service VIP (§11). This is the same shape §11 already gives HTTP services and mirrors the Shape B pattern the prior research recommended for HolmesGPT. The Image Factory (§24) does not grow a Python runtime or model weights by default.

Tight-air-gap operators who cannot pull from public model registries can pre-stage the LLM workload's microVM image in their private Garage at deploy time. The image is a pure blob in Garage; the factory doesn't need model-specific tooling.

### 6.3 Cost control — already has the right substrate

HolmesGPT has basic cost controls but no published per-investigation benchmarks ([[1]](./holmesgpt-integration-analysis.md) §6.1). The Overdrive-native answer reuses primitives already in the platform:

- **Per-investigation token budget** — a field on the `Investigation` object (§3.2). The agent's tool-call loop checks the budget before each LLM turn and aborts gracefully at exhaustion, marking `InvestigationState::Failed { reason: "budget exhausted" }`.
- **Per-job token budget** — stored in IntentStore.job.policies. The LLM agent reads the budget when spawning an investigation scoped to that job's identities.
- **Cluster-wide monthly cap** — a dedicated `llm_spend` reconciler reads accumulated spend from the incident memory (`SUM(tokens_in * price_in + tokens_out * price_out)` over the month), gates new investigations when the cap is approached, and routes Tier-3-LLM escalations to queue-and-notify when exhausted.
- **Metering at the credential-proxy** — when the LLM is an external API (OpenAI, Anthropic), the `builtin:credential-proxy` sidecar already sees every egress request. Adding per-request token-cost accounting on the egress path (parsing the response `usage` object) lands accurate spend data at the ObservationStore table Corrosion already replicates. This is the same pattern §6.3 of the prior research recommended for HolmesGPT-as-workload.
- **Metering in-process** — when the LLM is local (Ollama workload), rig-rs's response objects carry token counts directly; the agent writes them to the investigation trace.

### 6.4 Finding

**rig-rs's provider surface is sufficient; keep it.** Local-model support lands via the Overdrive workload pattern, not by embedding inference in the control plane. Cost control uses the existing credential-proxy + ObservationStore + libSQL incident-memory pipeline — no new subsystem. The `Investigation` resource (§3) carries the budget and attribution. **Confidence: High.**

---

## 7. Structural Advantages Overdrive Has Over HolmesGPT

Explicit list — the prompt asked for this; it matters for framing the whitepaper edits.

### 7.1 Kernel-native telemetry with SPIFFE identity on every event

HolmesGPT scrapes Prometheus, queries kubectl, reads log aggregators. Every data point has been *instrumented by someone* — the application, a sidecar, an operator. Identity correlation is label-based; labels can be inconsistent, forged, or missing.

Overdrive's eBPF layer produces structured telemetry from the kernel for every workload without application instrumentation, with full SPIFFE ID on both ends of every flow (§12). This is not an incremental improvement — it is a different class of data. An investigation agent running on this substrate has:

- No missing events. Every packet, every allocation, every policy verdict is visible.
- No identity ambiguity. The src_identity and dst_identity are cryptographic SPIFFE IDs, not IPs or labels.
- No instrumentation lag. The data is emitted as the kernel sees it, not after the application has been restarted with a patched agent.

The first implication for the SRE agent: correlation (§4) is a SQL query, not a heuristic.

### 7.2 DuckLake SQL + time travel

HolmesGPT investigations query Prometheus, Loki, Datadog in real time — each with its own query language, its own retention, its own rate limits. Cross-system correlation is the agent's problem.

Overdrive has one SQL endpoint spanning the full telemetry history (§17) with time travel (`AT (TIMESTAMP => '...')` syntax). The agent runs one SQL query that joins flow events, resource events, policy verdicts, and allocation transitions over any time range. "What were flow patterns at the time of the last incident?" is a one-liner. This is not theoretically available in HolmesGPT — the underlying stores don't compose.

### 7.3 Incident memory with embedding similarity

HolmesGPT has runbooks (static, human-authored) and evals (for regression testing the agent). It does *not* have persistent cross-incident learning — every new investigation starts cold.

Overdrive §12 ships incident memory in libSQL with embedding-based similarity search. "We saw this before; last time the cause was X and the fix was Y" is the first tool the agent can call on any new anomaly. This compounds with operational age — the longer the cluster runs, the better the agent's starting point on each new incident.

### 7.4 Reconciler model with typed actions

HolmesGPT proposes YAML patches. Overdrive's LLM proposes typed `Action` enums that flow through Raft. Every benefit in §5.4 above applies: type-level risk classification, compile-time exhaustiveness, deterministic replay, SPIFFE-bound identity.

### 7.5 DST coverage of the investigation agent

§21 is already industrially rare — few orchestrators ship a DST framework, none ship a DST framework with the LLM tier simulated. `SimLLM` (§6 proposal below) closes the gap; once closed, investigation agent regressions are CI-gated. No other SRE agent has this.

### 7.6 Owned dataplane — hypothesis verification

This is the most underexploited structural advantage. HolmesGPT cannot add a diagnostic probe to the running system — it can only query what already exists. Overdrive owns the dataplane; the agent can propose *temporary diagnostic attachments*:

```rust
Action::AttachDiagnosticProbe {
    alloc:    SpiffeId,
    bpf_sha:  Sha256,                 // content-addressed, from a curated catalog
    duration: Duration,               // hard-bounded, e.g. 5 min
    reason:   "hypothesis: TCP retransmits on egress to payments-db",
}
```

Mechanics: the probe catalog lives in Garage, each probe signed by the platform CA. The approval gate admits Tier 0 (reversible read probe from curated catalog) automatically; Tier 1 (time-bounded counter attachment) is auto-ratified. The probe attaches via aya-rs, emits into the same eBPF ringbuf Overdrive already consumes, and detaches automatically at `duration` expiry — enforced by a reconciler deadline.

The investigation agent can *verify* hypotheses, not merely propose them. This is a capability no HolmesGPT-class agent can have — they do not own the datapath. Naming this explicitly in the whitepaper is worthwhile.

### 7.7 Security posture — kernel-enforced, not cooperation-enforced

HolmesGPT's safety story is read-only RBAC plus tool-level memory caps. Overdrive's safety story is BPF LSM + credential-proxy + content-inspector + mTLS + XDP policy (§19). A compromised investigation agent hits the same kernel walls as a well-behaved one. The investigation agent does not need to be trusted — only the platform it runs on does.

### 7.8 Finding

**Seven structural advantages, none of which HolmesGPT can match without a different substrate.** These are not marketing points; they are load-bearing design properties. The whitepaper edit (§8 below) makes them explicit as the justification for why Overdrive builds natively rather than integrates externally. **Confidence: High.**

---

## 8. Roadmap and Whitepaper Deltas

### 8.1 Phase landing for each proposal

| Proposal | Phase | Justification |
|---|---|---|
| §6 `SimLLM` DST trait | **Phase 2** | Alongside the other DST trait stabilisation. The LLM agent ships in Phase 5; it must be born simulation-testable. Adding SimLLM after the agent lands is retrofit work, and §21's discipline is that nondeterminism boundaries are abstracted *from day one*. |
| §1 Toolset catalog (as WASM, `builtin:overdrive-core`) | **Phase 5** | Ships with the LLM agent — the agent has no useful tool surface without it. The hard-coded §12 tool list in the whitepaper today is a *default toolset*, not a scaffold for the real thing. |
| §3 Investigation resource + lifecycle | **Phase 5** | Prerequisite for cost metering (§6), correlation (§4), and approval-gate attribution (§5). Ships with the agent. |
| §5 Typed `Action` enum + risk-tier gate | **Phase 5** | Already implied by §12's `propose_action`; this proposal makes the type and the tier explicit. Ships with the agent. |
| §2 Runbook primitive + loader reconciler | **Phase 6** | Useful but not blocking for first LLM agent release. The agent can run without runbooks, matching incidents via bare embedding-similarity against past incidents. Runbooks add human-authored investigation guidance later. |
| §4 Correlation via `correlation_key` + DuckLake SQL | **Phase 5** | Needed before the agent can de-duplicate investigations across alert-storm scenarios. Add to Phase 5 not Phase 6. The mechanism is trivial — a column and a DuckLake query pattern — but without it an AlertManager storm would spawn hundreds of duplicate investigations. |

Revised placement: Phase 2 = `SimLLM`; Phase 5 = Investigation resource + toolset catalog + typed Action + correlation; Phase 6 = Runbooks + the diagnostic-probe catalog for §7.6.

### 8.2 Open design questions for the user

1. **Runbook format — YAML frontmatter + markdown (HolmesGPT-compatible) or pure Rust/WASM?** The research recommends YAML frontmatter + markdown — leverages community catalogs, format is cheap to parse, LLM-friendly. Pure Rust/WASM buys nothing investigation-side. *Recommended: adopt HolmesGPT format.*
2. **Investigation as new resource or extension of Allocation?** Research recommends new resource. Allocation is about "what's running where"; Investigation is about "a reasoning session on an anomaly" — different lifecycle, different consistency requirement, different readers. *Recommended: new resource.*
3. **Local LLM — embed in control-plane binary, or run as workload?** Research recommends workload. Preserves the ~30 MiB / ~80 MiB control-plane footprint. *Recommended: workload.*
4. **Toolset WASM interface — rig-rs-style Rust traits with a WASM adapter, or a WIT (WASM Interface Type) specification from day one?** Research is uncommitted — this is a §9 sidecar-SDK-alignment question. *Recommend: ship the Rust trait, define the WIT when the first third-party toolset ships.*
5. **Diagnostic-probe catalog — platform-maintained only, or operator-extensible?** Platform-maintained is the conservative call: the probes touch the kernel via aya-rs; a malicious probe could exfiltrate data. Operator-extensible probes require the same attestation story as WASM toolsets but with stricter verification (BPF verifier + code review + signing). *Recommend: platform-maintained in v1; operator-extensible in a later phase after the verification story is built out.*

### 8.3 Whitepaper edits — draftable text

#### Draft: `§12.X Native SRE Investigation Agent` — new subsection under §12

> Overdrive's Tier 3 reasoning surface is a native SRE investigation agent built on `rig-rs`, operating on the four primitives of any mature investigation system: **toolsets, runbooks, investigations, and remediations**. Each is a first-class Overdrive concept, implemented with primitives the platform already owns rather than as a separate subsystem.
>
> **Toolsets — the declarative catalog.** The agent's tool surface is a catalog of toolsets loaded at runtime. `builtin:overdrive-core` is a Rust trait object shipped in the binary, exposing SQL tools against DuckLake and ObservationStore, read-only projections of IntentStore, and the incident-memory retriever. Third-party toolsets are WASM modules — same execution primitive as reconcilers (§18), policies (§13), and sidecars (§9) — content-addressed by sha256 in Garage, loaded declaratively from the IntentStore, scoped to the subset of host functions their manifest requests. A toolset declares the tools it exposes (name, description, input/output JSON schemas, risk class); the agent sees the union of loaded toolsets' tools. Investigations cite the toolset hashes used so transcripts are reproducible.
>
> **Runbooks — LLM-interpreted investigation guides.** Runbooks are markdown documents with YAML frontmatter describing trigger conditions and required toolsets. They are stored in Garage (content-addressed), indexed in the incident-memory libSQL alongside past incidents via the same embedding-similarity system (§12). When an investigation is triggered, the agent's first tool call matches the incident description against loaded runbooks; top-k matches are retrieved and included in the agent's context. Runbooks guide *reasoning* — the steps are interpretive, not deterministic. The deterministic counterpart is the workflow primitive (§18): runbooks produce diagnoses and proposals; workflows execute ratified proposals. Format matches the HolmesGPT runbook format so community-maintained runbook catalogs are leverageable directly.
>
> **Investigation — a first-class resource.** Investigations join `Job`, `Node`, `Allocation`, `Policy`, `Certificate` in the core data model. An investigation has a lifecycle (triggered → gathering → reasoning → concluding → concluded), a trigger (alert, reconciler escalation, operator query, scheduled), a correlation key, a list of affected SPIFFE identities, a token and wall-clock budget, and a trace of tool calls and LLM turns. Live investigation state lives in the ObservationStore (Corrosion `investigation_state` table); on conclusion, an investigation is compressed into an incident row in the incident-memory libSQL with embedding-indexed diagnosis for future retrieval. An `InvestigationReconciler` drives the lifecycle; proposals from the agent are queued through the graduated approval gate.
>
> **Correlation — identity-based, not label-based.** Every eBPF event in Overdrive carries cryptographic SPIFFE identity on both ends (§12). Correlation across alerts is a DuckLake SQL query over events joined on `src_identity` / `dst_identity` / `alloc_id`, windowed by causal-time proximity. Investigations carry a `correlation_key` derived from the primary identity, signal class, and time bucket; an incoming event whose key matches a live investigation's appends to that investigation rather than spawning a duplicate. This collapses alert-storm scenarios to one investigation per underlying phenomenon without label-based heuristics.
>
> **Remediations — typed actions, tiered gate.** The agent proposes state changes by emitting typed `Action` enum variants — `RestartAllocation`, `ScaleJob`, `RollBackDeployment`, `DrainNode`, `ResizeAllocation`, `ProposePolicyEdit`, `AttachDiagnosticProbe`, `StartWorkflow`. The risk tier is encoded on the variant at the type level: Tier 0 (reversible reads) auto-executes; Tier 1 (low-blast-radius writes) auto-executes with operator notification; Tier 2 (high-blast-radius writes) requires human ratification. Proposals land in the IntentStore (Raft); once ratified, the target reconciler consumes the typed action and converges. Actions flowing through Raft rather than YAML patches flowing through kubectl is a structural consequence of the §18 reconciler model: compile-time exhaustiveness, deterministic replay, SPIFFE-bound identity.
>
> **Hypothesis verification via the owned dataplane.** Where HolmesGPT-class agents are confined to querying existing instrumentation, the Overdrive investigation agent can propose *temporary diagnostic attachments* — `Action::AttachDiagnosticProbe { bpf_sha, alloc, duration }`. Probes come from a platform-maintained, platform-signed catalog; they attach via aya-rs, emit into the existing eBPF ringbuf, and detach automatically at deadline. Hypotheses become verifiable within one investigation turn rather than queued behind a human-executed instrumentation rollout. This capability is structurally unavailable to orchestrators that do not own their dataplane.
>
> **Credential and prompt-injection posture.** Where the LLM is an external API, `builtin:credential-proxy` (§8) holds the provider keys; the agent never sees them. Where the agent ingests third-party content (runbook bodies fetched from the catalog, log chunks returned by tools, documentation excerpts), `builtin:content-inspector` (§9) scans on ingress and flags prompt-injection payloads before the LLM sees them. BPF LSM blocks raw socket creation and unauthorised binary execution regardless of what the agent decides to do (§19). Security is structural, not cooperation-dependent.
>
> **Cost metering.** Every LLM call is attributed to an `investigation_id`. Token spend is accumulated per investigation, per job (via the investigation's `related_allocs`), and cluster-wide. When the egress path is used (external LLM API), the credential-proxy parses the response `usage` object and writes costs into the ObservationStore; when the LLM is a local workload, rig-rs's response objects carry token counts directly. A dedicated `llm_spend` reconciler enforces per-job and cluster-wide monthly caps — Tier-3 escalations route to queue-and-notify when the cap is approached, preserving observability without incurring spend.
>
> **Simulation.** The `SimLLM` trait (§21) returns deterministic completions from seeded transcripts. The full investigation trace — prompt, tool calls, tool outputs, LLM turns — is captured per investigation; re-running deterministically in CI reproduces the final diagnosis or flags a regression. Investigation-agent correctness joins control-plane correctness as a DST-gated property.

#### Draft: `§21 SimLLM` — addition to the DST trait list

> ```rust
> // LLM inference — no external API calls in simulation
> trait Llm: Send + Sync {
>     async fn complete(
>         &self,
>         prompt: &Prompt,
>         tools:  &[ToolDef],
>     ) -> Result<Completion>;
> }
> ```
>
> | Trait | Production | Simulation |
> |---|---|---|
> | `Llm` | `RigLlm` (rig-rs over real provider) | `SimLlm` (replays seeded transcript; records deviations; deterministic) |
>
> `SimLlm` replays a captured investigation transcript step-for-step. A test seed identifies the transcript; on mismatch (agent chose a different tool or produced a different parameter set), the test fails with a diff. Invariant assertions include "every investigation concludes within budget" and "no investigation proposes a Tier 2 action without at least one prior tool-call motivating it."

#### Draft: `§23 Roadmap` — amendments

Amendments to the existing roadmap:

- **Phase 2** — add:
  > `SimLlm` DST trait alongside `SimClock`, `SimTransport`, `SimEntropy`, `SimDataplane`, `SimDriver`, `SimObservationStore` — deterministic LLM completion replay for agent-tier simulation testing. Lands before the agent itself, preserving the §21 principle that nondeterminism boundaries are abstracted from day one.
- **Phase 5** — expand the existing "LLM observability agent (rig-rs)" bullet:
  > LLM observability agent (rig-rs) with native SRE-investigation primitives: `Investigation` as a first-class resource (ObservationStore + incident-memory libSQL), declarative toolset catalog (`builtin:overdrive-core` Rust trait object + WASM extensibility), typed `Action` enum with risk-tier approval gate, `correlation_key`-based de-duplication. The hard-coded §12 tool list becomes the default toolset.
- **Phase 6** — add:
  > Runbook primitive — markdown + YAML frontmatter matching the HolmesGPT format, content-addressed in Garage, indexed in incident-memory libSQL via embedding similarity, loaded by a `RunbookLoader` reconciler. Platform-signed diagnostic-probe catalog for `Action::AttachDiagnosticProbe` — curated BPF programs the investigation agent can attach to verify hypotheses, with duration-bounded auto-detach enforced by a deadline reconciler.

---

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Prior research — holmesgpt-integration-analysis.md | local SSOT | High | Primary research | 2026-04-19 | Yes — 18 sources cited there |
| Overdrive whitepaper v0.12 | local SSOT | High | Primary source of record | 2026-04-19 | N/A (authoritative) |
| rig-rs — rig-core docs | docs.rs | High | Technical documentation | 2026-04-19 | Yes |
| rig-rs — official site | rig.rs | High | Official | 2026-04-19 | Yes |
| rig-rs — GitHub | github.com | High | Official | 2026-04-19 | Yes |
| modelcontextprotocol/rust-sdk | github.com | High | Official (MCP spec maintainers) | 2026-04-19 | Yes |
| HolmesGPT — Custom Toolsets | holmesgpt.dev | High | Official (project docs) | 2026-04-19 | Yes — via prior research |
| HolmesGPT — Built-in Toolsets | holmesgpt.dev | High | Official (project docs) | 2026-04-19 | Yes — via prior research |
| HolmesGPT — Runbooks | holmesgpt.dev | High | Official (project docs) | 2026-04-19 | Yes — via prior research |
| HolmesGPT — Remote MCP Servers | holmesgpt.dev | High | Official (project docs) | 2026-04-19 | Yes — via prior research |
| HolmesGPT — Why HolmesGPT | holmesgpt.dev | High | Official (project docs) | 2026-04-19 | Yes — via prior research |
| HolmesGPT — AI Providers | holmesgpt.dev | High | Official (project docs) | 2026-04-19 | Yes — via prior research |
| HolmesGPT — custom_toolset.yaml example | github.com | High | Official (source) | 2026-04-19 | Yes |
| HolmesGPT — tool_calling_llm.py | github.com | High | Official (source) | 2026-04-19 | Yes — via prior research |
| HolmesGPT — Helm values.yaml | github.com | High | Official (source) | 2026-04-19 | Yes — via prior research |
| CNCF — HolmesGPT project page | cncf.io | High | Official (foundation) | 2026-04-19 | Yes — via prior research |
| CNCF blog — HolmesGPT (2026-01-07) | cncf.io | High | Official (foundation) | 2026-04-19 | Yes — via prior research |
| LiteLLM docs — HolmesGPT | docs.litellm.ai | High | Technical documentation | 2026-04-19 | Yes — via prior research |
| MCP spec — Transports (2025-03-26) | modelcontextprotocol.io | High | Standards body | 2026-04-19 | Yes |
| Rust MCP SDK discussion | systemprompt.io | Medium-High | Practitioner | 2026-04-19 | Yes (against rmcp source) |
| HolmesGPT — DeepWiki toolset architecture | deepwiki.com | Medium-High | Third-party analysis | 2026-04-19 | Yes (against primary docs) |
| HolmesGPT — community-toolsets repo | github.com | High | Official (project org) | 2026-04-19 | Yes |
| HolmesGPT — Kubernetes permissions | holmesgpt.dev | High | Official (project docs) | 2026-04-19 | Yes — via prior research |
| HolmesGPT — canonical repo (CNCF) | github.com | High | Official (canonical source) | 2026-04-19 | Yes — via prior research |

Reputation: High: 22 (92%) | Medium-High: 2 (8%) | Avg reputation: 0.98.

---

## Knowledge Gaps

### Gap 1: WASM-in-control-plane startup cost for toolsets

**Issue**: §9 WASM sidecar SDK is specified in the whitepaper but not yet implemented; per-toolset instantiation overhead (first-call vs warm-pool) for investigation tool calls is not benchmarked.
**Attempted**: Whitepaper §9 (warm instance pool pattern); rig-rs docs; HolmesGPT `tool_calling_llm.py` for comparable latency baselines.
**Recommendation**: Pre-instantiate toolset WASM modules at load time, maintain a warm instance pool keyed on toolset sha. The §9 sidecar-pool pattern applies directly. Benchmark before Phase 5 GA to confirm first-call latency is <50ms and warm-call latency is <5ms (consistent with §16 claims for WASM functions).

### Gap 2: Embedding provider for incident-memory similarity

**Issue**: §12 specifies "embedding-based similarity search" but does not specify the embedding provider. Options: (a) same external LLM provider via rig-rs (OpenAI text-embedding-3, Anthropic, Gemini, Cohere); (b) local model via an Ollama workload; (c) in-process via `rust-bert` or similar. Cost, latency, and privacy trade-offs differ.
**Attempted**: Whitepaper §12; rig-rs docs for embedding support (confirmed supported); no explicit Overdrive-side decision documented.
**Recommendation**: Default to the same provider the agent uses for completion (via rig-rs's EmbeddingModel trait), with local-model fallback via the same workload-hosted-LLM pattern (§6.2). Decision can be deferred to Phase 5 implementation.

### Gap 3: MCP server exposure — still worth doing?

**Issue**: The prior research recommended Shape D — Overdrive exposes itself as an MCP server ([[1]](./holmesgpt-integration-analysis.md) §3 Shape D). The current framing is "build the SRE agent natively." Does an MCP server still matter?
**Attempted**: Re-read [[1]](./holmesgpt-integration-analysis.md) §3 Shape D and §5.
**Recommendation**: **Yes, independently.** An MCP server lets operator tooling (Claude Desktop, Cursor, other future MCP-speaking SRE tools) query Overdrive without each integration growing a custom client. It is orthogonal to whether Overdrive's *own* agent is native. The MCP server and the native agent are complementary: the native agent is the production self-healing path; MCP exposure is the operator-facing query path. Ship both. This amends the framing of the current document rather than contradicting the prior research.

### Gap 4: Concrete `AttachDiagnosticProbe` catalog

**Issue**: The proposal in §7.6 names the capability but does not enumerate which probes ship on day one.
**Attempted**: §7 eBPF dataplane; §22 real-kernel integration testing.
**Recommendation**: Seed catalog for Phase 6 should include (a) per-alloc TCP retransmit counter, (b) syscall histogram for a named alloc, (c) BPF LSM denial-rate counter for a named alloc, (d) per-alloc fd-open latency histogram. Each probe ships with a Tier 3 integration test fixture (§22) and a veristat baseline (§22 Tier 4). Full catalog design is a separate research item.

---

## Conflicting Information

None identified. Every design proposal composes with existing §12 / §18 / §21 primitives cleanly; no two proposals contradict each other; no cited source contradicts another on a load-bearing fact.

---

## Recommendations for Further Research

1. **`AttachDiagnosticProbe` probe catalog design.** Which BPF programs ship in the seed catalog; how signing and operator extensibility work; how the duration-bounded detach deadline is enforced under simulated and real clock skew. Tie-in to §22 Tier 4 verifier-complexity gates.
2. **Toolset dependency resolution.** When a runbook declares `required_toolsets: ["builtin:overdrive-core", "sha256:abc..."]` and the cluster does not have the required WASM toolset loaded, what happens? Load on demand from Garage? Refuse the investigation? Partial mode? Answer matters for multi-tenant scenarios.
3. **`SimLLM` transcript capture format.** JSON-Lines of `{turn_id, prompt_hash, tool_calls, tool_outputs, llm_response}` is the obvious shape, but long transcripts hit storage; compression and rollup strategies should be researched before the format is frozen.
4. **Investigation cost-regression testing.** Given stored transcripts, the CI harness can replay them against a *new* model (simulated or live) and measure token delta. This is an under-researched capability — it would let operators assess model-upgrade cost impact before rollout.
5. **Operator-extensible BPF probe story.** Gap 4 above notes the catalog is platform-maintained in v1. The operator-extensibility design — attestation, code review, signing — is a multi-phase research item that likely drafts alongside §24 Image Factory v2.

---

## Full Citations

[1] Schack Abildskov, M. (researcher: Nova). "HolmesGPT Integration Analysis for Overdrive". Overdrive docs. 2026-04-19. `/Users/marcus/conductor/workspaces/overdrive/taipei-v1/docs/research/holmesgpt-integration-analysis.md`. Accessed 2026-04-19. *(18 upstream sources cited there.)*

[2] Overdrive project. "Overdrive Whitepaper v0.12". Local SSOT. Accessed 2026-04-19. `/Users/marcus/conductor/workspaces/overdrive/taipei-v1/docs/whitepaper.md`.

[3] 0xPlaygrounds. "rig-core". docs.rs. Accessed 2026-04-19. <https://docs.rs/rig-core/latest/rig/>.

[4] 0xPlaygrounds. "Rig — Build Powerful LLM Applications in Rust". Accessed 2026-04-19. <https://www.rig.rs/>.

[5] 0xPlaygrounds. "rig — GitHub". Accessed 2026-04-19. <https://github.com/0xPlaygrounds/rig>.

[6] Model Context Protocol. "rust-sdk — The official Rust SDK for the Model Context Protocol". GitHub. Accessed 2026-04-19. <https://github.com/modelcontextprotocol/rust-sdk>.

[7] Model Context Protocol. "Transports — specification 2025-03-26". Accessed 2026-04-19. <https://modelcontextprotocol.io/specification/2025-03-26/basic/transports>.

[8] HolmesGPT project. "Custom Toolsets". holmesgpt.dev. Accessed 2026-04-19. <https://holmesgpt.dev/latest/data-sources/custom-toolsets/>.

[9] HolmesGPT project. "Built-in Toolsets". holmesgpt.dev. Accessed 2026-04-19. <https://holmesgpt.dev/latest/data-sources/builtin-toolsets/>.

[10] HolmesGPT project. "Runbooks". holmesgpt.dev. Accessed 2026-04-19. <https://holmesgpt.dev/latest/reference/runbooks/>.

[11] HolmesGPT project. "Why HolmesGPT". holmesgpt.dev. Accessed 2026-04-19. <https://holmesgpt.dev/latest/why-holmesgpt/>.

[12] HolmesGPT project. "custom_toolset.yaml example". GitHub. Accessed 2026-04-19. <https://github.com/robusta-dev/holmesgpt/blob/master/examples/custom_toolset.yaml>.

[13] HolmesGPT project. "community-toolsets". GitHub. Accessed 2026-04-19. <https://github.com/robusta-dev/holmesgpt-community-toolsets>.

[14] CNCF. "HolmesGPT: Agentic troubleshooting built for the cloud native era". CNCF Blog. 2026-01-07. Accessed 2026-04-19. <https://www.cncf.io/blog/2026/01/07/holmesgpt-agentic-troubleshooting-built-for-the-cloud-native-era/>.

---

## Research Metadata

Duration: ~40 min (this pass; total including prior research ~85 min) | Examined this pass: 10 new sources + prior research's 18 = 24 unique | Cited: 14 canonical entries (many prior-research sources cited transitively via [[1]](./holmesgpt-integration-analysis.md)) | Cross-refs: every design proposal referenced ≥1 whitepaper section + ≥1 HolmesGPT source | Confidence: High 100% (every major claim has primary-source backing from whitepaper or HolmesGPT canonical docs) | Output: `docs/research/native-sre-agent-design.md`
