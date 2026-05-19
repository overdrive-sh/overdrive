# Research: Service-VIP / Cluster-IP Range Configuration Patterns Across Orchestrators

**Date**: 2026-05-15 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 12

## Executive Summary

Every surveyed Kubernetes-family platform — kubeadm (`10.96.0.0/12`), k3s (`10.43.0.0/16`), k0s (`10.96.0.0/12`), MicroK8s (`10.152.183.0/24`), and Talos (`10.96.0.0/12`) — ships a hard-coded default service CIDR and starts without operator config. No platform refuses-to-start when the service CIDR is unspecified; no platform performs automated host-network conflict detection at init time. The default vs HA distinction is absent from the prior art: kubeadm, k0s, and Talos use the same posture in single-node and multi-node configurations. Nomad sits outside this lineage entirely — it has no VIP / cluster-IP concept; allocations use the host's IP and a random port, and service discovery is delegated to Consul or to Nomad's native service-discovery (registry only, no IP allocation).

For Overdrive, the evidence supports **default-with-override in both Phase 1 (single-node) and Phase 2 (HA / multi-node)**, with a structured startup warning in HA mode when the default is in use. The user's intuition that "operator-supplied should ONLY be required in HA mode" is partially correct — Phase 1 should default — but the stronger conclusion the evidence supports is that *no surveyed prior art refuses-to-start over the service CIDR alone*, including in production HA configurations. The service CIDR is a data-path concern (interpreted by the dataplane; not routed across nodes as a real network), and the failure modes that do matter (host LAN overlap, NetworkManager interference) are not mechanically detectable at init time in any surveyed platform and remain operator-pre-flight responsibilities.

A `/24` default (~254 VIPs) matches MicroK8s and is adequate for an XDP-internal-allocation-ID use case at modest scale; if Overdrive expects hundreds of services on a single Phase 1 node, prefer `/20` or `/16` (matching k3s). The default's job is "low-friction onboarding, not collision-proof correctness" — that posture is uniform across the surveyed prior art.

## Research Methodology
**Search Strategy**: Targeted fetches of canonical platform documentation (kubernetes.io, docs.k3s.io, talos.dev, hashicorp.com/Nomad GitHub) plus cross-reference against CNCF / open-source repositories.
**Source Selection**: Official platform documentation (high reputation, authoritative-single permitted) + open-source GitHub repositories for source-of-truth confirmation of defaults.
**Quality Standards**: For platform-specific defaults, official docs count as authoritative-single. For pattern claims that span platforms, target 2+ sources.

## Findings

### Q1: Default vs Operator-Supplied — by Platform

**Summary table of out-of-the-box defaults (IPv4):**

| Platform | Default service CIDR | Default pod/cluster CIDR | Source |
|---|---|---|---|
| Kubernetes (kubeadm) | `10.96.0.0/12` | none (must be set if using a CNI that requires it) | kubernetes.io/docs/reference/setup-tools/kubeadm/kubeadm-init |
| k3s | `10.43.0.0/16` | `10.42.0.0/16` | docs.k3s.io |
| k0s | `10.96.0.0/12` | `10.244.0.0/16` | docs.k0sproject.io |
| MicroK8s | `10.152.183.0/24` | (CNI-managed) | microk8s.io / canonical/microk8s issues |
| Talos Linux | `10.96.0.0/12` | `10.244.0.0/16` | docs.siderolabs.com/talos |
| Nomad | N/A — no VIP/cluster-IP concept | N/A | developer.hashicorp.com/nomad/docs/networking |

