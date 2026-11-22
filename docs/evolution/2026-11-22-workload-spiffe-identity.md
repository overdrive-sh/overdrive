# Evolution — A workload SVID names the workload, not the allocation

**Feature:** `workload-spiffe-identity`. **Finalized:** 2026-11-22.
**Scope:** the SPIFFE identity a workload presents, the audit row that records each issued certificate, the dataplane association between a backend id and an identity, and the gateway's peer check. Five implementation steps plus a review-fix commit.
**Waves run:** DESIGN (2026-11-21, user approval the same day), DISTILL (two review rounds), DELIVER (2026-11-21 to 2026-11-22). The defect was found and analysed before the feature began, so there was no DISCUSS wave.
The [feature contract and wave history](../feature/workload-spiffe-identity/feature-delta.md), [roadmap](../feature/workload-spiffe-identity/deliver/roadmap.json) and [execution log](../feature/workload-spiffe-identity/deliver/execution-log.json) remain the delivery records.

## Summary

A workload's SPIFFE ID is now `spiffe://overdrive.local/workload/<name>`, and every allocation of the workload shares it. Each allocation still holds its own SVID, with its own key, serial and one-hour rotation. `SpiffeId::for_workload` builds the identity and `SpiffeId::workload_id` inverts it, and they are the only parsers: the three private parsers that each took the shape apart now route through them.

The audit row `IssuedCertificateRow` takes a version bump (V2) and records `issued_for`, either an allocation or the gateway. `alloc status` lists each running allocation's own certificate. The dataplane association between a `BackendId` and an identity carries the workload identity, so a replacement allocation at a known endpoint keeps its id and changes nothing. The gateway requires the upstream peer to present the selected backend's workload identity, and a replacement allocation passes.

## Context

S-PIG-71 (the public route answers again after `overdrive serve` restarts) returned 502 for the whole 60 second window. Bisecting found that commit `4ebc41233` made the boot resync pass program every stored service row at start-up. That exposed a latent defect in the identity grain: the dataplane refused a replacement allocation at an endpoint it had memoised, because the replacement's per-allocation identity differed from the one recorded for the same address (`BackendIdentityConflict`). The resync pass did not introduce the defect. It made the sequence reachable on every restart.

The root-cause analysis is [`docs/analysis/root-cause-analysis-public-ingress-gateway-502-after-serve-restart.md`](../analysis/root-cause-analysis-public-ingress-gateway-502-after-serve-restart.md). The identity-grain options were researched in [`docs/research/dataplane/backend-identity-on-endpoint-reuse-comprehensive-research.md`](../research/dataplane/backend-identity-on-endpoint-reuse-comprehensive-research.md).

## Key decisions

Every decision was put to the user and approved on 2026-11-21.

| Decision | Outcome | ADR |
|---|---|---|
| D1 | A workload's SPIFFE ID is `spiffe://overdrive.local/workload/<name>`, shared by all its allocations. Each allocation keeps its own SVID | [0344](../product/architecture/adr-0344-a-workload-svid-names-the-workload-not-the-allocation.md) |
| D2 | The per-allocation distinction is an explicit `AllocationId`, not encoded in the identity. The audit row records it, and restart recovery and the `alloc status` listing key on it | [0345](../product/architecture/adr-0345-svid-audit-rows-and-restart-recovery-key-on-the-allocation-not-the-identity.md) |
| D3 | The gateway requires the peer to present the selected backend's workload identity. The check itself is unchanged, its meaning changes through the identity it is given | [0346](../product/architecture/adr-0346-gateway-client-mtls-requires-the-selected-backends-workload-identity.md) |
| D4 | The dataplane association carries the workload identity. A same-workload replacement at a memoised endpoint reuses the `BackendId`, and `BackendIdentityConflict` is removed | [0347](../product/architecture/adr-0347-dataplane-backend-identity-association-carries-the-workload-identity.md) |
| D5 | Supersession, not amendment: ADR-0234 and ADR-0241 keep their text and point at their superseders | 0346, 0347 |
| DR-1 (b) | When a different workload is programmed at an endpoint the dataplane still memoises, the endpoint keeps its `BackendId` and the id is re-associated with the new workload's identity. A connection whose receipt was taken for the previous workload and is verified after the re-association is checked against the new workload. The user accepted this consequence | 0347 |
| DR-2 | Flow telemetry stays workload-grain. No change | feature delta |
| PA-1 | `Backend.alloc` is renamed `Backend.workload_identity`. The persisted row layout is unchanged because rkyv archives positions, not names | feature delta |
| DD-2 | `issued_for` is a sum type, `Allocation(AllocationId)` or `Gateway`, because gateway SVIDs share the audit table and have no allocation | feature delta |

