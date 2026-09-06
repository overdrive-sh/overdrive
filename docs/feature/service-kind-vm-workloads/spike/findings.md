# Service-kind VM workloads: native-metal spike findings

**Date:** 2026-09-06

**Scope:** Application/component evidence for guest-reachable health probes

**Status:** Complete — H1 through H6 passed on native bare metal; no architecture, protocol, public API, or operational limit is accepted by this spike

## Outcome

| Hypothesis | Result | Observed evidence |
|---|---|---|
| H1 — a second guest-initiated vsock session can coexist with the lifecycle Beacon session | PASS | The lifecycle session remained connected after `READY`; a distinct control session connected while the VMM and primary workload remained alive. |
| H2 — the guest can execute and correlate concurrent probes | PASS | Three concurrent requests returned their matching request IDs and distinct outcomes: exit 0, exit 7, and termination by signal 15. |
| H3 — a timed-out probe and its process tree can be stopped without killing the workload or VM | PASS | Authoritative tracked rerun: timeout completed in 510 ms, no process-group member remained, and both the primary workload and VMM remained alive. |
| H4 — concurrency can be bounded and overload rejected promptly | PASS | Authoritative tracked rerun: three requests were admitted, a fourth was rejected as overloaded, total elapsed time was 1,205 ms, and no active request remained. The tested capacity of three was provisional harness data, not a product limit. |
| H5 — disconnect cleanup and reconnect are workable | PASS | One in-flight result became ambiguous on disconnect, the command was not replayed, a new session connected in 21 ms, two later scheduler ticks executed freshly, and cleanup completed within the bound. |
| H6 — the host can probe the current VM `workload_addr` through the production network path | PASS | A root-host TCP connection reached the guest and returned byte-exact `H6-GUEST-OK`; packet capture observed the SYN on the allocation TAP; production stop left no owned VMM, network namespace, systemd scope, or run-directory delta. |

## Evidence classification

This document keeps four classes separate:

- **Observed current implementation:** facts read from the current production composition, such as `ProbeRunner` scheduling, `VmDriver` ownership of live VM state, the one-shot `overdrive-init`, the Beacon lifecycle session, and `AllocationSpec.workload_addr`.
- **Accepted design:** contracts already fixed by accepted ADRs and feature artifacts, including lifecycle Beacon semantics and the current VM/network ownership boundaries.
- **Executed spike evidence:** behavior measured below on the native-metal target using real Cloud Hypervisor, the production kernel/rootfs artifacts, and, for H6, the built default-feature Overdrive binary and production VM/network composition.
- **Unimplemented possibility:** candidate mechanisms demonstrated only by the temporary H1–H5 harness. They establish feasibility and constraints; they do not become product design merely because they worked.

## Evidence-integrity correction

The original probe was incorrectly kept under gitignored `.context/`, contrary
to `.claude/rules/spike.md`. Its earlier failed-attempt raw captures were not
retained and cannot be reconstructed; the descriptions later in this document
are explicitly recollections, not raw evidence. The original passing excerpts
are likewise superseded.

The exact harness was promoted into two tracked, isolated increments and both
final passing paths were rerun from those paths on native metal. The following
captures are the authoritative reproducible evidence:

- H1–H5: `spike-scratch/service-kind-vm-workloads/increment-a-guest-exec-control/runs/0003.{meta,stdout,stderr}` (`exit_code=0`)
- H6: `spike-scratch/service-kind-vm-workloads/increment-b-host-network-probe/runs/0002.{meta,stdout,stderr}` (`exit_code=0`)

The tracked H1–H5 runs `0001` and `0002` preserve two remediation setup
failures before VM boot: stale remote ignored build output blocked rsync, then
the fail-closed preflight rejected missing explicit kernel/rootfs selections.
They were corrected without overwriting evidence. H6 run `0001` also passed,
but the byte-exact original fixture formatting was restored afterward, so H6
was rerun as `0002`; only `0002` is authoritative for the retained source.

## Reproduction identity and safety gate

The target address and credentials are intentionally absent. The local `.env` was excluded from synchronization and was never persisted on the target by the spike wrapper.

The canonical native-metal preflight ran before the spike and passed fail closed. It verified:

- literal `x86_64` architecture;
- `systemd-detect-virt=none`, with no hypervisor CPU flag or virtualization sysfs type;
- hardware virtualization support;
- `/dev/kvm` API access, open, and VM creation;
- unified cgroup v2;
- Cloud Hypervisor availability;
- readable production kernel and rootfs artifacts.

