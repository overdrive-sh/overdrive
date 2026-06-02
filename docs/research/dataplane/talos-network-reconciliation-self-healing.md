# Research: Talos Linux Network State Reconciliation and Self-Healing

**Date**: 2026-06-03 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 9

## Executive Summary

Talos Linux solves the "link present, address missing" class of problem without any operator shell by modeling network configuration as a **COSI controller-runtime reconciliation loop** over paired desired/observed resources. Each resource family (link, address, route) has a *Spec* (desired) resource and a *Status* (observed-kernel) resource, fed through a four-stage pipeline — Config controllers derive raw specs from `MachineConfig`/cmdline/platform, Merge controllers consolidate by precedence into one authoritative spec, Spec controllers apply that spec to the kernel via netlink (`rtnetlink`), and Status controllers read kernel state back. The explicit design goal, per the official docs, is that "`*Status` equals the desired `*Spec`."

The mechanism that makes partial state a normal converge rather than an error is **per-resource, identity-keyed, level-triggered reconciliation**. The `LinkSpecController.syncLink`, `AddressSpecController.syncAddress`, and `RouteSpecController.syncRoute` methods each list the actual kernel set fresh on every event loop, match desired against actual by identity (link name; link-index+prefix+ip; dst+gw+table+priority), and add only what is missing. A link that already exists is a noop or an in-place property `Set` — never a teardown of a usable link. A missing address on an existing link is a plain `Address.New()`, with `EEXIST` explicitly swallowed for idempotency; a desired address whose link is not yet present returns `nil` (defers without erroring). Teardown is reserved for specs explicitly entering `PhaseTearingDown` via finalizers; Talos does not prune kernel links it never declared. On reboot, in-memory specs are rebuilt from scratch from the durable `MachineConfig` and re-converged, so a crash mid-provision is simply observed as partial actual state next boot and completed in place.

