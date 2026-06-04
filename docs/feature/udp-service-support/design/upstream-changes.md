# DESIGN → DISCUSS back-propagation — udp-service-support

> **Back-prop contract.** DESIGN found two factual corrections to DISCUSS
> [REF] stories in `../feature-delta.md`. Per the back-propagation rule,
> DESIGN does **not** silently edit the DISCUSS [REF] stories. Each
> correction below quotes the original text verbatim, states the
> correction + rationale, and is **flagged for the user to apply** (or to
> direct an in-place edit with an explicit "Changed Assumptions"
> annotation). Both corrections are already authoritative in the DESIGN
> SSOT (ADR-0060 D4/D6 + `brief.md` § UDP service support + the DESIGN
> [REF] sections appended to `feature-delta.md`).

**Architect:** Morgan. **Date:** 2026-06-02.

---

## Correction 1 — empty-backends purge is PER-PROTO (not "both protos")

**Source:** ADR-0060 § "Per-proto purge (resolves D4)".

**Original text** — `feature-delta.md` US-02 § Domain Examples, Example 3
(lines 380–383), VERBATIM:

> #### 3: Error/Boundary — empty backend set removes both protos' entries
> Ana scales `dns-resolver` to 0 backends. The update removes the
> `(10.244.0.20, 5353, udp)` entry (cross-service purge logic, mirroring
> the Sim adapter's `difference` check) — no stale udp entry lingers.

**Problem.** The heading says "removes **both protos'** entries," which
contradicts D4 (and the example body itself, which only removes the `udp`
entry). With the per-listener model, a VIP can carry separate proto
frontends installed by separate `update_service` calls (US-05). Scaling the
UDP service to zero must purge **only** `frontend.proto`'s REVERSE_NAT keys
— a co-resident TCP frontend on the same VIP must survive.

**Correction.** Empty-backends purge is **per-proto**:
`update_service(frontend_udp, [])` purges only `frontend.proto`'s REVERSE_NAT
keys for the VIP; other protos of the same VIP are untouched; cross-service
shared-backend keys are preserved by the existing `live_keys` difference
check (`crates/overdrive-sim/src/adapters/dataplane.rs:343-347`).

**Suggested replacement heading + body:**

> #### 3: Error/Boundary — empty backend set removes only THIS proto's entries
> Ana scales `dns-resolver` (udp/5353) to 0 backends. The update removes the
> `(10.244.0.20, 5353, udp)` entry only; a co-resident tcp frontend on the
> same VIP (installed by a separate `update_service` call) keeps its
> `(…, tcp)` entries. Cross-service shared-backend keys are preserved by the
> `live_keys` difference check — no stale udp entry lingers, no live tcp
> entry is collaterally purged.

**Disposition:** FLAGGED for user to apply to US-02 Example 3. (The US-02
AC "Empty-backend updates purge the udp entry" is already per-proto-correct;
only the Example-3 heading is misleading.)

---

## Correction 2 — true blast radius is 8 sites; hydrator IS changed in US-01

**Source:** ADR-0060 § "True blast radius (resolves D6)" + D6.

**Original text** — `feature-delta.md` US-01 § Technical Notes (line 330),
VERBATIM:

> - Single-cut migration (C6); all call sites in the same PR. Blast radius = 5 sites (trait, EbpfDataplane, SimDataplane, action-shim dispatch, ReverseNatLockstep invariant); hydrator UNCHANGED for US-01/US-04.

Related original text — US-01 § Acceptance Criteria implies the protocol
"flows" from intent but does not enumerate the Action/desired-projection
sites; and the Scope Assessment / story map describe proto plumbing as
landing in US-05's hydrator fan-out.

**Problem.** C3 ("Proto is NEVER defaulted to `Tcp` anywhere on the
intent→hydrator→`ServiceFrontend`→dataplane path") cannot be satisfied if
the `Action::DataplaneUpdateService` and the `ServiceDesired` projection do
**not** carry the protocol. Today neither does
(`reconcilers/mod.rs:440` — no proto on the Action;
`service_map_hydrator.rs:40` — `ServiceDesired` has no proto; the two
action-emission sites at `:235` and `:263` build the Action without proto).
If US-01 leaves these unchanged, the action-shim has nowhere to read the
proto from and would be forced to default to `Tcp` — a direct C3 violation.
Therefore the hydrator's desired projection and the Action **are** changed
in US-01, and the blast radius is **8 sites, not 5**.

**Correction.** True US-01 blast radius (single-cut, C6):

| # | Site | Path |
|---|------|------|
| 1 | `Dataplane::update_service` trait | `overdrive-core/src/traits/dataplane.rs:101` |
| 2 | `ServiceFrontend` (CREATE NEW) | `overdrive-core/src/dataplane/service_frontend.rs` |
| 3 | `SimDataplane` + `reverse_nat_keys_for` | `overdrive-sim/src/adapters/dataplane.rs:266,289` |
| 4 | `EbpfDataplane::update_service` | `overdrive-dataplane/src/lib.rs` |
| 5 | action-shim dispatch | `action_shim/dataplane_update_service.rs:100,130,160` |
| 6 | `ReverseNatLockstep` invariant | `overdrive-sim/src/invariants/reverse_nat_lockstep.rs` |
| **7** | **`Action::DataplaneUpdateService`** (+ proto) | `overdrive-core/src/reconcilers/mod.rs:440` |
| **8** | **`ServiceDesired` + obs→desired projection** (+ proto) | `overdrive-core/src/reconcilers/service_map_hydrator.rs:40,235,263` |

Sites 7–8 are the additions. The hydrator's *multi-listener fan-out* (one
`update_service` per `Listener`) remains a US-05 concern — that is distinct
from US-01's single-proto plumbing. US-01 makes a single service carry its
one declared proto end-to-end; US-05 makes a service carry *multiple*
listeners.

**Proto provenance (ATLAS-1 correction).** The proto for site #8 is
sourced from a **listener-bearing fact** — `ListenerRow`
(`overdrive-core/src/traits/observation_store.rs:321`: `port`/`protocol`/
`vip`) and/or the `BackendDiscoveryBridge` per-listener projection
(`overdrive-control-plane/src/reconciler_runtime.rs:2569`, keyed
`ServiceId::derive(vip, port, "service-map")`). It is **NOT** sourced from
`service_backends`: the current desired projection reads only
`service_backends_rows` (`reconciler_runtime.rs:1322-1348`), and
`ServiceBackendRowV1` (`observation_store.rs:875`) carries neither port nor
proto. The proto MUST be sourced from a listener-bearing fact; if no
listener proto can be resolved for the desired projection, that is an error
(Failed/structured), NEVER a silent `Proto::Tcp` default (C3).

**Write-path note (ATLAS-2, forward-pointer).** The existing
`ServiceBackendRow` write path collapses listeners to the first
(`reconciler_runtime.rs:2015-2019`: first-listener-only, port default `0`,
no proto). US-01 must source proto from the listener fact above, NOT the
proto-less `ServiceBackendRow`; the multi-listener generalization (and the
resulting extra write-path site) is **US-05** scope.

**Suggested replacement for the US-01 Technical Notes bullet:**

> - Single-cut migration (C6); all call sites in the same PR. True blast radius = **8 sites**: trait, `ServiceFrontend` (new), SimDataplane, EbpfDataplane, action-shim dispatch, ReverseNatLockstep invariant, **`Action::DataplaneUpdateService` (+ proto)**, and **`ServiceDesired` + the observation→desired projection (+ proto)**. The DISCUSS "5 sites / hydrator unchanged" estimate was low: C3 (no `Tcp` default) requires the Action and the desired projection to carry proto from a **listener-bearing fact** (`ListenerRow` / `BackendDiscoveryBridge` per-listener projection — NOT `service_backends`, which carries neither port nor proto). The hydrator's *multi-listener fan-out* is still a separate US-05 concern.

**Disposition:** FLAGGED for user to apply to US-01 Technical Notes (line
330), and optionally to add an AC clause: "the protocol dimension is added
to `Action::DataplaneUpdateService` and `ServiceDesired`; the desired
projection reads it from a listener-bearing fact (`ListenerRow` and/or the
`BackendDiscoveryBridge` per-listener projection), never from the
proto-less `service_backends` row, and if no listener proto can be resolved
that is an error (Failed/structured), never a silent `Proto::Tcp` default."
C3 is unchanged in intent — only the enumeration of *where* proto is
carried (and *which* fact it is sourced from) is made precise.

---

## Summary for the user

| # | DISCUSS site | Correction | Action needed |
|---|---|---|---|
| 1 | US-02 Domain Example 3 (lines 380–383) | "removes both protos" → **per-proto** purge | Apply suggested heading+body, or approve in-place edit with "Changed Assumptions" note. |
| 2 | US-01 Technical Notes (line 330) | "5 sites / hydrator unchanged" → **8 sites**, proto plumbed end-to-end | Apply suggested bullet (+ optional AC clause), or approve in-place edit. |

Neither correction changes scope, slice boundaries, the JTBD trace, or any
locked decision (D1a–D8). Both are factual-accuracy fixes that the DESIGN
SSOT (ADR-0060, brief.md, feature-delta DESIGN [REF]) already reflects. No
GitHub issue is required (these are in-flight artifact corrections, not
deferrals).
