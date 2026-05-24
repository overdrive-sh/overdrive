# ADR-0057 — `[[health_check.startup|readiness|liveness]]` TOML spec; Service-kind only; defaults aligned with Kubernetes idioms

## Status

Accepted. 2026-05-24. Decision-makers: Morgan (proposing); DESIGN-wave
output of `docs/feature/service-health-check-probes/`.

Tags: phase-1, service-kind, operator-config, toml-spec.

**Amends**: ADR-0019 (operator config format TOML), ADR-0050
(intent-side workload aggregate). **Companion ADRs**: ADR-0054
(ProbeRunner), ADR-0055 (ServiceLifecycleReconciler), ADR-0058
(default-probe inference).

## Context

The operator declares health-check intent in TOML; the parser must
accept structured probe specs, infer defaults, validate at parse
time, and feed `ServiceSpec` aggregate (per ADR-0050) into the
intent store. Per `feature-delta.md` US-02 / US-03 / US-04 / US-05 /
US-07.

Open questions resolved here (P2-Q4):

- What are the per-field defaults (timeout, interval, max_attempts,
  failure_threshold, success_threshold)?
- What is the TOML shape for each mechanic (HTTP / TCP / Exec)?
- How does kind rejection work for `[job]` / `[schedule]`?

Industry alignment (research § 2.1): Kubernetes defaults are
`timeoutSeconds=1`, `periodSeconds=10`, `failureThreshold=3`,
`successThreshold=1`. Overdrive Phase 1 aligns where it matches
operator mental model; diverges where Phase 1 single-node
single-replica properties differ.

## Decision

### 1. TOML shape

```toml
# Service kind (sibling to [exec], [resources])
[service]
id = "payments"
replicas = 1

[[listener]]
port = 8080

[exec]
command = ["/usr/local/bin/payments-server"]

# Health checks — array of tables, role nested under `health_check`
# Empty array `[[health_check.startup]] = []` is explicit opt-out
# (preserves no-probe semantics, per C4 default-policy rule).

[[health_check.startup]]
type = "tcp"
port = 8080
timeout_seconds = 5      # default 5
interval_seconds = 2     # default 2
max_attempts = 30        # default 30 → startup_deadline = 60s

[[health_check.startup]]
type = "http"
path = "/healthz"
port = 8080
timeout_seconds = 5
interval_seconds = 2
max_attempts = 30

[[health_check.readiness]]
type = "http"
path = "/readyz"
port = 8080
timeout_seconds = 5
interval_seconds = 2
success_threshold = 1    # default 1 (matches K8s)
failure_threshold = 1    # default 1 (readiness only; readiness has no max_attempts)

[[health_check.liveness]]
type = "exec"
command = ["/usr/local/bin/healthcheck.sh", "--strict"]
timeout_seconds = 5
interval_seconds = 10    # liveness slower than readiness by default
failure_threshold = 3    # default 3 (matches K8s)
```

### 2. Defaults table (P2-Q4 resolution)

| Field | Startup | Readiness | Liveness | K8s default | Overdrive default | Rationale |
|---|---|---|---|---|---|---|
| `timeout_seconds` | required | required | required | 1 | **5** | Phase 1 absorbs HTTP slow path; 5s aligns with operator iteration; matches research § 2.1 commonly-overridden value |
| `interval_seconds` | required | required | required | 10 | **2** (startup), **2** (readiness), **10** (liveness) | Startup short for fast detection; readiness short for low traffic-removal latency; liveness slow to avoid restart-storm pressure |
| `max_attempts` | required | n/a | n/a | n/a | **30** | Startup-only; with 2s interval → 60s startup_deadline; matches K8s `failureThreshold × periodSeconds` math |
| `failure_threshold` | n/a | required | required | 3 | **1** (readiness), **3** (liveness) | Readiness: ejection latency = 1 tick (US-04 AC); liveness: 3-strike matches K8s and operator habit |
| `success_threshold` | n/a | required | n/a | 1 | **1** | P2-Q8: matches K8s default; configurable upward |

Divergence from K8s (each justified):

- **`timeout_seconds = 5` vs K8s `1`**: K8s's 1s default is widely
  considered too tight; research § 2.1 names it a common cause of
  false positives. Phase 1 chooses 5s as the more honest default;
  operators with sub-second SLOs override.
- **`interval_seconds = 2` for startup/readiness vs K8s `10`**: Phase
  1 single-node makes the 2s interval cheap; cross-region multi-node
  with 100s of probes would change this calculation. Liveness keeps
  10s default to match K8s restart-storm posture.

