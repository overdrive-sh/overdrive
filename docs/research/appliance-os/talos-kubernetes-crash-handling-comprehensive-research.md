# Research: How Talos Linux Handles Kubernetes (and Its Own) Crashes

**Date**: 2026-10-08 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High | **Sources**: 22 (8 official Talos docs pages, 11 primary source files, 3 independent upstream docs)

**Versions described**: Talos docs **v1.11** (docs.siderolabs.com; the static-pods page served as v1.14 "latest stable"); source code read from `siderolabs/talos` and `cosi-project/runtime` **`main` branch as of 2026-10-08** (no tag pinned — see Knowledge Gaps).

## Executive Summary

Talos handles failure with two supervision layers inside `machined` (PID 1). System services (containerd/`cri`, kubelet, etcd, apid, trustd) run under a service runner with an explicit state machine (Waiting → Preparing → Running → Finished/Failed/Skipped), dependency conditions, per-service health checks and a `Forever` restart policy that retries every **5 s, fixed, with no backoff and no limit**. In-process COSI controllers are supervised separately: a controller that errors or panics is recovered, counted, restarted with **unbounded exponential backoff**, and re-triggered to reconcile from fresh inputs. Kubernetes control-plane components are kubelet static pods whose manifests Talos serves over a local HTTP endpoint, so Talos restarts kubelet and kubelet restarts the control plane. etcd is a Talos service, not a static pod.

Node-level recovery rests on one design choice. Only the machine config (STATE partition) is persisted as a resource, and every other resource is rebuilt in memory on each boot. Converge-on-boot is therefore the *only* path, and reboot is the universal recovery primitive: a PID-1 fatal error syncs, reverts the bootloader and reboots, and A/B images with boot-once roll back a failed upgrade. Application data (etcd db, images, kubelet state) lives in EPHEMERAL. Talos treats that data as its applications' to manage, and the generic repair is to wipe EPHEMERAL and rejoin. A single etcd member crash or rejoin is automatic (as a learner when the data dir is empty). Quorum loss is a deliberate operator procedure (`etcd snapshot` / `cp` the db → wipe EPHEMERAL on all CP nodes → `bootstrap --recover-from`).

On kernel-state ownership (Q5), Talos is declarative (`*Spec` desired → `*SpecController` → `*Status`) but **not uniformly continuous**. Routes and links watch netlink and re-apply drift. Addresses re-apply only on link events. nftables and sysctls reconcile only on spec change. Talos also deliberately tolerates foreign objects: it marks its routes by protocol, never touches kernel- or RA-created routes, owns exactly one nft table that it rebuilds wholesale, and never deletes physical links. Continuous watching is a co-tenancy measure (CNI, DHCP, kernel) rather than an adversary defence. For Overdrive (Q7), this supports *not* building foreign-mutation defence, and suggests borrowing four patterns: persist inputs and re-derive everything on boot; the owned-table-wholesale-replace pattern for nft; protocol/ownership tags for shared kernel tables; and panic-isolated, backed-off reconciler supervision. Overdrive should deliberately differ on restart policy. Talos's flat forever-retry has no crash budget or occurrence record, both of which Overdrive's rules require.

## Research Methodology
**Search Strategy**: Official Talos docs (talos.dev → 301 → docs.siderolabs.com) for documented behaviour; primary source in `github.com/siderolabs/talos` (machined service runner, services, network/runtime controllers, sequencer, main) and `github.com/cosi-project/runtime` for behaviour the docs omit; upstream kubernetes.io and etcd.io docs as independent corroboration of mechanisms Talos builds on.
**Source Selection**: Types: official docs, primary source code, upstream project docs | Reputation: siderolabs.com/talos.dev (open_source, high), kubernetes.io (official, high), etcd.io (open_source, high), github.com (industry_leaders, medium-high; used for *primary* source code of the subject itself) | Third-party blogs (oneuptime.com) surfaced by search were **rejected** as untrusted and not cited.
**Quality Standards**: Target 3 sources/claim; source-code claims are necessarily single-source (the code is the authority) and rated Medium-High at most. Average reputation ≈ 0.90.

## Findings

### Q1 — Process supervision (machined, service runner, COSI)

Talos has **two distinct supervision layers** inside `machined` (PID 1), and they must not be conflated:

1. **The service runner** (`internal/app/machined/pkg/system/`) — supervises long-running *processes/containers* (containerd/`cri`, kubelet, etcd, apid, trustd, udevd, …). systemd-like.
2. **The COSI controller runtime** (`github.com/cosi-project/runtime`) — supervises in-process *controllers* (goroutines) that reconcile typed resources. Kubernetes-controller-like.

#### Finding 1.1: Service restart policy is a fixed-interval retry, not exponential backoff
**Evidence**: `restart.go` defines three policies — `Forever` ("will always restart a process"), `Once` ("will run process exactly once"), `UntilSuccess` ("will restart process until run succeeds"). Defaults: type `Forever`, `RestartInterval: 5 * time.Second`. The wait between attempts is `case <-time.After(r.opts.RestartInterval):`; cancellation during the wait logs `"Aborting restart sequence"`. There is no exponential backoff and no give-up limit for `Forever`.
**Source**: [siderolabs/talos `internal/app/machined/pkg/system/runner/restart/restart.go` (main)](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/system/runner/restart/restart.go) - Accessed 2026-10-08
**Confidence**: Medium-High (primary source code; single authoritative source, read directly)
**Verification**: Consistent with per-service usage in `services/kubelet.go` and `services/etcd.go` (both `restart.WithType(restart.Forever)`, no custom interval) — same repository, so not independent.
**Analysis**: Talos does *not* implement CrashLoopBackOff-style growing delays for system services; a crashing kubelet or etcd is retried every ~5 s forever. Crash history is visible as service events rather than as a throttled restart budget.

