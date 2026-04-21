# ADR-0007 — CR-SQLite deletion discipline: tombstones, bounded sweep, partition-rejoin refusal

## Status

Accepted. 2026-04-22.

## Context

The ObservationStore is CR-SQLite tables gossiped by Corrosion over SWIM+QUIC
(whitepaper §4, §17). Its consistency model is last-write-wins under logical
timestamps; writes converge eventually, and nodes hydrate kernel-visible state
(BPF maps, gateway route tables, scheduler inputs, investigation traces) from
their local SQLite replica of the gossiped tables.

CRDT stores of this shape have a well-characterised failure mode around
deletion. When a row is removed in place, a replica that was partitioned for
longer than the gossip propagation window can, on rejoin, replay its stale
copy as a newer observation under LWW and *resurrect* the deleted row
everywhere. The Antithesis report on Tigris Data
(https://antithesis.com/blog/2026/tigris_report/) documents the same class of
bug in a different implementation: delete-then-read resurrection, rename
inconsistency, partial-region failures re-exposing deleted objects. The bugs
were in normal code paths exercised by regular CI and only surfaced under
adversarial fault injection. Our ObservationStore is architecturally in the
same class.

The whitepaper's §4 Consistency Guardrails was amended on 2026-04-22 with a
new bullet — *Tombstones for deletion, with a bounded sweep window* — and a
matching *Tombstone sweep* entry in the §18 built-in reconciler list. That
amendment fixes the direction; this ADR records the operational rules that
make the scheme implementable: what the trait surface exposes, what the sweep
window is, how partition-rejoin refusal works, and which tables the discipline
applies to.

Prior art already lives in the codebase. §8 specifies a
`revoked_operator_certs` table with a reconciler that deletes rows whose
`expires_at` has passed. We need to decide whether that is a special case or
the general pattern; the §18 amendment implies generalisation, but the
semantics are not identical, and the difference is load-bearing.

## Decision

### 1. Delete is a tombstone write

The `ObservationStore::write` surface does not expose row deletion to
reconcilers, workflows, or node agents. Every gossiped (CR-SQLite CRR)
table that supports deletion carries two columns the writer sets directly:

```sql
deleted_at   INTEGER    -- logical timestamp of the tombstone write;
                        -- NULL when the row is live
tombstone    INTEGER    -- 0 = live, 1 = tombstone; indexed for sweep scans
```

A "delete" is a row mutation setting `tombstone = 1` and `deleted_at =
<logical_now>`. The row body is retained verbatim — tombstones carry the
full last-observed payload so a late-arriving reader converges to the same
terminal state as every other node. Full-row publication, per the §4
guardrail *Full rows over field diffs*, is the existing pattern; tombstones
follow it.

Readers filter `WHERE tombstone = 0` in subscriptions that materialise
kernel state. The filter is not optional. A `ReadView` wrapper around
`ObservationStore::subscribe` enforces this at the type level for BPF map
hydration paths, gateway route resolution, and scheduler input queries —
raw SQL access is limited to the sweeper and audit tooling. This is the
same shape as the `IntentStore`/`ObservationStore` trait separation in §4:
a type-level barrier, not a convention.

Rows whose lifecycle is purely state-transition (an allocation going
`pending → running → draining → terminated`) are **not** deleted. They
transition through the terminal state and remain queryable. Physical
removal of terminated-allocation rows is a separate compaction concern
with different retention semantics and is out of scope for this ADR.

### 2. Sweep window: `max_partition_duration × 3`, default 72 hours

The tombstone sweep reconciler reclaims rows where `tombstone = 1 AND
deleted_at < (logical_now − sweep_window)`. The window is the single
load-bearing parameter of the whole scheme — too short and a rejoining node
can resurrect deleted rows; too long and storage grows unboundedly.

Rather than pick a number unmoored from the rest of the system, the window
is derived:

```
sweep_window = max_partition_duration × safety_factor
            = 24h × 3
            = 72h
```

- **`max_partition_duration = 24h`** is the supported operational bound on
  how long a node may be partitioned from its region and still rejoin
  without operator intervention. Beyond this, manual re-bootstrap is
  required (see item 3). 24h covers single-day regional network
  incidents, cloud-provider control-plane outages in the longest
  post-mortems we have reference points for, and overnight partitions
  without operator escalation.
- **`safety_factor = 3`** absorbs logical-clock skew between peers,
  staggered sweep scheduling across nodes (sweeps do not run in lockstep),
  and the gossip-propagation tail — a row deleted at tick T on node A may
  not reach node B's local replica for seconds; node B's sweep decision
  must not race node A's tombstone arrival.

Operators override via cluster configuration:

```toml
[cluster.observation.tombstone]
max_partition_duration_hours = 24
safety_factor                = 3
# derived: sweep_window = 72h
```

The derived value is recorded in `node_health` on each node so a sweeper on
node A can refuse to reclaim if node B is announcing a longer window —
cluster-wide, the sweep window is the *maximum* of all announced windows.
A misconfigured node advertising a shorter window cannot shorten the
effective sweep horizon.

### 3. Partition-rejoin refusal

A node returning from a partition older than `sweep_window` is refused
rejoin and re-bootstrapped from a fresh regional snapshot.

**Signal.** The rejoin handler computes `gap = logical_now_peer −
node_health.last_heartbeat[self]` on first successful gossip exchange
after a partition heals. `last_heartbeat` is the logical timestamp of the
node's most recent `node_health` row as seen by the peer — authoritative
observation data, not local wall clock. Using logical time rather than
wall clock avoids both the "my clock drifted" and "my peer's clock drifted"
failure modes, and survives clock-skew fault injection under DST (§21).

If `gap > sweep_window`, the rejoining node's Corrosion peer refuses to
apply any of its buffered local writes to the cluster and emits a
structured `observation.rejoin.refused` event. The node continues running
read-only against its local replica (so ongoing workload traffic does not
immediately fail) but marks itself as *bootstrapping*.

**Bootstrap path.** The bootstrapping node:

1. Drops its local Corrosion SQLite.
2. Fetches a current snapshot from a healthy in-region peer via the same
   `IntentStore::bootstrap_from` contract used at initial provisioning
   (§17 — the contract is already general over snapshot format).
3. Re-hydrates BPF maps and gateway routes from the bootstrapped replica.
4. Re-joins gossip.

Buffered local writes from before the bootstrap are discarded. In practice
this means any `alloc_status` updates written by this node during the
partition are lost; the reconciler loop on the now-rejoined node
re-observes its workloads and re-emits status. Workloads themselves
continue running through the bootstrap; the refusal applies to gossip
participation, not workload execution.

This is the expensive half of the trade-off. A node partitioned for
longer than 72h cannot contribute its local observations back; the
assumption is that observations that old are no longer actionable, which
matches the reality of a 72h-partitioned node — its `alloc_status` rows
describe a world that has since moved on.

### 4. Per-table deletion mode

| Table (§4) | Deletion mode | Notes |
|---|---|---|
| `alloc_status` | State-transition, never deleted | Terminal state is `terminated`; row remains for audit. |
| `service_backends` | Tombstoned with sweep | Row removed when the backing allocation stops existing; BPF `SERVICE_MAP` hydration filters `tombstone = 0`. |
| `node_health` | Tombstoned with sweep | Decommissioning writes a tombstone; the sweep reclaims. A node absent from heartbeats is *stale*, not deleted. |
| `policy_verdicts` | Tombstoned with sweep | Verdict revocation (policy change, scope removal) writes a tombstone. |
| `revoked_operator_certs` | Tombstoned with `expires_at` sweep | See item 5. |
| `external_call_results` | Tombstoned with sweep | Result retained after reconciler consumes it; sweep reclaims after `sweep_window`. |
| `investigation_state` | State-transition, then tombstoned | Live investigations transition `triggered → … → concluded`; on conclusion the reconciler compresses to `incidents` libSQL and writes a tombstone on the `investigation_state` row. |

### 5. Relationship to `revoked_operator_certs`

The `revoked_operator_certs` sweep from §8 is a special case, not a
general one. Its sweep key is `expires_at` — the original TTL of the
revoked cert — not gossip-age. The semantics differ in a way that
matters:

- **Gossip-age sweep** (general): the tombstone is safe to reclaim once
  every live node has had time to converge, i.e. after `sweep_window`
  from `deleted_at`. The row's meaning expires with propagation.
- **`expires_at` sweep** (revocation): the tombstone is safe to reclaim
  once the underlying cert cannot be presented, i.e. after the original
  cert TTL. Gossip-age is irrelevant — a revocation of a 15-minute cert
  can be reclaimed in 15 minutes, and a revocation of a 24-hour cert
  must be retained for 24 hours, regardless of how long gossip took.

Both are implemented by the same `TombstoneSweeper` reconciler, which
dispatches on a per-table `SweepPolicy` enum:

```rust
enum SweepPolicy {
    GossipAge { window: Duration },          // sweep_window from cluster config
    ExpiresAt { column: &'static str },      // e.g. "expires_at" for revoked_operator_certs
    Never,                                   // alloc_status
}
```

Each CRR table registers its policy at schema declaration. The sweeper
iterates registered tables, applies the policy, and emits a single
`sweep.tombstone.reclaimed` counter per table-tick for observability.

This subsumes the existing §8 "revocation-sweep reconciler" — it is the
`ExpiresAt` case of the general sweeper, not a separate reconciler. The
§18 list should read *Tombstone sweep* with `revoked_operator_certs` as
one of its registered tables, not two parallel reconcilers. (This ADR
does not amend the whitepaper; the §18 bullet already uses the general
name.)

## Consequences

### Positive

- The Tigris-class delete-then-read resurrection bug becomes a named,
  tested scenario under DST rather than an emergent production incident.
- The trait-level barrier on the write API makes the failure shape
  *architectural* — a reconciler cannot issue an in-place delete because
  the API does not offer one.
- Sweep policy is per-table and extensible via the enum; adding a new
  CRR table forces a deliberate `SweepPolicy` choice at schema time.
- The partition-rejoin refusal aligns the observation layer with the
  intent layer's existing discipline — §4 already re-bootstraps a node
  whose Raft log is too far behind; gossip now has the symmetric rule.
- `revoked_operator_certs` stops being a one-off reconciler; the general
  shape covers the special case.

### Negative

- **Bounded extra storage per table.** Tombstoned rows carry their full
  payload for the sweep window. For a table with high churn, this is a
  real steady-state cost. `service_backends` in a cluster with many
  short-lived allocations is the canonical worst case. Storage impact is
  observable via the per-table `sweep.tombstone.reclaimed` counter and
  is expected to be a sizing input for operators running large fleets.
- **72-hour floor on partition recovery.** A node offline for more than
  72 hours cannot rejoin without operator-driven re-bootstrap. This is a
  hard constraint, not a tunable one below 24h — shortening
  `max_partition_duration` reduces the recovery window but tightens the
  sweep horizon, and below some threshold (empirically, a few hours)
  sweep racing with gossip becomes the dominant failure mode. The
  default is deliberately conservative.
- **Convergence window widens.** A node rejoining within the sweep window
  will not know about tombstones written during its absence until it
  receives them over gossip. Reads during this catch-up period may see
  the deleted row. The `ReadView` filter suppresses the *row*, but not
  the reality that other observations referencing it may still be
  inconsistent for seconds after rejoin. This is the eventual-consistency
  cost the architecture already accepts; the ADR does not widen it, but
  surfaces it explicitly.
- **Sweeper-gossip race is a named risk.** A sweep on node A running
  concurrently with a tombstone write from node B can reclaim a
  tombstone that node B has not yet propagated. The sweep-window
  safety factor (×3) is the mitigation; the DST scenario below gates
  the tuning.

### Neutral

- Schema migrations adding `deleted_at` and `tombstone` to existing CRR
  tables are additive per the §4 guardrail and follow the two-phase
  rollout already specified.

## Alternatives considered

### Option A — Hard deletion with "undelete" on rejoin

Delete in place; on rejoin, compare the rejoining node's local state
against the cluster and undo any resurrection. **Rejected.** The
comparison requires knowing what *should* have been deleted, which
requires a side channel — exactly the tombstone mechanism under a
different name, plus an additional reconciliation pass. The proposed
scheme is the simpler version of the same idea.

### Option B — Unbounded tombstone retention

Never sweep tombstones. Storage grows with cumulative delete volume,
which is wrong for high-churn tables (a cluster that provisions and
tears down one allocation per second across months). **Rejected.**
Tombstones that no live node can need are dead weight; the sweep window
is the mechanism for bounding that weight without re-introducing the
resurrection bug.

### Option C — Per-row TTL at the CRDT layer

Express deletion as a TTL column and let a CRDT-level process expire
rows. **Rejected.** cr-sqlite does not expose per-row TTL semantics in
its merge function; we would be implementing a parallel CRDT on top of
CR-SQLite. The tombstone-with-sweep pattern uses cr-sqlite's native
LWW semantics on the `tombstone` column — a tombstone arriving from a
peer is merged identically to any other row mutation, and the sweeper
is a plain SQL `DELETE` against the local replica (not gossiped).

### Option D — Global coordinator for tombstone GC

Elect a leader (or reuse the regional Raft leader) to orchestrate sweep
timing across the region. **Rejected.** Adds a failure mode (what
happens during leader turnover?), re-introduces a coordination
bottleneck the ObservationStore explicitly avoids (§4 principle: gossip
where it scales), and provides no additional safety beyond the
safety-factor approach — the sweep window is already chosen large
enough that uncoordinated sweeping is safe. Coordination would reduce
storage at the cost of complexity; the trade-off is wrong for this
layer.

### Option E — Tombstone-with-sweep + rejoin refusal (chosen)

See Decision above.

## Testing implications

Per `.claude/rules/testing.md`:

**Tier 1 (DST, `cargo xtask dst`).** Three named scenarios under
`crates/overdrive-sim/tests/dst/`:

- `tombstone_resurrection_refused.rs` — delete row, partition a node
  for `sweep_window + 1h`, heal, assert rejoining node's stale write
  does not resurrect the deleted row and that
  `observation.rejoin.refused` is emitted.
- `sweep_races_gossip.rs` — under injected gossip delay at the tail
  of `sweep_window`, assert the sweeper on any node never reclaims a
  tombstone that another node has not yet observed (invariant: "every
  tombstone reclaimed had propagated to every live node ≥ 1 tick
  earlier").
- `partition_within_window_converges.rs` — partition for
  `sweep_window − 1h`, heal, assert the rejoined node converges to
  the cluster's tombstone set without refusal and without
  resurrection.

Invariants added to the standing DST safety/liveness set:

```rust
assert_always!("no resurrection across partition-heal",
    !cluster.any_deleted_row_observed_live_after_heal());

assert_always!("sweep never reclaims unpropagated tombstone",
    cluster.all_reclaimed_tombstones_were_globally_observed());

assert_eventually!("rejoin within window converges",
    rejoining_node.converges_to_cluster_tombstone_set());
```

**Property-based (proptest).** Two properties on the sweeper:

- **Idempotent.** Running the sweep twice against the same replica
  produces the same final state as running it once.
- **Safe.** For any permutation of tombstone arrival order and any
  sweep-tick interleaving, the sweeper never reclaims a tombstone
  younger than `sweep_window` from the local logical clock.

**Mutation testing (cargo-mutants).** The `TombstoneSweeper`
reconciler and the `SweepPolicy` dispatch code are mandatory targets
under the existing ≥80% kill-rate gate. Canonical mutations to be
killed: `<` ↔ `<=` on the `deleted_at < now − window` comparator;
swap of `GossipAge` vs `ExpiresAt` branch; `tombstone = 1` filter
flipped to `tombstone = 0` in the sweep scan.

**Tier 3 (real-kernel integration).** No new test cases. The scheme
is observation-layer; BPF map hydration already filters on
subscription output, so an integration test that writes a tombstone
against a real Corrosion peer in an LVH VM and asserts the row
disappears from `bpftool map dump` is covered by the existing XDP
`SERVICE_MAP` test's SETUP/CHECK extensions.

## References

- `docs/whitepaper.md` §4 (Consistency Guardrails, ObservationStore),
  §8 (`revoked_operator_certs`), §17 (storage layers), §18
  (Tombstone sweep reconciler)
- `docs/product/architecture/adr-0005-test-distribution.md` (test
  layering)
- `docs/product/architecture/adr-0006-ci-wiring-dst-gates.md` (DST
  CI gating)
- `.claude/rules/testing.md` (Tier 1 DST, proptest, mutation testing)
- Antithesis, *Testing Tigris* (2026):
  https://antithesis.com/blog/2026/tigris_report/
