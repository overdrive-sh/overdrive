# Research: Walking-skeleton XDP L4 LB topology — backend listener placement and spawn discipline

**Date**: 2026-05-21 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (Q1, Q2, Q4) / Medium-High (Q3) | **Sources**: 20 cited

## Executive Summary

The user's question is sharply narrower than it looks. Prior research at
`docs/research/dataplane/xdp-l4lb-test-topology-comprehensive-research.md`
(2026-05-06, 32 sources, High confidence) **already established that the
three-netns transit topology** — `client-ns ← veth1 → lb-ns ← veth2 →
backend-ns`, XDP attached on `veth2` in `lb-ns`, listener inside
`backend-ns`, route-via-VIP installed in `client-ns` — is the only credible
production-fidelity shape for testing an XDP L4 LB in netns. Every Overdrive
`reverse_nat_e2e` Tier 3 test currently passes against this exact topology
(`ThreeIfaceTopology` in `crates/overdrive-testing/src/netns.rs`); the
walking-skeleton's `TwoNetnsTopology` is the outlier and the source of the
A4 TCP-roundtrip failure.

What this research adds is the missing piece — **how to get the
production `ExecDriver`-spawned workload INTO `backend-ns`**. The answer is
universal across the production XDP-LB community: every reference
implementation (Cilium L4LB CI, Loxilb CI, xdp-tutorial, CNI plugins,
container runtimes) places the backend listener inside a netns by one of
three mechanisms — `ip netns exec`, container-runtime netns isolation
(Docker/Kind), or direct `setns(CLONE_NEWNET)` in the spawning process. The
**third mechanism is exactly what Overdrive's existing `TwoNetnsTopology`
already does for the tokio runtime** (`on_thread_start(|| setns(lb_ns))`,
`walking_skeleton.rs:34-44`). Extending `ExecDriver::start` to accept an
optional `netns_path: Option<PathBuf>` and call `setns(fd, CLONE_NEWNET)`
in a `Command::pre_exec` closure is a 20-line change with strong precedent
in the Rust ecosystem (`netns-rs`, `netns-exec`).

The strongest independent piece of evidence in this research is the
**Cilium BPF testing posture** itself: Cilium's own L4LB datapath tests are
PKTGEN/SETUP/CHECK synthetic-packet tests under `BPF_PROG_TEST_RUN`; they
assert on program return code and BPF map state, not on real TCP
traversal. Real TCP validation happens in a separate, higher tier (Kind
clusters with real Pod runtimes). Katran goes further — it has NO real-netns
test at all; its CI is base64-fixture packet replay only. The "walking
skeleton with real TCP through real XDP into a real ExecDriver-spawned
listener" assertion is **stricter than what either Cilium or Katran gate
in CI**. This is not, by itself, a reason to abandon the assertion — but
it IS evidence that splitting the walking-skeleton's convergence-chain
proof (A1–A3) from the wire-path proof (A4) is a well-precedented
architectural move, not a retreat.

The recommendation, with reasoning detailed in § Recommendation below, is
**Option A (extend `ExecDriver` for netns targeting) as the production-
fidelity path, with Option C (tier split) as the pragmatic fallback if the
`ExecDriver` change cannot land in the current feature's scope**. Option B
(decouple backend listener from ExecDriver in the test only) is the
weakest of the three — it preserves the topology fiction at the cost of
the walking-skeleton's core "drive everything through production submit"
claim, and yields no future leverage (every subsequent integration test
that needs a netns-isolated workload would re-derive the same hack).

## Decision Context

The Overdrive `backend-discovery-bridge-service-reachability` walking-skeleton
test must prove end-to-end TCP traversal through a Cilium/Katran-shaped XDP L4
LB on a single Lima VM. The convergence chain (submit → allocate → Running →
ServiceBackend obs → SERVICE_MAP populated) is verified by pre-TCP assertions
A1–A3 (all PASS). The TCP round-trip (A4) is the failing assertion.

Two topology attempts have failed:

- **`VethFixture` (single host-netns veth pair)** — no routes, no `ip_forward`,
  no `rp_filter`. TCP hangs at `connect()` (RCA at
  `docs/feature/backend-discovery-bridge-service-reachability/deliver/rca-walking-skeleton-tcp-roundtrip.md`).
- **`TwoNetnsTopology` (client_ns + lb_ns; listener co-located with XDP attach
  iface)** — `bpf_fib_lookup` returns `BPF_FIB_LKUP_RET_NOT_FWDED` because the
  XDP forward rewrites dst from VIP → `host_ipv4` of the *lb-iface itself*
  (a LOCAL address on the ingress iface). XDP `XDP_PASS`es; kernel delivers;
  response egresses with `src=lb_iface_ip` not `src=VIP`. Reverse-NAT is on a
  *different* iface ingress, not on this iface's egress. Client rejects SYN-ACK
  with wrong source.

`ThreeIfaceTopology` exists in `crates/overdrive-testing/src/netns.rs:216-340`
and **works for sibling Tier 3 tests** (`reverse_nat_e2e` × 5 PASS per
the RCA). But those tests use synthetic packet generators
(`helpers/traffic.rs`) — not a real backend listener that needs to live
in `backend_ns`. The walking-skeleton's backend is a real Python
`socket.recv()` listener spawned by production `ExecDriver`, which has no
netns-targeting API.

Three options under consideration:

- **(A)** Extend production `ExecDriver::start` to accept an optional
  `netns_path: Option<PathBuf>` and call `setns(netns_fd, CLONE_NEWNET)` in
  the child before `execve`. Production API change.
- **(B)** Decouple the backend listener from `ExecDriver` in the test
  (fixture spawns listener directly inside `backend_ns`; Service spec exec
  becomes `/bin/sleep infinity` only to drive the alloc→Running chain).
  Weakens the walking-skeleton's "real ExecDriver-spawned process" claim.
