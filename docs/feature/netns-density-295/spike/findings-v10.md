# V-10 — Peer-CID authenticity at the PACKET level (crafted guest forging `src_cid`)

**Feature:** netns-density-295  **Item:** V-10 (A-8 / K-A1)  **Date:** 2026-10-09
**Kernel:** `7.0.0-29-generic` (host and guest, same bzImage), x86_64, `systemd-detect-virt=none`
**Substrate:** native x86_64 metal, real `/dev/kvm` (KVM_GET_API_VERSION=12, create-VM probe OK), virt=none — fail-closed preflight passed
**Fork CH:** `cloud-hypervisor v53.0.0`, sha256 `bb8f79275fd85a2ff89de33f247e90af6523c6b2db1f7c7de566a14aa8b512a8` (prebuilt binary reused from `spike-scratch/netns-density-295-guest-vsock-capture/out/`; fork is `vendors/cloud-hypervisor` branch `overdrive/vhost-kernel-vsock`; exact commit not recoverable from the binary, `--vsock backend=vhost-kernel` confirmed at runtime)
**Guest kernel:** sha256 `b51367c7dab2f3824ca811c7e33b7f6bb0ddc8122b48248335ba6164de8d9682`
**Probe:** `spike-scratch/netns-density-295-v10/` (raw evidence in `increment-a/evidence/`)

> Authorized defensive security research: validates the design's peer-CID-authenticity threat-model assumption by running an adversarial guest (crafted `src_cid`) inside a disposable operator-controlled VM. The forge apparatus attacks nothing outside that VM.

---

## Verdict: HOLDS — A-8 / K-A1 confirmed at the packet level

A guest that crafts vsock packets with a forged `src_cid` **cannot** make the host deliver a connection or datagram attributed to any CID other than the one the host assigned it. The host `kfree_skb`s every packet whose header `src_cid` ≠ the device's host-set `guest_cid`, *before* it reaches any `AF_VSOCK` socket. Peer-CID attribution (ADR-0157, ADR-0177) is authentic at the packet level; **ATTR-TOKEN does not need to additionally bind the packet level** against a src_cid-forging guest. (ATTR-TOKEN / ATTR-LIVE-CLAIM remain necessary for the *distinct* K-A2′ concern — CID **reuse across launches** — which this item does not touch.)

