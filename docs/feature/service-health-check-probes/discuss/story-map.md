# Story Map: service-health-check-probes

**User:** Ana — Overdrive platform engineer on a single-node dev host (also serves Omar, future operator)
**Goal:** Submit a Service-kind workload and trust the wire signal to reflect operator-meaningful liveness (probe-derived Stable / Failed), not kernel bare-fork acceptance.
**Job:** J-OPS-004 (NEW — Service-honesty sub-job; extends J-OPS-003)

## Backbone

| 1. Author spec | 2. Submit & probe | 3. Observe terminal | 4. Inspect live state |
|---|---|---|---|
| Declare probes in TOML (or omit for default) | Stream `Accepted` then `Stable` or `Failed` | Receive honest reason; act if Failed | `alloc status` Probes section for ongoing health |

### Walking Skeleton (Slice 01 — default TCP-connect startup probe)

The thinnest end-to-end slice that closes RCA-A for the most common case (operator declares no probes):

| 1. Author | 2. Submit & probe | 3. Terminal | 4. Inspect |
|---|---|---|---|
| Minimal `[service]` spec with one `[[listener]]`, NO probes | Server infers TCP-connect probe vs listener.port; ProbeRunner ticks; ProbeResultRow lands | `ServiceSubmitEvent::Stable { settled_in: Duration, witness: StartupProbe { 0 } }` OR `Failed { StartupProbeFailed | EarlyExit }` | (Slice 01 ships row data; Probes section render lives in Slice 06) |

Walking-skeleton invariants the slice MUST establish:

1. `ProbeRunner` trait exists in `overdrive-worker` (TCP-only is fine for Slice 01).
2. `ProbeResultRow` exists in `overdrive-core` with the LWW shape per `(alloc_id, probe_idx)`.
3. `ServiceLifecycleReconciler` exists (split from current `JobLifecycleReconciler` per ADR-0047), reads ProbeResultRow + AllocStatusRow, emits `Action::SetTerminalCondition { Stable | Failed }`.
4. `TerminalCondition::Stable { settled_in, witness }` and `TerminalCondition::Failed { reason: StartupProbeFailed | EarlyExit }` exist as new variants on the ADR-0037 enum.
5. `ServiceSubmitEvent::Stable` and `ServiceSubmitEvent::Failed` are wired as terminal arms on the streaming wire (ADR-0032 Amendment 2026-05-10 extension).
6. Default-TCP-probe inference fires when TOML has no `[[health_check.*]]` and at least one `[[listener]]`.
7. CLI matches `ServiceSubmitEvent::Stable` and `ServiceSubmitEvent::Failed` arms and prints honest render lines (no `"live"` literal).

### Release 1 — Honest Service signal under default and explicit probe configs

