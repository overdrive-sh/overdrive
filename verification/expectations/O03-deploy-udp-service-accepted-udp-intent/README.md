# O03 — `overdrive deploy <udp-spec>` is accepted and the intent carries the UDP protocol

**Surface:** O (operator CLI) · **KPI:** K1 (deploy half) · **Status:** `partial`

<!-- Status rationale (2026-06-02, SHA 7eea73cc, SEED=1, executed_in_lima: true,
runner_exit_code: 0). Sub-claims 1+2 are now CAPTURED BLACK-BOX and SATISFIED;
sub-claim 3 remains an honest operator-surface gap (pending). What changed: the
`single-node-dataplane-wiring` fix (ADR-0061) landed. Production `overdrive serve`
no longer attaches both XDP programs to `lo` (which collided EBUSY / failed
generic-SKB attach on this VM and aborted boot). It now provisions a dedicated
host-netns veth pair (`ovd-veth-cli`/`ovd-veth-bk`) at boot and attaches the two
distinct XDP programs to the two distinct veth ifaces. serve therefore BINDS
(`evidence/serve.log`: "control plane listening endpoint=https://127.0.0.1:7443/"),
writes the trust triple, and the black-box deploy reaches it. The ephemeral
trap-guarded serve (single Lima invocation, EXIT-trap sweep, before+after no-leak
probes) captured: deploy exit 0 (sub-claim 1) + `Accepted.` render (sub-claim 2).
The before+post-teardown probes both show a HEALTHY loopback and zero XDP/cgroup
residue — and because the fix moves XDP off `lo` entirely, the loopback-leak hazard
is structurally gone. Verified by a different-fox adversarial evidence audit (no
narration, SHA current, no dodged sub-claim). Sub-claim 3 (listener protocol
rendered black-box) stays `pending`: the deploy-accept render carries no proto
field and no operator read surface renders it — the `what, forever` witness is the
direct-handler test deploy_udp_walking_skeleton.rs (crate back-door LocalIntentStore
read). Overall `partial` because the expectation requires all three sub-claims for
`satisfied` and sub-claim 3 is an unrenderable structural gap, not yet captured. -->


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
- Anchor: ADR-0061 (`docs/product/architecture/adr-0061-single-node-veth-dataplane-wiring.md` — single-node serve provisions a veth pair + attaches the two XDP programs to two distinct ifaces; the boot fix that lets this capture reach a bound serve)
- Anchor: US-04 (`docs/feature/udp-service-support/feature-delta.md` § Walking skeleton — single UDP listener forward + reverse e2e)

## Verification

The runner brings up an EPHEMERAL control plane itself, inside a SINGLE Lima
invocation, with an EXIT-trap teardown sweep on every exit path (kill serve,
cgroup mass-kill+rmdir, XDP detach across ifaces) and before+after no-leak
probes (XDP attachment, loopback sanity, workload cgroups) written into
`evidence/` as proof the shared VM is left clean. Serve lifetime is seconds:
boot → deploy → capture → teardown. It uses the production default config —
post-ADR-0061 serve auto-provisions the `ovd-veth-cli`/`ovd-veth-bk` veth pair
and attaches XDP to it (NOT `lo`, NOT `eth0`); no SimDataplane override (that is
test-only, not black-box). Leaked cgroups/XDP across runs are a documented hazard
(`.claude/rules/{testing,debugging}.md`).

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
in `evidence/verification.yaml`. Sub-claims 1+2 are captured and satisfied;
sub-claim 3 is an unrenderable operator-surface gap, so the expectation is
`partial`. The `what, forever` witness for the deploy-accepted contract is
the direct-handler test landed under roadmap 01-05; this expectation is its
human-readable operator-surface companion.

## Evidence

Captured under `evidence/` by `harness/run-expectation.sh O03` (SHA
`7eea73cc`, `SEED=1`, `executed_in_lima: true`, `runner_exit_code: 0` — see
`evidence/verification.yaml`). Adversarially reviewed by a different-fox audit
(read-only, evidence-only): sub-claims 1+2 CONFIRMED executed (not narrated),
SHA current, sub-claim 3 correctly left pending.

- `evidence/serve.log` — the ephemeral `overdrive serve` the runner started.
  Post-ADR-0061 it BOUND: `control plane listening
  endpoint=https://127.0.0.1:7443/`. serve provisioned the veth pair and
  attached the two XDP programs to `ovd-veth-cli`/`ovd-veth-bk` (not `lo`), then
  bound the TLS listener and wrote the trust triple — so the deploy had an
  endpoint to reach.
- `evidence/deploy_dns_resolver.out` — the verbatim accept render: `Accepted.`
  followed by Workload ID `dns-resolver` / Intent key `workloads/dns-resolver`
  / Spec digest / Outcome `created` / Endpoint `https://127.0.0.1:7443/` / Next.
- `evidence/deploy_dns_resolver.meta` — `# exit: 0`.
- `evidence/build.log` — the single `cargo build -p overdrive-cli --bin
  overdrive` the serve+deploy share (succeeded).
- `evidence/probe_after_xdp.txt` — `bpftool prog show` lists the two distinct
  XDP programs `xdp_service_map` + `xdp_reverse_nat` loaded (on the veth pair).
- `evidence/probe_before_{loopback,xdp,cgroups}.txt` — clean start: loopback
  `HEALTHY` (refused fast), no XDP programs, no `alloc-*.scope`.
- `evidence/probe_post_teardown_{loopback,xdp,cgroups}.txt` — written by the
  EXIT-trap sweep: loopback `HEALTHY`, no XDP programs (`(none)`), no
  `alloc-*.scope`. The bring-up left the shared VM clean.
- `evidence/serve_deploy.out` — verbatim guest-side narration of the
  single-Lima-invocation bring-up (`INNER_DONE serve_status=ready deploy_rc=0`).

Per-sub-claim verdict:

| # | Sub-claim | Verdict | Reason |
|---|---|---|---|
| 1 | deploy exits `0` | `satisfied` | `deploy_dns_resolver.meta`: `# exit: 0`; `serve_deploy.out`: `INNER_DONE serve_status=ready deploy_rc=0`. serve bound (veth-attached XDP, ADR-0061) so the deploy reached a live endpoint. |
| 2 | stdout contains `Accepted.` | `satisfied` | `deploy_dns_resolver.out` line 1 is literally `Accepted.`, followed by the full `workload_submit_accepted` render (Workload ID / Intent key / Spec digest / Outcome / Endpoint / Next). |
| 3 | accepted intent carries `Proto::Udp` | `pending` (structural gap) | the deploy-accept render does NOT render the listener protocol, and no read surface (`od job list`, `od alloc status`) renders it. The 01-05 direct-handler test proves it via a crate back-door (`LocalIntentStore` read) unavailable to a black-box runner. `crates/overdrive-cli/tests/integration/deploy_udp_walking_skeleton.rs` is the `what, forever` witness. A read surface that renders the proto would close this gap. |

Sub-claim 3 stays `pending` until an operator surface renders the listener
protocol (an honest gap, not a failure); until then the authoritative proof of
`Proto::Udp`-threaded is the direct-handler test `deploy_udp_walking_skeleton.rs`,
which spawns serve with an injected SimDataplane and never touches XDP.
