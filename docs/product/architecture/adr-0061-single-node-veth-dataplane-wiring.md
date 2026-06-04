# ADR-0061 — Single-node dataplane interface wiring: dedicated veth pair, two XDP programs on two distinct ifaces, auto-provisioned at boot

## Status

Accepted. 2026-06-02. Decision-makers: Titan (system-design, proposing);
ratified by the user 2026-06-02. Tags: phase-2, dataplane, single-node,
production-boot, xdp, bug-fix.

**Amended 2026-06-03 (idempotent converge-on-boot)** — § 3 / § 3.1 and
DQ-4 reframed from **"detect-and-reuse / adopt untouched"** to
**"idempotent, per-resource desired-vs-actual converge-on-boot"**. The
prior "adopt untouched" semantics returned early on `ip link show`
success and left a half-provisioned pair — created by a serve boot that
crashed *after* the atomic `ip link add` but *before*
address/link-up/route assignment — incomplete, surfacing two layers
downstream as a misleading `iface::resolve_iface_ipv4` /
`IfaceAddrResolution` error. Because Overdrive's single-node target is a
Yocto-built immutable appliance image with **no SSH and no operator
shell** (Talos-style), the originally-floated "fail loud, tell the
operator to run `ip link del <cli>` and retry" remediation has no
operator to action it. The corrected model — validated by Talos Linux's
network controllers (`docs/research/dataplane/talos-network-reconciliation-self-healing.md`,
confidence High) — converges each resource independently against
observed kernel state and repairs partial state in place with zero
human intervention. The full continuous network-reconciler model (port
trait + Sim adapter + observed-state hydration + continuous tick) is the
deferred direction-of-travel tracked in **issue #197**; one-shot
converge-on-boot is the correct Phase-1 single-node minimum (research
R7). This is a **refinement** of the § 3 idempotence intent and a narrow
clarification of DQ-4 — not a reversal of the veth-pair / two-iface
decision (§ 1, § 2), which stands verbatim.

**Companion ADRs**: ADR-0043 (XDP L4LB three-iface transit test
topology), ADR-0045 (`bpf_redirect_neigh` / cross-iface datapath —
**preserved, not reversed**), ADR-0052 (backend discovery bridge +
`EbpfDataplane` production single-mode boot), ADR-0049 (platform-issued
Service VIP allocator), ADR-0025 (single-node startup wiring).

**Tracks**: production `overdrive serve` default-config boot abort
(RCA evidence:
`verification/expectations/O03-deploy-udp-service-accepted-udp-intent/evidence/serve.log`).

## Context

Production `overdrive serve` cannot boot in its **default single-node
configuration**.

`EbpfDataplane::new_with_pin_dir`
(`crates/overdrive-dataplane/src/lib.rs`) attaches **two distinct** XDP
programs: forward `xdp_service_map_lookup` to `client_iface` ingress
(~L489), reverse `xdp_reverse_nat_lookup` to `backend_iface` ingress
(~L534). The default single-node config —
`DataplaneConfig::loopback()`
(`crates/overdrive-control-plane/src/dataplane_config.rs:61-63`), wired
as `ServerConfig.dataplane` at
`crates/overdrive-control-plane/src/lib.rs:526` — sets **both ifaces to
`"lo"`**.

The kernel permits **exactly one program on a netdev's XDP hook**
(research `docs/research/dataplane/xdp-multiprog-same-iface-aya-research.md`
Finding 1.1). The second attach therefore returns `EBUSY` and boot
aborts. The error message hardcodes `DRV_MODE`
(`crates/overdrive-dataplane/src/lib.rs:551`), masking the real `EBUSY`
mechanism behind a misleading "native attach failed" string.