Measured identity:

| Item | Value |
|---|---|
| Git commit | `58694bd8097f937148c5cc51a5003e3a8f419c3a` |
| Host architecture | `x86_64` |
| Host kernel | `Linux 7.0.0-29-generic` |
| Virtualization report | `none` |
| CPU | `AMD EPYC 8024P 8-Core Processor` |
| Cloud Hypervisor | `v53.0` |
| Cgroup filesystem | `cgroup2fs` |
| Kernel artifact | `/srv/vm/overdrive-testing/kernel`, 17,295,752 bytes, SHA-256 `b51367c7dab2f3824ca811c7e33b7f6bb0ddc8122b48248335ba6164de8d9682` |
| Rootfs artifact | `/srv/vm/overdrive-testing/rootfs.ext4`, 67,108,864 bytes, SHA-256 `addf306cfbb412a12be7fb7189b3a1850bd853bb7d45266f6b1aae269393b65e` |

Authoritative sanitized command sequence:

```sh
RSYNC_BIN=spike-scratch/service-kind-vm-workloads/increment-a-guest-exec-control/rsync-without-local-env.sh OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4 OVERDRIVE_METAL_SCENARIO=service-vm-h1-h5-rerun cargo xtask metal run -- bash -lc 'cargo build --manifest-path spike-scratch/service-kind-vm-workloads/increment-a-guest-exec-control/Cargo.toml --release --target x86_64-unknown-linux-musl && bash spike-scratch/service-kind-vm-workloads/increment-a-guest-exec-control/run.sh'
RSYNC_BIN=spike-scratch/service-kind-vm-workloads/increment-b-host-network-probe/rsync-without-local-env.sh OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4 OVERDRIVE_METAL_SCENARIO=service-vm-h6-rerun cargo xtask metal run -- bash -lc 'cargo build --manifest-path spike-scratch/service-kind-vm-workloads/increment-b-host-network-probe/Cargo.toml --release --target x86_64-unknown-linux-musl && bash spike-scratch/service-kind-vm-workloads/increment-b-host-network-probe/run.sh'
```

The synchronization wrappers excluded `/.env`, feature-local `target/` and
`out/`, and the stale pre-remediation `.context` target directory. Capture
preserves stdout and stderr separately, replacing only the configured target
and target host with explicit redaction markers.

Tracked harness identity:

```text
6c76493fd49babe5fb159361710c3534a6048373634d60cff0b8271394be3b7c  spike-scratch/service-kind-vm-workloads/increment-a-guest-exec-control/Cargo.toml
fd6cd8e10966594eb76395bf048187d06e2a8d23c5ca72fcfb9e1cf0df54b859  spike-scratch/service-kind-vm-workloads/increment-a-guest-exec-control/Cargo.lock
ee7391167e347bf49cf531873b7d8ae148be54db9ad7bf4e3dbfe1f23ea3511b  spike-scratch/service-kind-vm-workloads/increment-a-guest-exec-control/src/spike_init.rs
f311da06798dadd18f7b1810f703f7103898925691d00e96cd912717e0970454  spike-scratch/service-kind-vm-workloads/increment-a-guest-exec-control/src/spike_probe.rs
420eb6bce17b30b5d3cf7c548d8ffdb754d4b69367c4ce3c87f1a8715f307da9  spike-scratch/service-kind-vm-workloads/increment-a-guest-exec-control/src/spike_workload.rs
1c939eadda4ef01f6d72870771aa067ab93d06d30b59f399e14404539b2e97d7  spike-scratch/service-kind-vm-workloads/increment-a-guest-exec-control/host_controller.py
9a374f64b85d74af68e325807aaab1a26a76146e6b9e0e505665bd8ef95c236e  spike-scratch/service-kind-vm-workloads/increment-a-guest-exec-control/run.sh
bec81ee6f6a85c1ac967013616b6955a872765b00666bd1cdc4b4fd2fa44cc52  spike-scratch/service-kind-vm-workloads/increment-a-guest-exec-control/capture.sh
13f510b10280b81836e0f522d3503cde9c96f1b4aae9fc47388b9fef25a3e6ad  spike-scratch/service-kind-vm-workloads/increment-a-guest-exec-control/rsync-without-local-env.sh
f2db2c3722476ed56ddace346214527f91cdf50bdccbbc52af3dee6aa1b71f89  spike-scratch/service-kind-vm-workloads/increment-b-host-network-probe/Cargo.toml
1112899d6e1a4957c136b8554e2db3d15a6b26eb0d62762b8af9636284832e1f  spike-scratch/service-kind-vm-workloads/increment-b-host-network-probe/Cargo.lock
420eb6bce17b30b5d3cf7c548d8ffdb754d4b69367c4ce3c87f1a8715f307da9  spike-scratch/service-kind-vm-workloads/increment-b-host-network-probe/src/spike_workload.rs
ddfcfddf0ed7ce35de031447b6b33b7a360a0495a7d0f308e10690ece6c79e3f  spike-scratch/service-kind-vm-workloads/increment-b-host-network-probe/run.sh
258465da6e9e117a5a1f49df3f7a1afcb4396fce0fdfac5f6c172f5093cc27e4  spike-scratch/service-kind-vm-workloads/increment-b-host-network-probe/capture.sh
13f510b10280b81836e0f522d3503cde9c96f1b4aae9fc47388b9fef25a3e6ad  spike-scratch/service-kind-vm-workloads/increment-b-host-network-probe/rsync-without-local-env.sh
```

