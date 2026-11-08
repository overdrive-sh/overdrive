# Resource-boundary alternatives under native evaluation

This is DESIGN analysis, not acceptance or implementation authority.

The initial finite-CID-window proposal was written before reading the actual pinned Linux source. Exact v7.0 source is now recovered and tag-verified against the repository's gitlink. It offers a smaller candidate: Linux's existing local-mode vsock namespace boundary can own CID and listener isolation together, with kernel reference-counted lifetime instead of a new userspace allocation/cleanup protocol.

| Candidate | Ownership and bounds | Present disposition |
|---|---|---|
| Original 224 CID windows plus claim records | New finite allocation state; must also prove owner/cleanup lifetime and separately handle listeners | Do not entrench. Re-evaluate after the source-supported namespace candidate runs. |
| Existing per-group VMM launch inside an anonymous local-mode host vsock network namespace | Existing numeric CID bands and guest-port contracts; kernel keys CIDs/listeners by held namespace, no shared allocator or numeric per-run hash | Bounded native primitive and real-VMM wrapper probes are being completed. This host namespace is launcher confinement; tests still exercise their disposable guest kernels. No outer guest around a whole run. |
| Per-worktree numeric hash | No atomic owner and no collision-free representation | Insufficient for the zero-collision outcome. |
| A new allocation daemon or generic guest pool | Adds an independent service/protocol | No need established; outside this bounded adaptation. |

Exact source facts at `v7.0`, commit `028ef9c96e96197026887c0f092424679298aae8`:

- `drivers/vhost/vsock.c:688-690` holds the opener's network namespace; `:74-88` matches a CID only when `vsock_net_check_mode` permits that namespace pair; `:788-824` makes the CID claim atomic under the existing kernel mutex.
- `include/net/af_vsock.h:305-323` permits different network namespaces to communicate only when both are global. A local-mode namespace is disjoint from other namespaces, including other local-mode instances.
- `net/vmw_vsock/af_vsock.c:781-832` binds/listens within the socket's namespace. Existing kernel-assigned port semantics remain available; same fixed port in separate local namespaces is also disjoint.
- `net/vmw_vsock/af_vsock.c:2824-2864,2923-2932` makes current mode read-only and derives a new child's mode from its parent's child mode. Therefore the bounded candidate uses a fresh anonymous parent namespace, sets **only that parent's** `net.vsock.child_ns_mode=local`, then creates the local child. It never changes the physical initial namespace's sysctl.
- `drivers/vhost/vsock.c:753-785` removes the device's CID and drops its held namespace reference on release. Production CID-NOCLAIM remains: only the actual VMM holds its vhost device. A temporary direct ioctl probe observes kernel namespace keying; it is not the proposed production launch path or a userspace vhost preclaim.

[Exact fetched files and hashes](kernel-source-provenance.json); [official tag/commit ref capture](../spike/linux-v7-tag.txt). The checkout's vendored directory remains untouched; only the required files were fetched from the exact upstream revision.

No new host network interface, host route, bridge, TAP, nftables rule, BPF program or test cgroup is needed by the namespace candidate. The two anonymous host namespace objects and their held handles would be explicitly owned launcher-confinement resources. User approval is still required before adopting that new material resource boundary. Native construction proof must establish actual VMM/guest routing and cleanup, not merely successful namespace creation.
