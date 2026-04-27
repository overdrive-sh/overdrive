# ADR-0025 — Single-node startup wiring: hostname-derived `NodeId` with optional config override; one-shot `node_health` write at boot

## Status

Accepted. 2026-04-27. Decision-makers: Morgan (proposing), user
ratification 2026-04-27. Tags: phase-1, first-workload,
application-arch.

## Context

Phase 1 is **single-node** per the DISCUSS-wave 2026-04-27 scope
correction. Control plane and worker run co-located on one
machine. There is exactly one node — the local host — and it is
implicit. No operator-facing node-registration verb exists; no
`POST /v1/nodes` handler will land in Phase 1.

But the §18 reconciler primitive needs the local node to *exist*
in observation: the `JobLifecycle` reconciler calls
`overdrive_scheduler::schedule(nodes, …)` (ADR-0024), which
expects a non-empty `BTreeMap<NodeId, Node>`. The
`SchedulerRespectsNodeCapacity`-shaped invariants and the
`alloc_status` rendering in `overdrive alloc status` both depend
on a real `NodeId` being present in the system.

Two startup-wiring decisions follow:

1. **What is the single node's `NodeId`?** If the value is wrong
   (collides with a future Phase 2+ multi-node deployment, or
   leaks operator-private data into telemetry), the migration
   pain compounds.
2. **Where is its row written?** The DISCUSS wave Priority One
   item 5 recommended "server bootstrap writes one row keyed by a
   deterministic local NodeId." This ADR fills in the exact
   mechanism.

## Decision

### 1. `NodeId` derivation: hostname fallback with optional `[node].id` override

```toml
# /etc/overdrive/config.toml or ~/.overdrive/config.toml
[node]
id = "prod-eu-west-1-a"     # OPTIONAL — overrides the default
```

The `ServerConfig` struct gains an optional `node_id: Option<NodeId>`
field (populated from a parsed operator config TOML if present,
default `None`). At server startup, the `NodeId` is resolved as:

```rust
let node_id: NodeId = match config.node_id {
    Some(id) => id,
    None     => NodeId::from_hostname()?,
};
```

`NodeId::from_hostname` is a new associated function on the
existing `NodeId` newtype:

```rust
impl NodeId {
    /// Derive a NodeId from `gethostname(3)`. Lowercases and
    /// validates against the existing NodeId character set; on
    /// non-conformant hostnames, returns
    /// `IdError::HostnameNotValidNodeId { raw }`.
    pub fn from_hostname() -> Result<Self, IdError> { … }
}
```

