# ADR-0054 — ProbeRunner subsystem: per-alloc-per-probe tokio tasks in `overdrive-worker`; `Prober` port traits per mechanic; `ProbeResultRow` as LWW observation

## Status

Accepted. 2026-05-24. Decision-makers: Morgan (proposing); DESIGN-wave
output of `docs/feature/service-health-check-probes/`.

Tags: phase-1, service-kind, application-arch, worker-subsystem,
port-trait.

**Companion ADRs**: ADR-0055 (ServiceLifecycleReconciler), ADR-0056
(per-kind streaming evolution for `Stable`/`Failed`), ADR-0057
(`[[health_check.*]]` TOML spec), ADR-0058 (default-probe inference),
ADR-0059 (exec-probe cgroup placement). Predecessors: ADR-0029
(worker crate), ADR-0030 (exec driver), ADR-0035 (reconciler
runtime), ADR-0048 (rkyv envelope).

## Context

Per `docs/feature/service-health-check-probes/feature-delta.md` and
the RCA at
`docs/analysis/root-cause-analysis-coinflip-submit-reports-running-on-exit-1.md`,
the Phase 1 Service-kind reconciler needs a continuous source of
"is the workload actually serving?" observation that the kernel's
acceptance of `fork+exec` cannot provide. Per `.claude/rules/development.md`
§ "Reconciler I/O" the reconciler is a pure sync function; it cannot
issue HTTP requests, open TCP sockets, or spawn child processes.
Probes must run somewhere else and surface as `ObservationStore` rows
the reconciler reads.

Open questions resolved here (P1-Q1 per feature-delta):

1. Where does the ProbeRunner live in the worker crate's task graph
   (per-alloc loop vs per-alloc tokio task vs shared scheduler)?
2. What is the trait surface — one omnibus `Prober` trait or one per
   mechanic (HTTP / TCP / Exec)?
3. What is the row shape that flows from runner to reconciler?
4. How does shutdown propagate (alloc stop → probe stop)?

Industry alignment (research § 3.3 D5): Kubernetes runs probes via
the kubelet's `prober.Manager` which spawns one goroutine per
container-probe-type combination, with a shared HTTP transport pool.
Nomad runs probes via the Consul-agent in a shared event loop. The
goroutine-per-probe shape gives independent failure isolation; the
shared-event-loop shape couples probe failures across allocs. K8s's
design has been validated at scale; copy the shape adapted to tokio.

## Decision

### 1. Placement — `overdrive-worker` adapter-host crate; one subsystem per node

The `ProbeRunner` lives in `crates/overdrive-worker/src/probe_runner/`
(new module tree). It is a sibling of `ExecDriver` (ADR-0030) and
`CgroupManager` (ADR-0026) under the `adapter-host` crate class
(ADR-0003). The runner is constructed once per node at binary boot
(`overdrive-cli::commands::serve`) and shared by reference (`Arc`)
across every Service-kind alloc on that node.

