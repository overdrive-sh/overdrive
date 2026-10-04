# Aya Rust SK_SKB vsock forwarding probe

**TCP WORKS at the measured native boundary:** 262,144 bytes in each direction,
backpressure and controlled half-close; 16,384 independent live TCP forwarding
owners, each with a real stock vhost device, CID and memory mapping, and 32,768
occupied SockHash keys. Every final owner passed CID-tagged traffic both ways.

**Nonempty UDP WORKS with BPF linearization:** 59,000-byte datagram boundaries,
distinct actual IPv4 peers on one server port, native endpoint truncation and
fail-closed route/direction negatives. **Complete UDP DOESN'T-WORK:** zero-length
datagrams are lost in both directions on the tested stock kernel path.

All new native execution is on the qualified metal fixture, inside a private
KVM host-role VM with stock 7.0.0-29. No custom kernel module, TAP, userspace
payload relay, production change or DESIGN promotion. `linux-reference/` is
pinned 7.2 primary-source reference text, not a kernel compiled for the probe.

The controller only creates/listens/accepts/connects mapped proxy sockets and
installs cookie routes. Aya `no_std` Rust SK_SKB forwards payload on destination
egress in both directions. Ordinary network apps and synthetic guest virtqueue
drivers are endpoint actors. The separate unregistered vsock baseline is also
an endpoint and never forwards between sockets.

`increment-a` through `increment-k` preserve each attempt. Accepted scale
receipts are h; final UDP/baseline/negative receipts are k. f's incorrect
SockMap occupancy count and i's credit-message fixture error remain recorded.
Sources match execution manifests. Complete raw evidence was retrieved before
each subsequent source sync; derived files are explicitly named. Builds/images
and private archives remain ignored under `out/`.

Retirement at 16,384 took 250.815s. h returns owned FDs 4→4 and reads stock
vhost_vsock usage 0 before private VM shutdown. QEMU and the canonical lease are
released; foreign host modules/links are preserved. Guest transparency,
Cloud Hypervisor, guest kTLS/mTLS, SSH, IPv6, UDP scale, controller restart
survival and throughput are unproved.

See [findings](../../docs/feature/netns-density-295/spike/aya-vsock-proxy-findings.md),
[attempt index](attempt-index.json), [evidence/source manifest](final-evidence-manifest.json)
and [cleanup readback](post-release-readback.json).

For another execution, copy an increment to a new output ID and run:

```sh
python3 spike-scratch/netns-density-295-aya-vsock-proxy/launch.py increment-new-id
```

Existing output IDs refuse overwrite. The canonical exclusive metal lease,
private VM budget and exact evidence retrieval are in the preserved launcher.