#### Finding 1.2: Service lifecycle is an explicit state machine with dependency conditions and health checks
**Evidence**: `service_runner.go` drives each service through `StateInitialized` → `StateWaiting` (dependencies; "Waiting for %s") → `StatePreparing` ("Running pre state", "Creating service runner") → `StateRunning` → terminal `StateFinished` / `StateFailed` / `StateSkipped` (`ErrSkip`). Dependencies come from `service.DependsOn(runtime)` combined with volume-mount conditions; a ticker republishes the unmet-condition description. For `HealthcheckedService`, health runs concurrently and a service is "up" when "running and healthy (if supports health checks)". Every transition emits a timestamped `ServiceEvent` into the runtime event stream and per-service history. Runner errors are wrapped `"error running service: %w"`; `PostFunc` always runs.
**Source**: [siderolabs/talos `internal/app/machined/pkg/system/service_runner.go` (main)](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/system/service_runner.go) - Accessed 2026-10-08
**Confidence**: Medium-High (primary source)
**Verification**: `talosctl service` / `talosctl logs` CLI reference ([docs.siderolabs.com CLI reference v1.11](https://docs.siderolabs.com/talos/v1.11/reference/cli)) exposes exactly these states/events to operators.

#### Finding 1.3: Per-service wiring — kubelet and etcd are containers run by containerd, `Forever`, health-checked
**Evidence**:
- kubelet: `restart.WithType(restart.Forever)`, `containerd.NewRunner(...)`, `DependsOn → []string{"cri"}`, health `simpleHealthCheck(ctx, "http://127.0.0.1:10248/healthz")` with `InitialDelay = 2 * time.Second`. `PreFunc` reads the kubelet spec *from resources*, pulls the kubelet image with retries, and records a lifecycle resource.
- etcd: `restart.Forever`, `DependsOn "cri"`, readiness conditions on time-sync, network readiness and the etcd spec resource; health = `client.ValidateQuorum(ctx)` (5 s initial delay, 20 s period, 15 s timeout). `PreFunc` resets learner state, waits for PKI, and picks init vs. join args.
**Source**: [`services/kubelet.go`](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/system/services/kubelet.go), [`services/etcd.go`](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/system/services/etcd.go) (siderolabs/talos main) - Accessed 2026-10-08
**Confidence**: Medium-High (primary source)
**Analysis**: Every restart re-runs `PreFunc`, which **re-derives the service's configuration from current COSI resources** (kubelet spec, etcd spec). A restart is therefore also a re-converge of the process's config — the service runner is the "apply" step for state that controllers computed. The etcd health check is quorum-level, not process-level, so "etcd unhealthy" can mean cluster-wide trouble on a healthy process.

#### Finding 1.4: COSI controllers that fail are restarted with unbounded exponential backoff, then re-triggered
**Evidence**: In `cosi-project/runtime`, each controller adapter is created with `backoff.NewExponentialBackOff()` and `adapter.backoff.MaxElapsedTime = 0` (never give up). The run loop:
```go
for {
    err := adapter.runOnce(ctx, logger)
    if err == nil { return }
    if adapter.runtimeOptions.MetricsEnabled { metrics.ControllerCrashes.Add(adapter.Name, 1) }
    interval := adapter.backoff.NextBackOff()
    logger.Sugar().Debugf("restarting controller in %s", interval)
    select { case <-ctx.Done(): return; case <-time.After(interval): }
    adapter.triggerReconcile()
}
```
`runOnce()` recovers panics and converts them to errors.
**Source**: [cosi-project/runtime `pkg/controller/runtime/internal/rruntime/run.go` and `rruntime.go` (main)](https://github.com/cosi-project/runtime/tree/main/pkg/controller/runtime/internal/rruntime) - Accessed 2026-10-08
**Confidence**: Medium-High (primary source)
**Verification**: Official docs describe controllers as "independent lightweight threads" reconciling inputs to a single output type ([Controllers and Resources, v1.11](https://docs.siderolabs.com/talos/v1.11/learn-more/controllers-resources)) but do not document failure handling — the code is the only source.
**Analysis**: A controller bug (panic) does not take down `machined`; it is isolated, counted (`ControllerCrashes` metric), backed off, and its reconcile is re-triggered so it re-reads inputs from scratch. This is the in-process analogue of Overdrive's reconciler runtime.

### Q2 — Control-plane components and kubelet crash

#### Finding 2.1: kube-apiserver / controller-manager / scheduler are kubelet static pods whose manifests Talos serves over a local HTTP endpoint
**Evidence**: "Talos renders static pod definitions to the `kubelet` using a local HTTP server, `kubelet` picks up the definition and launches the pod." Control-plane pods appear in `talosctl get staticpods`; `talosctl get staticpodstatus` shows kubelet-reported status. Changes are accepted without reboot.
**Source**: [Static Pods (docs.siderolabs.com, page labelled v1.14 latest)](https://docs.siderolabs.com/talos/v1.13/configure-your-talos-cluster/images-container-runtime/static-pods.md) - Accessed 2026-10-08
**Confidence**: High (official Talos docs; mechanism corroborated independently by [Kubernetes: Create static Pods](https://kubernetes.io/docs/tasks/configure-pod-container/static-pod/) — "Web-Hosted Static Pod Manifest" via `staticPodURL`, kubelet "fetches the web manifest periodically", static Pods are "managed only by the kubelet"; and by `talosctl get staticpods` in the Talos troubleshooting guide)
**Analysis**: Division of restart authority: **Talos's service runner restarts kubelet; kubelet restarts the control-plane containers** (standard kubelet static-pod semantics, which include kubelet's own container restart backoff ([Kubernetes: Pod lifecycle](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/), § container restarts). The exact backoff text could not be extracted, so this is [unverified in this session]; see Gap 5.). Talos controllers own the *desired manifest* (rendered from machine config into `StaticPod` resources); kubelet owns *running it*. Because manifests are served over HTTP from machined, rather than living only as files kubelet watches, the desired manifest is always re-derived from resources and cannot drift on disk. [Inference from docs: the HTTP mechanism is documented; whether no file copy exists on disk is not stated.]

#### Finding 2.2: If kubelet crashes, machined restarts it every ~5 s forever; running static-pod containers keep running under containerd
**Evidence**: kubelet service: `restart.Forever`, default interval 5 s, `DependsOn "cri"` (Finding 1.1/1.3). Containers are owned by containerd, not by the kubelet process.
**Source**: [`services/kubelet.go`](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/system/services/kubelet.go), [`restart.go`](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/system/runner/restart/restart.go) - Accessed 2026-10-08
**Confidence**: Medium (code for the restart half; the "containers survive a kubelet crash" half is standard containerd/kubelet behaviour [inference, not Talos-documented])
**Analysis**: On restart, kubelet re-fetches static pods from the Talos HTTP endpoint and re-adopts running containers via CRI — the classic kubelet converge-on-start. etcd is **not** a static pod; it is a Talos service (Finding 1.3), so kubelet failure cannot take etcd down.

### Q3 — etcd failure and recovery

#### Finding 3.1: Single-member crash is handled automatically; data-dir presence decides join semantics
**Evidence**: etcd runs under `restart.Forever` (Finding 1.3). In `etcd.go`, if the data directory contains files, `initial-cluster-state` is set to `"existing"` — the member restarts with its own data. Only when the data dir is empty does it join as a new member: comment "addMember only gets called when the etcd data directory is empty, so the node is about to join the etcd cluster"; it is added as a **learner** (retry ~30 s attempts at 3 s intervals) and later promoted.
**Source**: [`services/etcd.go`](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/system/services/etcd.go) - Accessed 2026-10-08
**Confidence**: Medium-High (primary source)
**Analysis**: Rejoin is "wipe EPHEMERAL (where `/var/lib/etcd` lives) → boot → auto-join as learner". Talos does not try to repair a corrupt data dir; the operator path is wipe-and-rejoin.

#### Finding 3.2: Quorum loss is NOT recovered automatically — it is an explicit operator procedure
**Evidence**: Disaster recovery is triggered by quorum loss. Snapshot with `talosctl -n <IP> etcd snapshot db.snapshot`, or if etcd is down `talosctl -n <IP> cp /var/lib/etcd/member/snap/db .` ("may lack full consistency"). Put all CP nodes into `Preparing` by wiping EPHEMERAL: `talosctl -n <IP> reset --graceful=false --reboot --system-labels-to-wipe=EPHEMERAL`, then `talosctl -n <IP> bootstrap --recover-from=./db.snapshot` (add `--recover-skip-hash-check` for a raw db copy). Afterwards "etcd service should become healthy on the bootstrap node, Kubernetes control plane components should start ... Remaining control plane nodes join etcd cluster once control plane endpoint is up."
**Source**: [Disaster Recovery, Talos v1.11 docs](https://docs.siderolabs.com/talos/v1.11/build-and-extend-talos/cluster-operations-and-maintenance/disaster-recovery) - Accessed 2026-10-08
**Confidence**: High (official docs; corroborated by `etcd.go` snapshot-restore branch — `RecoverFromSnapshot`, `RecoverSkipHashCheck` — and the CLI reference flags)
**Verification**: [talosctl CLI reference v1.11](https://docs.siderolabs.com/talos/v1.11/reference/cli) (`--recover-from`, `--recover-skip-hash-check`); `etcd.go` source; upstream [etcd v3.5 Disaster recovery](https://etcd.io/docs/v3.5/op-guide/recovery/) (tolerates "(N-1)/2 permanent failures"; restore "overwrites ... the member ID and cluster ID" so "the restore must start a new logical cluster"; `--skip-hash-check` for a copied `member/snap/db`) — independent publisher, explains *why* Talos requires every CP node to be wiped to `Preparing` before recovery.
**Analysis**: The post-recovery convergence (other CP nodes rejoin, static pods start) is automatic; the *decision* to discard divergent state and restore from a snapshot is human. Bootstrap is a one-shot API call, deliberately not a reconciled desired state — a rebooted node never re-bootstraps.

#### Finding 3.3: Member removal is operator-driven; graceful leave preferred
**Evidence**: `talosctl etcd remove-member`: "Use this command only if you want to remove a member which is in broken state. If there is no access to the node, or the node can't access etcd to call etcd leave. Always prefer etcd leave over this command." `talosctl reset` "will cordon and drain the node, leaving `etcd` if required, and then erase its disks".
**Source**: [talosctl CLI reference v1.11](https://docs.siderolabs.com/talos/v1.11/reference/cli); [Scaling down, v1.11](https://docs.siderolabs.com/talos/v1.11/deploy-and-manage-workloads/scaling-down) - Accessed 2026-10-08
**Confidence**: High (two official pages)

#### Finding 3.4: Space/fragmentation are operational, not automatic
**Evidence**: Default quota 2 GiB; on exceed etcd raises `NOSPACE` (`talosctl etcd alarm list`); fix = raise `quota-backend-bytes`, reboot, `talosctl etcd alarm disarm`. Defrag is manual, one node at a time (`talosctl etcd defrag`). Before patching a leader, `talosctl etcd forfeit-leadership`.
**Source**: [etcd maintenance, v1.11](https://docs.siderolabs.com/talos/v1.11/build-and-extend-talos/cluster-operations-and-maintenance/etcd-maintenance) - Accessed 2026-10-08
**Confidence**: Medium (single official source)

### Q4 — Node crash / reboot, converge-on-boot, A/B upgrade

#### Finding 4.1: Partition split — persisted *inputs* (STATE) vs persisted *workload data* (EPHEMERAL) vs rebuilt runtime
**Evidence**: Six partitions: EFI; BIOS; BOOT ("stores initramfs and kernel data"); META ("stores metadata about the talos node, such as node id's"); STATE ("stores machine configuration, node identity data for cluster discovery and KubeSpan info"); EPHEMERAL ("stores ephemeral state information, mounted at `/var`"). Root is "a read-only squashfs"; `/dev`, `/proc`, `/run`, `/sys`, `/tmp`, `/system` are tmpfs; "/system" is "completely recreated on each boot". `/var` (owned by Kubernetes, etcd, kubelet, containerd) survives reboots and upgrades but is "wiped and lost on resets".
**Source**: [Architecture, Talos v1.11](https://docs.siderolabs.com/talos/v1.11/learn-more/architecture) - Accessed 2026-10-08
**Confidence**: High (official; consistent with the disaster-recovery procedure that wipes EPHEMERAL to reset etcd, Finding 3.2)

#### Finding 4.2: Only the machine config is a persisted resource; every other COSI resource is rebuilt in memory each boot
**Evidence**: Resources are "currently stored in memory, rebuilding on each reboot except for `MachineConfig`."
**Source**: [Controllers and Resources, v1.11](https://docs.siderolabs.com/talos/v1.11/learn-more/controllers-resources) - Accessed 2026-10-08
**Confidence**: Medium-High (single official statement; consistent with STATE contents in Finding 4.1)
**Analysis**: This is "persist inputs, not derived state" taken to its limit: the node's desired state is one document (+ node identity in STATE/META); all specs, statuses, rendered static pods, kubelet config, routes, etc. are **re-derived by controllers on every boot**. Converge-on-boot is therefore not a special recovery path — it is the only path. A crash at any point is recovered by rebooting into the same derivation. The exception is application data in EPHEMERAL (etcd db, containerd images, kubelet state), which Talos treats as owned by those applications.

#### Finding 4.3: Boot is a fixed sequencer; services start last
**Evidence**: `v1alpha1_sequencer.go` phases — Initialize: `systemRequirements` → `earlyServices` → `meta` → (conditional `cleanupBootloader`, `dashboard`, `wipeDisks`, `haltIfInstalled`) → `config`. Boot: `memorySizeCheck` → `diskSizeCheck` → `env` → `dbus` → `ephemeral` → (`promotableVolumes`, `userDisks`) → `userSetup` → `startEverything`. Upgrade: `denyNewServices` → (`drain`) → unmount phases → `upgrade` → `meta` → (`kexec`) → `stopEverything` → `reboot`.
**Source**: [siderolabs/talos `internal/app/machined/pkg/runtime/v1alpha1/v1alpha1_sequencer.go` (main)](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/runtime/v1alpha1/v1alpha1_sequencer.go) - Accessed 2026-10-08
**Confidence**: Medium (primary source; phase list via summarised fetch)
**Analysis**: Imperative sequencing is retained only for the bootstrap of the reconciliation machinery itself (mounts, config load); after `startEverything`, everything is controller/service-driven.

#### Finding 4.4: A node with no config boots into maintenance mode
**Evidence**: "When a Talos node boots without a machine configuration, it enters maintenance mode, where the Talos API is available but only accepts unauthenticated requests" — served with a self-signed cert; `talosctl --insecure` works only for "a small subset of Talos API commands ... required for initial setup and maintenance operations".
**Source**: [Insecure / maintenance mode, Talos v1.11](https://docs.siderolabs.com/talos/v1.11/configure-your-talos-cluster/system-configuration/insecure) - Accessed 2026-10-08 (same page exists for v1.7–v1.13)
**Confidence**: Medium (official, single page family; summary-level fetch)

#### Finding 4.5: A/B images with boot-once and automatic rollback on failed boot; PID-1 fatal error reverts the bootloader
**Evidence**: Talos uses an "A-B image scheme in order to facilitate rollbacks"; "We set the bootloader to boot once with the new kernel and OS image, then we reboot"; "If an upgrade fails to boot, Talos will roll back to the previous version"; `talosctl rollback` for manual revert (e.g. workload incompatibility); `--stage` defers the upgrade to the next boot when filesystems cannot be unmounted. In source, `machined`'s top-level `recovery()` catches panics and `handle(ctx, err)` calls `revertBootloader(ctx)`, then `proc.KillAll()`, unmounts, `unix.Sync()` (up to 30 s), and `unix.Reboot(...)`. A process reaper (`reaper.Run()`) handles zombie collection as PID 1.
**Source**: [Upgrading Talos, v1.11](https://docs.siderolabs.com/talos/v1.11/configure-your-talos-cluster/lifecycle-management/upgrading-talos); [siderolabs/talos `internal/app/machined/main.go` (main)](https://github.com/siderolabs/talos/blob/main/internal/app/machined/main.go) - Accessed 2026-10-08
**Confidence**: Medium-High (official docs + source agree on rollback; exact revert condition inside `revertBootloader` not read — see Knowledge Gaps)
**Analysis**: The node-level crash policy is: a crash of PID 1 is never "restart the process", it is **sync → reboot, and if we were on a not-yet-confirmed new image, reboot into the old one**. Reboot is the universal recovery primitive because boot is the universal convergence path (Finding 4.2).

### Q5 — Kernel and host state ownership, drift

#### Finding 5.1: Networking is declarative — config sources → merged `*Spec` (desired) → `*SpecController` applies to kernel → `*Status` (observed)
**Evidence**: "Talos translates network configuration from multiple sources: machine configuration, cloud metadata, network automatic configuration (e.g. DHCP) into COSI resources." Merge precedence "from low to high": default → cmdline → platform → operator → configuration. "*SpecController applies merged `*Spec` resources to the kernel state" (`LinkSpecController`, `AddressSpecController`, `RouteSpecController`); "Talos networking controllers watch the kernel state and update resources accordingly." Status listings include "addresses set up by other facilities (e.g. `flannel.1/10.244.4.0/32` set up by CNI)".
**Source**: [Networking Resources, Talos v1.11](https://docs.siderolabs.com/talos/v1.11/learn-more/networking-resources) - Accessed 2026-10-08
**Confidence**: High for the model (official docs + source below)
**Analysis**: Talos explicitly expects **co-tenants** on its kernel state: the CNI (flannel/Cilium), kube-proxy, the kernel itself (connected routes), router advertisements. Its ownership model is therefore "I own what I created, I tolerate everything else", not "I am the only writer". This is the key difference from Overdrive's stated threat model.

#### Finding 5.2: Drift handling is per-controller and NOT uniform — routes and links are watched continuously; addresses partly; nftables and sysctls only on spec change
Evidence from siderolabs/talos `main` (accessed 2026-10-08):

| Kernel state | Controller | Kernel-event watch? | Drift repair | Foreign objects |
|---|---|---|---|---|
| Routes | [`network/route_spec.go`](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/route_spec.go) | **Yes** — `watch.NewRtNetlink(trigger.NewDefaultRateLimitedTrigger(ctx, r), unix.RTMGRP_LINK\|unix.RTMGRP_IPV4_ROUTE\|unix.RTMGRP_IPV6_ROUTE)`; every reconcile does `conn.Route.List()` and diffs (`findOwnedRoutesByKey`, `routeMatchesSpec`) | Yes — a deleted/changed owned route is re-installed on the next (rate-limited) trigger | "Routes with the same key created by someone else (e.g. by the kernel for a connected subnet, or learned from router advertisements) are never touched." Ownership tracked by route **protocol** + finalizers/tombstones |
| Links | [`network/link_spec.go`](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/link_spec.go) | **Yes** — `RTMGRP_LINK` ("watch link changes as some routes might need to be re-applied if the link appears") | Yes — "sync UP flag", "sync MTU if it's set in the spec" | Physical Ethernet links never deleted; only logical (Kind-bearing) links torn down |
| Addresses | [`network/address_spec.go`](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/address_spec.go) | Link events only (`RTMGRP_LINK`), **not** address events | Re-applied when a link (re)appears or a spec changes; a foreign `ip addr del` on a stable link is not observed until another trigger | Deletes by matching spec address (finalizer-tracked) |
| nftables | [`network/nftables_chain.go`](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/network/nftables_chain.go) | **No** — reacts to `r.EventCh()` (resource changes) only | On each reconcile, owns one table (`constants.DefaultNfTablesTableName`): "drop all chains, they will be re-created", flushed atomically | Chains in other tables: "not our chain continue" |
| sysctls | [`runtime/kernel_param_spec.go`](https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/runtime/kernel_param_spec.go) | **No** — spec-change events only | None for external `/proc/sys` writes | On spec removal, restores the saved default (`resetKernelParam`) or deletes |

**Confidence**: Medium-High (primary source, read directly; single source per row; the address-ownership nuance is the fetched summary's reading and is flagged Medium)
**Analysis**:
- Talos applies **converge-on-change + converge-on-boot** universally (every controller reconciles from scratch when first started, and resources are rebuilt each boot — Finding 4.2), and **continuous kernel-watch** only where *another legitimate actor* routinely mutates the same kernel object (links appearing/disappearing, kernel/RA routes, DHCP). That is a co-tenancy argument, not an adversarial one.
- For nftables and sysctls — state nobody else on a Talos node is expected to write — Talos does **not** defend against foreign mutation at runtime. The implied stance: with no shell and no SSH, the only foreign writers are privileged workloads (CNI, privileged pods), which Talos treats as out of its responsibility. [Interpretation; not stated in docs.]
- The owned-table-wholesale-replace pattern for nftables (one table, rebuild all chains atomically each reconcile) is the cleanest "single owner" pattern in Talos.

### Q6 — Operator interface when broken

#### Finding 6.1: Diagnosis is entirely through the authenticated Talos API (apid) plus the console dashboard
**Evidence**: Troubleshooting workflow uses `talosctl dmesg` ("check `talosctl dmesg` for messages starting with `retrying:` prefix"), `talosctl logs <service>` (incl. `controller-runtime`, kubelet, etcd), `talosctl service`, `talosctl health`, `talosctl get members`, `talosctl get staticpods`/`staticpodstatus`, `talosctl etcd members`. "If the static pod definitions are not rendered, check `etcd` and `kubelet` service health (see above) and the controller runtime logs (`talosctl logs controller-runtime`)." kubelet: "Check that `kubelet` image is available (`talosctl image ls --namespace system`)". "Some information can be gathered from the Interactive Dashboard which is available on the machine console." etcd: "Make sure that a single member was bootstrapped."
**Source**: [Troubleshooting, Talos v1.11](https://docs.siderolabs.com/talos/v1.11/troubleshooting/troubleshooting) - Accessed 2026-10-08
**Confidence**: High (official troubleshooting page + CLI reference [v1.11 CLI](https://docs.siderolabs.com/talos/v1.11/reference/cli): `dmesg` "Retrieve kernel logs", `logs` "Retrieve logs for a service", `dashboard` "Cluster dashboard with node overview, logs and real-time metrics", `health` "Check cluster health")
**Analysis**: Talos replaces the shell with **typed, read-mostly introspection**: the same COSI resources controllers reconcile are exposed via `talosctl get` (with `--watch`) and `talosctl inspect dependencies`. The controller-runtime log is the canonical place to see reconcile errors — i.e. the observability surface *is* the reconcile machinery.

#### Finding 6.2: Recovery actions are a small, closed set of typed API verbs — no arbitrary execution
**Evidence**: `talosctl service <svc> restart`, `talosctl reboot`, `talosctl reset` (graceful: "cordon and drain the node, leaving `etcd` if required, and then erase its disks"; selective: `--system-labels-to-wipe=EPHEMERAL`; `--graceful=false` for broken clusters), `talosctl rollback`, `talosctl etcd {leave,remove-member,snapshot,defrag,alarm disarm,forfeit-leadership}`, `talosctl bootstrap --recover-from`, `talosctl apply-config` (incl. `--insecure` in maintenance mode), `talosctl cp` to pull files (e.g. the raw etcd db).
**Source**: [CLI reference v1.11](https://docs.siderolabs.com/talos/v1.11/reference/cli); [Disaster recovery](https://docs.siderolabs.com/talos/v1.11/build-and-extend-talos/cluster-operations-and-maintenance/disaster-recovery); [Scaling down](https://docs.siderolabs.com/talos/v1.11/deploy-and-manage-workloads/scaling-down) - Accessed 2026-10-08
**Confidence**: High (multiple official pages)
**Analysis**: The coarse-grained hammer is "wipe a labelled partition and reboot": because everything except STATE is re-derivable or re-joinable, wiping EPHEMERAL is a safe, general repair for corrupted application state on one node. When the API is unreachable (network config broken, apid down), the operator falls back to the physical/virtual console dashboard, or reboots into maintenance mode; there is no remote root shell. [The unreachable-node limitation is stated in the troubleshooting summary; exact wording not captured.]

### Q7 — Design lessons for Overdrive

*Everything in this section is **interpretation** built on Findings 1–6; it is labelled as analysis, not sourced fact. Overdrive references are to `CLAUDE.md` § "Overdrive runs on its own appliance OS" and `.claude/rules/{reconcilers,development}.md`.*

#### 7.1 What Talos treats as its responsibility vs out of scope

| Talos owns (automatic) | Talos delegates / leaves to operator |
|---|---|
| Restarting its own services forever (5 s fixed interval) and its own controllers (exponential backoff, panic-isolated) — F1.1, F1.4 | Restarting Kubernetes containers — kubelet's job (static pods) — F2.1 |
| Re-deriving *all* runtime state from one persisted document on every boot — F4.2 | Application data integrity in EPHEMERAL (etcd db, images) — repair = wipe & rejoin — F3.1, F6.2 |
| Single-member etcd restart and empty-dir rejoin as learner — F3.1 | Quorum-loss recovery, snapshot choice, member removal — F3.2, F3.3 |
| Re-applying its own routes/links when the kernel or co-tenants perturb them — F5.2 | Defending nftables/sysctls against foreign runtime writes — F5.2 |
| Rolling back a failed-to-boot upgrade; reverting the bootloader on PID-1 fatal error — F4.5 | Rolling back a *booted but workload-incompatible* upgrade (`talosctl rollback`) — F4.5 |

The line Talos draws: **anything whose correct outcome is computable from the machine config is automatic; anything that requires choosing which data to keep (quorum loss, divergent members) is a human decision through a typed API.**

#### 7.2 Patterns that map directly onto Overdrive's appliance threat model (borrow)

1. **Persist one input document; rebuild everything else every boot (F4.2).** This is the strongest possible form of Overdrive's "persist inputs, not derived state" and makes converge-on-boot the *only* path rather than a recovery special-case. Overdrive analogue: IntentStore is the input; every kernel object (nft tables, routes, BPF maps, cgroups, netns) should be re-derivable from intent + observation on boot. Where Overdrive persists kernel state across a control-plane restart (bpffs pins, kTLS — per CLAUDE.md the survival semantics are a Tier-3 question), it should be *adopted via observe → diff*, as Talos does with existing etcd data (`initial-cluster-state=existing`), never trusted blind.
2. **Owned-table-wholesale-replace for nftables (F5.2).** Talos owns exactly one nft table, rebuilds all chains in it atomically each reconcile, and ignores other tables. This is the right shape for Overdrive's `overdrive-mtls` table: one owner, full replace, atomic flush — no per-rule diff bookkeeping.
3. **Ownership tags on shared kernel namespaces (F5.2).** Talos marks its routes by **protocol** and only ever deletes routes it owns, leaving kernel-connected and RA routes alone. Even on an appliance, the kernel itself (connected routes, link-local, IPv6 autoconf) is a co-writer; Overdrive should tag its routes (`proto`) and rules the same way so converge can distinguish "mine, wrong" from "kernel's".
4. **Two supervision layers with different policies (F1.1 vs F1.4).** Process supervision (fixed interval, forever) and in-process controller supervision (exponential backoff, panic recovery, `ControllerCrashes` metric, re-trigger reconcile after restart). Overdrive's reconciler runtime should match the second shape: a panicking reconciler is isolated, counted, backed off, and re-run from fresh hydration — not allowed to kill `overdrive serve`.
5. **Restart = re-derive config (F1.3).** Talos's `PreFunc` re-reads the service spec from resources on every restart. A restarted Overdrive-managed process (VMM, DNS responder) should likewise be launched from the current derived spec, never from a cached launch argv.
6. **PID-1 fatal error → sync → reboot, with bootloader revert (F4.5).** On an appliance with no operator, "reboot into the last known-good image" is the right last resort, and it is only safe because boot is the convergence path. Overdrive's image factory / ADR-0068 work should carry boot-once + confirm-on-healthy semantics.
7. **Single restart authority per layer (F2.1).** Talos restarts kubelet; kubelet restarts control-plane containers; Talos never restarts a static pod container directly. This mirrors Overdrive's "single restart authority" rule — one budget per owner, no split by cause.
8. **Operator surface = the reconcile model, read-only, plus a closed verb set (F6.1, F6.2).** `talosctl get <resource> --watch` and `logs controller-runtime` expose the same desired/observed resources the controllers use. Overdrive's CLI can expose intent, observation rows and reconciler View/errors the same way; recovery verbs should be a small typed set (restart workload, wipe-and-rejoin a partition-equivalent, restore-from-snapshot), never exec.

#### 7.3 Where Overdrive should deliberately differ

1. **Restart backoff.** Talos's `Forever` + fixed 5 s has no crash budget and no occurrence record beyond the event stream. Overdrive's rules require a crash to be observable as an occurrence (`LastTerminated` + monotone `restart_count`, development.md § "A convergent record cannot answer 'did it happen'") and a bounded backoff owned by one reconciler. Do not copy Talos's flat retry for workloads.
2. **Continuous kernel watch is a co-tenancy tool, not an adversary defence.** Talos watches netlink for routes/links because CNI, DHCP, RA and the kernel legitimately mutate them; it does *not* watch nft or sysctls, where it has no co-tenant. On Overdrive, where only Overdrive writes kernel state, this supports the CLAUDE.md stance: do **not** build runtime foreign-mutation defence. Continuous watching is justified only where the *kernel itself* or *another Overdrive component* changes the object (e.g. a link vanishing when a VMM dies) — that is Overdrive's-own-failure territory and maps to row-backed `interests()` or a host-backed `resync_schedule()`, not to drift paranoia.
3. **Quorum loss.** Talos leaves it entirely manual. Overdrive (single-node Phase 1; Raft later) should still keep "choose which history survives" a human, typed action — Talos and etcd upstream agree a restore "must start a new logical cluster" (F3.2) — but can make snapshot-and-restore a first-class verb from day one.
4. **Mixed drift semantics across controllers.** Talos's controllers are inconsistent (routes/links watched, addresses only on link events, nft/sysctl on spec change only — F5.2). Overdrive should make the wakeup model an explicit per-reconciler declaration (as ADR-0084 already does) rather than an accident of each controller's implementation.

## Source Analysis
| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Talos docs: Disaster Recovery v1.11 | docs.siderolabs.com | High (1.0) | official project docs | 2026-10-08 | Y (etcd.go, CLI ref, etcd.io) |
| Talos docs: etcd maintenance v1.11 | docs.siderolabs.com | High | official project docs | 2026-10-08 | N |
| Talos docs: CLI reference v1.11 | docs.siderolabs.com | High | official project docs | 2026-10-08 | Y |
| Talos docs: Scaling down v1.11 | docs.siderolabs.com | High | official project docs | 2026-10-08 | Y (CLI ref) |
| Talos docs: Static Pods (v1.13 URL / v1.14 label) | docs.siderolabs.com | High | official project docs | 2026-10-08 | Y (kubernetes.io) |
| Talos docs: Controllers and Resources v1.11 | docs.siderolabs.com | High | official project docs | 2026-10-08 | Y (cosi runtime source) |
| Talos docs: Networking Resources v1.11 | docs.siderolabs.com | High | official project docs | 2026-10-08 | Y (network controller source) |
| Talos docs: Architecture v1.11 | docs.siderolabs.com | High | official project docs | 2026-10-08 | Y (DR procedure) |
| Talos docs: Upgrading Talos v1.11 | docs.siderolabs.com | High | official project docs | 2026-10-08 | Y (main.go) |
| Talos docs: Insecure / maintenance mode v1.11 | docs.siderolabs.com | High | official project docs | 2026-10-08 | N |
| Talos docs: Troubleshooting v1.11 | docs.siderolabs.com | High | official project docs | 2026-10-08 | Y (CLI ref) |
| siderolabs/talos `restart.go` | github.com | Medium-High (0.8; primary source) | source code | 2026-10-08 | Y (service files) |
| siderolabs/talos `service_runner.go` | github.com | Medium-High | source code | 2026-10-08 | Y (CLI) |
| siderolabs/talos `services/kubelet.go` | github.com | Medium-High | source code | 2026-10-08 | N |
| siderolabs/talos `services/etcd.go` | github.com | Medium-High | source code | 2026-10-08 | Y (DR docs) |
| siderolabs/talos `network/route_spec.go` | github.com | Medium-High | source code | 2026-10-08 | Y (networking docs) |
| siderolabs/talos `network/link_spec.go` | github.com | Medium-High | source code | 2026-10-08 | Y (networking docs) |
| siderolabs/talos `network/address_spec.go` | github.com | Medium-High | source code | 2026-10-08 | Partial |
| siderolabs/talos `network/nftables_chain.go` | github.com | Medium-High | source code | 2026-10-08 | N |
| siderolabs/talos `runtime/kernel_param_spec.go` | github.com | Medium-High | source code | 2026-10-08 | N |
| siderolabs/talos `v1alpha1_sequencer.go`, `machined/main.go` | github.com | Medium-High | source code | 2026-10-08 | Y (upgrade docs) |
| cosi-project/runtime `rruntime/run.go`, `rruntime.go` | github.com | Medium-High | source code | 2026-10-08 | Partial (docs silent) |
| Kubernetes: Create static Pods | kubernetes.io | High | official upstream | 2026-10-08 | Y |
| etcd v3.5: Disaster recovery | etcd.io | High | official upstream | 2026-10-08 | Y |

Reputation: High: 13 (54%) | Medium-High: 11 (46%) | Avg: ≈ 0.91. Note: siderolabs is the vendor of Talos (commercial interest in presenting it favourably); this was mitigated by checking documented claims against source code. Rejected: oneuptime.com blog posts (untrusted domain, not cited).

## Knowledge Gaps
### Gap 1: Source not pinned to a release tag
**Issue**: Source files were read on `main` (2026-10-08), while docs are v1.11–v1.14. Behaviour (e.g. the address-controller netlink scope, nftables table name) may differ between releases. | **Attempted**: raw.githubusercontent `main` | **Recommendation**: Re-read the same files at the tag matching the version Overdrive benchmarks against (e.g. `v1.11.x`) before citing in an ADR.

### Gap 2: Exact rollback trigger
**Issue**: How Talos decides an upgrade "failed to boot" (boot-once without confirmation vs. explicit success marking in META, and the condition inside `revertBootloader`) was not read. | **Attempted**: `machined/main.go` (function body not in that file) | **Recommendation**: Read the bootloader packages (`internal/pkg/... /bootloader` grub & sd-boot) and META upgrade keys.

### Gap 3: Service-runner backoff beyond the 5 s default; per-service overrides
**Issue**: Only kubelet and etcd service definitions were read; apid, trustd, containerd, udevd may set different policies/intervals. Whether the `Forever` loop ever escalates (e.g. reboot after N failures) was not observed in `restart.go` — none found. | **Recommendation**: Grep `restart.WithRestartInterval` across `services/`.

### Gap 4: COSI controller backoff parameters
**Issue**: `backoff.NewExponentialBackOff()` defaults (cenkalti/backoff: initial 500 ms, multiplier 1.5, max interval 60 s) are inferred from the library, not confirmed for the vendored version; the `runOnce` panic-recovery code was summarised, not quoted. | **Recommendation**: Read `rruntime.go` and the vendored backoff version.

### Gap 5: Kubelet crash with running static-pod containers
**Issue**: That control-plane containers survive a kubelet crash (owned by containerd) and are re-adopted is standard kubelet behaviour, not Talos-documented. The kubernetes.io pod-lifecycle restart-backoff text could not be extracted (page truncated in fetch). | **Recommendation**: Tier-3 style check on a Talos VM (`talosctl service kubelet restart`, observe `crictl ps`).

### Gap 6: Address-controller ownership nuance
**Issue**: The fetch suggested `AddressSpecController` deletes by matching spec address rather than verifying creator; this conflicts mildly with the docs' "unmanaged resources are preserved" framing. | **Recommendation**: Read `address_spec.go` teardown path directly.

## Conflicting Information
### Conflict 1: "Controllers watch the kernel state" vs. controllers that only react to spec changes
**Position A**: "Talos networking controllers watch the kernel state and update resources accordingly" — Source: [Networking Resources v1.11](https://docs.siderolabs.com/talos/v1.11/learn-more/networking-resources), Reputation 1.0.
**Position B**: `nftables_chain.go` and `kernel_param_spec.go` reconcile only on `r.EventCh()` (resource changes); `address_spec.go` watches only `RTMGRP_LINK` — Source: siderolabs/talos `main` source, Reputation 0.8 (primary).
**Assessment**: Not a true contradiction: the docs statement concerns the *status/observation* controllers and the link/route spec controllers; nftables and sysctls are not network-status-watched. Source code is authoritative for the per-object behaviour; the docs' generalisation should not be read as "all Talos-owned kernel state is continuously reconciled".

## Recommendations for Further Research
1. Pin and re-read the cited files at a release tag (Gap 1), then cite path@tag in any Overdrive ADR.
2. Read Talos's bootloader/META upgrade-confirmation code (Gap 2) — directly relevant to Overdrive's image-factory / ADR-0068 boot-once design.
3. Compare Talos's KubeSpan/WireGuard controller for how it treats peer state across reboots (gossiped vs persisted) — relevant to Overdrive's observation-store design.
4. Empirical probe on a Talos VM: delete an owned route, an owned nft chain and a sysctl, and observe which are restored and when (validates Finding 5.2 without relying on code reading).

## Full Citations
[1] Sidero Labs. "Disaster Recovery". Talos Linux v1.11 docs. https://docs.siderolabs.com/talos/v1.11/build-and-extend-talos/cluster-operations-and-maintenance/disaster-recovery. Accessed 2026-10-08.
[2] Sidero Labs. "etcd Maintenance". Talos Linux v1.11 docs. https://docs.siderolabs.com/talos/v1.11/build-and-extend-talos/cluster-operations-and-maintenance/etcd-maintenance. Accessed 2026-10-08.
[3] Sidero Labs. "CLI reference (talosctl)". Talos Linux v1.11 docs. https://docs.siderolabs.com/talos/v1.11/reference/cli. Accessed 2026-10-08.
[4] Sidero Labs. "Scaling Down". Talos Linux v1.11 docs. https://docs.siderolabs.com/talos/v1.11/deploy-and-manage-workloads/scaling-down. Accessed 2026-10-08.
[5] Sidero Labs. "Static Pods". Talos Linux docs (served as v1.14). https://docs.siderolabs.com/talos/v1.13/configure-your-talos-cluster/images-container-runtime/static-pods.md. Accessed 2026-10-08.
[6] Sidero Labs. "Controllers and Resources". Talos Linux v1.11 docs. https://docs.siderolabs.com/talos/v1.11/learn-more/controllers-resources. Accessed 2026-10-08.
[7] Sidero Labs. "Networking Resources". Talos Linux v1.11 docs. https://docs.siderolabs.com/talos/v1.11/learn-more/networking-resources. Accessed 2026-10-08.
[8] Sidero Labs. "Architecture". Talos Linux v1.11 docs. https://docs.siderolabs.com/talos/v1.11/learn-more/architecture. Accessed 2026-10-08.
[9] Sidero Labs. "Upgrading Talos Linux". Talos Linux v1.11 docs. https://docs.siderolabs.com/talos/v1.11/configure-your-talos-cluster/lifecycle-management/upgrading-talos. Accessed 2026-10-08.
[10] Sidero Labs. "Insecure mode / maintenance mode". Talos Linux v1.11 docs. https://docs.siderolabs.com/talos/v1.11/configure-your-talos-cluster/system-configuration/insecure. Accessed 2026-10-08.
[11] Sidero Labs. "Troubleshooting". Talos Linux v1.11 docs. https://docs.siderolabs.com/talos/v1.11/troubleshooting/troubleshooting. Accessed 2026-10-08.
[12] siderolabs/talos. `internal/app/machined/pkg/system/runner/restart/restart.go`. GitHub, main. https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/system/runner/restart/restart.go. Accessed 2026-10-08.
[13] siderolabs/talos. `internal/app/machined/pkg/system/service_runner.go`. GitHub, main. https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/system/service_runner.go. Accessed 2026-10-08.
[14] siderolabs/talos. `internal/app/machined/pkg/system/services/kubelet.go`. GitHub, main. https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/system/services/kubelet.go. Accessed 2026-10-08.
[15] siderolabs/talos. `internal/app/machined/pkg/system/services/etcd.go`. GitHub, main. https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/system/services/etcd.go. Accessed 2026-10-08.
[16] siderolabs/talos. `internal/app/machined/pkg/controllers/network/{route_spec,link_spec,address_spec,nftables_chain}.go`. GitHub, main. https://github.com/siderolabs/talos/tree/main/internal/app/machined/pkg/controllers/network. Accessed 2026-10-08.
[17] siderolabs/talos. `internal/app/machined/pkg/controllers/runtime/kernel_param_spec.go`. GitHub, main. https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/controllers/runtime/kernel_param_spec.go. Accessed 2026-10-08.
[18] siderolabs/talos. `internal/app/machined/pkg/runtime/v1alpha1/v1alpha1_sequencer.go`. GitHub, main. https://github.com/siderolabs/talos/blob/main/internal/app/machined/pkg/runtime/v1alpha1/v1alpha1_sequencer.go. Accessed 2026-10-08.
[19] siderolabs/talos. `internal/app/machined/main.go`. GitHub, main. https://github.com/siderolabs/talos/blob/main/internal/app/machined/main.go. Accessed 2026-10-08.
[20] cosi-project/runtime. `pkg/controller/runtime/internal/rruntime/{run,rruntime}.go`. GitHub, main. https://github.com/cosi-project/runtime/tree/main/pkg/controller/runtime/internal/rruntime. Accessed 2026-10-08.
[21] Kubernetes Authors. "Create static Pods". kubernetes.io. https://kubernetes.io/docs/tasks/configure-pod-container/static-pod/. Accessed 2026-10-08.
[22] etcd Authors. "Disaster recovery" (v3.5 operations guide). etcd.io. https://etcd.io/docs/v3.5/op-guide/recovery/. Accessed 2026-10-08.

## Research Metadata
Duration: ~1 session | Examined: 27 (incl. rejected blogs) | Cited: 22 | Cross-refs: 11 findings cross-verified | Confidence: High ≈ 35%, Medium-High ≈ 45%, Medium ≈ 20%, Low 0% | Per-question: Q1 Medium-High, Q2 High (mechanism) / Medium (kubelet-crash detail), Q3 High, Q4 Medium-High, Q5 Medium-High, Q6 High, Q7 interpretation (no confidence rating; derived) | Output: docs/research/appliance-os/talos-kubernetes-crash-handling-comprehensive-research.md | Tool notes: talos.dev URLs 301 to docs.siderolabs.com; one cosi-project path 404 (resolved via directory listing); kubernetes.io pod-lifecycle page truncated in fetch.
