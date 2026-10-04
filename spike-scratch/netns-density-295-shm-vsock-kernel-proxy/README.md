# Shared virtqueues, vsock and host kernel TCP/UDP adapter

This preserved NW-SPIKE answers one question: can application data cross real
stock `vhost-vsock` shared virtqueues and be adapted to TCP/UDP by a loaded host
kernel module, with zero TAPs and zero userspace application forwarding relays?

**WORKS at the measured host boundary.** The final accepted evidence is the
qualified **metal fixture**, running an isolated QEMU/KVM host-role VM with the
stock Ubuntu **7.0.0-29-generic** kernel. `increment-i` held 16,384 independently
started vhost devices and 16,384 live kernel UDP adapter sockets, rechecking a
CID-tagged bidirectional datagram through every owner. Its functional prefix
also exercised TCP, half-close, backpressure, UDP inbound/outbound and truncation.
`increment-k` repeated only the small functional prefix with distinct UDP
endpoint addresses, 127.0.0.2 and 127.0.0.3. It also ran on the metal fixture.

The experimental `shmproxy.ko` executes in the private VM kernel, **never in the
physical host kernel**. It calls stock `sock_create_kern`, `kernel_accept`,
`kernel_connect`, `kernel_bind`, `kernel_sendmsg` and `kernel_recvmsg`. Minimal C
is stock socket-API glue, not C eBPF. No BPF program or kernel patch is used.
The standalone Rust harness supplies synthetic guest virtio endpoint bytes and
ordinary TCP/UDP endpoint actors. Those actors echo on their own ordinary
socket; they do not bridge ordinary sockets to vsock. All such adaptation is in
the module. The outer QEMU VM supplies CPU/RAM/boot hardware; it is not an
application relay and does not emulate a vsock application backend here.

Each counted transport opens its own `/dev/vhost-vsock`, sets an independent
CID and memory table, configures two actual split virtqueues and their eventfds,
and executes `VHOST_VSOCK_SET_RUNNING`. Each memory context is 256KiB. Every
counted adapter additionally owns one accepted kernel AF_VSOCK stream and one
kernel IPv4 UDP socket. The final audit reconciles open vhost FDs, different
mappings, kernel socket bindings, input/reverse kernel workers, live session
counters and actual traffic. This is 16,384 **transport/adapters**, not 16,384
running guest OS instances or a population of TCP streams behind one device.

## Preserved attempts

| Attempt | Evidence and outcome |
|---|---|
| a | Installed 7.0 headers rejected older `sockaddr` casts; zero loaded modules/devices. |
| b | Module linked; a foreign Cargo cache permission failed the harness build. No foreign cache was changed. |
| c | Module/harness built; the EFI zboot wrapper was unsuitable for direct arm64 QEMU boot. Owned VM terminated before module execution. |
| d | Decompressed the unmodified stock Image; real vhost→kernel AF_VSOCK accept succeeded; TCP failed with the private loopback link down. Clean module unload. |
| e | Functional TCP/UDP proof, two simultaneous transport owners. Historical Lima preparation. |
| f | Tagged owner checks through 256; inherited inner-process FD hard limit 4096 stopped later admission. Clean unload. Historical preparation. |
| g | A prototype 15s idle timer expired early owners during admission; port-reuse assertion failed. Historical preparation. |
| h | Corrected idle interval and blocking receives; 4,096 live adapters, every owner checked, clean unload. Historical preparation. |
| i | Final metal/KVM functional and 16,384 live kernel UDP adapter proof. Full original stdout and executed out archive retained; diagnostic JSON retrieval gap disclosed below. |
| j | Distinct-address functional preparation in Lima, completed before the user's prohibition. Historical only. |
| k | Final metal/KVM small functional proof with distinct UDP addresses. Full native JSON/serial receipts retained. |

No further Lima runs followed the user's explicit prohibition. Final claims use
metal i/k. No production crate, DESIGN artifact, roadmap, DES log or commit was
created or changed by this spike.

## Evidence preservation issue

The i launcher incorrectly retrieved the native ignored `out` archive instead
of the source increment's `evidence` directory. The subsequent k source sync
replaced the remote i diagnostic JSON files with the local source copy. The
original **complete stdout/serial log**, live kernel audits, source manifest and
exact executed module/harness/kernel/rootfs archive survived. Their source is
`increment-i/evidence/native-launch.log`; the original ignored archive SHA256 is
`9ce703db16577ac0c35e28d31182c8f155d277f191bd5b6cc4c570b4e4e9546e`.
Files named `derived` or `from-launch` explicitly identify extraction after the
run. The i outer-QEMU CPU samples are missing and are not invented. The k
launcher correctly retrieved native diagnostic JSON, but copied its synchronized
bootstrap log over the local launcher log; `launcher-result.recorded.json`
identifies the returned launcher result. The current launcher preserves its
local launcher log and retrieves evidence only. Executed attempt sources are
unchanged; i's actual launcher is retained as `launch.executed.py`.

Builds, VM images and private raw archives are preserved under ignored `out/`.
Only source, scripts, source captures and evidence belong in the reviewed commit.

## Run on the qualified metal fixture

```sh
python3 spike-scratch/netns-density-295-shm-vsock-kernel-proxy/launch.py increment-i
python3 spike-scratch/netns-density-295-shm-vsock-kernel-proxy/launch.py increment-k
```

The canonical exclusive metal lease is used. Existing output IDs refuse reuse;
make a new preserved increment for a new run. i uses a private 16GiB KVM VM,
four online/64 possible CPUs, stock PID/thread defaults, and an 8GiB outer-host
memory reserve. Only the inner harness's FD limit changes. No host sysctl,
foreign network configuration, global module unload or guest kernel fork occurs.

Read the [findings](../../docs/feature/netns-density-295/spike/shm-vsock-kernel-proxy-findings.md)
for framing, limits, the CH backend seam, unvalidated guest/kTLS/SSH boundaries,
and the slow teardown with retained stock close-work warnings.