Rationale: per `feature-delta.md` C1 ("Probe runner lives in the
WORKER process, not the control plane"). The runner takes real
network sockets / spawns real subprocesses / reads real wall-clock —
all `adapter-host` concerns. The control-plane sees only the
`ObservationStore` rows the runner writes; there is no direct API
surface between control-plane and runner.

### 2. Task graph — per-alloc-per-probe tokio task, supervised by per-alloc supervisor

When a Service alloc reaches `Running` (alloc-status row written by
ExecDriver), the worker subsystem spawns:

```
ProbeRunner::start_alloc(alloc_id, probe_descriptors, alloc_started_at)
  → spawns one per-alloc supervisor task
      → spawns N per-probe-instance tasks (one per ProbeDescriptor)
          → each loops: tick → run probe → write ProbeResultRow → sleep(interval)
```

The supervisor owns:

- A `CancellationToken` (tokio_util) that signals shutdown when the
  alloc transitions to a terminal state. Per-probe tasks select on
  `cancel.cancelled()` between ticks; cancellation drops in-flight
  probes (kills exec children, closes TCP sockets, aborts HTTP
  requests).
- The `Arc<dyn ObservationStore>` write surface, shared with
  per-probe tasks.
- A `JoinSet<()>` holding the per-probe tasks; on supervisor
  cancellation the JoinSet is dropped and every task is aborted.

Per-probe-task body shape (pseudo-code):

```rust
loop {
  tokio::select! {
    _ = cancel.cancelled() => break,
    _ = clock.sleep(interval) => {}
  }
  let outcome = match descriptor.mechanic {
    Mechanic::Tcp(addr)   => tcp.probe(addr, timeout).await,
    Mechanic::Http(req)   => http.probe(req, timeout).await,
    Mechanic::Exec(spec)  => exec.probe(spec, timeout, alloc_id).await,
  };
  let row = ProbeResultRow::lww(alloc_id, probe_idx, role, mechanic, outcome, clock.now_unix());
  obs.write_probe_result(&row).await?;
}
```

**Why per-alloc-per-probe-type tokio task (chosen) vs alternatives**:

| Shape | Why considered | Why rejected (where applicable) |
|---|---|---|
| **(a) Per-alloc-per-probe tokio task** (CHOSEN) | Failure isolation per probe; independent scheduling; aborts cleanly on cancel; matches K8s `prober.Manager` shape | — |
| (b) Single per-alloc loop multiplexing all probes via `select!` | Fewer tasks; single arbitration point | Head-of-line blocking: a slow exec probe (timeout=5s) starves a fast TCP probe; failure of one probe stalls all. Violates `feature-delta.md` Risk #1. |
| (c) Shared worker-process scheduler (one task across all allocs) | Lowest task count; central control | Cross-alloc head-of-line blocking; cascading failure surface (one bad probe affects every alloc); harder to reason about Drop semantics |
| (d) `std::thread`-per-probe (no tokio) | Strict isolation; pre-emptive scheduling | Wastes a thread per probe; tokio task is two orders of magnitude cheaper; the runner is I/O-bound, not CPU-bound |

Task overhead estimate (research § 3.3 D5): tokio task ≈ 64–128 B
stack + scheduler bookkeeping; a Service with 3 probes × 1 replica
costs ~512 B of supervisor state. Within `feature-delta.md`'s K2
guardrail "≤0.5% CPU per Service-alloc-with-3-probes" by 2+ orders
of magnitude.

### 3. Port traits — one per mechanic, `overdrive-core` declares; `overdrive-worker` provides production binding

Per `.claude/rules/development.md` § "Trait definitions specify
behavior, not just signature" and § "Port-trait dependencies", three
new port traits land in `overdrive-core::traits`:

```rust
// crates/overdrive-core/src/traits/prober.rs (new module)

pub type ProbeOutcome = Result<(), ProbeFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeFailure {
    ConnectionRefused,
    Timeout { after: Duration },
    HttpStatus { code: u16 },
    Redirect { code: u16 },          // 3xx — non-following, per US-02 AC
    ExecNonZero { exit_code: i32 },
    ExecSpawnFailed { reason: String }, // command not found, EACCES, etc.
    Io { reason: String },           // last-resort generic
}

#[async_trait]
pub trait TcpProber: Send + Sync {
    /// Open a TCP connection to `target` with `timeout`. Success when
    /// connect handshake completes; close immediately after.
    ///
    /// Preconditions: `target.ip()` is reachable from the worker's
    /// network namespace; `timeout > 0`.
    ///
    /// Postconditions on `Ok(())`: the TCP handshake completed
    /// (SYN → SYN-ACK → ACK). The socket is closed before return.
    /// Postconditions on `Err(ProbeFailure::ConnectionRefused)`: peer
    /// actively refused (RST received within timeout).
    /// Postconditions on `Err(ProbeFailure::Timeout { after })`: no
    /// SYN-ACK received within `timeout`; `after == timeout` ±
    /// scheduling jitter.
    ///
    /// Edge cases: `target.port() == 0` is allowed by the type but
    /// will always fail with `ConnectionRefused` (kernel rejects).
    /// Probes during workload shutdown: the supervisor cancels via
    /// `CancellationToken` BEFORE this method is called; concurrent
    /// cancellation aborts the future via `tokio::select!`.
    async fn probe(&self, target: SocketAddrV4, timeout: Duration) -> ProbeOutcome;
}

#[async_trait]
pub trait HttpProber: Send + Sync {
    /// Issue an HTTP GET to `req` with `timeout`. Success when
    /// the response status is 2xx within timeout.
    ///
    /// Preconditions: `req.url.scheme() == "http"` (plain HTTP only
    /// per C6); `req.method == GET` (Phase 1 only per US-02 AC);
    /// `req.body.is_empty()`.
    ///
    /// Postconditions on `Ok(())`: response received with status in
    /// 200..300; body discarded without read (TCP RST or graceful
    /// close is implementation-defined). No redirects followed.
    /// Postconditions on `Err(ProbeFailure::HttpStatus { code })`:
    /// response received with status NOT in 200..300 AND NOT in
    /// 300..400 within timeout.
    /// Postconditions on `Err(ProbeFailure::Redirect { code })`:
    /// response received with status in 300..400. Probe does NOT
    /// follow redirects (US-02 AC, research § 6.1 Pitfall 5).
    /// Postconditions on `Err(ProbeFailure::ConnectionRefused)`:
    /// TCP connect failed.
    /// Postconditions on `Err(ProbeFailure::Timeout { after })`: no
    /// complete response received within `timeout`.
    ///
    /// Edge cases: 204 No Content is Pass (2xx). 304 Not Modified
    /// is Redirect-class Fail (3xx, no body). HTTP/1.0 200 with no
    /// Content-Length terminated by close is Pass. HTTP/2 negotiated
    /// via prior knowledge is NOT supported in Phase 1 (operator
    /// receives HttpStatus on h2 upgrade attempt).
    async fn probe(&self, req: HttpProbeRequest, timeout: Duration) -> ProbeOutcome;
}

#[async_trait]
pub trait ExecProber: Send + Sync {
    /// Spawn `spec.command[0]` (with subsequent argv) as a member of
    /// the cgroup `alloc_cgroup`, with `timeout`. Success when exit
    /// status is 0 within timeout.
    ///
    /// Preconditions: `spec.command.first().is_some()`;
    /// `timeout > 0`; `alloc_cgroup` exists and the calling process
    /// has write permission on `alloc_cgroup/cgroup.procs` (per
    /// ADR-0059 / ADR-0026).
    ///
    /// Postconditions on `Ok(())`: child exited with code 0; the
    /// child's PID was a member of `alloc_cgroup` for the entire
    /// process lifetime (verified per ADR-0059 mechanism).
    /// Postconditions on `Err(ProbeFailure::ExecNonZero { exit_code })`:
    /// child exited with non-zero code within timeout.
    /// Postconditions on `Err(ProbeFailure::Timeout { after })`:
    /// child still running after `timeout` elapsed; SIGKILL sent;
    /// child reaped before return.
    /// Postconditions on `Err(ProbeFailure::ExecSpawnFailed { reason })`:
    /// `execve` failed (ENOENT, EACCES, ENOMEM); `reason` names the
    /// errno via `std::io::Error::raw_os_error` mapping.
    ///
    /// **Cgroup-placement failures** (per ADR-0059 mechanism):
    /// errors writing the probe child's PID into the workload's
    /// `cgroup.procs` file — `ENOSPC` (cgroup full, max-descendants
    /// hit), `EACCES` (cgroup delegation missing or revoked),
    /// `ENOENT` (cgroup directory vanished mid-probe, e.g. workload
    /// torn down between supervisor signal and prober dispatch),
    /// `EBUSY` (cgroup frozen by an external controller) — surface
    /// as `ProbeFailure::ExecSpawnFailed { reason }` carrying the
    /// underlying errno text (`std::io::Error::raw_os_error`
    /// mapping; same shape as `execve` failures above). The runner
    /// does **NOT** retry cgroup-placement errors automatically —
    /// the next scheduled tick re-attempts on its own cadence, so
    /// transient cgroup conditions self-heal without a per-call
    /// retry loop. **Retry-on-cgroup-error as a tunable policy is
    /// a DELIVER-wave decision deliberately deferred** so the
    /// trait contract stays stable across retry-strategy
    /// iterations; the contract pins only that cgroup errors map
    /// to the same typed variant `execve` errors do. Production
    /// adapter (`CgroupExecProber`) propagates the errno verbatim;
    /// sim adapter (`SimExecProber`) accepts an injected
    /// `ProbeFailure::ExecSpawnFailed { reason }` for cgroup-error
    /// scenario tests.
    ///
    /// Edge cases: empty `spec.command` is rejected by parser (per
    /// ADR-0057), so this method never sees it. A child that
    /// daemonises and detaches between the supervisor's spawn and
    /// the wait is reaped via cgroup-scoped SIGKILL on timeout (per
    /// ADR-0059's cgroup.kill mechanism). A child that survives
    /// SIGKILL within 1s is a kernel bug; the prober logs and
    /// returns `Timeout { after }`.
    async fn probe(
        &self,
        spec: ExecProbeSpec,
        timeout: Duration,
        alloc_cgroup: &CgroupPath,
    ) -> ProbeOutcome;
}
```

Production bindings live in `crates/overdrive-worker/src/probe_runner/`:

- `TokioTcpProber` — uses `tokio::net::TcpStream::connect` wrapped in
  `tokio::time::timeout`.
- `HyperHttpProber` — uses `hyper-util::client::legacy::Client` with
  a connection pool sized to N allocs × M probes; per-request timeout
  via `hyper::client::conn` shape.
- `CgroupExecProber` — uses the `CloneIntoCgroup`-shaped mechanism
  per ADR-0059.

Sim bindings live in `crates/overdrive-sim/src/adapters/probers.rs`
(new module):

- `SimTcpProber` / `SimHttpProber` / `SimExecProber` — driven by an
  injected `BTreeMap<(AllocationId, ProbeIdx), VecDeque<ProbeOutcome>>`
  the test harness pre-populates. Each `probe()` call pops the next
  outcome. Per `.claude/rules/development.md` § "Production code is
  not shaped by simulation" the production trait surface is the only
  shape; sim adapters honour the same `async fn probe(...)`
  signature without imposing yield or sleep contortions on
  production.

### 4. Three traits not one — rationale

A single `Prober` trait with `enum Mechanic { Tcp(...), Http(...), Exec(...) }`
was considered. Rejected because:

- Each mechanic has distinct preconditions, distinct postcondition
  shape, and distinct adapter type (TCP socket vs HTTP client pool
  vs cgroup-aware spawn). A single trait forces every adapter to
  carry every mechanic's dependency.
- The DST equivalence test per `.claude/rules/development.md` §
  "Trait definitions specify behavior, not just signature" can drive
  three small contracts independently; one omnibus trait would
  conflate the contracts and force every probe-type's edge cases
  into one harness.
- Trait surface stability is per-mechanic: HTTPS / mTLS / gRPC
  (Phase 3+ per C6) add to `HttpProber` only; new exec
  semantics (cgroup v1, namespaced exec) add to `ExecProber` only.

### 5. `ProbeResultRow` — LWW per `(alloc_id, probe_idx)`, additive to ObservationStore

Per `feature-delta.md` C2 and C3 / `.claude/rules/development.md` §
"Persist inputs, not derived state", the row is the most recent
observation per probe, not an append-mode tick history:

```rust
// crates/overdrive-core/src/observation/probe_result.rs (NEW module)

#[derive(
    Debug, Clone, PartialEq, Eq,
    rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
    Serialize, Deserialize,
)]
pub struct ProbeResultRow {
    pub alloc_id: AllocationId,
    pub probe_idx: ProbeIdx,            // newtype over u32
    pub role: ProbeRole,                // Startup | Readiness | Liveness
    pub mechanic: ProbeMechanic,        // Tcp { addr } | Http { url } | Exec { cmd0 }
    pub status: ProbeStatus,            // Pass | Fail
    pub last_fail_reason: Option<String>, // populated iff status == Fail
    pub last_observed_at: UnixInstant,  // wall-clock of the producing tick
    pub attempts: u32,                  // cumulative since alloc spawn (informational)
}
```

The `ObservationStore` trait gains:

```rust
async fn write_probe_result(&self, row: &ProbeResultRow) -> Result<(), Error>;
async fn list_probe_results_for_alloc(&self, alloc_id: &AllocationId)
    -> Result<BTreeMap<ProbeIdx, ProbeResultRow>, Error>;
```

Key shape `(alloc_id, probe_idx)` is a composite primary key in the
backing `redb` table per ADR-0012's `LocalObservationStore`
convention. Latest-writer-wins is structural (`redb::insert` semantics
on the composite key); no merge logic needed.

Per `.claude/rules/development.md` § "Ordered-collection choice" —
the return shape is `BTreeMap` not `HashMap` because the reconciler
iterates it (deciding tick for `Stable` walks every startup probe to
find the witness; `alloc status` render walks every probe to emit
the Probes section in stable order).

Per ADR-0048 § "Version-bump procedure" the new row ships as
`ProbeResultRowEnvelope::V1(ProbeResultRowV1)`; the public alias is
`type ProbeResultRow = ProbeResultRowV1`. Existing fixtures are
unaffected (greenfield row).

**Discriminant-offset invariant (load-bearing, per ADR-0048 § "Why a
per-type rkyv enum is forward-compatible across variant additions"):**
the `#[repr(u8)]` discriminant for `ProbeResultRowEnvelope::V1` is
fixed at variant-declaration order — V1 = 0. Future variants (V2,
V3, …) append at the tail only. Reordering variants, inserting a
new variant before V1, or removing V1 silently shifts the
discriminant byte at offset 0 of every archived `ProbeResultRow`
payload and breaks every existing V1 reader without a compile-time
signal. ADR-0048's structural defense — variants are only ever
appended — applies here verbatim.

The schema-evolution fixture at
`crates/overdrive-core/tests/schema_evolution/probe_result_row.rs`
MUST pin BOTH the archived bytes AND the variant discriminant
value: `const FIXTURE_V1_DISCRIMINANT: u8 = 0;` alongside the
hex-encoded `FIXTURE_V1` bytes. The test asserts both on every run;
a discriminant-shift regression fails loud at PR time rather than
manifesting as a silent decode corruption months later in
production. Cross-reference: known forward-traps documented in the
`feedback_rkyv_envelope_forward_traps.md` auto-memory derived from
prior ADR-0048 adversarial reviews — this ADR closes the
"discriminant offset re-pin discipline" gap for the
`ProbeResultRow` envelope.

### 6. Lifecycle integration — worker spawns runner per Service alloc

The worker subsystem's existing per-alloc supervisor (the
`Driver::start` → exit-watcher pathway in ADR-0030) gains a hook:

```rust
// crates/overdrive-worker/src/driver.rs (extension)

impl ExecDriver {
    async fn on_alloc_running(&self, alloc_id: &AllocationId, spec: &AllocationSpec) {
        if spec.kind == WorkloadKind::Service {
            self.probe_runner.start_alloc(
                alloc_id.clone(),
                spec.probe_descriptors.clone(),
                self.clock.now_unix(),
            ).await;
        }
    }

    async fn on_alloc_terminal(&self, alloc_id: &AllocationId) {
        self.probe_runner.stop_alloc(alloc_id).await;
    }
}
```

`probe_descriptors: Vec<ProbeDescriptor>` is a new field on
`AllocationSpec` populated by the submit handler from the validated
`ServiceSpec` (per ADR-0057). Job and Schedule kinds pass an empty
vec; the runner is a no-op for those kinds — the kind check above is
defence-in-depth, not the primary gate.

### 7. Earned Trust — `ProbeRunner::probe()` startup self-check

Per `.claude/rules/development.md` principle 12 (Earned Trust): the
runner exposes:

```rust
impl ProbeRunner {
    pub async fn probe(&self) -> Result<(), ProbeRunnerError>;
}
```

The composition root (`overdrive-cli::commands::serve`) calls this
after construction and before binding the HTTP server. The probe:

1. Spawns a sacrificial TCP listener on `127.0.0.1:0`.
2. Runs `TcpProber::probe(listener.addr, 100ms)` — must succeed.
3. Closes the listener.
4. Runs `TcpProber::probe(closed_addr, 100ms)` — must fail with
   `ConnectionRefused`.

Failure refuses startup with structured `health.startup.refused`
event (per ADR-0035 §7 / Earned Trust composition-root invariant).
HTTP and Exec probers are NOT exercised at this layer (HttpProber's
probe would need an external endpoint; Exec needs a workload cgroup
which doesn't exist yet). Their adapters carry their own
unit-level probes in Tier-2 (`tests/integration/probers.rs`).

This rule is itself enforced structurally per Earned Trust principle
12 sub-clause (c): the dst-lint AST scanner walks
`crates/overdrive-worker/src/probe_runner/` for a `probe(&self)` method
on the `ProbeRunner` impl block; a missing method fails the PR.

## Considered alternatives

### Alternative A — ProbeRunner in `overdrive-control-plane`

Place the runner in the control-plane crate as a sibling of
`ReconcilerRuntime`. Rejected: violates `feature-delta.md` C1 ("Probe
runner lives in the WORKER process"). The control plane is the
intent-layer custodian; observation comes from the machine that runs
the workload. In Phase 2+ multi-node, the runner must run on every
worker, not in a single central control plane.

### Alternative B — Reconciler-driven probes (synchronous, in `reconcile`)

Have `ServiceLifecycleReconciler::reconcile` call `prober.probe(...)`
directly. Rejected: violates `.claude/rules/development.md` §
"Reconciler I/O" — `reconcile` is pure sync, no `.await`, no I/O,
no DB handle. The reconciler is a pure consumer of observation
rows; probes are observation producers.

### Alternative C — Worker-side single-task scheduler

A single tokio task in the worker that holds a `BinaryHeap<(deadline,
ProbeId)>` and dispatches each due probe via `tokio::spawn` for the
actual probe call. Rejected: the per-alloc supervisor shape (chosen)
already gives the same scheduling structurally — the per-probe task's
`clock.sleep(interval)` IS the heap, just spread across N supervisor
threads. The single-scheduler shape adds central state without
adding isolation; failure of the scheduler stops every probe.

### Alternative D — Probe results on `LifecycleEvent` (broadcast)

Emit probe outcomes on the broadcast bus instead of writing to
`ObservationStore`. Rejected: violates ADR-0037 layering (only
reconciler-decided terminal conditions cross the broadcast boundary)
and `feature-delta.md` C2 (results are observation, not intent).
`LifecycleEvent` is for state transitions, not raw observation;
probe results are the latter.

### Alternative E — Append-mode `ProbeResultHistoryRow`

Each tick writes a new row keyed `(alloc_id, probe_idx, tick_id)`
preserving full history. Rejected: violates `.claude/rules/development.md`
§ "Persist inputs, not derived state" and `feature-delta.md`
Risk #2. Operational history (consecutive failures, last
last_observed_at) is recomputed at read time from the LWW row plus
the reconciler View. Phase 3+ may add a separate `probe_event_log`
table for forensic audit; out of scope here.

## Consequences

### Positive

- **Per-probe failure isolation**: a slow exec probe never blocks a
  fast TCP probe in the same alloc; a failing probe in alloc A never
  affects alloc B.
- **Reconciler stays pure** (`.claude/rules/development.md` §
  "Reconciler I/O" preserved): probe outcomes flow as observation
  rows; reconciler reads `BTreeMap<ProbeIdx, ProbeResultRow>` from
  `actual`.
- **Three small traits with explicit contracts** are easier to test
  individually than one omnibus trait. The DST equivalence harness
  (per `.claude/rules/development.md`) drives each trait through
  hand-picked + property-tested call sequences against both
  production and sim adapters.
- **Row cardinality is bounded by spec, not by time**: N allocs ×
  M probes × O(1) per probe (LWW). Phase 1 worst case
  (1 alloc × 3 probes) = 3 rows.
- **Phase 2 multi-node fan-out is structural**: each node runs its
  own `ProbeRunner`; rows gossip via Corrosion's per-row LWW;
  ownership rule is `node_id == probe-producing node`.

### Negative

- **Three new port traits + three sim adapters + three production
  adapters** to maintain. Bounded — each adapter is < 200 LOC.
- **New crate dependencies**: `hyper-util` + `hyper` ~ 1.x (HTTP
  client), `tokio-util` ~ 0.7.x (CancellationToken). Both already
  in the workspace graph; no new top-level deps.
- **Per-alloc supervisor task adds ~1 KB of state per Service
  alloc.** Phase 1 single-node single-replica = negligible. Phase
  2+ with replicas=N adds N × 1 KB; still negligible.
- **Three separate Prober traits generate six adapter
  implementations + three trait test suites.** TcpProber, HttpProber,
  and ExecProber each require a matching production impl
  (`TokioTcpProber`, `HyperHttpProber`, `CgroupExecProber`) AND a
  matching sim impl (`SimTcpProber`, `SimHttpProber`,
  `SimExecProber`), plus three independent DST equivalence harnesses
  per `.claude/rules/development.md` § "Trait definitions specify
  behavior, not just signature". Per-mechanic semantics divergence
  (TCP reachability vs HTTP URI parsing vs Exec subprocess
  lifecycle) justifies the separation today — see § 4 above — but
  the cost is real and worth flagging for a future-simplification
  candidate. A future iteration may consider a unified `Prober`
  trait with mechanic-specific methods, or an associated-type
  approach, if the test/impl duplication overhead exceeds the
  per-mechanic clarity benefit. **Trigger to revisit:** if a fourth
  mechanic (gRPC health-check, custom-script-with-different-cgroup-
  scoping, …) lands and the unified-trait alternative becomes
  structurally compelling, OR if observed maintenance friction on
  the three parallel test suites becomes a recurring drag in
  PR review.

### Quality-attribute impact

| Attribute | Impact |
|---|---|
| Performance — time behavior (probe latency) | p99 TCP probe ≤ timeout; p99 HTTP probe ≤ timeout (production adapter SLO); per-tick CPU ≤ K2 guardrail |
| Reliability — fault isolation | Per-probe-task isolation; cancellation tokens guarantee bounded shutdown |
| Maintainability — testability | Three small port traits; each has its own DST equivalence harness |
| Maintainability — modifiability | New mechanic (e.g. gRPC, Phase 3+) is a new port trait + new adapter, no changes to existing mechanics |
| Functional correctness | LWW per `(alloc_id, probe_idx)` makes the row shape unambiguous; no merge logic to get wrong |

## Cross-references

- ADR-0029 — overdrive-worker crate; this ADR extends it
- ADR-0030 — exec driver pattern; ExecProber reuses cgroup-placement
  machinery per ADR-0059
- ADR-0026 — cgroup v2 direct writes; ExecProber writes through
  same path
- ADR-0035 — reconciler runtime; this ADR specifies the
  observation-producer side
- ADR-0048 — rkyv envelope; ProbeResultRow ships as V1 per the
  procedure
- ADR-0055 — ServiceLifecycleReconciler; the consumer of ProbeResultRow
- ADR-0057 — `[[health_check.*]]` TOML spec
- ADR-0058 — default TCP-connect startup probe inference
- ADR-0059 — exec-probe cgroup placement mechanism
- `feature-delta.md` § "Wave: DISCUSS / HOW Risks surfaced to DESIGN wave"
  — Risks 1, 2, 3
- `.claude/rules/development.md` § "Reconciler I/O", § "Port-trait
  dependencies", § "Trait definitions specify behavior, not just
  signature", § "Production code is not shaped by simulation"
- Research § 3.3 D5, § 6.1, § 7.2 (Kubernetes prober.Manager shape)

## Changelog

- 2026-05-24 — Initial accepted version. Resolves P1-Q1 from
  `docs/feature/service-health-check-probes/feature-delta.md`.