Two further approvals: growth of the Quint model for DR-1 (b) (`AssociationFollowsProgrammedWorkload`, a `staleAssoc` hazard, a rewritten takeover run), and two companion tests (a seeded `overdrive-sim` invariant, and a host-versus-Sim dataplane equivalence test).

Persisted shapes are a greenfield single cut. A row written by a prior binary carries a per-allocation identity and is discarded with the data directory. No reader translates an `/alloc/` identity.

## What shipped

| Commit | What landed |
|---|---|
| `8f6377977` | `SpiffeId::for_workload` and `workload_id`, the parse SSOT in core |
| `a18364d1a` | `Backend.alloc` renamed to `Backend.workload_identity` |
| `376c38df3` | `IssuedCertificateRow` V2 records `issued_for` |
| `7d45ec7f6` | A workload SVID names the workload: `IssueSvid` carries the workload identity, `ever_issued` becomes a set of `AllocationId` |
| `494265e1e` | A backend id follows the workload programmed at its endpoint, in the host and Sim dataplane |
| `433f3fb91` | Restored coverage lost in the sweep, and the review fixes |

Earlier commits: the DISTILL acceptance tests (`75e972468`) and the Quint driver for the selection receipt moved to a guest placement (`94dd15d5d`).

## Verification

- S-PIG-71, the regression anchor, fails before the feature and passes on Metal after it, unchanged.
- The full suite passed 3758 of 3758 after the review fixes.
- The Quint subsystem `public-ingress-gateway` has 36 checks recorded, including the runs `replacement_reuses_backend_id`, `serve_restart_replacement` and `different_workload_takeover`.
- Mutation testing is disabled for this repository (CLAUDE.md), so no mutation gate ran.

## Reviews

DESIGN took two review rounds and DISTILL took two. DELIVER ran an adversarial and a subtractive review. The sweep that moved every test from per-allocation to per-workload identity had dropped some coverage, and `433f3fb91` restored it.

## Lessons

- **The per-allocation segment was never decided.** `/alloc/<a>` was copied from a whitepaper example and then built on. Nothing in an ADR chose it, and the gateway check, the dataplane memo and the audit row all inherited it. A shape that appears in an example is a hypothesis until a decision records it.
- **The first proposed fix treated the symptom.** Issuing a fresh backend id for each replacement would have made the 502 go away and left the grain wrong. The user rejected it. The identity grain was the defect.
- **Bisect over assumption.** The claim that S-PIG-71 "used to pass" was correct. Bisecting to `4ebc41233` turned a vague regression into a specific exposed sequence.

## Known unrelated issues found

- `an_outbound_connection_names_its_pump_threads_by_role` is flaky, failing about 30% of runs on `origin/main` as well.
- The `reconciler-evaluation-wake` Quint evidence on main is stale.
- `cargo fmt --check` reports drift in three files this feature did not touch, on main.

## Documentation changed

Website: `website/content/docs/concepts/identity.mdx`, `concepts/public-ingress-gateway.mdx` and `concepts/architecture.mdx` describe the workload-shaped identity, the per-allocation certificate, and the gateway's peer check. **No blog post, by the user's decision.**

## Scope limits and deferrals

None introduced. No GitHub issue was proposed.

## Permanent artifacts

- ADRs 0344 to 0347 in `docs/product/architecture/`, with ADR-0234 and ADR-0241 marked superseded.
- Code: `crates/overdrive-core/src/id.rs`, `crates/overdrive-core/src/ca/issued_certificate_row.rs`, `crates/overdrive-reconcilers/src/svid_lifecycle.rs`, `crates/overdrive-dataplane/src/mtls/gateway_client.rs`, `crates/overdrive-control-plane/src/handlers.rs`.
- The Quint model `specs/quint/public-ingress-gateway/`.

## Finalization

- **Issues closed:** none. No GitHub issue tracked the 502-after-restart defect; #242 (intended-peer pinning) received a comment that pinning is now workload-grain.
- **Metal cleanup:** _to be filled by the orchestrator (remote checkout and build output removed after any needed evidence is pulled; record the run id)_.
- **Website validation and documentation review:** `bun run typecheck` and `bun run build` pass. An independent reviewer verified every claim against the code; its four prose findings are fixed. No blog post, by the user's decision.
- **Session files removed:** _to be filled by the orchestrator (`deliver/.develop-progress.json`, after approval)_.
