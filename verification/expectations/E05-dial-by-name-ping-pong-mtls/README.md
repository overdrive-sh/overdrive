# E05 — two services dial each other by name; counters advance; each hop is mTLS'd

**Surface:** E (end-to-end) · **KPI:** K-DBN-3 · **Status:** `pending`

<!-- Status rationale: `pending` by design — the SAME posture as the sibling E04.
The black-box BIDIRECTIONAL ping-pong proof needs a converged full-system
deployment of TWO mesh workloads (a + b) on a real node, with the PRODUCTION
workload-identity CA → SVID → leg-C/leg-B mTLS path proven black-box (no
in-process test seam, no injected mtls_identity_override / TestPki), driven
through the BUILT overdrive binary (overdrive serve + two overdrive deploy).
That requires the EDD harness (#227 — a disposable full-system Lima VM) running
on the OS image (#75 — Image Factory MVP). Until both land, `od serve` + two `od
deploy` cannot be driven black-box with a real workload-identity CA, so this
expectation cannot be captured against the built binary. The in-process Tier-3
witnesses (the 02-02 walking skeleton `dns_responder_walking_skeleton.rs` for the
single-direction dial-by-name loop, and the 03-02 `dns_responder_ping_pong.rs`
RED scaffold for the bidirectional proof) cover the in-process slice with a test
PKI seam; E05 is the black-box operator-observable complement those tiers
under-serve. Do NOT self-stamp `satisfied` — bounce to a different-fox audit
once #227/#75 land. -->

## Expectation

Two mesh services deployed by the platform — `a` (reachable at
`a.svc.overdrive.local`) and `b` (reachable at `b.svc.overdrive.local`) —
**dial each other by name**, continuously, with the round-trips observable to
the operator:

- `a` resolves `b.svc.overdrive.local` (via `getaddrinfo`, NOT `dig` — the K2
  litmus: the production responder source-pins its reply) to a **stable
  frontend `F ∈ 10.98.0.0/16`** and calls B; B's inbound counter increments and
  its date refreshes.
- `b` resolves `a.svc.overdrive.local` the same way and calls A; A's inbound
  counter increments and its date refreshes.
- Both counters keep advancing on a **~10s ±5s cadence over a 60s window** —
  the operator-observable proof that the bidirectional loop is live, not a
  one-shot.
- **Each hop is intercepted + mTLS'd by the platform**: the inter-agent
  leg-B ↔ leg-C wire carries TLS-1.3 `application_data` records (content-type
  `0x17`), observable via `tcpdump` / `ss -tie`, with no cleartext — while the
  workloads themselves hold NO SVID material and dial PLAINTEXT (kernel-mediated
  mTLS; the workloads are identity-unaware — CLAUDE.md § "East-west mTLS tests").

This is the genuine operator-runnable proof for the dial-by-name feature: two
`overdrive deploy` commands against a booted `overdrive serve` produce a visible,
advancing, mutually-mTLS'd ping-pong — the whole feature, end to end.

- Anchor: S-DBN-PINGPONG (`two_services_dial_each_other_by_name_counters_advance_each_hop_mtls`, the Tier-3 scaffold in `crates/overdrive-control-plane/tests/integration/dns_responder_ping_pong.rs`)
- Anchor: K-DBN-3 (the bidirectional ping-pong KPI — both counters advance on a ~10s cadence over a 60s window, each hop mTLS'd)
- Anchor: roadmap step 03-02 (US-DBN-3 — bidirectional ping-pong demo + EDD expectation) and `docs/feature/dial-by-name-responder/slices/slice-02-bidirectional-ping-pong-demo.md`
- Anchor: ADR-0072 REV-2 (the stable-frontend dial-by-name contract) and GH #243 (dial-by-name-responder)

## Verification

Precondition (the deferral — identical to E04): a converged full-system
deployment of two mesh workloads (`a` + `b`) on a real node, with the
**production** workload-identity CA issuing the leg-C/leg-B SVIDs (no
`mtls_identity_override` test seam), driven through the **built `overdrive`
binary** (`overdrive serve` + `overdrive deploy a.toml` + `overdrive deploy
b.toml`). That harness is **#227** (the disposable full-system Lima VM on the
immutable OS) on **#75** (the OS image). Neither has landed, so this expectation
is `pending` and its `runner.sh` self-reports `pending` rather than narrating a
capture it cannot execute.

The example specs and the client program are READY for the capture: the two
`[service]`/`[exec]`/`[resources]`/`[[listener]]` specs
`examples/dial-by-name-responder/{a,b}.toml` are landed, and their `command`
runs the checked-in `examples/dial-by-name-responder/ping_pong.py` via
`/usr/bin/python3` (K3 — a real on-disk file next to the specs, no phantom path,
no build/staging step; an operator can run it by hand with plain `python3`).

Sub-claims (to be captured once #227 + #75 land):

1. `overdrive deploy a.toml` and `overdrive deploy b.toml` both exit 0
   (`Accepted.`), and both `a` and `b` reach Running-AND-HEALTHY (observable via
   `overdrive alloc status`).
2. From inside A's netns, `getaddrinfo("b.svc.overdrive.local")` (via `getent
   ahostsv4`, NOT `dig`) resolves to a stable `F ∈ 10.98.0.0/16` (never a
   `10.99.0.0/16` backend addr); symmetrically B resolves `a.svc.overdrive.local`.
3. Both inbound counters advance over a 60s window on a ~10s ±5s cadence
   (scraped from each workload's stdout / a CLI surface) — the bidirectional loop
   is live in both directions.
4. (Confidentiality) a wire capture on EACH hop's inter-agent leg-B ↔ leg-C
   path shows TLS-1.3 `application_data` records (content-type `0x17`) in both
   directions, and the workloads' PLAINTEXT request/response markers never appear
   on the encrypted inter-agent wire — kernel-mediated mTLS, the workloads hold
   no SVID.

`satisfied` requires sub-claims 1–4 captured on a #227 full-system Lima run
against the built binary, reviewed by a different-fox adversarial auditor reading
only the captured `evidence/` (per `.claude/rules/verification.md`).

## Evidence

None captured — `pending`. The `runner.sh` skeleton self-reports `pending` (the
`od serve` + `od deploy ×2` + per-hop wire-capture shape is sketched for when
#227 + #75 land; it does NOT narrate or fabricate a capture). Do NOT run
`harness/run-expectation.sh E05` to set `satisfied` before the precondition
exists.

The `what, forever` regression witnesses are the in-process Tier-3 modules: the
02-02 walking skeleton (`dns_responder_walking_skeleton.rs`, the single-direction
dial-by-name loop, GREEN) and the 03-02 ping-pong scaffold
(`dns_responder_ping_pong.rs`, the bidirectional proof, RED-scaffolded until the
EDD harness lands). E05 is the black-box operator-observable `why`; those modules
are the in-process `what`.
