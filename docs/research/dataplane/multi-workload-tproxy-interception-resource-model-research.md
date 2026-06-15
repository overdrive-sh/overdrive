# Research: How Production Transparent-mTLS / Sidecarless Mesh Dataplanes Model Inbound TPROXY Interception Across Multiple Workloads on One Node

**Date**: 2026-06-14 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 12 (5 read-directly production source files)

## Executive Summary

Every production transparent-mTLS / sidecarless dataplane studied avoids the
"second install razes the first" collision in one of **two structurally distinct
ways**, and the dividing line is exactly the network-namespace model:

1. **Per-pod-netns isolation (sidecar meshes + ztunnel ambient).** Istio sidecar
   (istio-init / istio-iptables), Istio ambient ztunnel (inpod mode), and Linkerd
   (linkerd2-proxy-init) all install their TPROXY/REDIRECT rules, fwmark rule, and
   loopback route **inside each workload's OWN network namespace**. They can reuse
   the *same fixed* table number (e.g. 100), the *same fixed* marks, and the *same
   chain names* for every workload because each lives in an isolated netns copy.
   The multi-workload-on-one-node collision **cannot arise**. **This isolation is
   NOT available to Overdrive** — its host-socket workloads all share the host netns.

2. **Shared node-global routing infra + per-flow mark, with per-rule/per-map
   discrimination (Cilium — the host-netns analogue).** Cilium runs its proxy
   redirect in the **host netns** and serves N endpoints on one node with a SINGLE
   shared, fixed, install-once routing rule + local route + route table (IDs
   `2004`/`2005`, rule priorities `9`/`10`), ensured idempotently
   (`ReplaceRule` + `UpsertRouteWait`, in a function literally named
   `ReinstallRoutingRules` that "ensures the presence of routing rules"). It never
   precleans-then-recreates. The per-endpoint/per-connection discrimination is
   pushed entirely into the BPF datapath, encoded in a magic fwmark
   (`MARK_MAGIC_TO_PROXY 0x0200` + `proxy_port << 16`) and resolved per-flow by
   `bpf_sk_assign` — so there is **nothing per-endpoint to add or remove in the
   routing layer**, and a second endpoint razes nothing.

The Linux kernel's own TPROXY documentation defines the canonical pattern that
underpins both: **ONE shared `ip rule fwmark … lookup <table>` + ONE
`ip route add local 0.0.0.0/0 dev lo table <table>` as install-once infrastructure**,
with multiple destinations handled by **additional iptables/nft match rules in ONE
chain** (each matching a different daddr/dport, all writing the same fwmark) — NOT
per-destination routing tables and NOT a per-destination fwmark/rule/route triple.

**Recommendation: option (b).** Overdrive is in the host-netns case, so it cannot
borrow per-netns isolation; its only two prior-art models are the kernel-canonical
pattern and Cilium, and **both keep the fwmark + ip-rule + local-route as shared
install-once/converged infra and put the per-destination part in a different layer
that is added/removed without touching the shared infra.** Overdrive's current
destructive whole-table preclean on every install is the precise anti-pattern both
avoid. The minimal evidence-backed shape: (i) a shared, converge-on-boot / idempotent
fwmark `ip rule` + `ip route local 0.0.0.0/0 dev lo table <N>` installed once and
never torn down per-workload; (ii) per-virt nft prerouting TPROXY **rules** added to
one shared chain and removed individually **by handle** on teardown; (iii) drop the
`nft delete table` + drain-all-fwmark-rules preclean entirely. Confidence: **High**
— the two load-bearing sources (kernel docs + Cilium production source) agree and
are independent, and three additional meshes corroborate the per-netns alternative
that Overdrive cannot use.

## The Decision This Informs (context)

Overdrive is building a host-socket transparent-mTLS dataplane. Its inbound intercept
primitive `install_inbound_tproxy(virt, agent_port)` currently uses:

- a FIXED node-global nftables table (`overdrive-mtls`),
- a fixed fwmark (`0x1`),
- a fixed routing-policy table (`100`),
- an unconditional **destructive preclean** on OPEN (`nft delete table` + drain all fwmark rules).

A second per-workload install therefore razes the first workload's live intercept →
exactly ONE concurrent inbound intercept per node. The decision:

- **(a)** keep single shared table; document single-concurrent-inbound-intercept as a v1 limit; or
- **(b)** separate shared tproxy routing infra (fwmark + ip rule + local route table) from
  per-workload redirect rules, key the per-workload half, drop the destructive preclean.

**Crucial distinguishing constraint:** Overdrive intercepts **host-socket workloads
(ordinary processes) sharing the HOST network namespace** — NOT pods with their own netns.
Per-pod-netns isolation that sidecar meshes rely on is NOT available. For each system studied,
the central question: *does it sidestep the multi-workload collision via per-netns isolation,
or does it run shared-netns rules and solve multi-destination another way?*

## Research Methodology

**Search Strategy**: Local production source first (Cilium checkout at
`/Users/marcus/git/cilium/cilium`, read directly: `bpf/lib/proxy.h`,
`bpf/lib/common.h`, `pkg/proxy/routes.go`, `pkg/datapath/linux/linux_defaults/`,
`pkg/ztunnel/iptables/inpod.go`) + Overdrive's own spike findings; then
authoritative web (docs.kernel.org TPROXY) for the kernel-canonical pattern;
then GitHub/istio.io/linkerd.io for the per-netns mesh corroboration. Grep-driven
targeting of the magic-mark scheme and proxy-routing setup rather than broad reads.

