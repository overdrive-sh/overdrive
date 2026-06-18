# SPIKE Decisions — transparent-mtls-enrollment (GH #236)

## Assumption Tested (Probe A)

On the running kernel, can a node-agent recover a workload's ORIGINAL outbound
destination after a `cgroup/connect4` BPF program redirected the connection to the
agent's local proxy leg (leg-F), using a kernel-side stash +
`cgroup/getsockopt(SO_ORIGINAL_DST)`, with **NO pre-programmed per-destination map
entry**?

## Probe Verdict: DOESN'T-WORK

Ran for real as root on Lima kernel **7.0.0-22-generic aarch64** (aya-rs Rust,
self-contained `spike-scratch/increment-a/`, zero `crates/` touch).
`getsockopt(SOL_IP, SO_ORIGINAL_DST)` returned `ENOENT`. The `connect4` redirect
itself works (it captured the real dst via the witness map, confirmed by `bpftool
map dump`) — only the cross-socket *recovery* is dead. Three independent walls,
any one fatal:

1. `connect4` fires **before** the ephemeral source port binds → cannot key a
   stash by the 4-tuple the accept side could reconstruct.
2. A `connect4` sockaddr **rewrite is not a netfilter DNAT** → conntrack holds no
   original tuple → `SO_ORIGINAL_DST` = `ENOENT`.
3. `cgroup/getsockopt` fires **only for tasks inside** the attached cgroup; the
   agent runs *outside* the workload cgroup. The workload's connect socket ≠ the
   agent's accept socket (per-socket storage/cookie not shared), and
   `bpf_get_socket_cookie` is **verifier-forbidden** in `cgroup/getsockopt`.

## Cilium reconciliation (read-only research — Cilium `main` @ `dac977e678`, v1.20.0-dev)

Cross-checked the verdict against Cilium's production datapath. **Zero
`cgroup/getsockopt` and zero `SO_ORIGINAL_DST`** in Cilium's entire BPF tree —
it never uses Probe A's shape. Cilium has two outbound paths:

- **Socket-LB** (`bpf/bpf_sock.c`) rewrites **direct to the backend** (no proxy
  hop → no recovery problem). Its socket-cookie reverse map (`cilium_lb4_reverse_sk`,
  `bpf/lib/sock.h`) works *only* because the forward-write (`connect4`) and
  reverse-read (`getpeername4`) are the **same workload socket** — not applicable
  to a mediating proxy with a different accept socket.
- **Mediating proxy** (the case we have): **TPROXY via `bpf_sk_assign` + fwmark
  `0x0200` → `IP_TRANSPARENT` listener → `getsockname` (TCP) / `IP_RECVORIGDSTADDR`
  (UDP)** — `bpf/lib/proxy.h`, `bpf/bpf_lxc.c`, `pkg/fqdn/dnsproxy/`.

Cilium **independently confirms the pivot**: TPROXY + `getsockname`, not
`connect4` + `getsockopt`. The in-tree DNS proxy (`pkg/fqdn/dnsproxy/`) is a
worked example of a transparent node-agent proxy.

## Promotion Decision: PIVOT → Path A

PIVOT (verdict DOESN'T-WORK). `increment-a/` is **preserved** (gitignored
evidence), not deleted.

**Chosen direction — Path A:** give exec workloads their own **netns+veth** and use
the project's existing **nft-TPROXY for BOTH directions**. The workload's egress
then *ingresses* the host-side veth (PREROUTING), where nft-TPROXY applies →
`IP_TRANSPARENT` leg-F → `getsockname` recovers orig-dst — **symmetric with the
already-proven inbound half** (`transparent-mtls-host-socket/spike/findings-inbound-intercept.md`).
This reuses our proven mechanism (no new `bpf_sk_assign` / BPF surface) and matches
Cilium's topology (workload-in-netns). The decision rationale: nft-TPROXY beats
`bpf_sk_assign` *for us* not because the kernel primitive is better, but because we
already run nft-TPROXY inbound — Path A unifies both directions on one proven
mechanism.