| Sub-claim | Result |
|---|---|
| Connection (STREAM) attributed to a forged CID | FALSIFIED (forgery fails) → A-8 holds |
| Datagram (SEQPACKET, the design's guest→host shape) attributed to a forged CID | FALSIFIED (forgery fails) → A-8 holds |
| Re-attribution across an established handshake (forge mid-stream) | FALSIFIED (forgery fails) → A-8 holds |

## Hypothesis, prediction, falsification

- **Hypothesis.** The host enforces `src_cid` authenticity in `vhost_vsock_handle_tx_kick()` (`drivers/vhost/vsock.c`), comparing the guest-supplied header `src_cid` against the host-set `vsock->guest_cid` (set only by the `VHOST_VSOCK_SET_GUEST_CID` ioctl, unreachable from the guest) and `dst_cid` against the local CID, dropping a mismatch.
- **Prediction** (upstream v7.0 source, cited at `drivers/vhost/vsock.c:552`):
  ```c
  /* Only accept correctly addressed packets */
  if (le64_to_cpu(hdr->src_cid) == vsock->guest_cid &&
      le64_to_cpu(hdr->dst_cid) == vhost_transport_get_local_cid())
          virtio_transport_recv_pkt(&vhost_transport, skb, vsock->net);
  else
          kfree_skb(skb);
  ```
  A forged-`src_cid` packet is `kfree_skb`'d before `virtio_transport_recv_pkt`; control traffic carries the real CID 3271; the vhost monitor tap (`virtio_transport_deliver_tap_pkt`, run before the check) still shows the forged `src_cid` on the wire.
- **Falsification.** The host listener accepts with `getpeername()` CID = the forged value (3272 or 2) rather than 3271.

**Predicted == actual, exactly.**

## Apparatus

- **Forge:** guest kernel module `vsock_forge.ko` — a `kretprobe` on `virtio_transport_get_local_cid()` (the single value the common layer stamps into `hdr->src_cid`), overriding its return to a forged CID. Built out-of-tree against the guest's `7.0.0-29-generic` headers; loaded only inside the throwaway guest. `forge_cid=0` = passthrough control.
- **Guest:** ordinary `AF_VSOCK` client (STREAM→`CID_HOST:1234`, SEQPACKET→`CID_HOST:1235`), busybox initramfs, stock vsock modules, no NIC.
- **Host:** node-shared `VMADDR_CID_ANY` STREAM+SEQPACKET listener (models ADR-0157's beacon listener), logs peer CID via `getpeername`; plus a `v10-vsockmon0` capture decoding `af_vsockmon_hdr.src_cid` to prove the as-sent (forged) CID on the wire.
- Attacker VM CID = 3271; CIDs 3270–3279 only; resources `v10-` prefixed.

## Evidence

### Control — `forge_cid=0`
```
GUEST connect(type=1 port=1234) OK / stream sent "V10-STREAM-from-3271"
GUEST connect(type=5 port=1235) OK / seqpacket sent "V10-SEQ-from-3271"
host: ACCEPT STREAM peer_cid=3271 ... ; ACCEPT SEQPACKET peer_cid=3271 ... ; SUMMARY accepts=2
wire: CONNECT src_cid=3271 dst_cid=2 dst_port=1234 ; CONNECT src_cid=2 dst_cid=3271 (RESPONSE) ; PAYLOAD src_cid=3271
```

### forge3272 — impersonate a sibling tenant CID
```
vsock_forge: override get_local_cid real=3271 -> forged=3272
GUEST connect(type=1 port=1234) FAIL errno=110(Connection timed out)
GUEST connect(type=5 port=1235) FAIL errno=110(Connection timed out)
host: SUMMARY accepts=0
wire: CONNECT src_cid=3272 dst_cid=2 dst_port=1234 raw=c80c0000... (0x0cc8=3272)   # no host RESPONSE — kfree_skb'd
```
The tap (before the check) proves the forge was genuine; `accepts=0` and the missing RESPONSE prove the host dropped it.

### forge2 — impersonate the host itself
```
vsock_forge: override get_local_cid real=3271 -> forged=2
GUEST connect FAIL errno=110 (both types) ; host accepts=0 ; wire: CONNECT src_cid=2 dst_cid=2 (dropped)
```
Strict equality to the assigned CID, not a block-list: even CID 2 is dropped.

### midstream — establish real, then forge
```
GUEST connect OK / midstream pre "V10-MID-PRE-from-3271"
host: ACCEPT STREAM peer_cid=3271 data="V10-MID-PRE-from-3271" (accepts=1)
set forge_cid=3272 -> wire: DISCONNECT src_cid=3272 (dropped); connection stays peer_cid=3271, never re-attributed
```

### Roll-up

| scenario | forge_cid | accepts | peer CIDs | wire frames forged | wire frames 3271 |
|---|---|---|---|---|---|
| control | 0 | 2 | 3271 | 0 | 8 |
| forge3272 | 3272 | 0 | — | 2 | 0 |
| forge2 | 2 | 0 | — | 2 | 0 |
| midstream | 3272 | 1 | 3271 | 1 (dropped) | 3 |

## Edge cases

- Forging the host CID (2) is also rejected — strict equality to the assigned `guest_cid`, not a block-list.
- The tap sees the forged packet; the socket layer never does — a host-side capture audit *can* see a misbehaving guest, but delivery/attribution cannot be fooled.
- The drop is silent; the forging guest's own `connect()` times out (`ETIMEDOUT`).
- `vhost-vsock` is guest↔host only; the `dst_cid == local_cid` clause also stops a guest reaching a sibling guest directly, regardless of CID.

## Bounds

- One kernel (7.0.0-29, host and guest). RERUN-618 carries the 6.18 re-run; the check is identical upstream across 6.x/7.x.
- Forge via a `kretprobe` override of `virtio_transport_get_local_cid()` — a faithful stand-in for a patched guest driver; the host check is on received header bytes regardless of how the guest produced them, so the result is method-independent.
- Single attacker VM: the drop happens at the source device's TX path against its own assigned CID, before any routing, so a victim VM's existence cannot change the outcome.

## What this changes in the design

Nothing to remediate. A-8 / K-A1 holds as assumed; the host-enforced CID makes the peer CID trustworthy for a live launch. V-10 moves Open → held/proven (bounded; 6.18 re-run under RERUN-618), discharging the last contract-shaping DISTILL blocker.

**Gate recommendation:** PROMOTE — V-10 discharged.

## Reproduce

```
python3 spike-scratch/netns-density-295-v10/run.py
```
Ships sources to `/tmp/v10-<ts>/`, runs under the canonical flock lease (no `cargo xtask metal run`, no whole-tree rsync), retrieves `evidence/`. Host state restored (module inventory before == after; only `vsockmon` loaded/removed by the probe).
