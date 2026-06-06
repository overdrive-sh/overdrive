# O06 — `describe` of a deployed Service returns the discriminated `DescribeSpecOutput::Service` carrying the platform-issued VIP

**Surface:** O (operator-observable control-plane describe endpoint) · **KPI:** — (operability — #183 inspection-path closure) · **Status:** `satisfied`

<!-- Status rationale (2026-06-06, SEED=1, executed_in_lima: true,
runner_exit_code: 0 — pinned SHA in evidence/verification.yaml). All FOUR
sub-claims (SC1 deploy / SC2 alloc-status works / SC2b CLI renders the VIP /
SC3 describe API discriminated shape + VIP) captured black-box and satisfied,
confirmed by a DIFFERENT-FOX adversarial evidence audit (a separate Haiku agent
that read ONLY evidence/ + the anchors, never the crates/ implementation, and
was instructed to refute). Verdict: SATISFIED — executed not narrated, numbers
add up (the alloc-status CLI VIP `10.96.0.2` byte-matches describe_api.json's
"vip":"10.96.0.2"; describe HTTP 200; empty curlerr), no sub-claim dodged,
anchors present and predate the capture.

Captured CONVERGED: the runner polls alloc status until the workload reaches
>=1 allocation (~1s) before capturing, so alloc_status_service.out shows
`Allocations: 1 / alloc-dns-resolver-0: Running / reason: driver started`.

SC2b is the operator-visible-VIP proof. It became true when the #220 rendering
half landed (`render_vip_section` — `alloc status` now prints a `VIP: <ipv4>`
line for Service reads, reading the vip the AllocStatusResponse envelope already
carried per ADR-0049). Before that, the VIP was on the wire (SC3) but the CLI
dropped it; the evidence was re-captured at that HEAD and re-audited (different
fox) — verdict held with SC2b added.

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

Four sub-claims, captured black-box against the built `overdrive` binary on
a real kernel (Lima):

1. **Precondition — deploy accepted.** `overdrive deploy <service-spec>`
   exits 0 and prints `Accepted.`; the Service intent is persisted and its
   VIP allocated at submit time (the precondition the reads below depend on).
2. **O-surface — the operator inspection command works.** `overdrive alloc
   status --job <id>` (the operator's real mTLS HTTP client against
   `GET /v1/allocs`, which carries `spec_digest` + the Service `vip` +
   `listeners` per ADR-0049) exits 0 against the deployed Service and renders
   the converged allocation.
3. **O-surface — the operator SEES the VIP in the CLI.** That same `alloc
   status` output now renders a `VIP: <ipv4>` line (the rendering half of
   #220 — `render_vip_section`). Before it landed, the VIP was on the wire but
   the CLI dropped it; this is the operator-visible-VIP proof.
4. **The #183 deliverable — discriminated describe shape + VIP on the wire.**
   A raw `GET /v1/jobs/{id}` against the running control plane (mTLS, using
   the serve-written trust triple) returns JSON whose `spec` carries
   `"kind": "service"` and a required `"vip"` IPv4 dotted-quad — the
   discriminated Service arm `JobSpecInput` structurally could not produce.
   This is the describe endpoint #183 widened (distinct from the `/v1/allocs`
   surface sub-claims 2–3 exercise; both now expose the VIP).

This is the **operator-observable closure of #183**: the describe endpoint
returns the discriminated Service shape with the platform-issued VIP, and the
operator's `alloc status` command now both works and prints that VIP.

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
