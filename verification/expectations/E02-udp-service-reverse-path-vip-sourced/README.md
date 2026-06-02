# E02 — a deployed UDP service's reply is sourced from the VIP, not the backend IP

**Surface:** E (end-to-end) · **KPI:** K1 (UDP reverse-path success) · **Status:** `pending`

<!-- Status rationale (2026-06-02, SHA ea3c86f6, SEED=1, executed_in_lima: true):
every sub-claim is topology-bound and none is capturable black-box on this VM.
The runner now captures a best-effort `bpftool map dump name REVERSE_NAT_MAP`
up front (CP-independent) — it fails ("can't find map"), which is honest
NEGATIVE evidence for sub-claim 2: no running dataplane ever pinned the map.
The reason it never will here is the same one O03 hit — production `overdrive
serve`'s EbpfDataplane attaches XDP to `lo` at boot and the attach FAILS on
this VM (xdp_reverse_nat_lookup.attach(lo, DRV_MODE): bpf_link_create failed;
see O03 evidence/serve.log) — so serve never binds, no deploy lands, and no
map is programmed. Sub-claim 3 (VIP-sourced wire capture) additionally needs
the real ThreeIfaceTopology veth/netns rig, which the runner deliberately does
NOT rebuild (that re-implements a test tier — forbidden by verification.md —
and risks leaked XDP/cgroups on the shared VM). Stays `pending`. The
`reverse_nat_udp_e2e.rs` Tier-3 test is the `what, forever` witness for
sub-claims 2+3; this expectation is the design-time `why`. -->


## Expectation

A single-UDP-listener service deployed end-to-end through a real control
plane and a real kernel completes a UDP round-trip whose **reply is sourced
from the VIP (`10.96.0.10:5353`), not the backend IP**. The reverse path
rewrites the connectionless UDP datagram's source the same way the TCP path
does — closing the #163 defect class where a UDP backend reply leaked its
own address.

This is the **full K1 proof** of the S-04-A walking skeleton: deploy →
`REVERSE_NAT_MAP` carries the `(backend_ip, 5353, udp)`→VIP key → the wire
capture shows the VIP source. It is the qualitative, human-readable `why`
for the regression alarm that already exists as the passing Tier-3 test
`crates/overdrive-dataplane/tests/integration/reverse_nat_udp_e2e.rs` — per
the Stabilize doctrine in `.claude/rules/verification.md`, that test is the
`what, forever`; this expectation is the design-time conversation it
guards.

- Anchor: S-04-A (`docs/feature/udp-service-support/distill/test-scenarios.md` — reverse path carries the VIP source; `bpftool` dump + wire capture)
- Anchor: K1 — UDP reverse-path success (`docs/feature/udp-service-support/feature-delta.md` § Outcome KPIs, 0%→100%; test-scenarios.md K-mapping: "K1 → S-04-A (wire capture source==VIP + `bpftool` dump)")
- Anchor: roadmap 01-03 (`docs/feature/udp-service-support/deliver/roadmap.json` — "Tier 3 walking skeleton: single UDP listener forward+reverse e2e via real `overdrive deploy` subprocess")
- Anchor: ADR-0060 (`docs/product/architecture/adr-0060-service-frontend-update-service-signature.md` — both adapters derive REVERSE_NAT keys from the same typed `ServiceFrontend`; proto=udp flows to the kernel rewrite)
- Anchor: US-04 (`docs/feature/udp-service-support/feature-delta.md` § Walking skeleton)

## Verification

Precondition: this is a Tier-3 surface. The `REVERSE_NAT_MAP` dump and the
wire capture both require the real `overdrive-testing` `ThreeIfaceTopology`
netns/veth setup, run as root inside Lima (`cargo xtask lima run --`). The
runner checks for the topology and a reachable control plane; if either is
absent it prints the exact setup commands and exits `pending` rather than
narrate the capture (leaked cgroups/XDP across runs are a documented
hazard, see `.claude/rules/{testing,debugging}.md`).

