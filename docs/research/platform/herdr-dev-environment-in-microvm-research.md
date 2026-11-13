# Research: Developing Overdrive with herdr inside microVMs on an Overdrive node

**Date**: 2026-11-08 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High (code/ADR facts High; herdr resource profile and some prior-art details Low) | **Sources**: 26 cited

## Executive Summary

**Is Service enough?** The `Service` kind is the right kind (long-lived, single replica is allowed, needs a TCP listener, supervised restart) and no evidence suggests its semantics must change. As it stands it is not enough: everything a dev box needs beyond "a supervised VM process" is missing around it. The guest rootfs is a disposable reflink clone of a read-only master, recreated on every restart; there is no volume, second disk, snapshot or disk-size field; volumes were cut from the VM driver on 2026-08-18 and the issues that own them (#97, #43) have empty acceptance criteria.

**Filesystem.** This is the blocking gap. herdr (Apache-2.0, Rust, a background server holding PTYs, attached locally or over OpenSSH) loses its processes on any server/VM restart but restores layout and agent conversations from files on disk, so a persistent disk is what makes restart recoverable. Overdrive development additionally needs tens of GB for `target/` and cargo caches. Fly Volumes, Coder and E2B all separate disk lifetime from compute lifetime.

**Separate sandbox primitive?** Not needed for a minimum viable box. Recommended (advisory): Service + a workload-id-keyed persistent volume + raw-TCP ingress, consistent with the April sprites research (no new driver; compose a profile later). #296 is necessary only if the box must be reachable on a private WireGuard address, and by itself it does not carry SSH: raw TCP ingress is #338, and WireGuard is #77. Cloud Hypervisor supports nested virtualization, so `kvm-tests` can *run* in the guest if the host enables nested KVM and the driver exposes VMX/SVM to the guest; what testing.md forbids is counting a nested run as Tier-3 evidence. Iteration can happen in the box; evidence still comes from metal unless that rule changes.

## Research Methodology
**Search Strategy**: herdr project site/README/docs (herdr.dev, github.com/herdrdev/herdr); Overdrive code, ADRs, feature deltas, research docs read directly with Grep/Read; GitHub issues fetched as public pages (the `gh` CLI was unavailable in this run, so issue pages were fetched via WebFetch; comment threads were included only where the page showed them); prior-art vendor docs (docs.fly.io, docs.e2b.dev, docs.github.com, coder.com).
**Source Selection**: Types: official/project docs, first-party code and ADRs | Reputation: high (primary) | Verification: herdr facts come from one project (multiple pages, not independent); Overdrive facts cross-checked code vs ADR vs issue where possible.
**Quality Standards**: 1 authoritative source minimum for primary-doc facts; analysis is labelled as such; unverified statements are marked. Not used: the whitepaper (outdated per project guidance). No Linux kernel claims are made, so no `vendors/linux` citations were needed beyond those quoted inside ADR text.
**Limitations**: no shell tool (could not run `gh`, `grep -c` over tests, or any measurement); Daytona/Modal docs not fetched.

## Findings

### 1. What herdr is and what it needs from its host

All facts below are from herdr's own site and README (accessed 2026-11-08). Third-party sources were not needed; where the primary docs are silent this is stated.

**Two products share the name.** herdr.dev lists three repos: `herdrdev/herdr` (the TUI), `herdrdev/herdr-core` (a server plus `hc` CLI) and `herdrdev/herdr-app` (a GUI client) [1][2]. The page states "Herdr TUI is a separate program with its own server. Agents in one don't appear in the other" [1]. The TUI is the one that matches "agent multiplexer"; `herdr-core` is the server behind the GUI app. This doc assumes the TUI (`herdr`) and notes core where it differs. A web search also surfaced the repo `ogulcancelik/herdr` and several forks; `herdrdev/herdr` is the repo the project site links, so it is treated as canonical [1][3].

| Question | Finding | Source |
|---|---|---|
| What it is | "The runtime your coding agents live on": a terminal workspace running Claude Code, Codex, Cursor, OpenCode, Grok etc. side by side with per-pane working/blocked/idle status; "one rust binary, no electron" | [3][4] |
| Process model | Background server holds the terminals (PTYs); the `herdr` client attaches. "It's a server running in the background, and the terminals live inside it." Server ignores SIGHUP on Linux/macOS; only `herdr server stop` or SIGTERM stops it | [4][5] |
| Attach | Local: run `herdr` (reattach), `ctrl+b q` detaches. Remote: `herdr --remote <target>` / `herdr machine add <name>` over **OpenSSH** | [3][5][6] |
| Remote requirements | Normal OpenSSH access to the box. herdr must be on the remote PATH or `~/.local/bin/herdr`; interactive setup offers to install it, non-interactive runs never install. Linux/macOS x86_64 or aarch64. No herdr-specific ports; "the only port mentioned is the optional one in an SSH URL". Auth is delegated to OpenSSH; herdr stores no keys | [6][5] |
| Socket API | "Newline-delimited JSON over a local socket": Unix domain socket at `~/.config/herdr/herdr.sock` (named sessions `~/.config/herdr/sessions/<name>/herdr.sock`). No TCP documented. No authentication described (access control is filesystem permissions on the socket, by inference [unverified]) | [7] |
| What it spawns | Whatever the pane runs (agent CLIs, shells). "Doesn't wrap or replace them." It detects 22 agent CLIs by process name and terminal output; agents emitting OSC 7501 report richer status | [3][4][1] |
| State on disk | Session dir holding `session.json` (layout, cwds, focus, native agent session ids), `session-backups/` (3 kept), `session-snapshots/` (up to 48), optional `session-history.json` (opt-in pane screen history). Config in `~/.config/herdr-app/` for the GUI (`settings.json`, `keys.json`, layouts, mods, themes) | [8][1] |
| Restart semantics | Detach/reattach keeps processes. Server restart or machine reboot: **processes die**; layout restores and supported agents can resume their conversation via native session refs | [8][5] |
| Network needs | Not documented by herdr. Inbound: only SSH for remote attach. Outbound: whatever the agents need (the model APIs, git remotes) plus `herdr.dev` for install/update. Agent forwarding (`ForwardAgent yes`) is supported | [6][3] |
| Resource profile | Not documented (no CPU/RAM/disk figures) [gap]. Panes' processes dominate; herdr itself is a single Rust binary | [5] |
| Platforms / licence | macOS, Linux (x86_64, arm64), Windows. Apache-2.0 (one fork listing says AGPL; the repo README and site say Apache-2.0, which is taken as correct) | [3][4] |
| Git worktrees | Not mentioned in the fetched pages. herdr stores per-pane working directories only. Worktree handling is the user's or agent's job [gap: not confirmed either way] | [8] |

**Implication for hosting.** herdr needs, from its host: (1) a long-lived Linux userspace with a persistent `$HOME` (session dir, `~/.config/herdr`), (2) a way for a human to get a terminal into it, which for remote use is OpenSSH to a user account with the `herdr` binary (so an **sshd or equivalent and a reachable TCP port**), (3) outbound internet for agents. It does not need a GUI, a public HTTP port, or inbound ports other than SSH. Its restart story ("processes die, layout and agent conversations resume") means the VM's disk survival matters more than process survival.

**Confidence**: High for process model, attach, remote requirements and state (primary docs, multiple pages of one project - single organisation, so independence is limited). Low for resource profile and worktrees (not documented).

### 2. What developing Overdrive demands of the environment

Source: the repo's own rules (`.claude/rules/testing.md`, `development.md`, `CLAUDE.md`) as loaded for this task. Classification of "inside a microVM guest" is analysis (labelled), because the guest owns its kernel.

| Demand (rule text) | Inside a guest? | Notes |
|---|---|---|
| Every test run goes through `cargo xtask lima run --` (root in a Linux VM) or `cargo xtask metal run --`; bare `cargo nextest run`, `cargo check` (macOS), `cargo clippy` are blocked by hooks (testing.md "Running tests - Lima VM"; development.md "Compile-checking") | **Tooling gap**: a guest is neither Lima nor the metal box. Hooks `block-bare-nextest` allow only `lima run` or `metal run`. A dev box needs a third, in-guest execution mode or the hooks/xtask must treat it as native Linux [analysis; no issue found] | The hook on Linux for `cargo check` is a no-op; nextest's is not |
| Root, cgroup v2 delegation (`/sys/fs/cgroup/overdrive.slice/...`), netns/veth, nft tables, routes | **Achievable**: these are the guest kernel's own objects; the guest is a full Linux with root | Lima is itself a virtualized guest and is the "canonical inner loop" (testing.md), so a microVM guest is the same class of environment |
| BPF/XDP/TC/sockops loading, bpffs, CAP_BPF, kTLS, BPF LSM (Tier 2/3) | **Achievable if the guest kernel is built for it** [analysis]: needs a 7.0 kernel with BPF, BPF LSM, kTLS, nft, veth, cgroup2. Kernel pin is Linux 7.0; testing.md says real-kernel evidence counts when it ran on a 7.0 kernel. BYO kernel must be a bzImage or ELF (`KernelImage::validate`) | Open risk: the guest already runs Overdrive's own guest-kernel eBPF for vsock capture (ADR-0150); whether developer tests that attach XDP/cgroup programs or own nft tables coexist with it is **unverified** - needs a spike |
| `leaked cgroup/XDP/netns` cleanup, `host-kernel-shared` nextest group, net-slot bands | Achievable; a throwaway guest is a *better* cleanup story than shared Lima (recreate rather than clean) | testing.md already says the Lima VM is disposable |
| `kvm-tests` lane: real Cloud Hypervisor microVM boots | **Runnable with nested KVM; non-signal as evidence** | See below |
| Large Rust `target/` (tens of GB), cargo registry, git checkouts/worktrees | Achievable but **needs a large persistent writable disk** | Overdrive target dir for the whole workspace plus Lima's `~/.cargo-target-lima`; sizes not measured here [gap]. The spec has one rootfs disk only |
| `vendors/cloud-hypervisor`, `vendors/linux` submodules (large) | Disk | `vendors/linux` is a read-only reference |
| Outbound network: git remote, crates.io, model APIs, `herdr.dev` | Needs guest egress | Section 3.3 |
| Reach the metal box for `kvm-tests` (`OVERDRIVE_METAL_TARGET`, `cargo xtask metal run`) | Achievable via egress SSH + a key in the guest | Needs a secret in the guest (section 3.5) |

**The KVM constraint (verified against testing.md).** The section "Running tests - native x86_64 bare-metal KVM host (`kvm-tests`)" states that the CH microVM driver's authoritative Tier-3 boot surface is the non-virtualized x86_64 metal host with "real VMX/SVM extensions, a usable `/dev/kvm`, and **no virtualization layer or nesting**", and "Lima, virtualized hosts, and nested hosts are compile-only/non-signal for this surface." The real-guest feature run "MUST fail closed before collecting evidence unless" `systemd-detect-virt` reports `none` and `/dev/kvm` passes the KVM API probe. This is a project evidence rule, not a capability limit: Cloud Hypervisor supports nested virtualization, so with nested KVM enabled on the host (`kvm_intel`/`kvm_amd` `nested=1`) and VMX/SVM exposed to the guest, a dev box gets a working `/dev/kvm` and can boot microVMs inside itself. What blocks `kvm-tests` there today is the fail-closed substrate probe (`systemd-detect-virt` will not report `none`), which aborts the run by design, plus the rule that a nested run is not Tier-3 evidence. Unverified here: whether Overdrive's CH fork launch path exposes VMX/SVM to the guest (the `vendors/cloud-hypervisor` submodule is not checked out in this workspace), and whether the appliance host kernel enables nested KVM. Consequence under the current rule: **a herdr dev box can do editing, `cargo check`/clippy, the default unit/proptest lane, DST (`cargo dst`), Quint model checks, and the Linux-kernel integration tests that use only the guest's own kernel (veth/netns/nft/XDP/cgroups); anything under the `kvm-tests` feature (VmDriver real boots, CH fork work) must still run on the metal box**, driven from the dev box with `cargo xtask metal run --`. (Which exact tests that excludes is determined by the `kvm-tests` Cargo feature; a count was not taken.)

### 3. Inventory of what Overdrive offers today
#### 3.1 Workload kinds and the VM spec

- Intent kinds are `Job`, `Service` and `Schedule` (`WorkloadIntentV1`, `crates/overdrive-core/src/aggregate/mod.rs:335-343`). There is exactly one driver, `WorkloadDriverV1::Vm(Vm)` (`mod.rs:156-159`), i.e. Cloud Hypervisor microVMs.
- `Vm` is four strings: `command`, `args`, `kernel`, `rootfs` (`mod.rs:178-189`). **There is no `persistent` flag, no env map, no mount/volume field, no disk-size field in the spec.** The `persistent = true` flag in project memory is a design intent from the sprites research, not code.
- `ServiceV1` = `id`, `replicas: NonZeroU32`, `resources`, `driver`, `listeners: Vec<Listener>` (>=1 required, TCP/UDP, `(port, protocol)` unique), plus startup/readiness/liveness probe lists (`mod.rs:358-367`; validation list at `mod.rs:392-411`). A Service must declare at least one listener.
- `kernel` and `rootfs` are operator-supplied **host filesystem paths** (BYO). The doc comment says per-VM volumes "are out of scope for this feature (deferred to overdrive-fs, GH #97 / virtiofsd, GH #43)" (`mod.rs:161-166`). Image tooling (OCI/Dockerfile to rootfs) is #259, open [10].
- The guest runs `command` as the workload under the platform's `overdrive-init` (PID 1, injected by the image factory per #259 comment; `ADR-0150` governs guest-kernel adaptation). So a dev box today would be "a rootfs whose `command` is a long-running process" - there is no systemd contract implied; if herdr needs sshd, `command` would have to start it or the rootfs's init would have to.
- Launch shape (`crates/overdrive-core/src/vm/config.rs`): the platform renders `console=ttyS0 panic=1 root=/dev/vda rw` (`config.rs:726-732`), attaches the rootfs as one virtio-blk raw disk (`DiskAttachment::to_disk_arg`, `config.rs:234`), derives vCPUs from `cpu_milli` as `max(1, ceil(cpu_milli/1000))` capped at 255 (`vcpus_for`, `config.rs:368`), and sets guest RAM = `resources.memory_bytes` with the cgroup ceiling = guest RAM + measured reserve (8 MiB + RAM/400, `config.rs:331`). CH runs as an unprivileged uid 4200 with seccomp `true`, Landlock and `prlimit`/`setpriv` (`config.rs:422`, `:458`, `:501`).

#### 3.2 Storage and filesystem

- **The rootfs is a per-launch reflink clone.** `RootfsPlan::for_alloc` clones the operator's read-only master into `<data_dir>/vm/clone-staging/.overdrive-vm-rootfs-<alloc>.img` via FICLONE, and that clone is what the guest sees as `/dev/vda` (`config.rs:593-636`, `:697`). A reboot-orphaned clone is reaped by a durable index sweep (`clone_index_dir`, ADR-0083 §D3f-h). So the guest's writes land on a **disposable clone keyed by allocation id**; a replaced allocation (new alloc id after a crash restart) boots from the pristine master again. Persistence of guest writes across restart: **none today**.
- Constraints on the BYO rootfs (issue #97 comment): same filesystem as clone-staging (FICLONE `EXDEV` otherwise) and directory traversable by uid 4200 [11].
- **Volumes were cut.** `[[vm.volume]]`, virtiofsd and `--memory shared=on` were cut 2026-08-18, deferred to #97 (managed block-volume / `overdrive-fs`) and #43 (virtiofsd lifecycle) (`docs/product/architecture/brief.md:851-853`, `:9069`, `:10234`; `aggregate/mod.rs:161-166`).
- #97 (open, Phase 6, blocked by #22 Garage and #42): "`overdrive-fs`" Rust chunk store - Garage chunks + per-rootfs libSQL metadata + NVMe cache, **single-writer per rootfs**, exposed via vhost-user-fs (seam vs vhost-user-blk still open); acceptance criteria empty. Its spike notes `fsync` must be honoured (silent data-loss window otherwise) and that virtiofs survives checkpoint/restore on CH v53 [11].
- #43 (open, Phase 3): virtiofsd lifecycle and cross-workload volume sharing; acceptance empty [12].
- #259 (open): OCI/Dockerfile to bootable rootfs; "writable state is a per-launch layer"; ext4-on-virtio-blk first, erofs + overlayfs as end state; avoid virtio-fs on the hot path [10].
- Net: **there is no persistent block device, volume, snapshot or disk-size knob for a VM workload anywhere in code, and the two issues that own it (#97, #43) have empty acceptance criteria.** The sprites research already recommended extending the microvm driver with a persistent rootfs bound to workload identity (its "Phase 2") [13].

#### 3.3 Networking (egress, east-west, ingress, non-HTTP)

- **The guest has no NIC.** ADR-0145 (user-approved; "Not yet implemented" in its own header, but the launcher already matches it): a VM allocation has no guest network device, no TAP, no bridge port; application traffic crosses one virtio-vsock device with kernel vhost-vsock (`ADR-0145` §Decision). `crates/overdrive-host/src/vmm.rs:1335` asserts `!args.contains("--net")` and the `--vsock` arg is `cid=<cid>,backend=vhost-kernel` (`vmm.rs:373-374`). Consequently **there is no `eth0` for sshd to bind in a conventional sense**; guest sockets are captured by guest-kernel eBPF and carried over vsock (ADR-0150).
- **Egress to the internet works by design.** ADR-0162 (accepted): a guest TCP flow whose destination is non-mesh is paired with a host TCP socket the owner `connect()`s directly to the original destination, forwarded in the kernel; UDP likewise; host-internal destinations are refused (ADR-0167); "NAT and egress policy for VMs are unchanged" (ADR-0162 §Consequences). The #295 spike reports unmodified `curl` fetched `http://example.com/` and a real Internet SSH banner arrived (ADR-0162 §Context). That is evidence for outbound HTTPS to `api.anthropic.com` / `github.com` and `git over ssh`, **but those spikes ran in the spike harness, not through `serve` + `deploy`** (ADR-0145 §Context: "None ran through `overdrive serve` + `overdrive deploy`"). DNS from the guest: spike used `getent`, `dig`, `nslookup` (ADR-0145).
- **Inbound to a VM port**: a remote mesh client reaches the VM through leg-C (mTLS) at the VM's `workload_addr`; a host platform client reaches a declared TCP listen port through an owner-opened vsock flow (ADR-0173, ADR-0172). Cross-node VM addressing is #317 (brief.md:1154). Dial-by-name east-west is ADR-0072. **A plain `ssh` client cannot speak mTLS to leg-C**, so SSH needs a front door.
- **Public ingress today is HTTPS only.** The gateway binds IPv4 TCP/443, TLS 1.3 + HTTP/1.1, one exact hostname/path route to one TCP Service listener (`public-ingress-gateway/feature-delta.md:183-185`, `:220`, `:2520`). Raw TCP ingress is #338 ("validated with SSH to a VM Service"; open, no comments, `feature-delta.md:959` marks it out of scope) [14]. HTTP/1.1 only means WebSocket-style or CONNECT tunnelling of SSH is not an existing feature either [unverified whether the L7 gateway supports `Upgrade`; not found in the delta].
- **Private (WireGuard) gateway**: #296 (open, assigned to maintainer, no comments): named Gateway resources with public and private listener bindings; private listener on the node's WireGuard address; WireGuard underlay is #77 (open, "mesh VPN underlay extensions: `wireguard`, platform-managed keys via enrollment", blocked by #36, #75, #84; acceptance empty) [15][16].
#### 3.4 Exec / attach

- The operator CLI has `deploy`, `job {list,stop}`, `workload {restart,describe}`, `node list`, `cluster {upgrade,status}`, `serve` (`crates/overdrive-cli/src/cli.rs:39-142`). **There is no `exec`, `console`, `attach`, `logs` or `shell` verb.**
- The guest console is a serial capture to `<run_dir>/console.log` (`VmRunDir::console_log`, `config.rs:818`), not an interactive channel.
- The guest control channel is vsock: `overdrive-init` (PID 1) sends a beacon `READY`, receives one `EXEC {argv}` for the workload command, reports `EXIT`/`SHUTDOWN`; it is not a persistent in-guest command supervisor (`docs/research/virtualization/vm-exec-health-probes-comprehensive-research.md:20-24, 115-131`). That research recommends against SSH or a QEMU-guest-agent-style component *for health probes* (lines 29-30); that is about probes, not human access. (It predates the NIC-less pivot; its "routed TAP" wording is stale.)
- Therefore the only human path into a guest today is a network service the workload itself runs (sshd) reached over a TCP path - which returns to section 3.3.

#### 3.5 Secrets

- Searching `crates/overdrive-core/src/aggregate` for `secret|env` returns no matches, and `Vm` has no environment field (`mod.rs:178-189`). **No mechanism delivers an API key or token to a guest.** The kernel command line is platform-owned with no operator input (`KernelCmdline`, `config.rs:705-710`). The "credential proxy" idea (dummy credentials in the workload, real ones held by the platform) appears in the sprites research [13] and in issue #98's title [17]; it is not implemented. Practical stopgaps (bake into the rootfs; write onto a persistent disk over SSH) both need the persistent disk first.

#### 3.6 Resource limits

- `[resources]` is `cpu_milli: u32` and `memory_bytes: u64` (`traits/driver.rs:283-286`). vCPUs = `max(1, ceil(cpu_milli/1000))`, capped at 255; the guest sees exactly `memory_bytes`; the cgroup ceiling adds a measured reserve (8 MiB + 1/400) (`config.rs:331, 368`). No disk size, IO limit or hotplug in the spec. A 16 vCPU / 64 GiB dev box is expressible; disk size is bounded only by the BYO rootfs image (the clone is a same-size reflink; `rlimit_fsize` = max(rootfs, RAM), `config.rs:903-907`).

#### 3.7 Existing sandbox research and GH issues

- `sprites-as-overdrive-primitive-research.md` (2026-04-19) recommends: no `sprite` driver; extend the `microvm` driver with a persistent rootfs bound to workload identity and CH snapshot/restore; add an `agent_workload` *profile* composing persistence, an auto gateway route, idle-eviction and the credential proxy; phase order snapshot, persistent rootfs, profile, idle-eviction (its §6.1) [13]. This doc builds on it. New facts since April: volumes were cut from #42 (2026-08-18), the guest became NIC-less (ADR-0145), and raw TCP ingress got its own issue (#338).
- The "persistent microVMs" arc, all open with **empty acceptance criteria**: #96 (CH snapshot/restore; by #99's dependency list [18], issue #96 itself not viewed), #97 overdrive-fs [11], #98 gateway auto-route + credential-proxy defaults [17], #99 idle-eviction with checkpoint [18]; #97 is blocked on #22 Garage (open, Phase 1) [19].
- Other issues viewed: #54 gateway subsystem [20] (open; the delivered `public-ingress-gateway` feature is its first slice), #303 in-guest kernel mTLS ("nothing here is accepted") [21], #259 [10], #338 [14], #296 [15], #77 [16], #43 [12].

### 4. Prior art (primary docs, accessed 2026-11-08)

| Platform | Persistence model | Access model | Lesson for Overdrive |
|---|---|---|---|
| Fly Machines + Volumes | Volume = "a slice of an NVMe drive on the same physical server as the Machine", **one Machine per volume**, tied to that host; host-drive failure takes the instance down; daily block snapshots (5 days default, 1-60); default 1 GB, max 500 GB, grow-only [22] | Machine networking via Fly proxy | Node-pinned single-writer volume is an accepted, shipping shape; the simplest persistent disk is local, not object-backed |
| Fly Sprites | Persistent ext4; NVMe while running, backed up to durable object storage on idle, restored on wake; memory persists too; unique URL per sprite [23][13] | Sprite CLI/URL (docs page does not state SSH) | The object-backed variant is the #97 design; it is a later, larger step than a local volume |
| E2B | Pause saves filesystem **and memory**; `filesystem`-only mode reboots on resume; paused sandboxes kept indefinitely; services unreachable while paused; running time cap 24h Pro / 1h Hobby [24] | SDK | Process-state persistence is a feature layered on top of disk persistence |
| GitHub Codespaces | A Docker container on a GitHub-hosted VM; 2-32 cores, 32-128 GB storage; connect via browser, VS Code or GitHub CLI [25] | Not SSH per this page | A dev environment is a sized VM with a fixed disk and a proxied access path; the page does not say what persists on stop [gap] |
| Coder workspaces | Resources are "persistent" (stay provisioned when stopped) or "ephemeral" (destroyed and recreated on restart); Coder agent provides SSH, terminal and IDE access [26] | Agent-mediated | The persistent/ephemeral split per resource is the same distinction a Service-with-volume needs |
| Daytona, Modal | Not fetched (turn budget). Modal Sandboxes and gVisor snapshots are covered in the existing sprites research [13] | | [gap] |

Independent sources agree on the architecture: **a long-lived dev environment = compute that can be stopped and replaced + a separately-lifecycled persistent disk + an access path that is not the app's own HTTP port.** Fly, Coder and E2B each separate disk lifetime from compute lifetime.

### 5. Gap analysis
#### 5.1 Is Service enough?

**As a workload kind: yes, it is the right kind. As it stands: no.** Mapping herdr's needs onto Service semantics (analysis, from the code in section 3):

| herdr / dev-box need | Service today | Fit |
|---|---|---|
| One long-lived instance | `replicas: NonZeroU32`, `replicas = 1` allowed | Fits. `Job` (run-to-completion) and `Schedule` do not |
| A process stays up | Supervised, restart on crash (single restart authority, ADR-0087 per reconcilers.md) | Fits; herdr's own server is the "app", sshd the front |
| Needs a TCP port to attach | Service requires >=1 `Listener`, TCP/UDP, port != 0 (`mod.rs:392-411`) | Fits (declare 22). The listener is the thing the platform will expose/probe |
| Readiness drives backend membership | Readiness probes gate the Service backend row | Mostly irrelevant for one human-attached box; harmless |
| Restart gives a fresh instance | `workload restart` and crash recovery replace the backend with a fresh allocation id and a fresh reflink clone of the master rootfs (`config.rs:593-636`) | **Fights** the dev box: everything the user wrote to disk vanishes. This is a storage gap, not a Service-semantics gap, provided the disk binds to the workload id rather than the allocation id (design question for #97/#43; not decided anywhere) |
| Human gets a shell | No exec/console/attach | Gap (sshd in the guest + TCP path) |
| Secrets, env | None | Gap |
| Guest size | cpu_milli + memory_bytes | Fits; no disk size |

No evidence was found that the Service model needs to change; what is missing is attached to it (disk, access, secrets), not part of it. herdr's own restart semantics help: after a VM restart herdr's processes are gone but `session.json` restores layout and can resume agent conversations [8], so **persistent disk alone gives an acceptable recovery**; process-state persistence (checkpoint, #96/#99) is an upgrade, not a prerequisite.

#### 5.2 Filesystem

What must persist (inferred from sections 1-2): the repo checkouts and any worktrees; the Rust `target/` directory and cargo registry (tens of GB, rebuild cost is high); herdr session dir and config (`~/.config/herdr*`, `session.json`, snapshots); agent session state and credentials (Claude/Codex homes, git credentials, ssh keys); shell history. What exists: a per-launch disposable reflink clone of a read-only master (section 3.2), no volume, no second disk, no snapshot, no size knob; volumes cut, #97/#43 open with empty acceptance. Prior art (section 4): node-pinned single-writer volume (Fly) is the shipping minimum; object-backed chunking (Sprites, #97's design) is the larger design and depends on #22 Garage (Phase 1, open). #97 itself flags that `fsync` must be honoured and that a root-only data dir breaks CH startup [11].

**Options for persistence (surfaced, not decided; per the repo's "agents propose, the user adds" rule):**

| Option | Prevents | Cost | Cheaper alternative |
|---|---|---|---|
| P1: `overdrive-fs` per #97 (Garage chunks + libSQL + NVMe cache) | Loss of disk on restart, node loss, enables migration/checkpoint arc | Largest: blocked on #22; seam (vhost-user-fs vs -blk) open; acceptance empty | P2 |
| P2: a node-local persistent raw volume attached as a second virtio-blk, single-writer, bound to the workload id (Fly-volume shape) | Loss on restart/redeploy on the same node | Medium: new spec field, lifecycle (create/delete/GC), size, flush correctness, no issue exists; node loss = data loss (accepted by Fly [22]) | P3 |
| P3: make the rootfs itself persistent (skip the per-launch clone, boot the image in place, workload-id keyed) | Same as P2 for the root disk | Smaller spec change but breaks the "pristine master" invariant (RootfsPlan, reclamation sweep ADR-0083), makes upgrades of the image destructive, and bloats the root disk | none |

Whichever is chosen, the dev box is the first consumer that needs a volume survive `workload restart` and a crash-restart; that lifecycle rule (volume belongs to the workload, allocation is replaceable) is the contract-shaping decision.

#### 5.3 Separate sandbox primitive? Options

- **(a) Service + persistent volume + private gateway with an L4 listener.** Adds only: volume (P1/P2), L4 ingress (#338) on a private binding (#296 + #77), image (#259 or BYO), secrets. Prevents: losing the box on restart; no way to attach. Cost: each piece is already a tracked or small item. Leaves out: pause/resume, scale-to-zero, auto URL, credential proxy.
- **(b) A first-class Sandbox/Workspace kind.** Would bundle the persistent disk, interactive attach, checkpoint, idle-eviction and credential proxy. Prevents: having every sandbox consumer re-assemble those pieces; gives first-class pause/resume semantics. Cost: new intent kind (rkyv envelope, wire, CLI, reconciler, describe), plus all of #96-#99, and the sprites research already recommended **against** a new driver and for a composed profile [13]. Premature for one consumer.
- **(c) A VM workload kind with persistence** (i.e. the volume half of (a) placed on `Vm`/driver instead of Service). Same as P2/P3 attached to the driver payload; essentially the storage half of (a), not an alternative to it.

**Recommendation (advisory):** (a), with the volume specified so that it composes with a later (b) (workload-identity-keyed, single-writer, flush-honouring - exactly #97's stated constraints). No evidence found that a separate Sandbox kind is needed for the herdr dev box; the question reopens when pause/resume or scale-to-zero is wanted, at which point #96/#99 supply the substance of (b) as a profile (matching the April research). The deciding user choice is P1 vs P2 for the first volume.

#### 5.4 Ordered gap list

Numbered in dependency order; MVP = minimum viable herdr dev box (one maintainer, one node, reached over a trusted network). Issue numbers were viewed on GitHub on 2026-11-08 unless marked.

| # | Gap | Issue | Blocks MVP? |
|---|---|---|---|
| 1 | **Persistent writable disk surviving restart/redeploy**, size knob, flush-correct, bound to workload id | #97 (open, empty AC, blocked on #22) and #43 (virtiofsd; open). No issue for the simpler node-local volume (P2) | **Yes** |
| 2 | **A bootable image**: Linux 7.0 kernel (bzImage/ELF), rootfs with toolchain, git, sshd, herdr, `overdrive-init` as PID 1 | #259 (open). BYO artifacts work today, so a hand-built image is possible; the factory only makes it repeatable | No (manual build possible) |
| 3 | **Raw TCP (SSH) ingress to a VM Service** | #338 (open, no comments; its validation case is exactly "SSH to a VM Service") | **Yes** (only inbound path for herdr remote attach) |
| 4 | **Restrict that ingress to a private network**: named Gateways with private listener | #296 (open) - note it is about named Gateway resources for the HTTP gateway; whether a private binding can carry #338's raw TCP is **not stated** in either issue [gap] | Not strictly, if exposure on a firewalled/public address with key-only SSH is acceptable to the maintainer; **Yes** if "private only" is a requirement |
| 5 | **WireGuard underlay** that gives the private binding an address to bind | #77 (open, blocked by #36 enrollment, #75 image factory MVP, #84; empty AC). Stopgap outside Overdrive: operator-run WireGuard/Tailscale on the host, plus the ability to bind #338 to that address (bind-address control **not specified** in #338) | Same as 4 |
| 6 | **Getting secrets into the guest** (model API keys, git tokens, metal SSH key) | None found. Credential-proxy defaults appear only in #98's title | Not for MVP (write onto the persistent disk after first login), but then gap 1 gates it |
| 7 | **Guest egress verified through `serve` + `deploy`** (api.anthropic.com, github.com, crates.io, DNS) | Design accepted (ADR-0162, ADR-0145); spikes only (ADR-0145 states none ran via serve+deploy); ADR-0145 header still says "Not yet implemented" while `vmm.rs` already launches NIC-less. No issue for dev-box egress validation | **Yes** (unverified premise; one Tier-3 check) |
| 8 | **Dev-box execution mode for repo tooling**: hooks and `xtask` assume Lima or metal | None found | Needed to *develop Overdrive from the box*; not needed for herdr itself |
| 9 | **Guest kernel config + coexistence of developer eBPF/nft with `overdrive-init`'s guest eBPF capture** | None found; unverified (ADR-0150) | Yes for kernel-level dev work; no for plain Rust work |
| 10 | Large disk / `target/` sizing, cargo cache strategy | Part of gap 1 | Part of gap 1 |
| 11 | Interactive exec/console verb in the CLI | None found; sshd makes it unnecessary for MVP | No |
| 12 | Checkpoint/restore to keep herdr's agent processes alive across restarts | #96 (not viewed; named in #99), #99 | No (herdr resumes agent sessions from `session.json`) |
| 13 | Auto-route (`expose = true`), credential-proxy sidecar | #98 | No |
| 14 | `kvm-tests` is fail-closed and non-signal in a nested guest | testing.md rule, not a CH limit (CH supports nested) | Iterate nested if the probe is relaxed for non-evidence runs; evidence runs on metal via `cargo xtask metal run --`. Needs: host nested KVM enabled, VMX/SVM passed to the guest |
| 15 | Cross-node VM addressing (box on one node, reached from another) | #317 (cited in brief.md:1154, **not viewed**) | No (single node) |

**Minimum viable slice:** gaps 1 + 3 + 7 (plus a hand-built image and a firewalled bind), then 4-5 when "private" is required. Nothing in this list requires changing the Service kind.

#### 5.5 The KVM constraint

See section 2: nested KVM works in Cloud Hypervisor, so the dev box *can* boot microVMs; testing.md makes such runs non-signal and the probe fails closed. Whether to allow nested runs for iteration (keeping metal as the evidence authority) is a decision for the maintainer, not a capability gap. The metal box stays the authority for CH-driver and `vendors/cloud-hypervisor` work.

## Knowledge Gaps
1. **herdr resource profile and git-worktree behaviour**: not documented on herdr.dev or the README (attempted: site, /core, /tui, docs pages for socket API, session state, connecting machines, persistence). Recommend measuring a running herdr server in a guest.
2. **herdr socket authentication**: the socket-API page describes none; assumed filesystem-permission based [unverified].
3. **Which products "herdr" means**: TUI (`herdrdev/herdr`) vs herdr-core/app. TUI assumed. Licence appears as Apache-2.0 on the project pages; one search snippet about a fork said AGPL - treated as fork noise.
4. **Whether #296's private listener can carry raw TCP**, and whether #338 can bind to a non-public address: neither issue says so.
5. **Guest egress through `serve` + `deploy`** has no recorded end-to-end run; ADR-0145's "Not yet implemented" header conflicts with the launcher already being NIC-less (`vmm.rs:1335`). Not reconciled.
6. **Issues not viewed**: #96, #317; #77/#296 comment threads were empty on the pages fetched.
7. **Developer-eBPF vs guest-capture eBPF coexistence** inside one guest kernel (ADR-0150): not investigated.
8. **Prior art**: Daytona, Modal, Gitpod not fetched; Codespaces persistence-on-stop not stated on the fetched page; Sprites docs page did not state SSH or sizes.
9. **Size of the Overdrive build footprint** (target dir, vendors) not measured.

## Conflicting Information
- ADR-0145 header: "Not yet implemented" vs `crates/overdrive-host/src/vmm.rs:1335` asserting no `--net` device and ADR-0172 stating "Under ADR-0145 the guest has no NIC". Code is treated as current; the ADR header is probably stale.
- The exec-probes research (guest "routed TAP") predates the NIC-less decision; its networking statements are stale, its guest-agent analysis is not.

## Source Analysis
| Source | Domain | Reputation | Type | Accessed | Cross-verified |
|---|---|---|---|---|---|
| herdr.dev pages | herdr.dev | Medium-High (primary, vendor) | project docs | 2026-11-08 | Partially (README, site agree) |
| herdrdev/herdr repo | github.com | Medium-High | project | 2026-11-08 | Y |
| Overdrive code/ADRs/issues | repo, github.com | High (first party) | code/design | 2026-11-08 | Y |
| docs.fly.io volumes, sprites | fly.io | High | official | 2026-11-08 | Y (with prior research) |
| docs.e2b.dev | e2b.dev | Medium-High | vendor docs (not on the allow-list; used for one factual claim) | 2026-11-08 | N |
| docs.github.com Codespaces | github.com | Medium-High | official | 2026-11-08 | N |
| coder.com docs | coder.com | Medium-High | vendor docs (not on allow-list) | 2026-11-08 | N |
Bias: all prior-art sources are vendors describing their own product.

## Full Citations
[1] herdr. "herdr - agents run in herdr-core; TUI is a separate program". https://herdr.dev. Accessed 2026-11-08.
[2] herdr. "herdr-core". https://herdr.dev/core/. Accessed 2026-11-08.
[3] herdrdev. "herdr (README)". https://github.com/herdrdev/herdr. Accessed 2026-11-08. (Also https://github.com/ogulcancelik/herdr.)
[4] herdr. "Herdr TUI". https://herdr.dev/tui/. Accessed 2026-11-08.
[5] herdr. "Persistence and remote". https://herdr.dev/docs/persistence-remote/. Accessed 2026-11-08.
[6] herdr. "Connecting machines". https://herdr.dev/docs/connecting-machines/. Accessed 2026-11-08.
[7] herdr. "Socket API". https://herdr.dev/docs/socket-api/. Accessed 2026-11-08.
[8] herdr. "Session state". https://herdr.dev/docs/session-state/. Accessed 2026-11-08.
[10] overdrive-sh/overdrive#259, OCI/Dockerfile to rootfs image factory. https://github.com/overdrive-sh/overdrive/issues/259. Accessed 2026-11-08.
[11] overdrive-sh/overdrive#97, overdrive-fs. https://github.com/overdrive-sh/overdrive/issues/97. Accessed 2026-11-08.
[12] overdrive-sh/overdrive#43, virtiofsd lifecycle. https://github.com/overdrive-sh/overdrive/issues/43. Accessed 2026-11-08.
[13] Overdrive. `docs/research/platform/sprites-as-overdrive-primitive-research.md`. 2026-04-19.
[14] overdrive-sh/overdrive#338, public ingress for raw TCP. https://github.com/overdrive-sh/overdrive/issues/338. Accessed 2026-11-08.
[15] overdrive-sh/overdrive#296, named Gateway resources. https://github.com/overdrive-sh/overdrive/issues/296. Accessed 2026-11-08.
[16] overdrive-sh/overdrive#77, WireGuard underlay. https://github.com/overdrive-sh/overdrive/issues/77. Accessed 2026-11-08.
[17] overdrive-sh/overdrive#98, gateway auto-route + credential-proxy defaults. https://github.com/overdrive-sh/overdrive/issues/98. Accessed 2026-11-08.
[18] overdrive-sh/overdrive#99, idle-eviction with checkpoint. https://github.com/overdrive-sh/overdrive/issues/99. Accessed 2026-11-08.
[19] overdrive-sh/overdrive#22, Garage object store. https://github.com/overdrive-sh/overdrive/issues/22. Accessed 2026-11-08.
[20] overdrive-sh/overdrive#54, gateway subsystem. https://github.com/overdrive-sh/overdrive/issues/54. Accessed 2026-11-08.
[21] overdrive-sh/overdrive#303, in-guest kernel mTLS. https://github.com/overdrive-sh/overdrive/issues/303. Accessed 2026-11-08.
[22] Fly.io. "Volumes overview". https://docs.fly.io/volumes/overview. Accessed 2026-11-08.
[23] Fly.io. "Sprites". https://docs.fly.io/sprites. Accessed 2026-11-08.
[24] E2B. "Sandbox persistence". https://docs.e2b.dev/sandbox/persistence. Accessed 2026-11-08.
[25] GitHub. "What are Codespaces". https://docs.github.com/en/codespaces/about-codespaces/what-are-codespaces. Accessed 2026-11-08.
[26] Coder. "Workspace lifecycle". https://coder.com/docs/user-guides/workspace-lifecycle. Accessed 2026-11-08.
Internal: ADR-0145, 0150, 0162, 0172, 0173, 0083, 0082; `docs/feature/public-ingress-gateway/feature-delta.md`; `crates/overdrive-core/src/{aggregate/mod.rs,vm/config.rs,traits/driver.rs}`; `crates/overdrive-host/src/vmm.rs`; `crates/overdrive-cli/src/cli.rs`; `.claude/rules/testing.md`, `development.md`; `docs/research/virtualization/vm-exec-health-probes-comprehensive-research.md`.
(Citation [9] unused.)

## Research Metadata
Examined ~45 pages/files | Cited 26 | Confidence: High 55%, Medium 30%, Low 15% | Output: docs/research/platform/herdr-dev-environment-in-microvm-research.md