Hostname is read once at boot, lowercased, and passed through
`NodeId::new(...)` for validation. Hostnames with characters
outside the `NodeId` accept set (e.g. underscores in some Linux
distros' default hostnames) fail with an actionable error
naming both the raw hostname and the override mechanism:

```
Error: hostname "my_dev_box" cannot be used as a NodeId
       (NodeId requires [a-z0-9-], got '_').

Set [node].id explicitly in ~/.overdrive/config.toml to override.
```

The override path is the operator's escape hatch: any deployment
where the hostname is operator-meaningful and stable
(Kubernetes-style replicaset names, AWS instance IDs, systemd
machine-IDs) sets it explicitly. Most developer machines have
acceptable hostnames and rely on the default.

### 2. Region: `Region("local")` default with optional `[node].region` override

```toml
[node]
region = "us-east-1"        # OPTIONAL — defaults to "local"
```

Phase 1 has no genuine multi-region semantics, but the `Node`
aggregate carries a `Region` field per ADR-0011. The default
`Region("local")` is the honest single-node value: it is not a
real geographic region; it is a placeholder that names the
single-node case explicitly. A future multi-region deployment
overrides it via config.

The override path mirrors the `node_id` override exactly: the
`ServerConfig` carries `region: Option<Region>`; at boot it
resolves to the configured value or `Region::new("local")`
deterministically.

### 3. Boot-time `node_health` write: one-shot, before the listener binds

Server bootstrap inside `run_server_with_obs_and_driver`
(ADR-0022) writes one `node_health` row before the HTTPS
listener is bound:

```
1. Mint ephemeral CA + leaf certs                        (existing)
2. Open LocalIntentStore                                 (existing)
3. Resolve NodeId, Region from config                    (NEW — this ADR)
4. Resolve Resources (capacity) from config              (NEW — this ADR)
5. Write one node_health row to ObservationStore         (NEW — this ADR)
6. Construct ReconcilerRuntime, register reconcilers     (existing + ADR-0023)
7. Construct Driver                                      (ADR-0022)
8. Build AppState, Router                                (existing)
9. Bind TCP listener                                     (existing)
10. Write trust triple to operator-config dir            (existing)
11. Spawn axum_server task; return ServerHandle          (existing)
```

The `node_health` row shape (per ADR-0011 / ADR-0012):

```
{
    node_id:        <resolved>,
    region:         <resolved>,
    last_heartbeat: tick.now,                  // boot time
    capacity:       <from config or detected>, // see §4 below
}
```

Step 5's failure shape is `ControlPlaneError::Internal`, exactly
like the LocalIntentStore::open failure shape. A failed
`node_health` write means the observation store is broken; the
server cannot serve requests against a broken observation store
(handler GETs would fail), so refusing to start is the right
choice.

The write is **one-shot**: after this initial write, neither the
server nor any reconciler updates the row in Phase 1. Phase 2+
will introduce a heartbeat reconciler (per whitepaper §18 § Built-
in reconcilers) that updates `last_heartbeat` on a cadence; Phase
1's static row is a placeholder for that reconciler's future
target.

### 4. Capacity discovery: configured value, fallback to "all-ones" sentinel

```toml
[node]
cpu_milli     = 4000          # OPTIONAL — defaults to detected/sentinel
memory_bytes  = 8589934592    # OPTIONAL — same
```

The local node's declared capacity is read from the operator
config when present. When absent, Phase 1 falls back to a
deliberately-conservative sentinel:

```rust
const PHASE1_DEFAULT_CAPACITY: Resources = Resources {
    cpu_milli:    1_000_000,           // 1000 cores — way over-provisioned
    memory_bytes: 1024 * 1024 * 1024 * 1024,  // 1 TiB
};
```

The sentinel exists for a single reason: the Phase 1 scheduler
(ADR-0024) needs *some* capacity envelope to compute "free
capacity" against. Real capacity discovery (cgroup-aware
introspection of CPU shares, memory limits, NUMA topology) is
genuinely out of Phase 1 scope — whitepaper §14 right-sizing is
Phase 2+ territory. The sentinel is honest about this: it
declares "Phase 1 single-node, capacity bounding is operator-
configured if present, otherwise effectively unlimited so the
scheduler can never reject placement on capacity."

The `cgroup-aware-detection` story lands in Phase 2+ alongside the
right-sizing reconciler; Phase 1 first-workload ships the
config-or-sentinel pattern.

### 5. Operator config: TOML, in `[node]` block

The new `node_id`, `region`, `cpu_milli`, `memory_bytes` fields
live under a `[node]` table in the operator config TOML
(ADR-0019). The existing trust-triple TOML structure extends
additively; existing configs without a `[node]` block continue to
work (every field is `Option<…>`; the resolution path falls back
to defaults).

```toml
endpoint = "https://127.0.0.1:7001"

[ca]
cert = "MIIB…"

[client]
cert = "MIIB…"
key  = "MIIE…"

[node]                      # NEW — all fields optional
id            = "prod-eu-west-1-a"
region        = "eu-west-1"
cpu_milli     = 4000
memory_bytes  = 8589934592
```

The trust triple is written by the server at boot per ADR-0010;
the `[node]` block is **not** rewritten by the server — it is
operator-owned. Servers READ the `[node]` block at startup and
NEVER write to it. This preserves the principle that the
operator config's identity material is server-managed and the
operator's Phase 1 escape hatches are operator-managed.

