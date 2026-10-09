# netns-density-295 — V-10: peer-CID authenticity at the PACKET level

Throwaway spike. Findings: `docs/feature/netns-density-295/spike/findings-v10.md`.

**Question (A-8 / K-A1).** Bind-level peer-CID authenticity is proven (P-17): a
guest cannot *bind* an `AF_VSOCK` socket to a CID the host did not assign it.
V-10 asks the packet-level question: can a guest that **crafts vsock packets
with a forged `src_cid`** (a patched guest driver / guest kernel module / raw
virtqueue writes) make the HOST kernel deliver a connection or datagram
attributed to a CID *other* than the one the host assigned that VM? ADR-0157 and
ADR-0177 attribute the node-shared `VMADDR_CID_ANY:1234` listener by peer CID
(plus the ATTR-TOKEN launch token); a packet-level forgery would void that.

**Hypothesis.** The host enforces `src_cid` authenticity in
`vhost_vsock_handle_tx_kick()` (`drivers/vhost/vsock.c`): it compares the
guest-supplied header `src_cid` against the host-set `vsock->guest_cid` (set only
by the `VHOST_VSOCK_SET_GUEST_CID` ioctl the VMM issues — the guest cannot touch
it) and `dst_cid` against the local CID, and `kfree_skb()`s a mismatch.

**Predicted outcome** (from the cited upstream v7.0 source,
`spike-scratch/netns-density-295-ch-vhost-vsock/linux-reference/drivers-vhost-vsock.c:552`):
a forged-`src_cid` packet is dropped BEFORE `virtio_transport_recv_pkt()`, so it
reaches no host socket. Control connections carry the real CID (3271). The host
vhost tap (`virtio_transport_deliver_tap_pkt`, called *before* the check) still
shows the forged `src_cid` on the wire, proving the forge was real and then
dropped.

**Falsification.** If the host listener ever accepts a connection / receives a
datagram whose `getpeername()` CID is the forged value (3272 or 2) rather than
the assigned 3271 — i.e. a forged `src_cid` is honoured — A-8 is falsified and
ATTR-TOKEN must bind the packet level, not only the socket-bind level (a
security decision back to DESIGN).

## Apparatus

- `forge/vsock_forge.c` — guest kernel module. A `kretprobe` on
  `virtio_transport_get_local_cid()` overrides its return to a forged CID, so the
  guest common layer stamps a forged `src_cid` into every packet it builds
  (`forge_cid=0` = passthrough control). Built out-of-tree against the host's
  `7.0.0-29-generic` headers (vermagic matches the guest kernel). Loaded ONLY in
  the guest, never on the host.
- `guest_client.c` — ordinary `AF_VSOCK` client (STREAM + SEQPACKET to
  `VMADDR_CID_HOST`), plus a `midstream` mode that flips the forge on mid-stream.
- `host_listener.c` — node-shared `VMADDR_CID_ANY` STREAM+SEQPACKET listener that
  logs the peer CID the host attributes to each accepted connection.
- `capture.py` — `AF_PACKET` capture on a `v10-vsockmon0` link; decodes
  `af_vsockmon_hdr.src_cid` to show the as-sent (forged) CID on the host wire.
- `guest-init.sh` — guest PID 1.
- `driver.sh` — host orchestrator (fail-closed x86_64/`/dev/kvm`/virt=none gate;
  reuses a prebuilt fork CH; builds; boots one guest per scenario; records).
- `run.py` — ships sources to `/tmp/v10-<ts>/`, runs `driver.sh` under the
  canonical `flock` lease, retrieves evidence (address redacted).

Scenarios (attacker VM CID = 3271): `control` (forge=0), `forge3272`
(impersonate a sibling tenant CID), `forge2` (impersonate the host),
`midstream` (establish real, then forge data packets).

CIDs used: 3270–3279 only. Resources prefixed `v10-`. Nothing not created by the
probe is touched; vsockmon/vhost_vsock unloaded only if the probe loaded them.
