# Journey A — Submit a Service (long-running)

> **Workload kind**: `Service` — long-running, restart-on-exit. Reaches `Running` and
> stays there. Exit is always a failure event. Equivalent to k8s Deployment, Nomad
> `service`, Cloud Run service.
>
> **Persona**: Ana, Overdrive platform engineer (matches `submit-a-job.yaml`).
>
> **Trace**: J-OPS-002 (primary), J-OPS-003 (secondary).

## Changed Assumptions

- 2026-05-10 — folded in GH #164 (service listener spec shape). The Service
  TOML now carries one or more `[[listener]]` blocks declaring `(port,
  protocol, vip?)`. Submit echo and `alloc status` gain Listeners sections.
  When `vip = None`, the runtime allocator referenced is
  [overdrive-sh/overdrive#167](https://github.com/overdrive-sh/overdrive/issues/167)
  (approved 2026-05-09); the spec layer ships the `Option`-shaped field
  forward-compatibly. Emotional arc updated to acknowledge the new
  "listeners declared but vip pending" experience — see arc below.

## Emotional arc

```
Skeptical → Focused → Confident → Patient-but-Trusting
   "is the    "the     "the         "the CLI told me
    spec      parser   reconciler    'running' AND
    valid?"   accepted  said yes"    listed my listeners,
              it (with                 even though one VIP
              listeners)"              is still pending #167"
```

The arc end-state is **Patient-but-Trusting** rather than purely Trusting
because a Service with `vip = None` listeners has not yet completed allocation.
The CLI's honesty about "(vip: pending allocation — see #167)" is what
preserves trust during the wait — silence about pending state would have
collapsed the arc into "Confused." Per UX emotional-design heuristics: name
the deferral, name the issue, give the operator a place to look.

This journey's emotional arc must NOT regress from today's behaviour. The current Phase 1
shape gets the Service case right by accident (a long-running process never exits within
the streaming window). The new explicit `[service]` kind makes that correctness intentional;
listeners make it complete.

## ASCII flow

```
   STEP 1: Author spec       STEP 2: Submit         STEP 3: Stream    STEP 4: Confidence
   ----------------------    -----------------      --------------    --------------------
   Edit payments.toml         overdrive job          NDJSON events    overdrive alloc status
                              submit ./payments      → Accepted       → state: Running
   [service]                  .toml                  → Pending        → kind: Service
   id = "payments"                                   → ConvergedRunning replicas: 1/1
   replicas = 1                                                       restart_count: 0
                                                                       since: 2.3s ago
   [exec]
   command = "/usr/bin/...
   args = [...]

   [resources]
   cpu_milli = 500
   memory_bytes = ...

   FEELS: Skeptical          FEELS: Focused         FEELS: Confident   FEELS: Trusting
   ARTIFACT: ${spec_path}    ARTIFACT: ${kind}      ARTIFACT:          ARTIFACT:
                             = Service              ${converged_       ${alloc_status_kind}
                                                     running_event}    = Service
```

## TUI mockups

### Step 1 — Author the spec (with listeners)

```
$ cat ./payments.toml
+----------------------------------------------------------------+
| [service]                                                       |
| id = "payments"                                                 |
| replicas = 1                                                    |
|                                                                 |
| [[listener]]                                                    |
| port     = 8080                                                 |
| protocol = "tcp"                                                |
| vip      = "10.0.0.1"                                           |
|                                                                 |
| [[listener]]                                                    |
| port     = 8081                                                 |
| protocol = "udp"                                                |
|                                                                 |
| [exec]                                                          |
| command = "/usr/local/bin/payments-server"                      |
| args = ["--port", "8080"]                                       |
|                                                                 |
| [resources]                                                     |
| cpu_milli = 500                                                 |
| memory_bytes = 268435456                                        |
+----------------------------------------------------------------+
```

### Step 2 — Submit

```
$ overdrive job submit ./payments.toml
+----------------------------------------------------------------+
| Submitting service 'payments' (kind=Service, replicas=1)       |
| Spec digest: sha256:a4c1...e9                                   |
| Endpoint:    https://127.0.0.1:7001/                            |
| Listeners:                                                      |
|   10.0.0.1:8080/tcp                                             |
|   (vip: pending allocation - see #167):8081/udp                 |
+----------------------------------------------------------------+
```

> **Note on `(vip: pending allocation — see #167)`**: when `vip` is omitted
> in the TOML, the spec layer carries `None` forward and the CLI renders an
> explicit pending marker referencing
> [overdrive-sh/overdrive#167](https://github.com/overdrive-sh/overdrive/issues/167).
> The runtime allocator behaviour (allocate vs. reject-at-admission) is #167's
> decision to land; the spec layer is forward-compatible with either outcome.

### Step 3 — Streaming convergence (success)

```
+----------------------------------------------------------------+
| Service 'payments' is running with 1/1 replicas (took 1.4s)   |
+----------------------------------------------------------------+
```

### Step 3' — Streaming convergence (Service failure within stability window)

```
+----------------------------------------------------------------+
| Service 'payments' failed to stabilise.                        |
|                                                                 |
|   alloc 'payments-0' exited with code 1 within 0.2s of start.  |
|   Restart attempts: 1 of 5. Next attempt in ~0.5s.             |
|                                                                 |
|   Run `overdrive alloc status --job payments` for full state.  |
+----------------------------------------------------------------+
```

> Note: this is an evolution of the current `ConvergedFailed` arm; no change in shape, only
> in vocabulary (the line says `Service` not `Job`).

### Step 4 — Confidence via `alloc status`

```
$ overdrive alloc status --job payments
+----------------------------------------------------------------+
| Job:    payments    (kind: Service)                            |
| Spec:   sha256:a4c1...e9                                        |
| Replicas (desired/running): 1/1                                |
| Listeners:                                                      |
|   10.0.0.1:8080/tcp                                             |
|   (vip: pending allocation - see #167):8081/udp                 |
|                                                                 |
| Alloc                  State    Restarts  Since                |
| ---------------------- -------- --------- ----------           |
| payments-0             Running  0         00:00:02.3            |
+----------------------------------------------------------------+
```

> The Listeners section in `alloc status` byte-equals the section printed in
> the submit echo for the same spec. Round-trip integrity is asserted by KPI
> K6.

## Failure modes

- Spec carries `[service]` AND `[job]` — parser rejects with "exactly one of [service] /
  [job] required".
- Spec carries `[service]` AND `[schedule]` — parser rejects with "[schedule] is only
  valid alongside [job], not [service]".
- Spec missing `[exec]` — parser rejects with "[exec] block is required for all
  workloads".
- Spec carries `[service]` with zero `[[listener]]` blocks — parser rejects with
  "a [service] requires at least one [[listener]] block".
- Two `[[listener]]` blocks share the same `(vip, port, protocol)` triple — parser
  rejects naming the duplicate.
- A `[[listener]]` declares `port = 0` — parser rejects with "port must be in 1..=65535".
- A `[[listener]]` declares an unsupported protocol (e.g. `sctp`, `icmp`) — parser
  rejects with "supported protocols: tcp, udp".
- Service crashes within the streaming window — current behaviour preserved (eventually
  emits `ConvergedFailed`); kind-aware vocabulary in the render line.

## Shared artifacts touchpoints

- `${spec_path}` — operator-supplied; consumed by parser, by submit RPC, by error messages.
- `${kind}` — derived from section presence; flows from parser → `WorkloadSpec` enum → RPC
  variant → `AllocStatusRow.kind` field → CLI render branch. Single source of truth: the
  parsed `WorkloadSpec` enum at the spec module boundary.
- `${spec_digest}` — `ContentHash::of(rkyv archive)`; computable locally and on server.
- `${endpoint}` — read from `ApiClient::base_url()` per `crates/overdrive-cli/CLAUDE.md`.
- `${listener_triple}` — `Vec<Listener>` on the Service spec; flows from operator TOML →
  parser → submit echo → `alloc status` Listeners section. Round-trip byte-identical.
- `${vip_assignment_state}` — `Option<ServiceVip>` per listener. `Some(addr)` renders the
  IPv4 literal; `None` renders the literal `(vip: pending allocation — see #167)`.

## Integration checkpoints

- After parser returns: `kind == Service` branch is the ONLY path that can reach the
  `format_running_summary` call site. (Job and Schedule kinds use different render
  functions.)
- Streaming protocol's `ServiceSubmitEvent::ConvergedRunning` is the only event type that
  can fire on this code path. A `Job`-kind submit cannot produce a `ConvergedRunning`
  event because the `JobSubmitEvent` enum does not carry that variant.
- `AllocStatusRow.kind` is denormalised onto the row at write time; it never disagrees
  with the spec's declared kind.

## Cross-references

- `journey-submit-service.yaml` — schema-form
- `journey-submit-service.feature` — Gherkin scenarios
- `shared-artifacts-registry.md` — artifact tracking across all four journeys