## Alternatives considered

### Alternative A — Hardcoded `NodeId::new("local")`

Always use the literal string `"local"` as the single node's id;
no config, no hostname, no override.

**Rejected.** The Phase 1 scope is single-node, but the system
will run on multiple operator machines simultaneously across the
project's lifetime — developer laptops, CI runners, integration
test VMs, demo deployments. A hardcoded `"local"` collides
across these environments when telemetry is aggregated, and
provides no path forward when Phase 2+ adds multi-node deployment
where the operator wants their nodes named meaningfully.
Hostname-derived defaults are the lowest-friction path that
remains correct in multi-environment use.

### Alternative B — Random UUID at first boot, persisted to config

On first boot, mint a fresh UUID, store it as the NodeId in the
operator config TOML. Subsequent boots read the persisted value.

**Rejected.** Persisting platform-managed state to the operator
config muddies the ownership boundary established for the trust
triple (ADR-0010): the trust triple is *cert material that
expires*; the NodeId is *identity that should be stable across
process restarts and machine moves*. Different rotation
semantics. Embedding a server-minted UUID in the operator config
also makes telemetry obscure — operators reading their own logs
see `NodeId("3f7a-…-c1b2")` instead of their hostname. Hostname-
derived defaults are operator-readable from day one; UUIDs are
opaque.

The UUID story may be the right answer in Phase 5+ when operator
identity gets cryptographic backing (operator SVIDs per
whitepaper §8); at that point the NodeId would carry SPIFFE-shape
binding to the trust root. Phase 1 is too early for that
machinery.

### Alternative C — Detect capacity from cgroup limits at boot

Read `/sys/fs/cgroup/.../cpu.max`, `memory.max`, and
`cpuset.cpus.effective` at boot; populate `Resources` from the
detected values.

**Rejected for Phase 1.** Capacity detection is genuinely Phase
2+ territory: the right-sizing reconciler (whitepaper §14 +
ADR-0026) reads cgroup pressure signals continuously and needs
its own discovery path. Doing partial detection in Phase 1
fragments the responsibility (boot-time detection here +
runtime-time right-sizing there) and locks in detection logic
that the Phase 2+ ADR may need to revisit. The Phase 1 sentinel
+ operator-config pattern keeps the boundary clean: Phase 1 owns
config-or-sentinel; Phase 2+ owns runtime detection + right-
sizing as one coherent subsystem.

### Alternative D — No `node_health` row in Phase 1

Defer the `node_health` write entirely. Phase 1 single-node has
exactly one node; the scheduler can be given a synthetic
single-element map at call time without going through
observation.

**Rejected.** This breaks the §18 architectural invariant:
"every node writes its own rows" (whitepaper §4). The
`node_health` row is the canonical answer to "does this node
exist?" — and the answer can be either "yes, at this row" or
"no, no row." Skipping the write means every consumer
(the lifecycle reconciler's hydrate path, the future
`NodeList` handler, future Phase 2+ aggregation) needs a
special case for "Phase 1 single-node bypass." That special
case never goes away cleanly. Writing the row honestly at boot
gives every consumer one shape, and Phase 2+ multi-node simply
adds more rows — no special case to retire.

### Alternative E — `[node]` config block is required

Fail at boot if `[node].id` is not set. No hostname-derived
fallback.

**Rejected on user pre-decision (D5).** Default-on-hostname is
the correct ergonomic floor for Phase 1 dev usage. Operators
running on machines with valid hostnames (the overwhelming
majority of dev / CI / demo cases) get sensible defaults; the
escape hatch is one config-line. Requiring explicit
configuration adds friction for the no-friction case. The hard
refusal pattern is reserved for cgroup delegation (ADR-0028),
where the safety property genuinely cannot be defaulted.

## Consequences

### Positive

