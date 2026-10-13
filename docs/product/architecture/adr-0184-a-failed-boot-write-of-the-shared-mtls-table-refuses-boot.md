# ADR-0184 — A failed boot write of the shared mTLS table refuses boot

## Status

**Accepted — decision QHOLD (its boot clause: a failed boot write of the shared
mTLS table refuses boot) approved by user 2026-10-11.** GH #295. Recorded in
the #295 feature delta, § *[REF] Composition — production entry points*.
Related: ADR-0169 (counted quiescence hold), ADR-0124 (bounded runtime
recovery and fail-stop).

## Context

During boot, after VM reclamation and the stale-state sweep and before any
guest forwarding opens, the mTLS worker writes the node's shared firewall
table: leg-C interception, the leg-S exemption, the host-internal output
rules (ADR-0167) and the constant rules that refuse guest-prefix traffic not
diverted to leg-C (ADR-0172). The table is written, and every later
replacement of it is made, in one nft transaction, so a failed write leaves
the table as it was: absent on a node that had none, complete otherwise.

On the appliance only Overdrive writes this table (ADR-0068), so a failed
boot write is one of Overdrive's own failures, and the restarted process
converges the table on its next boot. Without the table, no guest-prefix rule
and no leg-C interception exist, so forwarding cannot be opened safely. The
question is what boot does when the write fails.

## Decision

- **Boot refuses.** A failure of the shared table's boot write ends boot with
  a typed refusal, before guest forwarding opens and before any workload is
  admitted. `serve` never runs without its shared table.
- **The retry is the restart.** The external process supervisor restarts
  `serve`; the new boot observes the table and converges it again in one
  transaction (converge-on-boot, `.claude/rules/reconcilers.md`). No
  in-process retry loop and no runtime repair path exist for the boot write.
- **Runtime damage is unchanged.** Damage to the table after boot is a runtime
  recovery under a quiescence hold (ADR-0169), bounded by ADR-0124.

The refusal's typed error and its health event are pinned in the feature
delta (§ *Composition*, § *Typed error taxonomy*).

## Alternatives considered

- **Boot continues and `serve` runs quiesced until a runtime recovery
  repairs the table.** It routes a boot-time fault through the runtime
  recovery machinery, keeps a process serving with no forwarding until that
  repair, and adds a boot state ("running, quiesced, table absent") that
  every runtime owner must handle. The restart already gives the same
  repair through the one boot path. Rejected.
- **Retry the write inside boot with a bound.** A second, boot-only retry
  policy beside ADR-0124's bound and the supervisor restart, for a fault
  whose repair is the same transaction. Rejected.

## Consequences

- A failed boot write costs one `serve` restart; while `serve` is down no
  guest flow exists (ADR-0159).
- Recovery from a persistently failing write depends on the external process
  supervisor, as every fail-stop of ADR-0124 does.
- The rule is covered by the feature delta's § *Formal protocol model* and
  validation plan.