Authoritative capture identity:

```text
H1-H5 stdout SHA-256  7ec60943442710276d9f43dc96b6901bd3b846b6ac063d50514ccd335ee9cd70
H1-H5 stderr SHA-256  4816ffff36c6aa5806501a754542cacf57b78ea39ecba5b03b8e49618b10c876
H6 stdout SHA-256     951a81d3acda6a193f4d5824e9603e0749ba92dc10619dce27206b64cd568970
H6 stderr SHA-256     42e5f225c16f67ac7d429bdc6cd85b02840892cbf00c2023795bf0ec76d599f2
```

## Method and observations

### H1–H5: candidate guest control mechanism

H1–H5 booted the real production kernel and a private clone of the production rootfs under Cloud Hypervisor v53.0. The private image replaced the current one-shot guest init with an exploratory persistent PID 1. It retained the published Beacon behavior (`READY`, the primary `EXEC`, and an open lifecycle session) and independently initiated a second vsock connection for probe control.

The control framing, capacity of three, process-group mechanism, and message vocabulary were deliberately provisional. They were sufficient to exercise the hypotheses but are not proposed product contracts.

Final observed run:

```text
SUBSTRATE cloud_hypervisor=cloud-hypervisor v53.0 kernel=Linux 7.0.0-29-generic arch=x86_64 virtualization=none
H1 PASS beacon=ready distinct_control=session-1 vmm=alive
H2 PASS correlated=3 exit0=1 exit7=1 signal15=1 primary=alive
H3 PASS timeout=bounded elapsed_ms=510 process_group_residual=0 primary=alive vmm=alive
H4 PASS accepted=3 overloaded=1 elapsed_ms=1205 active_residual=0 provisional_capacity=3
H5 PASS ambiguous_count=1 replay=0 reconnect=session-2 reconnect_ms=21 fresh_tick_count=2 cleanup=bounded
CLEANUP vmm_exit=0 owned_run_dir=<owned-temporary-run-directory>
H1_H5_COMPLETE owned_vmm_residual=0
```

This proves that a persistent in-guest supervisor and a distinct guest-initiated control session are technically viable on the existing kernel/rootfs/vsock substrate. It does not prove that the supervisor must be PID 1 rather than a delegated daemon, nor does it choose a wire protocol, port, capacity, or host-side public interface.

### H6: production host-to-guest network path

H6 retained the production `overdrive-init` and used the built default-feature Overdrive binary, production `VmDriver`, production network provisioner, current VM addressing, and current stop path. Only a small TCP-listener workload fixture was injected into a private rootfs clone.

The connection traversed the root host through the allocation host-veth, network namespace, TAP, and guest address. A capture on the TAP independently observed the SYN.

Final observed run:

```text
SUBSTRATE cloud_hypervisor=cloud-hypervisor v53.0 kernel=Linux 7.0.0-29-generic arch=x86_64 virtualization=none
H6 PASS source_namespace=host target=guest_workload_addr route_dev=ovd-hv-0000 netns=ovd-ns-0000 tap=ovd-tp-0000 response=H6-GUEST-OK
H6_WIRE PASS tap_capture_syn=observed
H6_CLEANUP PASS owned_vmm=0 netns_delta=0 scope_delta=0 run_dir_delta=0
```