- **The single-node case is honest, not special-cased.** One
  node row in observation; one NodeId; one Region. Every
  downstream consumer reads observation; no Phase 1 bypass
  paths.
- **Phase 2+ multi-node is additive.** New nodes write their
  own rows; the existing single-node row continues to exist
  and remains the local node's health entry. No migration
  shape; no retiring of Phase 1-only code.
- **Operator escape hatches are explicit.** When the hostname
  is wrong (CI runners with synthetic names, dev VMs with
  underscore hostnames, dual-stack hosts), the config override
  is one line. When the default capacity sentinel is wrong
  (anyone wanting actual capacity bounds), the config override
  is two lines.
- **The trust-triple / operator-config separation is clean.**
  `[ca]`, `[client]`, `endpoint` are server-managed; `[node]`
  is operator-managed. The server reads `[node]`, never writes
  it.
- **Boot-time row write surfaces observation-store failures
  early.** If the observation store is broken, the server
  refuses to start instead of starting and serving 500s on
  every observation read.

### Negative

- **`NodeId::from_hostname` adds a `gethostname` dep on
  `overdrive-core`.** Or — more accurately — pulls `hostname`
  workspace dep into the `core`-class crate's compile graph. The
  `hostname` crate is pure-Rust and `dst-lint`-safe (no
  banned APIs); the dep edge is mechanical. Acceptable.
- **The Phase 1 default capacity sentinel is dishonest in
  one specific way: the scheduler will *never* reject on
  capacity in default-config dev usage.** The 1 TB / 1000 core
  sentinel is so far above reality that any submitted job
  fits. This is the Phase 1 single-node single-replica
  default scenario; capacity rejection is not a Phase 1
  scheduler invariant the platform actually exercises.
  Operators who want capacity bounds set them in config.
- **Hostname-derived NodeIds collide on hosts with the same
  hostname.** Two laptops both named `mbp` produce the same
  NodeId. This is a collision *between* deployments, not
  *within* one. Phase 2+ multi-node will need
  uniqueness-checking at admission (every Phase 2+ node
  registration verifies its NodeId is not already taken in
  Raft). Phase 1 single-node has only one row regardless of
  how it is named; no in-deployment collision.

### Quality-attribute impact

- **Maintainability — modifiability**: positive. The config
  block grows additively as Phase 2+ adds node-shape needs.
- **Maintainability — operability**: positive. Hostname is
  the most operator-readable default; explicit override is
  always available.
- **Reliability — fault tolerance**: positive. Boot-time
  observation write surfaces store breakage early.
- **Security — accountability**: neutral. The NodeId is a
  routing-shape identifier in Phase 1; whitepaper §8 SPIFFE-
  identity binding lands in Phase 5+.
- **Performance — time behaviour**: neutral. One additional
  observation write at boot; sub-millisecond for
  `LocalObservationStore`.

### Migration

There is no Phase 0 or external code to migrate. The
`Cargo.toml` of `overdrive-control-plane` may need to add the
`hostname` dep (already in workspace deps per the existing
`Cargo.toml`); `NodeId::from_hostname` is a new associated
function on the existing newtype.

## Compliance

- **ADR-0011** (`Node` aggregate intent-side): the boot-time
  `node_health` write is observation, not intent. The `Node`
  aggregate itself is NOT minted at Phase 1 boot — only the
  observation row. Intent-side `Node` aggregates land when
  multi-node deployment surfaces a use case. The boundary is
  preserved.
- **ADR-0012** (`LocalObservationStore` redb-backed): the
  boot-time write goes through `ObservationStore::write`,
  exactly the same path the future Phase 2+ heartbeat reconciler
  will use. No special-case write API.
- **ADR-0019** (operator config TOML): `[node]` is a new
  optional table. No format change; additive extension.
- **ADR-0010** (TLS bootstrap): the trust triple is written
  AFTER step 5 (per the existing Step ordering). The new
  `node_health` write at step 5 happens BEFORE bind, so it
  fails fast under broken observation. Compatible with the
  existing boot ordering.