The defaults table is **enforced by parser** (missing field →
`default`) and **documented in operator docs** as the single SSOT
for "what does this knob do?".

### 3. Validation rules

| Rule | Outcome |
|---|---|
| `type` value outside `tcp` / `http` / `exec` | `ParseError::UnknownProbeType { probe_idx, found }` |
| `tcp` without `port` | `ParseError::TcpProbeMissingPort { probe_idx }` |
| `http` without `path` | `ParseError::HttpProbeMissingPath { probe_idx }` |
| `http` without `port` | `ParseError::HttpProbeMissingPort { probe_idx }` |
| `exec` without non-empty `command` | `ParseError::ExecProbeMissingCommand { probe_idx }` |
| `timeout_seconds == 0` | `ParseError::ProbeTimeoutZero { probe_idx }` |
| `interval_seconds == 0` | `ParseError::ProbeIntervalZero { probe_idx }` |
| `max_attempts == 0` (startup only) | `ParseError::ProbeMaxAttemptsZero { probe_idx }` |
| `failure_threshold == 0` (readiness/liveness) | `ParseError::ProbeFailureThresholdZero { probe_idx }` |
| `success_threshold == 0` (readiness) | `ParseError::ProbeSuccessThresholdZero { probe_idx }` |
| HTTP `path` does not start with `/` | `ParseError::HttpProbePathNotAbsolute { probe_idx, found }` |
| **Probes on `[job]` (any role)** | `ParseError::ProbesNotAllowedOnKind { kind: "job", guidance: "Job has no readiness question; on completion is enough. Use exit code 0 to indicate success." }` |
| **Probes on `[schedule]`** | `ParseError::ProbesNotAllowedOnKind { kind: "schedule", guidance: "Schedule composes per-fire from its [job]; probes on Schedule are semantically meaningless." }` |
| Service with `replicas > 1` AND any liveness probe | accepted (Phase 1 single-replica, but parser permits — see C9) |

`probe_idx` is the 0-indexed position within the per-role array
(`[[health_check.startup]]` array). Inferred default probe
(ADR-0058) takes `probe_idx = 0`.

### 4. Kind-rejection — US-07 / Slice 07

The kind discriminator from ADR-0047 is the gate: parser walks the
TOML's top-level table presence; if `[[health_check.*]]` is present
AND the discriminator section is `[job]` or `[schedule]`, the
`ParseError::ProbesNotAllowedOnKind` fires with the kind-specific
guidance. Guidance text lives as a `const &'static str` per kind so
the CLI render + the OpenAPI error response carry identical text.

The rejection is structural — `ProbeDescriptor` does NOT appear on
`JobSpec` or `ScheduleSpec`; it appears only on `ServiceSpec`. A
future attempt to add probes to Job kind would require adding a
new field to `JobSpec` and a new validator; the parser rejection is
the single belt-and-suspenders gate.

### 5. ServiceSpec aggregate extension (per ADR-0050)

```rust
// crates/overdrive-core/src/aggregate/service_spec.rs (extension)

pub struct ServiceSpec {
    pub id: WorkloadId,
    pub replicas: NonZeroU32,
    pub resources: Resources,
    pub driver: WorkloadDriver,
    pub listeners: NonEmptyVec<Listener>,
    /// NEW per ADR-0057. Probe descriptors per role; outer Vec is
    /// 0-indexed `probe_idx`. Empty = explicit opt-out (no probes
    /// of this role). Missing role + no listener = no inference.
    pub startup_probes: Vec<ProbeDescriptor>,
    pub readiness_probes: Vec<ProbeDescriptor>,
    pub liveness_probes: Vec<ProbeDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, /* rkyv + serde */)]
pub struct ProbeDescriptor {
    pub idx: ProbeIdx,             // 0-indexed; parser-assigned
    pub role: ProbeRole,
    pub mechanic: ProbeMechanic,
    pub timeout: Duration,
    pub interval: Duration,
    pub max_attempts: Option<u32>,    // Some only when role == Startup
    pub failure_threshold: Option<u32>, // Some only when role != Startup
    pub success_threshold: Option<u32>, // Some only when role == Readiness
    pub inferred: bool,               // true iff Slice 01 default (ADR-0058)
}

pub enum ProbeMechanic {
    Tcp  { addr: SocketAddrV4 },
    Http { url: String, /* method GET fixed in Phase 1 */ },
    Exec { command: Vec<String> },
}
```

`ProbeDescriptor` is rkyv-archived as part of `ServiceSpec`. Per
ADR-0048 / ADR-0050 the `ServiceSpec` already lives inside a
versioned envelope; adding the three Vec fields is an additive
change requiring a `ServiceSpecEnvelope::V2` per ADR-0048's
"Version-bump procedure". The new envelope variant ships with a
`FIXTURE_V1` constant pinning the pre-feature shape so the
schema-evolution roundtrip remains intact.