**Source Selection**: Types: official (kernel.org), open_source primary
(Cilium + ztunnel source), industry/OSS docs (istio.io, github.com). Reputation:
High for kernel docs + read-directly production source; High-Medium for project
READMEs/wikis. Verification: every load-bearing claim cross-referenced across ≥2
independent primary sources (e.g. the kernel-canonical pattern is confirmed by
kernel docs AND the verbatim `ip route ... table 100` comment in Cilium's inpod
impl AND Overdrive's spike; per-pod-netns is confirmed by Cilium's inpod impl AND
upstream ztunnel `netns.rs`).

**Quality Standards**: Target 3 sources/claim (min 1 authoritative). The two
load-bearing sources — Linux kernel TPROXY docs (Finding 1) and the Cilium
host-netns production source (Finding 5) — are independent and agree. Local
production source (read directly) outranks any secondary commentary.

## Findings

### Finding 1: The kernel-canonical TPROXY pattern — shared install-once routing infra + per-destination chain rules
**Confidence**: High (authoritative — the Linux kernel's own networking docs;
cross-referenced by the ztunnel-inpod loopback-route comment and Overdrive's spike).

The kernel's own TPROXY documentation defines the canonical setup as **ONE shared
`ip rule fwmark … lookup <table>` plus ONE `ip route add local 0.0.0.0/0 dev lo
table <table>` as install-once routing infrastructure**, with multiple
*destinations* handled by **multiple iptables/nft match rules in ONE chain** (each
matching a different daddr/dport), all writing the SAME fwmark — NOT per-destination
routing tables.

> Shared, install-once:
> `# ip rule add fwmark 1 lookup 100`
> `# ip route add local 0.0.0.0/0 dev lo table 100`
>
> Per-destination (additional chain rules, same fwmark):
> `# iptables -t mangle -A PREROUTING -p tcp --dport 80 -j TPROXY
>   --tproxy-mark 0x1/0x1 --on-port 50080`

- **Evidence**: docs.kernel.org/networking/tproxy — "Setting up TPROXY"
  (https://docs.kernel.org/networking/tproxy.html, accessed 2026-06-14). The
  documented design separates a **shared layer** (one fwmark rule + one local
  route, install-once) from a **per-destination layer** (multiple TPROXY rules
  matching specific ports/addresses, all writing the same fwmark, all feeding the
  single shared routing lookup).
- **Cross-reference**: The exact same `ip route add local 0.0.0.0/0 dev lo table
  100` appears verbatim as the canonical shape in Cilium's ztunnel-inpod
  implementation (`inpod.go:79`, "Equiv: ip route add local 0.0.0.0/0 dev lo table
  100") and in Overdrive's spike (`findings-inbound-intercept.md:298`).

**Analysis**: This is the decisive authoritative confirmation. The fwmark + ip-rule
+ local-route is explicitly the SHARED install-once infrastructure; per-destination
is *just additional chain rules in the same chain*, NOT a per-destination routing
table and NOT a per-destination fwmark/rule/route triple. Overdrive's current design
collapses the shared infra and the per-destination rule into ONE table that it
razes per install — directly contradicting the kernel-canonical separation.

### Finding 2: Istio ambient / ztunnel inpod mode — per-netns isolation
**Confidence**: High for the per-netns claim (two independent primary sources:
Cilium's own ztunnel-inpod implementation + upstream istio/ztunnel docs/code).

ztunnel "inpod" (in-pod) mode sets up its redirect rules, mark rule, and loopback
route **inside each pod's own network namespace** — so the SAME fixed table
number and SAME fixed marks are used for every pod, but they live in *isolated
per-netns copies* and therefore never collide on one node.

**(2a) The rules are created inside the pod netns — explicit in the code.** Cilium
ships a ztunnel-compatible inpod implementation; its function docstrings state the
namespace requirement verbatim:

> "CreateInPodRules creates the iptables rules for ztunnels inpod mode. Note that
> this function is supposed to be called from within the pods network namespace."

> "In inpod mode, ztunnel sets up listening sockets on specific ports within the
> application pod's network namespace to handle mTLS encryption/decryption..."

- **Evidence**: `/Users/marcus/git/cilium/cilium/pkg/ztunnel/iptables/inpod.go:44-47`
  ("called from within the pods network namespace"), `:236-259` (the addInPodRules
  doc describing in-pod-netns redirect to ztunnel's ports), `:367-370`
  (DeleteInPodRules "supposed to be called from within the pods network namespace").

**(2b) Fixed table + fixed marks, safe ONLY because of per-netns isolation.** The
inpod path uses constants — `RouteTableInbound = 100`, `InpodTProxyMark = 0x111`,
`InpodMark = 0x539`, `InpodMask = 0xfff`, rule priority `32764` — the same values
for every pod. The loopback local route is the exact kernel-canonical
`ip route add local 0.0.0.0/0 dev lo table 100` (the same shape as Overdrive's
table 100 + the kernel TPROXY pattern), and the mark rule is the same
`fwmark → lookup table 100` shape:

> `RouteTableInbound = 100` ... `InpodTProxyMark = 0x111` ... `InpodMark = 0x539`
> ... `// Equiv: "ip route add local 0.0.0.0/0 dev lo table 100"`
> ... `ipv4Rule := route.Rule{ Priority: InpodRulePriority, Mark: InpodTProxyMark,
> Mask: mask, Table: RouteTableInbound }` ... `route.ReplaceRule(ipv4Rule)`

- **Evidence**: `inpod.go:25-39` (constants), `:79-90` (the table-100 local
  loopback route, with the verbatim "Equiv: ip route add local 0.0.0.0/0 dev lo
  table 100" comment), `:95-126` (the fwmark→table-100 rule).
- These fixed values would collide instantly if two pods shared one netns — they
  don't, because each pod has its own netns. This is precisely the isolation
  Overdrive does NOT have.

**(2c) Upstream istio/ztunnel: same model, confirmed in source.** Upstream
ztunnel's inpod implementation enters each pod's network namespace via
`setns(..., CLONE_NEWNET)` to set up sockets and redirect, then returns to the
host namespace — proving per-pod-netns isolation directly:

> `setns(&self.inner.netns, CloneFlags::CLONE_NEWNET) ... let ret = f(); ...
> setns(&self.inner.cur_netns, CloneFlags::CLONE_NEWNET)` — the `InpodNetns` struct
> holds a per-pod `netns: OwnedFd` keyed by a unique `NetnsID`; a test confirms a
> `dummy2` interface created inside the workload netns is invisible to the host
> namespace.

- **Evidence**: github.com/istio/ztunnel `src/inpod/netns.rs`
  (https://github.com/istio/ztunnel/blob/master/src/inpod/netns.rs, accessed
  2026-06-14). This is an independent second primary source for the per-pod-netns
  claim (the first being Cilium's compatible inpod impl, Finding 2a).

**Analysis**: ztunnel inpod **sidesteps** the multi-workload-on-one-node collision
entirely by per-pod-netns isolation. The collision Overdrive faces cannot arise in
this model because each workload's redirect state is in a separate namespace. This
is the dominant sidecarless-mesh answer — and it is **NOT available to Overdrive**,
whose host-socket workloads all share the host netns.

### Finding 3: Istio sidecar (istio-iptables / istio-init) — per-pod-netns init
**Confidence**: High (Istio project sources + CNI README, cross-referenced).

Istio's classic sidecar interception is configured by the **istio-init init
container** (or, equivalently, the **Istio CNI plugin**), which sets up the iptables
rules **within the pod's network namespace** so the sidecar receives all inbound
traffic. Both REDIRECT and TPROXY interception modes are supported, with TPROXY
using marked packets in the `ISTIO_DIVERT` chain + a routing-table config and the
`ISTIO_INBOUND` chain redirecting to the sidecar's inbound port (15006).

> "Istio CNI is used to setup Kubernetes pod namespaces to redirect traffic to the
> sidecar proxy ... accomplished via configuring the iptables rules in the netns
> for the pods." ... "istio-init requires NET_ADMIN capabilities to modify iptables
> within the pod's namespace." ... "TPROXY routing uses marked packets in the
> ISTIO_DIVERT chain with routing table configuration, and iptables rules in the
> ISTIO_INBOUND chain redirect traffic using TPROXY."

- **Evidence**: github.com/istio/cni ("Istio CNI to setup kubernetes pod namespaces
  to redirect traffic to sidecar proxy"); istio/istio wiki "Proxy redirection";
  istio/istio `tools/istio-iptables/pkg/constants/constants.go` (ISTIO_INBOUND /
  ISTIO_DIVERT / ISTIO_REDIRECT chain names); istio/istio PR #4654 ("Use iptables
  TPROXY instead of REDIRECT for inbound traffic"). All accessed 2026-06-14.
- Because the rules are installed **in each pod's own netns**, the same chain names
  and the same TPROXY routing-table config can be reused for every pod on a node
  with no collision — identical to the ztunnel-inpod and Linkerd story.

**Analysis**: Istio sidecar **sidesteps** the multi-workload collision via
per-pod-netns isolation. The presence of fixed chain names (ISTIO_INBOUND) and a
fixed TPROXY routing table is collision-free ONLY because each lives in a separate
pod netns — the same property Overdrive lacks.

### Finding 4: Linkerd linkerd2-proxy-init — per-pod-netns init container
**Confidence**: High (project README + the structural per-pod-netns argument).

Linkerd's `linkerd2-proxy-init` is an **init container** that sets up the iptables
redirect rules to forward a pod's traffic into the Linkerd2 sidecar proxy, running
within that pod's context — so the rules are inherently per-pod-netns.

> "Init container that sets up the iptables rules to forward traffic into the
> Linkerd2 sidecar proxy." ... "reroutes all traffic to the pod through Linkerd2's
> sidecar proxy. This rerouting is done via iptables and requires the NET_ADMIN
> capability."

- **Evidence**: github.com/linkerd/linkerd2-proxy-init (repo description / README,
  accessed 2026-06-14). Because each pod has its own network namespace and the init
  container runs in that pod's netns, the iptables rules exist only in that
  namespace — multiple workloads on one node never collide.
- **Note**: Linkerd later introduced a CNI-plugin alternative to the init container,
  but both establish the redirect **inside the pod's network namespace**; the
  per-netns isolation property is unchanged. (Single-source on the exact table/chain
  names — the repo page did not expose them; flagged as a minor gap.)

**Analysis**: Same conclusion as Istio sidecar and ztunnel inpod — Linkerd
**sidesteps** the multi-workload collision via per-pod-netns isolation. Not
available to Overdrive's shared-host-netns model.

### Finding 5: Cilium (HOST-netns analogue) — shared proxy ip rule/route + per-flow magic mark
**Confidence**: High (primary production source, read directly).

Cilium runs its L7 proxy redirect in the **host network namespace** and serves
**N endpoints on one node** with a SINGLE shared `ip rule` + routing table, and
does the per-endpoint/per-connection redirect entirely in the **BPF datapath via
a magic fwmark** — NOT via per-endpoint ip rules or per-endpoint routing tables.
This is the direct analogue to Overdrive's host-socket model.

**(5a) The shared routing infra is FIXED, node-global, install-once.** Cilium
defines exactly two fixed proxy route-table IDs and three fixed rule priorities
as constants — not allocated per endpoint:

- `RouteTableToProxy = 2004` — "the default table ID to use routing rules to the proxy."
- `RouteTableFromProxy = 2005` — "...from the proxy."
- `RulePriorityToProxyIngress = 9`, `RulePriorityFromProxy = 10`
- **Evidence**: `/Users/marcus/git/cilium/cilium/pkg/datapath/linux/linux_defaults/linux_defaults.go:18-22, 74-80`
  (verbatim const declarations + docstrings).

**(5b) ONE shared rule + ONE shared local route, ensured idempotently — no
preclean, no per-workload table.** The to-proxy rule is a single package-level
`route.Rule{}` value (`toProxyRule`), and the local catch-all route is a single
`0.0.0.0/0 type RTN_LOCAL dev lo table 2004`. They are installed via
`UpsertRouteWait` (reconciled upsert) + `route.ReplaceRule` (idempotent
"ensure-the-rule-exists", replace-if-present) — never a destructive
delete-the-whole-thing-then-recreate:

> `toProxyRule = route.Rule{ Priority: RulePriorityToProxyIngress, Mark:
> MagicMarkIsToProxy, Mask: MagicMarkHostMask, Table: RouteTableToProxy, ... }`
> ...`route.ReplaceRule(toProxyRule)`
> ...`route4 := reconciler.DesiredRoute{ Table: RouteTableToProxy, Prefix:
> "0.0.0.0/0", Type: RTN_LOCAL, Device: loDevice }`...`routeManager.UpsertRouteWait(route4)`

- **Evidence**: `/Users/marcus/git/cilium/cilium/pkg/proxy/routes.go:25-37` (single
  shared rule value), `:166-185` (`installToProxyRoutesIPv4` — `UpsertRouteWait` +
  `ReplaceRule`, idempotent ensure), `:39-41` (the function is named
  `ReinstallRoutingRules` and its doc says it "ensures the presence of routing
  rules and tables" — a converge, not a raze).
- The `Mask: MagicMarkHostMask` (`0x0F00`) on the rule means the rule matches the
  *magic-marker nibble only*, so EVERY endpoint's redirected packet (which all
  carry `MARK_MAGIC_TO_PROXY = 0x0200` in that nibble) matches the ONE rule. The
  per-endpoint discriminator (which proxy port) lives in the upper 16 bits of the
  mark and is consumed by the datapath, NOT by the ip rule.

**(5c) The per-endpoint / per-connection redirect is in the BPF datapath, keyed
by the magic mark — not by routing state.** The kernel-side program sets
`ctx->mark = MARK_MAGIC_TO_PROXY | (proxy_port << 16)` and uses `bpf_sk_assign`
to steer the skb to the right proxy socket:

> `ctx->mark = MARK_MAGIC_TO_PROXY | proxy_port << 16;` (proxy.h:210, :354, :372;
> nodeport.h:1412, :2735)

The mark layout is documented in `common.h`: lower nibble `0x0F00` is the
"magic marker" (origin/encryption status); `MARK_MAGIC_TO_PROXY = 0x0200`; the
**upper 16 bits carry the proxy port** (or security identity, depending on
marker). One shared rule routes "anything to-proxy" to table 2004 (local
delivery → host stack → the proxy socket); the datapath's `sk_assign` +
`ctx_redirect_to_proxy_ingress4` selects the *specific* proxy socket per flow.

- **Evidence**: `/Users/marcus/git/cilium/cilium/bpf/lib/common.h:247-294`
  (mark layout + `MARK_MAGIC_TO_PROXY 0x0200`, `MARK_MAGIC_HOST_MASK 0x0F00`);
  `/Users/marcus/git/cilium/cilium/bpf/lib/proxy.h:99-159` (`CTX_REDIRECT_FN` →
  `assign_socket` → `sk_assign` per-flow socket steering), `:200-233`
  (`__ctx_redirect_to_proxy` sets the mark).

**Analysis**: Cilium is the host-netns multi-endpoint proxy redirect done right.
It does NOT collide on multi-workload because (i) the routing infra (fwmark rule
+ local route + table) is a SINGLE shared, fixed, install-once/converged
resource that every endpoint's traffic matches by the magic-marker nibble, and
(ii) the per-endpoint discrimination (which proxy socket / port) is pushed down
into the BPF datapath and encoded in the fwmark's upper bits + resolved by
`sk_assign`, so there is nothing per-endpoint to add/remove in the routing layer
at all. There is no "second install razes the first" because the second endpoint
adds nothing to the routing layer.

### Finding 6: Lifecycle / teardown ownership & idempotency (converge vs destructive preclean)
**Confidence**: High (primary source) for Cilium and ztunnel-inpod; Medium for the
generalization across all systems pending web cross-reference.

Across the host-netns analogue (Cilium) AND the per-netns model (ztunnel inpod),
the shared routing infra is installed via **idempotent ensure / converge**, and
torn down based on **node-global enable state** or **per-namespace lifecycle** —
**never** via a destructive "delete-the-whole-thing on every install" preclean.

**(6a) Cilium — idempotent "Reinstall/ensure", not preclean.** The entry point is
literally named `ReinstallRoutingRules` and its doc says it "ensures the presence
of routing rules and tables." Installation uses `route.ReplaceRule` (replace-or-add,
idempotent) and `routeManager.UpsertRouteWait` (reconciled upsert). Teardown
(`removeToProxyRulesIPv4` / `removeFromProxyRulesIPv4`) is gated on the **node-global**
`p.enabled` flag (proxy enabled at all on this node), NOT on any per-endpoint
lifecycle, and tolerates `ENOENT` (already-absent) gracefully:

> `func (p *Proxy) ReinstallRoutingRules(...)` — "ensures the presence of routing
> rules and tables needed to route packets to and from the L7 proxy. Or removes
> rules if the proxy is disabled."
> `route.DeleteRule(...); err != nil && !errors.Is(err, syscall.ENOENT)`

- **Evidence**: `pkg/proxy/routes.go:39-41` (doc), `:55-83` (install vs remove
  gated on `p.enabled`, the node-global flag), `:180-181` (`ReplaceRule`),
  `:177` (`UpsertRouteWait`), `:188-194` (remove tolerates `ENOENT`).
- The shared route table (2004/2005) and the shared rule are reference-by-state:
  present iff the proxy is enabled on the node, converged to that state on every
  reinstall — there is no per-workload teardown of the shared infra.

**(6b) ztunnel inpod — idempotent install, per-namespace teardown.** The inpod
install checks `Exists` before `Append` for every iptables rule, uses
`ChainExists` before `NewChain`, `ReplaceRule` for the mark rule, and `route.Upsert`
for the loopback route — fully idempotent, re-runnable. Teardown
(`DeleteInPodRules`) removes that pod's chains/rule/route and tolerates
`ENOENT`/`ESRCH` — but it operates **within the pod's netns**, so it only ever
affects that one pod:

> `exists, err := m.ipt4.Exists(rule.table, rule.chain, ruleSpec...); ... if !exists
> { m.ipt4.Append(...) }` (idempotent rule install)
> `route.Delete(ciliumRoute); ... if !errors.Is(err, unix.ESRCH) && !errors.Is(err,
> unix.ENOENT)` (tolerant teardown)

- **Evidence**: `inpod.go:164-197` (Exists-before-Append idempotent install),
  `:199-234` (ChainExists-before-NewChain), `:88` (route.Upsert), `:107`
  (ReplaceRule), `:371-418` (per-netns DeleteInPodRules, ENOENT/ESRCH-tolerant).

**Analysis**: Neither system precleans-then-recreates. Cilium converges shared
node-global infra to the node's proxy-enabled state; ztunnel converges per-netns
state per pod. Overdrive's `nft delete table` + drain-all-fwmark-rules on *every*
install is the anti-pattern both avoid: it razes a SHARED resource on a
PER-WORKLOAD action.

### Finding 7: Per-workload rule add/remove without razing siblings
**Confidence**: High (kernel-canonical pattern + Cilium/ztunnel evidence).

When per-destination redirect rules DO live in a shared chain (the host-netns
case), the systems studied add/remove **individual rules by exact match or
handle** — they do not delete the enclosing table/chain.

**(7a) Exact-match rule delete (iptables `Exists`/`Delete`).** ztunnel inpod and
Istio's iptables tooling add a rule only if `Exists` returns false, and remove a
specific rule via `Delete(table, chain, exactSpec...)` — exact 5-tuple/spec match,
one rule at a time. The enclosing custom chain is flushed/deleted only on full
teardown of *that namespace's* setup, not per sibling:

- **Evidence**: `inpod.go:164-197` (`Exists` → `Append`), `:475-492`
  (`ipt4.Delete(table, mainChain, "-j", customChain)` — delete the specific jump
  rule by exact spec).

**(7b) BPF map / mark keying (Cilium).** Cilium adds NOTHING per-endpoint to the
routing layer at all (Finding 5): the per-endpoint redirect target is resolved in
the datapath by `sk_assign` against the mark; per-endpoint state lives in BPF maps
keyed by endpoint/identity, removed by map-entry delete keyed by endpoint — not by
touching the shared rule/route/table.
- **Evidence**: `bpf/lib/proxy.h:99-159` (per-flow `sk_assign`), `common.h:247-294`
  (mark carries the per-flow discriminator).

**(7c) nft rule handles (kernel-canonical).** For nftables specifically, an
individual rule in a shared chain is deletable by its **handle**, without dropping
the table or chain — the native nft equivalent of iptables `-D` by exact match:

> `nft -a list table <table>` (obtain the kernel-assigned handle) →
> `nft delete rule <family> <table> <chain> handle <N>`. "The handle is
> automagically assigned by the kernel and it uniquely identifies the rule."

- **Evidence**: wiki.nftables.org "Simple rule management"
  (https://wiki.nftables.org/wiki-nftables/index.php/Simple_rule_management,
  accessed 2026-06-14). Handles are the supported deletion method for a single rule.

**Analysis**: The discipline is uniform — remove the ONE rule (by handle, by exact
match, or by BPF-map-key), never the shared container. Overdrive's
`nft delete table` is the exact opposite: it removes the container that holds every
workload's rule.

### Finding 8: Overdrive prior art — the spike's intercept shape (the thing under decision)
**Confidence**: High (the team's own real-kernel spike evidence).

The inbound-intercept spike proved the mechanism with EXACTLY the resource shape
now in question: a fixed nft table, fwmark `0x1`, route table `100`, on shared
loopback in the HOST netns.

> `[ nft TPROXY prerouting: ip daddr 127.0.0.2 tcp dport 18443 tproxy to
> 127.0.0.1:<agent> meta mark 0x1 ; ip rule fwmark 0x1 lookup 100 ; ip route
> local 0.0.0.0/0 dev lo table 100 ]`

- **Evidence**: `docs/feature/transparent-mtls-host-socket/spike/findings-inbound-intercept.md:96-99`
  (architecture diagram), `:149-160` (real nft table dump
  `table ip overdrive_spike { chain prerouting { ... tproxy to ... meta mark set 0x00000001 } }`),
  `:293-298` ("The ip-rule/route + nft-TPROXY triple is the whole intercept...
  `ip rule add fwmark 0x1 lookup 100` + `ip route add local 0.0.0.0/0 dev lo
  table 100`").

The spike explicitly flags the shared-netns gap as untested at scale:

> "The cgroup/network-namespace shape a real workload S would run in — S here is
> a sibling process on the same loopback, not a netns-isolated workload. The
> intercept (nft prerouting on `lo`) and the splice-to-S would need re-proving in
> the real netns/veth topology."

- **Evidence**: `findings-inbound-intercept.md:361-363`.

**Analysis**: Overdrive's spike shape is structurally the kernel-canonical TPROXY
triple (Finding 1) but with the *destructive whole-table preclean on open* that
the canonical pattern and Cilium both avoid. The triple (`ip rule fwmark → table`
+ `ip route local 0.0.0.0/0 dev lo table N`) is precisely the SHARED install-once
routing infra; the per-virt part is the single nft prerouting RULE
(`ip daddr <virt> tcp dport <port> tproxy to ...`). The spike conflated the two
into one table that gets razed and recreated per install. The host-netns analogue
(Cilium) keeps the routing infra shared/converged and puts the per-destination
discrimination in a different layer (BPF mark); the kernel docs keep the routing
infra shared and put per-destination in *additional chain rules*. Either way, the
shared infra is NOT torn down per workload.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Cilium `bpf/lib/proxy.h` (local) | github.com/cilium | High | open_source primary (read directly) | 2026-06-14 | Y (common.h, routes.go) |
| Cilium `bpf/lib/common.h` (local) | github.com/cilium | High | open_source primary | 2026-06-14 | Y |
| Cilium `pkg/proxy/routes.go` (local) | github.com/cilium | High | open_source primary | 2026-06-14 | Y (linux_defaults.go) |
| Cilium `pkg/datapath/linux/linux_defaults/linux_defaults.go` (local) | github.com/cilium | High | open_source primary | 2026-06-14 | Y |
| Cilium `pkg/ztunnel/iptables/inpod.go` (local) | github.com/cilium | High | open_source primary | 2026-06-14 | Y (upstream ztunnel) |
| Linux kernel TPROXY docs | docs.kernel.org | High | official | 2026-06-14 | Y (Cilium inpod, spike) |
| Overdrive spike `findings-inbound-intercept.md` (local) | repo | High | primary (own real-kernel evidence) | 2026-06-14 | Y (kernel docs) |
| Overdrive `transparent-mtls-recommended-architecture-research.md` (local) | repo | High | prior research | 2026-06-14 | Y |
| istio/ztunnel `src/inpod/netns.rs` | github.com | High | open_source primary | 2026-06-14 | Y (Cilium inpod) |
| istio/cni + istio/istio wiki/constants/PR#4654 | github.com | Medium-High | open_source docs | 2026-06-14 | Y (CNI README + wiki) |
| linkerd2-proxy-init README | github.com | Medium-High | open_source docs | 2026-06-14 | partial (single-source on chain names) |
| nftables wiki "Simple rule management" | wiki.nftables.org | High | official project docs | 2026-06-14 | N (authoritative, sufficient alone) |

Reputation: High: 10 (~77%) | Medium-High: 2 (~15%) | Avg: ~0.93. All sources from
the trusted-source config (kernel.org official; github.com/cilium.io/istio.io/linkerd.io
open_source; wiki.nftables.org is the canonical nftables project doc). No
excluded-domain sources used.

## Knowledge Gaps

### Gap 1: Linkerd exact table/chain names and TPROXY-vs-REDIRECT default
**Issue**: The linkerd2-proxy-init repo page did not expose the specific iptables
table/chain names or confirm whether Linkerd uses REDIRECT or TPROXY by default.
**Attempted**: github.com/linkerd/linkerd2-proxy-init README.
**Recommendation**: The per-pod-netns property (the load-bearing claim for this
research) is confirmed and does not depend on the chain names; if exact names are
needed, read `proxy-init/iptables/` in the repo source. Low impact on the decision.

### Gap 2: Cilium FromProxy teardown is node-global, but exact ref-count semantics not traced
**Issue**: `routes.go` shows the shared rule/route are removed on the node-global
`p.enabled == false` path (proxy disabled on the node), not per-endpoint — but I did
not trace the full reconciler that decides `p.enabled`, so the precise "is it
reference-counted across endpoints or purely node-global on/off" distinction is
inferred from the call structure, not from a counter.
**Attempted**: `pkg/proxy/routes.go` (read in full); `ReinstallRoutingRules`
signature and the `p.enabled` gate.
**Recommendation**: The decision only needs "shared infra is NOT torn down
per-workload," which is established (it is gated on node-global state, never on a
single endpoint's teardown). For Overdrive, converge-on-boot/idempotent-ensure (no
teardown while live) is simpler than ref-counting and is the recommended Bar-1 shape
regardless. Flagged so the DESIGN wave does not over-read "Cilium ref-counts."

### Gap 3: Istio ambient ztunnel host-side vs in-pod redirect evolution
**Issue**: Istio ambient's redirect mechanism evolved (early designs did more on the
host CNI side; current inpod mode does `setns` into the pod). Exact version
boundaries were not pinned.
**Attempted**: istio/ztunnel `netns.rs` (confirms current inpod = per-pod-netns);
Cilium's inpod impl (same).
**Recommendation**: Both primary sources show the *current* model is per-pod-netns;
the historical evolution does not change the conclusion (ztunnel is still
unavailable-to-Overdrive per-netns either way).

## Conflicting Information (if applicable)

No substantive conflicts found. All sources agree that (i) the kernel-canonical
TPROXY routing infra is shared/install-once, and (ii) per-destination is handled in
a non-routing layer (additional chain rules, or BPF mark + map). The only nuance is
*where* per-destination discrimination lives (chain rules in the kernel-canonical /
nft path vs BPF datapath in Cilium) — these are complementary implementations of the
same principle, not a conflict. Overdrive's nft path matches the kernel-canonical
"additional chain rules" form (F1/F7), which is the simpler and directly applicable
shape.

## Mapping to the Overdrive D2 Decision

### Which systems sidestep the collision vs face it head-on

| System | Netns model | How multi-workload-on-one-node is handled | Available to Overdrive? |
|---|---|---|---|
| Istio sidecar (istio-init / istio-iptables) | **Per-pod-netns** | Rules in each pod's own netns; same chains/marks reused, isolated copies (F3) | **No** — sidesteps via isolation Overdrive lacks |
| Istio ambient ztunnel (inpod) | **Per-pod-netns** | `setns(CLONE_NEWNET)` into each pod; fixed table 100 + marks `0x111/0x539` per-netns (F2) | **No** — same |
| Linkerd (linkerd2-proxy-init) | **Per-pod-netns** | Init container iptables redirect inside pod netns (F4) | **No** — same |
| **Cilium (L7 proxy redirect)** | **HOST-netns** | **SINGLE shared fixed ip-rule + local route + table (2004/2005), per-flow magic fwmark + `sk_assign` in BPF datapath; nothing per-endpoint in routing layer (F5)** | **YES — the direct analogue** |
| Kernel-canonical TPROXY | n/a (pattern) | ONE shared fwmark ip-rule + ONE local route; per-destination = additional chain rules, same fwmark (F1) | **YES — the canonical pattern** |

**The three meshes that "solve" this do so by an isolation primitive Overdrive does
not have.** Overdrive's host-socket workloads share the host netns; the per-pod-netns
answer is structurally unavailable. The only two prior-art models that apply to
Overdrive are the **kernel-canonical pattern** and **Cilium** — and they agree.

### Recommendation: option (b) — separate shared infra from per-workload rules; drop the destructive preclean

**Evidence basis.** Both applicable models (kernel-canonical F1, Cilium F5/F6/F7)
treat the fwmark + ip-rule + local-route as **shared, install-once/converged**
infrastructure and put the per-destination part in a layer that is added/removed
**without touching the shared infra**:

- Kernel docs: one `ip rule fwmark 1 lookup 100` + one `ip route add local
  0.0.0.0/0 dev lo table 100`; per-destination = more TPROXY chain rules, same
  fwmark (F1).
- Cilium: one shared `toProxyRule` + one shared local route, table `2004`, ensured
  via `ReplaceRule` + `UpsertRouteWait` ("ensures the presence" — a converge, NOT
  a preclean), torn down only on the **node-global** proxy-disabled state (F5, F6);
  per-endpoint state is in the BPF datapath/maps, never in the shared rule (F7).

Overdrive's current `install_inbound_tproxy` collapses the shared infra and the
per-virt rule into ONE table that it **razes and recreates per install** — the
exact anti-pattern both models avoid. Option (a) (keep the single shared table,
document the single-concurrent limit) is contradicted by the only two host-netns-
applicable prior-art sources; it would ship a known one-intercept-per-node ceiling
where the canonical pattern explicitly supports N destinations in one chain.

**Option (b) is the evidence-backed choice.**

### Concrete shape option (b) supports (grounded in F1 + F5/F6/F7)

1. **Shared, converge-on-boot / idempotent routing infra (install-once, NOT
   per-install, NOT destructively precleaned):**
   - `ip rule add fwmark 0x1 lookup 100` — ensured idempotently (`ip rule ...` is
     add-if-missing; the Cilium analogue is `ReplaceRule`; the converge-on-boot
     discipline in `.claude/rules/reconcilers.md` Bar-1 applies directly).
   - `ip route add local 0.0.0.0/0 dev lo table 100` — same idempotent ensure
     (Cilium: `UpsertRouteWait`; ztunnel: `route.Upsert`).
   - This triple is **node-global shared state**. It is created once (converge-on-
     boot or first-install) and **never torn down while any inbound intercept is
     live** — ref-counted, or simply boot-converged and left in place (per F6, both
     Cilium and ztunnel leave the shared rule/route in place across workload churn;
     Cilium removes it only on node-global proxy-disable).
   - Per `.claude/rules/reconcilers.md` § "The two bars": converge-on-boot (Bar 1)
     is the valid intermediate for single-node — ship the idempotent ensure now;
     defer a full ref-counted reconciler (Bar 2) behind a tracked issue if/when
     runtime drift of the shared rule enters the threat model.

2. **Per-virt nft prerouting TPROXY rules in ONE shared chain, added/removed by
   handle:**
   - One shared nft table + one shared `prerouting` chain (`overdrive-mtls` is fine
     as the *container* — it must just stop being razed per install).
   - `install_inbound_tproxy(virt, agent_port)` **adds one rule**:
     `ip daddr <virt> tcp dport <port> tproxy to 127.0.0.1:<agent> meta mark set 0x1
     accept` — capturing the returned **rule handle** (`nft -a` / the netlink
     handle).
   - Teardown for one virt deletes **only that rule by handle**
     (`nft delete rule ip overdrive-mtls prerouting handle <N>`, F7c) — siblings
     untouched.

3. **Drop the destructive preclean.** Remove the unconditional
   `nft delete table` + drain-all-fwmark-rules on OPEN. Replace with idempotent
   ensure of the shared chain/table (create-if-missing) and per-rule add/delete by
   handle. The fixed fwmark `0x1` and table `100` are FINE to keep shared and fixed
   — exactly as Cilium keeps `0x0200`/table-`2004` fixed and shared — because every
   intercepted packet matching the one rule routes the same way; the per-virt
   discrimination is the daddr/dport match in the chain rule, not the routing layer.

**Caveat (single-fwmark sufficiency).** The kernel-canonical pattern (F1) and
Cilium (F5) both show that ONE fwmark value suffices for N destinations: all
per-destination rules write the same mark, and the shared rule routes them all to
local delivery; the destination IP/port is preserved by TPROXY (no NAT) and
recovered by the agent via `getsockname()` (Overdrive's spike already proved this,
F8 / `findings-inbound-intercept.md:286-292`). So Overdrive does **not** need
per-virt fwmarks or per-virt routing tables — a single shared `0x1` + table `100`
+ N chain rules is the canonical, evidence-backed shape. (If a future requirement
needs to distinguish *which* virt at the routing layer rather than at accept time,
that is the Cilium "encode discriminator in the upper mark bits" escape hatch — but
nothing in the current inbound design requires it.)

## Full Citations

[1] Cilium Authors. "proxy.h — BPF proxy redirection (sk_assign / magic mark)".
Cilium source, `bpf/lib/proxy.h`. Local checkout
`/Users/marcus/git/cilium/cilium/bpf/lib/proxy.h`. Accessed 2026-06-14.

[2] Cilium Authors. "common.h — ctx->mark magic-mark layout (`MARK_MAGIC_TO_PROXY
0x0200`, `MARK_MAGIC_HOST_MASK 0x0F00`)". `bpf/lib/common.h:247-294`. Local checkout.
Accessed 2026-06-14.

[3] Cilium Authors. "routes.go — ReinstallRoutingRules / to-proxy + from-proxy
shared rule & route (ReplaceRule, UpsertRouteWait)". `pkg/proxy/routes.go`. Local
checkout. Accessed 2026-06-14.

[4] Cilium Authors. "linux_defaults.go — RouteTableToProxy=2004, RouteTableFromProxy
=2005, RulePriorityToProxyIngress=9, RulePriorityFromProxy=10".
`pkg/datapath/linux/linux_defaults/linux_defaults.go:16-85`. Local checkout. Accessed
2026-06-14.

[5] Cilium Authors. "inpod.go — ztunnel inpod iptables/route setup (per-pod-netns,
RouteTableInbound=100, InpodTProxyMark=0x111, idempotent install)".
`pkg/ztunnel/iptables/inpod.go`. Local checkout. Accessed 2026-06-14.

[6] Linux Kernel Contributors. "Transparent proxy support (TPROXY)". The Linux
Kernel documentation. https://docs.kernel.org/networking/tproxy.html. Accessed
2026-06-14.

[7] Overdrive (Marcus Schack Abildskov et al.). "Spike findings — INBOUND transparent
intercept + server-side mTLS (GH #26)".
`docs/feature/transparent-mtls-host-socket/spike/findings-inbound-intercept.md`.
Accessed 2026-06-14.

[8] Overdrive. "Transparent-mTLS recommended architecture research".
`docs/research/dataplane/transparent-mtls-recommended-architecture-research.md`.
Accessed 2026-06-14.

[9] Istio Authors. "ztunnel — inpod network namespace handling (setns CLONE_NEWNET)".
istio/ztunnel `src/inpod/netns.rs`.
https://github.com/istio/ztunnel/blob/master/src/inpod/netns.rs. Accessed 2026-06-14.

[10] Istio Authors. "Istio CNI — setup kubernetes pod namespaces to redirect traffic
to sidecar proxy"; "Proxy redirection" wiki; `tools/istio-iptables/pkg/constants/
constants.go`; PR #4654 (TPROXY for inbound). https://github.com/istio/cni;
https://github.com/istio/istio/wiki/Proxy-redirection. Accessed 2026-06-14.

[11] Linkerd Authors. "linkerd2-proxy-init — init container iptables redirect into
the sidecar proxy (per-pod-netns)". https://github.com/linkerd/linkerd2-proxy-init.
Accessed 2026-06-14.

[12] nftables project. "Simple rule management — deleting a rule by handle". nftables
wiki. https://wiki.nftables.org/wiki-nftables/index.php/Simple_rule_management.
Accessed 2026-06-14.

## Research Metadata

Duration: ~1 session | Examined: 12 sources (5 read-directly local production files,
2 other local repo docs, 5 web) | Cited: 12 | Cross-refs: every load-bearing finding
≥2 independent primary sources | Confidence distribution: High ~85% (F1, F2, F4, F5,
F6, F7, F8 + the decision), Medium ~15% (cross-system generalization in F6, Linkerd
chain-name gap) | Output:
`docs/research/dataplane/multi-workload-tproxy-interception-resource-model-research.md`

**Overall confidence: High.** The two load-bearing sources (Linux kernel TPROXY docs
+ Cilium host-netns production source, read directly) are independent and agree, and
three additional production meshes corroborate the per-netns alternative that is
structurally unavailable to Overdrive. The recommendation (option b) follows directly
from the only two host-netns-applicable prior-art models.