## Design Implications (for DESIGN)

- **The topology change is the load-bearing decision.** Single-node v1 today runs
  workloads in the **host netns** (`veth_provisioner.rs:36-37` — "Single-node runs
  entirely in the host netns"). Path A requires a **per-workload netns+veth**. The
  `ExecDriver` already carries an opt-in `setns(CLONE_NEWNET)` hook
  (`driver.rs:181-186`) — the seam exists.
- **Recovery mechanism = TPROXY + `getsockname`, both directions.** Unify outbound
  onto the inbound nft-TPROXY path; the #234 shared-routing infra (fwmark `ip rule`
  + `local` route + shared nft `prerouting` chain) extends to egress.
- **`cgroup/connect4`-rewrite-to-local-leg + cross-socket recovery is RETIRED.**
  `program_declared_peer_redirect` (the test-only seam) is NOT the production path.
- **The agent still needs the peer identity to mTLS to** — that is the resolve
  layer (#178); enforce (this feature) consumes it. Per-connection resolution in
  the agent (enrollment model, #236) replaces the dead per-destination map.
- **Open for DESIGN:** Path A's *egress* nft-TPROXY on a per-workload veth +
  `getsockname` recovery is **UNVALIDATED on our exact topology** (Cilium proves
  the model; our wiring is unproven). Whether to de-risk via a thin spike
  (`increment-b/`) or during DELIVER is a DESIGN call.

## Constraints Discovered

- **No Tier-2 backstop** for `cgroup_sock_addr` / `cgroup_sockopt` (`BPF_PROG_TEST_RUN`
  returns ENOTSUPP) — any kernel-interception change must be validated by a real
  connect under Lima / Tier-3, never a `--no-run` gate.
- Kernel: dev Lima **7.0.0**; pinned appliance **6.18** is the authoritative merge
  signal (ADR-0068). TPROXY / `IP_TRANSPARENT` / `getsockname` are all far under
  any plausible floor.

## increment-b — Path A egress validation (Q1) — VERDICT: WORKS

Ran `spike-scratch/increment-b/` (Rust + `ip`/`nft` CLI, **no eBPF**; self-contained,
gitignored) — a real `connect()` from inside a workload netns, as root on Lima
kernel `7.0.0-22-generic`. **Path A egress is validated.**

- **`getsockname` recovered the dialed peer**, not leg-F: accepted socket
  `getsockname = 10.200.0.1:18777` == the real-backend the workload dialed
  (leg-F was `127.0.0.1:28777`). No per-destination map, no orig-dst loss;
  reproduced twice with fresh ephemeral ports.
- **nft-TPROXY fires for workload-netns egress at the host-side veth ingress**
  (PREROUTING) — proven by a without-rule control (reached backend directly) vs
  with-rule (landed on leg-F). Egress does NOT need OUTPUT-path TPROXY; the
  production `install_inbound_tproxy` recipe mirrors cleanly to the active side.
- **The F5 leg-dial `SO_MARK` exemption is load-bearing, not precautionary:** the
  agent's own host-netns dial IS captured by PREROUTING via the `ip route local …
  table 100` re-injection; the negative control (exemption removed) looped into
  leg-F. This is the production `MTLS_LEG_S_DIAL_MARK` discipline, confirmed
  mandatory for the egress direction too.

**Q1 gate: SATISFIED.** Path A is now validated end-to-end at the mechanism level
(egress here + the already-proven inbound). No routing-shape change needed.

**Converge-on-boot prerequisites surfaced for DESIGN/DELIVER** (detail in
`findings-egress-tproxy.md`): per-workload netns+veth (the `ExecDriver` `setns`
seam), `ip_forward=1`, `rp_filter` relaxation on the ingress veth, and the
leg-dial `SO_MARK`.

## increment-c — 02-06 adopt-on-restart runtime validation (SPIKE-A/B/C) — VERDICT: WORKS

Ran `spike-scratch/increment-c/` (Rust + libc/nix + `ip` CLI, **no eBPF/C**;
self-contained, gitignored) as root on Lima kernel `7.0.0-22-generic`, plus a
read-only code investigation for SPIKE-B. Full evidence in
`findings-adopt-restart.md`.

- **SPIKE-A — workload survival: WORKS.** A workload spawned with the production
  ExecDriver shape (`setsid()` + `kill_on_drop(false)` + own cgroup-v2 scope)
  SURVIVES a SIGKILL of its "serve" parent — reparented to ppid=1, `Ss` (own
  session), still enrolled in its `cgroup.procs`, still in its `ovd-ns-<slot>`
  netns. A CP restart leaves workloads running, so the allocator must ADOPT their
  existing slots, never re-assign smallest-free onto a survivor.
- **SPIKE-B — boot re-adoption: reconciler does NOT re-drive survivors (CONFIRMED).**
  `WorkloadLifecycle::reconcile` emits `Action::StartAllocation` only in the
  "No Running, no failed-needs-restart → schedule a fresh allocation" branch
  (`workload_lifecycle.rs:708`). An already-`Running` survivor matches the Running
  branch and emits no Start action, so the C3 `on_alloc_running` slot-rebuild never
  fires on restart. The C6 "rebuilt on restart by re-assigning for every
  still-Running alloc" premise has **no trigger** → a dedicated adopt-on-restart
  boot pass (02-06) is mandatory, not optional.
- **SPIKE-C — PID→netns slot recovery for a survivor: WORKS.** After the parent is
  dead, `/proc/<survivor>/ns/net` inode == `stat(/var/run/netns/ovd-ns-<slot>).st_ino`
  and `ip netns identify` recovers the name. The 02-06 binding-recovery mechanism
  (cgroup scope → PID → `/proc/ns/net` inode → `NetSlotAllocator::adopt`) is viable.

## Promotion Decision (increment-c): PROCEED-AS-DESIGNED

PROCEED (verdict WORKS on all three, 2026-06-18). The 02-06 adopt-on-restart
design (D-TME-12 "Amended 2026-06-18 (02-06 adopt-on-restart)" in
`design/wave-decisions.md`) is **VALIDATED** — no pivot. SPIKE-B strengthens it:
the boot pass is required precisely because nothing else rebuilds the slot↔alloc
map. `increment-c/` is preserved (gitignored evidence), not deleted. Building 02-06
still waits on 04-03 (the C3 wiring being live in production); this spike de-risked
the design ahead of that step.

**Design implications carried to 02-06** (detail in `findings-adopt-restart.md`):
adopt-don't-reassign (SPIKE-A); the dedicated `run_server` boot recovery pass is the
only trigger (SPIKE-B); recover bindings via cgroup→PID→`/proc/ns/net`→`adopt`,
GC orphan netns with no live PID (SPIKE-C). The inherited-fd hang the probe itself
hit (a survivor holding a parent's pipe/stdout → EOF never arrives) is a real
production hang vector — any survivor-facing fd must be close-on-exec / detached.

## Spike code

`spike-scratch/increment-a/` (gitignored): self-contained aya-rs workspace
(`ebpf/` + `loader/`), preserved as Probe-A evidence (verdict PIVOT).
`spike-scratch/increment-b/` (gitignored): self-contained Rust + `ip`/`nft`
egress-TPROXY harness, preserved as Q1 evidence (verdict WORKS).
`spike-scratch/increment-c/` (gitignored): self-contained Rust + libc/nix + `ip`
adopt-on-restart harness, preserved as 02-06 evidence (verdict WORKS; PROCEED).
No Phase-3 walking skeleton run — DESIGN is already complete; the walking skeleton
is DELIVER's first slice.