**Finding 1a — kubeadm defaults `--service-cidr` to `10.96.0.0/12`.**
- Evidence (kubeadm-init docs): `--service-cidr string     Default: "10.96.0.0/12"`. The flag is optional; kubeadm starts without an explicit value.
- Source: [kubeadm init reference](https://kubernetes.io/docs/reference/setup-tools/kubeadm/kubeadm-init/), accessed 2026-05-15. Reputation: High (official).
- Verification: [kubernetes/kubernetes PR #35290](https://github.com/kubernetes/kubernetes/pull/35290) which changed the default from `10.0.0.0/12` to `10.96.0.0/12`, with explicit rationale: "10.x/12 for x > 32 are not used by public cloud providers. Weave Net use 10.32/12, and thereby service IPs should be a range higher then this."
- Confidence: High.

**Finding 1b — kube-apiserver itself defaults the flag too** (kubeadm just forwards it). PR #35290 above changes the apiserver default. The flag `--service-cluster-ip-range` is therefore defaulted at the apiserver layer, not required.

**Finding 1c — k3s defaults `--service-cidr` to `10.43.0.0/16` and `--cluster-cidr` to `10.42.0.0/16`.**
- Evidence: docs.k3s.io/cli/server states `Default value: "10.43.0.0/16"` for `--service-cidr` and `"10.42.0.0/16"` for `--cluster-cidr`. Both are defaulted, not required.
- Source: [docs.k3s.io/cli/server](https://docs.k3s.io/cli/server), accessed 2026-05-15. Reputation: High (official).
- Confidence: High (authoritative-single).

**Finding 1d — k0s defaults `serviceCIDR` to `10.96.0.0/12` and `podCIDR` to `10.244.0.0/16`.**
- Evidence: "serviceCIDR: Network CIDR to use for cluster VIP services. Defaults to `10.96.0.0/12`." "podCIDR: Pod network CIDR to use in the cluster. Defaults to `10.244.0.0/16`."
- Source: [docs.k0sproject.io/stable/configuration](https://docs.k0sproject.io/stable/configuration/), accessed 2026-05-15. Reputation: High (official).
- Confidence: High.

**Finding 1e — MicroK8s defaults service CIDR to `10.152.183.0/24` (note: distinct from upstream — Canonical chose a different default).**
- Evidence: MicroK8s CNI configuration docs and issue tracker confirm `10.152.183.0/24` as the cluster-IP range; `10.152.183.1` is the apiserver, `10.152.183.10` is CoreDNS. Only 252 services fit in a /24 — operators must change the range for larger clusters.
- Source: [microk8s.io/docs/change-cidr](https://microk8s.io/docs/change-cidr), [issue canonical/microk8s#1917](https://github.com/canonical/microk8s/issues/1917). Reputation: High (official) + Medium-High (project tracker).
- Confidence: High (default value); Medium (rationale — Canonical hasn't published a written rationale akin to PR #35290).

**Finding 1f — Talos defaults `serviceSubnets` to `10.96.0.0/12` and `podSubnets` to `10.244.0.0/16`.**
- Evidence: ClusterNetworkConfig example YAML shows `serviceSubnets: [10.96.0.0/12]` and `podSubnets: [10.244.0.0/16]`. The fields are configurable but the example values are what the bundled `talosctl gen config` produces.
- Source: [docs.siderolabs.com/talos/v1.10/reference/configuration/v1alpha1/config](https://docs.siderolabs.com/talos/v1.10/reference/configuration/v1alpha1/config/), accessed 2026-05-15. Reputation: High (official).
- Verification: Multiple secondary sources confirm these as the `talosctl` generation defaults; siderolabs/talos issue #8298 is a feature request to change them, which itself implies they are the current defaults.
- Confidence: High.

**Finding 1g — Nomad has no VIP / cluster-IP concept.**
- Evidence: "In Nomad, allocations use the IP address of the client in which they are running and are assigned random port numbers" — developer.hashicorp.com/nomad/docs/networking. The Nomad `service` block's `address_mode` (`host` | `alloc` | `alloc_ipv6` | `driver` | `auto`) selects *which existing address to advertise*, not a VIP allocator. Service discovery is delegated to Consul or to Nomad's native service-discovery (registry only — no IP allocation).
- Source: [developer.hashicorp.com/nomad/docs/networking](https://developer.hashicorp.com/nomad/docs/networking), [developer.hashicorp.com/nomad/docs/job-specification/service](https://developer.hashicorp.com/nomad/docs/job-specification/service), accessed 2026-05-15. Reputation: official-vendor-docs (not in trusted-domain list — cross-referenced below). 
- Verification: Implicit consensus across Kubernetes vs Nomad comparisons; the docs themselves contrast against K8s.
- Confidence: High for the no-VIP claim (it is a fundamental architectural fact, not a versioned detail); Medium-High on source trust (vendor docs cross-referenced against direct quotes from the same vendor's networking page).

### Q2: Single-Node vs Multi-Node / HA Differences

**Finding 2a — None of the surveyed platforms differentiate the service CIDR default between single-node and multi-node.**
- The CIDR is a cluster-wide configuration item at the control-plane layer. Whether the cluster is one node or 1000, the apiserver / equivalent control-plane process holds the same `--service-cluster-ip-range` value. kubeadm, k3s, k0s, Talos, and MicroK8s all apply the same default regardless of cluster size.
- Source: Each platform's flag/field reference (above). Confidence: High by absence — no platform documentation describes a "single-node default" distinct from a "multi-node default."

**Finding 2b — k3s and MicroK8s ship a smaller default than upstream (k3s: /16, MicroK8s: /24) because they target single-node / small-cluster scenarios.**
- Interpretation (flagged): k3s and MicroK8s position themselves as edge / dev / single-node distributions. Their narrower defaults (`10.43.0.0/16` = 65k services; `10.152.183.0/24` = 254 services) reflect that they don't expect production-scale service counts. k0s and upstream kubeadm (both targeting multi-node prod) use the wider `10.96.0.0/12` (~1M services).
- Source: comparison of the four default values above. Confidence: Medium — this is interpretation, not stated rationale. No platform documents a "we chose a smaller default for single-node use cases" justification.

**Finding 2c — Cross-node routing concerns are NOT what drives the default choice.**
- The relevant historical rationale (PR #35290) cited *avoiding overlap with public-cloud RFC-1918 conventions and Weave Net's `10.32/12`* — host-network collision concerns, not multi-node-coordination concerns. The service CIDR is virtual / data-path-only in K8s (handled by kube-proxy iptables / IPVS / Cilium BPF); it is not routed across nodes as a real network. Multi-node clusters don't impose stricter requirements than single-node on the CIDR itself.
- Source: [PR #35290](https://github.com/kubernetes/kubernetes/pull/35290) commit description. Confidence: High.

### Q3: Missing-Config Failure Mode (Refuse-to-Start / Default / Warn)

**Finding 3a — Every K8s-family platform DEFAULTS rather than refuses to start.**
- kubeadm: `--service-cidr` has a hard-coded default `10.96.0.0/12`; init proceeds without it. ([kubeadm-init reference](https://kubernetes.io/docs/reference/setup-tools/kubeadm/kubeadm-init/), accessed 2026-05-15.)
- k3s: `--service-cidr` defaults to `10.43.0.0/16`. ([docs.k3s.io/cli/server](https://docs.k3s.io/cli/server), accessed 2026-05-15.)
- k0s: `serviceCIDR` defaults to `10.96.0.0/12`. ([docs.k0sproject.io/stable/configuration](https://docs.k0sproject.io/stable/configuration/), accessed 2026-05-15.)
- Talos: `talosctl gen config` produces `serviceSubnets: [10.96.0.0/12]` by default; the field can be edited or omitted-to-default (omission triggers the bundled default during config validation). ([docs.siderolabs.com/talos/v1.10/reference/configuration/v1alpha1/config](https://docs.siderolabs.com/talos/v1.10/reference/configuration/v1alpha1/config/), accessed 2026-05-15.)
- MicroK8s: defaults `10.152.183.0/24` baked into snap; no operator decision required at install time.
- Confidence: High across all five.

**Finding 3b — No platform refuses to start over the service CIDR.**
- Search across kubernetes/kubernetes issues, kubeadm preflight checks, and the four sibling distros surfaces no "service-CIDR-missing" preflight failure path. The closest hit ([issue #54252](https://github.com/kubernetes/kubernetes/issues/54252)) documents that kubeadm does NOT cross-check service-CIDR against kubelet's `cluster-dns` — which is an *under-validation* complaint, the inverse of "refuse-to-start".
- Confidence: High (failure mode is absence, confirmed via issue tracker search).

**Finding 3c — Post-init service-CIDR changes are documented as disruptive across all platforms.**
- kubernetes/kubernetes [issue #86497](https://github.com/kubernetes/kubernetes/issues/86497): after a service-CIDR change, existing Services retain old IPs and produce errors `the cluster IP X for service Y is not within the service CIDR Z; please recreate` — manual recreation required.
- Talos and k0s docs both flag that changing CIDRs on a running cluster is "disruptive and not recommended" (recreate cluster).
- Confidence: High.

### Q4: Host-Network Conflict Detection

**Finding 4a — No surveyed platform performs automated detection of conflict between the configured service CIDR and the host's actual network at init time.**
- kubeadm preflight checks do not enumerate host route tables or interfaces to compare against `--service-cidr`. The default `10.96.0.0/12` is *chosen* to be unlikely to collide with common cloud / on-prem RFC-1918 conventions (PR #35290 rationale), but no runtime probe is performed.
- Operator guidance is consistently "verify ranges manually before init": "You should verify that the custom CIDR doesn't conflict with existing routes before cluster creation."
- Source: PR #35290 rationale + community guidance (oneuptime.com blog cited in search results, treated as Medium trust — used only as confirmation of an absence claim already supported by official-docs silence). Confidence: High on the absence; Medium on the workaround being "manual operator responsibility".

**Finding 4b — The only documented automated check is a post-hoc service-IP-not-in-range error** ([issue #86497](https://github.com/kubernetes/kubernetes/issues/86497)). This catches *after* a Service is created with a stale ClusterIP, not at init time, and not against host network.

**Finding 4c — kube-apiserver issue [#54252](https://github.com/kubernetes/kubernetes/issues/54252) is an open ask for kubeadm to check that kubelet's `cluster-dns` is inside the service-CIDR.** The issue's existence confirms that even this *intra-cluster* consistency check is not performed; host-network conflict detection is several rungs further away from current behavior.
- Confidence: High.

### Q5: Single-Node Distributions (k3s, k0s, microk8s, Talos single-node)

**Finding 5a — All single-node-friendly distros default the service CIDR; none require operator-supplied config.**
- This is the explicit posture of every distro that targets low-friction single-node setup:
  - k3s: `curl ... | sh` installs and runs with `10.43.0.0/16` out of the box.
  - MicroK8s: `snap install microk8s --classic` installs with `10.152.183.0/24`.
  - k0s: `k0s install` defaults to `10.96.0.0/12`.
  - Talos single-node: `talosctl gen config` produces `10.96.0.0/12`; this is the default emitted by the config generator regardless of cluster size.
- Confidence: High.

**Finding 5b — Defaults differ in size more than substance.** k3s and MicroK8s pick narrower (`/16` and `/24` respectively) than upstream's `/12`, but the *posture* (default-with-override) is identical to upstream. No surveyed distro inverts the posture to "refuse-to-start-without-config" for single-node.

**Finding 5c — Talos's posture is identical between single-node and multi-node.** Talos generates the same `cluster.network.serviceSubnets: [10.96.0.0/12]` in `talosctl gen config` whether the cluster is one control-plane node or three. The single-node-vs-multi-node distinction does not surface in the network defaults.
- Source: [docs.siderolabs.com/talos/v1.10](https://docs.siderolabs.com/talos/v1.10/reference/configuration/v1alpha1/config/). Confidence: High.

## Recommendations for Overdrive

The user's intuition is well-supported by prior art, but the prior-art evidence pushes harder than the intuition. Prior art uniformly **defaults rather than refuses**, even in production-grade multi-node distros (kubeadm, k0s, Talos HA). No surveyed platform — single-node or HA — refuses to start without an operator-supplied service CIDR. The "refuse-to-start in HA" posture would make Overdrive stricter than upstream Kubernetes in HA mode, which is unusual and warrants explicit justification.

### Phase 1 (single-node) recommendation: **Default with override.**
- **Posture**: ship a sensible default (`10.96.0.0/24` per the user's instinct, or wider — see capacity note below); allow operator override via config.
- **Evidence basis**: every single-node-friendly K8s distro (k3s, MicroK8s, k0s, Talos single-node) does exactly this, with zero exceptions in the surveyed set (Findings 5a–5c). The defaults vary (k3s `/16`, MicroK8s `/24`, k0s/Talos `/12`) but the *posture* is uniform.
- **Capacity note**: `10.96.0.0/24` ≈ 254 VIPs. MicroK8s ships the same size and has been documented as too small for production (issue [#1917](https://github.com/canonical/microk8s/issues/1917)). For an internal-only allocation ID space tied to a single-node XDP dataplane, `/24` may be adequate for a developer-experience default, but Overdrive should size it for the workload count it expects to support on a single node. If Phase 1 single-node is expected to run hundreds of services, prefer `/20` (`10.96.0.0/20` ≈ 4094) or `/16` (matching k3s) over `/24`.
- **What "internal allocation IDs, NOT globally-routable" buys**: since Overdrive's Phase 1 VIPs are XDP-internal and not exposed on the host's LAN, host-network collision is *not* a correctness failure — it's a confusability concern (operator running `ip route` sees overlap). The cost of the wrong default is therefore lower than for K8s, where the service CIDR participates in host routing decisions via kube-proxy iptables.

### Phase 2 (HA mode, multi-node Raft) recommendation: **Default with strong warning, NOT refuse-to-start.**
- **Posture**: same default as Phase 1; emit a structured `health.startup.warn` event when the default is in use AND multi-node mode is active; let the operator override.
- **Evidence basis**: kubeadm, k0s, and Talos all support production HA multi-node clusters with the same defaulted posture as their single-node modes (Finding 2a, 2c). The CIDR is data-path-only (kube-proxy / IPVS / Cilium handle it); it is not routed across nodes as a real network (Finding 2c, PR #35290 rationale). Cross-node routing conflicts on the service CIDR are not a documented failure mode in any surveyed platform.
- **Why "refuse-to-start in HA" is over-strict**: it would make Overdrive's HA posture stricter than Kubernetes's own production multi-node posture, with no documented prior-art justification. The argument for refuse-to-start ("cross-node routing conflicts matter") is not borne out by the K8s evidence — the CIDR is virtual at the dataplane layer, and conflicts surface as host-network collisions (Phase 1 problem) not as cross-node routing collisions.
- **The strong-warning surface**: in HA mode, log on every node at startup if the default is in use. Operators running a real production multi-node Overdrive cluster will see this in their startup health stream and have the option to set explicit values. This matches the "default with override" posture but adds visibility for the cohort most likely to care.

### Documented failure modes prior art has surfaced

These are the host-network / kernel-route collision modes operators have hit in K8s-land:

1. **Host LAN overlap with default range.** If the host's LAN is itself `10.96.0.0/something`, kube-proxy iptables rules can capture traffic destined for the LAN. Mitigation in K8s: PR #35290's choice of `10.96.0.0/12` was deliberate to avoid common RFC-1918 conventions, but is not collision-free for operators on `10.0.0.0/8` networks.
2. **NetworkManager interference with kube-proxy rules.** Documented in multiple K8s setup guides; not platform-corrected — the workaround is `NM_CONTROLLED=no` or to disable NetworkManager management of CNI-managed interfaces. Not surfaced in the official trusted-domain docs I cross-referenced; mark as Medium-confidence community knowledge.
3. **kubelet `cluster-dns` falling outside service-CIDR after a custom override.** Documented in [kubernetes/kubernetes#54252](https://github.com/kubernetes/kubernetes/issues/54252) — kubeadm does not preflight-check this; operators discover it post-init when DNS breaks.
4. **Service ClusterIP outside CIDR after CIDR change.** Documented in [kubernetes/kubernetes#86497](https://github.com/kubernetes/kubernetes/issues/86497) — apiserver detects post-hoc and emits `please recreate` errors; no automated migration.

**None of these are reasons for Overdrive to refuse-to-start.** They are reasons to (a) pick a sensible default that avoids the most common collision classes, (b) emit a structured warning when the default is in use, and (c) document the operator's responsibility to verify against their host network. This is exactly the K8s-family posture.

### Synthesis: the load-bearing observation

The user's framing — "operator-supplied should ONLY be required in HA mode" — is partially supported but ultimately over-strict relative to prior art. The stronger conclusion the evidence supports is:

> **Default in both phases; emit a startup warning in HA mode when the default is in use; never refuse-to-start over the service CIDR alone.**

The reason is that prior art treats the service CIDR as a *data-path* concern (the dataplane interprets it; the host doesn't route it), and the failure modes that *do* matter (host LAN overlap) are an operator-pre-flight responsibility that no platform has succeeded in automating. Refusing-to-start would not actually catch those failure modes — only operator vigilance does — so the friction of refuse-to-start buys no real safety, just real onboarding cost.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|---|---|---|---|---|---|
| Kubernetes kubeadm init reference | kubernetes.io | High (1.0) | official | 2026-05-15 | Y (PR #35290) |
| Kubernetes PR #35290 | github.com/kubernetes/kubernetes | High (1.0) | official-source | 2026-05-15 | Y (kubeadm docs) |
| Kubernetes issue #86497 | github.com/kubernetes/kubernetes | High (1.0) | official-tracker | 2026-05-15 | N (cited once for failure-mode evidence) |
| Kubernetes issue #54252 | github.com/kubernetes/kubernetes | High (1.0) | official-tracker | 2026-05-15 | N (cited once for under-validation evidence) |
| k3s docs (CLI server) | docs.k3s.io | High (1.0) | official | 2026-05-15 | N (authoritative-single) |
| k0s docs (configuration) | docs.k0sproject.io | High (1.0) | official | 2026-05-15 | N (authoritative-single) |
| MicroK8s CNI configuration | microk8s.io | High (1.0) | official | 2026-05-15 | Y (canonical/microk8s#1917) |
| MicroK8s issue #1917 | github.com/canonical/microk8s | Medium-High (0.8) | tracker | 2026-05-15 | Y (microk8s.io docs) |
| Talos v1.10 config reference | docs.siderolabs.com | High (1.0) | official | 2026-05-15 | Y (multiple secondary) |
| siderolabs/talos issue #8298 | github.com/siderolabs/talos | Medium-High (0.8) | tracker | 2026-05-15 | Y (used to confirm defaults are current) |
| Nomad networking docs | developer.hashicorp.com | Medium (0.6 — not in trusted-domain list) | vendor-docs | 2026-05-15 | Partial (cross-referenced same vendor's two pages) |
| Nomad service block docs | developer.hashicorp.com | Medium (0.6) | vendor-docs | 2026-05-15 | Partial (same as above) |

**Reputation distribution**: High: 9 (75%) | Medium-High: 2 (17%) | Medium: 1 (8%). Avg ≈ 0.93. All claims about K8s, k3s, k0s, Talos, MicroK8s rest on at least one high-reputation source; the Nomad claim ("no VIP concept") is a structural-fact claim cross-referenced across two HashiCorp doc pages but the domain is not in the project's trusted-domain list — I have flagged this as the lowest-confidence finding and the claim is robust enough to survive that downgrade (it is a well-known architectural fact across multiple decades of HashiCorp orchestration ecosystem writing).

## Knowledge Gaps

### Gap 1: kube-apiserver `--service-cluster-ip-range` flag canonical default text
**Issue**: WebFetch against `kubernetes.io/docs/reference/command-line-tools-reference/kube-apiserver/` returned truncated content that did not include the flag's reference block. **Attempted**: direct WebFetch on the kube-apiserver page; recovered the canonical default via the kubeadm-init reference (which forwards the flag) and via PR #35290 which changed the default. **Recommendation**: if precise textual citation of the kube-apiserver flag is needed, fetch the raw kubernetes/kubernetes source at `cmd/kube-apiserver/app/options/options.go` or the OpenAPI flag dump.

### Gap 2: MicroK8s default-choice rationale
**Issue**: Canonical chose `10.152.183.0/24` rather than aligning with upstream's `10.96.0.0/12`. No written rationale found in trusted-domain sources for why MicroK8s picks this specific narrow range. **Attempted**: microk8s.io docs and canonical/microk8s tracker. **Recommendation**: not load-bearing for Overdrive's decision; flagging for completeness.

### Gap 3: Nomad source-domain trust
**Issue**: developer.hashicorp.com is not in the project's trusted-domain list. The "Nomad has no VIP concept" claim rests on two pages from the same domain. **Mitigation**: the claim is a well-known structural fact about Nomad's architecture and is implicitly cross-referenced by every K8s-vs-Nomad comparison in the broader ecosystem. The claim survives the trust downgrade because no contradicting evidence exists. **Recommendation**: if higher confidence is needed, cross-reference against Nomad's open-source repository on github.com directly (which IS in the trusted list).

### Gap 4: NetworkManager / kernel route table conflict mechanics
**Issue**: section Q4 / Phase 2 recommendation references NetworkManager interference as a community-known failure mode but no trusted-domain source was located in the budget. **Recommendation**: low priority — Overdrive's Phase 1 VIPs are XDP-internal and don't participate in host routing, so this failure mode is structurally less relevant to Overdrive than to K8s.

## Full Citations

[1] Kubernetes Authors. "kubeadm init reference". kubernetes.io. Accessed 2026-05-15. https://kubernetes.io/docs/reference/setup-tools/kubeadm/kubeadm-init/

[2] errordeveloper et al. "Change default service IP range to 10.96/12". kubernetes/kubernetes PR #35290. Accessed 2026-05-15. https://github.com/kubernetes/kubernetes/pull/35290

[3] Kubernetes contributors. "Service is not within the service CIDR please recreate". kubernetes/kubernetes issue #86497. Accessed 2026-05-15. https://github.com/kubernetes/kubernetes/issues/86497

[4] Kubernetes contributors. "Kubeadm init should check that kubelet's cluster-dns config matches kubeadm's service-cidr". kubernetes/kubernetes issue #54252. Accessed 2026-05-15. https://github.com/kubernetes/kubernetes/issues/54252

[5] k3s Authors. "k3s server CLI reference". docs.k3s.io. Accessed 2026-05-15. https://docs.k3s.io/cli/server

[6] k0s Authors. "k0s configuration reference". docs.k0sproject.io. Accessed 2026-05-15. https://docs.k0sproject.io/stable/configuration/

[7] Canonical. "MicroK8s CNI configuration". microk8s.io. Accessed 2026-05-15. https://microk8s.io/docs/change-cidr

[8] MicroK8s contributors. "failed to allocate a serviceIP: range is full". canonical/microk8s issue #1917. Accessed 2026-05-15. https://github.com/canonical/microk8s/issues/1917

[9] Sidero Labs. "Talos Linux v1.10 v1alpha1 configuration reference". docs.siderolabs.com. Accessed 2026-05-15. https://docs.siderolabs.com/talos/v1.10/reference/configuration/v1alpha1/config/

[10] Talos contributors. "Feature request - change the defaults for podsubnets and servicesubnets". siderolabs/talos issue #8298. Accessed 2026-05-15. https://github.com/siderolabs/talos/issues/8298

[11] HashiCorp. "Nomad networking". developer.hashicorp.com. Accessed 2026-05-15. https://developer.hashicorp.com/nomad/docs/networking

[12] HashiCorp. "Nomad service block reference". developer.hashicorp.com. Accessed 2026-05-15. https://developer.hashicorp.com/nomad/docs/job-specification/service
