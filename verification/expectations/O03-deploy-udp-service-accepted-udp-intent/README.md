# O03 — `overdrive deploy <udp-spec>` is accepted and the intent carries the UDP protocol

**Surface:** O (operator CLI) · **KPI:** K1 (deploy half) · **Status:** `satisfied`

<!-- Status rationale (2026-06-03, SHA 2ff69851, SEED=1, executed_in_lima: true,
runner_exit_code: 0). All three sub-claims captured black-box and satisfied,
confirmed by a different-fox adversarial evidence audit (executed not narrated,
no dodged sub-claim, no 15353/tcp coercion).

The fixture binds udp/15353 (NOT 5353 — systemd-resolved owns 5353 in the dev
VM; see the RCA docs/analysis/root-cause-analysis-convergence-dataplane-gap.md
and the fixture header). O03 is pre-convergence so the port choice does not
affect sub-claims 1-2, but the listener protocol render (sub-claim 3) shows the
fixture's actual port.

Two fixes unblocked the full capture:
1. single-node-dataplane-wiring (ADR-0061): production `overdrive serve` under
   the default config now provisions a dedicated veth pair (ovd-veth-cli/
   ovd-veth-bk) at boot and attaches the two XDP programs to the two distinct
   veth ifaces (not lo, which collided EBUSY / failed generic-SKB attach and
   aborted boot). serve therefore BINDS (evidence/serve.log: "control plane
   listening endpoint=https://127.0.0.1:7443/"), so the black-box deploy reaches
   it — sub-claims 1+2 (deploy exit 0 + `Accepted.`).
2. alloc-status listener rendering (commit 7e79007f handler/response + e9cec107
   the live-path render fix): `overdrive alloc status` now renders each Service
   listener as <port>/<protocol>, projected from the persisted
   WorkloadIntent::Service aggregate — INDEPENDENT of convergence. So a deployed
   UDP Service renders `15353/udp` immediately at 0 allocations (pre-convergence),
   which is the operator-surface proof that the accepted intent carries
   Proto::Udp, never coerced to Tcp (a coercion would render `15353/tcp`) —
   sub-claim 3.

The before+post-teardown probes both show a HEALTHY loopback and zero XDP/cgroup
residue; the fix moves XDP off lo entirely, so the loopback-leak hazard is
structurally gone. -->


## Expectation

An operator who runs `overdrive deploy dns-resolver.toml` — where
`dns-resolver.toml` declares a **udp** listener on 15353 and a backend bound
on 15353 — sees the deploy **exit 0** and print **`Accepted.`**, and the
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
- Anchor: ADR-0061 (`docs/product/architecture/adr-0061-single-node-veth-dataplane-wiring.md` — single-node serve provisions a veth pair + attaches the two XDP programs to two distinct ifaces; the boot fix that lets this capture reach a bound serve)
- Anchor: US-04 (`docs/feature/udp-service-support/feature-delta.md` § Walking skeleton — single UDP listener forward + reverse e2e)

## Verification

The runner brings up an EPHEMERAL control plane itself, inside a SINGLE Lima
invocation, with an EXIT-trap teardown sweep on every exit path (kill serve,
cgroup mass-kill+rmdir, XDP detach across ifaces) and before+after no-leak
probes (XDP attachment, loopback sanity, workload cgroups) written into
`evidence/` as proof the shared VM is left clean. Serve lifetime is seconds:
boot → deploy → `alloc status` → capture → teardown. It uses the production
default config — post-ADR-0061 serve auto-provisions the `ovd-veth-cli`/
`ovd-veth-bk` veth pair and attaches XDP to it (NOT `lo`, NOT `eth0`); no
SimDataplane override (test-only, not black-box). Leaked cgroups/XDP across runs
are a documented hazard (`.claude/rules/{testing,debugging}.md`).

The runner deploys a UDP `dns-resolver.toml` (udp/15353 listener + a real
`/usr/bin/socat` UDP-echo backend present in the Lima VM) against the built
binary, then runs `overdrive alloc status --job dns-resolver`, capturing both
commands' stdout/stderr and exit codes verbatim. Sub-claims:

1. The deploy command exits `0`.
2. Deploy stdout contains `Accepted.` (the `workload_submit_accepted` render shape).
3. The accepted intent carries the udp listener protocol — observable at the
   operator surface as `15353/udp` in `overdrive alloc status` (the Service's
   `Listeners:` section, projected from the persisted `WorkloadIntent::Service`
   aggregate). A `15353/tcp` line would be a `Proto` coercion to `Tcp` and fails
   the sub-claim. Because the listeners come from the intent (not the
   allocation), they render at 0 allocations — no convergence required.

`satisfied` requires all three, on a Lima run, with the SHA + seed pinned in
`evidence/verification.yaml`. All three are captured. The `what, forever`
witness for the deploy-accepted + `Proto::Udp`-threaded contract is the
direct-handler test landed under roadmap 01-05; this expectation is its
human-readable operator-surface companion.

## Evidence

Captured under `evidence/` by `harness/run-expectation.sh O03` (SHA
`2ff69851`, `SEED=1`, `executed_in_lima: true`, `runner_exit_code: 0` — see
`evidence/verification.yaml`). Adversarially reviewed by a different-fox audit
(read-only, evidence-only): all three sub-claims CONFIRMED executed (not
narrated), no `15353/tcp` coercion, no dodged sub-claim.

- `evidence/serve.log` — the ephemeral `overdrive serve` BOUND: `control plane
  listening endpoint=https://127.0.0.1:7443/`. serve provisioned the veth pair
  and attached the two XDP programs to `ovd-veth-cli`/`ovd-veth-bk` (not `lo`),
  then bound TLS and wrote the trust triple.
- `evidence/deploy_dns_resolver.out` — the verbatim accept render: `Accepted.`
  + Workload ID `dns-resolver` / Intent key / Spec digest / Outcome `created` /
  Endpoint `https://127.0.0.1:7443/` / Next.
- `evidence/deploy_dns_resolver.meta` — `# exit: 0`.
- `evidence/alloc_status_dns_resolver.out` — `overdrive alloc status --job
  dns-resolver` render including a `Listeners:` section with the line
  `  15353/udp` (and NO `15353/tcp`). The operator-surface proof of `Proto::Udp`.
- `evidence/build.log` — the single `cargo build -p overdrive-cli --bin
  overdrive` the serve+deploy+status share (succeeded).
- `evidence/probe_before_{loopback,xdp,cgroups}.txt` /
  `evidence/probe_post_teardown_{loopback,xdp,cgroups}.txt` — loopback `HEALTHY`
  before+after, no XDP programs (`(none)`), no `alloc-*.scope`: the bring-up
  left the shared VM clean.
- `evidence/serve_deploy.out` — verbatim guest-side narration
  (`INNER_DONE serve_status=ready deploy_rc=0`, `# alloc status exit: 0`).

Per-sub-claim verdict:

| # | Sub-claim | Verdict | Reason |
|---|---|---|---|
| 1 | deploy exits `0` | `satisfied` | `deploy_dns_resolver.meta`: `# exit: 0`; serve bound (veth-attached XDP, ADR-0061) so the deploy reached a live endpoint. |
| 2 | deploy stdout `Accepted.` | `satisfied` | `deploy_dns_resolver.out` line 1 is literally `Accepted.`, followed by the full `workload_submit_accepted` render. |
| 3 | accepted intent carries `Proto::Udp` | `satisfied` | `alloc_status_dns_resolver.out` renders `Listeners:` + `15353/udp` (and no `15353/tcp`), projected from the persisted Service intent — visible pre-convergence at 0 allocations. |

The proto render landed in two commits: `7e79007f` added the `listeners`
projection to `AllocStatusResponse` + the handler; `e9cec107` rendered it on the
live `overdrive alloc status` path (`render::alloc_status`). The crate-internal
`what, forever` witness remains `deploy_udp_walking_skeleton.rs`.