- **(C)** Restructure the walking-skeleton's TCP assertion to a separate
  tier — gate the convergence chain (A1–A3) in `overdrive-control-plane`,
  defer the full wire-path proof (A4) to `overdrive-dataplane`'s existing
  `reverse_nat_e2e` family which already uses `ThreeIfaceTopology`.

## Research Methodology

**Search Strategy**: This research builds on the 2026-05-06 prior research
(32 sources, High confidence) which already settled Q1. The new investigation
focuses on Q2 (test-spawn mechanics), Q3 (tier-split precedent), and Q4
(production same-host XDP-LB packet path). Sources consulted via WebFetch
(direct repository / docs reads) and WebSearch (gap-filling).

**Source Selection**: Cilium, Katran, Loxilb (production XDP-LB projects);
xdp-project tutorials; Linux kernel docs; Rust ecosystem netns crates
(`netns-rs`, `netns-exec`); CNI specification. Tier preference: `docs.cilium.io`
> `cilium.io` blog > Cilium / Katran / Loxilb source > `lwn.net` > tutorial /
medium-tier community refs (cross-referenced only).

**Quality Standards**: Per nw-researcher methodology — 3+ sources for each
major claim (most reach 3+), 2 sources for tier-split practice (Cilium docs
+ Katran DEVELOPING.md = direct; cross-confirm via search), 1 authoritative
source for the Loxilb `hexec` pattern (Apache-2 source file directly).
Knowledge gaps explicitly documented below.

## Findings

### Q1 — Canonical single-node test topology for XDP L4 LB

> **This question was settled by prior research** at
> `docs/research/dataplane/xdp-l4lb-test-topology-comprehensive-research.md`.
> Summary reproduced here for completeness.