| Slice | Stories included | Target outcome | KPI |
|---|---|---|---|
| 01 (WS) | US-01 Default TCP-startup-probe Stable/Failed end-to-end | Operator submits zero-probe Service and gets honest terminal | K1 Service-submit honesty rate |
| 02 | US-02 Explicit HTTP startup probe | Operator declares HTTP probe; same wire semantics | K1 |
| 03 | US-03 Explicit Exec startup probe | Operator declares Exec probe (in-cgroup); same wire semantics | K1 |
| 08 | US-08 EarlyExit detection within startup deadline | RCA-A coinflip case closed for Service kind | K1 (the specific failure that motivated #170) |

### Release 2 — Continuous health (readiness + liveness) + operator visibility

| Slice | Stories included | Target outcome | KPI |
|---|---|---|---|
| 04 | US-04 Readiness probe flips `Backend.healthy` | Failing backends removed from dataplane fingerprint within 1 tick | K2 Dataplane-health convergence |
| 05 | US-05 Liveness probe threshold triggers restart | Service auto-restarts on liveness failure (Service-kind only) | K3 Liveness-restart effectiveness |
| 06 | US-06 `alloc status` Probes section | Operator sees current probe state per alloc | K4 Operator probe-visibility coverage |

### Release 3 — Kind-rejection guardrails

| Slice | Stories included | Target outcome | KPI |
|---|---|---|---|
| 07 | US-07 Reject probes on Job/Schedule with named guidance | Operator confused-by-shape gets directed to right primitive | K5 Misshapen-spec named-error rate |

## Priority Rationale

**Sequencing logic.** Slice 01 (walking skeleton) MUST land first — it establishes the trait surface (`ProbeRunner`), the obs row (`ProbeResultRow`), the reconciler split (Service / Job lifecycle), the new TerminalCondition variants (`Stable`, `Failed` with reasons), and the wire variants (`ServiceSubmitEvent::Stable`, `ServiceSubmitEvent::Failed`). Every subsequent slice composes onto that foundation.

| Priority | Slice | Rationale |
|---|---|---|
| P0 | 01 (WS) | Closes RCA-A for the common case (no-probes-declared). Largest outcome leverage per LOC. Validates the riskiest assumption (the wire-shape + reconciler-shape compose end-to-end). |
| P0 | 08 | RCA-A coinflip case directly — the specific user pain that motivated #170. Stacks onto Slice 01's `Failed` arm with the EarlyExit reason variant. |
| P1 | 07 | Independent of probe runner — pure parser change. Can land in parallel with 01. Cheap insurance against operator confusion. |
| P1 | 02 | HTTP is the dominant probe mechanic in k8s-shape ecosystems; declared-probe operators expect this next. |
| P2 | 03 | Exec probe extends the runner trait but requires cgroup discipline; lower demand than HTTP but completes the k8s parity surface. |
| P2 | 04 | Readiness flips an existing field (`Backend.healthy` at `crates/overdrive-core/src/dataplane/fingerprint.rs:95`). Smaller surface but high integration risk — defer until 01–03 stable. |
| P3 | 05 | Liveness-driven restart reuses `Action::RestartAllocation` from JobLifecycle; the new semantics are "Service-kind-only" gating. |
| P3 | 06 | Pure operator-CLI surface — best landed last so the Probes section has all probe types and roles to render coherently. |

**Dependencies graph:**

```
Slice 01 (WS) ──┬── Slice 02 (HTTP startup)
                ├── Slice 03 (Exec startup)
                ├── Slice 04 (Readiness) ──── Slice 05 (Liveness)
                ├── Slice 06 (CLI Probes section)
                └── Slice 08 (EarlyExit)

Slice 07 (kind-rejection) — independent; can land in parallel with 01.
```

Each slice is independently demoable: Slice 04 (readiness) needs Slice 01 (runner + Service reconciler) but adds zero requirement on Slice 02 / 03 — readiness can run as a TCP probe with the default-inference logic.

## Backlog suggestions

| Story | Slice | Priority | Outcome link | Dependencies |
|---|---|---|---|---|
| US-01 | 01 (WS) | P0 | K1 | None |
| US-02 | 02 | P1 | K1 | US-01 |
| US-03 | 03 | P2 | K1 | US-01 |
| US-04 | 04 | P2 | K2 | US-01 |
| US-05 | 05 | P3 | K3 | US-04 (readiness must work first), US-01 |
| US-06 | 06 | P3 | K4 | US-01..05 (renders all probe types/roles) |
| US-07 | 07 | P1 | K5 | None (independent) |
| US-08 | 08 | P0 | K1 | US-01 |

## Non-stories (out of scope per #170)

- HTTPS / mTLS probes → Phase 3+ (whitepaper §11 sockops + kTLS); separate feature.
- gRPC probes → fast follow-up, separate feature.
- Probe-driven autoscaling → Phase 5+.
- Custom probe handlers beyond HTTP / TCP / Exec.
- Job-kind probes (rejected by Slice 07).
- Schedule-kind probes (rejected by Slice 07).