This confirms that the existing `AllocationSpec.workload_addr` and VM network topology provide the correct default target and route for host-originated TCP and HTTP probes. It does not test mesh interception behavior or redefine the network ownership accepted by its prerequisite feature.

## Harness iterations and lessons

The following attempts were stopped or failed before a hypothesis result. They are reported so the final pass is not mistaken for a first-attempt result:

1. The first H1–H5 run passed H1, then the exploratory PID 1 let one worker call `waitpid(-1)` and reap children owned by sibling requests. The harness was corrected to reap by process group. This exposes a real design constraint: concurrent request handlers must not independently perform global PID 1 reaping; child ownership must be centralized or partitioned.
2. The next H1–H5 run passed H1–H3, then the Python controller discarded additional newline-delimited frames coalesced in one transport read. Buffer preservation fixed the harness. The product decoder must likewise preserve and decode every complete frame rather than equating one read with one message.
3. The first H6 preparation stopped because the default build correctly rejected a stale BPF object. No VM boot occurred. The BPF artifact was rebuilt.
4. The next preparation attempted the configured Lima path because native BPF build opt-in was absent. It was stopped before VM boot and rerun with `OVERDRIVE_BPF_NATIVE=1` on the verified native host.
5. One synchronization attempt failed before preflight because a root-owned temporary build directory conflicted with deletion. The ignored build-output directory was excluded; no spike ran in that attempt.

All owned VMMs and target-side temporary resources were cleaned up after the final runs.

## Constraints established for DESIGN

1. A distinct guest-initiated vsock session can coexist with the Beacon lifecycle session. The evidence supports keeping probe traffic separate from Beacon; DESIGN still owns the exact session owner, port, protocol, authentication assumptions, and versioning.
2. A persistent in-guest supervisor is viable. The current one-shot init must either evolve into that supervisor or delegate to one. A separate daemon was not tested, so the evidence does not rule it out.
3. Multiple in-flight commands require explicit correlation. The framing decoder must preserve multiple messages delivered by one transport read and partial messages split across reads.
4. PID 1 child ownership and reaping must be centralized or partitioned. Independent concurrent `waitpid(-1)` loops are unsafe.
5. Timeout and cancellation must contain the guest probe process tree and must never target the VMM's host cgroup. The spike killed ordinary fork descendants as a process group; it did not test `setsid`, daemonization, or the availability and operational behavior of guest cgroups. The exact guest containment primitive remains open.
6. Bounded concurrency and prompt overload are feasible. The tested limit of three is not a product recommendation; DESIGN must set or defer the limit with an explicit configuration/default contract.
7. A disconnected session can own cancellation of its active probes. Ambiguous requests must not be replayed; later scheduler ticks may execute as fresh requests. Session generation and request identity must make that distinction explicit.
8. Existing `AllocationSpec.workload_addr` is the appropriate default target for VM TCP/HTTP probes, and the current production network topology can be reused.
9. Host-workload Exec behavior remains separate: the current host cgroup prober and its containment semantics are unchanged by this evidence.
10. The exact Rust port types, method signatures, routing context, wire schema, limits, and error model remain DESIGN decisions. The spike introduces none of them as public API.

## Limitations

- The exploratory H1–H5 init and controller are throwaway feasibility code, not production crates or verification artifacts.
- The probe-control transport was not tested across host daemon crash/restart, guest reboot, protocol-version mismatch, malformed or oversized frames, sustained load, or adversarial guest behavior.
- Process-group cleanup covered ordinary descendants but not deliberate session escape, double-fork daemonization, or guest-cgroup enforcement.
- H6 tested direct host-originated TCP reachability, not HTTP parsing, TLS, the transparent-mTLS interception prerequisite, or external mesh traffic.
- The measurements establish correctness feasibility on the identified native host and artifacts; they are not performance capacity results.

## Evidence gate

H1–H6 provide sufficient native-metal evidence to resume Application/component DESIGN. They remove the earlier feasibility uncertainty around concurrent guest control, bounded execution/cleanup, reconnect behavior, and host-to-guest probe reachability. They do **not** accept an architecture. Component ownership must now be chosen explicitly before public API or protocol shape is written.