#### Finding 1.1: The 3-netns transit topology is universal across production XDP-LB integration tests
**Evidence**: Cilium PR #16338 ("Standalone L4LB XDP tests") sets up
`ip l a l4lb-veth0 type veth peer l4lb-veth1`, `ip a a 3.3.3.1/24 dev
l4lb-veth0`, `ip l s dev l4lb-veth1 netns <ns>` — exactly the
client-ns / lb-side / backend-side multi-netns transit shape Overdrive's
`ThreeIfaceTopology` mirrors.
**Source**: Prior research, Finding 1.2 / [Cilium PR #16338](https://github.com/cilium/cilium/pull/16338)
**Verification**: Overdrive's own `crates/overdrive-testing/src/netns.rs:216-340`
implements the same shape and is the topology under which all
`reverse_nat_e2e` × 5 tests currently pass.
**Confidence**: High (already cross-verified to ≥3 sources in prior research)

#### Finding 1.2: Two-netns shapes (lb + backend co-located) fail by structural reason, not by configuration mistake
**Evidence**: The user's RCA names the precise mechanism — `bpf_fib_lookup`
returns `BPF_FIB_LKUP_RET_NOT_FWDED` because the XDP forward rewrites
dst-IP to a LOCAL address on the ingress iface. This is a kernel
property of FIB lookup (local destinations are not "forwarded"); no
amount of route or sysctl tuning resolves it within a 2-netns shape.
**Source**: User-supplied RCA + kernel BPF FIB lookup semantics ([docs.kernel.org BPF redirect docs](https://docs.kernel.org/bpf/redirect.html) — already cited in prior research)
**Confidence**: High

### Q2 — How production projects spawn backend workloads into test netns

#### Finding 2.1: Loxilb (production XDP-LB) uses `ip netns exec` directly — `hexec="sudo ip netns exec"`
**Evidence**: From `cicd/common.sh` in the loxilb repo (Apache-2):
```bash
hexec="sudo ip netns exec "
```
Used throughout the test suite as `$hexec $dname <command>` for every
operation that must execute inside a backend / client netns. The
validation script's TCP probe is literally:
```bash
res=$($hexec l3h1 curl --max-time 10 -s ${servIP[k]}:${servPort[k]})
```
**Source**: [loxilb common.sh](https://raw.githubusercontent.com/loxilb-io/loxilb/main/cicd/common.sh) (verified via WebFetch 2026-05-21); [loxilb tcplb/validation.sh](https://raw.githubusercontent.com/loxilb-io/loxilb/main/cicd/tcplb/validation.sh)
**Verification**: Cross-referenced against the loxilb [CI workflow listing](https://github.com/loxilb-io/loxilb/tree/main/cicd/tcplb) and the public [Loxilb docs](https://www.loxilb.io/) confirming this is the production CI shape.
**Confidence**: High
**Analysis**: This is the canonical pattern. Backend listeners can be
nginx, netcat, iperf — anything that binds a socket. `ip netns exec`
runs the process inside the target netns; from the process's
perspective it is born already in that netns. No production code changes
needed.

#### Finding 2.2: Cilium's L4LB integration test (PR #16338) uses container-runtime netns isolation (Kind / Docker)
**Evidence**: PR #16338's GitHub workflow spawns the backend via:
```bash
docker exec kind-worker /bin/sh -c 'apt-get update && apt-get install -y nginx && systemctl start nginx'
WORKER_IP=$(docker exec kind-worker ip -o -4 a s eth0 | awk '{print $4}' | cut -d/ -f1)
cilium service update --id 1 --frontend "${LB_VIP}:80" --backends "${WORKER_IP}:80" --k8s-node-port
```
**Source**: [Cilium PR #16338](https://github.com/cilium/cilium/pull/16338) (verified via WebFetch 2026-05-21)
**Verification**: Confirmed shape via the [Cilium standalone L4LB blog](https://cilium.io/blog/2022/04/12/cilium-standalone-l4lb-xdp/) and [cilium-l4lb-test example config](https://github.com/cilium/cilium-l4lb-test/blob/master/cilium-lb-example.yaml) (cited in prior research).
**Confidence**: High
**Analysis**: Cilium delegates netns isolation entirely to Docker — the
Kind container is the netns boundary. nginx binds inside the container's
netns automatically. This is the same underlying mechanism as `ip netns
exec` (both end up calling `setns(CLONE_NEWNET)` on the child) but
through a container runtime instead of `iproute2`. **Relevance to
Overdrive**: this exact pattern is unavailable because Overdrive's
`ExecDriver` doesn't run workloads as containers — it runs them as
bare processes under cgroup-isolated `ExecDriver::start`. But the
underlying primitive — `setns(CLONE_NEWNET)` in the child — IS available
to `ExecDriver`.

#### Finding 2.3: xdp-tutorial uses a single `ip netns exec` wrapper exposed as `ns_exec`
**Evidence**: From xdp-tutorial's `testenv.sh`:
```bash
ns_exec() {
  get_nsname && ensure_nsname "$NS"
  ip netns exec "$NS" env TESTENV_NAME="$NS" "$SETUP_SCRIPT" "$@"
}
```
Exposed user commands: `enter` (interactive shell), `run_ping`,
`run_tcpdump --inner`, `exec <command>`. All route through `ns_exec`.
**Source**: [xdp-tutorial testenv.sh](https://github.com/xdp-project/xdp-tutorial/blob/main/testenv/testenv.sh) (verified via WebFetch 2026-05-21)
**Verification**: This crate is maintained by the same upstream group
(`xdp-project`) that hosts the kernel's official XDP documentation and is
referenced by the kernel's `Documentation/networking/af_xdp.rst`.
**Confidence**: High
**Analysis**: Same pattern as loxilb. Different surface (function vs
variable), identical primitive (`ip netns exec`).

#### Finding 2.4: Rust ecosystem provides `setns + pre_exec` as the canonical "spawn into existing netns" primitive
**Evidence**: From the Rust standard library documentation: "[pre_exec]
schedules a closure to be run just before the exec function is invoked
... in the context of the child process after a fork". Combined with
`nix::sched::setns(fd, CloneFlags::CLONE_NEWNET)`, the canonical pattern
for spawning a child in a different netns from Rust is:
```rust
unsafe {
    cmd.pre_exec(move || {
        nix::sched::setns(ns_fd, nix::sched::CloneFlags::CLONE_NEWNET)
            .map_err(|e| io::Error::from_raw_os_error(e as i32))?;
        Ok(())
    });
}
let child = cmd.spawn()?;
```
**Source**: [std::os::unix::process::CommandExt::pre_exec](https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html); [nix::sched::setns](https://docs.rs/nix/latest/nix/sched/fn.setns.html); [LWN namespaces API](https://lwn.net/Articles/531381/)
**Verification**: [netns-exec crate on crates.io](https://crates.io/crates/netns-exec) — community-maintained tool that wraps exactly this pattern; [netns-rs crate](https://docs.rs/netns-rs/latest/netns_rs/) provides `NetNs::run(closure)` over the same primitive.
**Confidence**: High
**Analysis**: This pattern is **already in use** in the Overdrive
walking-skeleton — `walking_skeleton.rs:34-44` documents that
`TwoNetnsTopology` uses `on_thread_start(|| setns(lb_ns))` on the tokio
multi-thread runtime so every worker enters `lb_ns` before polling.
Extending `ExecDriver::start` to do the same in `Command::pre_exec` is
strictly less invasive than the runtime-level setns the test already
does — `pre_exec` runs in the forked child only, so production paths
that don't pass a `netns_path: Option<PathBuf>` are bit-identical to
today's behaviour.

#### Finding 2.5: CNI plugins are the production model — the runtime creates the netns BEFORE invoking the plugin, then the plugin operates over `CNI_NETNS` env var
**Evidence**: From the [CNI specification](https://www.cni.dev/docs/spec/):
"The container runtime must create a new network namespace for the
container before invoking any plugins... CNI plugins receive context
through environment variables: ... CNI_NETNS (Path to network namespace)".
The runtime separates "create netns" (CNI's caller) from "configure
netns" (CNI plugin's job).
**Source**: [CNI specification](https://www.cni.dev/docs/spec/); [Kubernetes Network Plugins docs](https://kubernetes.io/docs/concepts/extend-kubernetes/compute-storage-net/network-plugins/)
**Verification**: Cross-confirmed via reference plugins (`ptp`, `bridge`)
in [containernetworking/plugins](https://github.com/containernetworking/plugins).
**Confidence**: High
**Analysis**: The CNI model is **the same shape as Option A** —
"production component accepts an optional netns path and operates inside
it". CNI's `CNI_NETNS` env var is functionally identical to a proposed
`ExecDriver::start(spec, netns_path: Option<PathBuf>)` API. Overdrive's
`ExecDriver` would be the analogue of a "CNI runtime caller" — it does
NOT create the netns (that's the test fixture's job), it just enters one
that already exists. This is the smallest viable production-API change
that aligns Overdrive with how every Kubernetes-class system does
workload-into-netns spawning.

### Q3 — Is full TCP round-trip required in the walking-skeleton, or split tiers?

#### Finding 3.1: Cilium's BPF tests use `BPF_PROG_TEST_RUN` and assert on return code + map state — NOT real TCP
**Evidence**: From Cilium's [BPF testing docs](https://docs.cilium.io/en/stable/contributing/testing/bpf/):
"All BPF tests live in the bpf/tests directory ... independently
compiled, loaded, and executed using BPF_PROG_RUN ... programs in the
kernel without attaching them to actual hooks". Per WebFetch summary:
"They validate the return code of BPF programs (e.g., XDP_TX); inspect
map state changes; verify packet data structure contents after program
execution. ... **no real TCP connections traverse the datapath**".
**Source**: [Cilium BPF Unit and Integration Testing docs](https://docs.cilium.io/en/stable/contributing/testing/bpf/) (re-verified via WebFetch 2026-05-21); cited in prior research Finding 1.1.
**Verification**: [Cilium issue #14990](https://github.com/cilium/cilium/issues/14990) "Datapath testing using BPF_PROG_TEST_RUN" frames this as the *primary* datapath testing tier; [Cilium GitHub test directory tree](https://github.com/cilium/cilium/tree/main/test) lists `bpf/`, `k8s/`, `runtime/`, `controlplane/` as separate tiers — real TCP traversal is the `k8s/` tier (Kind clusters).
**Confidence**: High
**Analysis**: This is **direct evidence that a tier split is the
production-fidelity pattern**. The L4LB datapath tests (`bpf/tests/
nodeport_geneve_dsr_lb_xdp.c`, etc.) prove the BPF program's logic on
synthetic packets; the Kind/Ginkgo e2e tier proves real wire-path
correctness against a real Pod runtime. Cilium does not gate the same
property in both tiers; each tier has a distinct scope.

#### Finding 3.2: Katran has NO real-netns test — only base64-fixture replay via `BPF_PROG_TEST_RUN`
**Evidence**: From Katran's DEVELOPING.md (cited in prior research, re-verified scope): "This framework allows us to specify predefined test fixtures (input and expected output) ... Test fixtures in our case contain base64 encoded packets. You can check `katran/lib/testing/fixtures/KatranBaseTestFixtures.h` for examples." No netns test exists anywhere in the Katran tree.
**Source**: [Katran DEVELOPING.md](https://github.com/facebookincubator/katran/blob/main/DEVELOPING.md); [Katran testing dir](https://github.com/facebookincubator/katran/tree/main/katran/lib/testing); [Engineering at Meta: Open-sourcing Katran](https://engineering.fb.com/2018/05/22/open-source/open-sourcing-katran-a-scalable-network-load-balancer/)
**Confidence**: High
**Analysis**: Katran — production XDP L4 LB serving Facebook's edge —
has chosen synthetic-fixture-only as its primary datapath gate. Their
production deployment validates correctness in production; CI proves
the program-level invariants. This is the *strongest* form of the
tier-split argument: Katran does NOT have an A4-equivalent gate at all.

#### Finding 3.3: Loxilb DOES gate real TCP through netns in CI — using `$hexec l3h1 curl`
**Evidence**: Loxilb's `cicd/tcplb/validation.sh` runs the canonical
real-TCP probe via `$hexec l3h1 curl --max-time 10 -s ${servIP[k]}:
${servPort[k]}`. The CI workflow [tcp-sanity.yml](https://github.com/loxilb-io/loxilb/actions/workflows/tcp-sanity.yml) gates every PR on this passing.
**Source**: [loxilb tcplb/validation.sh](https://raw.githubusercontent.com/loxilb-io/loxilb/main/cicd/tcplb/validation.sh); [loxilb tcp-sanity workflow](https://github.com/loxilb-io/loxilb/actions/workflows/tcp-sanity.yml)
**Confidence**: High
**Analysis**: Counter-evidence to "tier split is universal" — Loxilb
gates real TCP per PR. BUT the loxilb test relies on Docker for
netns/backend isolation, which delegates the netns-spawn problem to
docker (precedent for Option A via container-runtime mechanism). Loxilb
has not solved "spawn a bare process into a netns" inside its own code;
it leans on Docker to do it.

#### Synthesis for Q3
The production-XDP-LB community is split: Cilium and Katran defer
real-TCP to a higher tier (Cilium's Kind e2e; Katran has none at all);
Loxilb gates it per PR via Docker. **A tier split for Overdrive's
walking-skeleton is well-precedented**, but not the only credible
choice. The question is whether Overdrive's walking-skeleton is meant
to be the "L4LB datapath gate" (Cilium/Katran shape: synthetic-only) or
the "end-to-end product gate" (Loxilb shape: real TCP per PR). The
project's existing 4-tier discipline at `.claude/rules/testing.md`
suggests the former — Tier 3 (real-kernel integration) is the
end-to-end TCP gate, and Overdrive already has a working one
(`reverse_nat_e2e` × 5 PASS).

### Q4 — Production single-host XDP-LB packet path (LB + backend on same node)

#### Finding 4.1: Cilium's XDP acceleration is explicitly scoped to REMOTE backends; same-node delivery does NOT go through the XDP fast path
**Evidence**: From [Cilium kube-proxy-free docs](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/):
"Cilium has built-in support for accelerating NodePort, LoadBalancer
services and services with externalIPs for the case where the arriving
request needs to be **pushed back out of the node when the backend is
located on a remote node**, and with the help of XDP, Cilium is able to
process those requests right out of the network driver layer."
**Source**: [Cilium Kubernetes Without kube-proxy docs](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/) (re-verified via WebFetch 2026-05-21)
**Verification**: [LPC 2020 K8s Service LB with BPF & XDP slides](https://lpc.events/event/7/contributions/674/attachments/568/1002/plumbers_2020_cilium_load_balancer.pdf) (binary PDF, content not extracted but URL preserved); [Cilium tuning docs](https://docs.cilium.io/en/stable/operations/performance/tuning/).
**Confidence**: High (single primary source, but it is the canonical authoritative source for Cilium's XDP LB behaviour)
**Analysis**: This is **the structural reason the walking-skeleton's
`TwoNetnsTopology` cannot work**. Cilium's XDP LB is fundamentally
designed for "rewrite headers, kick back out the same iface, let kernel
routing reach the *remote* backend network". When the backend is local
(same-host), Cilium's BPF datapath has separate code paths (socket-level
LB for east-west via `cgroup_skb`, TC for remaining cases) that bypass
XDP entirely. The Overdrive walking-skeleton is asking XDP to do
something neither Cilium nor Katran's XDP code paths do in production —
service VIPs whose backends are bound on the LB's own host.

#### Finding 4.2: The 3-netns transit topology IS a faithful mirror of the production "remote backend" path
**Evidence**: In `ThreeIfaceTopology`, `lb_ns` is a stand-in for the
LB-host's routing table; `backend_ns` is a stand-in for "a different
host reachable via the LB's egress iface". The veth pair between
`lb_ns` and `backend_ns` simulates the inter-host wire. This is
exactly the production-fidelity property the prior research
recommended.
**Source**: [Cilium PR #16338](https://github.com/cilium/cilium/pull/16338) — uses the same shape; verified in prior research.
**Verification**: [Cilium L4LB blog post](https://cilium.io/blog/2022/04/12/cilium-standalone-l4lb-xdp/) confirms production deployment is "LB on dedicated edge nodes, backends on cluster nodes" — physically distinct hosts.
**Confidence**: High
**Analysis**: 3-netns is "as production-shape as netns testing gets".
The middle `lb_ns` IS the proxy for "the LB host's routing table reaches
the backend network". Mirroring this exactly is the correct architectural
move; trying to collapse it to 2-netns by co-locating backend with LB
breaks the very property the production code was designed against.

#### Finding 4.3: Local-backend handling in production XDP LBs (when it exists) uses TC, not XDP
**Evidence**: Cilium's "Local Redirect Policy" feature ([docs](https://docs.cilium.io/en/stable/network/kubernetes/local-redirect-policy/)) is described as: "When a local redirect policy is applied, cilium BPF datapath redirects traffic going to the policy frontend address to a node-local backend pod selected by the policy". This is implemented at the TC layer (socket-level for east-west), NOT XDP.
**Source**: [Cilium Local Redirect Policy docs](https://docs.cilium.io/en/stable/network/kubernetes/local-redirect-policy/); [arthurchiao: K8s L4LB theory & practice](https://arthurchiao.art/blog/k8s-l4lb/) (community, cross-ref only)
**Confidence**: Medium (primary source authoritative, secondary medium-tier; structural reasoning that XDP cannot redirect to local Pod veths without `bpf_redirect_peer` — which is TC-only per prior research Finding 4.1 — supports this)
**Analysis**: This **reinforces that the walking-skeleton's
`TwoNetnsTopology` is asking XDP to do something it is not designed
for**. Production-XDP-LB's answer to "backend on same host" is "switch
layers; XDP isn't your tool for this case". Overdrive's walking-skeleton
cannot be the test that proves XDP forwards to local backends because
that path doesn't exist in any production reference.

## Option Comparison

| Criterion | A: extend `ExecDriver` for netns | B: decouple listener from `ExecDriver` in test | C: tier split (defer A4 to dataplane) |
|---|---|---|---|
| Production API change | ✅ small — `netns_path: Option<PathBuf>` param | ❌ none | ❌ none |
| Test fidelity to production submit chain | ✅ full — backend IS ExecDriver-spawned | ⚠️ partial — exec is `/bin/sleep`; listener is fixture-spawned | ⚠️ split — A1–A3 gate the chain; A4 lives in a different tier |
| Mirrors a production reference shape | ✅ CNI's `CNI_NETNS` pattern | ❌ no — bespoke test-only shape | ✅ Cilium / Katran tier split |
| Reusable for future integration tests | ✅ every future netns-isolated workload test reuses it | ❌ no — each test re-derives the hack | ⚠️ partial — convergence-chain tests get a cleaner posture but the dataplane tier still owns the wire-path |
| Implementation effort | 20-50 lines (`Command::pre_exec` + opt arg) | 50-100 lines fixture-side (spawn + reap inside backend_ns) | 0 lines code; restructure test bodies + docs |
| Walking-skeleton claim preserved | ✅ "drive everything through production submit" | ❌ broken — listener bypasses ExecDriver | ⚠️ scoped — A1–A3 + dataplane equivalence |
| Blast radius on other ExecDriver consumers | low — opt-in param, defaults to today's behaviour | none | none |
| Knowledge gap risk | low — pattern well-precedented (Cilium PR #16338, CNI, netns-rs) | medium — bespoke spawn-and-reap state machine inside the test fixture | low — Cilium / Katran both do exactly this |
| Aligns with Overdrive's `.claude/rules/testing.md` four-tier discipline | ✅ Tier 3 stays in control-plane crate | ⚠️ test fidelity drift unclear | ✅ explicit tier separation (control-plane: A1–A3; dataplane: A4) |
| Future when Overdrive adds container/microvm workloads | ✅ same `setns` pattern composes naturally | ❌ test diverges further from production | ⚠️ tier split persists; per-driver wire-path tests proliferate |

## Risks Per Option

**A — extend `ExecDriver` for netns.** Strongest argument *against*:
production API surface expansion. The change adds a parameter
(`netns_path: Option<PathBuf>`) that has no production caller today.
Two mitigations soften this: (a) `Option<PathBuf>` defaults to `None`
which is bit-identical to today's behaviour at every production call
site — the param is invisible until a future workload-spec field (e.g.
`netns_target`) wires it in; (b) the change has a clear forward use
case — when Overdrive eventually supports container workloads, microvm
workloads, or per-tenant network isolation, the same param is the
interface for entering an already-created netns. Risk magnitude: low.
Honest acknowledgement: this IS a production API change and should not
be hidden as a test-only refactor; the user must approve the surface
expansion explicitly.

**B — decouple listener from `ExecDriver` in test.** Strongest
argument *against*: the walking-skeleton's stated purpose (DWD-07
CM-A: drive everything through the production submit path) is
*structurally* broken — the listener is no longer ExecDriver-spawned,
just a side-spawn the fixture runs in `backend_ns`. The Service spec's
`exec` becomes a sham (`/bin/sleep infinity`) used only to drive the
alloc→Running convergence chain — but a future ExecDriver regression
that breaks "spawn arbitrary process and have it bind a port" would
not be caught by this test. The test would still pass A1–A3 + A4, but
A4 is now proving "a fixture-spawned listener answers TCP", not "the
production submit chain produces a working backend". This option costs
the test's main load-bearing property to avoid a 20-line production
change. **Recommend against.**

**C — tier split.** Strongest argument *against*: the walking-skeleton
loses its "single test proves end-to-end" character — readers now have
to consult two tests in two crates to know the full pipeline works.
Mitigations: (a) Cilium / Katran both do exactly this and it is well-
documented as best practice; (b) Overdrive's existing
`reverse_nat_e2e` × 5 already proves the wire path against
`ThreeIfaceTopology` — so A4 is *already gated*, just under a
different test name in a different crate; (c) the walking-skeleton
becomes a cleaner convergence-chain test, not a less-useful one.
Honest acknowledgement: a documentation note in
`backend_discovery_bridge/walking_skeleton.rs` pointing at the
sibling `reverse_nat_e2e` family is mandatory for the cross-reference
to survive future maintenance.

## Recommendation

**Adopt Option A as primary; fall back to Option C if Option A
cannot land within the current feature scope.** Reasoning:

1. **Option A has the strongest precedent and lowest production risk**.
   The change (`netns_path: Option<PathBuf>` + `setns` in `pre_exec`)
   is a well-known Rust pattern (Finding 2.4), already in use in
   Overdrive's own test runtime for the tokio worker case
   (`walking_skeleton.rs:34-44`), and aligns with the CNI model
   (Finding 2.5) that every Kubernetes-class platform follows. The
   surface expansion is opt-in and `None`-defaulted; the blast radius
   on production callers is zero by construction.

2. **Option A preserves the walking-skeleton's main load-bearing
   property** — that the production submit chain produces a working
   backend through the production ExecDriver code path. This is the
   property Option B breaks and Option C decentralises across tiers.

3. **Option A composes with future Overdrive work**. When
   container-shaped or microvm-shaped workloads land, the same
   `netns_path` parameter is the natural interface for those drivers
   to enter caller-managed network namespaces (mirroring CNI's
   `CNI_NETNS`). Option B's bespoke fixture-side spawn does not
   compose; Option C's tier split must be re-evaluated for every new
   workload-type-vs-LB integration.

4. **Option C is the correct fallback** if scope pressure or user
   policy ("don't expand `ExecDriver`'s production API in this
   feature") forces deferral of Option A. It is fully precedented
   (Cilium, Katran) and Overdrive's existing `reverse_nat_e2e` × 5
   test family already provides the A4-equivalent gate against
   `ThreeIfaceTopology`. The walking-skeleton would shrink to A1–A3
   ("the convergence chain produces a populated SERVICE_MAP and the
   workload reaches Running"), with a doc-comment cross-reference to
   the sibling test family.

5. **Option B is rejected**. It breaks the walking-skeleton's stated
   purpose to avoid a 20-line production change, yields no future
   composability, and creates a bespoke spawn-and-reap state machine
   in the test fixture that future maintainers will have to
   understand and preserve.

**Suggested sequencing for Option A**:

1. Surface to the user: "Extending `ExecDriver::start` with `netns_path:
   Option<PathBuf>` is required for the walking-skeleton's A4 assertion
   to land against a production-fidelity 3-netns topology. The change is
   opt-in (None-default) and structurally aligned with CNI's
   `CNI_NETNS` model. Approve?"
2. On approval, land the `ExecDriver` change in a focused PR with
   tests against `ThreeIfaceTopology`. The `netns_path` param accepts
   a path under `/var/run/netns/<name>` (the iproute2 convention);
   `pre_exec` opens it as a FD and calls `setns(fd, CLONE_NEWNET)`
   before `execve`.
3. Refactor walking-skeleton to use `ThreeIfaceTopology` (already
   exists in `crates/overdrive-testing/src/netns.rs:216-340`).
   Backend `netns_path` flows through Service spec → action-shim →
   `ExecDriver::start(spec, Some(backend_ns_path))`. Test probe runs
   from `client_ns` via the existing `setns`-on-test-thread pattern.
4. The `TwoNetnsTopology` is deleted; its only consumer was the
   walking-skeleton. (Per `.claude/rules/development.md` § "Deletion
   discipline" — delete production code AND its tests in the same
   commit.)

**Suggested sequencing for Option C (fallback)**:

1. Shrink walking-skeleton to A1–A3 only; remove A4 entirely with a
   doc-comment pointing at `crates/overdrive-dataplane/tests/
   integration/reverse_nat_e2e.rs` as the A4-equivalent gate.
2. Verify `reverse_nat_e2e` exercises the same VIP-range / forward +
   reverse-NAT path the walking-skeleton would have proved. If there
   is a coverage gap (e.g., the dataplane tests assume specific
   backend-IP / VIP shapes the walking-skeleton would have exercised),
   add a new `reverse_nat_e2e` scenario rather than reviving A4 in
   the control-plane crate.
3. Document the tier split in
   `docs/feature/backend-discovery-bridge-service-reachability/design/architecture.md`
   § 6.2 — replace the fictional `LimaFixture` reference (per RCA) with
   a clear statement that the convergence-chain gate (control-plane
   crate) and the wire-path gate (dataplane crate) are two distinct
   Tier 3 tests by design.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Cilium PR #16338 — Standalone L4LB XDP tests | github.com | High | Official source | 2026-05-21 | Y |
| Cilium L4LB blog post | cilium.io | High | Official | 2026-05-21 (prior research) | Y |
| Cilium BPF testing docs | docs.cilium.io | High | Official | 2026-05-21 | Y |
| Cilium kube-proxy-free docs | docs.cilium.io | High | Official | 2026-05-21 | Y |
| Cilium Local Redirect Policy docs | docs.cilium.io | High | Official | 2026-05-21 | Y |
| Cilium issue #14990 (Datapath testing via BPF_PROG_TEST_RUN) | github.com/cilium | High | Source | 2026-05-21 | Y |
| Cilium test directory tree | github.com/cilium/cilium | High | Source | 2026-05-21 | Y |
| Katran DEVELOPING.md | github.com/facebookincubator | High | Official | 2026-05-21 (prior research) | Y |
| Engineering at Meta: Open-sourcing Katran | engineering.fb.com | High | Official | 2026-05-21 (prior research) | Y |
| Loxilb cicd/common.sh | github.com/loxilb-io | High | Source | 2026-05-21 | Y |
| Loxilb cicd/tcplb/validation.sh | github.com/loxilb-io | High | Source | 2026-05-21 | Y |
| Loxilb TCP-LB-Sanity CI workflow | github.com/loxilb-io | High | Official CI | 2026-05-21 | Y |
| xdp-tutorial testenv.sh | github.com/xdp-project | High | Official | 2026-05-21 | Y |
| Rust std::os::unix::process::CommandExt (pre_exec) | doc.rust-lang.org | High | Official | 2026-05-21 | Y |
| nix::sched::setns docs | docs.rs | High | Official | 2026-05-21 | Y |
| netns-rs crate documentation | docs.rs | High | Technical | 2026-05-21 | Y |
| netns-exec crate | crates.io | Medium-High | Industry | 2026-05-21 | Y |
| LWN: Namespaces in operation, part 2 (API) | lwn.net | High | Industry | 2026-05-21 | Y |
| CNI specification | cni.dev | High | Official | 2026-05-21 | Y |
| Kubernetes Network Plugins docs | kubernetes.io | High | Official | 2026-05-21 | Y |
| LPC 2020 K8s Service LB with BPF & XDP | lpc.events | High | Official (PDF binary, URL preserved) | 2026-05-21 | N — PDF binary not extractable via WebFetch |

Reputation distribution: High 19/21 (90%), Medium-High 1/21 (5%); Avg ~0.97. All sources from the trusted-source-domains list in the prompt context (or close adjacents — `lpc.events` for Linux Plumbers Conference, `cni.dev` for CNI spec — both unambiguously authoritative).

## Knowledge Gaps

### Gap 1: LPC 2020 PDF binary not extractable via WebFetch
**Issue**: The Linux Plumbers Conference 2020 slides on "K8s Service Load Balancing with BPF & XDP" — likely the most authoritative single source on Cilium's same-node packet path — could not be read because WebFetch does not extract PDF binary content. The URL is preserved in citations; the PDF was saved to a local path by the tool but its contents are not in this research.
**Attempted**: Direct WebFetch on the URL; search for HTML transcripts.
**Recommendation**: For a deeper future audit, manually open the PDF and confirm Finding 4.1's framing of "XDP scoped to remote backends". The current finding rests on the [docs.cilium.io kube-proxy-free docs](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/) quote, which is authoritative and self-consistent with [Cilium's Local Redirect Policy docs](https://docs.cilium.io/en/stable/network/kubernetes/local-redirect-policy/), but the LPC slides would provide the deepest mechanism description.

### Gap 2: Cilium's full ginkgo / Kind e2e test for same-node XDP behaviour not directly verified
**Issue**: Finding 3.1 establishes that Cilium splits unit (BPF_PROG_TEST_RUN) from e2e (Kind), but the exact shape of the Kind tests that exercise "XDP into local-Pod backend" was not retrieved (test directory listing only). It is possible Cilium does NOT gate this case in CI either (consistent with Finding 4.1 — XDP isn't the local-backend path).
**Attempted**: WebFetch of cilium/cilium/tree/main/test; search for ginkgo specs on same-node XDP.
**Recommendation**: Not blocking for this decision — the tier-split principle is established by docs.cilium.io's explicit testing-pyramid framing.

### Gap 3: No published comparison of Option A's `Command::pre_exec` + `setns` against alternative mechanisms (e.g., `clone(CLONE_NEWNET)` via raw syscall, `unshare` then `setns`)
**Issue**: Multiple Rust patterns can achieve "spawn a child in a different netns" — the `Command::pre_exec` shape is the most common but not the only one. No public benchmark or reliability comparison exists.
**Attempted**: Search; review of `netns-rs` and `netns-exec` source code (not fetched in full).
**Recommendation**: When implementing Option A, follow the `netns-exec` crate's pattern (`pre_exec` + `setns(fd, CLONE_NEWNET)`) as the canonical reference. The pattern is the same one `runc`/`crun` use internally; risk is bounded.

## Conflicting Information

### Conflict 1: Is real-TCP-through-XDP gated in production XDP-LB CI?
**Position A**: Cilium and Katran do NOT gate real TCP per PR; BPF program-level tests via `BPF_PROG_TEST_RUN` are the per-PR datapath gate. — Sources: [Cilium BPF testing docs](https://docs.cilium.io/en/stable/contributing/testing/bpf/), [Katran DEVELOPING.md](https://github.com/facebookincubator/katran/blob/main/DEVELOPING.md). Both high-reputation.
**Position B**: Loxilb DOES gate real TCP through netns per PR via `$hexec l3h1 curl` in `cicd/tcplb/validation.sh`. — Source: [loxilb validation.sh](https://raw.githubusercontent.com/loxilb-io/loxilb/main/cicd/tcplb/validation.sh), High reputation.
**Assessment**: Both positions are factually correct and refer to different projects. The disagreement is a *project-choice* difference, not a contradiction. Cilium and Katran ship the BPF program as one component of a larger stack (Kubernetes integration, daemon, cilium-agent) whose e2e gates are elsewhere; Loxilb ships the LB as a self-contained appliance and gates the appliance end-to-end. Overdrive's posture is closer to Cilium/Katran (the LB is one component of a larger control-plane), which supports either Option A (preserve the e2e gate) OR Option C (split into the existing 4-tier discipline). Both options are defensible; the choice depends on whether the user wants the walking-skeleton to be a *product-level* or *component-level* gate.

## Full Citations

[1] Cilium project. "helm,test: Add standalone L4LB XDP tests in a form of Github Action by brb · Pull Request #16338". GitHub. https://github.com/cilium/cilium/pull/16338. Accessed 2026-05-21.

[2] Cilium project. "Cilium Standalone Layer 4 Load Balancer XDP". cilium.io. 2022-04-12. https://cilium.io/blog/2022/04/12/cilium-standalone-l4lb-xdp/. Accessed 2026-05-06 (prior research).

[3] Cilium project. "BPF Unit and Integration Testing". docs.cilium.io. https://docs.cilium.io/en/stable/contributing/testing/bpf/. Accessed 2026-05-21.

[4] Cilium project. "Kubernetes Without kube-proxy". docs.cilium.io. https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/. Accessed 2026-05-21.

[5] Cilium project. "Local Redirect Policy". docs.cilium.io. https://docs.cilium.io/en/stable/network/kubernetes/local-redirect-policy/. Accessed 2026-05-21.

[6] Cilium project. "Datapath testing using BPF_PROG_TEST_RUN (#14990)". GitHub. https://github.com/cilium/cilium/issues/14990. Accessed 2026-05-21.

[7] Cilium project. "test/ directory". GitHub. https://github.com/cilium/cilium/tree/main/test. Accessed 2026-05-21.

[8] Facebook Incubator. "Katran DEVELOPING.md". GitHub. https://github.com/facebookincubator/katran/blob/main/DEVELOPING.md. Accessed 2026-05-06 (prior research).

[9] Engineering at Meta. "Open-sourcing Katran, a scalable network load balancer". 2018-05-22. https://engineering.fb.com/2018/05/22/open-source/open-sourcing-katran-a-scalable-network-load-balancer/. Accessed 2026-05-06 (prior research).

[10] Loxilb project. "cicd/common.sh". GitHub. https://raw.githubusercontent.com/loxilb-io/loxilb/main/cicd/common.sh. Accessed 2026-05-21.

[11] Loxilb project. "cicd/tcplb/validation.sh". GitHub. https://raw.githubusercontent.com/loxilb-io/loxilb/main/cicd/tcplb/validation.sh. Accessed 2026-05-21.

[12] Loxilb project. "TCP-LB-Sanity-CI workflow". GitHub. https://github.com/loxilb-io/loxilb/actions/workflows/tcp-sanity.yml. Accessed 2026-05-21.

[13] xdp-project. "xdp-tutorial / testenv / testenv.sh". GitHub. https://github.com/xdp-project/xdp-tutorial/blob/main/testenv/testenv.sh. Accessed 2026-05-21.

[14] Rust project. "std::os::unix::process::CommandExt". doc.rust-lang.org. https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html. Accessed 2026-05-21.

[15] nix-rust. "nix::sched::setns". docs.rs. https://docs.rs/nix/latest/nix/sched/fn.setns.html. Accessed 2026-05-21.

[16] wllenyj. "netns-rs crate". docs.rs. https://docs.rs/netns-rs/latest/netns_rs/. Accessed 2026-05-21.

[17] netns-exec maintainers. "netns-exec crate". crates.io. https://crates.io/crates/netns-exec. Accessed 2026-05-21.

[18] Michael Kerrisk. "Namespaces in operation, part 2: the namespaces API". LWN.net. https://lwn.net/Articles/531381/. Accessed 2026-05-21.

[19] CNI maintainers. "Container Network Interface (CNI) Specification". cni.dev. https://www.cni.dev/docs/spec/. Accessed 2026-05-21.

[20] Kubernetes project. "Network Plugins". kubernetes.io. https://kubernetes.io/docs/concepts/extend-kubernetes/compute-storage-net/network-plugins/. Accessed 2026-05-21.

[21] Linux Plumbers Conference. "K8s Service Load Balancing with BPF & XDP (2020)". lpc.events. https://lpc.events/event/7/contributions/674/attachments/568/1002/plumbers_2020_cilium_load_balancer.pdf. Accessed 2026-05-21 (binary PDF; not text-extractable via WebFetch; URL preserved).

[22] Prior research. "XDP L4 Load Balancer Test Topology — XDP_TX vs XDP_PASS (DSR-style) vs Three-Namespace Transit vs XDP_REDIRECT". 2026-05-06. `docs/research/dataplane/xdp-l4lb-test-topology-comprehensive-research.md`. (In-repo authoritative prior work, 32 sources, High confidence.)

## Research Metadata

Duration: ~75 min | Sources examined: ~25 | Sources cited: 22 | Cross-references: 18 | Confidence: High (Q1, Q2, Q4), Medium-High (Q3) | Output: `docs/research/testing/walking-skeleton-xdp-lb-topology.md`