The runner deploys a UDP `dns-resolver.toml` (udp/5353 + backend on 5353)
through Lima, sends a UDP datagram to the VIP, and captures `bpftool map
dump REVERSE_NAT_MAP` plus a `tcpdump` on the client veth verbatim.
Sub-claims:

1. The deploy exits `0` and prints `Accepted.` (inherits O03; included here
   as the precondition of the e2e path, not re-proven).
2. `bpftool map dump REVERSE_NAT_MAP` shows the `(backend_ip, 5353, udp)`
   key mapping to the VIP (the D — dataplane/kernel — sub-claim).
3. The wire capture on the client veth shows the backend's reply **sourced
   from the VIP (`10.96.0.10:5353`)**, never the backend IP (the #163 defect
   guard).

`satisfied` requires sub-claims 2 and 3 on a Lima run with the topology up,
SHA + seed pinned in `evidence/verification.yaml`. A runner that can only
bring up part of the chain captures what it can and leaves the rest
`pending` — it must not mark a sub-claim it did not execute. The seed-pinned
Lima capture is the design-time snapshot; `reverse_nat_udp_e2e.rs` is the CI
regression alarm that fails loudly when the surface drifts.

## Evidence

Captured under `evidence/` by `harness/run-expectation.sh E02` (SHA
`ea3c86f6`, `SEED=1`, `executed_in_lima: true` — see
`evidence/verification.yaml`).

- `evidence/reverse_nat_map_dump_preflight.out` / `.meta` — best-effort
  `bpftool map dump name REVERSE_NAT_MAP`, run CP-independently in Lima. It
  fails (exit 1, "can't find map") because no running dataplane pinned the map
  — honest NEGATIVE evidence for sub-claim 2. The map never appears here because
  production serve's XDP attach to `lo` fails at boot (see O03
  `evidence/serve.log`), so no deploy/convergence ever programs it.
- `evidence/preflight_cluster.out` / `.meta` — `od cluster status` ran in Lima
  and failed with `failed to reach overdrive control plane … could not connect
  to server` (exit 1). No control plane was up; the e2e path could not start.
- `evidence/run.log` — the runner captured the absent map dump + the unreachable
  CP and exited `pending` for every sub-claim rather than narrate the dataplane
  chain (leaked cgroups/XDP across runs are a documented hazard).

Per-sub-claim verdict:

| # | Sub-claim | Verdict | Reason |
|---|---|---|---|
| 1 | deploy exits `0` + `Accepted.` (precondition) | `pending` | not reached — preflight failed (no CP) |
| 2 | `REVERSE_NAT_MAP` shows `(backend_ip,5353,udp)`→VIP | `pending` | not reached — no running dataplane/topology to dump (`bpftool map dump name REVERSE_NAT_MAP` never ran) |
| 3 | wire reply sourced from VIP `10.96.0.10:5353`, not backend IP | `pending` | not reached — requires `ThreeIfaceTopology` veth + live UDP round-trip + `tcpdump` on the client veth |

Sub-claims 2+3 are Tier-3 surfaces requiring the real `overdrive-testing`
`ThreeIfaceTopology` setup run as root in Lima. The runner does NOT reproduce
that black-box (the topology helpers are crate code; a half-built veth + leaked
XDP is a documented hazard) and does NOT run `reverse_nat_udp_e2e.rs` as
evidence (it links crates — a rejected fifth tier). The `what, forever` witness
for sub-claims 2+3 (and the #163 VIP-source guard) is
`crates/overdrive-dataplane/tests/integration/reverse_nat_udp_e2e.rs`; this
expectation is the design-time `why` that test guards (Stabilize doctrine,
`.claude/rules/verification.md`).

To capture: bring up `cargo overdrive serve …` and the `ThreeIfaceTopology`
veth/dataplane in a Lima-routed terminal, then re-run
`harness/run-expectation.sh E02`.