`lo` is wrong on a **second** count. It has no native XDP driver, so the
attach is always generic/SKB mode — which Cilium documents can **bypass
cloned skbs** on the TCP retransmit / segmentation path (research Finding
6.1; cilium/cilium #12910). A loopback dataplane may silently miss
traffic even after the collision is fixed. `lo` is the wrong attach
target, not merely a collision to dodge.

Phase 1 is **single-node-in-scope** (ADR-0025); the default `serve` path
MUST boot and MUST steer traffic correctly. The research enumerated five
options and recommended **Option E (dedicated veth pair)**, optionally
combined with **Option B (merge the two programs)**, leaving open
**Gap G-4: how single-node steers traffic through the veth client side**.
This ADR settles the wiring and closes G-4 — and sharpens the research's
B-as-fallback recommendation: B is **dominated** for Overdrive's current
datapath.

## Decision

### 1. Provision a dedicated veth pair for single-node; do not attach to `lo`

Single-node `overdrive serve` attaches its two XDP programs to a
dedicated host-netns **veth pair** — `ovd-veth-cli` (client side, forward
program) ↔ `ovd-veth-bk` (backend side, reverse program) — instead of
pointing both `client_iface` and `backend_iface` at `lo`.

This restores the **two-distinct-iface invariant** the production code
already assumes (the two `attach` calls target two netdev XDP hooks → no
`EBUSY`), and restores **native veth XDP** semantics (no generic-mode
cloned-skb bypass). Zero kernel-side or BPF-mechanism change; the
`EbpfDataplane` attach shape is unchanged.

The default config values become:

```toml
[dataplane]
client_iface  = "ovd-veth-cli"   # was the bug-shaped "lo"
backend_iface = "ovd-veth-bk"
```

Two-NIC / multi-NIC production deployments override these with real NIC
names (and skip auto-provisioning — § 3); that path is **not regressed**.

### 2. Keep two distinct programs on two distinct ifaces — reject merging

The two programs are **not** merged into one staged `#[xdp]` entry
(research Option B). The research treated B as a viable
fallback/complement under the assumption that the two programs are
independent early-returning stages on **one** ingress hook. That
assumption held when the forward path was `XDP_TX` (bounce out the same
iface). **ADR-0045 pivoted the datapath to cross-iface delivery**:
`xdp_service_map_lookup` does `bpf_fib_lookup` + `bpf_redirect(fib.ifindex,
0)` to push the rewritten frame **out the backend-facing iface**;
`xdp_reverse_nat_lookup` redirects responses back. A merged program on a
**single** hook has **no second iface to `bpf_redirect` to** — the
forward→backend cross-iface hop is the structural point of the two-iface
split and cannot be reproduced on one netdev. `ThreeIfaceTopology`'s own
docstring states it: *"`XDP_TX` … cannot deliver from `lb_veth_a` to
`lb_veth_b`. Cross-iface delivery uses `bpf_redirect(fib.ifindex, 0)`."*

Merging would **reverse ADR-0045's pivot** and risk every veth e2e
Tier-3 test that exercises the cross-iface programs. Two distinct
programs on two distinct veth ifaces is the only shape that boots, fixes
correctness, and preserves the landed datapath.

### 3. Auto-provision the veth pair at `serve` boot

`overdrive serve` provisions the veth pair, addresses, and route at boot,
**before** `EbpfDataplane::new`, in the host network namespace
(single-node — no netns boundary).

**§ 3.1 — Idempotent converge-on-boot (amended 2026-06-03).** The
provisioner does **not** short-circuit on the presence of the client
iface. It performs a one-shot, per-resource desired-vs-actual converge
against observed kernel state, repairing whatever the last (possibly
crashed) boot left partial. Each resource is converged independently
(research Finding 2.3 / 3.4 / R2 — *"a MISSING address on an EXISTING
link is a normal `Address.New()`, and `EEXIST` is explicitly swallowed
(idempotent)"*):

1. **veth pair** — `ip link show <client_iface>`; if absent,
   `ip link add <client_iface> type veth peer name <backend_iface>`
   (atomic — creates both ends together); if present, noop.
2. **client address** — assign the VIP-range gateway IP to
   `<client_iface>` if missing; swallow `EEXIST` (already-assigned is the
   success case, not a failure).
3. **backend address** — when a backend gateway is derived (the range
   has a second usable host), assign it to `<backend_iface>` if missing;
   swallow `EEXIST`.
4. **both ends UP** — `ip link set <iface> up` for each end; idempotent
   (re-upping an already-up link is a noop).
5. **on-link route** — `ip route add <vip_range> dev <client_iface>` so
   the VIP range is on-link via the client-side veth; swallow the
   `File exists` collision (the code already does — the connected route
   the kernel auto-creates on address assignment legitimately collides
   here).

Re-running the converge over an **already-complete** pair is a silent
noop at every step. Re-running over a **half-provisioned** pair — the
crash-mid-provision case — completes the missing resources in place. The
provisioner therefore tolerates being interrupted at any point and
re-run from the top across reboots (research R7: converge-on-boot gives
self-healing across reboots without a continuous controller). This is
the **same idempotence the original § 3 intended**, sharpened from "adopt
the present pair untouched" to "converge each resource to its desired
state" so a partial pair is *completed*, not adopted incomplete.

**§ 3.2 — The genuinely-corrupted edge: client iface present, peer
absent (amended 2026-06-03).** The crash-mid-provision case is fully
covered by § 3.1: `ip link add ... peer ...` is **atomic** and creates
both ends together, so a crash after it can only leave a pair that is
*present but under-addressed* — which § 3.1 completes. The one residual
corrupted shape is **client iface present but its veth peer absent**.
This is only reachable if the peer was *separately* deleted or moved to
another netns after creation — never the crash-mid-provision path.

**Ruling: recreate the pair (option a).** When `<client_iface>` is
present but `<backend_iface>` (its declared peer) is absent, the
provisioner deletes the orphaned client end and recreates the pair from
scratch, then converges addresses/up/route per § 3.1. Rationale: the
default-veth-name gate (§ 3.3 below) guarantees this path is reachable
**only** for Overdrive-owned ifaces — a half-pair carrying the default
veth names is Overdrive's by construction, since an operator naming real
NICs skips provision entirely. An appliance OS with no operator shell
must self-heal a corrupted resource it owns; refusing (option b) would
strand a Yocto image at a `serve`-refuses-to-boot state with no human to
clear it — the same dead-end the "fail loud, run `ip link del`"
remediation hit. The alternative (b — refuse with a distinct
`VethPeerMissing` error) is **rejected** for the single-node appliance
target precisely because there is no operator to action the refusal;
it would be the correct ruling only on a host where unowned ifaces could
plausibly collide with the default names, which the gate forecloses.
This is a **narrow refinement of DQ-4's "never tear down"**, not a
reversal: DQ-4 governs a *usable* pair (no teardown on shutdown / for
reuse); recreating a *corrupted, Overdrive-owned* half-pair to make it
usable is the self-heal an appliance OS requires.

**§ 3.3 — Own only what you declare.** Convergence (including the § 3.2
recreate) touches **only** the configured default veth ifaces. The
serve-boot provision gate fires solely when the configured ifaces equal
the `DEFAULT_CLIENT_IFACE` / `DEFAULT_BACKEND_IFACE` SSOT consts; an
operator who names real NICs skips provision entirely (unchanged from §
3 original). This mirrors Talos's discipline — *"Own only what you
declare; do not clobber foreign interfaces … Talos does not prune
unowned links"* (research R6 / Finding 3.5). The recreate in § 3.2 is
not link-pruning of foreign state; it is repair of a declared, owned
resource.

Provision-at-serve-boot is the **single-node default**, but it is **not
the only** owner of the interface lifecycle. Because the converge is
idempotent and completes partial state, an **OS image (Yocto) or VM-boot
provisioner (Lima)** can stand the veth pair up at OS-init time — exactly
how the current Lima dev VM provisions its veth/networking at VM boot —
and `serve` will converge over it (a noop when complete, a completion
when partial). The two mechanisms are **interchangeable by construction**
because the converge is idempotent: the same converge-and-complete
property that makes a serve **restart** (or crash-recovery) cheap (DQ-4)
is the property that lets an external provisioner own the interface
(DQ-1). One property serves both cases. The provisioner therefore never
tears down a **usable** pair on shutdown (leave-and-reuse), so whichever
entity created it — `serve`, a Yocto image, or the Lima VM boot —
retains ownership across the process lifetime.

The provisioner is a **new `adapter-host` component** (in
`overdrive-worker` or `overdrive-host`), NOT in `overdrive-testing`
(which is dev-dep-only and shells `ip netns add`; a production binary
must never link it). It mirrors a **subset** of
`crates/overdrive-testing/src/netns.rs::ThreeIfaceTopology::create` — the
veth/address/route sequence **without** the netns machinery.

`CAP_NET_ADMIN` is already a hard precondition for XDP attach and cgroup
management (ADR-0026, ADR-0052), so auto-provision adds **no new
privilege**. Per ADR-0025 § Alternative E, hard-refusal is reserved for
safety properties that genuinely cannot be defaulted (cgroup delegation);
interface provisioning is defaultable, so "single-node serve just boots"
is the correct ergonomic floor.

The gateway IP and route are **derived at provision time from
`[dataplane.vip_allocator].ranges`** (ADR-0049), not persisted — per
`.claude/rules/development.md` § "Persist inputs, not derived state".

### 4. Single-node traffic steering (closes Gap G-4)

The single host plays all three roles the Tier-3 topology splits across
`client-ns` / `lb-ns` / `backend-ns` (ADR-0043), collapsed into the host
netns:

- An `overdrive deploy` client or workload connecting to `<vip>:<port>`
  routes the SYN **out `ovd-veth-cli`** (the host route from § 3.3);
  the forward XDP program at the peer ingress does the SERVICE_MAP
  lookup + `bpf_fib_lookup` + `bpf_redirect` across the pair to the
  backend (ADR-0045, unchanged).
- The VIP-range gateway address on `ovd-veth-cli` makes the VIP on-link
  so ARP/neigh resolves (same trick as `ThreeIfaceTopology` where the
  VIP IS the address on `lb_veth_a`).
- The `BackendDiscoveryBridge`'s `host_ipv4` (ADR-0052 § 1) resolves via
  `getifaddrs` on `client_iface` → the veth client-side address, so
  `Backend.ipv4` points at a reachable on-host address. **No bridge code
  change.**
- `bpf_fib_lookup` resolves the egress iface + next-hop MAC from the host
  routing table the provisioner populated — exactly the single-NIC
  routing-host model ADR-0043 § Context describes, on one host.

### 5. Honest `EBUSY` diagnostic (defensive depth)

A new typed variant `DataplaneError::IfaceXdpSlotBusy { iface: String }`
in `overdrive-core`. The attach path classifies `raw_os_error() ==
EBUSY` into this variant **before** the current `DRV_MODE`-hardcoded
string (`lib.rs:551`), per `.claude/rules/development.md` § Errors
("distinct failure modes get distinct error variants; never collapse
into a catch-all whose Display prescribes the wrong remediation"). The
`Display` names the iface and the real cause + remediation (verify
`client_iface ≠ backend_iface`; detach a stale Overdrive XDP program per
`debugging.md` § "Leftover XDP attachments across runs"). It bubbles up
through the existing `DataplaneBootError::Construct` (ADR-0052 § 3) — no
new boot-error variant. E makes the collision unreachable on the default
path; this guard catches the operator who points both ifaces at one real
NIC or whose veth provisioning half-failed.

## Alternatives Considered

### A — Merge the two programs into one staged XDP entry (research Option B), on a veth pair (E+B)

**Rejected.** § 2: ADR-0045's cross-iface `bpf_redirect` datapath needs
two ifaces; a single merged program on one hook has no second iface to
redirect to and cannot reproduce the forward→backend hop. It would
reverse ADR-0045 and risk the veth e2e tests, in exchange for "one
program instead of two" — no benefit for Overdrive's current datapath.

### B — Merge programs and keep attaching to `lo` (research Option B, B-on-`lo`)

**Rejected.** Same single-hook cross-iface defect as A, plus it keeps the
`lo`/generic-XDP cloned-skb correctness hole (Finding 6.1). Boots, but
ships a known silent-traffic-miss mode.

### C — `dummy` interface instead of veth

**Rejected.** `dummy` ifaces force generic XDP (same cloned-skb hole as
`lo`) and provide only one netdev (no second hook). veth is the only
single-host option that gives two native-XDP hooks.

### D — Tail calls (research Option C) / libxdp dispatcher (research Option A) / XDP+TC split (research Option D)

**Rejected** per the research: C is dominated by a merged program for two
fixed stages (and shares A/B's single-hook problem); A needs a
multi-week libxdp-dispatcher hand-roll or C-FFI dep (aya 0.13.x ships no
dispatcher); D reverses ADR-0045 by re-porting reverse-NAT to TC. None
fix the `lo` correctness hole and all cost more than E.

### E — `dataplane setup` subcommand or operator-external provisioning (D-G4 (b)/(c))

**Deferred to user ratification, not adopted as default.** An explicit
provisioning verb (b) or operator-managed ifaces (c) trade the
"just-boots" ergonomic floor for an extra operator step and a "did you
run setup?" failure mode — the friction ADR-0025 § Alt E rejected.
Auto-provision (a) is recommended; (b)/(c) remain available for operators
who name real NICs (they skip auto-provision).

## Consequences

### Positive

- **Default single-node `serve` boots.** The `EBUSY` abort is removed at
  its root (two distinct hooks).
- **Correct XDP semantics.** Native veth XDP eliminates the
  generic-mode cloned-skb bypass that `lo` cannot avoid.
- **ADR-0045 preserved verbatim.** The cross-iface `bpf_redirect`
  datapath, the two-program shape, and every veth e2e Tier-3 test are
  untouched.
- **Zero kernel-side / BPF change; no kernel-floor bump; no FFI dep.**
  Within aya 0.13.x's surface.
- **Reuses the proven topology shape.** The production provisioner
  mirrors a subset of the ADR-0043 Tier-3 topology the tests already
  validate.
- **Honest diagnostics.** `IfaceXdpSlotBusy` replaces the masking
  `DRV_MODE` string for the residual collision case.
- **OS-image / VM-boot provisioning is compatible by construction
  (Yocto, Lima).** Serve-boot provisioning converges a pre-existing veth
  pair to its desired state (completing partial state, noop when already
  complete) rather than failing or adopting it incomplete, so an external
  provisioner that owns interface setup at OS-init time — the future
  **Yocto OS image**, or the current **Lima dev VM** that already
  provisions its veth/networking at VM boot — slots in transparently.
  `serve` converges over the pair it finds. Serve-boot auto-provisioning
  and OS-image pre-provisioning are **interchangeable** because the
  converge is idempotent; there is no "did the OS already set this up?"
  branch to maintain. This is the **same idempotent-converge property**
  as the restart / crash-recovery case (DQ-4) — one property covers
  serve-restart, crash-mid-provision recovery, and OS-pre-provisioned
  ownership, so no second mechanism is needed. *(Amended 2026-06-03 —
  "adopts untouched / reuses untouched" sharpened to "converges to
  desired state"; see § 3.1.)*
- **Appliance self-heal with no operator (amended 2026-06-03).** On the
  Yocto immutable appliance target (no SSH, no operator shell), the
  provisioner repairs a partial or corrupted-owned pair in place across
  reboots with zero human intervention — the property a Talos-style
  appliance OS requires (research, confidence High). The rejected "fail
  loud, run `ip link del` and retry" remediation had no operator to
  action it.

### Negative

- **`serve` gains a boot-time veth-provisioning step.** New `adapter-host`
  component + `CAP_NET_ADMIN` use (already required). Idempotent
  converge-on-boot keeps restart cheap and recovers a crash-mid-provision
  half-pair (§ 3.1); teardown is leave-a-usable-pair (mirrors bpffs-pin
  persistence per ADR-0052 § 3 Drop) — manual cleanup of a stale pair
  documented in debugging.md (DQ-4, resolved: idempotent converge).
- **Single-node steering is IPv4-only**, matching the IPv4-only datapath
  today. IPv6 single-node veth steering for AF_INET6 VIPs is **deferred
  to issue #195** (DQ-3); it depends on IPv6 dataplane forwarding (issue
  #155) and is not part of this fix.
- **Two-NIC operators must name real ifaces to skip auto-provision.**
  The default-veth-names gate (vs an explicit `provision` knob) is the
  fix's posture; the explicit `[dataplane] provision = "veth" | "none"`
  opt-out knob is **deferred to issue #194** (DQ-2).
- **One-shot converge-on-boot only; no continuous reconciler (amended
  2026-06-03).** The provisioner converges once per boot, before
  `EbpfDataplane::new`. This self-heals across *reboots* (each boot
  re-diffs and completes whatever the last, possibly-crashed boot left
  partial — research R7), which fully resolves the partial-provision
  bug. It does **not** repair *runtime* drift — an address deleted by
  something else *while serve is up* is not restored until the next
  boot. Per research R7 and Phase-1 single-node scope, runtime drift is
  **not in the threat model**: the veth pair is provisioned once and not
  externally perturbed. The full continuous network-reconciler model
  (a `NetworkProvisioner` port trait + `Sim` adapter + observed-state
  hydration + continuous tick, in the §18 reconciler shape) is the
  deferred direction-of-travel tracked in **issue #197** — a forward
  pointer to that tracked design decision, **not** planned or imminent
  Phase-1 work.

### Quality-attribute impact

- **Correctness — bug fix structurally closed**: positive (large). The
  default boot path works and steers traffic.
- **Reliability — fault tolerance**: positive. Native veth XDP removes a
  silent-traffic-miss mode; `IfaceXdpSlotBusy` surfaces the residual
  collision honestly.
- **Maintainability — operability**: positive. "single-node serve just
  boots"; structured EBUSY remediation.
- **Portability**: neutral. veth + XDP remain Linux-only
  (`#[cfg(target_os = "linux")]`, existing).

## Compliance — what survives from prior ADRs

- **ADR-0043** (three-iface test topology) — preserved; the production
  provisioner mirrors a host-netns subset, the test topology is unchanged.
- **ADR-0045** (cross-iface `bpf_redirect` datapath) — **preserved
  verbatim**; this decision depends on it.
- **ADR-0052** (`EbpfDataplane` production boot) — extended additively:
  veth provisioning runs **before** `EbpfDataplane::new`; the parse →
  construct → probe → use composition is unchanged. `DataplaneBootError`
  carries the new `IfaceXdpSlotBusy` cause via `Construct`.
- **ADR-0049** (Service VIP allocator) — consumed: the provisioner
  derives the on-link gateway + route from `[dataplane.vip_allocator]
  .ranges`.
- **ADR-0025** (single-node startup wiring) — followed: defaultable
  provisioning gets a default (not a hard refusal); hard refusal stays
  reserved for cgroup delegation.
- **`.claude/rules/development.md`** — § Errors (distinct failure mode →
  distinct variant: the per-`ip(8)`-step `VethProvisionError` variants,
  plus the `VethPeerMissing` shape considered and rejected in § 3.2),
  § Persist inputs not derived state (gateway/route derived at provision
  time), § Shared real-infra fixtures (`overdrive-testing` stays
  dev-dep-only; production provisioner is a separate `adapter-host`
  component).
- **Idempotent converge-on-boot (amended 2026-06-03)** — § 3.1 / § 3.2
  follow the Talos-validated per-resource desired-vs-actual model
  (research Findings 2.3 / 3.4 / 3.5; R2 / R6 / R7). Add-if-missing with
  `EEXIST` / `File exists` swallowed; recreate only a corrupted,
  Overdrive-owned half-pair; touch only declared default-veth ifaces.

## Changed assumptions / design constraint — OS-image-adoptable provisioning

A load-bearing constraint added at ratification, beyond the original
"auto-provision at boot" recommendation:

> **Serve-boot provisioning MUST be idempotent and compatible with
> OS-image pre-provisioning.** Provision-at-serve-boot is the single-node
> default; it converges a pre-existing veth pair to its desired state
> (completing any partial state, noop when already complete) so an OS
> image (Yocto) or VM-boot provisioner (Lima) can own the interface
> lifecycle instead. The two mechanisms are interchangeable by
> construction because the converge is idempotent.

This sharpens § 3 from "create the pair, reuse on restart" to "converge
the pair to desired state whoever created it" *(amended 2026-06-03 —
was "adopt the pair whoever created it"; "adopt untouched" left a
crash-mid-provision half-pair incomplete, see § 3.1)*. The Phase-1
expectation is that the **Yocto OS image** will set up networking at
OS-init time — the same shape the **Lima dev VM** already uses to
provision its veth/networking at VM boot. Serve-boot provisioning
therefore must never assume it is the creator: it checks each resource
(`ip link show`, address presence, link state, route presence),
completes whatever is missing, and only creates the pair from scratch
when nothing pre-exists. The constraint is satisfied by the **same
idempotent-converge property** DQ-4 requires for serve restarts and
crash recovery — one property, two callers (a restarting / recovering
`serve`; an external OS/VM provisioner). No separate "OS-provisioned
mode" branch exists or is needed.

## Open questions — resolved at ratification (2026-06-02)

All four open questions are decided. The two scope-cuts are tracked by
existing GitHub issues; neither is created by this ADR.

- **DQ-1** (provisioning owner) — **resolved: serve-boot auto-provision
  (option a)**, made adoptable by an OS-image / VM-boot provisioner via
  the idempotent converge-on-boot property (§ 3.1). The Yocto OS image
  and the Lima dev VM can own interface setup at OS-init time; serve-boot
  converges a pre-existing pair (noop when complete, completion when
  partial). Serve-boot and OS-image provisioning are interchangeable by
  construction. *(Amended 2026-06-03 — "reuses a pre-existing pair"
  sharpened to "converges a pre-existing pair"; see § 3.1.)*
- **DQ-4** (teardown semantics) — **resolved: idempotent converge,
  never tear down a usable pair** on shutdown (mirrors bpffs-pin
  persistence per ADR-0052 § 3). This is the single property that makes
  both DQ-1's OS-image adoption and the serve-restart / crash-recovery
  case work. *(Amended 2026-06-03 — "idempotent reuse" sharpened to
  "idempotent converge"; "never tear down" refined to "never tear down a
  **usable** pair" — § 3.2 recreates a genuinely-corrupted,
  Overdrive-owned half-pair (client present, peer absent) to self-heal
  it, which is repair of a declared resource, not a teardown-for-reuse.)*
- **DQ-5** (continuous network reconciler) — **deferred to issue #197.**
  One-shot converge-on-boot is the Phase-1 single-node minimum (research
  R7); a continuous reconciler that repairs runtime drift while serve is
  up is the deferred direction-of-travel, not Phase-1 scope. #197 is a
  forward pointer to that tracked design decision. *(Added 2026-06-03.)*
- **DQ-2** (explicit `[dataplane] provision = "veth" | "none"` opt-out
  knob) — **deferred to issue #194.** The fix ships implicit-by-default
  veth names + idempotent reuse; the explicit operator-tunable knob is
  tracked in #194.
- **DQ-3** (IPv6 / AF_INET6 single-node veth steering) — **deferred to
  issue #195.** Depends on IPv6 dataplane forwarding (#155). This fix is
  IPv4-only.

## References

- `docs/research/dataplane/xdp-multiprog-same-iface-aya-research.md` —
  five-option analysis; recommends E (+B); Gap G-4.
- `docs/research/dataplane/talos-network-reconciliation-self-healing.md`
  — (amended 2026-06-03) Talos-validated idempotent per-resource
  converge model; Findings 2.3 / 3.4 / 3.5; recommendations R2 (swallow
  `EEXIST`), R6 (own only what you declare), R7 (one-shot
  converge-on-boot sufficient for single-node, continuous reconciler not
  required). Confidence High, 9 trusted sources.
- **GitHub issue #197** — deferred continuous network-reconciler model
  (`NetworkProvisioner` port trait + Sim adapter + observed-state
  hydration + continuous tick). Direction-of-travel beyond Phase-1
  converge-on-boot; not planned/imminent work (amended 2026-06-03).
- `docs/research/dataplane/aya-rs-usage-comprehensive-research.md` §B.1 —
  aya 0.13.x XDP attach surface + native→SKB fallback.
- ADR-0043 — XDP L4LB three-iface transit test topology.
- ADR-0045 — `bpf_redirect_neigh` / cross-iface datapath (preserved).
- ADR-0052 — backend discovery bridge + `EbpfDataplane` production boot.
- ADR-0049 — platform-issued Service VIP allocator.
- ADR-0025 — single-node startup wiring (ergonomic-floor precedent).
- `crates/overdrive-dataplane/src/lib.rs` — the two attaches; the
  `DRV_MODE`-masking error string.
- `crates/overdrive-control-plane/src/dataplane_config.rs` — `loopback()`
  default + `[dataplane]` parser.
- `crates/overdrive-testing/src/netns.rs` — `ThreeIfaceTopology` reference
  shape.
- `crates/overdrive-dataplane/tests/integration/reverse_nat_e2e.rs` —
  the veth e2e Tier-3 tests not to regress.
- `docs/feature/single-node-dataplane-wiring/feature-delta.md` — full
  component decomposition, Reuse Analysis, G-4 design.
- `docs/feature/single-node-dataplane-wiring/design/{wave-decisions,c4-diagrams}.md`.
</content>