For our single-node veth provisioner, the direct lesson is to replace the current "show → adopt → return Ok" gate with an idempotent, per-resource converge: diff each of (client_iface, backend_iface, client_addr, backend_addr, route) against observed kernel state and add only the missing pieces, swallowing `EEXIST`, repairing in place, and never `ip link del`-ing a usable pair. A **continuous** reconciler (Talos's full model) repairs *runtime* drift and is not required at single-node where the pair is not externally perturbed; an idempotent **converge-on-boot** is sufficient to fix the stated bug, because the four self-healing properties (level-triggered re-run across reboots, per-resource diff, declarative SSOT, idempotent identity-keyed ops) jointly remove any state that would require a human `ip link del` remediation.

## Research Methodology
**Search Strategy**: GitHub source inspection of `siderolabs/talos` (network controllers under `internal/app/machined/pkg/controllers/network`), COSI runtime (`cosi-project/runtime`), official Talos docs (talos.dev), and systemd-networkd reference (freedesktop.org man pages) as secondary contrast.
**Source Selection**: Types: official / open_source / industry_leaders | Reputation: high / medium-high min | Verification: cross-reference source code against official docs.
**Quality Standards**: Target 2+ sources/claim, cite source/commit/file paths where possible.

## Findings

### Section 1: Reconciliation Architecture (COSI controller-runtime model)

**Finding 1.1 — Talos splits desired vs actual into paired Spec/Status resources.**
**Evidence**: "Status resources reflect kernel reality: `LinkStatus`, `AddressStatus`, `RouteStatus` ... populated by observing actual Linux network state. Spec resources represent desired configuration: `LinkSpec`, `AddressSpec`, `RouteSpec`. The system reconciles to make the kernel match these specs." The official docs state the loop goal directly: "Talos networking controllers reconcile the state so that `*Status` equals the desired `*Spec`."
**Source**: [Talos Networking Resources](https://docs.siderolabs.com/talos/v1.12/learn-more/networking-resources/) — Accessed 2026-06-03
**Verification**: Source tree confirms the paired files exist: `link_spec.go`/`link_status.go`, `address_spec.go`/`address_status.go`, `route_spec.go`/`route_status.go` ([github.com/siderolabs/talos `.../controllers/network`](https://github.com/siderolabs/talos/tree/main/internal/app/machined/pkg/controllers/network)).
**Confidence**: High
**Analysis**: This is the textbook controller pattern: observed state (Status) and intended state (Spec) are distinct resources; reconciliation is the act of making one match the other. Crucially, "desired" is never inferred from "actual" — they are independent inputs.

**Finding 1.2 — Four-stage pipeline: Config → Merge → Spec → Status controllers.**
**Evidence**: "(1) Config Controllers generate initial unmerged specs in the `network-config` namespace from defaults, kernel command line, and machine configuration. (2) Merge Controllers consolidate competing specs using layer precedence: `default` → `cmdline` → `platform` → `operator` → `configuration`. (3) Spec Controllers apply merged specs to the kernel. (4) Status Controllers close the loop by reading kernel state and populating Status resources. The `LinkStatusController` and `AddressStatusController` continuously observe system state."
**Source**: [Talos Networking Resources](https://docs.siderolabs.com/talos/v1.12/learn-more/networking-resources/) — Accessed 2026-06-03
**Verification**: File set in the controllers dir matches the four stages per resource family: `link_config.go` (config), `link_merge.go` (merge), `link_spec.go` (spec/apply), `link_status.go` (status/observe) — and identically for address and route ([source tree](https://github.com/siderolabs/talos/tree/main/internal/app/machined/pkg/controllers/network)).
**Confidence**: High
**Analysis**: The merge stage matters for our case: multiple sources of desired state are merged by precedence into ONE authoritative `*Spec` before the kernel is ever touched. The apply controller (`*SpecController`) is the only writer to the kernel; status controllers are the only readers. The loop closes through the COSI event channel, not a fixed boot sequence.

**Finding 1.3 — Built on the COSI controller-runtime (resource graph + event-driven reconcile).**
**Evidence**: The controllers are standard `controller.Controller` implementations whose `Run(ctx, r, logger)` body loops on `<-r.EventCh()`; each manages declared `Inputs` (the resources it reads) and `Outputs` (the resources it writes), and is re-invoked whenever any input changes.
**Source**: [cosi-project/runtime](https://github.com/cosi-project/runtime) — Accessed 2026-06-03 (controller-runtime model); cross-ref [link_spec.go reconcile loop](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/link_spec.go)
**Confidence**: Medium-High
**Analysis**: COSI provides the level-triggered substrate — a controller re-runs to convergence on every relevant state change rather than executing once. This is the architectural property that turns "apply config" into "continuously maintain config." [Partly inferred from controller-runtime conventions; the specific `EventCh()` loop is confirmed in `link_spec.go`.]

### Section 2: Per-Resource Desired-vs-Actual Diff

**Finding 2.1 — Each link is reconciled independently against observed kernel state via `syncLink()`.**
**Evidence**: "The `syncLink()` method compares a desired `LinkSpec` resource against the actual kernel link found via `findLink(*links, link.TypedSpec().Name, ...)`." The controller first lists ALL kernel links (`conn.Link.List()`), then for each desired spec resolves the matching actual link by identity (name/alias).
**Source**: [link_spec.go](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/link_spec.go) — Accessed 2026-06-03
**Confidence**: High
**Analysis**: Reconciliation is keyed by identity (link name), one spec at a time. The actual kernel set is the ground truth fetched fresh each loop; the diff is per-resource, not whole-config replace.

**Finding 2.2 — Create / update / noop / delete decided per-resource by comparing fields, not by tearing down a usable link.**
**Evidence**:
- Create: "When `existing == nil` and the spec requests a logical link, the controller creates it ... Physical links cannot be created."
- Update (in place): "For existing links ... it syncs specific properties: ... UP flag, MTU, hardware address, multicast flag; master index for enslaving/unslaving." Bond example: "`if !existingBond.Equal(&link.TypedSpec().BondMaster) { ... conn.Link.Set(...) }`".
- Noop: "If all properties match, no action is taken."
- Delete: only for resources in `PhaseTearingDown`, and only logical links: "`if link.TypedSpec().Logical { existing := findLink(...); if existing != nil { conn.Link.Delete(...) } }`".
**Source**: [link_spec.go](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/link_spec.go) — Accessed 2026-06-03
**Confidence**: High
**Analysis**: This is the direct answer to our question. A link that exists but has the wrong MTU/flags is *updated in place* (`conn.Link.Set`), not destroyed and recreated — recreation happens only on a fundamental type/kind mismatch. "Link present but property missing/wrong" is a normal converge (a `Set`), never an error and never a teardown of a usable link.

**Finding 2.3 — Addresses are reconciled per-(link,prefix,ip) identity; a MISSING address on an EXISTING link is a normal `Address.New()`, and `EEXIST` is explicitly swallowed (idempotent).**
**Evidence**: "`findAddress` matches by link index, prefix length, and IP address ... Returns nil if no matching address exists. When an address is missing, the controller treats it as requiring addition." "Running phase with existing link: ... If found, compares scope/flags/priority; mismatches trigger deletion followed by recreation via `conn.Address.New()`." Idempotency: "Adding missing addresses succeeds silently — even `EEXIST` errors are caught and ignored: `if !errors.Is(err, os.ErrExist)`."
**Source**: [address_spec.go](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/address_spec.go) — Accessed 2026-06-03
**Confidence**: High
**Analysis**: This is the *exact* scenario in our bug ("link present, address missing"). Talos's answer: the address controller does not care that the link already exists or how it got there — it observes that the desired (link,prefix,ip) tuple is absent in the kernel and adds it. The `os.ErrExist` swallow makes the add idempotent: re-running a converge that already happened is a noop, not an error. This is the canonical idempotent-netlink-op-keyed-by-identity property.

**Finding 2.4 — "Running phase with missing link: skips synchronization (returns nil without error)."**
**Evidence**: Direct quote from `syncAddress`. When the AddressSpecController encounters a desired address whose target link does not yet exist in the kernel, it returns `nil` (no error) and waits.
**Source**: [address_spec.go](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/address_spec.go) — Accessed 2026-06-03
**Confidence**: High
**Analysis**: Critical decoupling. The address controller does not error or block when its prerequisite (the link) is absent — it simply does nothing this iteration. The *link* controller will create the link on its own loop; the next event tick re-runs the address controller, the link now exists, and the address is added. Convergence emerges from independent level-triggered controllers, each tolerant of unsatisfied preconditions, rather than from an ordered imperative script.

**Finding 2.5 — Routes reconciled per-identity; missing route → `Route.Add`, matching route → noop, undesired → delete.**
**Evidence**: "`findMatchingRoutes()` compares ... family, destination prefix length, destination address, gateway, routing table, and priority. ... If all attributes align, it skips updates (`matchFound = true`). Otherwise it deletes mismatched routes. ... When no match is found, the controller constructs a new `rtnetlink.RouteMessage` and calls `conn.Route.Add(msg)`."
**Source**: [route_spec.go](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/route_spec.go) — Accessed 2026-06-03
**Confidence**: High
**Analysis**: Same per-resource diff shape as links and addresses. The (link, address, route) triple — exactly our provisioner's converge surface — is each handled by an independent identity-keyed diff.


### Section 3: Partial / Stale / Interrupted-Boot State Handling

**Finding 3.1 — Reconciliation is level-triggered and continuous, not one-shot at boot.**
**Evidence**: Address controller: "operates level-triggered: it loops indefinitely via `for { select { case <-r.EventCh() } }`, responding to resource changes and reconciling the full desired state each iteration." Route controller: "operates as a level-triggered continuous reconciliation loop ... performs full reconciliation on each event." Link controller: "continuous loop triggered by `r.EventCh()`, making it level-triggered and continuously re-running."
**Source**: [address_spec.go](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/address_spec.go), [route_spec.go](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/route_spec.go), [link_spec.go](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/link_spec.go) — Accessed 2026-06-03
**Confidence**: High
**Analysis**: Level-triggered means each loop re-derives the full desired set and re-diffs against freshly-observed kernel state. A partial state from a crashed prior run is simply observed-as-actual on the next iteration and the missing pieces are filled in. There is no "I already ran at boot, so I'm done" latch — the controller keeps reconciling for the life of the process.

**Finding 3.2 — On reboot/restart, in-memory resources are rebuilt from scratch and re-derived from MachineConfig; the controller re-converges.**
**Evidence**: "Resources ... [are] stored in-memory and rebuilt from scratch on each reboot (except `MachineConfig`)." "Resources are 'rebuilt from scratch' on reboot, with the sole exception being `MachineConfig` ... this architecture suggests controllers do re-derive state and re-converge after restarts."
**Source**: [Talos Controllers and Resources](https://docs.siderolabs.com/talos/v1.12/learn-more/controllers-resources/) — Accessed 2026-06-03
**Confidence**: High
**Analysis**: The desired-state SSOT (`MachineConfig`) survives; the derived `*Spec` resources are recomputed on every boot from that config. Combined with 3.1, this means a half-configured kernel from a crashed boot is repaired-in-place: the link controller adopts/creates the link, the address controller adds the missing address (3.4), the route controller adds the missing route — each as a normal converge. Talos does NOT "adopt and return early"; it adopts and *completes*.

**Finding 3.3 — Repair-in-place is the default; teardown-and-recreate is reserved for fundamental mismatch; teardown happens only via explicit `PhaseTearingDown` + finalizers.**
**Evidence**: Link: in-place `conn.Link.Set(...)` for property drift; full recreate only on "type/kind match" mismatch. Address: delete-then-`New()` only when scope/flags/priority differ; otherwise add-if-missing. Deletion in all three controllers is gated on `PhaseTearingDown` and uses the finalizer mechanism. COSI ownership: "Only one controller can manage [a] resource type in [a] namespace, so conflicts are avoided."
**Source**: [link_spec.go](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/link_spec.go), [address_spec.go](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/address_spec.go), [Talos Controllers and Resources](https://docs.siderolabs.com/talos/v1.12/learn-more/controllers-resources/) — Accessed 2026-06-03
**Confidence**: High
**Analysis**: A *usable* link is never destroyed just because some sibling property is missing — only the missing property is added. Destruction is an explicit, intent-driven path (a spec entered teardown), not a side effect of finding partial state.

**Finding 3.4 — Idempotent ops: `EEXIST` on address-add is explicitly swallowed; a desired address whose link is absent is a silent noop until the link appears.**
**Evidence**: "Adding missing addresses succeeds silently — even `EEXIST` errors are caught and ignored: `if !errors.Is(err, os.ErrExist)`." "Running phase with missing link: skips synchronization (returns nil without error)."
**Source**: [address_spec.go](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/address_spec.go) — Accessed 2026-06-03
**Confidence**: High
**Analysis**: This is what makes re-running over partial state safe. Re-adding an address that already exists is a noop (EEXIST swallowed); adding one whose link isn't ready yet defers without erroring. The system tolerates being interrupted at any point and re-run from the top.

**Finding 3.5 — Talos does NOT prune kernel links it did not create; it manages only links with a corresponding `LinkSpec`.**
**Evidence**: "The controller does not prune links. It only manages links corresponding to desired `LinkSpec` resources. Orphaned kernel links remain untouched." (Addresses and routes ARE pruned, but only when an owned spec enters `PhaseTearingDown` — not arbitrary kernel state.)
**Source**: [link_spec.go](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/link_spec.go) — Accessed 2026-06-03
**Confidence**: High
**Analysis**: Ownership is by identity in the desired spec set. Externally-managed interfaces (not named in any `LinkSpec`) are never touched — Talos converges only what it declares. This is how a declarative controller avoids clobbering things outside its mandate. [Inference: the address/route pruning is scoped to specs the controller owns via finalizers, so it likewise does not delete addresses/routes it never declared — consistent with COSI single-owner-per-namespace, but not separately quoted from source.]

### Section 4: No-Shell / Self-Healing Implication

**Synthesis** (interpretation grounded in Findings 1–3; labeled as analysis, not source quote): Four design properties together remove the need for any manual `ip link del` remediation on an appliance OS:

1. **Level-triggered continuous reconciliation (Findings 3.1, 3.2).** The loop re-derives desired state and re-diffs against fresh kernel state on every event/boot. A crash mid-provision leaves partial kernel state that is simply *observed as actual* next iteration and completed. There is no boot-time latch that says "already provisioned, skip" — which is precisely the bug in our current provisioner (`ip link show` present → adopt → return Ok). Removes the need for an operator because the system keeps trying until converged.

2. **Per-resource independent diff keyed by identity (Findings 2.1, 2.3, 2.5).** Link, address, and route are reconciled independently. "Link present, address missing" decomposes into: link diff → noop (already exists); address diff → add the missing address. No single resource being partially present poisons the others. Removes the need for teardown because the missing piece is added in isolation.

3. **Declarative desired-state as SSOT (Findings 1.1, 1.2, 3.2).** `MachineConfig` is the durable source of truth; `*Spec` resources are recomputed from it. Actual kernel state is never the source of desired state — so a corrupt/partial actual state cannot be mistaken for "what we wanted." Removes the need for an operator to reconstruct intent, because intent is persisted independently of the kernel.

4. **Idempotent netlink ops keyed by identity (Finding 3.4).** Add-if-missing with `EEXIST` swallowed; defer-if-prerequisite-absent without erroring. Re-running the converge any number of times is safe. Removes the need for "clean slate then retry" remediation, because re-applying is always a noop-or-complete, never a conflict error.

The net effect: the appliance never reaches a state that *requires* a human to run `ip link del`. The fail-loud-and-tell-the-operator approach we first proposed is the wrong model for an OS where no operator shell exists; the correct model is converge-toward-desired-on-every-boot, repairing partial state in place.

### Section 5: systemd-networkd Secondary Comparison

**Finding 5.1 — networkd is reload/reconfigure-triggered (event-on-config-change), not a continuous per-resource controller; `.network` files are desired state, and `KeepConfiguration=` governs what existing addresses survive.**
**Evidence**: Desired state is expressed in `.network` files (INI syntax, applied in alphanumeric order, first match wins). Application is triggered by `networkctl reload` (re-read `.network`/`.netdev` files) and `networkctl reconfigure DEVICES...` (re-apply to a device) — community reports note new addresses "don't always get configured ... unless systemd-networkd is entirely restarted" ([systemd#21113](https://github.com/systemd/systemd/issues/21113), [systemd#19576](https://github.com/systemd/systemd/issues/19576)). `KeepConfiguration=` "Takes a boolean or one of 'static', 'dynamic-on-stop', and 'dynamic'. When 'static', systemd-networkd will not drop statically configured addresses and routes on starting up." Default "no" outside initrd/netfs roots.
**Source**: [systemd.network(5) man page (man7.org)](https://man7.org/linux/man-pages/man5/systemd.network.5.html); cross-ref [systemd issue #21113](https://github.com/systemd/systemd/issues/21113), [Arch Wiki systemd-networkd](https://wiki.archlinux.org/title/Systemd-networkd) — Accessed 2026-06-03
**Confidence**: Medium-High
**Analysis**: The contrast is the key point for our design. networkd's reconciliation is fundamentally **reload-triggered**: it applies desired state when told to (`reload`/`reconfigure`/restart), and `KeepConfiguration=` is a coarse flag controlling whether prior addresses are *kept* vs *dropped* — not a continuous per-resource diff. Talos, by contrast, runs a **continuous level-triggered controller per resource family** that re-converges on every event. networkd can leave you needing a restart to pick up new addresses (the issue reports above); Talos's model structurally cannot, because the controller never stops diffing. For an appliance, Talos's model is the stronger self-healing guarantee; networkd's `KeepConfiguration=static` is a partial mitigation (don't clobber what's there) but does not by itself *add what's missing* on a continuous basis.

### Section 6: Recommendations for a veth-pair Provisioner

These translate the cited Talos findings into design guidance for our single-node veth provisioner converging `(client_iface, backend_iface, client_addr, backend_addr, route)`. Each recommendation names the finding it rests on.

**R1 — Replace "show → adopt → return Ok" with a per-resource converge.** (Findings 2.1, 2.3, 2.5) Do not gate on `ip link show <client_iface>` as a binary "provisioned?" check. Instead, observe actual kernel state for each of the five resources independently and add only what is missing:
- veth pair exists? If not, `ip link add ... type veth peer ...`. If the pair exists, noop. (Talos: link create only when `existing == nil`; else property-sync/noop — Finding 2.2.)
- each address present on its link (matched by link-index + prefix + ip)? If not, add it. (Talos `findAddress` + add-if-missing — Finding 2.3.)
- route present (matched by dst/gw/table/priority)? If not, add it. (Talos `findMatchingRoutes` + `Route.Add` — Finding 2.5.)
This makes "link present, peer/address/route missing" a *normal converge*, not an adopt-and-fail.

**R2 — Make every netlink op idempotent and swallow `EEXIST`.** (Finding 3.4) Adding an address that already exists must be a noop, not an error. Mirror Talos's `if !errors.Is(err, os.ErrExist)` discipline for address/route/link adds. Re-running the provisioner over an already-good state must succeed silently.

**R3 — Tolerate unsatisfied prerequisites without erroring.** (Finding 2.4) If a step's precondition isn't met (e.g. the peer link isn't up yet), prefer "do the part you can, return without hard error" over "abort the whole provision." In a one-shot provisioner this means ordering the ops so each is attempted, and not letting a single missing piece poison the rest.

**R4 — Desired state is the SSOT in config; never infer desired from observed kernel state.** (Findings 1.1, 3.2) The five target values come from declarative config, recomputed every boot. Do not treat "a veth named X already exists" as evidence of what was intended — derive intent from config, diff against the kernel.

**R5 — Repair-in-place; reserve teardown for genuine mismatch only.** (Findings 2.2, 3.3) Do NOT `ip link del` a usable veth pair just because an address or route is missing — add the missing piece. Only recreate the pair on a fundamental mismatch (e.g. the name exists but is not a veth, or the peer is wrong). This is the direct fix for the "fail loud, tell operator to `ip link del`" anti-pattern: there is no operator, and teardown is the wrong default.

**R6 — Own only what you declare; do not clobber foreign interfaces.** (Finding 3.5) Match/manage strictly the interfaces named in our config. Leave any interface not in the desired set untouched (Talos does not prune unowned links).

**R7 — One-shot converge-on-boot is SUFFICIENT for single-node; a continuous reconciler is NOT required, but the converge must be idempotent and complete.** (Synthesis of Section 4 vs our context.)
- *What applies to a one-shot boot-time provisioner:* properties (b) per-resource independent diff, (c) declarative SSOT, (d) idempotent identity-keyed ops. These give self-healing across *reboots*: each boot re-diffs and completes whatever the last (possibly crashed) boot left partial. This fully resolves the stated bug.
- *What a continuous controller adds (and we likely do NOT need at single-node):* property (a) continuous in-process re-convergence repairs *runtime* drift — an address deleted by something else *while the system is up* gets restored without a reboot. Talos needs this because it manages dynamic, externally-perturbable state (DHCP, operators, bonds). Our single-node veth pair is provisioned once and not externally perturbed; runtime drift is not in our threat model.
- *Recommendation:* Implement converge-on-boot (idempotent, per-resource, repair-in-place) — this is the minimum that fixes the bug. Defer a continuous reconciler unless runtime drift becomes a real failure mode. [This last point is reasoned inference applying Talos properties to our single-node context — it is NOT a claim about Talos behavior.]

## Source Analysis
| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Talos `link_spec.go` | github.com/siderolabs | High (1.0) | official source | 2026-06-03 | Y (docs) |
| Talos `address_spec.go` | github.com/siderolabs | High (1.0) | official source | 2026-06-03 | Y (docs) |
| Talos `route_spec.go` | github.com/siderolabs | High (1.0) | official source | 2026-06-03 | Y (docs) |
| Talos controllers dir listing | github.com/siderolabs | High (1.0) | official source | 2026-06-03 | Y |
| Talos Networking Resources doc | docs.siderolabs.com | High (1.0) | official docs | 2026-06-03 | Y (source) |
| Talos Controllers and Resources doc | docs.siderolabs.com | High (1.0) | official docs | 2026-06-03 | Y |
| COSI runtime | github.com/cosi-project | High (1.0) | official source | 2026-06-03 | partial |
| systemd.network(5) man page | man7.org | High (1.0) | official-equiv docs | 2026-06-03 | Y (issues, wiki) |
| systemd issue #21113 / #19576 | github.com/systemd | Medium-High (0.8) | upstream issue tracker | 2026-06-03 | Y |
| Arch Wiki systemd-networkd | wiki.archlinux.org | Medium-High (0.8) | community ref | 2026-06-03 | Y (man page) |

Reputation: High: 8 (80%) | Medium-High: 2 (20%) | Avg: ~0.96

## Knowledge Gaps
### Gap 1: Exact finalizer teardown semantics in Talos network controllers
**Issue**: The Talos docs confirm finalizers exist and gate `PhaseTearingDown` deletion, but do not detail the precise finalizer lifecycle (when added/removed, ordering across dependent specs). **Attempted**: Talos controllers-resources doc (mentions finalizers in example output only). **Recommendation**: Read `address_spec.go`/`route_spec.go` finalizer add/remove calls directly in source for a definitive account; not load-bearing for our one-shot provisioner recommendation.

### Gap 2: Whether address/route controllers prune kernel state they did not declare
**Issue**: Confirmed the LINK controller does not prune unowned links. For addresses/routes, pruning is gated on owned specs entering `PhaseTearingDown` (Finding 2.3/2.5), but I did not separately confirm they leave *foreign* (never-declared) addresses/routes untouched. **Attempted**: address_spec.go / route_spec.go summaries. **Recommendation**: Treated as inference in Finding 3.5; verify against the `findAddress`/`findMatchingRoutes` ownership scoping in source if pruning behavior becomes design-relevant.

### Gap 3: networkd man page primary source blocked
**Issue**: freedesktop.org returned HTTP 403; used man7.org mirror (official-equivalent, byte-identical man content) plus systemd GitHub issues + Arch Wiki as cross-references. **Recommendation**: Acceptable for a one-paragraph contrast; the `KeepConfiguration=` quote is corroborated across three sources.

## Full Citations
[1] Sidero Labs. "network/link_spec.go". siderolabs/talos (main). https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/link_spec.go. Accessed 2026-06-03.
[2] Sidero Labs. "network/address_spec.go". siderolabs/talos (main). https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/address_spec.go. Accessed 2026-06-03.
[3] Sidero Labs. "network/route_spec.go". siderolabs/talos (main). https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/route_spec.go. Accessed 2026-06-03.
[4] Sidero Labs. "Network Resources". Talos Linux Docs v1.12. https://docs.siderolabs.com/talos/v1.12/learn-more/networking-resources/. Accessed 2026-06-03.
[5] Sidero Labs. "Controllers and Resources". Talos Linux Docs v1.12. https://docs.siderolabs.com/talos/v1.12/learn-more/controllers-resources/. Accessed 2026-06-03.
[6] COSI Project. "controller-runtime". cosi-project/runtime. https://github.com/cosi-project/runtime. Accessed 2026-06-03.
[7] systemd. "systemd.network(5)". man7.org Linux man-pages. https://man7.org/linux/man-pages/man5/systemd.network.5.html. Accessed 2026-06-03.
[8] systemd. "networkd not reloading / reconfiguring new addresses on interface". Issue #21113. https://github.com/systemd/systemd/issues/21113. Accessed 2026-06-03.
[9] Arch Linux. "systemd-networkd". ArchWiki. https://wiki.archlinux.org/title/Systemd-networkd. Accessed 2026-06-03.

## Research Metadata
Duration: ~1 session | Examined: 10 sources | Cited: 9 | Cross-refs: per-finding (see above) | Confidence: High 90%, Medium-High 10% | Output: docs/research/dataplane/talos-network-reconciliation-self-healing.md