### 6. Default-probe inference — see ADR-0058

When `startup_probes` is empty AND `listeners` is non-empty, the
parser synthesises a default TCP probe per ADR-0058 (separate ADR
because the inference rule has its own design rationale and
SemVer surface). Inferred descriptor has `idx = 0`, `inferred =
true`.

The parser distinguishes:

- **Absent `[[health_check.startup]]`** → infer default.
- **Empty `[[health_check.startup]] = []`** → explicit opt-out;
  `startup_probes = vec![]`; reconciler treats as "no startup gate";
  alloc reaches `Stable` immediately upon `Running` (preserves
  Phase 1 first-Running semantics for spec-by-spec compatibility).

This distinction is parser-level (TOML `Value::Array::is_some` vs
`Value::Array::is_empty`); the validated `ServiceSpec` carries the
same `Vec<ProbeDescriptor>` shape for both cases — the reconciler
treats an empty `startup_probes` as "Stable on first Running" by
construction.

## Considered alternatives

### Alternative A — `[probes]` table (singular, not array-of-tables)

```toml
[probes]
startup = [{ type = "tcp", port = 8080 }]
readiness = [{ type = "http", path = "/healthz" }]
```

Rejected: array-of-tables `[[health_check.startup]]` is more
idiomatic TOML and matches the existing `[[listener]]` convention
from ADR-0047 §1. Inline-table syntax is less operator-friendly for
multi-probe Services.

### Alternative B — K8s defaults verbatim

`timeout_seconds = 1`, `period_seconds = 10`, etc. Rejected per §2:
K8s 1s timeout is widely criticised as too tight; Overdrive chooses
5s as the more honest default. Operators with sub-second SLOs
override explicitly.

### Alternative C — `[[probe]]` flat, `role` field per entry

```toml
[[probe]]
role = "startup"
type = "tcp"
port = 8080
```

Rejected: flat list with role field requires re-derivation of
"all-startup-probes" everywhere; per-role array makes the role
structural. Matches K8s shape (`startupProbe` vs `readinessProbe`
vs `livenessProbe` as named fields).

### Alternative D — Per-probe `initial_delay_seconds` (K8s style)

Allow operators to specify `initial_delay_seconds` to delay first
probe tick. Rejected per `feature-delta.md` C12 — initial-delay is
deferred to Phase 2; Phase 1 probes immediately on alloc Running.
Adding it Phase 2+ is additive.

## Consequences

### Positive

- **Operator config matches k8s muscle memory** (per US-02 push
  force) while diverging only where defensible.
- **`ProbeDescriptor` aggregate field on `ServiceSpec` only** —
  Job / Schedule cannot represent probes structurally.
- **Parser-time validation surfaces typed errors** for every
  malformed-spec case; CLI renders named guidance.
- **Inferred-vs-explicit-opt-out distinction** preserves Phase 1
  spec compatibility while making the inference visible (ADR-0058).

### Negative

- **`ServiceSpecEnvelope::V2` bump** per ADR-0048's
  six-step procedure (additive Vec fields). Single-commit, fixture
  file added; existing fixtures untouched.
- **Defaults table is a SemVer surface**: a future change to defaults
  silently changes operator-visible behaviour. Convention: changes
  to defaults are documented as breaking and announced via the
  operator changelog. (Validated by integration test that pins each
  default value.)

### Quality-attribute impact

| Attribute | Impact |
|---|---|
| Usability — operator config | k8s shape + named guidance on error |
| Functional correctness | Parser-time validation prevents misshapen specs from reaching IntentStore |
| Compatibility — evolvability | `ServiceSpec` evolution via rkyv envelope V2 |
| Maintainability — modifiability | New probe mechanics land as additional `ProbeMechanic` variants |

## Cross-references

- ADR-0019 — operator config TOML; this ADR adds health-check sections
- ADR-0047 — kind discriminator; gates probe acceptance to Service
- ADR-0048 — rkyv envelope; `ServiceSpec` V2 bump
- ADR-0050 — intent-side workload aggregate; `ServiceSpec` lives here
- ADR-0054 — ProbeRunner consumes `ProbeDescriptor`
- ADR-0055 — ServiceLifecycleReconciler consumes parsed spec
- ADR-0058 — default-probe inference
- `feature-delta.md` P2-Q4
- Research § 2.1 (K8s defaults)

## Changelog

- 2026-05-24 — Initial accepted version. Resolves P2-Q4.
