# O03 — `overdrive deploy <udp-spec>` is accepted and the intent carries the UDP protocol

**Surface:** O (operator CLI) · **KPI:** K1 (deploy half) · **Status:** `pending`

<!-- Status rationale (2026-06-02, SHA ea3c86f6, SEED=1, executed_in_lima: true):
sub-claims 1+2 are NOT capturable black-box on this Lima VM. The runner now
brings up an ephemeral, trap-guarded `overdrive serve` itself (single Lima
invocation, EXIT-trap sweep, before+after no-leak probes) — but production
`overdrive serve` constructs the real `EbpfDataplane`, which ATTACHES XDP to the
configured client_iface (default `lo`) at boot, BEFORE it binds the TLS
listener. On this VM that attach fails:
`xdp_reverse_nat_lookup.attach(lo, DRV_MODE): bpf_link_create failed` (generic
SKB-mode fallback also fails — see evidence/serve.log). serve therefore never
binds and never writes the trust triple, so the black-box deploy cannot reach it
here. The runner does NOT attach XDP to `eth0` (the VM's only NIC, shared across
the user's Conductor workspaces — the exact forbidden hazard) and does NOT fall
back to a test-only SimDataplane (not black-box reachable). Sub-claim 3 remains a
structural operator-surface gap (listener protocol not rendered on any black-box
read surface). Stays `pending`. The before+post-teardown no-leak probes both show
a HEALTHY loopback and zero XDP/cgroup residue — proof the bring-up attempt left
the shared VM clean. The `what, forever` witness for all three sub-claims is the
direct-handler test deploy_udp_walking_skeleton.rs (spawns serve with an injected
SimDataplane, so it never touches XDP). -->


## Expectation

An operator who runs `overdrive deploy dns-resolver.toml` — where
`dns-resolver.toml` declares a **udp** listener on 5353 and a backend bound
on 5353 — sees the deploy **exit 0** and print **`Accepted.`**, and the
accepted/submitted intent carries the **udp** listener protocol (the
`ServiceFrontend` proto is `Proto::Udp`, never coerced to `Tcp`).

This is the **operator-surface, deploy half** of the S-04-A walking
skeleton — cleanly capturable black-box against the built binary without
the real `ThreeIfaceTopology` veth setup that the wire-capture half
requires. It proves the entry point (CLI verb `Deploy`, NOT `job submit`)
parses a UDP spec, accepts it, and threads `Proto::Udp` through the typed
`ServiceFrontend` surface ADR-0060 introduced — the precondition the
end-to-end reverse-path proof (E02) builds on.

- Anchor: S-04-A (`docs/feature/udp-service-support/distill/test-scenarios.md` — walking skeleton: deploy exits 0 + prints `Accepted.`)
- Anchor: roadmap 01-05 (`docs/feature/udp-service-support/deliver/roadmap.json` — "S-04-A driving-adapter companion: `overdrive deploy <udp-spec>` accepted via direct-handler test")
- Anchor: ADR-0060 (`docs/product/architecture/adr-0060-service-frontend-update-service-signature.md` — `ServiceFrontend(vip, port, proto)` carries the L4 protocol; proto is never positionally reconstructed)
- Anchor: US-04 (`docs/feature/udp-service-support/feature-delta.md` § Walking skeleton — single UDP listener forward + reverse e2e)

## Verification

The runner brings up an EPHEMERAL control plane itself, inside a SINGLE Lima
invocation, with an EXIT-trap teardown sweep on every exit path (kill serve,
cgroup mass-kill+rmdir, XDP detach across ifaces) and before+after no-leak
probes (XDP attachment, loopback sanity, workload cgroups) written into
`evidence/` as proof the shared VM is left clean. Serve lifetime is seconds:
boot → deploy → capture → teardown. It does NOT attach XDP to `eth0` and does
NOT use a SimDataplane override (test-only, not black-box) — leaked cgroups/XDP
across runs are a documented hazard (`.claude/rules/{testing,debugging}.md`).

The runner deploys a UDP `dns-resolver.toml` (udp/5353 listener + a real
`/usr/bin/socat` UDP-echo backend present in the Lima VM) against the built
binary and captures the deploy command's stdout/stderr and exit code verbatim.
Sub-claims:

1. The deploy command exits `0`.
2. Stdout contains `Accepted.` (the `workload_submit_accepted` render shape).
3. The accepted intent carries the udp listener protocol — observable at the
   operator surface as `Proto::Udp` on the submitted `ServiceFrontend`
   (never `Tcp`). The runner observes this via the deploy/accept output or a
   follow-up read of the submitted intent; if the operator surface does not
   yet expose the proto for inspection, the runner captures sub-claims 1–2
   and leaves sub-claim 3 `pending` (rather than narrating it).

`satisfied` requires all three, on a Lima run, with the SHA + seed pinned
in `evidence/verification.yaml`. The `what, forever` witness for the
deploy-accepted contract is the direct-handler test landed under roadmap
01-05; this expectation is its human-readable operator-surface companion.

## Evidence

Captured under `evidence/` by `harness/run-expectation.sh O03` (SHA
`ea3c86f6`, `SEED=1`, `executed_in_lima: true`, `runner_exit_code: 0` — see
`evidence/verification.yaml`).

- `evidence/serve.log` — the ephemeral `overdrive serve` the runner started.
  The binary compiled and ran, but refused to bind: after `native XDP attach
  not supported by driver; falling back to generic (SKB) mode iface=lo`, the
  generic attach ALSO failed —
  `EbpfDataplane construction failed (client_iface=lo, backend_iface=lo):
  xdp_reverse_nat_lookup.attach(lo, DRV_MODE): bpf_link_create failed`. This is
  the executed root cause: production serve attaches XDP at boot BEFORE binding,
  and XDP attach to `lo` fails on this VM. The trust triple was never written,
  so the deploy had no endpoint to reach.
- `evidence/build.log` — the single `cargo build -p overdrive-cli --bin
  overdrive` the serve+deploy share (succeeded).
- `evidence/probe_before_{loopback,xdp,cgroups}.txt` — clean start: loopback
  `HEALTHY` (refused fast), no XDP programs, no `alloc-*.scope`.
- `evidence/probe_post_teardown_{loopback,xdp,cgroups}.txt` — written by the
  EXIT-trap sweep on every exit path (including this serve-failure path):
  loopback `HEALTHY`, no XDP programs, no `alloc-*.scope`. Identical to the
  before-probes — the bring-up attempt left the shared VM clean (no leaked XDP
  on `lo`, no leaked workload cgroups).
- `evidence/serve_deploy.out` — verbatim guest-side narration of the whole
  single-Lima-invocation bring-up (`INNER_DONE serve_status=not-ready`).

Per-sub-claim verdict:

| # | Sub-claim | Verdict | Reason |
|---|---|---|---|
| 1 | deploy exits `0` | `pending` | not reached — production serve refused to bind: `EbpfDataplane` XDP attach to `lo` failed at boot (`bpf_link_create failed`, generic SKB fallback also failed). No endpoint to deploy against. Not a CLI defect; an environment limit (this VM's `lo` does not accept the XDP program — Tier-3 tests use dedicated veth, not `lo`). |
| 2 | stdout contains `Accepted.` | `pending` | not reached — same XDP-attach blocker; deploy never ran. |
| 3 | accepted intent carries `Proto::Udp` | `pending` (structural gap) | the deploy-accept render (`render::workload_submit_accepted`: Workload ID / Intent key / Spec digest / Outcome / Endpoint / Next) does NOT render the listener protocol, and no read surface (`od job list`, `od alloc status`) renders it either. The 01-05 direct-handler test proves it via a crate back-door (`LocalIntentStore` read) unavailable to a black-box runner. `crates/overdrive-cli/tests/integration/deploy_udp_walking_skeleton.rs` is the `what, forever` witness. |

To capture 1+2 black-box, production serve would need a kernel/iface that
accepts the reverse-NAT XDP program on a safe-to-attach interface (not `lo`,
which fails here; not `eth0`, the shared VM's only NIC). On such a host, the
runner brings serve up and captures 1+2 with no code change. Until then, the
authoritative proof of deploy-accepted + `Proto::Udp`-threaded is the
direct-handler test `deploy_udp_walking_skeleton.rs`, which spawns serve with an
injected SimDataplane and never touches XDP. Sub-claim 3 stays `pending`
regardless until an operator surface renders the listener protocol (an honest
gap, not a failure).