- **`development.md` newtypes-STRICT**: `NodeId::from_hostname`
  routes through the existing validating constructor; no
  bypass.
- **DST replay**: the boot path is deterministic given
  `(config, hostname, clock.now)` — `SimClock` deterministic
  in DST; `hostname` is read once and stable for the
  process lifetime; config is data. Replay shape preserved.

## References

- ADR-0010 — TLS bootstrap; operator config TOML structure.
- ADR-0011 — Job/Node/Allocation aggregates intent layer.
- ADR-0012 — `LocalObservationStore` redb-backed observation
  store.
- ADR-0019 — Operator config format TOML.
- ADR-0026 — cgroup v2 direct writes (companion ADR for the
  control-plane slice resource declarations).
- Whitepaper §4 — Workload isolation on co-located nodes.
- Whitepaper §14 — Right-sizing; the Phase 2+ home for real
  capacity detection.
- `docs/feature/phase-1-first-workload/discuss/wave-decisions.md`
  — Priority One item 5 enumerates the decision.
- `docs/feature/phase-1-first-workload/discuss/user-stories.md`
  — US-04 calls out the single-node rendering.
- `crates/overdrive-core/src/id.rs` — current `NodeId` newtype.

## Amendment 2026-04-27 — Worker Crate Extraction

Decision §3 above is amended: the `node_health` row writer **moves
from "server bootstrap writes the row" to "worker subsystem startup
writes the row"** per ADR-0029. The original framing in §3 read:

> Server bootstrap inside `run_server_with_obs_and_driver`
> (ADR-0022) writes one `node_health` row before the HTTPS listener
> is bound

with the relocated step `5. Write one node_health row to
ObservationStore` listed inside the control-plane bootstrap
sequence.

**The relocation:** the `node_health` row writer is a
worker-subsystem responsibility, not a control-plane-bootstrap
responsibility. The reasoning is whitepaper §3-§5 directly: the
*worker* is what represents the node's runtime presence on a
machine — the worker is what runs allocations on that node, what
holds the cgroup hierarchy for that node's workloads, and what
should therefore own the `node_health` row that says "this node
exists, this is its capacity, this is its last heartbeat." Phase 2+
multi-node has a worker on each node writing its own row; making
the writer a worker-subsystem concern in Phase 1 means the Phase
2+ multi-node case is identical (one worker per node, each writing
its own row), not a special case to retire.

In Phase 1 single-node `role = "control-plane+worker"`, the worker
subsystem boots alongside the control plane in the same `overdrive
serve` process and writes its row at its own startup, before the
control-plane bootstrap binds the listener. Phase 2+ dedicated
worker nodes (`role = "worker"`) write their rows at worker
startup with no control-plane co-located.

**What is unchanged:** every other detail of this ADR. The
hostname-fallback rule for `NodeId` resolution, the optional
`[node].id` config-override path, the `Region("local")` default
with `[node].region` override, the capacity sentinel + override
mechanism, the `[node]` config block being operator-owned (server
reads, never writes), the actionable error messages, the failure
disposition (worker startup refuses to proceed if the
`ObservationStore` is broken — same shape as the control-plane's
prior refusal). The trust-triple separation from `[node]` is
preserved.

**The boot sequence relocates accordingly.** The worker subsystem
runs its own `node_health` row write as part of its startup, before
declaring "worker started." The composition root in
`overdrive-cli::serve` orchestrates: cgroup pre-flight → CA + TLS
mint → IntentStore open → worker subsystem startup (which writes
`node_health` and constructs `Driver`) → control-plane subsystem
startup (which receives `Arc<dyn Driver>` from the worker and
builds `AppState`) → bind listener → spawn axum_server. Path-shape
of the boot ordering stays "the worker's row write happens before
the control plane binds its listener," matching the original ADR's
fail-fast intent.

See ADR-0029 for the extraction rationale and the binary-composition
pattern.

