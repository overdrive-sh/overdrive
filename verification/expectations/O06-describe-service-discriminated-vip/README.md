# O06 — `describe` of a deployed Service returns the discriminated `DescribeSpecOutput::Service` carrying the platform-issued VIP

**Surface:** O (operator-observable control-plane describe endpoint) · **KPI:** — (operability — #183 inspection-path closure) · **Status:** `satisfied`

<!-- Status rationale (2026-06-06, SHA 07ede7fc, SEED=1, executed_in_lima: true,
runner_exit_code: 0). All three sub-claims captured black-box and satisfied,
confirmed by a DIFFERENT-FOX adversarial evidence audit (a separate Haiku agent
that read ONLY evidence/ + the anchors, never the crates/ implementation, and
was instructed to refute). Verdict: SATISFIED — executed not narrated, numbers
add up (describe_api.json carries "kind":"service" + "vip":"10.96.0.2", HTTP 200,
empty curlerr), no sub-claim dodged, anchors present and predate the capture.

Evidence was re-captured CONVERGED after the audit (a strict strengthening, not
a change to the load-bearing claim): the runner now polls alloc status until the
workload reaches >=1 allocation (~1s) before capturing, so alloc_status_service.out
shows `Allocations: 1 / alloc-dns-resolver-0: Running / reason: driver started`
instead of the 0-allocation empty-state. The describe_api.json (the #183
deliverable — discriminated shape + VIP) is BYTE-IDENTICAL between the audited
and converged captures (same vip 10.96.0.2, same spec_digest), so the audit
verdict transfers unchanged.

Note: the converged capture incidentally double-confirms GH #219 — the
`alloc status` 0-allocation empty-state hint ("the scheduler + driver land in
phase-1-first-workload", alloc.rs:110) is stale: the driver demonstrably started
("reason: driver started"), so the scheduler+driver have not only landed (that
feature shipped) but are actively running the workload. #219 tracks the message
fix; it is a pre-existing alloc-status concern, NOT part of #183's describe
wire-shape scope. -->

## Expectation

An operator who has deployed a **Service** (`overdrive deploy <service-spec>`)
can inspect that workload through the control-plane **describe** endpoint
(`GET /v1/jobs/{id}`) and receives the **kind-discriminated `DescribeSpecOutput`**
response — specifically `DescribeSpecOutput::Service` (`"kind": "service"`)
carrying the **platform-issued VIP** as a required dotted-quad field.

Before #183 this surface returned **HTTP 400** for any non-Job workload
(`describe_workload` hard-rejected Service/Schedule intents because
`WorkloadDescription.spec` was typed `JobSpecInput`, Job-only). After #183 /
ADR-0064 the response is widened to a `oneOf` discriminator and the Service
arm surfaces the VIP the platform allocated at submit time (ADR-0049) — the
address the operator could not previously learn from this endpoint.

Three sub-claims, captured black-box against the built `overdrive` binary on
a real kernel (Lima):

1. **Precondition — deploy accepted.** `overdrive deploy <service-spec>`
   exits 0 and prints `Accepted.`; the Service intent is persisted and its
   VIP allocated at submit time (the precondition the describe read depends
   on).
2. **O-surface — the real CLI round-trips the widened describe response.**
   `overdrive alloc status --job <id>` (which calls `describe_workload` via
   the operator's real mTLS HTTP client) exits 0 against the deployed
   Service. This is the regression witness for the wire-shape migration: the
   pre-#183 shape, or any shape the CLI cannot deserialize, would fail this
   call. (Note: no CLI verb renders the VIP itself today — `alloc status`
   consumes the describe response only for `spec_digest`. The VIP is carried
   in the wire response, proven by sub-claim 3 + the anchor test below.)
3. **The #183 deliverable — discriminated shape + VIP on the wire.** A raw
   `GET /v1/jobs/{id}` against the running control plane (mTLS, using the
   serve-written trust triple) returns JSON whose `spec` carries
   `"kind": "service"` and a required `"vip"` IPv4 dotted-quad — the
   discriminated Service arm `JobSpecInput` structurally could not produce.

This is the **operator-observable closure of #183**: the describe inspection
path works for Service workloads and exposes the platform-issued address.

- Anchor: GH #183 (`WorkloadDescription Service-arm wire-shape widening — oneOf discriminator for describe_workload`) — the issue this expectation closes.
- Anchor: ADR-0064 (`docs/product/architecture/adr-0064-describe-side-spec-output-discriminator.md` — `DescribeSpecOutput` distinct kind-discriminated `oneOf`; Service arm carries a REQUIRED `vip: ServiceVip`; OQ-1/OQ-4/OQ-7).
- Anchor: acceptance test `describe_service_returns_discriminated_shape_with_vip` (`crates/overdrive-control-plane/tests/integration/describe_round_trip.rs`, commit `e848cbdd`) — the "what, forever" Tier-3 witness that the describe response is `DescribeSpecOutput::Service { vip, .. }` carrying the submit-allocated VIP. Predates this verification.
- Anchor: ADR-0049 (`docs/product/architecture/adr-0049-platform-issued-service-vip-allocator.md` — the VIP is platform-issued, keyed by `spec_digest`; the operator never names it; describe SURFACES it read-only).

## Verification

Captured via `verification/harness/run-expectation.sh O06` — the runner boots
an ephemeral `overdrive serve` inside Lima (post-ADR-0061 veth dataplane
model), deploys a Service spec, captures `overdrive alloc status` (the CLI
round-trip) and a raw mTLS `GET /v1/jobs/{id}` (the describe wire response),
then tears down with the XDP / cgroup leak sweep + before/after no-leak
probes (per `.claude/rules/debugging.md` and `testing.md`). Black-box only —
no `overdrive-*` crate is linked.

See `evidence/` for the pinned capture (`evidence/verification.yaml` records
the SHA, dirty state, DST seed, and Lima execution flag).
